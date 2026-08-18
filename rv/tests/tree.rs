//! Tests for the sidebar's node model: a flat list, a directory tree, or
//! commits holding the files they touched.
//!
//! [`rv::tree`] is pure — no terminal, no store, no app — so everything below
//! is a plain function call on string paths. Two facts matter more than the
//! rest and are pinned as properties rather than examples:
//!
//! * a chain of single-child directories is **one** row, because a 29-file
//!   review that spends half its rows on punctuation is worse than the flat
//!   list it replaced; and
//! * the tree lists **exactly** the files the flat list does — never fewer,
//!   never more, never the same file twice. A tree that loses a file is worse
//!   than no tree, so it is checked over arbitrary path sets and not only over
//!   the paths someone thought of.

use std::collections::HashSet;

use proptest::prelude::*;
use rv::tree::Group;
use rv::tree::Node;
use rv::tree::NodeKind;
use rv::tree::build;
use rv::tree::build_grouped;

/// Nothing folded away.
fn nothing() -> HashSet<String> {
    HashSet::new()
}

/// The labels of every row, in order.
fn labels(nodes: &[Node]) -> Vec<&str> {
    nodes.iter().map(|node| node.label.as_str()).collect()
}

/// The labels of the directory rows, in order.
fn dir_labels(nodes: &[Node]) -> Vec<&str> {
    nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Dir { .. }))
        .map(|node| node.label.as_str())
        .collect()
}

/// Every file index the rows carry, in row order.
fn file_indices(nodes: &[Node]) -> Vec<usize> {
    nodes
        .iter()
        .filter_map(|node| match node.kind {
            NodeKind::File { index } => Some(index),
            _ => None,
        })
        .collect()
}

/// The rows drawn the way the sidebar will draw them: indented by depth, with
/// a fold marker on everything that holds children. Assertions against this
/// are how a shape stays readable rather than merely correct.
fn sketch(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|node| {
            let marker = match &node.kind {
                NodeKind::Commit { collapsed, .. } | NodeKind::Dir { collapsed, .. } => {
                    if *collapsed {
                        "> "
                    } else {
                        "v "
                    }
                }
                NodeKind::File { .. } => "  ",
            };
            format!("{}{marker}{}", "  ".repeat(node.depth), node.label)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// The bookmark view
// ---------------------------------------------------------------------------

#[test]
fn a_single_child_chain_collapses_into_one_row() {
    let nodes = build(
        &["docs/superpowers/specs/a.md", "docs/superpowers/specs/b.md"],
        &nothing(),
        true,
    );
    assert_eq!(
        dir_labels(&nodes),
        ["docs/superpowers/specs"],
        "one row, not three"
    );
}

#[test]
fn the_tree_lists_exactly_the_files_the_flat_list_does() {
    let paths = ["a.rs", "src/b.rs", "src/deep/c.rs", "d.rs"];
    let nodes = build(&paths, &nothing(), true);
    let mut files = file_indices(&nodes);
    files.sort_unstable();
    assert_eq!(
        files,
        [0, 1, 2, 3],
        "a tree that loses a file is worse than no tree"
    );
}

#[test]
fn a_collapsed_directory_hides_its_files_but_stays_visible() {
    let collapsed = HashSet::from(["src".to_owned()]);
    let nodes = build(&["a.rs", "src/b.rs", "src/c.rs"], &collapsed, true);
    assert!(
        nodes.iter().any(|node| node.label == "src"),
        "the directory row remains"
    );
    assert!(
        !nodes.iter().any(|node| node.label.ends_with("b.rs")),
        "its children are hidden"
    );
    assert!(
        nodes.iter().any(|node| node.label.ends_with("a.rs")),
        "siblings are unaffected"
    );
}

#[test]
fn a_collapsed_directory_says_so_on_its_own_row() {
    let nodes = build(&["src/b.rs"], &HashSet::from(["src".to_owned()]), true);
    assert!(
        nodes.iter().any(|node| matches!(
            node.kind,
            NodeKind::Dir {
                collapsed: true,
                ..
            }
        )),
        "the row carries the fold state, so the renderer never has to consult the set itself"
    );
}

#[test]
fn a_directory_is_keyed_by_its_whole_path_not_its_last_segment() {
    let paths = ["src/a.rs", "src/deep/b.rs", "other/deep/c.rs"];
    let nodes = build(&paths, &HashSet::from(["src/deep".to_owned()]), true);

    assert!(
        !nodes.iter().any(|node| node.label.ends_with("b.rs")),
        "the named directory folded"
    );
    assert!(
        nodes.iter().any(|node| node.label.ends_with("c.rs")),
        "a directory that merely shares its last segment did not"
    );
    assert!(
        nodes.iter().any(|node| node.label.ends_with("a.rs")),
        "and its parent's own files are untouched"
    );
}

#[test]
fn folding_a_parent_hides_the_whole_subtree() {
    let nodes = build(
        &["src/a.rs", "src/deep/b.rs"],
        &HashSet::from(["src".to_owned()]),
        true,
    );
    assert_eq!(labels(&nodes), ["src"], "one row, and nothing beneath it");
}

#[test]
fn a_directory_holding_one_file_keeps_the_file_on_a_row_of_its_own() {
    let nodes = build(&["src/only.rs", "top.rs"], &nothing(), true);
    assert_eq!(
        dir_labels(&nodes),
        ["src"],
        "a chain of directories merges; a directory and a file do not"
    );
    assert!(
        nodes
            .iter()
            .any(|node| node.label == "only.rs" && node.depth == 1),
        "the file hangs under it"
    );
}

#[test]
fn a_file_row_is_labelled_with_its_name_and_a_flat_row_with_its_path() {
    let tree = build(&["src/deep/a.rs"], &nothing(), true);
    assert_eq!(labels(&tree), ["src/deep", "a.rs"]);

    let flat = build(&["src/deep/a.rs"], &nothing(), false);
    assert_eq!(
        labels(&flat),
        ["src/deep/a.rs"],
        "the flat list is unchanged"
    );
}

#[test]
fn the_flat_list_is_the_paths_in_order_at_depth_zero() {
    let paths = ["src/z.rs", "a.rs", "src/b.rs"];
    let nodes = build(&paths, &HashSet::from(["src".to_owned()]), false);

    assert_eq!(labels(&nodes), paths, "input order, untouched");
    assert_eq!(file_indices(&nodes), [0, 1, 2]);
    assert!(
        nodes.iter().all(|node| node.depth == 0),
        "a list has no depth"
    );
    assert!(
        nodes
            .iter()
            .all(|node| matches!(node.kind, NodeKind::File { .. })),
        "and no directories to fold"
    );
}

#[test]
fn a_review_with_no_files_is_no_rows() {
    assert!(build(&[], &nothing(), true).is_empty());
    assert!(build(&[], &nothing(), false).is_empty());
}

#[test]
fn a_real_review_reads_as_a_tree() {
    let paths = [
        "README.md",
        "docs/superpowers/plans/2026-08-18-rv-viewport.md",
        "docs/superpowers/specs/2026-08-18-rv-viewport-design.md",
        "rv-core/src/store.rs",
        "rv-core/tests/constraints.rs",
        "rv-core/tests/store.rs",
        "rv/src/app.rs",
        "rv/src/tree.rs",
        "rv/tests/tree.rs",
    ];
    let nodes = build(&paths, &nothing(), true);

    assert_eq!(
        sketch(&nodes),
        "\
v docs/superpowers
  v plans
      2026-08-18-rv-viewport.md
  v specs
      2026-08-18-rv-viewport-design.md
v rv
  v src
      app.rs
      tree.rs
  v tests
      tree.rs
v rv-core
  v src
      store.rs
  v tests
      constraints.rs
      store.rs
  README.md",
        "directories first and in order, files under them, nine files in fifteen rows"
    );
}

#[test]
fn a_merged_chain_folds_under_its_whole_label() {
    // Merging is what gives the row its identity: there is no `docs` row to
    // fold, so `docs` is not the key — the key is the row the reviewer's
    // cursor is actually on, which is the whole chain.
    let paths = ["docs/superpowers/specs/a.md", "docs/superpowers/specs/b.md"];
    let key = match &build(&paths, &nothing(), true)[0].kind {
        NodeKind::Dir { key, .. } => key.clone(),
        other => panic!("the first row is a directory, not {other:?}"),
    };
    assert_eq!(key, "docs/superpowers/specs");

    assert_eq!(
        labels(&build(&paths, &HashSet::from([key]), true)),
        ["docs/superpowers/specs"],
        "folding the row under the cursor folds the chain it stands for"
    );
    assert_eq!(
        labels(&build(&paths, &HashSet::from(["docs".to_owned()]), true)).len(),
        3,
        "and a key no row carries folds nothing"
    );
}

#[test]
fn a_folded_review_is_its_top_level_and_nothing_else() {
    let paths = [
        "README.md",
        "docs/superpowers/plans/a.md",
        "rv-core/src/store.rs",
        "rv-core/tests/store.rs",
        "rv/src/app.rs",
        "rv/tests/app.rs",
    ];
    let collapsed = HashSet::from([
        "docs/superpowers/plans".to_owned(),
        "rv".to_owned(),
        "rv-core".to_owned(),
    ]);

    assert_eq!(
        sketch(&build(&paths, &collapsed, true)),
        "\
> docs/superpowers/plans
> rv
> rv-core
  README.md",
        "four rows for four things, each one keystroke from opening again"
    );
}

// ---------------------------------------------------------------------------
// The commits view
// ---------------------------------------------------------------------------

#[test]
fn a_commit_holds_its_files_the_way_a_directory_holds_its_own() {
    let groups = [
        Group {
            change_id: "ytskpxpw",
            description: "close the alias bypass",
            paths: &["rv-core/tests/constraints.rs"],
        },
        Group {
            change_id: "zmomvwzm",
            description: "enforce the constraints",
            paths: &["rv-core/src/store.rs", "rv-core/tests/store.rs"],
        },
    ];
    let nodes = build_grouped(&groups, &nothing(), true);

    let commits: Vec<&str> = nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Commit { change_id, .. } => Some(change_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        commits,
        ["ytskpxpw", "zmomvwzm"],
        "one node per change, in order"
    );
    assert!(
        nodes
            .iter()
            .all(|node| matches!(node.kind, NodeKind::Commit { .. }) || node.depth > 0),
        "everything else hangs beneath a commit"
    );
}

#[test]
fn a_commit_row_reads_as_the_change_it_is() {
    let groups = [Group {
        change_id: "ytskpxpw",
        description: "close the alias bypass\n\nthe second line is not the subject",
        paths: &["a.rs"],
    }];
    let nodes = build_grouped(&groups, &nothing(), false);

    assert_eq!(
        nodes[0].label, "ytskpxpw close the alias bypass",
        "the change and its subject, on one row"
    );
}

#[test]
fn a_change_nobody_described_still_says_which_change_it_is() {
    let groups = [Group {
        change_id: "ytskpxpw",
        description: "",
        paths: &["a.rs"],
    }];
    let nodes = build_grouped(&groups, &nothing(), false);
    assert_eq!(nodes[0].label, "ytskpxpw (no description set)");
}

#[test]
fn collapsing_a_commit_hides_its_files_and_leaves_its_siblings_alone() {
    let groups = [
        Group {
            change_id: "aaaa",
            description: "first",
            paths: &["a.rs"],
        },
        Group {
            change_id: "bbbb",
            description: "second",
            paths: &["b.rs"],
        },
    ];
    let nodes = build_grouped(&groups, &HashSet::from(["aaaa".to_owned()]), false);

    assert!(
        nodes.iter().any(
            |node| matches!(&node.kind, NodeKind::Commit { change_id, collapsed } if change_id == "aaaa" && *collapsed)
        ),
        "the commit row remains, and says it is folded"
    );
    assert!(
        !nodes.iter().any(|node| node.label.ends_with("a.rs")),
        "its files are hidden"
    );
    assert!(
        nodes.iter().any(|node| node.label.ends_with("b.rs")),
        "the other change is untouched"
    );
}

#[test]
fn a_file_touched_by_two_commits_appears_under_each() {
    let groups = [
        Group {
            change_id: "aaaa",
            description: "first",
            paths: &["shared.rs"],
        },
        Group {
            change_id: "bbbb",
            description: "second",
            paths: &["shared.rs"],
        },
    ];
    let nodes = build_grouped(&groups, &nothing(), false);

    let count = nodes
        .iter()
        .filter(|node| node.label.ends_with("shared.rs"))
        .count();
    assert_eq!(
        count, 2,
        "each change shows what it touched, not what is unique to it"
    );
    assert_eq!(
        file_indices(&nodes),
        [0, 1],
        "and the two rows address two different diffs"
    );
}

#[test]
fn the_same_directory_under_two_changes_folds_independently() {
    let groups = [
        Group {
            change_id: "aaaa",
            description: "first",
            paths: &["src/a.rs"],
        },
        Group {
            change_id: "bbbb",
            description: "second",
            paths: &["src/b.rs"],
        },
    ];
    let nodes = build_grouped(&groups, &nothing(), true);
    let key = nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Dir { key, .. } => Some(key.clone()),
            _ => None,
        })
        .expect("the first change has a directory row");

    let folded = build_grouped(&groups, &HashSet::from([key]), true);
    assert!(
        !folded.iter().any(|node| node.label.ends_with("a.rs")),
        "the directory named by the key folded"
    );
    assert!(
        folded.iter().any(|node| node.label.ends_with("b.rs")),
        "the identically named directory under the other change did not"
    );
}

#[test]
fn a_commits_view_reads_as_changes_holding_files() {
    let groups = [
        Group {
            change_id: "ytskpxpw",
            description: "close the alias bypass",
            paths: &["rv-core/tests/constraints.rs"],
        },
        Group {
            change_id: "zmomvwzm",
            description: "enforce the constraints",
            paths: &["rv-core/src/store.rs", "rv-core/tests/store.rs"],
        },
    ];

    assert_eq!(
        sketch(&build_grouped(&groups, &nothing(), true)),
        "\
v ytskpxpw close the alias bypass
  v rv-core/tests
      constraints.rs
v zmomvwzm enforce the constraints
  v rv-core
    v src
        store.rs
    v tests
        store.rs"
    );

    assert_eq!(
        sketch(&build_grouped(&groups, &nothing(), false)),
        "\
v ytskpxpw close the alias bypass
    rv-core/tests/constraints.rs
v zmomvwzm enforce the constraints
    rv-core/src/store.rs
    rv-core/tests/store.rs",
        "the same rows, flat"
    );
}

#[test]
fn a_change_with_no_files_is_still_a_row() {
    let groups = [Group {
        change_id: "aaaa",
        description: "an empty change",
        paths: &[],
    }];
    assert_eq!(labels(&build_grouped(&groups, &nothing(), true)).len(), 1);
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// Path sets that include the awkward ones: empty segments, dots, repeats and
/// a path that is a prefix of another. The tree has to survive all of them,
/// because the paths come from a repository and not from this file.
fn paths() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop::collection::vec("[a-c.]{0,2}", 1..4).prop_map(|segments| segments.join("/")),
        0..14,
    )
}

fn borrowed(paths: &[String]) -> Vec<&str> {
    paths.iter().map(String::as_str).collect()
}

proptest! {
    /// The one that matters: every file, exactly once, whatever the paths.
    #[test]
    fn the_tree_never_loses_or_invents_a_file(paths in paths()) {
        let borrowed = borrowed(&paths);
        let mut indices = file_indices(&build(&borrowed, &nothing(), true));
        indices.sort_unstable();
        prop_assert_eq!(indices, (0..paths.len()).collect::<Vec<_>>());
    }

    /// Folding may hide a file but must never duplicate one or conjure an
    /// index that addresses nothing.
    #[test]
    fn folding_only_ever_hides(paths in paths(), fold in prop::collection::vec("[a-c.]{0,2}", 0..4)) {
        let borrowed = borrowed(&paths);
        let collapsed: HashSet<String> = fold.into_iter().collect();
        let nodes = build(&borrowed, &collapsed, true);
        let indices = file_indices(&nodes);
        let unique: HashSet<usize> = indices.iter().copied().collect();

        prop_assert_eq!(indices.len(), unique.len(), "no file is listed twice");
        prop_assert!(indices.iter().all(|index| *index < paths.len()), "no index is invented");
        prop_assert!(indices.len() <= paths.len());
    }

    /// A flattened tree: the first row is a root, and a row is at most one
    /// level deeper than the row before it. Anything else is an indent that
    /// hangs off nothing.
    #[test]
    fn depth_never_jumps_by_more_than_one(paths in paths()) {
        let borrowed = borrowed(&paths);
        let nodes = build(&borrowed, &nothing(), true);
        let mut previous = None;
        for node in &nodes {
            match previous {
                None => prop_assert_eq!(node.depth, 0, "the first row is a root"),
                Some(before) => prop_assert!(node.depth <= before + 1, "an indent hangs off nothing"),
            }
            previous = Some(node.depth);
        }
    }

    /// The chain rule, as a property rather than an example: no directory that
    /// is drawn open has exactly one child and that child a directory. Every
    /// such pair would be two rows saying one thing.
    #[test]
    fn no_open_directory_has_a_lone_directory_child(paths in paths()) {
        let borrowed = borrowed(&paths);
        let nodes = build(&borrowed, &nothing(), true);
        for (at, node) in nodes.iter().enumerate() {
            if !matches!(node.kind, NodeKind::Dir { collapsed: false, .. }) {
                continue;
            }
            let children: Vec<&Node> = nodes[at + 1..]
                .iter()
                .take_while(|below| below.depth > node.depth)
                .filter(|below| below.depth == node.depth + 1)
                .collect();
            let lone_dir = children.len() == 1
                && matches!(children[0].kind, NodeKind::Dir { .. });
            prop_assert!(!lone_dir, "{} should have merged with its only child", node.label);
        }
    }

    /// The same conservation law in the commits view, where the indices run
    /// across the changes: `groups[0]`'s first path is 0 and the numbering
    /// carries on into the next change.
    #[test]
    fn every_change_lists_every_file_it_touched(
        groups in prop::collection::vec(paths(), 0..4),
        tree in any::<bool>(),
    ) {
        let borrowed: Vec<Vec<&str>> = groups.iter().map(|paths| borrowed(paths)).collect();
        let groups: Vec<Group<'_>> = borrowed
            .iter()
            .enumerate()
            .map(|(at, paths)| Group {
                change_id: ["aaaa", "bbbb", "cccc", "dddd"][at],
                description: "a change",
                paths,
            })
            .collect();
        let total: usize = groups.iter().map(|group| group.paths.len()).sum();

        let nodes = build_grouped(&groups, &nothing(), tree);
        let mut indices = file_indices(&nodes);
        indices.sort_unstable();
        prop_assert_eq!(indices, (0..total).collect::<Vec<_>>());
    }
}
