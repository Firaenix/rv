//! The border a pane draws around itself, and how a list marks its selection.

use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;

use super::text::colour;
use crate::gradient;

/// A pane's block: rounded, titled, and marked when it holds the focus.
///
/// The mark is **three signals for one fact**: a `▸` on the title, a bold
/// border, and the border in [`gradient::FOCUS`] — the magenta this interface
/// spends on nothing else, because green is an addition, red a removal, blue a
/// comment and orange an alert.
///
/// The `▸` is redundant on purpose and stays: a sixteen-colour terminal renders
/// the magenta as whatever it likes or not at all, and a reader who does not
/// separate magenta from red gets nothing from the hue. Colour *enhances* the
/// signal here and is never the only carrier of it.
pub(super) fn pane(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().border_type(BorderType::Rounded);
    if focused {
        block.title(format!("▸ {title}")).border_style(
            Style::default()
                .fg(colour(gradient::FOCUS))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        block.title(title)
    }
}

/// How a list marks its selected row: reversed while the list has the focus,
/// and a dim underline while it does not — so there is exactly one place on
/// screen the next keystroke will land.
pub(super) fn selection_style(focused: bool) -> Style {
    if focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().add_modifier(Modifier::DIM | Modifier::UNDERLINED)
    }
}
