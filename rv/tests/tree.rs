//! Tests for the sidebar's node model: a flat list, a directory tree, or
//! commits holding the files they touched, each row carrying what it costs to
//! review and the rows ordered by that cost on request.
//!
//! [`rv::tree`] is pure — no terminal, no store, no app — so everything below
//! is a plain function call on string paths and a stats lookup. Three facts
//! matter more than the rest and are pinned as properties rather than
//! examples:
//!
//! * a chain of single-child directories is **one** row, because a 29-file
//!   review that spends half its rows on punctuation is worse than the flat
//!   list it replaced;
//! * the tree lists **exactly** the files the flat list does — never fewer,
//!   never more, never the same file twice. A tree that loses a file is worse
//!   than no tree, so it is checked over arbitrary path sets and not only over
//!   the paths someone thought of; and
//! * **an order only ever permutes those rows.** Under every [`Sort`] and both
//!   groupings the bag of rows is identical and only its sequence moves, which
//!   is the same conservation law as above with sorting added to it — a sort
//!   that loses a file is worse than no sort.

use std::collections::HashSet;

use proptest::prelude::*;
use rv::gradient::Stat;
use rv::tree::Group;
use rv::tree::Node;
use rv::tree::NodeKind;
use rv::tree::Sort;
use rv::tree::abbreviate;
use rv::tree::build;
use rv::tree::build_grouped;

/// Nothing folded away.
fn nothing() -> HashSet<String> {
    HashSet::new()
}

/// A review nobody measured. Every shape test wants this: a shape does not
/// depend on a weight, so the tests about rows say nothing about sizes.
fn unmeasured(_: usize) -> Stat {
    Stat::default()
}

/// The rows for `paths` in natural order and unmeasured — what every test
/// about *shape* asks for.
fn shape(paths: &[&str], collapsed: &HashSet<String>, tree: bool) -> Vec<Node> {
    build(paths, collapsed, tree, Sort::Natural, &unmeasured)
}

/// The commits view in natural order and unmeasured. See [`shape`].
fn grouped_shape(groups: &[Group<'_>], collapsed: &HashSet<String>, tree: bool) -> Vec<Node> {
    build_grouped(groups, collapsed, tree, Sort::Natural, &unmeasured)
}

/// A stats lookup addressed the way the tree addresses a file — by its index
/// into `paths` — written in the tests as the paths it measures, which is how
/// a reviewer would say it.
///
/// A path with no entry weighs nothing, which is what a rename or a binary
/// does.
fn stats_of<'a>(
    paths: &'a [&'a str],
    entries: &'a [(&'a str, u32, u32)],
) -> impl Fn(usize) -> Stat {
    move |index| {
        paths
            .get(index)
            .and_then(|path| entries.iter().find(|(name, _, _)| name == path))
            .map_or_else(Stat::default, |(_, added, removed)| Stat {
                added: *added,
                removed: *removed,
            })
    }
}

/// A stats lookup for the commits view, where the index runs *across* the
/// changes: the entries are given in that same concatenated order, so a file
/// two changes touched can weigh differently under each.
fn by_index(entries: &[(u32, u32)]) -> impl Fn(usize) -> Stat {
    move |index| {
        entries
            .get(index)
            .map_or_else(Stat::default, |(added, removed)| Stat {
                added: *added,
                removed: *removed,
            })
    }
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

/// The row labelled `label`, which the test expects to exist.
fn row<'a>(nodes: &'a [Node], label: &str) -> &'a Node {
    nodes
        .iter()
        .find(|node| node.label == label)
        .unwrap_or_else(|| panic!("no {label} row in {:?}", labels(nodes)))
}

/// A [`Stat`], written the way the tests read.
fn stat(added: u32, removed: u32) -> Stat {
    Stat { added, removed }
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

/// Every row as one comparable line, sorted — so two orderings can be compared
/// as bags rather than as sequences. This is what "only the order moves" is
/// measured against.
fn bag(nodes: &[Node]) -> Vec<String> {
    let mut rows: Vec<String> = nodes
        .iter()
        .map(|node| {
            let kind = match &node.kind {
                NodeKind::Commit {
                    change_id,
                    collapsed,
                } => format!("commit {change_id} {collapsed}"),
                NodeKind::Dir { key, collapsed } => format!("dir {key} {collapsed}"),
                NodeKind::File { index } => format!("file {index}"),
            };
            format!(
                "{} {} {kind} +{} -{}",
                node.depth, node.label, node.stat.added, node.stat.removed
            )
        })
        .collect();
    rows.sort();
    rows
}

// ---------------------------------------------------------------------------
// The bookmark view
// ---------------------------------------------------------------------------

#[test]
fn a_single_child_chain_collapses_into_one_row() {
    let nodes = shape(
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
    let nodes = shape(&paths, &nothing(), true);
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
    let nodes = shape(&["a.rs", "src/b.rs", "src/c.rs"], &collapsed, true);
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
    let nodes = shape(&["src/b.rs"], &HashSet::from(["src".to_owned()]), true);
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
    let nodes = shape(&paths, &HashSet::from(["src/deep".to_owned()]), true);

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
    let nodes = shape(
        &["src/a.rs", "src/deep/b.rs"],
        &HashSet::from(["src".to_owned()]),
        true,
    );
    assert_eq!(labels(&nodes), ["src"], "one row, and nothing beneath it");
}

#[test]
fn a_directory_holding_one_file_keeps_the_file_on_a_row_of_its_own() {
    let nodes = shape(&["src/only.rs", "top.rs"], &nothing(), true);
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
    let tree = shape(&["src/deep/a.rs"], &nothing(), true);
    assert_eq!(labels(&tree), ["src/deep", "a.rs"]);

    let flat = shape(&["src/deep/a.rs"], &nothing(), false);
    assert_eq!(
        labels(&flat),
        ["src/deep/a.rs"],
        "the flat list is unchanged"
    );
}

#[test]
fn the_flat_list_is_the_paths_in_order_at_depth_zero() {
    let paths = ["src/z.rs", "a.rs", "src/b.rs"];
    let nodes = shape(&paths, &HashSet::from(["src".to_owned()]), false);

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
    assert!(shape(&[], &nothing(), true).is_empty());
    assert!(shape(&[], &nothing(), false).is_empty());
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
    let nodes = shape(&paths, &nothing(), true);

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
    let key = match &shape(&paths, &nothing(), true)[0].kind {
        NodeKind::Dir { key, .. } => key.clone(),
        other => panic!("the first row is a directory, not {other:?}"),
    };
    assert_eq!(key, "docs/superpowers/specs");

    assert_eq!(
        labels(&shape(&paths, &HashSet::from([key]), true)),
        ["docs/superpowers/specs"],
        "folding the row under the cursor folds the chain it stands for"
    );
    assert_eq!(
        labels(&shape(&paths, &HashSet::from(["docs".to_owned()]), true)).len(),
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
        sketch(&shape(&paths, &collapsed, true)),
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
    let nodes = grouped_shape(&groups, &nothing(), true);

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
    let nodes = grouped_shape(&groups, &nothing(), false);

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
    let nodes = grouped_shape(&groups, &nothing(), false);
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
    let nodes = grouped_shape(&groups, &HashSet::from(["aaaa".to_owned()]), false);

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
    let nodes = grouped_shape(&groups, &nothing(), false);

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
    let nodes = grouped_shape(&groups, &nothing(), true);
    let key = nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Dir { key, .. } => Some(key.clone()),
            _ => None,
        })
        .expect("the first change has a directory row");

    let folded = grouped_shape(&groups, &HashSet::from([key]), true);
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
        sketch(&grouped_shape(&groups, &nothing(), true)),
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
        sketch(&grouped_shape(&groups, &nothing(), false)),
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
    assert_eq!(labels(&grouped_shape(&groups, &nothing(), true)).len(), 1);
}

// ---------------------------------------------------------------------------
// What a row costs to review
// ---------------------------------------------------------------------------

#[test]
fn every_file_row_carries_what_it_costs_to_review() {
    let paths = ["added.rs", "removed.rs"];
    let stats = stats_of(&paths, &[("added.rs", 40, 0), ("removed.rs", 0, 25)]);
    let nodes = build(&paths, &nothing(), false, Sort::Natural, &stats);

    assert_eq!(nodes[0].stat, stat(40, 0));
    assert_eq!(nodes[1].stat, stat(0, 25));
}

#[test]
fn a_directory_shows_its_subtrees_total() {
    let paths = ["src/a.rs", "src/b.rs"];
    let stats = stats_of(&paths, &[("src/a.rs", 10, 2), ("src/b.rs", 5, 3)]);
    let nodes = build(&paths, &nothing(), true, Sort::Natural, &stats);

    assert_eq!(
        row(&nodes, "src").stat,
        stat(15, 5),
        "a collapsed row that hides its weight is a row you must expand to judge"
    );
}

#[test]
fn a_folded_directory_still_says_what_it_is_hiding() {
    // The whole point: the number is what tells you whether to open it.
    let paths = ["src/a.rs", "src/deep/b.rs"];
    let stats = stats_of(&paths, &[("src/a.rs", 10, 2), ("src/deep/b.rs", 5, 3)]);
    let nodes = build(
        &paths,
        &HashSet::from(["src".to_owned()]),
        true,
        Sort::Natural,
        &stats,
    );

    assert_eq!(labels(&nodes), ["src"], "nothing beneath it");
    assert_eq!(nodes[0].stat, stat(15, 5), "and its whole subtree on it");
}

#[test]
fn a_merged_chain_totals_everything_below_the_chain() {
    let paths = ["docs/specs/a.md", "docs/specs/b.md"];
    let stats = stats_of(
        &paths,
        &[("docs/specs/a.md", 7, 1), ("docs/specs/b.md", 2, 4)],
    );
    let nodes = build(&paths, &nothing(), true, Sort::Natural, &stats);

    assert_eq!(row(&nodes, "docs/specs").stat, stat(9, 5));
}

#[test]
fn a_commit_row_totals_the_files_it_touched() {
    let groups = [
        Group {
            change_id: "aaaa",
            description: "first",
            paths: &["a.rs", "b.rs"],
        },
        Group {
            change_id: "bbbb",
            description: "second",
            paths: &["c.rs"],
        },
    ];
    let stats = by_index(&[(10, 1), (5, 2), (3, 3)]);
    let nodes = build_grouped(&groups, &nothing(), false, Sort::Natural, &stats);

    assert_eq!(
        nodes[0].stat,
        stat(15, 3),
        "a change is the sum of its files"
    );
    assert_eq!(row(&nodes, "bbbb second").stat, stat(3, 3));
}

#[test]
fn a_file_nobody_measured_weighs_nothing() {
    // A rename moved no line. Inventing a weight for it would sort it above
    // something that actually changed.
    let paths = ["renamed.rs"];
    let nodes = build(
        &paths,
        &nothing(),
        false,
        Sort::Natural,
        &stats_of(&paths, &[]),
    );
    assert_eq!(nodes[0].stat, Stat::default());
}

#[rstest::rstest]
#[case(0, "0")]
#[case(42, "42")]
#[case(999, "999")]
#[case(1000, "1k")]
#[case(1234, "1.2k")]
#[case(9999, "10k")]
#[case(45678, "46k")]
#[case(999_999, "1M")]
#[case(1_500_000, "1.5M")]
#[case(u32::MAX, "4.3G")]
fn large_counts_abbreviate(#[case] n: u32, #[case] expected: &str) {
    assert_eq!(abbreviate(n), expected);
}

#[test]
fn an_abbreviation_is_never_wider_than_the_column_it_has() {
    // The sidebar is narrow, and a count that overflows its column is what
    // this function exists to prevent. Four characters covers every u32.
    for n in [0, 9, 99, 999, 1000, 9999, 10_000, 999_999, u32::MAX] {
        let text = abbreviate(n);
        assert!(
            text.len() <= 4,
            "{n} abbreviated to {text}, which is wider than the column"
        );
    }
}

// ---------------------------------------------------------------------------
// Ordering
// ---------------------------------------------------------------------------

#[test]
fn sorting_by_additions_puts_the_biggest_first() {
    let paths = ["small.rs", "huge.rs", "mid.rs"];
    let stats = stats_of(
        &paths,
        &[("small.rs", 3, 0), ("huge.rs", 300, 0), ("mid.rs", 30, 0)],
    );
    let nodes = build(&paths, &nothing(), false, Sort::Added, &stats);

    assert_eq!(labels(&nodes), ["huge.rs", "mid.rs", "small.rs"]);
}

#[test]
fn sorting_by_removals_weighs_the_other_hand() {
    let paths = ["grew.rs", "shrank.rs"];
    let stats = stats_of(&paths, &[("grew.rs", 300, 1), ("shrank.rs", 0, 25)]);

    assert_eq!(
        labels(&build(&paths, &nothing(), false, Sort::Removed, &stats)),
        ["shrank.rs", "grew.rs"],
        "the biggest deletion first, whatever the additions say"
    );
    assert_eq!(
        labels(&build(&paths, &nothing(), false, Sort::Added, &stats)),
        ["grew.rs", "shrank.rs"],
        "and the other way under the other hand"
    );
}

#[test]
fn natural_is_the_order_the_thing_already_has() {
    // One mode rather than two, which is why one key serves both views: in the
    // bookmark view it is the order the paths arrived in, and in the commits
    // view it is the order of the stack.
    let paths = ["z.rs", "a.rs", "m.rs"];
    let stats = stats_of(&paths, &[("z.rs", 1, 0), ("a.rs", 99, 0), ("m.rs", 50, 0)]);
    assert_eq!(
        labels(&build(&paths, &nothing(), false, Sort::Natural, &stats)),
        paths,
        "path order, untouched by the weights"
    );

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
    let nodes = build_grouped(
        &groups,
        &nothing(),
        false,
        Sort::Natural,
        &by_index(&[(1, 0), (99, 0)]),
    );
    assert_eq!(
        labels(&nodes),
        ["aaaa first", "a.rs", "bbbb second", "b.rs"],
        "stack order, untouched by the weights"
    );
}

#[test]
fn sorting_does_not_flatten_the_tree() {
    // A reviewer asked for both; they compose. Siblings sort against each
    // other and directories keep their children.
    // The names run the other way from the sizes on purpose: alphabetical
    // order would pass this by accident.
    let paths = ["src/a_small.rs", "src/z_huge.rs", "top.rs"];
    let stats = stats_of(
        &paths,
        &[
            ("src/a_small.rs", 1, 0),
            ("src/z_huge.rs", 99, 0),
            ("top.rs", 50, 0),
        ],
    );
    let nodes = build(&paths, &nothing(), true, Sort::Added, &stats);

    assert!(
        nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Dir { .. })),
        "still a tree"
    );
    let under_src: Vec<&str> = nodes
        .iter()
        .filter(|node| node.depth > 0)
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(
        under_src,
        ["z_huge.rs", "a_small.rs"],
        "siblings sorted, nesting intact"
    );
}

#[test]
fn a_directory_sorts_among_its_siblings_by_its_aggregate() {
    let paths = ["a/x.rs", "b/y.rs", "b/z.rs"];
    let stats = stats_of(
        &paths,
        &[("a/x.rs", 1, 0), ("b/y.rs", 5, 0), ("b/z.rs", 5, 0)],
    );
    let nodes = build(&paths, &nothing(), true, Sort::Added, &stats);

    assert_eq!(
        dir_labels(&nodes),
        ["b", "a"],
        "b totals 10 and outranks a's 1"
    );
}

#[test]
fn a_heavy_file_outranks_a_light_directory_beside_it() {
    // Sorting answers "what is the biggest thing here". A rule that pinned
    // directories above files would answer a different question, and the row
    // worth opening first would sit below three trivial folders.
    let paths = ["trivial/tweak.rs", "rewritten.rs"];
    let stats = stats_of(
        &paths,
        &[("trivial/tweak.rs", 1, 0), ("rewritten.rs", 400, 0)],
    );
    let nodes = build(&paths, &nothing(), true, Sort::Added, &stats);

    assert_eq!(labels(&nodes), ["rewritten.rs", "trivial", "tweak.rs"]);
    assert_eq!(
        labels(&build(&paths, &nothing(), true, Sort::Natural, &stats)),
        ["trivial", "tweak.rs", "rewritten.rs"],
        "and natural order still draws the directories first"
    );
}

#[test]
fn a_tie_keeps_the_order_it_already_had() {
    // Two files of the same size have no reason to swap, and a sort that
    // reshuffles them makes the sidebar move under the cursor for nothing.
    let paths = ["z.rs", "a.rs", "m.rs"];
    let stats = stats_of(&paths, &[("z.rs", 7, 0), ("a.rs", 7, 0), ("m.rs", 7, 0)]);

    assert_eq!(
        labels(&build(&paths, &nothing(), false, Sort::Added, &stats)),
        paths,
        "the flat list keeps path order"
    );
    assert_eq!(
        labels(&build(&paths, &nothing(), true, Sort::Added, &stats)),
        ["a.rs", "m.rs", "z.rs"],
        "and the tree keeps the order it draws in"
    );
}

#[test]
fn commits_sort_by_what_they_weigh_and_keep_their_files() {
    let groups = [
        Group {
            change_id: "aaaa",
            description: "a tweak",
            paths: &["tweak.rs"],
        },
        Group {
            change_id: "bbbb",
            description: "the big one",
            paths: &["big.rs", "also.rs"],
        },
    ];
    let stats = by_index(&[(2, 0), (100, 0), (50, 0)]);
    let nodes = build_grouped(&groups, &nothing(), false, Sort::Added, &stats);

    assert_eq!(
        labels(&nodes),
        [
            "bbbb the big one",
            "big.rs",
            "also.rs",
            "aaaa a tweak",
            "tweak.rs"
        ],
        "a change is a directory here too: it sorts by its aggregate and takes its files with it"
    );
}

#[test]
fn sorting_the_commits_view_does_not_renumber_the_files() {
    // The index addresses the caller's own list of (change, file) pairs, which
    // is in stack order. Reordering the rows must not move what a row points
    // at, or every row would open the wrong diff.
    let groups = [
        Group {
            change_id: "aaaa",
            description: "a tweak",
            paths: &["tweak.rs"],
        },
        Group {
            change_id: "bbbb",
            description: "the big one",
            paths: &["big.rs"],
        },
    ];
    let stats = by_index(&[(2, 0), (100, 0)]);
    let nodes = build_grouped(&groups, &nothing(), false, Sort::Added, &stats);

    assert_eq!(
        labels(&nodes)[0],
        "bbbb the big one",
        "the rows did move, which is what makes the rest of this test worth asserting"
    );
    let big = row(&nodes, "big.rs");
    assert_eq!(
        big.kind,
        NodeKind::File { index: 1 },
        "still the second pair"
    );
    assert_eq!(
        row(&nodes, "tweak.rs").kind,
        NodeKind::File { index: 0 },
        "and the first is still the first"
    );
}

#[test]
fn the_order_cycles_natural_added_removed() {
    // One key, three modes, and the name the sidebar's title says.
    assert_eq!(Sort::default(), Sort::Natural);
    assert_eq!(Sort::Natural.next(), Sort::Added);
    assert_eq!(Sort::Added.next(), Sort::Removed);
    assert_eq!(Sort::Removed.next(), Sort::Natural, "it cycles");

    assert_eq!(Sort::Natural.label(), "natural");
    assert_eq!(Sort::Added.label(), "added");
    assert_eq!(Sort::Removed.label(), "removed");
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

/// Every order there is.
fn orders() -> impl Strategy<Value = Sort> {
    prop_oneof![Just(Sort::Natural), Just(Sort::Added), Just(Sort::Removed),]
}

/// Sizes with plenty of ties and plenty of zeroes, since those are the cases
/// an order is most likely to mishandle.
fn sizes() -> impl Strategy<Value = Vec<(u32, u32)>> {
    prop::collection::vec((0u32..4, 0u32..4), 0..14)
}

proptest! {
    /// The one that matters: every file, exactly once, whatever the paths.
    #[test]
    fn the_tree_never_loses_or_invents_a_file(paths in paths()) {
        let borrowed = borrowed(&paths);
        let mut indices = file_indices(&shape(&borrowed, &nothing(), true));
        indices.sort_unstable();
        prop_assert_eq!(indices, (0..paths.len()).collect::<Vec<_>>());
    }

    /// Folding may hide a file but must never duplicate one or conjure an
    /// index that addresses nothing.
    #[test]
    fn folding_only_ever_hides(paths in paths(), fold in prop::collection::vec("[a-c.]{0,2}", 0..4)) {
        let borrowed = borrowed(&paths);
        let collapsed: HashSet<String> = fold.into_iter().collect();
        let nodes = shape(&borrowed, &collapsed, true);
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
    fn depth_never_jumps_by_more_than_one(paths in paths(), sort in orders()) {
        let borrowed = borrowed(&paths);
        let nodes = build(&borrowed, &nothing(), true, sort, &unmeasured);
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
        let nodes = shape(&borrowed, &nothing(), true);
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

        let nodes = grouped_shape(&groups, &nothing(), tree);
        let mut indices = file_indices(&nodes);
        indices.sort_unstable();
        prop_assert_eq!(indices, (0..total).collect::<Vec<_>>());
    }

    /// Conservation under sorting: under every order and both groupings the
    /// set of files is unchanged and only the sequence moves. A sort that
    /// loses a file is worse than no sort.
    #[test]
    fn no_order_loses_or_invents_a_file(
        paths in paths(),
        stats in sizes(),
        sort in orders(),
        tree in any::<bool>(),
    ) {
        let borrowed = borrowed(&paths);
        let stat_of = by_index(&stats);
        let mut indices = file_indices(&build(&borrowed, &nothing(), tree, sort, &stat_of));
        indices.sort_unstable();
        prop_assert_eq!(indices, (0..paths.len()).collect::<Vec<_>>());
    }

    /// The stronger form: an order does not merely keep the files, it keeps
    /// every *row* — same labels, same depths, same keys, same weights — and
    /// changes nothing but their sequence.
    #[test]
    fn an_order_only_permutes_the_rows(
        paths in paths(),
        stats in sizes(),
        sort in orders(),
        tree in any::<bool>(),
        fold in prop::collection::vec("[a-c.]{0,2}", 0..4),
    ) {
        let borrowed = borrowed(&paths);
        let collapsed: HashSet<String> = fold.into_iter().collect();
        let stat_of = by_index(&stats);
        let natural = build(&borrowed, &collapsed, tree, Sort::Natural, &stat_of);
        let sorted = build(&borrowed, &collapsed, tree, sort, &stat_of);
        prop_assert_eq!(bag(&sorted), bag(&natural));
    }

    /// And the same in the commits view, where an order also moves the changes
    /// themselves.
    #[test]
    fn no_order_loses_a_file_in_the_commits_view(
        groups in prop::collection::vec(paths(), 0..4),
        stats in sizes(),
        sort in orders(),
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
        let stat_of = by_index(&stats);

        let sorted = build_grouped(&groups, &nothing(), tree, sort, &stat_of);
        let mut indices = file_indices(&sorted);
        indices.sort_unstable();
        prop_assert_eq!(indices, (0..total).collect::<Vec<_>>());
        prop_assert_eq!(
            bag(&sorted),
            bag(&build_grouped(&groups, &nothing(), tree, Sort::Natural, &stat_of)),
            "only the order moves"
        );
    }

    /// A directory stands for its subtree, so its weight is its subtree's —
    /// whatever the shape, whatever the order, folded or not.
    #[test]
    fn a_parent_weighs_what_hangs_beneath_it(paths in paths(), stats in sizes(), sort in orders()) {
        let borrowed = borrowed(&paths);
        let stat_of = by_index(&stats);
        let nodes = build(&borrowed, &nothing(), true, sort, &stat_of);

        for (at, node) in nodes.iter().enumerate() {
            if !matches!(node.kind, NodeKind::Dir { .. }) {
                continue;
            }
            let subtree: Stat = nodes[at + 1..]
                .iter()
                .take_while(|below| below.depth > node.depth)
                .filter(|below| matches!(below.kind, NodeKind::File { .. }))
                .fold(Stat::default(), |total, below| total + below.stat);
            prop_assert_eq!(node.stat, subtree, "{} does not weigh its subtree", node.label);
        }
    }
}
