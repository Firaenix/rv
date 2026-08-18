//! How a review looks: one function that paints an [`App`] onto a frame.
//!
//! Nothing here holds state or decides anything the state machine could decide
//! instead — [`draw`] takes `&App` rather than `&mut App` precisely so that it
//! cannot. Every frame is painted from scratch from the app's fields, which is
//! why `rv/tests/app.rs` can assert on a rendered frame with a `TestBackend`
//! and no terminal at all.
//!
//! The layout is a bar over two panes:
//!
//! ```text
//! ┌────────────────────────────────────────────────┐
//! │ status (1 row) — or the comment box (3 rows)   │
//! ├──────────────┬─────────────────────────────────┤
//! │ files (30%)  │ diff (70%)                      │
//! └──────────────┴─────────────────────────────────┘
//! ```
//!
//! The bar carries the status line while browsing and becomes the comment box
//! while typing, rather than adding a fourth region: the two are never needed
//! at once, and a review is worth every row the diff can have.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::model::ChangeKind;
use rv_core::model::Side;

use crate::app::App;
use crate::app::Mode;

/// Rows the comment box needs: its two borders and the line being typed.
const COMMENT_ROWS: u16 = 3;

/// Rows a bordered pane spends on its own borders.
const BORDER_ROWS: u16 = 2;

/// What the diff pane says about a diff [`rv_core::diff`] suppressed and gave
/// no lines: difftastic's `unchanged` status, which emits no chunks, so there
/// is nothing to put the sentence above.
const SUPPRESSED_EMPTY: &str = "no semantic change";

/// The same, as a header over a suppressed diff that *does* have lines — the
/// `similar` fallback's terminator-only change, whose difference is real but
/// lives where no line's `text` can show it.
///
/// A note rather than a replacement, because the reviewer can put the highlight
/// on those lines and comment on them: a pane that swallowed them would let
/// `j`/`k` walk through rows it never drew and let a comment land on one of
/// them. The wording starts with [`SUPPRESSED_EMPTY`] so both branches read the
/// same way.
const SUPPRESSED_NOTE: &str = "no semantic change — the difference is not visible below";

/// Paints the whole reviewer.
pub fn draw(frame: &mut Frame, app: &App) {
    let bar_rows = match app.mode() {
        Mode::Browse => 1,
        Mode::Comment => COMMENT_ROWS,
    };
    let [bar, panes] =
        Layout::vertical([Constraint::Length(bar_rows), Constraint::Min(0)]).areas(frame.area());
    let [sidebar, diff] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).areas(panes);

    draw_bar(frame, app, bar);
    draw_sidebar(frame, app, sidebar);
    draw_diff(frame, app, diff);
}

/// The status line, or the comment being typed.
fn draw_bar(frame: &mut Frame, app: &App, area: Rect) {
    match app.mode() {
        Mode::Browse => frame.render_widget(Paragraph::new(app.status()), area),
        Mode::Comment => frame.render_widget(
            Paragraph::new(app.buffer()).block(Block::bordered().title("Comment")),
            area,
        ),
    }
}

/// The file list, one row per changed file, marked by how it changed.
fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .files()
        .iter()
        .map(|file| ListItem::new(format!("{:<2} {}", marker(file.kind), file.path)))
        .collect();
    let list = List::new(items)
        .block(Block::bordered().title(format!("Files ({})", app.files().len())))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default()
        .with_selected(app.selected_file().is_some().then_some(app.file_index()));
    frame.render_stateful_widget(list, area, &mut state);
}

/// The selected file's diff, windowed so the highlighted line stays visible.
fn draw_diff(frame: &mut Frame, app: &App, area: Rect) {
    let Some(file) = app.selected_file() else {
        frame.render_widget(
            Paragraph::new("no changed files in this range").block(Block::bordered().title("Diff")),
            area,
        );
        return;
    };
    let Some(diff) = app.selected_diff() else {
        // Only reachable if a file's blobs have not been read yet, which the
        // app does before this function is ever called for that file.
        frame.render_widget(
            Paragraph::new("no diff loaded").block(Block::bordered().title(file.path.clone())),
            area,
        );
        return;
    };

    let block = Block::bordered().title(title(diff));
    let height = area.height.saturating_sub(BORDER_ROWS) as usize;
    let text = body(app, diff, height);
    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// What the diff pane calls itself: the path, plus where its lines came from,
/// so a fallback diff is never mistaken for difftastic's structural one.
fn title(diff: &FileDiff) -> String {
    match &diff.source {
        DiffSource::Difftastic { language } => format!("{} — difftastic ({language})", diff.path),
        DiffSource::Similar => format!("{} — fallback", diff.path),
        DiffSource::Binary => format!("{} — binary", diff.path),
    }
}

/// The diff pane's contents: the visible window of lines, under a note where
/// the diff is suppressed, or the one sentence that explains why there are no
/// lines at all.
///
/// `suppressed` does not imply "no lines". It used to — it was set only from
/// difftastic's `unchanged` status, which emits no chunks — and this function
/// short-circuited on the flag accordingly. The `similar` fallback now also
/// sets it for a change that lives entirely in the line terminators, and
/// reports that change's lines as `Context`; short-circuiting on the flag there
/// showed a sentence in place of content [`App`] was still letting the reviewer
/// navigate through and comment on. The note goes *above* the lines instead, so
/// that what can be reached is what is drawn.
///
/// The note takes a row from the window, and only where there is one to take:
/// below two rows the lines win, since a pane that spent its only row on the
/// note would hide the highlight — which is the failure this branch exists to
/// avoid.
fn body<'a>(app: &App, diff: &'a FileDiff, height: usize) -> Text<'a> {
    if diff.source == DiffSource::Binary {
        return Text::from("binary file, not shown by line");
    }
    if diff.lines.is_empty() {
        return Text::from(if diff.suppressed {
            SUPPRESSED_EMPTY
        } else {
            "no lines to show"
        });
    }

    let note = diff.suppressed && height >= 2;
    let height = height - usize::from(note);
    let window = window(app.line_index(), diff.lines.len(), height);
    let mut lines: Vec<Line> = Vec::with_capacity(window.len() + usize::from(note));
    if note {
        lines.push(Line::styled(
            SUPPRESSED_NOTE,
            Style::default().fg(Color::Yellow),
        ));
    }
    lines.extend(
        diff.lines[window.clone()]
            .iter()
            .zip(window)
            .map(|(line, index)| {
                let (sigil, color) = match line.kind {
                    LineKind::Added => ('+', Color::Green),
                    LineKind::Removed => ('-', Color::Red),
                    LineKind::Context => (' ', Color::Gray),
                };
                let number = match line_number(line) {
                    Some(number) => format!("{number:>5}"),
                    None => " ".repeat(5),
                };
                let mut style = Style::default().fg(color);
                if index == app.line_index() {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                Line::styled(format!("{number} {sigil}{}", line.text), style)
            }),
    );
    Text::from(lines)
}

/// The line number to label a diff line with: the one on the side a comment
/// there would anchor to — `left` for a removed line, `right` otherwise.
///
/// Not `right.or(left)`: difftastic aligns a changed line with its counterpart
/// and gives the pair *both* numbers, so labelling a removed line by its
/// head-side number showed one number while [`App`] stored the base-side one,
/// and the status line then reported a third thing. The pane now says what the
/// anchor says.
///
/// The fallback to the other side is orientation only, for a line with no
/// number of its own: such a line cannot be commented on at all — the app
/// refuses rather than anchoring it somewhere approximate.
fn line_number(line: &DiffLine) -> Option<u32> {
    match crate::app::anchored_side(line.kind) {
        Side::Left => line.left.or(line.right),
        Side::Right => line.right.or(line.left),
    }
}

/// The half-open range of diff lines to draw: `height` of them, centered on
/// `selected` where the file is long enough to center anything.
fn window(selected: usize, total: usize, height: usize) -> Range<usize> {
    if height == 0 || total == 0 {
        return 0..0;
    }
    if total <= height {
        return 0..total;
    }
    let start = selected.saturating_sub(height / 2).min(total - height);
    start..start + height
}

/// The sidebar's one- or two-character mark for how a file changed.
fn marker(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "+",
        ChangeKind::Removed => "-",
        ChangeKind::Renamed => "->",
        ChangeKind::Modified => "~",
    }
}
