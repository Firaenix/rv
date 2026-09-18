//! A comment sticks to the code it is about, not to the line number it was
//! written at.
//!
//! The resolver has always known where a commented line went; the pane drew
//! the box at the stored number anyway and hung a `· moved` tag on it, so a
//! review of a file an agent had just edited showed every comment floating
//! over unrelated code. These pin the other half: where the box is drawn,
//! where `Enter` jumps, what the browser says.

use crossterm::event::KeyCode;
use rv::session;
use rv_core::model::Confidence;
use rv_core::model::Side;

use crate::support::*;

/// The diff row index the box is drawn under: the last diff row before the
/// first box row.
fn line_above_first_box(app: &rv::app::App) -> Option<String> {
    let plan = app.plan();
    let first_box = plan
        .rows
        .iter()
        .position(|row| row.comment().is_some() || row.flag().is_some())?;
    plan.rows[..first_box]
        .iter()
        .rev()
        .find_map(|row| match row {
            rv::rows::Row::Diff { line, .. } => Some(line.text.trim_end().to_owned()),
            _ => None,
        })
}

fn comment_on_line_two() -> Fixture {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Down).expect("onto let x = 1");
    assert_eq!(
        app.selected_line().expect("a line").text.trim(),
        "let x = 1;"
    );
    write_comment(&mut app, "why one?");
    workspace
}

fn rewrite(workspace: &Fixture, text: &str, message: &str) {
    workspace.write("a.rs", text);
    workspace.jj(&["describe", "-m", message]);
    workspace.jj(&["new"]);
}

/// The commented line survives verbatim, pushed down by lines above it: the
/// box is drawn under it at its new place, `Enter` in the browser lands on
/// it, and the browser names the new line.
#[test]
fn a_comment_follows_its_line_when_the_line_moves() {
    let workspace = comment_on_line_two();
    rewrite(
        &workspace,
        "// one\n// two\n// three\nfn a() {\n    let x = 1;\n}\n",
        "push it down",
    );
    let mut app = workspace.app();
    let comment = &app.comments()[0];
    assert_eq!(app.confidence(comment), Confidence::Moved);
    assert_eq!(
        line_above_first_box(&app).as_deref(),
        Some("    let x = 1;"),
        "the box is not under the line it is about"
    );
    assert_eq!(
        app.comments_for_line(4).len(),
        1,
        "the comment is not placed on line 5 of the new text (diff line 4)"
    );

    to_comments(&mut app);
    let sidebar = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        sidebar.contains(":5 "),
        "the browser names the stored line, not the placed one:\n{sidebar}"
    );
    app.on_key(KeyCode::Enter).expect("jump");
    assert_eq!(
        app.selected_line().expect("a line").text.trim(),
        "let x = 1;",
        "Enter jumped to the stored line, not the placed one"
    );
}

/// The commented line itself was edited, and lines were added above it: the
/// stored context still says where it is. The box goes there — weakly, since
/// the text changed — rather than to the raw number, which now names a
/// comment line the remark has nothing to do with.
#[test]
fn an_edited_line_is_found_by_its_neighbours() {
    let workspace = comment_on_line_two();
    rewrite(
        &workspace,
        "// one\n// two\n// three\nfn a() {\n    let x = 2;\n}\n",
        "address the comment, push it down",
    );
    let app = workspace.app();
    let comment = &app.comments()[0];
    assert_eq!(app.confidence(comment), Confidence::Weak);
    assert_eq!(
        line_above_first_box(&app).as_deref(),
        Some("    let x = 2;"),
        "the box is not under the edited line"
    );
}

/// A flag is anchored the same way and follows the same rule.
#[test]
fn a_flag_follows_its_line_too() {
    let workspace = Fixture::new();
    let review = session::read(workspace.root(), None, None).expect("read the review");
    session::flags::add_flag(&review, "a.rs", Side::Right, 2, "look here").expect("flag");
    rewrite(
        &workspace,
        "// one\n// two\nfn a() {\n    let x = 1;\n}\n",
        "push it down",
    );
    let app = workspace.app();
    assert_eq!(
        line_above_first_box(&app).as_deref(),
        Some("    let x = 1;"),
        "the flag is not under the line it points at"
    );
    assert_eq!(app.flags_for_line(3).len(), 1);
}

/// The live loop: an agent edits the file under a running review — no jj
/// command, no keypress — and on the watch's next tick the comment is drawn
/// under the line it is about, at its new place.
#[test]
fn a_comment_follows_an_edit_the_watch_picks_up() {
    let workspace = comment_on_line_two();
    let mut app = workspace.app();
    let t0 = std::time::Instant::now();
    app.auto_refresh(t0);
    assert_eq!(
        line_above_first_box(&app).as_deref(),
        Some("    let x = 1;")
    );

    workspace.write("a.rs", "// one\n// two\nfn a() {\n    let x = 1;\n}\n");
    app.auto_refresh(t0 + std::time::Duration::from_secs(3));
    app.finish_loading();
    app.finish_merging();
    assert!(app.status().starts_with("refreshed"), "{}", app.status());
    assert_eq!(
        line_above_first_box(&app).as_deref(),
        Some("    let x = 1;"),
        "after the edit landed, the box is not under its line"
    );
    assert_eq!(app.comments_for_line(3).len(), 1);
}

/// A second comment written on a line that already carries a moved one goes
/// into the same stack, and both are still there when the review reopens.
#[test]
fn a_new_comment_stacks_under_a_moved_one() {
    let workspace = comment_on_line_two();
    rewrite(
        &workspace,
        "// one\n// two\nfn a() {\n    let x = 1;\n}\n",
        "push it down",
    );
    let mut app = workspace.app();
    to_comments(&mut app);
    app.on_key(KeyCode::Enter)
        .expect("jump to the moved comment");
    assert_eq!(
        app.selected_line().expect("a line").text.trim(),
        "let x = 1;"
    );
    write_comment(&mut app, "and another");
    assert_eq!(app.comments_for_line(app.line_index()).len(), 2);

    let reopened = workspace.app();
    assert_eq!(reopened.comments_for_line(3).len(), 2);
    let bodies: Vec<&str> = reopened
        .comments_for_line(3)
        .iter()
        .map(|comment| comment.body.as_str())
        .collect();
    assert_eq!(bodies, ["why one?", "and another"]);
}

/// Nothing to follow: the line is gone and its neighbours do not agree on a
/// place. The raw number is still the fallback, tagged weak, as before.
#[test]
fn a_deleted_line_still_falls_back_to_its_number() {
    let workspace = comment_on_line_two();
    rewrite(
        &workspace,
        "fn completely_different() {\n    let y = 9;\n}\n",
        "rewrite the file",
    );
    let app = workspace.app();
    let comment = &app.comments()[0];
    assert_eq!(app.confidence(comment), Confidence::Weak);
    assert_eq!(app.comments_for_line(1).len(), 1);
}
