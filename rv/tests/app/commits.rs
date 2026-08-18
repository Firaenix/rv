//! The commits tab: which change touched which file.

use crossterm::event::KeyCode;
use rv::app::SidebarTab;
use rv::tree::NodeKind;

use rv::app::App;

use crate::support::*;

/// A workspace whose changes touch different files, so a row can only be under
/// the right change by being put there.
///
/// Three changes rather than two: `Fixture::new` ends on `jj new`, so the range
/// carries an empty working-copy change as well as the two described ones. That
/// empty change is part of the stack and is listed like any other — a reviewer
/// who cannot see it cannot tell an empty change from a missing one.
fn two_changes() -> Fixture {
    let fixture = Fixture::new();
    fixture.write("c.rs", "fn c() {\n    let c = 3;\n}\n");
    fixture.jj(&["describe", "-m", "a second change"]);
    fixture.jj(&["new"]);
    fixture
}


/// Walks the commits list down to its first file row, whichever change holds
/// it. The first change of a stack is often the empty working copy, which has
/// no files under it at all.
fn down_to_a_file(app: &mut App) {
    for _ in 0..20 {
        if matches!(
            app.commit_nodes().get(app.sidebar_row()).map(|n| &n.kind),
            Some(NodeKind::File { .. })
        ) {
            return;
        }
        app.on_key(KeyCode::Down).expect("next row");
    }
    panic!("no file row in the commits list");
}

/// The same for a change row that actually holds files.
fn down_to_a_change_with_files(app: &mut App) {
    for _ in 0..20 {
        let nodes = app.commit_nodes();
        let row = app.sidebar_row();
        let is_full_change = matches!(nodes.get(row).map(|n| &n.kind), Some(NodeKind::Commit { .. }))
            && matches!(nodes.get(row + 1).map(|n| &n.kind), Some(NodeKind::File { .. }));
        if is_full_change {
            return;
        }
        app.on_key(KeyCode::Down).expect("next row");
    }
    panic!("no change with files in the commits list");
}

#[test]
fn tab_reaches_the_commits_tab_between_the_files_and_the_comments() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Tab).expect("tab");

    assert_eq!(app.sidebar_tab(), SidebarTab::Commits);
}

/// The point of the tab: a change is a row, and the files it touched hang off
/// it.
#[test]
fn each_change_is_a_row_holding_the_files_it_touched() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);

    let nodes = app.commit_nodes();
    let commits: Vec<&str> = nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Commit { .. }))
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(
        commits.len(),
        app.changes().len(),
        "not one row per change: {nodes:?}"
    );
    assert!(
        commits
            .iter()
            .any(|label| label.contains("a second change")),
        "a change's row does not name it: {commits:?}"
    );

    // Every file row sits below the change that touched it, which is what the
    // depths say.
    for node in &nodes {
        if matches!(node.kind, NodeKind::File { .. }) {
            assert!(node.depth > 0, "a file row is not under a change: {node:?}");
        }
    }
}

/// The second change touched `c.rs` and nothing else; the first touched the
/// other two. Getting this wrong is not subtle — walking the stack the wrong
/// way attributes every file to its neighbour and gives the oldest change a
/// diff full of removals — but it is invisible without a fixture whose changes
/// touch different files. A view that listed every file under every change would be no
/// more useful than the file list.
#[test]
fn a_change_lists_only_its_own_files() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);

    let nodes = app.commit_nodes();
    let mut current = String::new();
    let mut under_second = Vec::new();
    for node in &nodes {
        match &node.kind {
            NodeKind::Commit { .. } => current = node.label.clone(),
            NodeKind::File { .. } if current.contains("a second change") => {
                under_second.push(node.label.clone());
            }
            _ => {}
        }
    }
    assert_eq!(
        under_second,
        vec!["c.rs".to_owned()],
        "the second change's row holds the wrong files"
    );
}

/// A commit row folds like a directory, because in this view it is one.
#[test]
fn s_folds_a_change_and_hides_its_files() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    down_to_a_change_with_files(&mut app);
    let before = app.commit_nodes().len();

    app.on_key(KeyCode::Char('s')).expect("fold the change");

    let after = app.commit_nodes().len();
    assert!(
        after < before,
        "folding a change hid nothing: {before} rows before, {after} after"
    );

    app.on_key(KeyCode::Char('s')).expect("unfold it");
    assert_eq!(app.commit_nodes().len(), before, "unfolding lost rows");
}

/// `t` means the same thing here as in the file list: the files under a change
/// are a tree or a flat list.
#[test]
fn t_switches_the_files_under_a_change_between_a_list_and_a_tree() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");

    let flat = app.commit_nodes();
    app.on_key(KeyCode::Char('t')).expect("tree");
    let tree = app.commit_nodes();

    assert!(
        tree.len() > flat.len(),
        "the tree added no directory rows: {} flat, {} tree",
        flat.len(),
        tree.len()
    );
    assert!(
        tree.iter()
            .any(|node| matches!(node.kind, NodeKind::Dir { .. })),
        "the tree has no directories in it: {tree:?}"
    );
}

/// Walking onto a file row selects that file, so the diff pane follows the
/// cursor here as it does in the file list.
#[test]
fn moving_onto_a_file_row_selects_that_file() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");

    down_to_a_file(&mut app);

    let path = app
        .files()
        .get(app.file_index())
        .map(|file| file.path.clone())
        .expect("a selected file");
    let nodes = app.commit_nodes();
    assert_eq!(
        nodes[app.sidebar_row()].label,
        path,
        "the row under the cursor and the selected file disagree"
    );
}

/// The pane says which list it is showing, because two lists of paths that look
/// alike are exactly when a reviewer needs telling.
#[test]
fn the_pane_names_itself() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);

    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        text.contains(&format!("Commits ({})", app.changes().len())),
        "the pane does not say it is listing changes:\n{text}"
    );
}
