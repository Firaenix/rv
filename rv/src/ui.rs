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
//! │ sidebar (30%)│ diff (70%)                      │
//! └──────────────┴─────────────────────────────────┘
//! ```
//!
//! The bar carries the status line while browsing and becomes the comment box
//! while typing, rather than adding a fourth region: the two are never needed
//! at once, and a review is worth every row the diff can have.
//!
//! # Comment boxes
//!
//! A comment is drawn beneath the diff line it is anchored to, in a box made of
//! box-drawing characters *inside* the pane's own `Text` rather than as a
//! nested widget: a ratatui `Block` cannot nest inside a `Paragraph`, and
//! hand-drawn borders keep [`body`] a pure `state → Text` function that a
//! `TestBackend` can assert on cell by cell.
//!
//! Which rows those are is not decided here. [`crate::rows`] flattens "the
//! diff's lines plus their comments" into a list of drawable rows and windows
//! it, because a box is several rows tall and "the third diff line" therefore
//! stops being "the third row on screen" the moment a comment exists. This
//! module maps one row to one styled [`Line`] and does no arithmetic about
//! where a row sits.
//!
//! # Colour
//!
//! Blue means *comment*, and nothing else. Focus is therefore shown without
//! colour at all — a `▸` on the focused pane's title and a bold border — so
//! that the two never compete for the same cue. A comment that is no longer
//! open drops to grey and dim, which is the one deliberate exception: it is
//! still a comment, but not one asking for an answer.

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
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
use rv_core::store::Comment;
use rv_core::store::CommentState;

use crate::app::App;
use crate::app::Focus;
use crate::app::Mode;
use crate::app::SidebarTab;
use crate::rows::Plan;
use crate::rows::Row;
use crate::rows::plan;
use crate::rows::window;

/// Rows the comment box needs: its two borders and the line being typed.
const COMMENT_ROWS: u16 = 3;

/// Rows a bordered pane spends on its own borders. The same number of columns,
/// which is why it is used for both.
const BORDER_ROWS: u16 = 2;

/// Columns a diff line spends before its text starts: a five-wide number
/// field, a space, and the one-character sigil.
///
/// A comment box is indented by exactly this much, so it hangs off the line it
/// is about — under the code rather than under the line numbers.
const GUTTER: usize = 7;

/// Columns a comment box spends on itself per body row: `│`, a space, a space
/// and `│`.
const BOX_PADDING: usize = 4;

/// The marker a clipped row ends with.
///
/// A review tool that silently hides the code being judged is failing at its
/// one job: this repository contains 154-character lines, and the first real
/// session on `rv` read them in a 75-column pane with no sign that anything
/// had been cut. Diff lines are **not** wrapped instead — the row model is
/// built on one row per diff line, and a reviewer counting lines against a file
/// needs that correspondence — so they are marked.
const CLIPPED: char = '…';

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

/// What the Comments tab says when the review has no comments in it yet.
const NO_COMMENTS_YET: &str = "no comments yet";

/// Paints the whole reviewer.
pub fn draw(frame: &mut Frame, app: &App) {
    let bar_rows = match app.mode() {
        // A confirmation is a question in the status line, not a box to type
        // in, so it takes the same single row browsing does.
        Mode::Browse | Mode::ConfirmDelete { .. } => 1,
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
        // `ConfirmDelete` puts its question in the status line, so it draws
        // exactly as browsing does.
        Mode::Browse | Mode::ConfirmDelete { .. } => frame.render_widget(
            // Clipped with a marker like everything else: a status line is a
            // sentence, and half a sentence about a deletion is worse than
            // none.
            Paragraph::new(clip(app.status(), usize::from(area.width))),
            area,
        ),
        // The **tail** of the buffer, not its head: a `Paragraph` neither wraps
        // nor scrolls, so a comment longer than the bar used to be typed blind
        // from the character that reached the right-hand edge onwards.
        Mode::Comment => {
            let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
            frame.render_widget(
                Paragraph::new(tail(app.buffer(), width)).block(Block::bordered().title("Comment")),
                area,
            )
        }
    }
}

/// The left column: the review's files, or its comments.
///
/// One column with two tabs rather than two columns, because a reviewer moves
/// through comments the way they move through files — the same keys, in the
/// same place on screen — and because a review is worth every column the diff
/// can have.
fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus() == Focus::Sidebar;
    match app.sidebar_tab() {
        SidebarTab::Files => draw_files(frame, app, area, focused),
        SidebarTab::Comments => draw_comment_browser(frame, app, area, focused),
    }
}

/// The file list, one row per changed file, marked by how it changed.
fn draw_files(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let items: Vec<ListItem> = app
        .files()
        .iter()
        .map(|file| {
            ListItem::new(clip(
                &format!("{:<2} {}", marker(file.kind), file.path),
                width,
            ))
        })
        .collect();
    let list = List::new(items)
        .block(pane(format!("Files ({})", app.files().len()), focused))
        .highlight_style(selection_style(focused));

    let mut state = ListState::default()
        .with_selected(app.selected_file().is_some().then_some(app.file_index()));
    frame.render_stateful_widget(list, area, &mut state);
}

/// Every comment in the review, wherever it lives: `path:line`, its state, and
/// the first line of its body.
///
/// The reason it exists is arithmetic rather than taste. The first real session
/// on `rv` spent 2,200 of its 11,101 keystrokes on `j` and `]`, with one known
/// line costing 940 consecutive presses of `j`; `Enter` on a row here is that
/// same trip in one keystroke.
fn draw_comment_browser(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let block = pane(format!("Comments ({})", app.comments().len()), focused);
    if app.comments().is_empty() {
        frame.render_widget(Paragraph::new(NO_COMMENTS_YET).block(block), area);
        return;
    }

    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let items: Vec<ListItem> = app
        .comments()
        .iter()
        .map(|comment| {
            ListItem::new(Line::styled(
                clip(&summary(comment), width),
                comment_style(comment),
            ))
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(selection_style(focused));

    let mut state = ListState::default().with_selected(Some(app.browser_index()));
    frame.render_stateful_widget(list, area, &mut state);
}

/// One comment on one row: where it is, what state it is in, and the first line
/// of what it says.
fn summary(comment: &Comment) -> String {
    let first = comment.body.lines().next().unwrap_or_default();
    format!(
        "{}:{} {} {first}",
        comment.anchor.file,
        comment.anchor.line,
        state_name(comment.state),
    )
}

/// The selected file's diff, windowed so the highlighted line stays visible.
fn draw_diff(frame: &mut Frame, app: &App, area: Rect) {
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

    let block = pane(title(diff), focused);
    let height = usize::from(area.height.saturating_sub(BORDER_ROWS));
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let text = body(app, diff, width, height);
    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// A pane's block: bordered, titled, and marked when it holds the focus.
///
/// The mark is a `▸` and a bold border, never a colour — see the module docs.
fn pane(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered();
    if focused {
        block
            .title(format!("▸ {title}"))
            .border_style(Style::default().add_modifier(Modifier::BOLD))
    } else {
        block.title(title)
    }
}

/// How a list marks its selected row: reversed while the list has the focus,
/// and a dim underline while it does not — so there is exactly one place on
/// screen the next keystroke will land.
fn selection_style(focused: bool) -> Style {
    if focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().add_modifier(Modifier::DIM | Modifier::UNDERLINED)
    }
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

/// The diff pane's contents: the visible window of rows, under a note where the
/// diff is suppressed, or the one sentence that explains why there are no lines
/// at all.
///
/// Rows, not lines. A comment box is several rows tall, so what fits in the
/// pane is decided over [`crate::rows`]'s flattened row list rather than over
/// the diff's own lines — otherwise a line with a comment on it would push the
/// highlight off the bottom of the pane while the window still believed it was
/// on screen.
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
fn body<'a>(app: &'a App, diff: &'a FileDiff, width: usize, height: usize) -> Text<'static> {
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
    let height = height.saturating_sub(usize::from(note));

    // What a wrapped body row may occupy: the pane, less the gutter the box
    // hangs off, less the box's own two borders and their padding.
    let text_width = width.saturating_sub(GUTTER + BOX_PADDING);
    let plan = plan(
        diff,
        &|index| app.comments_for_line(index),
        app.collapsed(),
        text_width,
    );
    let visible = window(plan.rows.len(), anchor_row(app, &plan), height);

    let mut lines: Vec<Line> = Vec::with_capacity(visible.len() + usize::from(note));
    if note {
        lines.push(Line::styled(
            SUPPRESSED_NOTE,
            Style::default().fg(Color::Yellow),
        ));
    }
    lines.extend(
        plan.rows[visible]
            .iter()
            .map(|row| draw_row(app, row, width)),
    );
    Text::from(lines)
}

/// The row the window is centred on: the selected comment's box while the
/// cursor is inside a stack, and the selected diff line otherwise.
///
/// A cursor that could scroll off the pane it is steering is a cursor the
/// reviewer cannot use, and inside a stack the thing being steered is the box
/// rather than the line it hangs off.
fn anchor_row(app: &App, plan: &Plan) -> usize {
    let line = app.line_index();
    if app.focus() == Focus::Stack
        && let Some(row) = plan.row_of_comment(line, app.comment_index())
    {
        return row;
    }
    plan.row_of_line(line).unwrap_or(0)
}

/// One row of the plan, as one styled line of the pane.
fn draw_row(app: &App, row: &Row<'_>, width: usize) -> Line<'static> {
    match row {
        Row::Diff { index, line } => diff_row(app, *index, line, width),
        Row::BoxTop { comment, .. } => {
            let style = box_style(app, comment);
            let heading = format!("─ {} ", label(comment));
            let rule = "─".repeat(box_width(width).saturating_sub(2 + heading.chars().count()));
            clip_spans(
                vec![Span::styled(
                    format!("{}╭{heading}{rule}╮", indent(width)),
                    style,
                )],
                width,
            )
        }
        Row::BoxBody { comment, text, .. } => {
            let style = box_style(app, comment);
            let pad = box_width(width).saturating_sub(BOX_PADDING + text.chars().count());
            clip_spans(
                vec![
                    Span::styled(format!("{}│ ", indent(width)), style),
                    // The body keeps the terminal's own foreground: it is the
                    // part being *read*, and the border already says whose it
                    // is.
                    Span::raw(text.clone()),
                    Span::styled(format!("{} │", " ".repeat(pad)), style),
                ],
                width,
            )
        }
        Row::BoxBottom { comment, .. } => {
            let style = box_style(app, comment);
            let rule = "─".repeat(box_width(width).saturating_sub(2));
            clip_spans(
                vec![Span::styled(format!("{}╰{rule}╯", indent(width)), style)],
                width,
            )
        }
        Row::BoxCollapsed { comment, .. } => {
            let style = box_style(app, comment);
            let first = comment.body.lines().next().unwrap_or_default();
            let text = format!("{}▸ {} — {first}", indent(width), label(comment));
            Line::styled(clip(&text, width), style)
        }
    }
}

/// One line of the diff: its number on the side a comment there would anchor
/// to, its sigil, and its text — clipped, with [`CLIPPED`] where there was more
/// of it than the pane could show.
fn diff_row(app: &App, index: usize, line: &DiffLine, width: usize) -> Line<'static> {
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
    Line::styled(
        clip(&format!("{number} {sigil}{}", line.text), width),
        style,
    )
}

/// A comment box's title: the id it is filed under and the state it is in, so
/// the box on screen and the entry in the store name each other.
fn label(comment: &Comment) -> String {
    format!("{} · {}", comment.id, state_name(comment.state))
}

/// A comment state's name, spelled the way the store serializes it.
fn state_name(state: CommentState) -> &'static str {
    match state {
        CommentState::Open => "open",
        CommentState::AwaitingVerification => "awaiting-verification",
        CommentState::Resolved => "resolved",
        CommentState::Outdated => "outdated",
    }
}

/// How a comment's box is drawn: blue while it is open, brighter and bold while
/// the cursor is on it, grey and dim once it is neither.
///
/// The last of the three is the only place a comment is not blue, and it is not
/// a second meaning for the colour: a resolved or outdated comment is still a
/// comment, but it is not one asking for an answer, and drawing it as loudly as
/// one that is would bury the review under its own history.
fn comment_style(comment: &Comment) -> Style {
    if comment.state == CommentState::Open {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)
    }
}

/// The same, plus the selection: the box the stack cursor is on is brighter and
/// bold, so `d` and `s` visibly have a target.
fn box_style(app: &App, comment: &Comment) -> Style {
    let selected = app
        .selected_comment()
        .is_some_and(|cursor| cursor.id == comment.id);
    if selected {
        Style::default()
            .fg(Color::LightBlue)
            .add_modifier(Modifier::BOLD)
    } else {
        comment_style(comment)
    }
}

/// The blank left of a comment box, so it hangs off its line's text rather than
/// off the pane's edge. Never wider than the pane.
fn indent(width: usize) -> String {
    " ".repeat(GUTTER.min(width))
}

/// How many columns a box has to draw itself in.
fn box_width(width: usize) -> usize {
    width.saturating_sub(GUTTER)
}

/// `text`, clipped to `width` columns with [`CLIPPED`] in place of the last one
/// when there was more of it.
///
/// By characters rather than by bytes: a clip that split a multi-byte character
/// would panic on the very comments this reviewer is meant to survive.
fn clip(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(width - 1).collect();
    clipped.push(CLIPPED);
    clipped
}

/// A styled row, clipped to `width` columns across all of its spans.
///
/// Plain truncation, with no marker: this is for the rows a box draws around
/// its own content, where the marker would be claiming that a border had been
/// cut short — which is true, and not something the reviewer can do anything
/// about. What gets marked is content: see [`clip`].
fn clip_spans(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut kept = Vec::with_capacity(spans.len());
    let mut room = width;
    for span in spans {
        if room == 0 {
            break;
        }
        let length = span.content.chars().count();
        if length <= room {
            room -= length;
            kept.push(span);
        } else {
            let head: String = span.content.chars().take(room).collect();
            room = 0;
            kept.push(Span::styled(head, span.style));
        }
    }
    Line::from(kept)
}

/// The last `width` characters of `text`.
///
/// The comment bar follows what is being typed rather than showing where the
/// comment started: a `Paragraph` does not scroll, and the head of a long body
/// is the half the reviewer has already read.
fn tail(text: &str, width: usize) -> String {
    let length = text.chars().count();
    text.chars().skip(length.saturating_sub(width)).collect()
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

/// The sidebar's one- or two-character mark for how a file changed.
fn marker(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "+",
        ChangeKind::Removed => "-",
        ChangeKind::Renamed => "->",
        ChangeKind::Modified => "~",
    }
}
