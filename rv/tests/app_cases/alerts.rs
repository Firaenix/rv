//! Alerts.

use std::cell::RefCell;
use std::time::Duration;
use std::time::Instant;
use proptest::prelude::*;
use rv::app::App;
use rv::session;
use rv_core::model::ChangeKind;
use rv_core::model::FileChange;

use crate::support::*;

/// About five seconds, from spec §9 — written here rather than read off the
/// implementation, because a test that asks the code what its own deadline is
/// asserts nothing about the deadline.
const ALERT_LIFETIME: Duration = Duration::from_secs(5);

/// An app holding a live alert never lets the event loop block past that
/// alert's deadline, and an app holding none never asks it to wake up at all.
///
/// The wake-up is the whole of what makes "it leaves on its own" true: a toast
/// raised at t=0 in front of a reviewer who then walks away is still on screen
/// at t=∞ if nothing tells `event::poll` when to come back.
#[test]
fn a_live_alert_always_asks_the_loop_back_before_it_expires() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    run_cases(
        64,
        (1usize..4, 0u64..7000, 0u64..7000),
        |(count, first, second)| {
            let app = &mut *app.borrow_mut();
            let t0 = Instant::now();
            app.expire_alerts(t0 + Duration::from_secs(60));
            prop_assert!(
                app.alerts().is_empty(),
                "the last case left an alert behind"
            );

            for index in 0..count {
                app.alert(format!("alert {index}"), t0);
            }
            for elapsed in [0, first.min(second), second.max(first)] {
                let now = t0 + Duration::from_millis(elapsed);
                app.expire_alerts(now);
                match app.next_deadline(now) {
                    Some(wait) => prop_assert!(
                        wait <= ALERT_LIFETIME,
                        "the loop was told to sleep {wait:?}, past the alert's whole life"
                    ),
                    None => prop_assert!(
                        app.alerts().is_empty(),
                        "a live alert let the loop block forever"
                    ),
                }
            }
            Ok(())
        },
    );
}

/// A blob the repository refuses to read is an alert rather than a silent zero.
///
/// `App::measure` diffs every file in the review before the first frame so the
/// sidebar's counts and colours are facts about the whole review, and it
/// swallows a read failure so that one bad file cannot stop a review of five
/// hundred from opening. Swallowing it *silently* is the part that was wrong:
/// the row then reads `+0 -0` and looks like a file nobody touched.
///
/// Provoked the way `a_review_with_no_changes_refuses_to_attribute_a_comment`
/// provokes its branch — by assembling a [`session::Review`], which is `pub`
/// with `pub` fields, around a path jj will not resolve.
#[test]
fn a_blob_that_cannot_be_read_is_an_alert_rather_than_a_silent_zero() {
    let fixture = Fixture::multi();
    let mut review = session::build(fixture.root(), Some("@--"), None).expect("build the review");
    assert!(
        review
            .repo
            .read_blob(&review.session.head_commit, "/outside.rs")
            .is_err(),
        "the path this case is built on resolves after all"
    );
    review.files.push(FileChange {
        path: "/outside.rs".to_owned(),
        source_path: None,
        kind: ChangeKind::Modified,
        binary: false,
    });

    let app = App::new(review).expect("open the reviewer");
    let messages: Vec<&str> = app
        .alerts()
        .iter()
        .map(|alert| alert.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("outside.rs")),
        "the unreadable blob was measured as zero and never mentioned: {messages:?}"
    );
}
