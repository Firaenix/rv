//! Something went wrong, floating over the panes until it ages out.

use std::time::Duration;
use std::time::Instant;

use super::App;

/// How long an alert stays on screen (spec §9).
const ALERT_LIFETIME: Duration = Duration::from_secs(5);

/// How much of that life is spent fading out.
const ALERT_FADE: Duration = Duration::from_secs(1);

/// How many steps the fade takes. A terminal cannot alpha-blend, so the fade is
/// a ramp in Oklab lightness; fewer steps than this reads as a flicker.
const ALERT_FADE_STEPS: u32 = 4;

/// A **status** describes state and lives in the bar; an **alert** is something
/// that went wrong and needs noticing.
///
/// `raised` is an [`Option`] because nothing inside [`App`] calls
/// [`Instant::now`]: the places that know something went wrong have no clock in
/// reach, so they raise unstamped and [`App::expire_alerts`] stamps it on the
/// loop's next pass. An unstamped alert is live, drawn at full strength, and
/// asks the loop straight back for its stamp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    /// What went wrong, as one sentence.
    pub message: String,
    /// When the loop first saw it, or [`None`] until it has.
    pub raised: Option<Instant>,
}

impl Alert {
    /// Whether this alert is still worth drawing at `now`.
    #[must_use]
    pub fn live(&self, now: Instant) -> bool {
        self.age(now) < ALERT_LIFETIME
    }

    /// How far through its fade this alert is at `now`, from `0.0` to `1.0`.
    ///
    /// Stepped rather than continuous because the deadlines
    /// [`App::next_deadline`] hands the loop *are* the steps: a continuous ramp
    /// would mean a wake-up per frame, or a fade that only advances when
    /// something else redraws.
    #[must_use]
    pub fn fade(&self, now: Instant) -> f32 {
        let age = self.age(now);
        let Some(into) = age.checked_sub(ALERT_LIFETIME - ALERT_FADE) else {
            return 0.0;
        };
        let steps = f64::from(ALERT_FADE_STEPS);
        let step = (into.as_secs_f64() / ALERT_FADE.as_secs_f64() * steps).floor();
        (step.clamp(0.0, steps) / steps) as f32
    }

    /// How long this alert has been up at `now`, or nothing while unstamped.
    ///
    /// Saturating: a `now` before the stamp means "no time has passed" rather
    /// than a panic.
    fn age(&self, now: Instant) -> Duration {
        self.raised
            .map(|raised| now.saturating_duration_since(raised))
            .unwrap_or_default()
    }

    /// How long until this alert next changes what is on screen.
    ///
    /// [`Duration::ZERO`] while unstamped, so the loop comes back at once to
    /// stamp it rather than blocking on a key with an unaged toast up.
    fn next_change(&self, now: Instant) -> Duration {
        if self.raised.is_none() {
            return Duration::ZERO;
        }
        let age = self.age(now);
        (0..=ALERT_FADE_STEPS)
            .map(|step| ALERT_LIFETIME - ALERT_FADE + ALERT_FADE * step / ALERT_FADE_STEPS)
            .find(|deadline| *deadline > age)
            .map_or(Duration::ZERO, |deadline| deadline - age)
    }
}

impl App {
    /// Raises an alert, stamped with the time the caller has.
    ///
    /// For callers that know what time it is — the event loop, and every test.
    /// Everything inside this module raises through [`App::raise`] instead,
    /// because a key press has no clock in reach and must not grow one.
    pub fn alert(&mut self, message: impl Into<String>, now: Instant) {
        self.push_alert(message.into(), Some(now));
    }

    /// The same, from a place with no clock.
    pub(super) fn raise(&mut self, message: impl Into<String>) {
        self.push_alert(message.into(), None);
    }

    /// Puts `message` up, unless it is already up: the same failure raised
    /// twice is one thing that went wrong, and `x · x` says nothing the first
    /// `x` did not.
    fn push_alert(&mut self, message: String, raised: Option<Instant>) {
        if self.alerts.iter().any(|alert| alert.message == message) {
            return;
        }
        self.alerts.push(Alert { message, raised });
    }

    /// Stamps whatever is unstamped and drops whatever has aged out.
    ///
    /// Stamping *before* the sweep is what keeps an alert raised this pass from
    /// being expired in the same breath.
    pub fn expire_alerts(&mut self, now: Instant) {
        for alert in &mut self.alerts {
            alert.raised.get_or_insert(now);
        }
        self.alerts.retain(|alert| alert.live(now));
    }

    /// What has gone wrong lately, oldest first.
    pub fn alerts(&self) -> &[Alert] {
        &self.alerts
    }

    /// How long the event loop may block for, or [`None`] when nothing on
    /// screen ages and it may wait for a key forever.
    ///
    /// This is the whole of what makes "it leaves on its own" true: without it
    /// a toast in front of a reviewer who walked away stays until they return.
    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        self.alerts.iter().map(|alert| alert.next_change(now)).min()
    }
}
