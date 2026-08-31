//! The diff pane: which rows are on screen, and what each of them draws as.
//!
//! The window is over [`crate::rows`]'s flattened row list rather than over the
//! diff's own lines, because a comment box is several rows tall: windowing by
//! line would push the highlight off the bottom of the pane while the window
//! still believed it was on screen.
//!
//! The three questions a pointer asks about what is *inside* this pane are
//! answered here too, for the reason [`visible`] is: the window's offset and
//! the note above a suppressed diff are this module's arithmetic, and a hit
//! test with its own copy of either would resolve clicks against a screen that
//! was never painted.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use rv_core::diff::DiffSource;
use rv_core::diff::FallbackReason;
use rv_core::diff::FileDiff;
use rv_core::diff::MINIMUM_DIFFT;
use rv_core::highlight;

use super::BORDER_ROWS;
use super::BOX_PADDING;
use super::GUTTER;
use super::code::Highlighting;
use super::code::diff_row;
use super::comment_box;
use super::pane::pane;
use crate::app::App;
use crate::app::Focus;
use crate::rows::Plan;
use crate::rows::Row;
use crate::rows::window;

/// What the pane says about a diff [`rv_core::diff`] suppressed and gave no
/// lines: difftastic's `unchanged` status, which emits no chunks.
///
/// One spelling, shared with the before/after block inside an outdated comment's
/// box, which reports the same condition about a fragment.
const SUPPRESSED_EMPTY: &str = crate::rows::NO_SEMANTIC_CHANGE;

/// The same, as a header over a suppressed diff that *does* have lines — the
/// `similar` fallback's terminator-only change.
///
/// A note rather than a replacement, because the reviewer can put the highlight
/// on those lines and comment on them: a pane that swallowed them would let
/// `j`/`k` walk through rows it never drew.
const SUPPRESSED_NOTE: &str = "no semantic change — the difference is not visible below";

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

pub(super) fn draw_diff(frame: &mut Frame, app: &App, area: Rect) {
    // The stack is drawn *inside* this pane, so it marks this pane as the one
    // the next keystroke lands in.
    let focused = matches!(app.focus(), Focus::Diff | Focus::Stack);
    let Some(file) = app.selected_file() else {
        frame.render_widget(
            Paragraph::new("no changed files in this range")
                .block(pane("Diff".to_owned(), focused)),
            area,
        );
        return;
    };
    let Some(diff) = app.selected_diff() else {
        // Only reachable if a file's blobs have not been read yet, which the
        // app does before this function is ever called for that file.
        frame.render_widget(
            Paragraph::new("no diff loaded").block(pane(file.path.clone(), focused)),
            area,
        );
        return;
    };

    let highlighting = Highlighting::of(app);
    // The path's own answer, not the cache's: see `NO_GRAMMAR`.
    let block = pane(
        title(
            diff,
            highlight::language_of(&diff.path),
            app.context_bailed(),
        ),
        focused,
    );
    let text = body(app, highlighting, diff, area);
    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// The diff pane's row plan and the slice of it that is on screen, for a pane
/// drawn in `pane` — the whole rectangle, borders included.
///
/// Public because this is the one place the answer is decided, and because the
/// defect it exists to prevent is precisely a window and a cursor disagreeing
/// about which rows exist: a caller that computed its own would be asserting
/// about a third thing neither the pane nor the keyboard uses.
///
/// It also **reports the width the boxes were drawn at** back to `app`: the
/// renderer is the only thing that knows how wide a comment box really is, and
/// the row cursor is an index into rows whose count depends on it.
#[must_use]
pub fn visible(app: &App, pane: Rect) -> (Plan<'_>, Range<usize>) {
    let width = usize::from(pane.width.saturating_sub(BORDER_ROWS));
    app.note_body_width(width.saturating_sub(GUTTER + BOX_PADDING));

    let plan = app.plan();
    let height = content_rows(app, pane);
    let total = plan.rows.len();
    let rows = window(total, anchor_row(app, &plan), height);
    (plan, parked(rows, total, app.diff_scroll()))
}

/// Which row of the plan is under the `row`-th content row of a diff pane drawn
/// at `pane`, or `None` where that row holds no plan row at all.
#[must_use]
pub fn diff_row_at(app: &App, pane: Rect, row: usize) -> Option<usize> {
    let height = usize::from(pane.height.saturating_sub(BORDER_ROWS));
    let row = row.checked_sub(usize::from(suppressed_note(app, height)))?;
    let (_, rows) = visible(app, pane);
    let index = rows.start.checked_add(row)?;
    (index < rows.end).then_some(index)
}

/// The first row on screen after `delta` rows of wheel, clamped to the plan.
#[must_use]
pub fn diff_scrolled(app: &App, pane: Rect, delta: isize) -> usize {
    let (plan, rows) = visible(app, pane);
    let last = plan.rows.len().saturating_sub(rows.len());
    rows.start.saturating_add_signed(delta).min(last)
}

/// How many rows of the plan a diff pane of this size shows: its own, less its
/// borders, less the suppressed note where there is one to take.
fn content_rows(app: &App, pane: Rect) -> usize {
    let height = usize::from(pane.height.saturating_sub(BORDER_ROWS));
    height.saturating_sub(usize::from(suppressed_note(app, height)))
}

/// Whether the pane draws a note above the lines saying the diff is suppressed.
fn suppressed_note(app: &App, height: usize) -> bool {
    app.selected_diff()
        .is_some_and(|diff| diff.suppressed && height >= 2)
}

/// `natural` moved to wherever the wheel has parked the view, or left alone
/// when it has not.
///
/// The park is the *first row on screen* rather than an offset from the cursor,
/// so a selection moving under a parked view does not drag the view with it —
/// which is the whole of "scrolling is looking". Its length is the natural
/// window's, so a pane never shows a row past the end of the plan.
fn parked(natural: Range<usize>, rows: usize, scroll: Option<usize>) -> Range<usize> {
    let Some(start) = scroll else {
        return natural;
    };
    let height = natural.len();
    let start = start.min(rows.saturating_sub(height));
    start..start.saturating_add(height)
}

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
/// `bailed` is [`App::context_bailed`]'s answer for this file — appended
/// last, after the grammar note, so a reviewer reads "what this pane is
/// showing" before "what it could not show".
#[must_use]
pub fn title(diff: &FileDiff, language: Option<&'static str>, bailed: bool) -> String {
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
    if bailed {
        format!("{with_line_diff}{CONTEXT_BAILED}")
    } else {
        with_line_diff
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

/// The visible window of rows, under a note where the diff is suppressed, or
/// the one sentence that explains why there are no lines at all.
///
/// `suppressed` does not imply "no lines". It used to, and this function
/// short-circuited on the flag accordingly; the `similar` fallback now also
/// sets it for a change that lives entirely in the line terminators and reports
/// that change's lines as `Context`, so short-circuiting showed a sentence in
/// place of content the reviewer could still navigate and comment on. The note
/// goes *above* the lines instead, and only where there is a row to take —
/// below two rows the lines win, since a pane that spent its only row on the
/// note would hide the highlight.
fn body<'a>(
    app: &'a App,
    highlighting: Highlighting<'a>,
    diff: &'a FileDiff,
    pane: Rect,
) -> Text<'static> {
    if diff.source == DiffSource::Binary {
        return Text::from("binary file, not shown by line");
    }
    if app.displayed_lines().is_empty() {
        return Text::from(if diff.suppressed {
            SUPPRESSED_EMPTY
        } else {
            "no lines to show"
        });
    }

    let width = usize::from(pane.width.saturating_sub(BORDER_ROWS));
    let height = usize::from(pane.height.saturating_sub(BORDER_ROWS));
    let note = suppressed_note(app, height);
    let (plan, rows) = visible(app, pane);

    // Asked once and handed down the row loop: it is derived from the row
    // cursor over this very plan, and a forty-row pane would otherwise rebuild
    // the plan forty times to paint one frame.
    let selected = app.line_index();
    let mut lines: Vec<Line> = Vec::with_capacity(rows.len() + usize::from(note));
    if note {
        lines.push(Line::styled(
            SUPPRESSED_NOTE,
            Style::default().fg(Color::Yellow),
        ));
    }
    lines.extend(
        plan.rows[rows]
            .iter()
            .map(|row| draw_row(app, highlighting, row, selected, width)),
    );
    Text::from(lines)
}

/// One row of the plan, as one styled line of the pane.
///
/// `selected` is the diff line the row cursor is on, passed down rather than
/// asked per row — see [`body`].
fn draw_row(
    app: &App,
    highlighting: Highlighting<'_>,
    row: &Row<'_>,
    selected: usize,
    width: usize,
) -> Line<'static> {
    match row {
        // Comment boxes are wrapped to the pane and never overflow it, so the
        // sideways scroll moves the code and leaves them anchored.
        Row::Diff { index, line } => diff_row(
            highlighting,
            *index,
            line,
            selected,
            width,
            app.diff_hscroll(),
        ),
        Row::BoxTop { comment, .. } => comment_box::box_top(app, comment, width),
        Row::BoxBody {
            comment,
            text,
            kind,
            ..
        } => comment_box::box_body(app, comment, text, *kind, width),
        Row::BoxRule { comment, .. } => comment_box::box_rule(app, comment, width),
        Row::BoxDiff {
            comment,
            text,
            kind,
            ..
        } => comment_box::box_diff(app, comment, text, *kind, width),
        Row::BoxBottom { comment, .. } => comment_box::box_bottom(app, comment, width),
        Row::BoxCollapsed { comment, .. } => comment_box::box_collapsed(app, comment, width),
    }
}

/// The row the window is centred on: the selected comment's box while the
/// cursor is inside a stack, and the **row cursor** otherwise.
///
/// A cursor that could scroll off the pane it is steering is a cursor the
/// reviewer cannot use, and inside a stack the thing being steered is the box
/// rather than the line it hangs off.
///
/// Outside a stack it is [`App::cursor_row`] and nothing derived from it. That
/// is the fix for spec §10's defect: this used to anchor on
/// `row_of_line(line_index())`, so the anchor could only ever rest on a *diff*
/// row and the rows of a box taller than the pane were in no window at any
/// cursor position. Clamped against this plan rather than trusted, because the
/// plan is rebuilt per frame at whatever width the pane has.
fn anchor_row(app: &App, plan: &Plan) -> usize {
    if app.focus() == Focus::Stack
        && let Some(row) = plan.row_of_comment(app.line_index(), app.comment_index())
    {
        return row;
    }
    app.cursor_row().min(plan.rows.len().saturating_sub(1))
}
