//! The chrome's colours: ANSI palette indices, resolved by the terminal.
//!
//! Nothing here is an RGB value on purpose. An indexed colour is looked up in
//! the terminal's own palette **at render time**, so the interface follows the
//! user's theme — including a theme changed while rv is running — the same way
//! helix and zellij do, and rv never needs a theme option of its own. The same
//! ruling spec §6 makes for syntax colours, extended to the chrome.
//!
//! One meaning per hue, as ever: magenta is the *focus*, blue a *comment*,
//! cyan a *commit hash*, yellow an *alert*, and nothing may quietly claim a
//! second meaning.
//!
//! The exception is additions and removals. Green and red carry a *proportion*
//! — the gradient across a sidebar row, the wash under a diff line — and a
//! proportion is arithmetic between two endpoints, which an index cannot do.
//! Those two stay [`crate::gradient`]'s RGB values, blended there.

use ratatui::style::Color;

/// The focused pane, and the selectable prefix of a change id.
pub const FOCUS: Color = Color::Magenta;

/// A comment, everywhere one appears.
pub const COMMENT: Color = Color::Blue;

/// The selectable prefix of a commit hash.
pub const HASH: Color = Color::Cyan;

/// Something that wants attention: a stale anchor, a failed write, a question.
pub const ALERT: Color = Color::Yellow;
