//! The `?` tip and the `? ?` keymap, both drawn from the runtime keymap —
//! the shipped table plus whatever `keybindings.toml` changed — so a key can
//! be rebound, added or unbound without either popup needing to be told.
//!
//! The keymap is dealt into **sections**: everything that works everywhere,
//! grouped as the binding table groups it, and then one section per pane for
//! the keys that only mean something there. A key that acts in two panes is
//! listed under both: the reader is looking for "what can I press *here*",
//! and a row that says so twice costs less than one they have to find.
//!
//! The sections flow into as many columns as the popup is wide, top to
//! bottom then left to right, each column as wide as its own rows need. A
//! keymap taller than the popup scrolls. Nothing here is packed by hand.

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
use crate::app::Context;
use crate::app::Group;
use crate::app::Leader;
use crate::app::keymap::RuntimeBinding;

mod flow;

use flow::flow;

const HELP_GAP: usize = 2;

/// What the last row of a scrolled keymap says.
const MORE: &str = "… j/k for more";

/// The panes a binding can be scoped to, in the order the keymap lists them.
const PANES: &[Context] = &[
    Context::Files,
    Context::Commits,
    Context::Comments,
    Context::Flags,
    Context::Diff,
    Context::Stack,
];

pub(super) enum HelpRow {
    Heading(String),
    Key {
        chord: String,
        what: String,
        enabled: bool,
    },
}

/// The key sequence that runs `binding`, as the reviewer would type it: the
/// leader's *runtime* key, then the binding's own.
fn chord(binding: &RuntimeBinding, app: &App) -> String {
    match binding.leader {
        Some(leader) => format!("{} {}", leader_label(app, leader), binding.keys_label),
        None => binding.keys_label.clone(),
    }
}

fn leader_label(app: &App, leader: Leader) -> String {
    let key = app.keymap().leader_key(leader);
    if key == ' ' {
        "Space".to_owned()
    } else {
        key.to_string()
    }
}

fn pane_title(context: Context) -> &'static str {
    match context {
        Context::Files => "Files list",
        Context::Commits => "Commits list",
        Context::Comments => "Comments list",
        Context::Flags => "Flags list",
        Context::Diff => "Diff",
        Context::Stack => "Comment stack",
        Context::Writing | Context::Confirming | Context::Finding => "",
    }
}

fn key_row(binding: &RuntimeBinding, app: &App) -> HelpRow {
    HelpRow::Key {
        chord: chord(binding, app),
        what: binding.what.to_owned(),
        enabled: app.rt_binding_enabled(binding),
    }
}

/// Every row of the keymap in reading order: the global keys under their
/// group headings, then each pane's own keys under the pane's heading.
fn help_rows(app: &App) -> Vec<HelpRow> {
    let bindings = app.keymap().bindings();
    let mut rows = Vec::new();
    for group in Group::ALL {
        let global: Vec<&RuntimeBinding> = bindings
            .iter()
            .filter(|binding| binding.group == *group && binding.contexts.is_empty())
            .collect();
        if global.is_empty() {
            continue;
        }
        rows.push(HelpRow::Heading(group.heading().to_owned()));
        rows.extend(global.into_iter().map(|binding| key_row(binding, app)));
    }
    for pane in PANES {
        let scoped: Vec<&RuntimeBinding> = Group::ALL
            .iter()
            .flat_map(|group| {
                bindings.iter().filter(move |binding| {
                    binding.group == *group && binding.contexts.contains(pane)
                })
            })
            .collect();
        if scoped.is_empty() {
            continue;
        }
        rows.push(HelpRow::Heading(pane_title(*pane).to_owned()));
        rows.extend(scoped.into_iter().map(|binding| key_row(binding, app)));
    }
    rows
}

/// The tip's rows: the leaders, with the keys they open on now, and the keys
/// that mean something only where the cursor is.
fn tip_rows(app: &App) -> Vec<(String, String)> {
    let context = app.context();
    let mut rows: Vec<(String, String)> = Leader::ALL
        .iter()
        .map(|leader| (leader_label(app, *leader), format!("{} …", leader.title())))
        .collect();
    rows.extend(
        app.keymap()
            .bindings()
            .iter()
            .filter(|binding| binding.leader.is_none() && binding.contexts.contains(&context))
            .map(|binding| (binding.keys_label.clone(), binding.what.to_owned())),
    );
    rows.push(("? ?".to_owned(), "all keys".to_owned()));
    rows
}

#[must_use]
pub fn tip_size(app: &App) -> (u16, u16) {
    let rows = tip_rows(app);
    let keys = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    let what = rows
        .iter()
        .map(|(_, w)| w.chars().count())
        .max()
        .unwrap_or(0);
    let inner = (keys + HELP_GAP + what).max(tip_title(app).chars().count());
    (
        u16::try_from(rows.len())
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
    let rows = tip_rows(app);
    let keys = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS)).saturating_sub(1);
    let lines: Vec<Line<'static>> = rows
        .iter()
        .map(|(key, what)| {
            clip_spans(
                vec![
                    Span::raw(" "),
                    Span::styled(
                        format!("{key:<keys$}"),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" ".repeat(HELP_GAP)),
                    Span::raw(what.clone()),
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
    let all = help_rows(app);
    if height == 0 || width == 0 {
        return Text::default();
    }
    // A keymap that does not fit gives its last row to the scroll hint
    // rather than drawing a row and then writing over it.
    let mut flowed = flow(&all, height, width, HELP_GAP, app.help_scroll());
    if flowed.truncated && height > 1 {
        flowed = flow(&all, height - 1, width, HELP_GAP, app.help_scroll());
    }
    let lines = (0..height)
        .map(|row| {
            let mut spans = Vec::with_capacity(flowed.columns.len() * 3);
            for (index, column) in flowed.columns.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw(" ".repeat(HELP_GAP)));
                }
                spans.extend(help_cell(
                    column.rows.get(row).copied().flatten(),
                    column.keys,
                    column.width(HELP_GAP),
                ));
            }
            clip_spans(spans, width)
        })
        .collect::<Vec<_>>();
    let mut text = Text::from(lines);
    if flowed.truncated
        && let Some(last) = text.lines.last_mut()
    {
        *last = Line::styled(MORE, Style::default().add_modifier(Modifier::DIM));
    }
    text
}

fn help_cell(row: Option<&HelpRow>, keys_w: usize, column: usize) -> Vec<Span<'static>> {
    let what_w = column.saturating_sub(keys_w + HELP_GAP);
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
