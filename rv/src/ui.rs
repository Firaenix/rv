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
//! ╭──────────────╮│╭───────────────────────────────╮
//! │ sidebar (30%)│││ diff (70%)                    │
//! ╰──────────────╯│╰───────────────────────────────╯
//!  status bar (1 row) — or the comment box (3 rows)
//! ```
//!
//! The bar carries the status bar while browsing and becomes the comment box
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
//! [`draw`] hands the `Layout` it painted back to [`App::note_layout`], and the
//! three questions a pointer asks about *what is inside* a pane — which plan row
//! is under this pane row ([`diff_row_at`]), which list entry
//! ([`sidebar_index_at`]), and where the wheel moves the view to
//! ([`diff_scrolled`], [`sidebar_scrolled`]) — are answered here as well, for
//! the same reason: the window's offset, the note above a suppressed diff and a
//! list's scroll are this module's arithmetic, and a hit test with its own copy
//! of any of them would resolve clicks against a screen that was never painted.
//!
//! # Time
//!
//! [`draw`] takes a `now`. The only thing on screen that ages is the alert
//! toast, whose border steps down in Oklab lightness over its last second, and
//! taking the instant as an argument is what keeps the renderer — like [`App`]
//! itself — free of the clock. See `rv/src/app.rs`'s module docs.
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
//! **The chrome** — borders, the file list, comment boxes, the status bar, the
//! gutter — spends one colour per meaning, and every one of them is declared in
//! [`crate::gradient`] so that no second meaning can be added quietly: blue is a
//! *comment*, green an *addition*, red a *removal*, orange an *alert*, and
//! magenta the *focused pane* and nothing else. A comment that is no longer open
//! drops to grey and dim, which is the one deliberate exception: it is still a
//! comment, but not one asking for an answer.
//!
//! Focus is shown three times over — the `▸` on the title, a bold border, and
//! the magenta — because the two cheap signals survive a sixteen-colour
//! terminal and a reader who does not separate magenta from red. Colour
//! enhances the mark; it never carries it alone.
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
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
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

use crate::app::Alert;
use crate::app::App;
use crate::app::BINDINGS;
use crate::app::Binding;
use crate::app::Focus;
use crate::app::Group;
use crate::app::Mode;
use crate::app::SidebarTab;
use crate::app::anchored_side;
use crate::gradient;
use crate::gradient::Rgb;
use crate::gradient::Stat;
use crate::layout::Chrome;
use crate::layout::Layout;
use crate::layout::Split;
use crate::layout::layout;
use crate::rows::BodyKind;
use crate::rows::Plan;
use crate::rows::Row;
use crate::rows::window;
use crate::statusbar;
use crate::tree;
use crate::tree::Node;
use crate::tree::NodeKind;

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

/// The fewest columns a file list row will show of a path before it gives up
/// its counts instead.
///
/// The path is the row's *identity* and the counts are context: a row reading
/// `+40 -0` and nothing else names no file, while a row reading `added.…` still
/// does. Eight columns is a short name and the clip marker.
const MIN_PATH_COLUMNS: usize = 8;

/// How many columns the change bar takes on a row wide enough to carry one.
///
/// Six, which is enough to read a proportion at a glance — a third against two
/// thirds is two cells against four — and few enough that a row only has to be
/// eight columns wider than its own name to earn one.
const BAR_COLUMNS: usize = 6;

/// The glyph the change bar is drawn with. Drawn as a **foreground**, on the
/// terminal's own ground: see [`change_bar`].
const BAR: char = '█';

/// The mark a file list row that holds others carries: pointing down when its
/// contents are shown, right when they are folded away.
///
/// Three columns wide, like the change marks beside it, so names line up down
/// the column whatever kind of row they are on.
const OPEN: &str = "▾  ";
/// See [`OPEN`].
const FOLDED: &str = "▸  ";

/// Paints the whole reviewer, as it stands at `now`.
///
/// Every rectangle comes from [`layout`]; nothing is computed here. That is
/// what makes a click land on what the reviewer can see — [`crate::layout::hit`]
/// reads the same `Layout` this paints from, and this is where that `Layout` is
/// handed to [`App::note_layout`] for it.
///
/// `now` is a parameter rather than a call to the clock for the same reason
/// nothing inside [`App`] reads one: the only thing on screen that ages is the
/// toast, and its fade being a function of an argument is what makes "it is dim
/// at four and a half seconds" an assertion rather than a sleep.
pub fn draw(frame: &mut Frame, app: &App, now: Instant) {
    let alerts: Vec<&Alert> = app.alerts().iter().filter(|a| a.live(now)).collect();
    let rects = layout(frame.area(), app.split(), chrome(app, !alerts.is_empty()));
    // Before anything is painted, so that a gesture arriving between this frame
    // and the next resolves against the geometry this frame had.
    app.note_layout(rects);

    draw_bar(frame, app, rects.bar);
    draw_sidebar(frame, app, rects.sidebar);
    draw_diff(frame, app, rects.diff);
    // Over the panes, and under the keymap: a reviewer who asked for the manual
    // is reading it, and an alert that covered it would be interrupting the one
    // thing they asked to see.
    if let Some(toast) = rects.toast {
        draw_toast(frame, &alerts, toast, now);
    }
    if let Some(popup) = rects.popup {
        draw_help(frame, app, popup);
    }
}

/// What the layout needs to know about the frame being painted.
///
/// The bar's height is the only thing a [`Mode`] decides about the geometry, so
/// it is the only thing that crosses over. `toast` is a `bool` for the same
/// reason: how many alerts there are does not change where the panel goes.
fn chrome(app: &App, toast: bool) -> Chrome {
    Chrome {
        bar_rows: match app.mode() {
            // A confirmation is a question in the status line, not a box to
            // type in, so it takes the same single row browsing does.
            Mode::Browse | Mode::ConfirmDelete { .. } => 1,
            Mode::Comment => COMMENT_ROWS,
        },
        help_open: app.help_open(),
        toast,
    }
}

/// The geometry of a frame nobody has painted yet: an 80x24 terminal at the
/// default split, browsing.
///
/// The narrowest terminal anyone reviews in, so what [`App`] assumes before its
/// first frame can only be *smaller* than what it gets. Two things read it: the
/// width a comment box wraps at ([`default_body_width`]) and the rectangles a
/// gesture resolves against, which is what makes a click arriving before the
/// first frame land somewhere plausible rather than nowhere.
#[must_use]
pub fn default_layout() -> Layout {
    layout(
        Rect::new(0, 0, 80, 24),
        Split::default(),
        Chrome {
            bar_rows: 1,
            help_open: false,
            toast: false,
        },
    )
}

/// The status bar, the confirmation being answered, or the comment being typed.
///
/// **Browsing draws [`crate::statusbar`]'s segments**, not `app.status()` across
/// the row. That is what fixes the defect the `?` popup was a workaround for:
/// the status used to *be* the bar, so the first `d` a reviewer pressed replaced
/// the keymap with `deleted comment at a.rs:42` and it never came back. As one
/// segment among six it can displace nothing, and it is the first thing dropped
/// when the terminal is narrow because it is the only part of the bar that stops
/// being true on its own.
///
/// **A confirmation is not a status message**, so it keeps the whole row. It is
/// a modal question whose answer destroys written work, and a question that
/// could be dropped for want of room is a question the reviewer answers blind.
/// It is clipped with a marker rather than dropped, for the same reason: half a
/// sentence about a deletion is worse than none, but it is far better than none
/// at all.
fn draw_bar(frame: &mut Frame, app: &App, area: Rect) {
    match app.mode() {
        Mode::Browse => {
            let view = status_view(app);
            frame.render_widget(
                Paragraph::new(statusbar::render(
                    &statusbar::segments(&view),
                    area.width,
                    app.ascii(),
                )),
                area,
            );
        }
        Mode::ConfirmDelete { .. } => frame.render_widget(
            Paragraph::new(clip(app.status(), usize::from(area.width))),
            area,
        ),
        // The **tail** of the buffer, not its head: a `Paragraph` neither wraps
        // nor scrolls, so a comment longer than the bar used to be typed blind
        // from the character that reached the right-hand edge onwards.
        Mode::Comment => {
            let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
            frame.render_widget(
                Paragraph::new(tail(app.buffer(), width)).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title("Comment"),
                ),
                area,
            )
        }
    }
}

/// What the status bar needs to know about the review, read off the app in one
/// place.
///
/// The bar takes plain data rather than an `&App` — see [`statusbar::View`] —
/// so this is the whole of the coupling between the two, and the bar stays
/// testable without a workspace.
///
/// `mode` is `BROWSE` and nothing else, because this is the only mode that
/// draws the bar: a comment box replaces it while typing and a confirmation
/// takes the row whole. Naming the *context* the cursor is in — `FILES`,
/// `DIFF`, `STACK` — is the next wave's, and the segment is here now so that
/// what a reviewer reads in the bar is a fact about the keyboard rather than
/// about which pane happened to draw last.
fn status_view(app: &App) -> statusbar::View<'_> {
    statusbar::View {
        mode: "BROWSE",
        file: app.selected_file().map(|file| file.path.as_str()),
        file_index: app.file_index(),
        file_count: app.files().len(),
        stat: app.selected_file().map(|_| app.stat(app.file_index())),
        scope: &app.session().revset,
        open_comments: app
            .comments()
            .iter()
            .filter(|comment| comment.state == CommentState::Open)
            .count(),
        status: app.status(),
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

/// The file list: one row per file, per directory that holds files, or per
/// change that touched them — marked by how it changed, counted, and where the
/// row has columns to spare, measured by a small change bar.
///
/// Every row comes from [`App::sidebar_nodes`], which is [`crate::tree`]'s
/// answer and the same list the cursor walks. Nothing here decides which rows
/// exist or what order they are in; this module turns one row into one line.
///
/// # Nothing here paints a background
///
/// The gradient does **not** wash the row. Spec §7 rules that out after two
/// rounds of looking at the running tool: a full-row wash reads as a selection
/// and competes with the real one, and even a text-width wash destroys what the
/// pane exists to show, because in tree mode the structure *is* the indentation
/// and the fold marks and neither survives being painted over. Thirty files
/// became thirty slabs of green and the tree stopped looking like a tree.
///
/// So the colour lives in the **counts**, as a foreground on the terminal's own
/// ground, and the proportion survives as [`change_bar`] — a mark on the row
/// rather than the row itself. The only full-row background in this pane is the
/// selection, which is therefore unambiguous.
///
/// # What goes when the pane is narrow
///
/// The bar first, then the counts, then the path is clipped. Each is more the
/// row's identity than the last.
///
/// The bar is decided for the **whole list** rather than per row, from the
/// longest name in it: a bar that appeared on the short rows and not the long
/// ones would be a ragged column, and one that appeared by clipping a name
/// would be buying context with identity. It is drawn only where every name
/// still fits beside it.
///
/// # The shape and the order go on the bottom border
///
/// The title already carries the focus mark and the count, and at 80 columns
/// the sidebar has twenty-one columns inside its borders — `▸ Files (2) · list
/// · natural` is twenty-eight, so putting the order up there would truncate
/// away exactly the thing it is there to say, on the terminal a reviewer over
/// ssh actually has. The bottom border is empty and is as much the pane's title
/// as the top one.
fn draw_files(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let nodes = app.sidebar_nodes();
    let heads: Vec<String> = nodes.iter().map(|node| head(app, node)).collect();
    // One counts column for the whole list, as wide as its widest entry, so the
    // names line up down the pane instead of ending wherever their own row's
    // numbers happened to start. Zero when nothing in the review changed a
    // line, which is when there is no column to reserve.
    let counted: Vec<(String, String)> = nodes.iter().map(|node| counts(node.stat)).collect();
    let counts_width = counted.iter().map(counts_columns).max().unwrap_or(0);
    let longest = heads
        .iter()
        .map(|head| head.chars().count())
        .max()
        .unwrap_or(0);
    let bar =
        usize::from(counts_width > 0 && width >= longest + 1 + BAR_COLUMNS + 1 + counts_width)
            * BAR_COLUMNS;

    let items: Vec<ListItem> = nodes
        .iter()
        .zip(&heads)
        .zip(&counted)
        .map(|((node, head), counts)| {
            ListItem::new(file_row(node, head, counts, counts_width, bar, width))
        })
        .collect();
    let list = List::new(items)
        .block(pane(format!("Files ({})", app.files().len()), focused).title_bottom(shape(app)))
        .highlight_style(selection_style(focused));

    let mut state = list_state(app, area, nodes.len(), app.sidebar_row());
    frame.render_stateful_widget(list, area, &mut state);
}

/// Which slice of a sidebar list is drawn, and whether the selection is in it.
///
/// The offset is **handed to the widget** rather than left to it. ratatui scrolls
/// a `List` far enough to keep its selected item visible, which is exactly right
/// while the view is following the selection and exactly wrong once the wheel
/// has parked it somewhere else: the widget would quietly scroll back, and the
/// row a click resolved to would not be the row that was drawn. So the offset
/// comes from [`list_offset`] — the same function hit-testing reads — and the
/// selection is passed only while it is inside that window, which is what stops
/// ratatui from moving it. A selection off screen is drawn nowhere, which is
/// what being scrolled away from it means.
fn list_state(app: &App, area: Rect, rows: usize, selected: usize) -> ListState {
    let height = usize::from(area.height.saturating_sub(BORDER_ROWS));
    let offset = list_offset(selected, rows, height, app.sidebar_scroll());
    let shown = (offset..offset.saturating_add(height)).contains(&selected);
    ListState::default()
        .with_offset(offset)
        .with_selected((rows > 0 && shown).then_some(selected))
}

/// What the file list says about itself along its bottom border: whether it is
/// a list or a tree, and what order its rows are in.
///
/// Said out loud rather than left to be inferred from the rows, because an
/// order you cannot see is an order you cannot trust: a reviewer who does not
/// know the list is sorted by additions reads its first row as "the first file"
/// rather than "the biggest change".
fn shape(app: &App) -> String {
    format!(
        " {} · {} ",
        if app.tree_view() { "tree" } else { "list" },
        app.sort().label()
    )
}

/// One row of the file list: its name on the left, and — right-aligned in a
/// column shared by the whole list — its change bar and its counts.
///
/// `bar` is `0` where the list gave the bar up, and the counts go with it when
/// even they would leave the name less than [`MIN_PATH_COLUMNS`].
fn file_row(
    node: &Node,
    head: &str,
    counts: &(String, String),
    counts_width: usize,
    bar: usize,
    width: usize,
) -> Line<'static> {
    let tail = if bar > 0 { bar + 1 } else { 0 } + counts_width;
    // One column of gap at least, always: a name clipped right up against its
    // own numbers reads as one word, which is how `docs/specs/…+10` happens.
    let names = width.saturating_sub(tail + 1);
    if counts_width == 0 || names < MIN_PATH_COLUMNS {
        return Line::from(clip(head, width));
    }

    let name = clip(head, names);
    let mut spans = vec![
        Span::raw(name.clone()),
        Span::raw(" ".repeat(names + 1 - name.chars().count())),
    ];

    let (added, removed) = counts;
    if added.is_empty() {
        // Nothing changed here, so the row says nothing rather than `+0 -0` and
        // draws no bar: zero is not a measurement, and a gradient over zero
        // lines would be inventing a ratio. It keeps the columns, so the rows
        // that do have numbers stay lined up.
        spans.push(Span::raw(" ".repeat(tail)));
        return Line::from(spans);
    }
    if bar > 0 {
        spans.extend(change_bar(node.stat, bar));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(" ".repeat(counts_width - counts_columns(counts))));
    spans.push(Span::styled(
        added.clone(),
        Style::default().fg(colour(gradient::ADDED)),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        removed.clone(),
        Style::default().fg(colour(gradient::REMOVED)),
    ));
    Line::from(spans)
}

/// A row's name, indent and change mark included, before anything is clipped.
fn head(app: &App, node: &Node) -> String {
    format!(
        "{}{}{}",
        "  ".repeat(node.depth),
        row_mark(app, node),
        node.label
    )
}

/// The proportion of a change, as `columns` cells of [`BAR`] running from
/// [`gradient::ADDED`] through [`gradient::pivot`]'s seam to
/// [`gradient::REMOVED`].
///
/// A **foreground**, not a wash: see [`draw_files`]. Consecutive cells of one
/// colour are one span, so a bar that is flat green is one span rather than six.
fn change_bar(stat: Stat, columns: usize) -> Vec<Span<'static>> {
    let Some(ratio) = stat.added_ratio() else {
        return vec![Span::raw(" ".repeat(columns))];
    };
    let width = u16::try_from(columns).unwrap_or(u16::MAX);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut ink: Option<Rgb> = None;
    for column in 0..columns {
        let colour_of =
            gradient::column_colour(ratio, u16::try_from(column).unwrap_or(u16::MAX), width);
        if ink != Some(colour_of) {
            if let Some(previous) = ink {
                spans.push(Span::styled(
                    std::mem::take(&mut run),
                    Style::default().fg(colour(previous)),
                ));
            }
            ink = Some(colour_of);
        }
        run.push(BAR);
    }
    if let Some(previous) = ink {
        spans.push(Span::styled(run, Style::default().fg(colour(previous))));
    }
    spans
}

/// The three columns a file list row spends on saying what kind of row it is:
/// how a file changed, or whether a row that holds others is open or folded.
fn row_mark(app: &App, node: &Node) -> String {
    match &node.kind {
        NodeKind::Dir { collapsed, .. } | NodeKind::Commit { collapsed, .. } => {
            if *collapsed { FOLDED } else { OPEN }.to_owned()
        }
        NodeKind::File { index } => match app.files().get(*index) {
            Some(file) => format!("{:<2} ", marker(file.kind)),
            // A row addressing a file the review does not have cannot happen —
            // the rows are built from that very list — and is drawn blank
            // rather than panicking a frame over it.
            None => " ".repeat(3),
        },
    }
}

/// What a row costs to review, as the two numbers the pane prints — added and
/// removed — or two empty strings where it cost no lines.
///
/// Two rather than one string because they are drawn in two colours: the added
/// count in [`gradient::ADDED`] and the removed one in [`gradient::REMOVED`],
/// which is where the sidebar's colour lives now that no row is washed.
///
/// Abbreviated by [`tree::abbreviate`], which is never wider than four
/// characters, so the counts cannot push the path out of a narrow column by
/// being long.
///
/// A row that changed no lines — a pure rename, a mode change — says nothing
/// rather than `+0 -0`: zero is not a measurement of anything.
fn counts(stat: Stat) -> (String, String) {
    if stat.total() == 0 {
        return (String::new(), String::new());
    }
    (
        format!("+{}", tree::abbreviate(stat.added)),
        format!("-{}", tree::abbreviate(stat.removed)),
    )
}

/// How many columns [`counts`]'s answer takes, the space between the two
/// numbers included.
fn counts_columns((added, removed): &(String, String)) -> usize {
    if added.is_empty() {
        return 0;
    }
    added.chars().count() + 1 + removed.chars().count()
}

/// One of [`crate::gradient`]'s colours, as ratatui sends it.
fn colour(Rgb(red, green, blue): Rgb) -> Color {
    Color::Rgb(red, green, blue)
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

    let mut state = list_state(app, area, app.comments().len(), app.browser_index());
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
    app.note_body_width(width.saturating_sub(GUTTER + BOX_PADDING));

    let plan = app.plan();
    let height = content_rows(app, pane);
    let total = plan.rows.len();
    let rows = window(total, anchor_row(app, &plan), height);
    (plan, parked(rows, total, app.diff_scroll()))
}

/// How many rows of the plan a diff pane of this size shows: its own, less its
/// borders, less the suppressed note where there is one to take — see [`body`].
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
/// window's, so a pane showing fewer rows than it has room for still shows
/// exactly those and never a row past the end of the plan.
fn parked(natural: Range<usize>, rows: usize, scroll: Option<usize>) -> Range<usize> {
    let Some(start) = scroll else {
        return natural;
    };
    let height = natural.len();
    let start = start.min(rows.saturating_sub(height));
    start..start.saturating_add(height)
}

/// Which row of the plan is under the `row`-th content row of a diff pane drawn
/// at `pane`, or `None` where that row holds no plan row at all.
///
/// The mouse's half of [`visible`], and the reason it is here rather than in
/// [`App`]: the note above a suppressed diff takes the pane's first row without
/// being a row of the plan, and the window's offset is this module's arithmetic.
/// A hit test with its own copy of either would resolve clicks against a screen
/// that was never painted.
#[must_use]
pub fn diff_row_at(app: &App, pane: Rect, row: usize) -> Option<usize> {
    let height = usize::from(pane.height.saturating_sub(BORDER_ROWS));
    let row = row.checked_sub(usize::from(suppressed_note(app, height)))?;
    let (_, rows) = visible(app, pane);
    let index = rows.start.checked_add(row)?;
    (index < rows.end).then_some(index)
}

/// The first row of the diff pane's plan on screen after `delta` rows of wheel,
/// clamped to the plan.
///
/// Answered here for the reason [`diff_row_at`] is: where the view is now is
/// this module's arithmetic, and the wheel moves it from there.
#[must_use]
pub fn diff_scrolled(app: &App, pane: Rect, delta: isize) -> usize {
    let (plan, rows) = visible(app, pane);
    let last = plan.rows.len().saturating_sub(rows.len());
    rows.start.saturating_add_signed(delta).min(last)
}

/// Which entry of the sidebar's list is under the `row`-th content row of a
/// sidebar drawn at `pane`, or `None` where the list has no such entry.
#[must_use]
pub fn sidebar_index_at(app: &App, pane: Rect, row: usize) -> Option<usize> {
    let (count, _, offset) = list_view(app, pane);
    let index = offset.checked_add(row)?;
    (index < count).then_some(index)
}

/// The same for the wheel: the first entry on screen after `delta` rows of it.
#[must_use]
pub fn sidebar_scrolled(app: &App, pane: Rect, delta: isize) -> usize {
    let (count, height, offset) = list_view(app, pane);
    let last = count.saturating_sub(height);
    offset.saturating_add_signed(delta).min(last)
}

/// What the sidebar is showing: how many rows its list has, how many of them
/// fit, and which one is on top.
///
/// One function for both tabs, because both are a `List` of one-row items with
/// one selection, and the mouse's question is the same for either.
fn list_view(app: &App, pane: Rect) -> (usize, usize, usize) {
    let height = usize::from(pane.height.saturating_sub(BORDER_ROWS));
    let (count, selected) = match app.sidebar_tab() {
        SidebarTab::Files => (app.sidebar_nodes().len(), app.sidebar_row()),
        SidebarTab::Comments => (app.comments().len(), app.browser_index()),
    };
    (
        count,
        height,
        list_offset(selected, count, height, app.sidebar_scroll()),
    )
}

/// Which entry a list `height` rows tall starts at.
///
/// Two rules, and the second is why this exists at all:
///
/// * with no parked view, the list scrolls as little as it can to keep the
///   selection on screen — which is what ratatui's own `ListState` does from an
///   offset of zero, spelled out here so that the offset the renderer *hands*
///   the widget is the one hit-testing reads;
/// * with one, the reviewer's own position wins, and the selection is simply
///   off screen while it does. A list that snapped back to its selection on
///   every wheel notch could not be scrolled past it at all, and rv would be
///   telling a reviewer with two hundred files that they may only look at the
///   twenty around their cursor.
fn list_offset(selected: usize, rows: usize, height: usize, scroll: Option<usize>) -> usize {
    let last = rows.saturating_sub(height);
    match scroll {
        Some(offset) => offset.min(last),
        None => selected.saturating_sub(height.saturating_sub(1)).min(last),
    }
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

/// A pane's block: rounded, titled, and marked when it holds the focus.
///
/// The mark is **three signals for one fact**: a `▸` on the title, a bold
/// border, and the border in [`gradient::FOCUS`] — the magenta this interface
/// spends on nothing else, because green is an addition, red a removal, blue a
/// comment and orange an alert, and a fifth meaning for any of them would be
/// ambiguous exactly when a reviewer is scanning fast.
///
/// The `▸` is redundant on purpose and stays. A sixteen-colour terminal renders
/// the magenta as whatever it likes or not at all, and a reader who does not
/// separate magenta from red gets nothing from the hue; colour *enhances* the
/// signal here and is never the only carrier of it.
fn pane(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().border_type(BorderType::Rounded);
    if focused {
        block.title(format!("▸ {title}")).border_style(
            Style::default()
                .fg(colour(gradient::FOCUS))
                .add_modifier(Modifier::BOLD),
        )
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
    let note = suppressed_note(app, height);
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
    usize::from(default_layout().diff.width.saturating_sub(BORDER_ROWS))
        .saturating_sub(GUTTER + BOX_PADDING)
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
// Alerts
// ---------------------------------------------------------------------------

/// The mark an alert leads with, so the panel says what it is before it is
/// read: a warning, not a status.
const ALERT_MARK: char = '⚠';

/// What sits between two alerts sharing the panel.
const ALERT_SEPARATOR: &str = " · ";

/// The floating panel: what has gone wrong, in orange, over the panes.
///
/// **One panel, however many alerts.** [`crate::layout::layout`] gives the toast
/// three rows — two borders and a message — and no rectangle in this reviewer is
/// computed anywhere but there, so several alerts share the row rather than
/// stacking down the screen. What matters is that none of them is lost, and the
/// row is clipped like every other row here rather than truncated silently.
///
/// It is drawn over the panes and is **not** a click target: [`crate::layout`]
/// has no `Target` for it on purpose, because a toast that could be clicked
/// would be a dialog, and a dialog is something a reviewer has to answer.
///
/// The fade is an Oklab ramp in `Rgb`, like the rest of the chrome this
/// interface owns and unlike the *code*, which is painted in indexed colours so
/// that it comes from the reviewer's own theme. Spec §9 asks for the toast to
/// disappear without fading at sixteen colours; nothing in this codebase detects
/// the terminal's colour depth — [`gradient::column_colour`] and
/// [`line_background`] emit `Rgb` unconditionally — so a probe written for the
/// toast alone would be the only one there is, and a second opinion about the
/// terminal is worse than a fade that degrades the way every other colour here
/// already does.
fn draw_toast(frame: &mut Frame, alerts: &[&Alert], area: Rect, now: Instant) {
    if alerts.is_empty() {
        return;
    }
    // The freshest alert decides the fade: they share one border, and dimming
    // it because an older message is nearly done would fade out a warning that
    // has just arrived.
    let fade = alerts
        .iter()
        .map(|alert| alert.fade(now))
        .fold(1.0_f32, f32::min);
    let style = Style::default().fg(colour(gradient::oklab_mix(
        gradient::ALERT,
        gradient::INK_DARK,
        fade,
    )));

    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let messages: Vec<&str> = alerts.iter().map(|alert| alert.message.as_str()).collect();
    let text = format!("{ALERT_MARK} {}", messages.join(ALERT_SEPARATOR));

    // Over whatever the panes drew there, rather than blended with it: a
    // warning read through a diff is a warning read twice.
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Line::styled(clip(&text, width), style)).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(style),
        ),
        area,
    );
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
        Paragraph::new(text).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title("▸ Keys — ? or Esc to close"),
        ),
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
