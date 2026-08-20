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
use rv_core::diff::LineKind;
use rv_core::model::Confidence;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::SettledBy;

use super::BOX_PADDING;
use super::GUTTER;
use super::text::clip;
use super::text::clip_spans;
use crate::app::App;
use crate::gradient;
use crate::rows::BodyKind;
use crate::theme;

/// Columns a before/after row spends on its sigil and the space after it.
const SIGIL: usize = 2;

/// What the before/after block's divider says: which half of it is which.
const WHEN_WRITTEN: &str = "when this was written";
const NOW: &str = "now";

/// A box's top: its heading, then a rule out to the right-hand corner.
pub(super) fn box_top(app: &App, comment: &Comment, width: usize) -> Line<'static> {
    let style = box_style(app, comment);
    let heading = format!("─ {} ", label(comment));
    let tag = drift_tag(app.confidence(comment));
    let tagged = tag.map_or(0, |(text, _)| text.chars().count());
    let rule = "─".repeat(box_width(width).saturating_sub(2 + tagged + heading.chars().count()));
    let mut spans = vec![Span::styled(format!("{}╭{heading}", indent(width)), style)];
    if let Some((text, tag_style)) = tag {
        spans.push(Span::styled(text, tag_style));
    }
    spans.push(Span::styled(format!("{rule}╮"), style));
    clip_spans(spans, width)
}

/// How confidently the comment's anchor was placed, where that is worth a
/// reviewer's attention — and nothing at all where it is not.
///
/// [`Confidence::Exact`] is the common case, so naming it would put a word on
/// every box in the review to report that nothing had happened. Only the two
/// tiers that mean the comment may have drifted are named.
///
/// `Weak` is the one that matters, and it is drawn in [`theme::ALERT`] — the
/// colour this interface already spends on a stale anchor. The commented
/// *content* is gone and only the line number stands, so the box points at line
/// 48 with nothing guaranteeing line 48 is what the remark was about. Acting on
/// that as though it were an exact hit is the failure this tag exists to
/// prevent, and a reviewer must not have to compare two boxes to see it.
///
/// `Moved` found its content, just somewhere else; worth saying, not worth
/// shouting, so it stays grey.
///
/// [`Confidence::Outdated`] is unnamed here because the heading already says
/// `outdated`, and the before/after block below it is the long form of the same
/// fact.
fn drift_tag(confidence: Confidence) -> Option<(&'static str, Style)> {
    match confidence {
        Confidence::Moved => Some(("· moved ", Style::default().fg(Color::Gray))),
        Confidence::Weak => Some((
            "· weak anchor ",
            Style::default()
                .fg(theme::ALERT)
                .add_modifier(Modifier::BOLD),
        )),
        Confidence::Exact | Confidence::Outdated => None,
    }
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

/// The before/after block's divider: a `├`/`┤` rule naming its two halves, so
/// the lines under it read as *then* against *now* rather than as an ordinary
/// diff that happens to be indoors.
pub(super) fn box_rule(app: &App, comment: &Comment, width: usize) -> Line<'static> {
    let style = box_style(app, comment);
    let heading = format!("─ {WHEN_WRITTEN} ──── {NOW} ");
    let rule = "─".repeat(box_width(width).saturating_sub(2 + heading.chars().count()));
    clip_spans(
        vec![Span::styled(
            format!("{}├{heading}{rule}┤", indent(width)),
            style,
        )],
        width,
    )
}

/// One line of the before/after block: the sigil for its side, then the text,
/// inside the box's own border.
///
/// The sigil rather than a wash, which is what the diff pane uses: a wash is a
/// background across the whole row, and a row that is four columns of border
/// around a band of colour reads as a second pane rather than as part of a
/// comment. The hue is [`gradient`]'s own, so this block and the pane above it
/// cannot end up with two greens.
pub(super) fn box_diff(
    app: &App,
    comment: &Comment,
    text: &str,
    kind: LineKind,
    width: usize,
) -> Line<'static> {
    let style = box_style(app, comment);
    let room = box_width(width).saturating_sub(BOX_PADDING + SIGIL);
    let text = clip(text, room);
    let pad = room.saturating_sub(text.chars().count());
    clip_spans(
        vec![
            Span::styled(format!("{}│ ", indent(width)), style),
            Span::styled(format!("{} {text}", sigil(kind)), diff_style(kind)),
            Span::styled(format!("{} │", " ".repeat(pad)), style),
        ],
        width,
    )
}

/// What marks which side of the before/after a line came from.
fn sigil(kind: LineKind) -> char {
    match kind {
        LineKind::Added => '+',
        LineKind::Removed => '-',
        LineKind::Context => ' ',
    }
}

/// Green for what is there now, red for what the comment was written against,
/// and the terminal's own foreground for the lines both versions share.
fn diff_style(kind: LineKind) -> Style {
    match kind {
        LineKind::Added => Style::default().fg(super::text::colour(gradient::ADDED)),
        LineKind::Removed => Style::default().fg(super::text::colour(gradient::REMOVED)),
        LineKind::Context => Style::default().add_modifier(Modifier::DIM),
    }
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
/// export dimmed, and the box's own remark about itself dimmed and italic.
///
/// Dim rather than a colour, for the same reason focus is not a colour: blue
/// means *comment* here and a second hue would be a second meaning for it. What
/// a reply needs is to be *quieter* than the remark it answers.
///
/// A note is quieter still *and* slanted, because it is the only text in a box
/// nobody wrote: "the anchor could not be located" is the tool speaking, and a
/// reviewer must not read it as part of the comment.
fn body_style(kind: BodyKind) -> Style {
    match kind {
        BodyKind::Body => Style::default(),
        BodyKind::Reply => Style::default().add_modifier(Modifier::DIM),
        BodyKind::Note => Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC),
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
