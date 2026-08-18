//! How a review looks: one function that paints an [`App`] onto a frame.
//!
//! Nothing here holds state or decides anything the state machine could decide
//! instead — [`draw`] takes `&App` rather than `&mut App` precisely so that it
//! cannot. Every frame is painted from scratch from the app's fields, which is
//! why `rv/tests/app.rs` can assert on a rendered frame with a `TestBackend`
//! and no terminal at all.
//!
//! The layout is two panes over a bar:
//!
//! ```text
//! ┌──────────────┬┬────────────────────────────────┐
//! │ sidebar (30%)││ diff (70%)                     │
//! ├──────────────┴┴────────────────────────────────┤
//! │ status (1 row) — or the comment box (3 rows)   │
//! └────────────────────────────────────────────────┘
//! ```
//!
//! The bar carries the status line while browsing and becomes the comment box
//! while typing, rather than adding a fourth region: the two are never needed
//! at once, and a review is worth every row the diff can have.
//!
//! **This module computes no rectangle of its own.** Every `Rect` comes from
//! [`crate::layout`], which hit-testing reads from too, so a click cannot land
//! somewhere other than what was drawn. The one thing this module decides about
//! the geometry is how many rows the bar wants, which it hands over as a
//! [`Chrome`] — see [`crate::layout::layout`] for why that is a number rather
//! than a [`Mode`].
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
//! There are two layers here and they are kept apart deliberately — **by
//! channel, not by hue**.
//!
//! **The chrome** — borders, comment boxes, the status line, the gutter — has
//! three colours with one meaning each: blue is a *comment*, green an
//! *addition*, red a *removal*. Focus is therefore shown without colour at all
//! — a `▸` on the focused pane's title and a bold border — so that the two
//! never compete for the same cue. A comment that is no longer open drops to
//! grey and dim, which is the one deliberate exception: it is still a comment,
//! but not one asking for an answer.
//!
//! **The code** inside the diff pane carries its own syntax colours, and they
//! are the *terminal's* rather than rv's: [`capture_colour`] emits only the 16
//! indexed ANSI colours, which every scheme redefines for itself, so a keyword
//! is whatever the reviewer's own theme calls magenta.
//!
//! An earlier version of this module kept the two apart by hue instead, banning
//! green, red and blue from the syntax palette because the chrome spends all
//! three in this very pane. That is the wrong axis, and it cost the mapping its
//! semantics: it pushed comments onto index 7 — the terminal's *white* — which
//! is the loudest thing on the screen on a dark scheme and invisible on a light
//! one, and is the defect a user reported. The colours are split by channel
//! instead, which is where they actually cannot collide:
//!
//! | Colour | Chrome owns | Code owns |
//! |---|---|---|
//! | green | the **background** wash on an added line, and the `+` in the gutter | a string literal, as a **foreground** on code text |
//! | red | the **background** wash on a removed line, and the `-` in the gutter | nothing |
//! | blue | a comment box's **border glyphs**, drawn on the box's own rows | a function name, as a foreground on code text |
//! | index 8 | nothing | a comment, and nothing else |
//!
//! A wash is a background and a syntax colour is a foreground, so the two never
//! contend for the same channel; a box's border is drawn on rows that hold no
//! code at all, and the gutter's sigil sits in the seven columns before a line's
//! text starts. So [`Capture::Function`] moved to blue and [`Capture::String`]
//! to green, which is what spec §6's table asks for, and neither can be read as
//! chrome: no cell carries both.
//!
//! Added and removed lines carry a **dim wash of the palette's own green and
//! red** ([`crate::gradient::ADDED`] and [`crate::gradient::REMOVED`], so the
//! diff and the sidebar's change bar cannot drift into two pairs), with the
//! syntax colours at full strength on top. The selected line is a *brighter*
//! wash rather than reversed video: reversing swaps foreground and background,
//! which on a tinted line puts the syntax colours into the wash and the wash
//! into the text, legible in neither direction.
//!
//! Dim is the second axis, and it means *not the thing being asked about*: the
//! comment above, a `reply:` inside a box, which is the agent's answer rather
//! than the reviewer's remark, and a key in the `?` popup that would do nothing
//! from where the cursor is. Nothing is hidden — a reply is part of the
//! conversation the box holds, and a key a reviewer cannot find is a key they
//! do not have — but none of them competes with what is still waiting on
//! somebody.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::Clear;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::highlight::Capture;
use rv_core::highlight::Highlights;
use rv_core::model::ChangeKind;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use crate::app::App;
use crate::app::BINDINGS;
use crate::app::Binding;
use crate::app::Focus;
use crate::app::Group;
use crate::app::Mode;
use crate::app::SidebarTab;
use crate::app::anchored_side;
use crate::gradient;
use crate::layout::Chrome;
use crate::layout::Split;
use crate::layout::layout;
use crate::rows::BodyKind;
use crate::rows::Plan;
use crate::rows::Row;
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

/// What the diff pane's title adds for a file rv ships no grammar for.
///
/// Said out loud rather than left to the reviewer to infer from a screen of
/// white text: a tool that presents "I could not" as "there was nothing to
/// find" is guessing on the reader's behalf.
const NO_GRAMMAR: &str = " — no highlighting";

/// How far an added or removed line's tint is taken toward [`gradient::INK_DARK`].
///
/// Far: the wash marks a line, it does not shout at one, and the syntax colours
/// painted on top of it need the contrast more than the tint does.
const WASH: f32 = 0.74;

/// The same for the selected line, which is the same hue a step brighter — see
/// the module docs for why it is not `REVERSED`.
const WASH_SELECTED: f32 = 0.50;

/// The selected line's tint where the line is neither added nor removed. A
/// context line carries no hue of its own, so the highlight is a neutral band
/// off [`gradient::INK_LIGHT`] rather than a colour that would claim one.
const WASH_SELECTED_CONTEXT: f32 = 0.78;

/// Columns between the key column and its description inside the `?` popup,
/// and between one column of the popup and the next.
const HELP_GAP: usize = 2;

/// Paints the whole reviewer.
///
/// Every rectangle comes from [`layout`]; nothing is computed here. That is
/// what makes a click land on what the reviewer can see — [`crate::layout::hit`]
/// reads the same `Layout` this paints from.
pub fn draw(frame: &mut Frame, app: &App) {
    let rects = layout(frame.area(), app.split(), chrome(app));

    draw_bar(frame, app, rects.bar);
    draw_sidebar(frame, app, rects.sidebar);
    draw_diff(frame, app, rects.diff);
    // Last, and only if the layout gave it room: it is drawn *over* the panes,
    // which is why `hit` tests it first.
    if let Some(popup) = rects.popup {
        draw_help(frame, app, popup);
    }
}

/// What the layout needs to know about the frame being painted.
///
/// The bar's height is the only thing a [`Mode`] decides about the geometry, so
/// it is the only thing that crosses over.
fn chrome(app: &App) -> Chrome {
    Chrome {
        bar_rows: match app.mode() {
            // A confirmation is a question in the status line, not a box to
            // type in, so it takes the same single row browsing does.
            Mode::Browse | Mode::ConfirmDelete { .. } => 1,
            Mode::Comment => COMMENT_ROWS,
        },
        help_open: app.help_open(),
        toast: false,
    }
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

    let highlighting = Highlighting::of(app);
    let block = pane(title(diff, highlighting.language()), focused);
    let text = body(app, highlighting, diff, area);
    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// The diff pane's row plan and the slice of it that is on screen, for a pane
/// drawn in `pane`.
///
/// `pane` is the whole rectangle, borders included — the very `Rect` [`layout`]
/// hands [`draw_diff`] — so no caller repeats the border arithmetic and none
/// can ask about a pane that was never drawn.
///
/// Public because this is the one place the answer is decided, and because the
/// defect it exists to prevent is precisely a window and a cursor disagreeing
/// about which rows exist: a caller (or a test) that computed its own would be
/// asserting about a third thing neither the pane nor the keyboard uses.
///
/// It also **reports the width the boxes were drawn at** back to `app` — see
/// [`App::note_body_width`]. The renderer is the only thing that knows how wide
/// a comment box really is, and the row cursor is an index into rows whose
/// count depends on it.
#[must_use]
pub fn visible(app: &App, pane: Rect) -> (Plan<'_>, Range<usize>) {
    let width = usize::from(pane.width.saturating_sub(BORDER_ROWS));
    let height = usize::from(pane.height.saturating_sub(BORDER_ROWS));
    app.note_body_width(width.saturating_sub(GUTTER + BOX_PADDING));

    let plan = app.plan();
    // The suppressed note takes a row from the window, and only where there is
    // one to take — see [`body`].
    let note = app
        .selected_diff()
        .is_some_and(|diff| diff.suppressed && height >= 2);
    let height = height.saturating_sub(usize::from(note));
    let rows = window(plan.rows.len(), anchor_row(app, &plan), height);
    (plan, rows)
}

/// The two blobs' highlight spans for the file being drawn, one per side,
/// fetched once per frame.
///
/// Per frame rather than per row because [`App::highlights`] resolves a
/// `(commit, path)` key, and a diff pane is forty rows: asking once and handing
/// the answer down the row loop is the difference between two lookups a frame
/// and eighty.
#[derive(Clone, Copy)]
struct Highlighting<'a> {
    left: Option<&'a Highlights>,
    right: Option<&'a Highlights>,
}

impl<'a> Highlighting<'a> {
    fn of(app: &'a App) -> Self {
        Self {
            left: app.highlights(Side::Left),
            right: app.highlights(Side::Right),
        }
    }

    /// The spans for `line`, taken from the blob on **the side the line is
    /// anchored to** and looked up at that side's own number.
    ///
    /// [`anchored_side`] is asked here and nowhere else in this module, and
    /// there is deliberately no fallback to the other side's number: a removed
    /// line looked up at its head-side number would be painted with the colours
    /// of whatever now stands there, which for a rewrite in place is a
    /// different token of the same width — a lie told in a colour rather than
    /// in words, and invisible to any test whose fixture renames the file.
    fn spans(&self, line: &DiffLine) -> &'a [rv_core::highlight::Span] {
        let side = anchored_side(line.kind);
        let (highlights, number) = match side {
            Side::Left => (self.left, line.left),
            Side::Right => (self.right, line.right),
        };
        match (highlights, number) {
            (Some(highlights), Some(number)) => highlights.line(number),
            _ => &[],
        }
    }

    /// What the pane's title calls the grammar in use, or `None` where there is
    /// none to name.
    ///
    /// The head side answers where it can, because that is the version of the
    /// file the review is about; a deleted file has only a base side, and is
    /// named by it rather than reported as plain.
    fn language(&self) -> Option<&'static str> {
        self.right
            .and_then(Highlights::language)
            .or_else(|| self.left.and_then(Highlights::language))
    }
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

/// What the diff pane calls itself: the path, where its lines came from — so a
/// fallback diff is never mistaken for difftastic's structural one — and, where
/// rv ships no grammar for the file, that its code is plain because of that
/// rather than because there was nothing to colour.
fn title(diff: &FileDiff, language: Option<&'static str>) -> String {
    let source = match &diff.source {
        DiffSource::Difftastic { language } => format!("{} — difftastic ({language})", diff.path),
        DiffSource::Similar => format!("{} — fallback", diff.path),
        DiffSource::Binary => format!("{} — binary", diff.path),
    };
    match language {
        // A binary file needs no second sentence about why it is not coloured:
        // it is not shown by line at all, and the title already says so.
        Some(_) => source,
        None if diff.source == DiffSource::Binary => source,
        None => format!("{source}{NO_GRAMMAR}"),
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
fn body<'a>(
    app: &'a App,
    highlighting: Highlighting<'a>,
    diff: &'a FileDiff,
    pane: Rect,
) -> Text<'static> {
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

    let width = usize::from(pane.width.saturating_sub(BORDER_ROWS));
    let height = usize::from(pane.height.saturating_sub(BORDER_ROWS));
    let note = diff.suppressed && height >= 2;
    let (plan, rows) = visible(app, pane);

    // Asked once and handed down the row loop rather than per row: it is
    // derived from the row cursor over this very plan, and a forty-row pane
    // would otherwise rebuild the plan forty times to paint one frame.
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

/// The row the window is centred on: the selected comment's box while the
/// cursor is inside a stack, and the **row cursor** otherwise.
///
/// A cursor that could scroll off the pane it is steering is a cursor the
/// reviewer cannot use, and inside a stack the thing being steered is the box
/// rather than the line it hangs off.
///
/// Outside a stack it is [`App::cursor_row`] and nothing derived from it. That
/// is the fix for the defect in spec §10: this used to anchor on
/// `row_of_line(line_index())`, so the anchor could only ever rest on a *diff*
/// row, and the rows of a box taller than the pane were in no window at any
/// cursor position. The cursor and the anchor are now one number.
///
/// Clamped against this plan rather than trusted, because the plan is rebuilt
/// per frame at whatever width the pane has and the cursor was last clamped
/// against the previous one.
fn anchor_row(app: &App, plan: &Plan) -> usize {
    if app.focus() == Focus::Stack
        && let Some(row) = plan.row_of_comment(app.line_index(), app.comment_index())
    {
        return row;
    }
    app.cursor_row().min(plan.rows.len().saturating_sub(1))
}

/// The width a comment box's text is wrapped at before any frame has been
/// drawn.
///
/// The renderer is the only thing that knows how wide a box really is, and it
/// reports that after every frame — see [`visible`]. This is what [`App`]
/// assumes until the first one: an 80-column terminal at the default split,
/// less the pane's borders, the gutter a box hangs off and the box's own frame.
/// The narrowest terminal anyone reviews in, so the first frame can only widen
/// a box rather than narrow it.
#[must_use]
pub fn default_body_width() -> usize {
    let rects = layout(
        Rect::new(0, 0, 80, 24),
        Split::default(),
        Chrome {
            bar_rows: 1,
            help_open: false,
            toast: false,
        },
    );
    usize::from(rects.diff.width.saturating_sub(BORDER_ROWS)).saturating_sub(GUTTER + BOX_PADDING)
}

/// One row of the plan, as one styled line of the pane.
///
/// `selected` is the diff line the row cursor is on — the line that owns the
/// row under the cursor — passed down rather than asked per row; see [`body`].
fn draw_row(
    app: &App,
    highlighting: Highlighting<'_>,
    row: &Row<'_>,
    selected: usize,
    width: usize,
) -> Line<'static> {
    match row {
        Row::Diff { index, line } => diff_row(highlighting, *index, line, selected, width),
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
        Row::BoxBody {
            comment,
            text,
            kind,
            ..
        } => {
            let style = box_style(app, comment);
            let pad = box_width(width).saturating_sub(BOX_PADDING + text.chars().count());
            clip_spans(
                vec![
                    Span::styled(format!("{}│ ", indent(width)), style),
                    // The body keeps the terminal's own foreground: it is the
                    // part being *read*, and the border already says whose it
                    // is.
                    Span::styled(text.clone(), body_style(*kind)),
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
/// to, its sigil, and its text — washed by what kind of line it is, syntax
/// coloured on top, and clipped with [`CLIPPED`] where there was more of it
/// than the pane could show.
///
/// The wash goes on every cell of the row, not only the ones with text on them,
/// so an added line reads as a band rather than as a ragged edge.
fn diff_row(
    highlighting: Highlighting<'_>,
    index: usize,
    line: &DiffLine,
    selected_line: usize,
    width: usize,
) -> Line<'static> {
    let selected = index == selected_line;
    // The gutter keeps the kind's hue and takes the bright version of it on the
    // selected row: the same green on the brighter green band is a `+` a
    // reviewer has to look for, and the sigil is the one part of the row that
    // still says *added* on a terminal that renders no background at all.
    let (sigil, colour) = match (line.kind, selected) {
        (LineKind::Added, false) => ('+', Color::Green),
        (LineKind::Added, true) => ('+', Color::LightGreen),
        (LineKind::Removed, false) => ('-', Color::Red),
        (LineKind::Removed, true) => ('-', Color::LightRed),
        (LineKind::Context, false) => (' ', Color::Gray),
        (LineKind::Context, true) => (' ', Color::White),
    };
    let number = match line_number(line) {
        Some(number) => format!("{number:>5}"),
        None => " ".repeat(5),
    };

    let ground = match line_background(line.kind, selected) {
        Some(background) => Style::default().bg(background),
        None => Style::default(),
    };
    let mut spans = vec![Span::styled(format!("{number} {sigil}"), ground.fg(colour))];
    spans.extend(highlighted(&line.text, highlighting.spans(line), ground));
    clip_row(spans, width, ground)
}

/// `text` cut into styled spans by `highlights`, with the gaps between them
/// left on the terminal's own foreground.
///
/// The spans were measured against the *blob* line, and `text` is the diff's
/// rendering of it, which is close to but need not be byte-for-byte the same —
/// difftastic does its own thing with whitespace. So every offset is clamped to
/// `text` and walked back to a character boundary before it is used, and a span
/// that clamps to nothing is dropped. A span list that arrived out of order
/// could not make this slice backwards either: an offset behind where the walk
/// has already reached is skipped.
fn highlighted(
    text: &str,
    highlights: &[rv_core::highlight::Span],
    ground: Style,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut at = 0usize;
    for span in highlights {
        let start = boundary(text, span.start as usize);
        let end = boundary(text, span.end as usize);
        if start < at || end <= start {
            continue;
        }
        if start > at {
            spans.push(Span::styled(text[at..start].to_owned(), ground));
        }
        spans.push(Span::styled(
            text[start..end].to_owned(),
            ground.fg(capture_colour(span.capture)),
        ));
        at = end;
    }
    if at < text.len() {
        spans.push(Span::styled(text[at..].to_owned(), ground));
    }
    spans
}

/// The largest index `at or below` `at` that is a character boundary of `text`.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The background a diff line of this kind is drawn on, or `None` where it is
/// left on the terminal's own ground.
///
/// Public because it is the one place the answer is decided: the renderer
/// paints from it, and anything asking "which row is the selected one" reads
/// from it rather than keeping a second copy of the palette. `selected` is a
/// *brighter* version of the same hue rather than `REVERSED` — see the module
/// docs.
///
/// The hues are [`gradient::ADDED`] and [`gradient::REMOVED`] themselves, taken
/// toward the ink, so the diff pane and the sidebar's change bar cannot end up
/// with two greens and two reds that drift.
#[must_use]
pub fn line_background(kind: LineKind, selected: bool) -> Option<Color> {
    let (hue, wash) = match (kind, selected) {
        (LineKind::Added, false) => (gradient::ADDED, WASH),
        (LineKind::Added, true) => (gradient::ADDED, WASH_SELECTED),
        (LineKind::Removed, false) => (gradient::REMOVED, WASH),
        (LineKind::Removed, true) => (gradient::REMOVED, WASH_SELECTED),
        // A context line is not a change, so it carries no hue at all — and the
        // highlight on one is a neutral band rather than a colour that would
        // claim it was.
        (LineKind::Context, false) => return None,
        (LineKind::Context, true) => (gradient::INK_LIGHT, WASH_SELECTED_CONTEXT),
    };
    let gradient::Rgb(red, green, blue) = gradient::oklab_mix(hue, gradient::INK_DARK, wash);
    Some(Color::Rgb(red, green, blue))
}

/// The foreground one kind of source token is painted with.
///
/// Public so a test can name a colour without copying this table.
///
/// Every value here is one of the **16 indexed ANSI colours**, which are a
/// pass-through to the reviewer's own scheme rather than a palette rv chose:
/// emit index 4 and the terminal substitutes whatever *its* theme calls blue,
/// so a Solarized user gets Solarized and a Gruvbox user gets Gruvbox, in rv as
/// in every other tool they run. An `Rgb` value would do the opposite — dictate
/// an exact colour and ignore the scheme — which is what makes a syntax theme
/// something a user then has to configure. rv should never need a theme option,
/// because rv should never be the thing deciding. See the module docs for which
/// layer owns which colour, and `rv/tests/app.rs`'s
/// `code_is_painted_only_in_indexed_colours` for the boundary asserted in cells.
///
/// The mapping is semantic rather than chromatic (spec §6):
///
/// | Capture | Index | |
/// |---|---|---|
/// | Comment | 8, bright black | the muted tone every scheme defines against its own background |
/// | Keyword | 5, magenta | |
/// | Function | 4, blue | |
/// | Type | 6, cyan | |
/// | String | 2, green | |
/// | Number, Constant | 3, yellow | |
/// | Punctuation, Variable, Other | default | unstyled, so they inherit the terminal's own foreground |
///
/// [`Capture::Comment`] is the one that had to change: it was index 7, the
/// terminal's *white*, which is as loud as the code it annotates on a dark
/// scheme and near-invisible on a light one. Index 8 is the tone every scheme
/// defines for exactly this, and it now means one thing in this pane and one
/// only, because [`Capture::Punctuation`] gave it up.
///
/// [`Capture::Punctuation`], [`Capture::Variable`] and [`Capture::Other`] keep
/// the terminal's own foreground: most of a line is one of the three, and
/// colouring the majority of the text is how a highlighter stops being a
/// highlighter.
#[must_use]
pub fn capture_colour(capture: Capture) -> Color {
    match capture {
        Capture::Keyword => Color::Magenta,
        Capture::Function => Color::Blue,
        Capture::Type => Color::Cyan,
        Capture::String => Color::Green,
        // tree-sitter-rust reports integer and float literals as
        // `constant.builtin`, so Rust numbers arrive as `Constant`; the two
        // share a colour because they are the same thing to a reader.
        Capture::Number | Capture::Constant => Color::Yellow,
        Capture::Comment => Color::DarkGray,
        Capture::Punctuation | Capture::Variable | Capture::Other => Color::Reset,
    }
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

/// How a row of a box's text is drawn: the reviewer's own words at the
/// terminal's full contrast, an answer folded in from the export dimmed.
///
/// Dim rather than a colour, for the same reason focus is not a colour: blue
/// means *comment* here and a second hue would be a second meaning for it. A
/// reply is still part of the comment — it shares the box, and the box says
/// whose it is — so what the reply needs is to be *quieter* than the remark it
/// answers, which is the one thing a reviewer scanning a screen of boxes is
/// looking for.
fn body_style(kind: BodyKind) -> Style {
    match kind {
        BodyKind::Body => Style::default(),
        BodyKind::Reply => Style::default().add_modifier(Modifier::DIM),
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

/// A diff row, fitted to exactly `width` columns: padded with `ground` where
/// there was room to spare, and cut with a [`CLIPPED`] marker where there was
/// not.
///
/// Both halves matter. The padding is what makes a tinted line read as a band
/// across the pane instead of stopping wherever its text does. The marker is
/// [`clip`]'s promise kept for a row that is now several spans rather than one
/// string: a review tool that silently hides the code being judged is failing
/// at its one job.
fn clip_row(spans: Vec<Span<'static>>, width: usize, ground: Style) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let total: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    if total <= width {
        let mut spans = spans;
        spans.push(Span::styled(" ".repeat(width - total), ground));
        return Line::from(spans);
    }

    // One column is kept back for the marker, which inherits the style of
    // whichever span it cut into so that it reads as part of the text rather
    // than as a glyph the pane added on its own.
    let mut kept: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 1);
    let mut room = width - 1;
    let mut marker = ground;
    for span in spans {
        let length = span.content.chars().count();
        if length <= room {
            room -= length;
            marker = span.style;
            kept.push(span);
            continue;
        }
        let head: String = span.content.chars().take(room).collect();
        marker = span.style;
        if !head.is_empty() {
            kept.push(Span::styled(head, span.style));
        }
        break;
    }
    kept.push(Span::styled(CLIPPED.to_string(), marker));
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

// ---------------------------------------------------------------------------
// The `?` keymap
// ---------------------------------------------------------------------------

/// One row of the popup: a group's heading, or one binding.
enum HelpRow {
    Heading(&'static str),
    Key {
        binding: &'static Binding,
        enabled: bool,
    },
}

/// The whole keymap, drawn over the panes.
///
/// Drawn from [`BINDINGS`] rather than from a list of its own, which is what
/// makes "a binding that exists cannot be undocumented" true rather than
/// aspirational: there is no second table to forget to update.
fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let height = usize::from(area.height.saturating_sub(BORDER_ROWS));
    let text = help_text(app, width, height);
    // The popup covers what is under it rather than blending with it: a keymap
    // read through a diff is a keymap read twice.
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).block(Block::bordered().title("▸ Keys — ? or Esc to close")),
        area,
    );
}

/// The keymap laid out in as many columns as `width` fits.
///
/// One column of twenty-one rows does not fit the fourteen a 70%-of-24-rows
/// popup has, and 80x24 is what a reviewer over ssh actually has — so the
/// columns are not decoration. The narrowest number of rows that fits in the
/// columns available is chosen, and a group is never split across a column
/// boundary: a heading with nothing under it teaches nothing.
///
/// A popup too small for even that falls back to a single scrolling column, and
/// `scroll` is the only place [`App::help_scroll`] is used.
fn help_text(app: &App, width: usize, height: usize) -> Text<'static> {
    let blocks = help_blocks(app);
    let keys = BINDINGS
        .iter()
        .map(|binding| binding.keys.chars().count())
        .max()
        .unwrap_or(0);
    let what = BINDINGS
        .iter()
        .map(|binding| binding.what.chars().count())
        .max()
        .unwrap_or(0);
    let column = keys + HELP_GAP + what;
    // `(width + gap) / (column + gap)`: n columns need n-1 gaps between them.
    let columns = ((width + HELP_GAP) / (column + HELP_GAP)).max(1);

    let packed = (1..=height)
        .find_map(|rows| pack(&blocks, rows).filter(|packing| packing.len() <= columns));
    let packed = packed.unwrap_or_else(|| scrolled(&blocks, height, app.help_scroll()));

    let rows = packed.iter().map(Vec::len).max().unwrap_or(0);
    let lines = (0..rows)
        .map(|row| {
            let mut spans = Vec::with_capacity(packed.len() * 3);
            for (index, cells) in packed.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw(" ".repeat(HELP_GAP)));
                }
                spans.extend(help_cell(cells.get(row), keys, what));
            }
            clip_spans(spans, width)
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

/// One cell of the popup's grid, padded to the column's width so the ones under
/// it line up.
fn help_cell(row: Option<&&HelpRow>, keys: usize, what: usize) -> Vec<Span<'static>> {
    let column = keys + HELP_GAP + what;
    match row {
        None => vec![Span::raw(" ".repeat(column))],
        Some(HelpRow::Heading(heading)) => vec![Span::styled(
            format!("{heading:<column$}"),
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )],
        Some(HelpRow::Key { binding, enabled }) => {
            // Dim rather than hidden: a reviewer should see that the key exists
            // and that here is the wrong place for it — see the module docs.
            let (key_style, what_style) = if *enabled {
                (
                    Style::default().add_modifier(Modifier::BOLD),
                    Style::default(),
                )
            } else {
                let dim = Style::default().add_modifier(Modifier::DIM);
                (dim, dim)
            };
            vec![
                Span::styled(format!("{:<keys$}", binding.keys), key_style),
                Span::raw(" ".repeat(HELP_GAP)),
                Span::styled(format!("{:<what$}", binding.what), what_style),
            ]
        }
    }
}

/// The keymap as one block per [`Group`]: its heading, then its bindings in
/// table order.
fn help_blocks(app: &App) -> Vec<Vec<HelpRow>> {
    Group::ALL
        .iter()
        .map(|group| {
            let mut rows = vec![HelpRow::Heading(group.heading())];
            rows.extend(
                BINDINGS
                    .iter()
                    .filter(|binding| binding.group == *group)
                    .map(|binding| HelpRow::Key {
                        binding,
                        enabled: app.binding_enabled(binding),
                    }),
            );
            rows
        })
        .filter(|rows| rows.len() > 1)
        .collect()
}

/// Deals `blocks` into columns of at most `rows` rows each, keeping every block
/// whole. `None` when some block is taller than a column can be.
fn pack(blocks: &[Vec<HelpRow>], rows: usize) -> Option<Vec<Vec<&HelpRow>>> {
    if rows == 0 {
        return None;
    }
    let mut columns: Vec<Vec<&HelpRow>> = vec![Vec::new()];
    for block in blocks {
        if block.len() > rows {
            return None;
        }
        let last = columns.last_mut().expect("there is always one column");
        if last.len() + block.len() > rows {
            columns.push(Vec::new());
        }
        columns
            .last_mut()
            .expect("there is always one column")
            .extend(block.iter());
    }
    Some(columns)
}

/// The fallback for a popup too small to hold the keymap however it is dealt:
/// one column, `height` rows of it, starting `scroll` rows in.
///
/// Clamped here rather than in [`App`], which deliberately knows nothing about
/// how big the terminal is: holding `j` down cannot scroll past the end.
fn scrolled(blocks: &[Vec<HelpRow>], height: usize, scroll: usize) -> Vec<Vec<&HelpRow>> {
    let flat: Vec<&HelpRow> = blocks.iter().flatten().collect();
    let start = scroll.min(flat.len().saturating_sub(height));
    let end = start.saturating_add(height).min(flat.len());
    vec![flat[start..end].to_vec()]
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
