//! The file list: one row per file, per directory that holds files, or per
//! change that touched them.
//!
//! # Nothing here paints a background
//!
//! The gradient does **not** wash the row. Spec §7 rules that out after two
//! rounds of looking at the running tool: a full-row wash reads as a selection
//! and competes with the real one, and even a text-width wash destroys what the
//! pane exists to show, because in tree mode the structure *is* the indentation
//! and the fold marks. So the colour lives in the **counts**, as a foreground
//! on the terminal's own ground, and the proportion survives as [`change_bar`]
//! — a mark on the row rather than the row itself.
//!
//! # What goes when the pane is narrow
//!
//! The bar first, then the counts, then the path is clipped: each is more the
//! row's identity than the last. The bar is decided for the **whole list** from
//! its longest name, because a bar on the short rows and not the long ones
//! would be a ragged column.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use rv_core::model::ChangeKind;

mod counts;
mod row;

use row::file_row;

use counts::counts;
use counts::counts_columns;

use super::BORDER_ROWS;
use super::list::list_state;
use super::pane::pane;
use super::pane::selection_style;
use crate::app::App;
use crate::tree::Node;
use crate::tree::NodeKind;

/// The fewest columns a row will show of a path before it gives up its counts.
///
/// The path is the row's *identity* and the counts are context: a row reading
/// `+40 -0` and nothing else names no file, while `added.…` still does.
pub(super) const MIN_PATH_COLUMNS: usize = 8;

/// How many columns the change bar takes on a row wide enough to carry one.
///
/// Six: enough to read a proportion at a glance, and few enough that a row only
/// has to be eight columns wider than its own name to earn one.
pub(super) const BAR_COLUMNS: usize = 6;

/// The glyph the change bar is drawn with, as a **foreground** on the
/// terminal's own ground.
pub(super) const BAR: char = '█';

/// The mark a row that holds others carries: pointing down when its contents
/// are shown, right when they are folded away. Three columns wide, like the
/// change marks beside it, so names line up down the column.
const OPEN: &str = "▾  ";
/// See [`OPEN`].
const FOLDED: &str = "▸  ";

/// # The shape and the order go on the bottom border
///
/// The title already carries the focus mark and the count, and at 80 columns
/// the sidebar has twenty-one columns inside its borders — `▸ Files (2) · list
/// · natural` is twenty-eight, so putting the order up there would truncate
/// away exactly the thing it is there to say.
pub(super) fn draw_files(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let title = format!("Files ({})", app.files().len());
    draw_nodes(frame, app, area, focused, &app.sidebar_nodes(), title);
}

/// The same list, one level up: the stack's changes, each holding the files it
/// touched. `t` and `o` mean here what they mean there — the files under a
/// change are a tree or a list, ordered the same way — because they are the
/// same rows drawn from the same model.
pub(super) fn draw_commits(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let title = format!("Commits ({})", app.changes().len());
    draw_nodes_titled(
        frame,
        app,
        area,
        focused,
        &app.commit_nodes(),
        title,
        under_the_commits(app),
    );
}

/// One list of [`Node`]s, with its counts column, its change bars and its
/// selection.
fn draw_nodes(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    focused: bool,
    nodes: &[Node],
    title: String,
) {
    let bottom = shape(app);
    draw_nodes_titled(frame, app, area, focused, nodes, title, bottom);
}

/// The same, with what the bottom border says passed in: the file list states its
/// shape and its order there, and the commits list states which change the cursor
/// is in.
fn draw_nodes_titled(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    focused: bool,
    nodes: &[Node],
    title: String,
    bottom: String,
) {
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let heads: Vec<String> = nodes.iter().map(|node| head(app, node)).collect();
    // One counts column for the whole list, as wide as its widest entry, so the
    // names line up down the pane. Zero when nothing in the review changed a
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
            ListItem::new(file_row(
                node,
                head,
                lead_of(app, node),
                counts,
                counts_width,
                bar,
                width,
            ))
        })
        .collect();
    let list = List::new(items)
        .block(pane(title, focused).title_bottom(bottom))
        .highlight_style(selection_style(focused));

    let mut state = list_state(app, area, nodes.len(), app.sidebar_row());
    frame.render_stateful_widget(list, area, &mut state);
}

/// What the file list says about itself along its bottom border.
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

/// The commits list's bottom border: its shape and order, and the key that shows
/// a change in full.
///
/// The description itself is *on the row*, after the hash. It lived here for one
/// wave and the reviewer's verdict was that a border is not a place to read: the
/// text is cut off wherever the sidebar is narrow, which is everywhere. A clipped
/// subject on the row costs nothing now that `i` shows the whole message.
fn under_the_commits(app: &App) -> String {
    let shape = shape(app);
    if app.change_under_cursor().is_some() {
        format!("{shape}· i info ")
    } else {
        shape
    }
}

/// How many columns of a row come before its label: the indent, and the fold
/// mark that says whether the row is open.
fn lead_of(app: &App, node: &Node) -> usize {
    node.depth * 2 + row_mark(app, node).chars().count()
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

/// The three columns a row spends on saying what kind of row it is: how a file
/// changed, or whether a row that holds others is open or folded.
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

/// A row's name, indent and change mark included, before anything is clipped.
fn head(app: &App, node: &Node) -> String {
    format!(
        "{}{}{}",
        "  ".repeat(node.depth),
        row_mark(app, node),
        node.label
    )
}
