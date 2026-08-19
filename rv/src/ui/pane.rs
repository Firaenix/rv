//! The border a pane draws around itself, and how a list marks its selection.

use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;

use crate::theme;

/// A pane's block: rounded, titled, and marked when it holds the focus.
///
/// The mark is **three signals for one fact**: a `▸` on the title, a bold
/// border, and the border in [`theme::FOCUS`] — the magenta this interface
/// spends on nothing else, because green is an addition, red a removal, blue a
/// comment and yellow an alert.
///
/// The `▸` is redundant on purpose and stays: whatever the theme renders
/// magenta as, a reader who does not separate it from red gets nothing from the
/// hue. Colour *enhances* the signal here and is never the only carrier of it.
pub(super) fn pane(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().border_type(BorderType::Rounded);
    if focused {
        block.title(format!("▸ {title}")).border_style(
            Style::default()
                .fg(theme::FOCUS)
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
