//! `rv browse`: the tree at `@`, each file shown whole.

use rv::app::App;
use rv::app::DiffEngine;
use rv::session;
use rv_core::diff::DiffSource;
use rv_core::diff::LineKind;
use rv_core::model::ChangeKind;
use rv_core::model::Confidence;
use rv_core::store::CommentState;

use crate::support::*;

/// The tree review of `workspace`, settled like [`Fixture::app`].
pub fn browse(workspace: &Fixture, subpath: Option<&str>) -> App {
    let review = session::browse(workspace.root(), None, subpath).expect("browse the tree");
    let mut app = App::open(review, DiffEngine::Structural).expect("open the reviewer");
    app.finish_loading();
    app.finish_merging();
    app
}

#[test]
fn opening_a_tree_file_renders_every_line_as_context() {
    let workspace = Fixture::new();
    let app = browse(&workspace, None);

    let diff = app.selected_diff().expect("the first file is loaded");
    assert_eq!(diff.path, "a.rs");
    assert_eq!(diff.source, DiffSource::WholeFile);
    let lines = app.displayed_lines();
    let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(texts, SOURCE.lines().collect::<Vec<_>>());
    for (number, line) in (1..).zip(lines.iter()) {
        assert_eq!(line.kind, LineKind::Context, "{line:?}");
        assert_eq!((line.left, line.right), (Some(number), Some(number)));
    }

    let frame = render(&app);
    assert!(frame.contains("let x = 1;"), "{frame}");
    assert!(frame.contains("whole file"), "{frame}");
}

#[test]
fn a_tree_review_lists_the_whole_tree_and_nothing_changed() {
    let workspace = Fixture::new();
    let app = browse(&workspace, None);

    let files: Vec<(&str, ChangeKind)> = app
        .files()
        .iter()
        .map(|file| (file.path.as_str(), file.kind))
        .collect();
    assert_eq!(
        files,
        [
            ("a.rs", ChangeKind::Unchanged),
            ("b.rs", ChangeKind::Unchanged)
        ]
    );
    assert_eq!(app.session().revset, "tree(@)");
    assert_eq!(app.session().base_commit, app.session().head_commit);
    assert!(app.session().changes.is_empty());
}

#[test]
fn a_subpath_narrows_the_tree_to_the_files_under_it() {
    let workspace = Fixture::nested();
    let app = browse(&workspace, Some("docs/"));

    let paths: Vec<&str> = app.files().iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["docs/specs/a.md", "docs/specs/b.md"]);
}

#[test]
fn a_subpath_with_no_files_under_it_is_an_error() {
    let workspace = Fixture::new();
    let Err(error) = session::browse(workspace.root(), None, Some("nowhere")) else {
        panic!("browsing an empty subpath opened a review");
    };
    assert!(error.to_string().contains("nowhere"), "{error:#}");
}

#[test]
fn a_note_anchored_in_a_tree_survives_an_insertion_above_it() {
    for inserted in 1..=3 {
        let workspace = Fixture::new();
        let mut app = browse(&workspace, None);
        let line = select_line(&mut app, |line| line.text.contains("let x = 1;"));
        assert_eq!(line.right, Some(2));
        write_comment(&mut app, "why one?");
        drop(app);

        let header = numbered("// inserted", inserted);
        workspace.write("a.rs", &format!("{header}{SOURCE}"));
        workspace.jj(&["status"]);
        let app = browse(&workspace, None);

        let [comment] = app.comments() else {
            panic!("expected the one note: {:?}", app.comments());
        };
        assert_eq!(comment.state, CommentState::Open, "{comment:?}");
        assert_eq!(comment.anchor.line, 2 + inserted, "{comment:?}");

        let review = session::read_tree(workspace.root(), None).expect("read the tree");
        let stored = review.store.comments().expect("read the stored note");
        assert_eq!(stored[0].anchor.line, 2, "the store keeps the written line");
        assert_eq!(
            rv::stale::resolution(&review, &stored[0]),
            (Some(2 + inserted), Confidence::Moved)
        );
    }
}

#[test]
fn a_tree_review_and_the_range_review_keep_their_notes_apart() {
    let workspace = Fixture::new();
    let mut app = browse(&workspace, None);
    select_line(&mut app, |line| line.text.contains("let x = 1;"));
    write_comment(&mut app, "on the tree");
    drop(app);

    let range = workspace.app();
    assert!(range.comments().is_empty(), "{:?}", range.comments());
    let tree = browse(&workspace, None);
    assert_eq!(tree.comments().len(), 1, "{:?}", tree.comments());
}
