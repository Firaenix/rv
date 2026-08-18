//! Tests for the one function that decides where everything is.
//!
//! [`rv::layout`] is pure — an area, a split and what chrome is showing go in,
//! every rectangle comes out — so nothing below needs a terminal, a `Frame` or
//! an [`rv::app::App`]. That is the whole point of the module: `ui::draw`
//! paints from a [`rv::layout::Layout`] and [`rv::layout::hit`] reads from the
//! same one, so a click cannot land somewhere other than what was drawn.
//!
//! The property at the bottom is the one that matters. Every cell of either
//! pane must answer with exactly the row that was painted there — and with
//! *nothing* on the two rows that are border rather than content — at every
//! geometry a terminal can be, because the failure this module exists to
//! prevent is silent: a click that resolves to the wrong row looks exactly like
//! a click that resolved to the right one.
//!
//! That property is only as good as the range it walks. Its first version
//! asked whether each cell hit *something* over `rect.y + 1..rect.bottom()`,
//! which skipped the top border deliberately and stopped one row short of the
//! bottom border by accident — so the bottom border sat inside the range,
//! answering with a content row one past the last one drawn, and the property
//! reported green at every terminal size.

use proptest::prelude::*;
use ratatui::layout::Rect;
use rstest::rstest;
use rv::layout::Chrome;
use rv::layout::Split;
use rv::layout::Target;
use rv::layout::hit;
use rv::layout::layout;

/// The chrome of a plain browsing frame: a one-row bar, no popup, no toast.
fn browsing() -> Chrome {
    Chrome {
        bar_rows: 1,
        help_open: false,
        toast: false,
    }
}

// ---------------------------------------------------------------------------
// Where the pieces go
// ---------------------------------------------------------------------------

/// The bar is the *last* row, not the first. It was drawn above the panes
/// until this module existed; nvim, tmux and zellij all put it below, which is
/// where a reader's eye goes for state rather than content.
#[test]
fn the_bar_sits_along_the_bottom_under_both_panes() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    assert_eq!(l.bar.height, 1);
    assert_eq!(l.bar.bottom(), 24, "the bar is the last row of the area");
    assert_eq!(l.bar.width, 100, "it spans both panes");
    assert_eq!(
        l.sidebar.bottom(),
        l.bar.y,
        "the panes stop where the bar starts"
    );
    assert_eq!(l.diff.bottom(), l.bar.y);
    assert_eq!(l.sidebar.y, 0, "and start at the top of the area");
}

#[test]
fn the_panes_tile_the_area_with_a_divider_between_them() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    assert_eq!(l.bar.height, 1);
    assert_eq!(l.sidebar.x, 0);
    assert_eq!(l.divider.width, 1, "the divider is one column");
    assert_eq!(l.sidebar.right(), l.divider.x, "no gap before the divider");
    assert_eq!(l.divider.right(), l.diff.x, "no gap after it");
    assert_eq!(l.diff.right(), 100, "the panes reach the right edge");
    assert_eq!(
        l.sidebar.height, l.diff.height,
        "the panes are the same height"
    );
    assert_eq!(
        l.divider.y, l.sidebar.y,
        "the divider runs the full height of the panes"
    );
    assert_eq!(l.divider.bottom(), l.sidebar.bottom());
}

/// The layout is drawn wherever it is put, not at the origin. A sub-area is
/// how a popup or a future embedded view would be laid out, and a layout that
/// silently snapped to `(0, 0)` would paint over its host.
#[test]
fn the_layout_respects_an_area_that_does_not_start_at_the_origin() {
    let l = layout(Rect::new(4, 2, 60, 20), Split::new(30), browsing());
    assert_eq!(l.sidebar.x, 4);
    assert_eq!(l.sidebar.y, 2);
    assert_eq!(l.diff.right(), 64);
    assert_eq!(l.bar.bottom(), 22);
    assert_eq!(l.bar.x, 4);
}

/// A comment box needs three rows, and it takes them from the panes rather
/// than from the screen: the bar is still the bottom of the area.
#[test]
fn the_bar_grows_downwards_when_the_comment_box_opens() {
    let area = Rect::new(0, 0, 100, 24);
    let browse = layout(area, Split::new(30), browsing());
    let comment = layout(
        area,
        Split::new(30),
        Chrome {
            bar_rows: 3,
            ..browsing()
        },
    );

    assert_eq!(comment.bar.height, 3);
    assert_eq!(
        comment.bar.bottom(),
        24,
        "it is still the bottom of the area"
    );
    assert_eq!(
        comment.diff.height + 2,
        browse.diff.height,
        "the two rows come out of the panes"
    );
    assert_eq!(comment.diff.bottom(), comment.bar.y);
}

#[rstest]
#[case(100, 30)]
#[case(40, 30)]
#[case(24, 50)]
fn the_sidebar_honours_its_minimum_or_the_area_halves(#[case] width: u16, #[case] ratio: u16) {
    let l = layout(Rect::new(0, 0, width, 24), Split::new(ratio), browsing());
    let sidebar = l.sidebar.width;
    let diff = l.diff.width;
    assert!(
        sidebar > 0 && diff > 0,
        "neither pane vanishes at width {width}"
    );
    // The divider is neither pane's, so the floors are about what is left.
    let shared = width.saturating_sub(l.divider.width);
    if shared >= Split::MIN_SIDEBAR + Split::MIN_DIFF {
        assert!(
            sidebar >= Split::MIN_SIDEBAR,
            "sidebar keeps its floor when there is room"
        );
        assert!(
            diff >= Split::MIN_DIFF,
            "the diff keeps its floor when there is room"
        );
    }
}

/// The ratio is a percentage of the space the two panes share, so a wider
/// terminal gives the sidebar more columns.
#[test]
fn a_larger_ratio_gives_the_sidebar_more_columns() {
    let area = Rect::new(0, 0, 200, 24);
    let narrow = layout(area, Split::new(20), browsing()).sidebar.width;
    let wide = layout(area, Split::new(60), browsing()).sidebar.width;
    assert!(narrow < wide, "{narrow} is not narrower than {wide}");
}

/// A terminal too small for either floor still renders, and renders something
/// usable: whatever there is, split evenly. A `u16` subtraction that
/// underflows is the classic ratatui panic, and a reviewer who resized their
/// window is not interested in a backtrace.
#[rstest]
#[case(0, 0)]
#[case(0, 24)]
#[case(1, 1)]
#[case(3, 2)]
#[case(100, 0)]
#[case(100, 1)]
fn a_terminal_too_small_for_the_layout_does_not_panic(#[case] width: u16, #[case] height: u16) {
    let l = layout(Rect::new(0, 0, width, height), Split::new(30), browsing());
    assert!(l.sidebar.right() <= width);
    assert!(l.diff.right() <= width);
    assert!(l.bar.bottom() <= height);
    assert!(l.diff.bottom() <= height);
    assert_eq!(hit(&l, width, height), None, "nothing is outside the area");
}

#[test]
fn nudging_the_split_stays_inside_its_bounds() {
    let mut split = Split::new(Split::DEFAULT);
    for _ in 0..100 {
        split = split.nudged(2);
    }
    assert!(
        split.ratio() <= 80,
        "cannot be dragged past the right bound"
    );
    for _ in 0..200 {
        split = split.nudged(-2);
    }
    assert!(split.ratio() >= 5, "cannot be dragged past the left bound");
}

/// A nudge that stays inside the bounds moves by exactly what it was asked
/// for: the clamp is a fence, not the behaviour.
#[test]
fn a_nudge_inside_the_bounds_moves_by_its_delta() {
    assert_eq!(Split::new(30).nudged(2).ratio(), 32);
    assert_eq!(Split::new(30).nudged(-2).ratio(), 28);
    assert_eq!(Split::new(30).nudged(0).ratio(), 30);
}

// ---------------------------------------------------------------------------
// What is under the pointer
// ---------------------------------------------------------------------------

#[test]
fn a_click_on_the_divider_reports_the_divider() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    assert_eq!(hit(&l, l.divider.x, 5), Some(Target::Divider));
}

#[test]
fn a_click_in_a_pane_reports_the_row_under_the_pointer() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    let first = l.diff.y + 1; // +1 for the pane's top border
    assert_eq!(hit(&l, l.diff.x + 3, first), Some(Target::DiffRow(0)));
    assert_eq!(hit(&l, l.diff.x + 3, first + 4), Some(Target::DiffRow(4)));
    assert_eq!(
        hit(&l, l.sidebar.x + 1, first + 2),
        Some(Target::SidebarRow(2))
    );
}

/// A row index is relative to the pane, not to the screen, so the same row of
/// content answers the same index however tall the bar under it is.
#[test]
fn a_row_index_does_not_move_when_the_bar_changes_height() {
    let area = Rect::new(0, 0, 100, 24);
    let browse = layout(area, Split::new(30), browsing());
    let comment = layout(
        area,
        Split::new(30),
        Chrome {
            bar_rows: 3,
            ..browsing()
        },
    );
    assert_eq!(hit(&browse, 40, 6), Some(Target::DiffRow(5)));
    assert_eq!(hit(&comment, 40, 6), Some(Target::DiffRow(5)));
}

#[test]
fn a_click_on_the_bar_reports_the_bar() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    assert_eq!(hit(&l, 0, l.bar.y), Some(Target::Bar));
    assert_eq!(hit(&l, 99, l.bar.y), Some(Target::Bar));
}

#[test]
fn a_click_outside_everything_reports_nothing() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    assert_eq!(hit(&l, 200, 200), None);
}

/// The top border carries the pane's title, not a row of content, so a click
/// on it selects nothing rather than selecting the first row.
#[test]
fn a_click_on_a_panes_top_border_reports_nothing() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    assert_eq!(hit(&l, l.diff.x + 3, l.diff.y), None);
    assert_eq!(hit(&l, l.sidebar.x + 1, l.sidebar.y), None);
}

/// And neither does the bottom one. A pane is a bordered block, so a rect of
/// height `h` paints `h - 2` rows inside it and the last one a click can land
/// on is `bottom() - 2`. Counting `bottom() - 1` as content hands the caller a
/// row index one past everything that was drawn: a click on the bottom edge of
/// the file list selects nothing, or — once the caller clamps — the wrong file.
#[test]
fn a_click_on_a_panes_bottom_border_reports_nothing() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    assert_eq!(hit(&l, l.diff.x + 3, l.diff.bottom() - 1), None);
    assert_eq!(hit(&l, l.sidebar.x + 1, l.sidebar.bottom() - 1), None);
    assert_eq!(
        hit(&l, l.diff.x + 3, l.diff.bottom() - 2),
        Some(Target::DiffRow(usize::from(l.diff.height) - 3)),
        "the row above it is the last one the pane draws"
    );
    assert_eq!(
        hit(&l, l.sidebar.x + 1, l.sidebar.bottom() - 2),
        Some(Target::SidebarRow(usize::from(l.sidebar.height) - 3)),
    );
}

/// A pane with no room between its borders has no rows to click at all, rather
/// than one row that is both borders at once.
#[rstest]
#[case(3)] // two pane rows under the bar: both of them border
#[case(2)] // one pane row: the top border, with no bottom to reach
fn a_pane_too_short_for_content_reports_no_rows(#[case] height: u16) {
    let l = layout(Rect::new(0, 0, 100, height), Split::new(30), browsing());
    for row in 0..height {
        for column in [l.sidebar.x + 1, l.diff.x + 1] {
            assert!(
                !matches!(
                    hit(&l, column, row),
                    Some(Target::SidebarRow(_) | Target::DiffRow(_))
                ),
                "({column},{row}) is a content row in a {height}-row terminal"
            );
        }
    }
}

#[test]
fn the_popup_takes_priority_over_whatever_is_beneath_it() {
    let l = layout(
        Rect::new(0, 0, 100, 24),
        Split::new(30),
        Chrome {
            help_open: true,
            ..browsing()
        },
    );
    let popup = l.popup.expect("the popup has a rect when it is open");
    assert_eq!(hit(&l, popup.x + 2, popup.y + 2), Some(Target::Popup));
}

#[test]
fn there_is_no_popup_while_the_help_is_closed() {
    let l = layout(Rect::new(0, 0, 100, 24), Split::new(30), browsing());
    assert_eq!(l.popup, None);
    assert_eq!(l.toast, None, "and no toast while nothing is alerting");
}

/// The popup is centred and leaves the panes visible around it, so the
/// reviewer can still see what the keys they are reading about would act on.
#[test]
fn the_popup_is_centred_inside_the_area() {
    let area = Rect::new(0, 0, 100, 24);
    let l = layout(
        area,
        Split::new(30),
        Chrome {
            help_open: true,
            ..browsing()
        },
    );
    let popup = l.popup.expect("a rect when it is open");
    assert!(
        popup.width >= 20 && popup.height >= 6,
        "{popup:?} is too small to read"
    );
    assert!(popup.right() <= area.right() && popup.bottom() <= area.bottom());
    assert_eq!(
        popup.x - area.x,
        area.right() - popup.right(),
        "the same margin on both sides"
    );
    assert_eq!(popup.y - area.y, area.bottom() - popup.bottom());
}

/// A toast is drawn over the panes but is never a click target: it takes no
/// key and no gesture, and it leaves on its own. Clicking where one happens to
/// be floating still reaches the pane beneath it.
#[test]
fn the_toast_floats_at_the_top_and_swallows_no_clicks() {
    let area = Rect::new(0, 0, 100, 24);
    let l = layout(
        area,
        Split::new(30),
        Chrome {
            toast: true,
            ..browsing()
        },
    );
    let toast = l.toast.expect("a rect while something is alerting");
    assert!(
        toast.bottom() <= area.height / 2,
        "{toast:?} is not near the top"
    );
    assert_eq!(
        toast.x - area.x,
        area.right() - toast.right(),
        "the same margin on both sides"
    );
    assert_eq!(
        hit(&l, toast.x + 2, toast.y + 1),
        hit(
            &layout(area, Split::new(30), browsing()),
            toast.x + 2,
            toast.y + 1
        ),
        "the toast changed what a click means"
    );
}

// ---------------------------------------------------------------------------
// The width the split hands out
// ---------------------------------------------------------------------------

#[test]
fn the_split_hands_out_the_columns_the_panes_share() {
    assert_eq!(Split::new(30).sidebar_width(99), 29);
    assert_eq!(Split::new(50).sidebar_width(100), 50);
    assert_eq!(
        Split::new(80).sidebar_width(99),
        99 - Split::MIN_DIFF,
        "the diff keeps its floor even at the widest split"
    );
    assert_eq!(
        Split::new(5).sidebar_width(99),
        Split::MIN_SIDEBAR,
        "and the sidebar keeps its own at the narrowest"
    );
    assert_eq!(
        Split::new(30).sidebar_width(20),
        10,
        "too small for either floor: halve it"
    );
    assert_eq!(Split::new(30).sidebar_width(0), 0);
}

// ---------------------------------------------------------------------------
// The property the whole module exists for
// ---------------------------------------------------------------------------

proptest! {
    /// Every cell of a pane answers with the row that was painted there, and
    /// the pane's two border rows answer with nothing at all.
    ///
    /// The whole rect is walked, borders included, because both edges of the
    /// range are where the arithmetic goes wrong. A predecessor of this test
    /// walked `rect.y + 1..rect.bottom()` and only asked whether the answer was
    /// `Some`: it never saw that the bottom border was reporting content row
    /// `height - 2`, one past the last row `draw` paints, at every size a
    /// terminal can be.
    #[test]
    fn every_cell_of_a_pane_round_trips_to_the_row_that_was_painted_there(
        width in 8u16..120, height in 4u16..40, ratio in 5u16..80,
    ) {
        let area = Rect::new(0, 0, width, height);
        let l = layout(area, Split::new(ratio), browsing());
        for (rect, name, row_target) in [
            (l.sidebar, "sidebar", Target::SidebarRow as fn(usize) -> Target),
            (l.diff, "diff", Target::DiffRow as fn(usize) -> Target),
        ] {
            for row in rect.y..rect.bottom() {
                // On a pane one row tall the single row is both borders; the
                // arithmetic must not decide it is neither.
                let border = row == rect.y || row + 1 == rect.bottom();
                let expected = (!border).then(|| row_target(usize::from(row - rect.y - 1)));
                for column in rect.x..rect.right() {
                    prop_assert_eq!(
                        hit(&l, column, row), expected,
                        "{} cell ({}, {}) in {:?}", name, column, row, rect
                    );
                }
            }

            // And the rows that do answer are exactly the ones inside the
            // borders: as many as `draw` has to paint into, numbered from zero.
            let inner = usize::from(rect.height.saturating_sub(2));
            let answered = (rect.y..rect.bottom())
                .filter(|&row| hit(&l, rect.x, row).is_some())
                .count();
            prop_assert_eq!(answered, inner, "{} answered {} of {} rows", name, answered, inner);
        }
        prop_assert_eq!(hit(&l, l.divider.x, l.divider.y), Some(Target::Divider));
    }

    /// A cell in one pane never answers with the other pane's rows: the
    /// divider is the fence, and a click one column either side of it belongs
    /// to exactly one list. Borders included — a border that leaked would leak
    /// into the *neighbouring* pane's rows, which is the worse failure.
    #[test]
    fn no_cell_belongs_to_both_panes(
        width in 8u16..120, height in 4u16..40, ratio in 5u16..80,
    ) {
        let l = layout(Rect::new(0, 0, width, height), Split::new(ratio), browsing());
        for row in l.sidebar.y..l.sidebar.bottom() {
            for column in l.sidebar.x..l.sidebar.right() {
                prop_assert!(
                    !matches!(hit(&l, column, row), Some(Target::DiffRow(_))),
                    "sidebar cell ({column},{row}) answered {:?}", hit(&l, column, row)
                );
            }
            for column in l.diff.x..l.diff.right() {
                prop_assert!(
                    !matches!(hit(&l, column, row), Some(Target::SidebarRow(_))),
                    "diff cell ({column},{row}) answered {:?}", hit(&l, column, row)
                );
            }
        }
    }
}
