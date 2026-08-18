//! A comment drawn beneath the diff line it is anchored to.
//!
//! The box is made of box-drawing characters *inside* the pane's own `Text`
//! rather than as a nested widget: a ratatui `Block` cannot nest inside a
//! `Paragraph`, and hand-drawn borders keep the pane a pure `state → Text`
//! function that a `TestBackend` can assert on cell by cell.
//!
//! Which rows a box occupies is [`crate::rows`]'s answer, not this module's: a
//! box is several rows tall, so "the third diff line" stops being "the third
//! row on screen" the moment a comment exists.

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::SettledBy;

use super::BOX_PADDING;
use super::GUTTER;
use super::text::clip;
use super::text::clip_spans;
use crate::app::App;
use crate::rows::BodyKind;

/// A box's top: its heading, then a rule out to the right-hand corner.
pub(super) fn box_top(app: &App, comment: &Comment, width: usize) -> Line<'static> {
    let style = box_style(app, comment);
    let heading = format!("─ {} ", label(comment));
    let rule = "─".repeat(box_width(width).saturating_sub(2 + heading.chars().count()));
    clip_spans(
        vec![Span::styled(
            format!("{}╭{heading}{rule}╮", indent(width)),
            style,
        )],
        width,
    )
}

/// One row of a box's text, padded out to its right-hand border.
pub(super) fn box_body(
    app: &App,
    comment: &Comment,
    text: &str,
    kind: BodyKind,
    width: usize,
) -> Line<'static> {
    let style = box_style(app, comment);
    let pad = box_width(width).saturating_sub(BOX_PADDING + text.chars().count());
    clip_spans(
        vec![
            Span::styled(format!("{}│ ", indent(width)), style),
            // The body keeps the terminal's own foreground: it is the part
            // being *read*, and the border already says whose it is.
            Span::styled(text.to_owned(), body_style(kind)),
            Span::styled(format!("{} │", " ".repeat(pad)), style),
        ],
        width,
    )
}

/// A box's bottom rule.
pub(super) fn box_bottom(app: &App, comment: &Comment, width: usize) -> Line<'static> {
    let style = box_style(app, comment);
    let rule = "─".repeat(box_width(width).saturating_sub(2));
    clip_spans(
        vec![Span::styled(format!("{}╰{rule}╯", indent(width)), style)],
        width,
    )
}

/// A folded box: one row, its label and the first line of its body.
pub(super) fn box_collapsed(app: &App, comment: &Comment, width: usize) -> Line<'static> {
    let style = box_style(app, comment);
    let first = comment.body.lines().next().unwrap_or_default();
    let text = format!(
        "{}▸ {}{} — {first}",
        indent(width),
        state_mark(comment.state),
        label(comment)
    );
    Line::styled(clip(&text, width), style)
}

/// A box's title: the id it is filed under, the state it is in, and — where it
/// was settled — who settled it, so the box on screen and the entry in the
/// store name each other.
///
/// The actor is printed rather than implied. An agent may resolve its own
/// finding, and the one thing a reviewer must be able to see is that it did.
fn label(comment: &Comment) -> String {
    let state = state_name(comment.state);
    match comment.settled_by {
        Some(SettledBy::Agent) => format!("{} · {state} by agent", comment.id),
        Some(SettledBy::User) | None => format!("{} · {state}", comment.id),
    }
}

/// A comment state's name, spelled the way the store serializes it.
pub(super) fn state_name(state: CommentState) -> &'static str {
    match state {
        CommentState::Open => "open",
        CommentState::AwaitingVerification => "awaiting-verification",
        CommentState::Resolved => "resolved",
        CommentState::Abandoned => "abandoned",
        CommentState::Outdated => "outdated",
    }
}

/// The mark a settled state carries beside its name: a tick for work that
/// happened, nothing for work that did not.
///
/// Abandoned is marked by the strikethrough in [`comment_style`] instead —
/// crossing a remark out is what dropping it unfixed looks like, and a second
/// glyph would say the same thing twice.
pub(super) fn state_mark(state: CommentState) -> &'static str {
    match state {
        CommentState::Resolved => "✓ ",
        _ => "",
    }
}

/// Blue while a comment is open, grey and dim once it is neither, and struck
/// through where it was abandoned.
///
/// Grey is not a second meaning for blue: a settled or outdated comment is
/// still a comment, just not one asking for an answer, and drawing it as loudly
/// as one that is would bury the review under its own history.
///
/// The strikethrough separates the two settled states without a second colour.
/// *Fixed* and *dropped unfixed* are different facts about a review, and a
/// reader who cannot tell them apart is reading a summary that lies about what
/// the review concluded.
pub(super) fn comment_style(comment: &Comment) -> Style {
    match comment.state {
        CommentState::Open => Style::default().fg(Color::Blue),
        CommentState::Abandoned => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::DIM | Modifier::CROSSED_OUT),
        _ => Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
    }
}

/// The reviewer's own words at full contrast, an answer folded in from the
/// export dimmed.
///
/// Dim rather than a colour, for the same reason focus is not a colour: blue
/// means *comment* here and a second hue would be a second meaning for it. What
/// a reply needs is to be *quieter* than the remark it answers.
fn body_style(kind: BodyKind) -> Style {
    match kind {
        BodyKind::Body => Style::default(),
        BodyKind::Reply => Style::default().add_modifier(Modifier::DIM),
    }
}

/// The same, plus the selection: the box the stack cursor is on is brighter and
/// bold, so `d` and `s` visibly have a target.
fn box_style(app: &App, comment: &Comment) -> Style {
    let selected = app
        .selected_comment()
        .is_some_and(|cursor| cursor.id == comment.id);
    if selected {
        Style::default()
            .fg(Color::LightBlue)
            .add_modifier(Modifier::BOLD)
    } else {
        comment_style(comment)
    }
}

/// The blank left of a box, so it hangs off its line's text rather than off the
/// pane's edge. Never wider than the pane.
fn indent(width: usize) -> String {
    " ".repeat(GUTTER.min(width))
}

/// How many columns a box has to draw itself in.
fn box_width(width: usize) -> usize {
    width.saturating_sub(GUTTER)
}
