//! Syntax highlighting as plain data (spec §7). [`Highlights::of`] runs a
//! tree-sitter grammar over a file's bytes and returns [`Span`]s: for each
//! line, the byte ranges that belong to a [`Capture`] class.
//!
//! No colours live here. `rv-core` is terminal-free, so `Capture` is a
//! vocabulary of *kinds* — keyword, string, comment — and the mapping from a
//! kind to an actual terminal colour lives in the `rv` crate, which is the
//! only place that knows what a terminal is.
//!
//! Three rules shape the rest of this module.
//!
//! **Detection is by name, and only by name.** Nothing sniffs the content. A
//! wrong guess paints a file as a language it is not, which reads worse than
//! no colour at all, so a file whose name selects no grammar comes back with
//! [`language`](Highlights::language) `None` and no spans and is rendered
//! plain. Two tables do the selecting: an extension table, and a short
//! filename table for names whose extension does not name their language —
//! `Cargo.lock` is TOML, and it is the first file in a Rust repository
//! alphabetically, so an rv that cannot colour it looks like an rv whose
//! highlighting does not work. A name earns a filename row only if it is
//! unambiguous; guessing there is content sniffing by another spelling. Those
//! two tables are the *only* place rv decides what a file is written in:
//! [`symbols`](crate::symbols) asks them through [`language_of`] rather than
//! keeping a list of its own, so a file cannot be coloured as one language and
//! searched as another.
//!
//! **Nothing here fails.** A blob that is not UTF-8, a half-typed function
//! that does not parse, a path that is not a path — each produces an answer.
//! A reviewer opening a file must never be the thing that kills the process,
//! and tree-sitter recovers from parse errors on its own, so a broken file
//! still gets most of its colour.
//!
//! **Every span is clamped to its own line.** Offsets are relative to the
//! start of the line, never to the start of the file; a construct that
//! crosses a newline is cut into one span per line; and a span never reaches
//! past the end of the line's text, so a consumer indexing by column cannot
//! panic on a range the grammar reported past the end. [`Span::slice`] is the
//! safe way to take a span's text when the string in hand is not byte-for-byte
//! the blob line the span was measured against — a diff line, for instance.

use tree_sitter_highlight::HighlightEvent;
mod bounds;
mod captures;
mod configs;
mod grammars;

pub use captures::Capture;
pub use grammars::language_of;

use captures::capture_at;

use bounds::ceil_char_boundary;
use bounds::floor_byte_boundary;
use bounds::text_end;
use grammars::grammar_for_path;

use tree_sitter_highlight::Highlighter;

/// One highlighted run: `start..end` are byte offsets **within** 1-based
/// `line`, not within the file, and both are guaranteed to land on a character
/// boundary of that line's text.
///
/// Within a line, spans are ordered by `start` and never overlap, so a
/// renderer can walk them in a single pass and every column belongs to at most
/// one capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub line: u32,
    pub start: u32,
    pub end: u32,
    pub capture: Capture,
}

impl Span {
    /// The text this span covers in `text`, clamped so that it is always a
    /// substring and never a panic.
    ///
    /// The span was measured against a line of the blob that was highlighted.
    /// A caller often holds something close to but not identical to that line
    /// — a `DiffLine`'s text, say, which may have been produced by a different
    /// tool. Rather than make every such caller repeat the same bounds and
    /// character-boundary checks, this clamps `end` to the length of `text`
    /// and walks both ends back to a character boundary, yielding a shorter
    /// slice (possibly empty) instead of slicing out of range or through the
    /// middle of a multi-byte character.
    #[must_use]
    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        let end = floor_char_boundary(text, (self.end as usize).min(text.len()));
        let start = floor_char_boundary(text, (self.start as usize).min(end));
        &text[start..end]
    }
}

/// The largest index `i <= at` that is a character boundary of `text`.
fn floor_char_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Every highlighted run in one file, ordered by line and then by column.
///
/// The default value — no language, no spans — is what a file with no grammar
/// gets, and is what `rv` renders plain.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlights {
    spans: Vec<Span>,
    language: Option<&'static str>,
}

impl Highlights {
    /// Highlights `source` using whichever grammar `path`'s name selects.
    ///
    /// Never fails: a path with no known name, a grammar that cannot be
    /// built, bytes that are not UTF-8 and source that does not parse all
    /// produce a value rather than an error. The first two report no language
    /// (so the caller can say *why* a file is plain); the last two report the
    /// language and whatever spans tree-sitter managed to recover.
    #[must_use]
    pub fn of(source: &[u8], path: &str) -> Highlights {
        let Some(grammar) = grammar_for_path(path) else {
            return Highlights::default();
        };
        let Some(config) = (grammar.configuration)() else {
            // The grammar's query failed to compile — a version skew between
            // the grammar crate and tree-sitter. Report no language rather
            // than a language with no spans, so the UI says "plain" honestly.
            return Highlights::default();
        };

        let language = Some(grammar.name);
        if grammar.name == "bash" && bash_input_would_crash_tree_sitter(source) {
            // tree-sitter-bash 0.25.1 crashes with SIGSEGV on Linux when its
            // internal error-recovery path meets a `{` followed later by any
            // four-byte UTF-8 codepoint (upstream tree-sitter/tree-sitter-bash#337).
            // rv promises `Highlights::of` never fails for any bytes, so a
            // reviewer opening a `.sh` file that happens to hit the pattern
            // must not be the thing that kills the process. Report the
            // language and no spans, the same shape as a grammar that gave up
            // mid-parse.
            return Highlights {
                spans: Vec::new(),
                language,
            };
        }

        let lines = LineIndex::of(source);
        let mut highlighter = Highlighter::new();
        let injection = grammar.injection;
        let Ok(events) = highlighter.highlight(config, source, None, injection) else {
            return Highlights {
                spans: Vec::new(),
                language,
            };
        };

        let mut spans = Vec::new();
        let mut stack: Vec<Capture> = Vec::new();
        for event in events {
            // A mid-stream error means the parse was cancelled or the layer
            // gave up; keep the spans collected so far rather than throwing
            // away a nearly complete file's colour.
            let Ok(event) = event else { break };
            match event {
                HighlightEvent::HighlightStart(highlight) => {
                    stack.push(capture_at(highlight.0));
                }
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    if let Some(&capture) = stack.last() {
                        lines.push_spans(&mut spans, source, start, end, capture);
                    }
                }
            }
        }

        tidy(&mut spans);
        Highlights { spans, language }
    }

    /// The language whose grammar produced these spans, or `None` when the
    /// file's extension names no grammar rv ships. `rv` shows this in the
    /// pane's title so a reader knows why a file is plain.
    #[must_use]
    pub fn language(&self) -> Option<&'static str> {
        self.language
    }

    /// The spans on 1-based `line`, in column order. A line with no captures,
    /// line `0`, and any line past the end of the file all give back an empty
    /// slice — a viewport scrolled past the end of a file asks for those
    /// constantly.
    #[must_use]
    pub fn line(&self, line: u32) -> &[Span] {
        let start = self.spans.partition_point(|span| span.line < line);
        let end = self.spans.partition_point(|span| span.line <= line);
        &self.spans[start..end]
    }
}

/// True when `source` matches the pattern that segfaults
/// `tree-sitter-bash 0.25.1` on Linux
/// ([tree-sitter/tree-sitter-bash#337][1]): an ASCII `{` followed anywhere
/// later by a byte in `0xF0..=0xF7`, which is the lead byte of any four-byte
/// UTF-8 codepoint. The grammar's error-recovery path enters a broken state
/// after the `{` and dereferences an OOB pointer at the four-byte lead; the
/// intervening bytes do not matter, so a linear scan with a `bool` flag is
/// enough.
///
/// This is a real crash a `.sh` file in a repository could hit — the
/// discovery route in the upstream report is exactly rv's own property
/// fuzzer — so the guard has to be in [`Highlights::of`] rather than
/// confined to the tests. macOS is unaffected in practice, but the check is
/// unconditional: the file that opens rv on Linux is often the same file a
/// contributor read on macOS.
///
/// [1]: https://github.com/tree-sitter/tree-sitter-bash/issues/337
fn bash_input_would_crash_tree_sitter(source: &[u8]) -> bool {
    let mut after_open_brace = false;
    for &byte in source {
        if byte == b'{' {
            after_open_brace = true;
        } else if after_open_brace && (0xF0..=0xF7).contains(&byte) {
            return true;
        }
    }
    false
}

/// Sorts spans into (line, column) order, drops any that overlap the one
/// before them, and merges neighbours that touch and share a capture.
///
/// The event stream is already ordered and disjoint — tree-sitter-highlight
/// emits `Source` events that partition the file — so the sort and the overlap
/// check are defence in depth: they make [`Highlights::line`]'s binary search
/// and the type's disjointness guarantee hold no matter what a future grammar
/// or a future version of the highlighter emits. The merge is not defensive:
/// adjacent runs with the same capture (a `::` next to a `(`, both
/// punctuation) are genuinely one styled run, and merging them keeps the span
/// list roughly as short as the eye says it should be.
fn tidy(spans: &mut Vec<Span>) {
    spans.sort_by_key(|span| (span.line, span.start));

    let mut tidied: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans.iter().copied() {
        match tidied.last_mut() {
            Some(last) if last.line == span.line && span.start < last.end => {} // overlap: drop
            Some(last)
                if last.line == span.line
                    && last.end == span.start
                    && last.capture == span.capture =>
            {
                last.end = span.end;
            }
            _ => tidied.push(span),
        }
    }
    *spans = tidied;
}

// ---------------------------------------------------------------------------
// Lines
// ---------------------------------------------------------------------------

/// Where each line of a blob starts and where its *text* ends. The end
/// excludes the `\n` that terminates the line and the `\r` of a CRLF ending,
/// because neither is a column a renderer paints — clamping to this end is
/// what keeps a span inside the text a consumer actually has.
///
/// A trailing newline does not open a final empty line: `"a\n"` is one line.
struct LineIndex {
    /// `(start, text_end)` byte offsets, one entry per line, ascending.
    lines: Vec<(usize, usize)>,
}

impl LineIndex {
    fn of(source: &[u8]) -> LineIndex {
        let mut lines = Vec::new();
        let mut start = 0usize;
        for (offset, &byte) in source.iter().enumerate() {
            if byte == b'\n' {
                lines.push((start, text_end(source, start, offset)));
                start = offset + 1;
            }
        }
        if start < source.len() {
            lines.push((start, text_end(source, start, source.len())));
        }
        LineIndex { lines }
    }

    /// The index of the line containing byte offset `at`, or `None` when the
    /// blob has no lines at all.
    fn line_at(&self, at: usize) -> Option<usize> {
        let after = self.lines.partition_point(|(start, _)| *start <= at);
        after.checked_sub(1)
    }

    /// Cuts the byte range `start..end` of `source` into one [`Span`] per line
    /// it touches, each clamped to that line's text and snapped to character
    /// boundaries, and appends them to `out`.
    fn push_spans(
        &self,
        out: &mut Vec<Span>,
        source: &[u8],
        start: usize,
        end: usize,
        capture: Capture,
    ) {
        if start >= end {
            return;
        }
        let Some(mut index) = self.line_at(start) else {
            return;
        };
        while let Some(&(line_start, text_end)) = self.lines.get(index) {
            if line_start >= end {
                break;
            }
            let from = start.max(line_start);
            let to = end.min(text_end);
            // `to` can sit at or before `from` when the range covers only a
            // line's newline (or its `\r`), which is not a paintable column.
            if from < to
                && let Some(span) = span_of(source, index, line_start, from, to, capture)
            {
                out.push(span);
            }
            index += 1;
        }
    }
}

/// Builds the span for `from..to` on the line starting at `line_start`,
/// snapping both ends inward to character boundaries of `source` so that the
/// resulting range never splits a multi-byte character. Snapping only ever
/// shrinks the range, so it cannot make two spans overlap.
///
/// Gives back `None` for a range that snapping emptied, and for the absurd
/// case of a file so large that a line number or column does not fit in a
/// `u32` — dropping the span is better than reporting a wrapped offset.
fn span_of(
    source: &[u8],
    index: usize,
    line_start: usize,
    from: usize,
    to: usize,
    capture: Capture,
) -> Option<Span> {
    let from = ceil_char_boundary(source, from, to);
    let to = floor_byte_boundary(source, to, from);
    if from >= to {
        return None;
    }
    Some(Span {
        line: u32::try_from(index + 1).ok()?,
        start: u32::try_from(from - line_start).ok()?,
        end: u32::try_from(to - line_start).ok()?,
        capture,
    })
}
