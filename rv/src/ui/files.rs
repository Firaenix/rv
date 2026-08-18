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
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use rv_core::model::ChangeKind;

use super::BORDER_ROWS;
use super::list::list_state;
use super::pane::pane;
use super::pane::selection_style;
use super::text::clip;
use super::text::colour;
use crate::app::App;
use crate::gradient;
use crate::gradient::Rgb;
use crate::gradient::Stat;
use crate::tree;
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
const BAR_COLUMNS: usize = 6;

/// The glyph the change bar is drawn with, as a **foreground** on the
/// terminal's own ground.
const BAR: char = '█';

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
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let nodes = app.sidebar_nodes();
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
            ListItem::new(file_row(node, head, counts, counts_width, bar, width))
        })
        .collect();
    let list = List::new(items)
        .block(pane(format!("Files ({})", app.files().len()), focused).title_bottom(shape(app)))
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

/// One row: its name on the left, and — right-aligned in a column shared by the
/// whole list — its change bar and its counts.
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
    // own numbers reads as one word.
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
/// Consecutive cells of one colour are one span, so a flat green bar is one
/// span rather than six.
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

/// What a row costs to review, as the two numbers the pane prints — or two
/// empty strings where it cost no lines, because zero is not a measurement.
///
/// Two strings rather than one because they are drawn in two colours, which is
/// where the sidebar's colour lives now that no row is washed. Abbreviated by
/// [`tree::abbreviate`], which is never wider than four characters, so the
/// counts cannot push the path out of a narrow column by being long.
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

/// The sidebar's one- or two-character mark for how a file changed.
fn marker(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "+",
        ChangeKind::Removed => "-",
        ChangeKind::Renamed => "->",
        ChangeKind::Modified => "~",
    }
}
