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

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::HashSet;

use crate::gradient::Stat;

/// What a change with an empty description is labelled with, matching what jj
/// itself shows.
const NO_DESCRIPTION: &str = "(no description set)";

/// Separates a change's id from a path inside the collapse key of a row in the
/// commits view. A jj change id is letters only and a path may not contain a
/// colon on every platform rv supports, so the two halves can always be told
/// apart — and `src` under one change folds without folding `src` under the
/// next.
const KEY_SEPARATOR: char = ':';

/// The order the sidebar's rows are in.
///
/// One mode rather than one per view, which is why one key serves both:
/// [`Sort::Natural`] means "the order the thing already has" — path order for
/// files, stack order for commits — and the other two weigh a row by one hand
/// of its [`Stat`], heaviest first.
///
/// An order applies *within* the grouping and never across it: siblings sort
/// against each other, a directory sorts among its own siblings by its
/// aggregate, and its children stay under it. A reviewer asked for a tree and
/// for sorting; they compose, and neither disables the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// The order the thing already has: path order in the bookmark view,
    /// stack order in the commits view.
    #[default]
    Natural,
    /// Most lines added first.
    Added,
    /// Most lines removed first.
    Removed,
}

impl Sort {
    /// The next order, cycling — what the one key that switches them does.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Natural => Self::Added,
            Self::Added => Self::Removed,
            Self::Removed => Self::Natural,
        }
    }

    /// The one word the sidebar's title says, so that the name of a mode is
    /// declared beside the mode rather than invented by a renderer.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Added => "added",
            Self::Removed => "removed",
        }
    }

    /// What this order weighs a row by, or `None` when it weighs nothing and
    /// the rows keep the order they arrived in.
    const fn weigh(self, stat: Stat) -> Option<u32> {
        match self {
            Self::Natural => None,
            Self::Added => Some(stat.added),
            Self::Removed => Some(stat.removed),
        }
    }
}

/// Puts `items` in `sort`'s order, heaviest first, and leaves them exactly as
/// they were under [`Sort::Natural`].
///
/// The sort is stable, so rows of equal weight keep the order they already had
/// rather than swapping for nothing — a sidebar that reshuffles equal rows
/// moves under the cursor and buys nothing for it. One helper and three
/// callers, so a directory's children, a flat list and a stack of changes
/// cannot come to disagree about what "sorted" means.
fn order<T>(items: &mut [T], sort: Sort, stat: impl Fn(&T) -> Stat) {
    if matches!(sort, Sort::Natural) {
        return;
    }
    items.sort_by_key(|item| Reverse(sort.weigh(stat(item)).unwrap_or_default()));
}

/// A count as a narrow sidebar can afford to print it: `42` stays `42`, `1234`
/// becomes `1.2k` and `45678` becomes `46k`.
///
/// Never more than four characters wide, for any `u32`. The counts are the
/// first thing dropped when the sidebar is squeezed, and a number that
/// overflowed its column would push the path out instead — which is the wrong
/// thing to lose, since the gradient still carries the ratio but nothing else
/// carries the name.
///
/// A value under ten in its unit keeps one decimal, because `1.2k` and `9.8k`
/// are four times apart and `1k` against `10k` would be the only alternative;
/// above ten the decimal is noise and is dropped. Rounding that would carry a
/// value up to the next unit moves it there rather than printing `1000k`.
#[must_use]
pub fn abbreviate(n: u32) -> String {
    if n < 1_000 {
        return n.to_string();
    }

    let mut scale = 1_000u64;
    for suffix in ["k", "M", "G"] {
        // The value in this unit, rounded half up: first to a tenth, then —
        // if that has grown past ten — to the unit itself.
        let tenths = (u64::from(n) * 10 + scale / 2) / scale;
        if tenths < 100 {
            let (whole, tenth) = (tenths / 10, tenths % 10);
            return if tenth == 0 {
                format!("{whole}{suffix}")
            } else {
                format!("{whole}.{tenth}{suffix}")
            };
        }
        let units = (u64::from(n) + scale / 2) / scale;
        if units < 1_000 {
            return format!("{units}{suffix}");
        }
        scale *= 1_000;
    }

    // Unreachable for a `u32`, whose largest value is 4.3G and so returns at
    // "G" or sooner. Total rather than panicking: a count is not worth a
    // crash.
    n.to_string()
}

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
    /// What the row is, and everything only that kind of row has.
    pub kind: NodeKind,
    /// What the row costs to review: its own lines for a file, and its whole
    /// subtree's for a directory or a change — including the part a fold is
    /// hiding, which is the number that says whether to open it.
    pub stat: Stat,
}

/// The three things a sidebar row can be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A change, holding the files it touched.
    Commit {
        /// The change's id, which is also the key it folds under.
        change_id: String,
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
}

/// One change and the files it touched, as the commits view is given them.
///
/// Borrowed rather than owned because the rows are rebuilt every frame from
/// state the app already holds; nothing here outlives the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group<'a> {
    /// The change's id, and the key its row folds under.
    pub change_id: &'a str,
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

    let mut nodes = Vec::new();
    for (group, base, total) in stack {
        let folded = collapsed.contains(group.change_id);
        nodes.push(Node {
            label: commit_label(group),
            depth: 0,
            kind: NodeKind::Commit {
                change_id: group.change_id.to_owned(),
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

/// `change_id subject`, with the subject being the first line of the
/// description — a change's row is one row, and jj descriptions are written
/// with that convention already.
fn commit_label(group: &Group<'_>) -> String {
    let subject = group.description.lines().next().unwrap_or_default().trim();
    let subject = if subject.is_empty() {
        NO_DESCRIPTION
    } else {
        subject
    };
    format!("{} {subject}", group.change_id)
}

/// A file's row, labelled with `label`.
fn file_node(label: &str, index: usize, depth: usize, stat: Stat) -> Node {
    Node {
        label: label.to_owned(),
        depth,
        kind: NodeKind::File { index },
        stat,
    }
}

/// A directory while the tree is being assembled.
///
/// Files are a `Vec` and not a map, because two identical paths must produce
/// two rows: they are two files as far as this module is concerned, and a map
/// keyed by name would silently swallow one of them. That is precisely the
/// loss the conservation property exists to catch.
#[derive(Default)]
struct Assembled<'a> {
    /// Subdirectories by name, in the order they will be drawn.
    dirs: BTreeMap<&'a str, Assembled<'a>>,
    /// The files directly inside, as `(name, index, size)`.
    files: Vec<(&'a str, usize, Stat)>,
    /// Everything beneath here, added up — computed once the tree is
    /// assembled, so a directory row can stand for its subtree without the
    /// renderer or the order re-walking it.
    stat: Stat,
}

/// Sorts every path into the directory that holds it, and measures each one.
///
/// A path is split on `/`: the last segment is the file's name and the rest
/// are the directories above it, so `a/b.rs` is `b.rs` inside `a`, and a path
/// with no separator is a file at the root. Empty segments are kept rather
/// than skipped — a repository is where these come from, and dropping one
/// would lose the file that owns it.
fn assemble<'a>(
    files: impl Iterator<Item = (usize, &'a str)>,
    stat_of: &dyn Fn(usize) -> Stat,
) -> Assembled<'a> {
    let mut root = Assembled::default();
    for (index, path) in files {
        let mut segments = path.split('/').peekable();
        let mut at = &mut root;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                at.files.push((segment, index, stat_of(index)));
                break;
            }
            at = at.dirs.entry(segment).or_default();
        }
    }
    root.sort();
    root.measure();
    root
}

impl Assembled<'_> {
    /// Puts every level in drawing order: files by name, with two files of the
    /// same name left in the order they arrived so their indices stay
    /// ascending. Directories are already ordered by the `BTreeMap` holding
    /// them, and are drawn before the files.
    ///
    /// This is [`Sort::Natural`], and the base every other order is a stable
    /// permutation of — which is what makes a tie keep a meaningful order
    /// rather than an arbitrary one.
    fn sort(&mut self) {
        self.files.sort_by_key(|(name, _, _)| *name);
        for dir in self.dirs.values_mut() {
            dir.sort();
        }
    }

    /// Adds every subtree up and leaves the total on the directory that holds
    /// it, bottom up.
    fn measure(&mut self) -> Stat {
        let mut total = self
            .files
            .iter()
            .fold(Stat::default(), |total, (_, _, stat)| total + *stat);
        for dir in self.dirs.values_mut() {
            total += dir.measure();
        }
        self.stat = total;
        total
    }

    /// Whether this directory holds exactly one thing and that thing is a
    /// directory — the case a chain merges away.
    fn lone_dir(&self) -> Option<(&str, &Self)> {
        if !self.files.is_empty() || self.dirs.len() != 1 {
            return None;
        }
        self.dirs
            .iter()
            .next()
            .map(|(name, dir)| (*name, dir as &Self))
    }
}

/// One of a directory's rows while its contents are being put in order.
///
/// Directories and files compete in the same list on purpose. A size order
/// answers "what is the biggest thing here", and a rule that pinned every
/// directory above every file would answer a different question — leaving the
/// row worth opening first below three folders holding a line each.
enum Child<'a, 'p> {
    /// A directory, already merged with any chain hanging off it.
    Dir {
        /// The merged chain's label.
        label: String,
        /// What hangs beneath it.
        below: &'a Assembled<'p>,
        /// Its whole subtree's size.
        stat: Stat,
    },
    /// A file directly inside.
    File {
        /// Its name.
        name: &'p str,
        /// Its index into the caller's list.
        index: usize,
        /// Its size.
        stat: Stat,
    },
}

impl Child<'_, '_> {
    /// What this row weighs.
    const fn stat(&self) -> Stat {
        match self {
            Self::Dir { stat, .. } | Self::File { stat, .. } => *stat,
        }
    }
}

/// `dir`'s contents as rows: every chain of single-child directories merged
/// into one, directories before files, and the whole lot then put in `sort`'s
/// order.
fn children<'a, 'p>(dir: &'a Assembled<'p>, sort: Sort) -> Vec<Child<'a, 'p>> {
    let mut children = Vec::with_capacity(dir.dirs.len() + dir.files.len());
    for (name, child) in &dir.dirs {
        // Merge a chain of single-child directories into one row: while this
        // directory holds nothing but one directory, the two are one thing
        // with one name. The chain weighs what its foot weighs, which is what
        // the directory it started at weighs.
        let mut label = (*name).to_owned();
        let mut below = child;
        while let Some((name, only)) = below.lone_dir() {
            label.push('/');
            label.push_str(name);
            below = only;
        }
        children.push(Child::Dir {
            label,
            below,
            stat: child.stat,
        });
    }
    children.extend(dir.files.iter().map(|(name, index, stat)| Child::File {
        name,
        index: *index,
        stat: *stat,
    }));

    order(&mut children, sort, Child::stat);
    children
}

/// Writes `dir`'s contents onto `nodes` in `sort`'s order, each directory
/// followed immediately by its own subtree.
///
/// `prefix` is what a child's collapse key starts with — `""` at the root of
/// the bookmark view, `"<change>:"` at the root of a change's subtree, and the
/// parent's own key plus `/` below that.
fn flatten(
    dir: &Assembled<'_>,
    prefix: &str,
    depth: usize,
    collapsed: &HashSet<String>,
    sort: Sort,
    nodes: &mut Vec<Node>,
) {
    for child in children(dir, sort) {
        match child {
            Child::Dir { label, below, stat } => {
                let key = format!("{prefix}{label}");
                let folded = collapsed.contains(&key);
                nodes.push(Node {
                    label,
                    depth,
                    kind: NodeKind::Dir {
                        key: key.clone(),
                        collapsed: folded,
                    },
                    stat,
                });
                if !folded {
                    flatten(below, &format!("{key}/"), depth + 1, collapsed, sort, nodes);
                }
            }
            Child::File { name, index, stat } => nodes.push(file_node(name, index, depth, stat)),
        }
    }
}
