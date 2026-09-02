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
use crate::app::Group;
use crate::app::Leader;
use crate::app::keymap::RuntimeBinding;

const HELP_GAP: usize = 2;

enum HelpRow {
    Heading(&'static str),
    Key {
        chord: String,
        what: String,
        enabled: bool,
    },
}

fn row_chord(row: &HelpRow) -> Option<&str> {
    match row {
        HelpRow::Key { chord, .. } => Some(chord),
        HelpRow::Heading(_) => None,
    }
}

fn rt_chord(binding: &RuntimeBinding, app: &App) -> String {
    match binding.leader {
        Some(Leader::Mode) => format!("⎵ {}", binding.keys_label),
        Some(leader) => {
            let key = app.keymap().leader_key(leader);
            let label = if key == ' ' {
                "Space".to_owned()
            } else {
                key.to_string()
            };
            format!("{} {}", label, binding.keys_label)
        }
        None => binding.keys_label.clone(),
    }
}

struct Layer {
    keys: &'static str,
    what: &'static str,
}

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
        what: "next mode",
    },
    Layer {
        keys: "Space",
        what: "actions here …",
    },
    Layer {
        keys: "m",
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

fn tip_title(app: &App) -> String {
    format!("▸ {} — ? ? all keys", app.context().name())
}

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

fn help_text(app: &App, width: usize, height: usize) -> Text<'static> {
    let blocks = help_blocks(app);
    let shown = || blocks.iter().flatten().filter_map(row_chord);
    let keys = shown().map(|c| c.chars().count()).max().unwrap_or(0);
    let what = blocks
        .iter()
        .flatten()
        .filter_map(|row| match row {
            HelpRow::Key { what, .. } => Some(what.chars().count()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let column = keys + HELP_GAP + what;
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

fn help_cell(row: Option<&&HelpRow>, keys_w: usize, what_w: usize) -> Vec<Span<'static>> {
    let column = keys_w + HELP_GAP + what_w;
    match row {
        None => vec![Span::raw(" ".repeat(column))],
        Some(HelpRow::Heading(heading)) => vec![Span::styled(
            format!("{heading:<column$}"),
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )],
        Some(HelpRow::Key {
            chord,
            what,
            enabled,
        }) => {
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
                Span::styled(format!("{chord:<keys_w$}"), key_style),
                Span::raw(" ".repeat(HELP_GAP)),
                Span::styled(format!("{what:<what_w$}"), what_style),
            ]
        }
    }
}

fn help_blocks(app: &App) -> Vec<Vec<HelpRow>> {
    Group::ALL
        .iter()
        .map(|group| {
            let mut rows = vec![HelpRow::Heading(group.heading())];
            rows.extend(
                app.keymap()
                    .bindings()
                    .iter()
                    .filter(|binding| {
                        binding.group == *group && binding.leader != Some(Leader::Context)
                    })
                    .map(|binding| HelpRow::Key {
                        chord: rt_chord(binding, app),
                        what: binding.what.to_owned(),
                        enabled: app.rt_binding_enabled(binding),
                    }),
            );
            rows
        })
        .filter(|rows| rows.len() > 1)
        .collect()
}

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

fn scrolled(blocks: &[Vec<HelpRow>], height: usize, scroll: usize) -> Vec<Vec<&HelpRow>> {
    let flat: Vec<&HelpRow> = blocks.iter().flatten().collect();
    let start = scroll.min(flat.len().saturating_sub(height));
    let end = start.saturating_add(height).min(flat.len());
    vec![flat[start..end].to_vec()]
}
