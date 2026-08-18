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
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use rv_core::model::ChangeKind;

mod counts;

use counts::change_bar;
use counts::counts;
use counts::counts_columns;

use super::BORDER_ROWS;
use super::list::list_state;
use super::pane::pane;
use super::pane::selection_style;
use super::text::clip;
use super::text::colour;
use crate::app::App;
use crate::gradient;
use crate::tree::Node;
use crate::tree::NodeKind;

/// The fewest columns a row will show of a path before it gives up its counts.
///
/// The path is the row's *identity* and the counts are context: a row reading
/// `+40 -0` and nothing else names no file, while `added.…` still does.
const MIN_PATH_COLUMNS: usize = 8;

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

/// One row: its name on the left, and — right-aligned in a column shared by the
/// whole list — its change bar and its counts.
///
/// `bar` is `0` where the list gave the bar up, and the counts go with it when
/// even they would leave the name less than [`MIN_PATH_COLUMNS`].
fn file_row(
    node: &Node,
    head: &str,
    lead: usize,
    counts: &(String, String),
    counts_width: usize,
    bar: usize,
    width: usize,
) -> Line<'static> {
    let tail = if bar > 0 { bar + 1 } else { 0 } + counts_width;
    // One column of gap at least, always: a name clipped right up against its
    // own numbers reads as one word.
    let names = width.saturating_sub(tail + 1);
    if counts_width == 0 || names < MIN_PATH_COLUMNS {
        return Line::from(clip(head, width));
    }

    // A commit row gives up its subject before it gives up an id, and gives up
    // the second id whole rather than half: half a hash cannot be pasted, so
    // printing five characters of one is worse than printing none.
    let fitted = fit_commit(node, head, lead, names);
    let name = clip(fitted.as_deref().unwrap_or(head), names);
    let mut spans = name_spans(node, &name, lead);
    spans.push(Span::raw(" ".repeat(names + 1 - name.chars().count())));

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

/// How many columns of a row come before its label: the indent, and the fold
/// mark that says whether the row is open.
fn lead_of(app: &App, node: &Node) -> usize {
    node.depth * 2 + row_mark(app, node).chars().count()
}

/// What tier of the tree a row is, as a style.
///
/// Three, because a list of a hundred paths in one ink is a wall: a **directory**
/// is scaffolding and reads dim, a **file** is the thing being reviewed and keeps
/// the terminal's own foreground, and a **change** is a heading and is handled by
/// [`name_spans`], which colours its ids.
///
/// Foreground only. Spec §7 rules out a background wash on a sidebar row after
/// two rounds of looking at the running tool — thirty files became thirty slabs
/// of green and the tree stopped looking like a tree — and the selection is still
/// the one full-row background in this pane.
fn structure_style(node: &Node) -> Style {
    match node.kind {
        // Dim, not coloured: a directory is where a file *is*, and a hue here
        // would compete with the counts, which are the only thing in this pane
        // that means green and red.
        NodeKind::Dir { .. } => Style::default().add_modifier(Modifier::DIM),
        NodeKind::File { .. } | NodeKind::Commit { .. } => Style::default(),
    }
}

/// A change row's name, cut down to what `names` columns can hold.
///
/// Three forms, widest first: both ids and the subject, both ids, the change id
/// alone. `None` for any other row, and for a change row whose full form already
/// fits.
///
/// The ids come before the subject because they are what a reviewer *acts* on —
/// pasted into `jj show`, typed to select the change — and the subject is on the
/// bar whenever the cursor is in the change anyway. And an id is kept whole or
/// dropped: `e…` is not a commit hash, it is a hash-shaped hole, and a row that
/// prints one invites a paste that cannot work.
fn fit_commit(node: &Node, head: &str, lead: usize, names: usize) -> Option<String> {
    let NodeKind::Commit {
        short_change,
        short_commit,
        subject,
        ..
    } = &node.kind
    else {
        return None;
    };
    let room = names.saturating_sub(lead);
    let full = format!("{short_change} {short_commit} {subject}");
    if full.chars().count() <= room {
        return None;
    }
    let ids = format!("{short_change} {short_commit}");
    let text = if ids.chars().count() <= room {
        ids
    } else {
        short_change.clone()
    };
    // The row's own lead, not blanks: it carries the fold mark, and a change row
    // that lost its `▾` would look like a change with nothing under it.
    let prefix: String = head.chars().take(lead).collect();
    Some(format!("{prefix}{text}"))
}

/// A row's name, as one span for a file or a directory and several for a change.
///
/// A change row prints two ids and a subject, and the leading characters of each
/// id are the ones you can type to select it — so those characters are bright and
/// the rest of the id is dim, exactly as `jj log` draws them. Anything else makes
/// a reviewer count characters to find out what to paste.
///
/// `name` is already clipped, so this splits what survived rather than the
/// original: a row narrow enough to lose half an id highlights half of it and
/// nothing beyond the edge. `lead` is how many columns the indent and the fold
/// mark take, and it is passed in rather than measured back out of `name` — a
/// clipped row is shorter than the text it was made from, so subtracting the
/// parts from the whole put the ids three columns off on every row that did not
/// fit.
fn name_spans(node: &Node, name: &str, lead: usize) -> Vec<Span<'static>> {
    let NodeKind::Commit {
        short_change,
        short_commit,
        unique,
        ..
    } = &node.kind
    else {
        return vec![Span::styled(name.to_owned(), structure_style(node))];
    };

    // Where the ids sit in the drawn row: the indent and the fold mark come
    // first, and `head` built the row as `<indent><mark><change> <commit> <subject>`.
    let mut spans = vec![Span::raw(name.chars().take(lead).collect::<String>())];
    let mut at = lead;
    for (id, ink) in [
        (short_change, gradient::FOCUS),
        (short_commit, gradient::HASH),
    ] {
        if at >= name.chars().count() {
            break;
        }
        let (bright, dim) = split_at_char(name, at, *unique, id.chars().count());
        if !bright.is_empty() {
            spans.push(Span::styled(
                bright,
                Style::default().fg(colour(ink)).add_modifier(Modifier::BOLD),
            ));
        }
        if !dim.is_empty() {
            spans.push(Span::styled(
                dim,
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
        at += id.chars().count();
        // The space between the ids, and after the second one, the subject.
        let rest: String = name.chars().skip(at).take(1).collect();
        spans.push(Span::raw(rest));
        at += 1;
    }
    // Whatever is left is the subject: quieter than the ids, which are the part
    // you act on.
    spans.push(Span::styled(
        name.chars().skip(at).collect::<String>(),
        Style::default().add_modifier(Modifier::DIM),
    ));
    spans
}

/// The `unique` bright characters of the id starting at `at`, and however much
/// of the rest of it survived the clip.
fn split_at_char(name: &str, at: usize, unique: usize, length: usize) -> (String, String) {
    let id: Vec<char> = name.chars().skip(at).take(length).collect();
    let cut = unique.min(id.len());
    (
        id[..cut].iter().collect(),
        id[cut..].iter().collect(),
    )
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

