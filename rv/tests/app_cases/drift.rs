//! Where the cursor stands while the background answers.
//!
//! `DiffEngine::Auto` draws the fast `similar` diff at once and replaces it
//! twice more: when the structural diff lands (`apply_refined`), and when the
//! whole-file merge built off it lands (`apply_merged`). Both swap the whole
//! line list, so a *row* index held across one addresses a list that no longer
//! exists — which is how a jump to a comment used to end up on unrelated code,
//! at a row that depended on which answer arrived first.
//!
//! Every case is written through [`Fixture::auto_app`], the one constructor
//! that leaves that work outstanding, and drains it by hand. Nothing sleeps and
//! nothing reads a clock: `finish_loading` and `finish_merging` block on the
//! workers' own channels, so the only ordering these cases see is the one they
//! ask for.

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use rv::app::App;
use rv::app::anchored_side;
use rv::tree::NodeKind;
use rv_core::diff::DiffLine;
use rv_core::model::Side;

use crate::support::*;

/// A place in the code, in the same three parts the app records a cursor
/// anchor in: the path on the anchored side, that side, and the number there.
type Position = (String, Side, u32);

/// Where a diff line of the selected file belongs, as the pane and the save
/// path read it: the anchored side, that side's path, and the number there.
fn place(app: &App, line: &DiffLine) -> Option<Position> {
    let file = app.selected_file()?;
    let side = anchored_side(line.kind);
    let path = match side {
        Side::Left => file.source_path.as_deref().unwrap_or(&file.path),
        Side::Right => file.path.as_str(),
    };
    Some((path.to_owned(), side, anchored_number(line)?))
}

/// Where in the code the cursor stands.
///
/// Recomputed here rather than asked of the app: `cursor_position` is the very
/// function the fix records its anchor with, and an oracle that called it would
/// agree with the fix by construction.
fn position(app: &App) -> Option<Position> {
    place(app, lines(app).get(app.line_index())?)
}

/// The lines the pane is drawing, for a failure message: every assertion below
/// is about which of them the cursor is on, and a bare index says nothing.
fn landed(app: &App) -> String {
    lines(app)
        .iter()
        .enumerate()
        .map(|(index, line)| {
            format!(
                "\n  [{index}] {:?} left={:?} right={:?} {:?}",
                line.kind, line.left, line.right, line.text
            )
        })
        .collect()
}

/// Which displayed line carries `wanted`, and which one is nearest to it on the
/// same path and side — the two answers `App::line_index_at_anchor` chooses
/// between, computed from the drawn lines alone.
fn exact_and_nearest(app: &App, wanted: &Position) -> (Option<usize>, Option<usize>) {
    let (path, side, number) = wanted;
    let candidates: Vec<(usize, u32)> = lines(app)
        .iter()
        .enumerate()
        .filter_map(|(index, line)| match place(app, line)? {
            (ref here, on, carried) if here == path && on == *side => Some((index, carried)),
            _ => None,
        })
        .collect();
    let exact = candidates
        .iter()
        .find(|(_, carried)| carried == number)
        .map(|(index, _)| *index);
    let nearest = candidates
        .iter()
        .min_by_key(|(_, carried)| carried.abs_diff(*number))
        .map(|(index, _)| *index);
    (exact, nearest)
}

/// Writes one comment `downs` lines into `path` through the keyboard, from a
/// fully settled reviewer, and answers the place it was anchored at.
///
/// Through [`Fixture::app`] — difftastic, both queues drained — so the line is
/// one the *merged* whole-file view shows and the anchor is one the save path
/// actually made.
fn seed(fixture: &Fixture, path: &str, downs: usize, body: &str) -> Position {
    let mut app = fixture.app();
    select_path(&mut app, path);
    walk_to_line(&mut app, downs);
    comment(&mut app);
    type_text(&mut app, body);
    press(&mut app, KeyCode::Enter);

    let comments = fixture.comments();
    assert_eq!(comments.len(), 1, "the seed wrote {comments:?}");
    let anchor = &comments[0].anchor;
    assert_eq!(anchor.file, path);
    (anchor.file.clone(), anchor.side, anchor.line)
}

/// Jumps to the review's only comment from the browser, the way a reviewer
/// does: the comments mode, then `Enter` on the row the browser opens on.
fn jump_to_the_comment(app: &mut App) {
    to_comments(app);
    let anchor = app
        .browsed_comment()
        .expect("the browser opens on the only comment")
        .anchor
        .clone();
    press(app, KeyCode::Enter);
    assert_eq!(
        app.status(),
        format!("jumped to {}:{}", anchor.file, anchor.line),
        "the jump was refused"
    );
}

fn bodies(app: &App) -> Vec<String> {
    app.comments_for_line(app.line_index())
        .iter()
        .map(|comment| comment.body.clone())
        .collect()
}

/// A jump to a comment ends on that comment's own line, and is still there
/// after both background results have landed on top of it.
///
/// The chained case the fix exists for. The comment is seeded deep enough in
/// `alpha.rs` that the changed-only structural diff drops its line as context,
/// so all three views the pane shows in turn disagree about which row the line
/// is at, and a held row index lands somewhere else in each.
///
/// The oracle is the reviewer's own: `comments_for_line` at the cursor. The
/// middle checkpoint is the honest exception — the structural diff genuinely
/// does not carry the line, so the strongest true claim there is the
/// nearest-surviving one `the_cursor_falls_back_to_the_nearest_surviving_line`
/// states in full.
#[test]
fn a_jump_holds_its_comment_through_the_refinement_and_the_merge() {
    let fixture = Fixture::multi();
    let anchored = seed(&fixture, "alpha.rs", 7, "deep finding");

    let mut app = fixture.auto_app();
    jump_to_the_comment(&mut app);
    assert!(
        app.refining(),
        "nothing was left to land, so this case tested nothing"
    );
    assert_eq!(
        position(&app),
        Some(anchored.clone()),
        "the jump itself missed: {}",
        landed(&app)
    );
    assert_eq!(
        bodies(&app),
        vec!["deep finding".to_owned()],
        "the comment jumped to is not on the line jumped to: {}",
        landed(&app)
    );

    app.finish_loading();
    let (exact, nearest) = exact_and_nearest(&app, &anchored);
    assert_eq!(
        exact,
        None,
        "the structural diff kept line {}, so the chain this case is about never happened: {}",
        anchored.2,
        landed(&app)
    );
    assert_eq!(
        app.line_index(),
        nearest.expect("a surviving line on the anchored side"),
        "the refinement left the cursor somewhere other than the nearest surviving line: {}",
        landed(&app)
    );

    app.finish_merging();
    assert_eq!(
        position(&app),
        Some(anchored),
        "the merge landed the cursor on other code than the comment's: {}",
        landed(&app)
    );
    assert_eq!(
        bodies(&app),
        vec!["deep finding".to_owned()],
        "the comment jumped to is no longer under the cursor: {}",
        landed(&app)
    );
}

/// Two reviewers who drained the same work in different orders are looking at
/// the same line.
///
/// A refinement re-kicks the merge, so a merge built off the fast diff can land
/// *after* the refinement has already replaced it: which of the two results is
/// applied first is a fact about two worker threads, and nothing the reviewer
/// can see may depend on it.
#[test]
fn the_order_the_results_arrive_in_does_not_change_where_the_cursor_ends_up() {
    let fixture = Fixture::multi();
    let anchored = seed(&fixture, "alpha.rs", 7, "deep finding");

    let mut refined_first = fixture.auto_app();
    jump_to_the_comment(&mut refined_first);
    refined_first.finish_loading();
    refined_first.finish_merging();

    let mut merged_first = fixture.auto_app();
    jump_to_the_comment(&mut merged_first);
    // The first `finish_merging` is the interesting one: no merge is running
    // yet, because the fast diff is not one the merger will build on, so this
    // is a reviewer who asked for the merged view before there was anything to
    // merge — and then got both answers in the other order.
    merged_first.finish_merging();
    merged_first.finish_loading();
    merged_first.finish_merging();

    assert_eq!(
        position(&refined_first),
        position(&merged_first),
        "the two drain orders ended on different code: {}\nagainst:{}",
        landed(&refined_first),
        landed(&merged_first)
    );
    assert_eq!(
        position(&merged_first),
        Some(anchored),
        "neither order ended on the comment's own line: {}",
        landed(&merged_first)
    );
}

/// When the diff that lands does not carry the anchored line at all, the cursor
/// falls back to the nearest line it does carry on the same path and side —
/// never to whatever the old row index now names.
///
/// The comment is seeded near the top of `alpha.rs` on purpose: the nearest
/// surviving line is then *above* the row the jump landed on, so the honest
/// answer and the "keep the row, clamp it" one are different rows.
#[test]
fn the_cursor_falls_back_to_the_nearest_surviving_line() {
    let fixture = Fixture::multi();
    let anchored = seed(&fixture, "alpha.rs", 1, "shallow finding");

    let mut app = fixture.auto_app();
    jump_to_the_comment(&mut app);
    let jumped_row = app.cursor_row();
    assert_eq!(position(&app), Some(anchored.clone()));

    app.finish_loading();
    let (exact, nearest) = exact_and_nearest(&app, &anchored);
    assert_eq!(
        exact,
        None,
        "line {} survived the refinement, so the fallback branch never ran: {}",
        anchored.2,
        landed(&app)
    );
    let nearest = nearest.expect("a surviving line on the anchored side");
    assert_eq!(
        app.line_index(),
        nearest,
        "the cursor is not on the nearest surviving line: expected {:?}, got {:?}{}",
        lines(&app).get(nearest),
        lines(&app).get(app.line_index()),
        landed(&app)
    );
    assert_ne!(
        app.cursor_row(),
        jumped_row,
        "the cursor kept the row it jumped to, which the new diff numbers \
         differently: {}",
        landed(&app)
    );
}

/// Walking to another change re-reads where the cursor stands, so the results
/// that follow resolve the pair on screen's own place and never the one before
/// it.
///
/// [`Fixture::twice_changed`] is where the two can be told apart: two pairs over
/// one path, so walking from one to the other changes the diff on screen
/// *without* moving the bookmark file selection, while the same number names
/// different code in each. The row survives that walk untouched, and a row is
/// exactly what a landing result invalidates — so the place under it is read
/// again on the way in, and it is that place the cursor holds.
#[test]
fn walking_to_another_change_reads_the_cursors_place_again() {
    let fixture = Fixture::twice_changed();
    let mut app = fixture.auto_app();

    to_commits(&mut app);
    // Down onto the older change's heading, then onto its file row, which is
    // what selects that pair and shows its own diff.
    press_n(&mut app, KeyCode::Down, 2);
    let older = selected_pair(&app);
    press(&mut app, KeyCode::Right);
    press_n(&mut app, KeyCode::Down, 4);
    let read_there = position(&app).expect("a place in the older pair's diff");

    press(&mut app, KeyCode::Left);
    press_n(&mut app, KeyCode::Up, 2);
    let newer = selected_pair(&app);
    assert_ne!(older, newer, "the walk never left the older pair");
    let file = app.file_index();
    let read_here = position(&app).expect("a place in the newer pair's diff");
    assert_ne!(
        read_here, read_there,
        "both pairs put the same place under the cursor, so this case tested nothing"
    );
    assert!(
        app.refining(),
        "the newer pair had nothing outstanding, so this case tested nothing"
    );

    app.finish_loading();
    app.finish_merging();

    assert_eq!(app.file_index(), file, "the selected file moved");
    assert_eq!(selected_pair(&app), newer, "the selected pair moved");
    assert_eq!(
        position(&app),
        Some(read_here.clone()),
        "the cursor left the place this pair had under it: {}",
        landed(&app)
    );
    let (exact, nearest) = exact_and_nearest(&app, &read_there);
    if let Some(elsewhere) = exact.or(nearest) {
        assert_ne!(
            app.line_index(),
            elsewhere,
            "the cursor was aimed at {read_there:?}, which was read from the other pair: {}",
            landed(&app)
        );
    }
}

/// The commits tab's selected pair, as the row under the sidebar cursor names
/// it: the pair index the handlers use, and the change it sits under.
fn selected_pair(app: &App) -> (usize, Option<usize>) {
    let nodes = app.nodes();
    let node = nodes
        .get(app.sidebar_row())
        .expect("a row under the sidebar cursor");
    match node.kind {
        NodeKind::File { index } => (index, app.commit_change(index)),
        ref other => panic!("the sidebar cursor is on {other:?}, not a file row"),
    }
}

/// A view the reviewer parked with the wheel is still parked after both results
/// have landed.
///
/// Scrolling is looking: the pane stays where it was put until the *selection*
/// moves, and a background result is not the reviewer moving anything —
/// `App::reanchor_cursor` promises exactly this and keeps it by putting
/// `diff_scroll` back after re-seating the cursor.
///
/// The park comes after the jump rather than before it because a jump *is* the
/// selection moving: `set_cursor_row` un-parks the view deliberately, so
/// parking first would leave nothing for the results to preserve.
#[test]
fn a_view_parked_with_the_wheel_is_still_parked_when_the_results_land() {
    let fixture = Fixture::multi();
    let anchored = seed(&fixture, "long.rs", 12, "long finding");

    let mut app = fixture.auto_app();
    jump_to_the_comment(&mut app);
    // The frame first: the wheel resolves against the layout that was painted,
    // so a gesture without one would be aimed at a screen nobody saw.
    let pane = {
        let _ = render(&app, 100, 24);
        app.painted_layout().diff
    };
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: pane.x + 2,
        row: pane.y + 2,
        modifiers: KeyModifiers::NONE,
    })
    .expect("the wheel over the diff pane");
    let parked = app.diff_scroll().expect("the wheel parked the view");

    app.finish_loading();
    let refined = app.diff_scroll();
    app.finish_merging();
    assert_eq!(refined, Some(parked), "the refinement un-parked the view");
    assert_eq!(app.diff_scroll(), Some(parked), "the merge un-parked it");
    assert_eq!(
        position(&app),
        Some(anchored),
        "the parked view kept its scroll but lost the cursor: {}",
        landed(&app)
    );
}
