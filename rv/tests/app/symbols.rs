//! Stepping between symbols with `n` and `N`, and finding one with `/`.

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::Focus;
use rv::app::Mode;

use crate::support::*;

/// Two files with several definitions each, so a walk has somewhere to go and
/// crossing a file boundary is something that happens rather than something
/// asserted about a one-file review.
fn symbols() -> Fixture {
    let fixture = Fixture::new();
    fixture.write(
        "first.rs",
        "fn alpha() {}\n\nfn beta() {}\n\nstruct Gamma;\n",
    );
    fixture.write("second.rs", "fn delta() {}\n\nfn epsilon() {}\n");
    fixture.jj(&["describe", "-m", "some definitions"]);
    fixture.jj(&["new"]);
    fixture
}

/// The names the index holds, in walk order.
fn names(app: &mut App) -> Vec<String> {
    app.index()
        .entries()
        .iter()
        .map(|entry| entry.symbol.name.clone())
        .collect()
}

#[test]
fn the_index_holds_every_definition_in_the_review() {
    let workspace = symbols();
    let mut app = workspace.app();

    let found = names(&mut app);
    for wanted in ["alpha", "beta", "Gamma", "delta", "epsilon"] {
        assert!(
            found.iter().any(|name| name == wanted),
            "{wanted} is not in the index: {found:?}"
        );
    }
}

/// `n` moves forward through the walk and `N` back, and neither wraps: a jump
/// from the last symbol to the first looks exactly like a jump that failed.
#[test]
fn n_walks_forward_and_n_shift_walks_back_without_wrapping() {
    let workspace = symbols();
    let mut app = workspace.app();
    let total = names(&mut app).len();
    assert!(total >= 4, "the fixture has too few symbols: {total}");

    let mut visited = Vec::new();
    for _ in 0..total + 2 {
        app.on_key(KeyCode::Char('n')).expect("next symbol");
        visited.push((app.file_index(), app.line_index()));
    }
    // The last two presses had nowhere to go, so the cursor stopped rather than
    // returning to the top.
    assert_eq!(
        visited[visited.len() - 1],
        visited[visited.len() - 2],
        "`n` wrapped instead of stopping: {visited:?}"
    );
    assert!(
        app.status().contains("last symbol"),
        "the end of the walk is not announced: {:?}",
        app.status()
    );

    let at_end = (app.file_index(), app.line_index());
    app.on_key(KeyCode::Char('N')).expect("previous symbol");
    assert_ne!(
        (app.file_index(), app.line_index()),
        at_end,
        "`N` did not move back"
    );
}

/// The walk is over the whole review, not one file: a reviewer stepping through
/// symbols should not have to change files by hand.
#[test]
fn the_walk_crosses_from_one_file_into_the_next() {
    let workspace = symbols();
    let mut app = workspace.app();
    let total = names(&mut app).len();

    let mut files = std::collections::HashSet::new();
    for _ in 0..total {
        app.on_key(KeyCode::Char('n')).expect("next symbol");
        files.insert(app.file_index());
    }

    assert!(
        files.len() > 1,
        "the walk never left one file: visited {files:?}"
    );
}

/// A jump says where it went, because the cursor moving in a long diff is not
/// self-evident.
#[test]
fn a_jump_names_the_symbol_and_where_it_is() {
    let workspace = symbols();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('n')).expect("next symbol");

    let status = app.status().to_owned();
    assert!(
        status.contains(".rs:"),
        "the status does not say where it landed: {status:?}"
    );
    assert_eq!(app.focus(), Focus::Diff, "a jump left the diff unfocused");
}

/// A review of files rv has no grammar for has nothing to walk, and says so
/// rather than looking broken.
#[test]
fn a_review_with_no_indexable_files_says_so() {
    let workspace = Fixture::plain();
    let mut app = workspace.app();
    assert!(app.index().is_empty(), "notes.txt produced symbols");

    app.on_key(KeyCode::Char('n')).expect("next symbol");

    assert!(
        app.status().contains("no symbols"),
        "an unindexable review does not say so: {:?}",
        app.status()
    );
}

#[test]
fn slash_opens_the_picker_and_escape_closes_it() {
    let workspace = symbols();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('/')).expect("open the picker");
    assert_eq!(app.mode(), Mode::Pick);

    app.on_key(KeyCode::Esc).expect("cancel");
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.buffer(), "", "the query outlived the picker");
}

/// The point of the picker: type part of a name, press Enter, land on it.
#[test]
fn the_picker_jumps_to_the_symbol_you_typed() {
    let workspace = symbols();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('/')).expect("open the picker");
    type_text(&mut app, "epsilon");
    app.on_key(KeyCode::Enter).expect("jump");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(
        app.status().contains("epsilon"),
        "the picker did not land on the symbol: {:?}",
        app.status()
    );
    assert_eq!(
        app.files()[app.file_index()].path,
        "second.rs",
        "the jump did not change files"
    );
}

/// A name that begins with the query beats one that merely contains it, because
/// the first is what the reviewer meant.
#[test]
fn a_prefix_match_outranks_a_substring_one() {
    let workspace = Fixture::new();
    workspace.write("names.rs", "fn beta_one() {}\n\nfn alpha_beta() {}\n");
    workspace.jj(&["describe", "-m", "two betas"]);
    workspace.jj(&["new"]);
    let mut app = workspace.app();
    let _ = app.index();

    app.on_key(KeyCode::Char('/')).expect("open the picker");
    type_text(&mut app, "beta");

    let best = app.matches();
    assert_eq!(
        best.first().map(|entry| entry.symbol.name.as_str()),
        Some("beta_one"),
        "the prefix match is not first: {:?}",
        best.iter()
            .map(|entry| entry.symbol.name.as_str())
            .collect::<Vec<_>>()
    );
}

/// The query and its candidates are on screen, or there is nothing to choose
/// from.
#[test]
fn the_picker_shows_the_query_and_its_matches() {
    let workspace = symbols();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('/')).expect("open the picker");
    type_text(&mut app, "et");

    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(text.contains("/et"), "the query is not on screen:\n{text}");
    assert!(text.contains("beta"), "no candidate is on screen:\n{text}");
}

/// A query nothing matches says so and takes the reviewer nowhere.
#[test]
fn a_query_that_matches_nothing_jumps_nowhere() {
    let workspace = symbols();
    let mut app = workspace.app();
    let before = (app.file_index(), app.line_index());

    app.on_key(KeyCode::Char('/')).expect("open the picker");
    type_text(&mut app, "zzzzz");
    app.on_key(KeyCode::Enter).expect("try to jump");

    assert_eq!(
        (app.file_index(), app.line_index()),
        before,
        "the cursor moved"
    );
    assert!(
        app.status().contains("no symbol matches"),
        "the miss is not announced: {:?}",
        app.status()
    );
}

/// The scope rule the whole feature is shaped by: from the Commits tab, `n`
/// walks the symbols of **that change** and no others.
///
/// A reviewer reading one change of a stack is asking about that change. A walk
/// that wandered into a neighbour's files would be answering a question they did
/// not ask, and the file it landed in would look like part of the change.
#[test]
fn the_commits_tab_narrows_the_walk_to_one_change() {
    let workspace = Fixture::new();
    workspace.write("early.rs", "fn early_one() {}\n\nfn early_two() {}\n");
    workspace.jj(&["describe", "-m", "the earlier change"]);
    workspace.jj(&["new"]);
    workspace.write("late.rs", "fn late_one() {}\n\nfn late_two() {}\n");
    workspace.jj(&["describe", "-m", "the later change"]);
    workspace.jj(&["new"]);

    let mut app = workspace.app();
    let whole_bookmark = names(&mut app);
    assert!(
        whole_bookmark.iter().any(|n| n == "early_one")
            && whole_bookmark.iter().any(|n| n == "late_one"),
        "the bookmark scope is missing a change's symbols: {whole_bookmark:?}"
    );

    // Onto the commits tab, and down to a file of the later change.
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    while app
        .nodes()
        .get(app.sidebar_row())
        .is_none_or(|node| node.label != "late.rs")
    {
        app.on_key(KeyCode::Down).expect("next row");
    }

    let scoped = names(&mut app);
    assert!(
        scoped.iter().any(|name| name == "late_one"),
        "the change's own symbols are missing: {scoped:?}"
    );
    assert!(
        !scoped.iter().any(|name| name == "early_one"),
        "the walk reaches another change's symbols: {scoped:?}"
    );
}

/// Moving between changes re-scopes the walk rather than serving a cached one.
#[test]
fn the_scope_follows_the_cursor_between_changes() {
    let workspace = Fixture::new();
    workspace.write("early.rs", "fn early_one() {}\n");
    workspace.jj(&["describe", "-m", "the earlier change"]);
    workspace.jj(&["new"]);
    workspace.write("late.rs", "fn late_one() {}\n");
    workspace.jj(&["describe", "-m", "the later change"]);
    workspace.jj(&["new"]);

    let mut app = workspace.app();
    to_commits(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");

    let mut seen: Vec<Vec<String>> = Vec::new();
    for _ in 0..app.nodes().len() {
        if matches!(
            app.nodes().get(app.sidebar_row()).map(|node| &node.kind),
            Some(rv::tree::NodeKind::File { .. })
        ) {
            seen.push(names(&mut app));
        }
        app.on_key(KeyCode::Down).expect("next row");
    }

    assert!(
        seen.len() >= 2,
        "the fixture did not offer two changes with files: {seen:?}"
    );
    assert_ne!(
        seen[0],
        seen[seen.len() - 1],
        "the index was served from the cache after the scope changed: {seen:?}"
    );
}
