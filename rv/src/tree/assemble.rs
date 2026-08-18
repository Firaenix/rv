//! Building the directory tree, and flattening it back into rows.

use std::collections::BTreeMap;
use std::collections::HashSet;

use super::Node;
use super::NodeKind;
use super::sort::Sort;
use super::sort::order;
use crate::gradient::Stat;

/// A file's row, labelled with `label`.
pub(super) fn file_node(label: &str, index: usize, depth: usize, stat: Stat) -> Node {
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
pub(super) struct Assembled<'a> {
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
pub(super) fn assemble<'a>(
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
pub(super) enum Child<'a, 'p> {
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
pub(super) fn children<'a, 'p>(dir: &'a Assembled<'p>, sort: Sort) -> Vec<Child<'a, 'p>> {
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
pub(super) fn flatten(
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
