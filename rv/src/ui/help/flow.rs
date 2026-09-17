//! Dealing the keymap's rows into columns.
//!
//! Top to bottom, then left to right, like a newspaper: every column is
//! `height` rows and as wide as its own rows need, and a heading is never
//! stranded as a column's last row. Columns are added while they fit the
//! width; whatever does not fit is reached by scrolling, a whole row of
//! columns at a time.

use super::HelpRow;

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
    /// Whether rows were left over that no column had room for.
    pub(super) truncated: bool,
}

/// Every column the rows fill at `height`, in order.
fn deal<'a>(rows: &'a [HelpRow], height: usize) -> Vec<Column<'a>> {
    let mut columns: Vec<Column<'a>> = Vec::new();
    let mut current: Vec<Option<&'a HelpRow>> = Vec::new();
    let close = |current: &mut Vec<Option<&'a HelpRow>>, columns: &mut Vec<Column<'a>>| {
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
        let heading_would_strand =
            matches!(row, HelpRow::Heading(_)) && current.len() + 1 == height && height > 1;
        if current.len() == height || heading_would_strand {
            close(&mut current, &mut columns);
        }
        current.push(Some(row));
    }
    close(&mut current, &mut columns);
    columns
}

pub(super) fn flow<'a>(
    rows: &'a [HelpRow],
    height: usize,
    width: usize,
    gap: usize,
    scroll: usize,
) -> Flowed<'a> {
    let height = height.max(1);
    let dealt = deal(rows, height);
    // Scrolling moves by whole columns, and never past the point where the
    // last column is on screen.
    let fits_from = |start: usize| -> usize {
        let mut used = 0;
        let mut count = 0;
        for column in &dealt[start..] {
            let needed = column.width(gap) + if count > 0 { gap } else { 0 };
            if used + needed > width && count > 0 {
                break;
            }
            used += needed;
            count += 1;
        }
        count
    };
    let last_start = (0..dealt.len())
        .rev()
        .find(|start| start + fits_from(*start) >= dealt.len())
        .unwrap_or(0);
    let start = scroll.min(last_start);
    let shown = fits_from(start);
    let truncated = start + shown < dealt.len();
    let columns = dealt.into_iter().skip(start).take(shown).collect();
    Flowed { columns, truncated }
}
