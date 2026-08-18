//! The sidebar's rows: a flat list, a directory tree, or commits holding the
//! files they touched — one model with three node kinds rather than three
//! widgets that would drift apart.
//!
//! A commit is a directory in every respect that matters here: it holds
//! children, it folds away under the same key, and its subtree is what a
//! summary over it is computed from. Making it a third [`NodeKind`] rather
//! than a second widget means one selection model, one collapse rule and one
//! place to walk a subtree; three of anything would drift.
//!
//! Two rules earn the module its existence:
//!
//! * **A chain of single-child directories is one row.** `docs/superpowers/
//!   specs` is one row and not three, because a 29-file review has perhaps 40
//!   rows to spend and a tree that spends half of them on punctuation is worse
//!   than the flat list it replaced.
//! * **The tree lists exactly the files the flat list does.** Every path in,
//!   exactly one [`NodeKind::File`] out, carrying that path's index — so a
//!   file can never be lost behind a directory that was drawn wrong.
//!
//! Nothing here knows about ratatui: a node is a label, a depth and a kind,
//! and the renderer owns every glyph, colour and column of it. Nothing here is
//! review state either — which rows are folded is a session-only preference
//! held by the caller, and none of it reaches `.review/`.

use std::collections::BTreeMap;
use std::collections::HashSet;

/// What a change with an empty description is labelled with, matching what jj
/// itself shows.
const NO_DESCRIPTION: &str = "(no description set)";

/// Separates a change's id from a path inside the collapse key of a row in the
/// commits view. A jj change id is letters only and a path may not contain a
/// colon on every platform rv supports, so the two halves can always be told
/// apart — and `src` under one change folds without folding `src` under the
/// next.
const KEY_SEPARATOR: char = ':';

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
/// false, because a list has nothing to fold. With `tree` false the answer is
/// exactly the list the sidebar has always drawn: one row per path, in the
/// order given, at depth 0.
pub fn build(paths: &[&str], collapsed: &HashSet<String>, tree: bool) -> Vec<Node> {
    let files = paths.iter().copied().enumerate();
    if !tree {
        return files
            .map(|(index, path)| file_node(path, index, 0))
            .collect();
    }

    let mut nodes = Vec::with_capacity(paths.len());
    flatten(&assemble(files), "", 0, collapsed, &mut nodes);
    nodes
}

/// The commits view: each change holds the files it touched, and `tree`
/// chooses whether those files are a directory tree or a flat list beneath it.
///
/// File indices run *across* the changes: the first path of the first group is
/// 0 and the numbering carries on into the next group, so a file two changes
/// touched gets one index per change and each row addresses that change's diff
/// of it. The caller reads its own list of (change, file) pairs in the same
/// order and the indices line up.
pub fn build_grouped(groups: &[Group<'_>], collapsed: &HashSet<String>, tree: bool) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut next = 0;
    for group in groups {
        let folded = collapsed.contains(group.change_id);
        nodes.push(Node {
            label: commit_label(group),
            depth: 0,
            kind: NodeKind::Commit {
                change_id: group.change_id.to_owned(),
                collapsed: folded,
            },
        });

        let base = next;
        next += group.paths.len();
        let files = group
            .paths
            .iter()
            .copied()
            .enumerate()
            .map(move |(at, path)| (base + at, path));
        if folded {
            continue;
        }
        if tree {
            let prefix = format!("{}{KEY_SEPARATOR}", group.change_id);
            flatten(&assemble(files), &prefix, 1, collapsed, &mut nodes);
        } else {
            nodes.extend(files.map(|(index, path)| file_node(path, index, 1)));
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
fn file_node(label: &str, index: usize, depth: usize) -> Node {
    Node {
        label: label.to_owned(),
        depth,
        kind: NodeKind::File { index },
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
    /// The files directly inside, as `(name, index)`.
    files: Vec<(&'a str, usize)>,
}

/// Sorts every path into the directory that holds it.
///
/// A path is split on `/`: the last segment is the file's name and the rest
/// are the directories above it, so `a/b.rs` is `b.rs` inside `a`, and a path
/// with no separator is a file at the root. Empty segments are kept rather
/// than skipped — a repository is where these come from, and dropping one
/// would lose the file that owns it.
fn assemble<'a>(files: impl Iterator<Item = (usize, &'a str)>) -> Assembled<'a> {
    let mut root = Assembled::default();
    for (index, path) in files {
        let mut segments = path.split('/').peekable();
        let mut at = &mut root;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                at.files.push((segment, index));
                break;
            }
            at = at.dirs.entry(segment).or_default();
        }
    }
    root.sort();
    root
}

impl Assembled<'_> {
    /// Puts every level in drawing order: files by name, with two files of the
    /// same name left in the order they arrived so their indices stay
    /// ascending. Directories are already ordered by the `BTreeMap` holding
    /// them, and are drawn before the files.
    fn sort(&mut self) {
        self.files.sort_by_key(|(name, _)| *name);
        for dir in self.dirs.values_mut() {
            dir.sort();
        }
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

/// Writes `dir`'s contents onto `nodes` in drawing order: directories first,
/// each followed immediately by its own subtree, then the files.
///
/// `prefix` is what a child's collapse key starts with — `""` at the root of
/// the bookmark view, `"<change>:"` at the root of a change's subtree, and the
/// parent's own key plus `/` below that.
fn flatten(
    dir: &Assembled<'_>,
    prefix: &str,
    depth: usize,
    collapsed: &HashSet<String>,
    nodes: &mut Vec<Node>,
) {
    for (name, child) in &dir.dirs {
        // Merge a chain of single-child directories into one row: while this
        // directory holds nothing but one directory, the two are one thing
        // with one name.
        let mut label = (*name).to_owned();
        let mut child = child;
        while let Some((name, only)) = child.lone_dir() {
            label.push('/');
            label.push_str(name);
            child = only;
        }

        let key = format!("{prefix}{label}");
        let folded = collapsed.contains(&key);
        nodes.push(Node {
            label,
            depth,
            kind: NodeKind::Dir {
                key: key.clone(),
                collapsed: folded,
            },
        });
        if !folded {
            flatten(child, &format!("{key}/"), depth + 1, collapsed, nodes);
        }
    }

    nodes.extend(
        dir.files
            .iter()
            .map(|(name, index)| file_node(name, *index, depth)),
    );
}
