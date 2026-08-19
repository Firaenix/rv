//! The sidebar's rows: a flat list, a directory tree, or commits holding the
//! files they touched — one model with three node kinds rather than three
//! widgets that would drift apart.
//!
//! A commit is a directory in every respect that matters here: it holds
//! children, it folds away under the same key, its subtree is what a summary
//! over it is computed from, and it takes its files with it when an order
//! moves it. Making it a third [`NodeKind`] rather than a second widget means
//! one selection model, one collapse rule, one aggregate and one place to walk
//! a subtree; three of anything would drift.
//!
//! Three rules earn the module its existence:
//!
//! * **A chain of single-child directories is one row.** `docs/superpowers/
//!   specs` is one row and not three, because a 29-file review has perhaps 40
//!   rows to spend and a tree that spends half of them on punctuation is worse
//!   than the flat list it replaced.
//! * **The tree lists exactly the files the flat list does.** Every path in,
//!   exactly one [`NodeKind::File`] out, carrying that path's index — so a
//!   file can never be lost behind a directory that was drawn wrong. An order
//!   is held to the same law: under every [`Sort`] and both groupings the bag
//!   of rows is identical and only its sequence moves, because a sort that
//!   loses a file is worse than no sort.
//! * **Every row says what it costs to review**, and a row that holds others
//!   says what its whole subtree costs — for the same reason it carries the
//!   subtree's gradient. A collapsed row that hides its own weight is a row
//!   you have to expand to judge, which is exactly the work folding it was
//!   meant to save.
//!
//! Nothing here knows about ratatui: a node is a label, a depth, a kind and a
//! [`Stat`], and the renderer owns every glyph, colour and column of it.
//! Nothing here is review state either — which rows are folded and which order
//! they are in are session-only preferences held by the caller, and none of it
//! reaches `.review/`.

mod assemble;
mod commit;
mod sort;

pub use sort::Sort;
pub use sort::abbreviate;

use assemble::assemble;
use assemble::file_node;
use assemble::flatten;
use commit::commit_label;
use commit::short;
use commit::subject_of;
use commit::unique_prefix;
use sort::order;

use std::collections::HashSet;

use crate::gradient::Stat;

/// What a change with an empty description is labelled with, matching what jj
/// itself shows.
pub(crate) const NO_DESCRIPTION: &str = "(no description set)";

/// Separates a change's id from a path inside the collapse key of a row in the
/// commits view. A jj change id is letters only and a path may not contain a
/// colon on every platform rv supports, so the two halves can always be told
/// apart — and `src` under one change folds without folding `src` under the
/// next.
pub(crate) const KEY_SEPARATOR: char = ':';

/// One row of the sidebar.
///
/// `depth` is how far the row is indented, in levels: a root is 0 and a child
/// is one more than its parent. The renderer turns that into columns; the
/// model deliberately does not, because the indent a terminal can afford is
/// the renderer's problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// What the row reads as: a file's name, a directory chain, or a change
    /// and its subject. Never a whole path in the tree — except on a root
    /// directory, where the chain *is* the path — and always the whole path in
    /// the flat list, which is the list the sidebar has always drawn.
    pub label: String,
    /// How many levels in the row sits.
    pub depth: usize,
    pub kind: NodeKind,
    /// What the row costs to review: its own lines for a file, and its whole
    /// subtree's for a directory or a change — including the part a fold is
    /// hiding, which is the number that says whether to open it.
    pub stat: Stat,
}

/// How many characters of an id a row shows.
///
/// Eight, which is what `jj log` shows and what a reviewer copying an id out of
/// the screen expects to be able to paste. A change id is thirty-two characters
/// and a commit hash forty: printed whole, either one fills the sidebar and
/// leaves no room for the description, which is what the row is *for*.
pub const ID_SHORT: usize = 8;

/// The three things a sidebar row can be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A change, holding the files it touched.
    Commit {
        /// The change's id, which is also the key it folds under.
        change_id: String,
        /// The first [`ID_SHORT`] characters of the change id, and of the commit
        /// hash: what the row actually prints.
        short_change: String,
        /// See `short_change`.
        short_commit: String,
        /// How many leading characters pick this change out of the review.
        ///
        /// jj highlights the shortest prefix that names a revision uniquely, so
        /// that the highlighted part is exactly what you can type to select it.
        /// This is the same measure taken over the changes *in this review*,
        /// which is the set the sidebar is listing — the only set this module
        /// knows about, and the one a reviewer is choosing between.
        unique: usize,
        /// The change's subject, or a stand-in where it has none.
        subject: String,
        /// Whether its files are hidden.
        collapsed: bool,
    },
    /// A directory, holding whatever is under it.
    Dir {
        /// The identity this directory folds under: its path from the review
        /// root, prefixed by the owning change's id in the commits view so
        /// that the same directory under two changes folds independently. In
        /// the bookmark view it is exactly the path.
        ///
        /// The row carries it so that the key a keystroke toggles is the key
        /// this row was built from. A renderer that rebuilt it from the label
        /// and the indent would be reconstructing something this module
        /// already knows, and would get it wrong the first time a chain
        /// merged.
        key: String,
        /// Whether its contents are hidden.
        collapsed: bool,
    },
    /// A file, addressing the caller's list of them.
    File {
        /// The file's position in what was passed in: an index into `paths`
        /// for [`build`], and into the concatenation of every group's paths in
        /// order for [`build_grouped`], so that a file touched by two changes
        /// is two rows addressing two diffs.
        ///
        /// It is the *input* position and never the row's own: an order moves
        /// rows about, and a row that renumbered itself as it moved would open
        /// a different file than the one it names.
        index: usize,
    },
    /// The way back out of a zoomed subtree: the first row of a zoomed view,
    /// labelled with where the reviewer is.
    ///
    /// Never built here — [`build`] and [`build_grouped`] return whole views,
    /// and the zoom that carves one down is the caller's state. It is a node
    /// kind so the zoomed view stays one list with one cursor and one hit
    /// test, rather than a header the mouse cannot land on.
    Up,
}

/// One change and the files it touched, as the commits view is given them.
///
/// Borrowed rather than owned because the rows are rebuilt every frame from
/// state the app already holds; nothing here outlives the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group<'a> {
    /// The change's id, and the key its row folds under.
    pub change_id: &'a str,
    /// The commit the change currently is, which is what a reader pastes into
    /// `git show`.
    pub commit_id: &'a str,
    /// Its description. Only the first line reaches the row.
    pub description: &'a str,
    /// The files it touched, in the order they should be listed.
    pub paths: &'a [&'a str],
}

/// The bookmark view: every changed file, as a directory tree or a flat list.
///
/// `collapsed` holds the keys of the directories the reviewer has folded away
/// — see [`NodeKind::Dir::key`] — and is ignored entirely when `tree` is
/// false, because a list has nothing to fold. With `tree` false and `sort`
/// natural the answer is exactly the list the sidebar has always drawn: one
/// row per path, in the order given, at depth 0.
///
/// `stat_of` is asked for a file's size by that file's **index**, which is how
/// this module addresses a file everywhere else. A path would be the obvious
/// key and the wrong one: [`build_grouped`] lists the same path under every
/// change that touched it, weighing differently under each. A file `stat_of`
/// has no answer for weighs nothing, which is what a rename did.
pub fn build(
    paths: &[&str],
    collapsed: &HashSet<String>,
    tree: bool,
    sort: Sort,
    stat_of: &dyn Fn(usize) -> Stat,
) -> Vec<Node> {
    let files = paths.iter().copied().enumerate();
    if !tree {
        let mut nodes: Vec<Node> = files
            .map(|(index, path)| file_node(path, index, 0, stat_of(index)))
            .collect();
        order(&mut nodes, sort, |node| node.stat);
        return nodes;
    }

    let mut nodes = Vec::with_capacity(paths.len());
    flatten(
        &assemble(files, stat_of),
        "",
        0,
        collapsed,
        sort,
        &mut nodes,
    );
    nodes
}

/// The commits view: each change holds the files it touched, and `tree`
/// chooses whether those files are a directory tree or a flat list beneath it.
///
/// File indices run *across* the changes: the first path of the first group is
/// 0 and the numbering carries on into the next group, so a file two changes
/// touched gets one index per change and each row addresses that change's diff
/// of it. The caller reads its own list of (change, file) pairs in the same
/// order and the indices line up — including under a [`Sort`], which moves the
/// change rows around but may not renumber what they hold.
///
/// See [`build`] for `stat_of`.
pub fn build_grouped(
    groups: &[Group<'_>],
    collapsed: &HashSet<String>,
    tree: bool,
    sort: Sort,
    stat_of: &dyn Fn(usize) -> Stat,
) -> Vec<Node> {
    // Number the files in the caller's order first and order the changes
    // second: an index belongs to the input, a row's position to the view.
    let mut stack: Vec<(&Group<'_>, usize, Stat)> = Vec::with_capacity(groups.len());
    let mut next = 0;
    for group in groups {
        let base = next;
        next += group.paths.len();
        let total = (base..next)
            .map(stat_of)
            .fold(Stat::default(), |total, file| total + file);
        stack.push((group, base, total));
    }
    order(&mut stack, sort, |(_, _, total)| *total);

    // Measured over every change the review lists, which is the set the reviewer
    // is choosing between — see `NodeKind::Commit::unique`.
    let ids: Vec<&str> = groups.iter().map(|group| group.change_id).collect();

    let mut nodes = Vec::new();
    for (group, base, total) in stack {
        let folded = collapsed.contains(group.change_id);
        nodes.push(Node {
            label: commit_label(group),
            depth: 0,
            kind: NodeKind::Commit {
                change_id: group.change_id.to_owned(),
                short_change: short(group.change_id),
                short_commit: short(group.commit_id),
                unique: unique_prefix(group.change_id, &ids),
                subject: subject_of(group),
                collapsed: folded,
            },
            stat: total,
        });
        if folded {
            continue;
        }

        let files = group
            .paths
            .iter()
            .copied()
            .enumerate()
            .map(move |(at, path)| (base + at, path));
        if tree {
            let prefix = format!("{}{KEY_SEPARATOR}", group.change_id);
            flatten(
                &assemble(files, stat_of),
                &prefix,
                1,
                collapsed,
                sort,
                &mut nodes,
            );
        } else {
            let mut rows: Vec<Node> = files
                .map(|(index, path)| file_node(path, index, 1, stat_of(index)))
                .collect();
            order(&mut rows, sort, |node| node.stat);
            nodes.extend(rows);
        }
    }
    nodes
}
