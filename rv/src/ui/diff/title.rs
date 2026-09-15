//! What the diff pane calls itself — split from [`super`] for length.

use rv_core::diff::DiffSource;
use rv_core::diff::FallbackReason;
use rv_core::diff::FileDiff;
use rv_core::diff::MINIMUM_DIFFT;
use rv_core::store::CommentState;

use crate::app::App;
use crate::app::reviewed::Freshness;

/// What the title adds for a file rv ships no grammar for.
///
/// Said out loud rather than left to be inferred from a screen of white text: a
/// tool that presents "I could not" as "there was nothing to find" is guessing
/// on the reader's behalf.
///
/// Decided from the **path**, not from whether a parse has landed. Highlighting
/// runs off the drawing thread, so for the first frames of a large file there are
/// no spans yet — and a title reading "no highlighting" over a Rust file that is
/// merely still being parsed is the same guess in the other direction.
const NO_GRAMMAR: &str = " — no highlighting";

/// What the title adds when full-file context was attempted and declined —
/// §3/§4.4 of the design spec: difftastic elided a region (a reformat with
/// a different line count on each side) so there is no honest line-for-line
/// pairing to fill it with, and the pane fell back to the changed-only view
/// rather than guess.
const CONTEXT_BAILED: &str =
    " — full context unavailable (a reformatted region difftastic did not report)";

/// What the title adds when the syntax-aware merge declined and rv's §4.6
/// `--byte-limit 0` retry supplied the merged full-context result instead.
/// Composed after the ordinary engine label so the reviewer reads
/// `difftastic (Rust) — full context (line diff)` — the file's language is
/// still Rust and the syntax highlighting is unchanged (highlighting reads
/// from `highlight::language_of(&diff.path)`, not from `DiffSource`), but
/// the pairings the merge walked come from difftastic's line-oriented
/// engine rather than its tree-diff.
const LINE_DIFF_CONTEXT: &str = " — full context (line diff)";

/// What the title adds when even difftastic's line-oriented retry could not
/// pair a region 1:1 and `similar`'s whole-file diff built full context
/// instead — the recovery [`CONTEXT_BAILED`] used to be the last word on.
/// Distinct wording from [`LINE_DIFF_CONTEXT`] because it is a genuinely
/// different engine, not difftastic's own second invocation: the changed-line
/// boundaries shown are `similar`'s line-level opinion, not difftastic's
/// structural one.
const FALLBACK_CONTEXT: &str = " — full context (fallback line diff)";

/// What the pane calls itself: the path, where its lines came from — so a
/// fallback diff is never mistaken for difftastic's structural one, and a
/// fallback rv *chose* is never mistaken for one forced on it by a difftastic
/// it cannot read — and, where rv ships no grammar, that its code is plain
/// because of that rather than because there was nothing to colour.
///
/// Public for the same reason [`visible`] is: this is the one place the claim
/// the pane makes about its own contents is decided, and the claim is
/// load-bearing — it is what tells a reviewer whether they are reading
/// difftastic's structural diff or a line diff standing in for it, and why.
/// `bailed` is [`App::context_bailed`]'s answer for this file and
/// `via_fallback` is [`App::context_via_fallback`]'s — the two are mutually
/// exclusive in practice (see [`super::super::app::merges::MergeState`]) —
/// appended last, after the grammar note, so a reviewer reads "what this
/// pane is showing" before "what it could not show" or "how it recovered".
#[must_use]
pub fn title(
    diff: &FileDiff,
    language: Option<&'static str>,
    bailed: bool,
    via_fallback: bool,
) -> String {
    let source = match &diff.source {
        DiffSource::Difftastic { language, .. } => {
            format!("{} — difftastic ({language})", diff.path)
        }
        DiffSource::Similar { reason } => match fallback_cause(*reason) {
            Some(cause) => format!("{} — fallback ({cause})", diff.path),
            None => format!("{} — fallback", diff.path),
        },
        DiffSource::Binary => format!("{} — binary", diff.path),
    };
    let with_grammar = match language {
        // A binary file needs no second sentence about why it is not coloured:
        // it is not shown by line at all, and the title already says so.
        Some(_) => source,
        None if diff.source == DiffSource::Binary => source,
        None => format!("{source}{NO_GRAMMAR}"),
    };
    let with_line_diff = if matches!(
        &diff.source,
        DiffSource::Difftastic {
            line_oriented: true,
            ..
        }
    ) {
        format!("{with_grammar}{LINE_DIFF_CONTEXT}")
    } else {
        with_grammar
    };
    let with_fallback = if via_fallback {
        format!("{with_line_diff}{FALLBACK_CONTEXT}")
    } else {
        with_line_diff
    };
    if bailed {
        format!("{with_fallback}{CONTEXT_BAILED}")
    } else {
        with_fallback
    }
}

/// Why the pane is showing a line diff, where that is something a reviewer can
/// act on. `None` where it is not: rv was told not to run difftastic, so the
/// plain word "fallback" is already the whole truth and a parenthetical would
/// only restate the flag the reviewer just passed.
fn fallback_cause(reason: FallbackReason) -> Option<String> {
    match reason {
        FallbackReason::NotAttempted => None,
        FallbackReason::NotInstalled => Some("no difft on PATH".to_owned()),
        FallbackReason::UnreadableVersion => Some("difft version unreadable".to_owned()),
        FallbackReason::TooOld(version) => {
            Some(format!("difft {version} predates {MINIMUM_DIFFT}"))
        }
        FallbackReason::UnreadableOutput => Some("difft output unreadable".to_owned()),
    }
}

/// The title with the file's tick appended, where it has one: `✓ reviewed`,
/// or `≈ reviewed` for a tick the file has changed under. Open comments are
/// counted alongside, because ticked and unresolved are not exclusive and
/// hiding one behind the other would present a guess as a fact.
pub(super) fn reviewed_title(app: &App, title: String) -> String {
    let Some(freshness) = app.shown_reviewed() else {
        return title;
    };
    let mark = match freshness {
        Freshness::Current => "✓ reviewed",
        Freshness::Changed => "≈ reviewed, changed since",
    };
    let open = app
        .comments()
        .iter()
        .filter(|comment| {
            comment.state == CommentState::Open
                && app
                    .selected_file()
                    .is_some_and(|file| file.path == comment.anchor.file)
        })
        .count();
    if open == 0 {
        format!("{title} — {mark}")
    } else {
        format!("{title} — {mark}, {open} open")
    }
}
