//! The commits tab: which change touched which file.

use crossterm::event::KeyCode;
use ratatui::style::Modifier;
use rv::layout::Split;
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

/// The row shows eight characters of each id, not thirty-two of one.
///
/// A full change id is thirty-two characters and fills the sidebar, which is
/// what it did: the row read `wzlmltkwvqsonsomoklrz…` and the description — the
/// thing the row is *for* — never appeared at all.
#[test]
fn a_commit_row_shows_short_ids_and_its_description() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);

    let change = app
        .changes()
        .iter()
        .find(|change| change.description.starts_with("a second change"))
        .expect("the described change");
    let short_change: String = change.change_id.chars().take(8).collect();
    let short_commit: String = change.commit_id.chars().take(8).collect();

    let row = app
        .commit_nodes()
        .into_iter()
        .find(|node| node.label.contains("a second change"))
        .expect("a row for the change");

    assert!(
        row.label.contains(&short_change),
        "the row does not carry the change id: {:?}",
        row.label
    );
    assert!(
        row.label.contains(&short_commit),
        "the row does not carry the commit hash: {:?}",
        row.label
    );
    assert!(
        !row.label.contains(&change.change_id),
        "the row still prints the whole change id: {:?}",
        row.label
    );
    assert!(
        row.label.contains("a second change"),
        "the description is missing: {:?}",
        row.label
    );
}

/// The characters you can select a change by are the bright ones, exactly as
/// `jj log` draws them — so a reviewer knows what to type without counting.
#[test]
fn the_prefix_you_select_by_is_highlighted_and_the_rest_is_dim() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);

    let change = app
        .changes()
        .iter()
        .find(|change| change.description.starts_with("a second change"))
        .expect("the described change")
        .clone();
    let short: String = change.change_id.chars().take(8).collect();
    let frame = frame_at(&app, 100, 24);
    let row = sidebar_row_for(&frame, &short);

    // The whole eight-character id as the needle, so the column it starts at is
    // not in doubt — a one-character probe finds its first match anywhere on the
    // row, including inside the fold mark.
    // A *character* offset, not a byte one: the fold mark is three bytes and one
    // column, so a byte index is three columns wrong before the id even starts.
    let text = rows_of(&frame)[usize::from(row)].clone();
    let start = text
        .char_indices()
        .position(|(byte, _)| text[byte..].starts_with(&short))
        .expect("the id is on the row");
    let at = |offset: usize| {
        frame[(u16::try_from(start + offset).expect("a small column"), row)].style()
    };

    let first = at(0);
    assert!(
        first.add_modifier.contains(Modifier::BOLD),
        "the first character of the id is not the one you select by: {first:?}"
    );
    let last = at(7);
    assert!(
        last.add_modifier.contains(Modifier::DIM),
        "the rest of the id is not dimmed: {last:?}"
    );
}

/// Two changes never share a highlighted prefix: the highlight is a claim that
/// typing those characters picks *this* change out.
#[test]
fn the_highlighted_prefix_tells_the_changes_apart() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);

    let mut prefixes = Vec::new();
    for node in app.commit_nodes() {
        if let rv::tree::NodeKind::Commit {
            change_id, unique, ..
        } = &node.kind
        {
            assert!(*unique >= 1, "a change is selectable by nothing at all");
            prefixes.push(change_id.chars().take(*unique).collect::<String>());
        }
    }

    for (at, prefix) in prefixes.iter().enumerate() {
        for (other_at, other) in prefixes.iter().enumerate() {
            if at != other_at {
                assert!(
                    !other.starts_with(prefix.as_str()),
                    "{prefix:?} does not pick one change out: it also starts {other:?}"
                );
            }
        }
    }
}

/// Standing on a change — or inside its files — says which change it is, so a
/// reviewer who has scrolled its row off the top still knows what they are
/// reading.
#[test]
fn the_bar_names_the_change_the_cursor_is_in() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    down_to_a_file(&mut app);

    let (change, _, subject) = app
        .change_under_cursor()
        .expect("the cursor is inside a change");
    let bar = last_row(&frame_at(&app, 140, 24));
    assert!(
        bar.contains(&change),
        "the bar does not name the change: {bar:?}"
    );
    assert!(
        bar.contains(&subject) || subject.is_empty(),
        "the bar does not carry its description: {bar:?}"
    );
}

/// And says nothing about a change from the tabs that are not listing them: a
/// bar that claimed a change while the file list was showing would be naming
/// something the reviewer cannot see.
#[test]
fn the_other_tabs_name_no_change() {
    let workspace = two_changes();
    let mut app = workspace.app();

    assert!(app.change_under_cursor().is_none(), "the files tab named one");
    to_comments(&mut app);
    assert!(
        app.change_under_cursor().is_none(),
        "the comments tab named one"
    );
}

/// The point of the whole view: a file row under a change shows **that change's**
/// diff of the file, not the branch's.
///
/// `a.rs` gains one line in the first change and one more in the second. The
/// file list shows both additions, because that is what the branch did; the row
/// under the second change shows one, because that is what the change did. A view
/// that showed the branch's diff under a change row would be labelling one thing
/// as another — and a comment written on it would be anchored to a revision its
/// quoted text does not come from.
#[test]
fn a_file_row_shows_the_diff_of_the_change_it_sits_under() {
    let workspace = Fixture::new();
    workspace.write("grow.rs", "fn one() {}\n");
    workspace.jj(&["describe", "-m", "the first line"]);
    workspace.jj(&["new"]);
    workspace.write("grow.rs", "fn one() {}\nfn two() {}\n");
    workspace.jj(&["describe", "-m", "the second line"]);
    workspace.jj(&["new"]);

    let mut app = workspace.app();
    // The branch's own diff of the file: both lines are new.
    let branch = app
        .files()
        .iter()
        .position(|file| file.path == "grow.rs")
        .expect("grow.rs is in the review");
    while app.file_index() != branch {
        app.on_key(KeyCode::Char(']')).expect("next file");
    }
    let whole = app.selected_diff().expect("a diff").lines.len();
    assert_eq!(whole, 2, "the branch's diff is not both lines: {whole}");

    // And the second change's, which is one of them.
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    let mut found = false;
    for _ in 0..app.nodes().len() {
        if app
            .nodes()
            .get(app.sidebar_row())
            .is_some_and(|node| node.label == "grow.rs")
            && app
                .change_under_cursor()
                .is_some_and(|(_, _, subject)| subject == "the second line")
        {
            found = true;
            break;
        }
        app.on_key(KeyCode::Down).expect("next row");
    }
    assert!(found, "no grow.rs row under the second change");

    let scoped = app.selected_diff().expect("a diff").lines.len();
    assert_eq!(
        scoped, 1,
        "the change's row shows the branch's diff rather than its own: {scoped} lines"
    );
}

/// And a comment written there is anchored between that change's own commits, so
/// the text it quotes can be read back from the revision it names.
#[test]
fn a_comment_written_under_a_change_is_anchored_to_that_change() {
    let workspace = Fixture::new();
    workspace.write("grow.rs", "fn one() {}\n");
    workspace.jj(&["describe", "-m", "the first line"]);
    workspace.jj(&["new"]);
    workspace.write("grow.rs", "fn one() {}\nfn two() {}\n");
    workspace.jj(&["describe", "-m", "the second line"]);
    workspace.jj(&["new"]);

    let mut app = workspace.app();
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    while !app
        .nodes()
        .get(app.sidebar_row())
        .is_some_and(|node| node.label == "grow.rs")
    {
        app.on_key(KeyCode::Down).expect("next row");
    }
    let change = app
        .change_under_cursor()
        .expect("a change under the cursor")
        .1;

    app.on_key(KeyCode::Right).expect("onto the diff");
    write_comment(&mut app, "about this change's line");

    let stored = workspace.store().comments().expect("read the store");
    let comment = stored.last().expect("a saved comment");
    assert!(
        comment.commit_id.starts_with(&change),
        "the comment is anchored to {:?} rather than to the change on screen ({change})",
        comment.commit_id
    );
    // And the anchor's stored context is the text that revision holds, which is
    // what makes it verifiable at all.
    assert!(
        !comment.anchor.context.is_empty(),
        "the anchor quotes nothing"
    );
}

/// A narrow sidebar gives up the subject before either id, and gives up the
/// second id **whole** rather than half.
///
/// `e…` is not a commit hash, it is a hash-shaped hole: a reviewer who pastes it
/// gets nothing, and the row that printed it invited them to. The ids are what a
/// row is acted on through, and the subject is on the bar whenever the cursor is
/// in the change anyway.
///
/// Swept over widths rather than asserted at three chosen ones: which form fits
/// depends on the counts column, the change bar and the split, and a test that
/// hard-coded the crossover would be re-deriving the layout instead of checking
/// the rule.
#[test]
fn a_narrow_commit_row_drops_the_subject_before_an_id() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);

    let change = app
        .changes()
        .iter()
        .find(|change| change.description.starts_with("a second change"))
        .expect("the described change")
        .clone();
    let short_change: String = change.change_id.chars().take(8).collect();
    let short_commit: String = change.commit_id.chars().take(8).collect();

    // Wide enough, and the row says everything.
    let wide = sidebar_text(&frame_at(&app, 200, 24), 200, 24, Split::default());
    assert!(
        wide.contains(&format!("{short_change} {short_commit}")),
        "a wide row does not carry both ids:\n{wide}"
    );
    assert!(
        wide.contains("a second change"),
        "a wide row does not carry the subject:\n{wide}"
    );

    // And at every width, the change row is one of the three whole forms — never
    // a partly-printed id, and never without its fold mark.
    for width in (40u16..=200).step_by(4) {
        let text = sidebar_text(&frame_at(&app, width, 24), width, 24, Split::default());
        let Some(line) = text.lines().find(|line| line.contains(&short_change)) else {
            // Too narrow for even the change id: the row is clipped to nothing
            // recognisable, which is the file list's existing behaviour.
            continue;
        };
        // Four characters, not one: a one- or two-character "prefix" of a hash
        // coincides with the change id or the counts on most rows, and calling
        // that a half-printed hash would fail on a row that is entirely correct.
        // Four is long enough that a coincidence is a millionth and short enough
        // that any real truncation is caught.
        let partial = (4..short_commit.len())
            .map(|cut| &short_commit[..cut])
            .any(|prefix| line.contains(prefix) && !line.contains(&short_commit));
        assert!(
            !partial,
            "at {width} columns the row prints part of the commit hash: {line:?}"
        );
        assert!(
            line.contains('▾') || line.contains('▸'),
            "at {width} columns the row lost its fold mark: {line:?}"
        );
    }
}

/// The description is under the list, where a narrow row cannot carry it.
///
/// It was on the status bar first, which is exactly where a reviewer could not
/// find it: the bar drops segments by rank when the row is short, and a long
/// subject is the widest thing on it — so the one segment that had to survive was
/// the first to go. The border under the rows was carrying only the shape and the
/// order, and it is directly under the thing it describes.
#[test]
fn the_change_under_the_cursor_is_described_under_the_list() {
    let workspace = two_changes();
    let mut app = workspace.app();
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    down_to_a_file(&mut app);

    let (_, _, subject) = app.change_under_cursor().expect("a change");
    let frame = frame_at(&app, 100, 24);
    let border = sidebar_shape(&frame);
    assert!(
        border.contains(&subject),
        "the description is not under the list: {border:?}"
    );

    // And the file list still says its shape and its order there.
    to_comments(&mut app);
    to_commits(&mut app);
    to_comments(&mut app);
    to_files(&mut app);
    let files = sidebar_shape(&frame_at(&app, 100, 24));
    assert!(
        files.contains("list") && files.contains("natural"),
        "the file list lost its shape and order: {files:?}"
    );
}

/// A directory reads dimmer than the files in it, so a hundred paths in one ink
/// stop being a wall.
///
/// Foreground only: there is no background wash on a sidebar row, and the counts
/// are still the one thing in this pane that means green and red.
#[test]
fn a_directory_row_is_quieter_than_the_files_under_it() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('t')).expect("tree");
    // Off row 0, so the selection's own styling is not what is measured.
    app.on_key(KeyCode::Down).expect("next row");

    let frame = frame_at(&app, 100, 24);
    let nodes = app.nodes();
    let directory = nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::Dir { .. }))
        .expect("a directory row");
    let file = nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::File { .. }))
        .expect("a file row");

    let dim = |label: &str| {
        let row = sidebar_row_for(&frame, label);
        style_of_text(&frame, row, label)
            .add_modifier
            .contains(Modifier::DIM)
    };
    assert!(
        dim(&directory.label),
        "a directory reads exactly like a file: {:?}",
        directory.label
    );
    assert!(
        !dim(&file.label),
        "a file is dimmed too, so the tiers are not distinguishable: {:?}",
        file.label
    );
}
