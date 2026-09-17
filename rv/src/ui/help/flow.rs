//! Dealing the keymap's rows into columns.
//!
//! Top to bottom, then left to right, like a newspaper: every column is as
//! wide as its own rows need, a heading is never stranded as a column's last
//! row, and a spacer row before each heading is dropped at a column's top.
//!
//! The column height is chosen to fill the popup **evenly**: of every height
//! at which all the columns fit side by side, the one whose share of the
//! height and share of the width are closest — so a wide terminal gets more,
//! shorter columns rather than one tall one in the left-hand fifth of the
//! screen, and a tall one does not get a dozen stubby ones along the top.
//! Whatever width is left over is spread between the columns, up to
//! [`MAX_GAP`], so the text has room. A keymap that fits at no height
//! scrolls, a whole column at a time.

use super::HelpRow;

/// The widest the space between two columns grows to when spreading them:
/// past this the eye loses the row it was following across the screen.
const MAX_GAP: usize = 10;

pub(super) struct Column<'a> {
    /// `height` cells, `None` where a column ends early or a heading was
    /// pushed to the next.
    pub(super) rows: Vec<Option<&'a HelpRow>>,
    /// The chord column's width, and the description column's.
    pub(super) keys: usize,
    pub(super) what: usize,
    /// The widest heading, which spans both.
    heading: usize,
}

impl Column<'_> {
    pub(super) fn width(&self, gap: usize) -> usize {
        (self.keys + gap + self.what).max(self.heading)
    }
}

pub(super) struct Flowed<'a> {
    pub(super) columns: Vec<Column<'a>>,
    /// The space to draw between columns.
    pub(super) gap: usize,
    /// How many rows the columns are.
    pub(super) height: usize,
    /// Whether rows were left over that no column had room for.
    pub(super) truncated: bool,
}

/// Every column the rows fill at `height`, in order.
fn deal<'a>(rows: &'a [HelpRow], height: usize) -> Vec<Column<'a>> {
    let mut columns: Vec<Column<'a>> = Vec::new();
    let mut current: Vec<Option<&'a HelpRow>> = Vec::new();
    let close = |current: &mut Vec<Option<&'a HelpRow>>, columns: &mut Vec<Column<'a>>| {
        while matches!(current.last(), Some(Some(HelpRow::Blank))) {
            current.pop();
        }
        if current.is_empty() {
            return;
        }
        let (keys, what, heading) =
            current
                .iter()
                .flatten()
                .fold((0, 0, 0), |(k, w, h), row| match row {
                    HelpRow::Heading(text) => (k, w, h.max(text.chars().count())),
                    HelpRow::Key { chord, what, .. } => {
                        (k.max(chord.chars().count()), w.max(what.chars().count()), h)
                    }
                    HelpRow::Blank => (k, w, h),
                });
        let mut rows = std::mem::take(current);
        rows.resize(height, None);
        columns.push(Column {
            rows,
            keys,
            what,
            heading,
        });
    };
    for row in rows {
        if matches!(row, HelpRow::Blank) && current.is_empty() {
            continue;
        }
        let heading_would_strand =
            matches!(row, HelpRow::Heading(_)) && current.len() + 1 >= height && height > 1;
        if current.len() == height || heading_would_strand {
            close(&mut current, &mut columns);
            if matches!(row, HelpRow::Blank) {
                continue;
            }
        }
        current.push(Some(row));
    }
    close(&mut current, &mut columns);
    columns
}

/// How many of `columns` from `start` fit side by side in `width`, at
/// least one.
fn fitting(columns: &[Column<'_>], start: usize, width: usize, gap: usize) -> usize {
    let mut used = 0;
    let mut count = 0;
    for column in &columns[start..] {
        let needed = column.width(gap) + if count > 0 { gap } else { 0 };
        if used + needed > width && count > 0 {
            break;
        }
        used += needed;
        count += 1;
    }
    count
}

pub(super) fn flow<'a>(
    rows: &'a [HelpRow],
    height: usize,
    width: usize,
    gap: usize,
    scroll: usize,
) -> Flowed<'a> {
    let available = height.max(1);
    // Of the heights at which every column fits, the one that fills the
    // popup most evenly; failing that, the full height, and the rest scrolls.
    let evenness = |dealt: &[Column<'_>], rows_tall: usize| -> usize {
        let used: usize = dealt.iter().map(|column| column.width(gap)).sum::<usize>()
            + gap * dealt.len().saturating_sub(1);
        (rows_tall * 1000 / available).min(used * 1000 / width.max(1))
    };
    let (dealt, height) = (1..=available)
        .map(|rows_tall| (deal(rows, rows_tall), rows_tall))
        .filter(|(dealt, _)| fitting(dealt, 0, width, gap) == dealt.len())
        .max_by_key(|(dealt, rows_tall)| evenness(dealt, *rows_tall))
        .unwrap_or_else(|| (deal(rows, available), available));
    // Scrolling moves by whole columns, and never past the point where the
    // last column is on screen.
    let last_start = (0..dealt.len())
        .rev()
        .find(|start| start + fitting(&dealt, *start, width, gap) >= dealt.len())
        .unwrap_or(0);
    let start = scroll.min(last_start);
    let shown = fitting(&dealt, start, width, gap);
    let truncated = start + shown < dealt.len();
    let columns: Vec<Column<'a>> = dealt.into_iter().skip(start).take(shown).collect();
    let used: usize = columns.iter().map(|column| column.width(gap)).sum();
    let spread = match columns.len() {
        0 | 1 => gap,
        count => (width.saturating_sub(used) / (count - 1)).clamp(gap, MAX_GAP),
    };
    Flowed {
        columns,
        gap: spread,
        height,
        truncated,
    }
}
