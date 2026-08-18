//! One row of the list: its name, and what it gives up as the pane narrows.
//!
//! A change row gives up its subject before either id, and gives up the second
//! id whole rather than half — a hash printed as `e…` cannot be pasted, so a row
//! that prints one invites a paste that cannot work.

use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use super::super::text::clip;
use super::super::text::colour;
use super::MIN_PATH_COLUMNS;
use super::counts::change_bar;
use super::counts::counts_columns;
use crate::gradient;
use crate::tree::Node;
use crate::tree::NodeKind;

/// One row: its name on the left, and — right-aligned in a column shared by the
/// whole list — its change bar and its counts.
///
/// `bar` is `0` where the list gave the bar up, and the counts go with it when
/// even they would leave the name less than [`MIN_PATH_COLUMNS`].
pub(super) fn file_row(
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
                Style::default()
                    .fg(colour(ink))
                    .add_modifier(Modifier::BOLD),
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
    (id[..cut].iter().collect(), id[cut..].iter().collect())
}
