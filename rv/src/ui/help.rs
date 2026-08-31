//! The `?` keymap: a contextual tip in the corner, or the whole table over the
//! panes.
//!
//! Both are drawn from [`BINDINGS`] rather than from lists of their own, which
//! is what makes "a binding that exists cannot be undocumented" true rather
//! than aspirational: there is no second table to forget to update. The tip is
//! the rows whose [`Binding::contexts`] name the context the cursor is in; the
//! full popup is every row, grouped.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;

use super::BORDER_ROWS;
use super::text::clip_spans;
use crate::app::App;
use crate::app::BINDINGS;
use crate::app::Binding;
use crate::app::Group;
use crate::app::Leader;

/// Columns between the key column and its description, and between one column
/// of the popup and the next.
const HELP_GAP: usize = 2;

/// One row of the popup: a group's heading, or one binding.
enum HelpRow {
    Heading(&'static str),
    Key {
        binding: &'static Binding,
        enabled: bool,
    },
}

/// How a binding is spelled in the full keymap: the chord a reviewer presses. A
/// direct key is itself; a key under a leader is the leader then the key — `v f`.
/// The `Space` leader is drawn as `⎵` here so its column stays as narrow as the
/// letter leaders', which is what lets the map deal into three columns at 80x24.
fn chord(binding: &Binding) -> String {
    match binding.leader {
        Some(Leader::Mode) => format!("⎵ {}", binding.keys),
        Some(leader) => format!("{} {}", leader.label(), binding.keys),
        None => binding.keys.to_owned(),
    }
}

/// One row of the layers overview `?` shows: how a key or a leader is spelled,
/// and what it reaches.
struct Layer {
    keys: &'static str,
    what: &'static str,
}

/// The layers overview: the handful of keys that move around, and the four
/// leaders every other key hangs off. `?` shows this; `? ?` opens the whole map.
const LAYERS: &[Layer] = &[
    Layer {
        keys: "↑↓",
        what: "move",
    },
    Layer {
        keys: "←→",
        what: "out / in",
    },
    Layer {
        keys: "Tab",
        what: "tree / diff",
    },
    Layer {
        keys: "Space",
        what: "mode …",
    },
    Layer {
        keys: "g",
        what: "goto …",
    },
    Layer {
        keys: "c",
        what: "comment …",
    },
    Layer {
        keys: "v",
        what: "view …",
    },
    Layer {
        keys: "? ?",
        what: "all keys",
    },
];

/// How big the tip wants to be, borders included: one row per layer, wide enough
/// for its longest row and its own title.
///
/// Asked by [`crate::ui`] before the layout is computed, because only this
/// module knows what the tip says; where the box goes is [`crate::layout`]'s.
#[must_use]
pub fn tip_size(app: &App) -> (u16, u16) {
    let keys = LAYERS
        .iter()
        .map(|l| l.keys.chars().count())
        .max()
        .unwrap_or(0);
    let what = LAYERS
        .iter()
        .map(|l| l.what.chars().count())
        .max()
        .unwrap_or(0);
    let inner = (keys + HELP_GAP + what).max(tip_title(app).chars().count());
    (
        u16::try_from(LAYERS.len())
            .unwrap_or(u16::MAX)
            .saturating_add(BORDER_ROWS),
        u16::try_from(inner)
            .unwrap_or(u16::MAX)
            .saturating_add(BORDER_ROWS + 2),
    )
}

/// What the tip calls itself: where the reviewer is, and the way to the whole
/// keymap.
fn tip_title(app: &App) -> String {
    format!("▸ {} — ? ? all keys", app.context().name())
}

/// The layers overview: the leaders and the moves that reach every key, above
/// the bar's `? help` hint. `? ?` unrolls the whole keymap.
pub(super) fn draw_tip(frame: &mut Frame, app: &App, area: Rect) {
    let keys = LAYERS
        .iter()
        .map(|l| l.keys.chars().count())
        .max()
        .unwrap_or(0);
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS)).saturating_sub(1);
    let lines: Vec<Line<'static>> = LAYERS
        .iter()
        .map(|layer| {
            clip_spans(
                vec![
                    Span::raw(" "),
                    Span::styled(
                        format!("{:<keys$}", layer.keys),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" ".repeat(HELP_GAP)),
                    Span::raw(layer.what),
                ],
                width,
            )
        })
        .collect();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(tip_title(app)),
        ),
        area,
    );
}

pub(super) fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let height = usize::from(area.height.saturating_sub(BORDER_ROWS));
    let text = help_text(app, width, height);
    // The popup covers what is under it rather than blending with it: a keymap
    // read through a diff is a keymap read twice.
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title("▸ Keys — ? or Esc to close"),
        ),
        area,
    );
}

/// The keymap laid out in as many columns as `width` fits.
///
/// One column of twenty-one rows does not fit the fourteen a 70%-of-24-rows
/// popup has, and 80x24 is what a reviewer over ssh actually has — so the
/// columns are not decoration. The narrowest number of rows that fits is
/// chosen, and a group is never split across a column boundary: a heading with
/// nothing under it teaches nothing.
///
/// A popup too small for even that falls back to a single scrolling column, and
/// [`scrolled`] is the only place [`App::help_scroll`] is used.
fn help_text(app: &App, width: usize, height: usize) -> Text<'static> {
    let blocks = help_blocks(app);
    let keys = BINDINGS
        .iter()
        .map(|binding| chord(binding).chars().count())
        .max()
        .unwrap_or(0);
    let what = BINDINGS
        .iter()
        .map(|binding| binding.what.chars().count())
        .max()
        .unwrap_or(0);
    let column = keys + HELP_GAP + what;
    // `(width + gap) / (column + gap)`: n columns need n-1 gaps between them.
    let columns = ((width + HELP_GAP) / (column + HELP_GAP)).max(1);

    let packed = (1..=height)
        .find_map(|rows| pack(&blocks, rows).filter(|packing| packing.len() <= columns));
    let packed = packed.unwrap_or_else(|| scrolled(&blocks, height, app.help_scroll()));

    let rows = packed.iter().map(Vec::len).max().unwrap_or(0);
    let lines = (0..rows)
        .map(|row| {
            let mut spans = Vec::with_capacity(packed.len() * 3);
            for (index, cells) in packed.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw(" ".repeat(HELP_GAP)));
                }
                spans.extend(help_cell(cells.get(row), keys, what));
            }
            clip_spans(spans, width)
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

/// One cell of the popup's grid, padded to the column's width so the ones under
/// it line up.
fn help_cell(row: Option<&&HelpRow>, keys: usize, what: usize) -> Vec<Span<'static>> {
    let column = keys + HELP_GAP + what;
    match row {
        None => vec![Span::raw(" ".repeat(column))],
        Some(HelpRow::Heading(heading)) => vec![Span::styled(
            format!("{heading:<column$}"),
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )],
        Some(HelpRow::Key { binding, enabled }) => {
            // Dim rather than hidden: a reviewer should see that the key exists
            // and that here is the wrong place for it.
            let (key_style, what_style) = if *enabled {
                (
                    Style::default().add_modifier(Modifier::BOLD),
                    Style::default(),
                )
            } else {
                let dim = Style::default().add_modifier(Modifier::DIM);
                (dim, dim)
            };
            vec![
                Span::styled(format!("{:<keys$}", chord(binding)), key_style),
                Span::raw(" ".repeat(HELP_GAP)),
                Span::styled(format!("{:<what$}", binding.what), what_style),
            ]
        }
    }
}

/// The keymap as one block per [`Group`]: its heading, then its bindings in
/// table order.
fn help_blocks(app: &App) -> Vec<Vec<HelpRow>> {
    Group::ALL
        .iter()
        .map(|group| {
            let mut rows = vec![HelpRow::Heading(group.heading())];
            rows.extend(
                BINDINGS
                    .iter()
                    .filter(|binding| binding.group == *group)
                    .map(|binding| HelpRow::Key {
                        binding,
                        enabled: app.binding_enabled(binding),
                    }),
            );
            rows
        })
        .filter(|rows| rows.len() > 1)
        .collect()
}

/// Deals `blocks` into columns of at most `rows` rows each, keeping every block
/// whole. `None` when some block is taller than a column can be.
fn pack(blocks: &[Vec<HelpRow>], rows: usize) -> Option<Vec<Vec<&HelpRow>>> {
    if rows == 0 {
        return None;
    }
    let mut columns: Vec<Vec<&HelpRow>> = vec![Vec::new()];
    for block in blocks {
        if block.len() > rows {
            return None;
        }
        let last = columns.last_mut().expect("there is always one column");
        if last.len() + block.len() > rows {
            columns.push(Vec::new());
        }
        columns
            .last_mut()
            .expect("there is always one column")
            .extend(block.iter());
    }
    Some(columns)
}

/// The fallback for a popup too small to hold the keymap however it is dealt:
/// one column, `height` rows of it, starting `scroll` rows in.
///
/// Clamped here rather than in [`App`], which deliberately knows nothing about
/// how big the terminal is: holding `j` down cannot scroll past the end.
fn scrolled(blocks: &[Vec<HelpRow>], height: usize, scroll: usize) -> Vec<Vec<&HelpRow>> {
    let flat: Vec<&HelpRow> = blocks.iter().flatten().collect();
    let start = scroll.min(flat.len().saturating_sub(height));
    let end = start.saturating_add(height).min(flat.len());
    vec![flat[start..end].to_vec()]
}
