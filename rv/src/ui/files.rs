//! The file list: one row per file, per directory that holds files, or per
//! change that touched them.
//!
//! # Nothing here paints a background
//!
//! The gradient does **not** wash the row. Spec §7 rules that out after two
//! rounds of looking at the running tool: a full-row wash reads as a selection
//! and competes with the real one. The proportion is carried by the row's own
//! **text** instead — the name runs green through the seam to red, split where
//! the change is split — as a foreground on the terminal's own ground. It
//! replaced a column of bar cells that only appeared on wide panes and told
//! the reviewer nothing the tinted name does not. `g` turns the tint off,
//! `#` the counts, because both are decoration over the name's one job.
//!
//! # The counts sit on the left
//!
//! In two right-aligned columns ahead of the tree, so every name starts at the
//! same column whatever its depth or length. On the right they hung off the
//! end of names of every length and the eye had to hunt for each pair.
//!
//! # What goes when the pane is narrow
//!
//! The counts first, then the path is clipped: the path is the row's identity
//! and the counts are context.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use rv_core::model::ChangeKind;

mod counts;
mod icons;
mod row;

use row::Head;
use row::file_row;

use counts::CountsColumns;
use counts::counts;
use icons::DIR_COLOUR;
use icons::DIR_ICON_FOLDED;
use icons::DIR_ICON_OPEN;
use icons::file_icon;
use icons::icon_colour;
use ratatui::style::Color;

use super::BORDER_ROWS;
use super::list::list_state;
use super::pane::pane;
use super::pane::selection_style;
use super::text::shift;
use crate::app::App;
use crate::app::SidebarTab;
use crate::app::reviewed::Freshness;
use crate::tree::Node;
use crate::tree::NodeKind;

/// The fewest columns a row will show of a path before it gives up its counts.
///
/// The path is the row's *identity* and the counts are context: a row reading
/// `+40 -0` and nothing else names no file, while `added.…` still does.
pub(super) const MIN_PATH_COLUMNS: usize = 8;

/// The mark a row that holds others carries: pointing down when its contents
/// are shown, right when they are folded away. Three columns wide, like the
/// change marks beside it, so names line up down the column; the third
/// column is where a change row's tick goes.
const OPEN: &str = "▾";
/// See [`OPEN`].
const FOLDED: &str = "▸";
/// The mark on the row that leads back out of a zoomed subtree.
const UP: &str = "▴  ";
/// A reviewed file's tick, and the tick of one that changed after it was
/// reviewed.
pub(super) const TICK: char = '✓';
/// See [`TICK`].
pub(super) const TICK_STALE: char = '≈';

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
    let heads: Vec<Head> = nodes
        .iter()
        .map(|node| Head {
            text: head(app, node),
            lead: lead_of(app, node),
            icon: icon_of(app, node),
        })
        .collect();
    // One pair of counts columns for the whole list, each as wide as its widest
    // entry, so the numbers and the names line up down the pane. Zero when
    // nothing in the review changed a line — or when the reviewer has put the
    // column away with `#` — which is when there is no column to reserve.
    let counted: Vec<(String, String)> = nodes
        .iter()
        .map(|node| {
            if app.counts_shown() {
                counts(node.stat)
            } else {
                (String::new(), String::new())
            }
        })
        .collect();
    let columns = CountsColumns::fitting(&counted);

    let items: Vec<ListItem> = nodes
        .iter()
        .zip(&heads)
        .zip(&counted)
        .map(|((node, head), counts)| {
            ListItem::new(file_row(node, head, counts, columns, width, app.tint()))
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

/// Where the row's nerd-font icon sits, as a character offset into the head,
/// and the colour it takes — its file type's own, a folder's blue — or
/// `None` where there is no icon or nothing to colour it: `RV_ASCII`, a tick
/// standing in the icon's column, a plain document.
fn icon_of(app: &App, node: &Node) -> Option<(usize, Color)> {
    if app.ascii() {
        return None;
    }
    let at = node.depth * 2 + flag_mark(app, node).chars().count() + 2;
    match &node.kind {
        NodeKind::Dir { .. } => Some((at, DIR_COLOUR)),
        NodeKind::File { .. } if tick_mark(app, node).is_none() => {
            icon_colour(&node.label).map(|colour| (at, colour))
        }
        _ => None,
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

/// What a row spends on saying what kind of row it is: how a file changed, or
/// whether a row that holds others is open or folded — with a nerd-font folder
/// icon, or the file's own kind of icon, beside it, unless `RV_ASCII` turned
/// the patched glyphs off.
///
/// A reviewed tick takes the icon's column rather than one of its own: a file
/// that has been read is the one thing its row most needs to say, and a column
/// on every row would cost the names two characters each to say it on a few.
fn row_mark(app: &App, node: &Node) -> String {
    let icons = !app.ascii();
    let tick = tick_mark(app, node);
    let flagged = flag_mark(app, node);
    let mark = match &node.kind {
        NodeKind::Dir { collapsed, .. } if icons => {
            let (mark, icon) = if *collapsed {
                (FOLDED, DIR_ICON_FOLDED)
            } else {
                (OPEN, DIR_ICON_OPEN)
            };
            format!("{mark} {icon} ")
        }
        NodeKind::Dir { collapsed, .. } | NodeKind::Commit { collapsed, .. } => {
            let mark = if *collapsed { FOLDED } else { OPEN };
            format!("{mark} {}", tick.unwrap_or(' '))
        }
        NodeKind::Up => UP.to_owned(),
        NodeKind::File { index } => match app.files().get(*index) {
            Some(file) if icons => format!(
                "{:<2}{} ",
                marker(file.kind),
                tick.unwrap_or_else(|| file_icon(&node.label))
            ),
            Some(file) => format!("{:<2}{}", marker(file.kind), tick.unwrap_or(' ')),
            // A row addressing a file the review does not have cannot happen —
            // the rows are built from that very list — and is drawn blank
            // rather than panicking a frame over it.
            None => " ".repeat(3),
        },
    };
    format!("{flagged}{mark}")
}

/// The `⚑` a row carries while a flag under it waits to be looked at — and
/// nothing at all, not even the column, in a review with no open flags, so
/// the names pay for it only where there is something to find. A change row
/// carries it for any of its files.
fn flag_mark(app: &App, node: &Node) -> &'static str {
    let open: Vec<&str> = app
        .flags()
        .iter()
        .filter(|flag| !flag.acknowledged)
        .map(|flag| flag.anchor.file.as_str())
        .collect();
    if open.is_empty() {
        return "";
    }
    let names = |file: &rv_core::model::FileChange| {
        open.contains(&file.path.as_str())
            || file
                .source_path
                .as_deref()
                .is_some_and(|source| open.contains(&source))
    };
    let flagged = match (&node.kind, app.sidebar_tab()) {
        (NodeKind::File { index }, SidebarTab::Files) => app.files().get(*index).is_some_and(names),
        (NodeKind::File { index }, _) => app
            .commit_path(*index)
            .is_some_and(|path| open.contains(&path)),
        (NodeKind::Commit { change_id, .. }, _) => app
            .changes()
            .iter()
            .position(|change| change.change_id == *change_id)
            .is_some_and(|change| {
                app.commit_change_paths(change)
                    .iter()
                    .any(|path| open.contains(&path.as_str()))
            }),
        _ => false,
    };
    if flagged { "⚑ " } else { "  " }
}

/// The row's reviewed tick: `✓` for a file reviewed as it stands, `≈` for one
/// that changed under its tick, nothing otherwise — and on a change row, the
/// tick every file under it shares. Only the tab's own scope counts: the files
/// tab shows the range's ticks, the commits tab each change's.
fn tick_mark(app: &App, node: &Node) -> Option<char> {
    let freshness = match (&node.kind, app.sidebar_tab()) {
        (NodeKind::File { index }, SidebarTab::Files) => app
            .files()
            .get(*index)
            .and_then(|file| app.reviewed_mark(&file.path, None)),
        (NodeKind::File { index }, SidebarTab::Commits) => {
            let change = app.commit_change(*index);
            let change_id = change.and_then(|change| app.changes().get(change));
            app.commit_path(*index).and_then(|path| {
                app.reviewed_mark(path, change_id.map(|change| change.change_id.as_str()))
            })
        }
        (NodeKind::Commit { change_id, .. }, _) => app
            .changes()
            .iter()
            .position(|change| change.change_id == *change_id)
            .and_then(|change| app.change_reviewed(change)),
        _ => None,
    };
    match freshness {
        Some(Freshness::Current) => Some(TICK),
        Some(Freshness::Changed) => Some(TICK_STALE),
        None => None,
    }
}

/// A row's name, indent and change mark included, before anything is clipped —
/// scrolled sideways where the reviewer has asked to see the tail of the names.
///
/// A commit row does not scroll: its ids are the part a reviewer acts on, and
/// scrolling them off would leave a subject nobody can select anything by.
fn head(app: &App, node: &Node) -> String {
    let label = match &node.kind {
        // Neither scrolls: a commit's ids and the way out of a zoom are the
        // parts a reviewer acts on.
        NodeKind::Commit { .. } | NodeKind::Up => node.label.clone(),
        NodeKind::Dir { .. } | NodeKind::File { .. } => shift(&node.label, app.sidebar_hscroll()),
    };
    format!(
        "{}{}{}",
        "  ".repeat(node.depth),
        row_mark(app, node),
        label
    )
}
