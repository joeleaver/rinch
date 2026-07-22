//! The caret blink clock.
//!
//! The desktop runtime is event-driven (winit `ControlFlow::Wait`): it only wakes
//! for input, so there is no free-running animation tick. To blink the focused
//! editor's caret, the runtime calls [`caret_blink_tick`](crate::caret_blink_tick)
//! every iteration, which consults this clock to decide the current phase and how
//! long until the next toggle — the runtime then arms `ControlFlow::WaitUntil` for
//! exactly that long. When no caret is blinking the clock reports nothing and the
//! loop returns to `Wait` (zero idle CPU).
//!
//! The phase is anchored at the last [`reset`] (a caret move or edit), so the
//! caret is always solid immediately after an interaction — standard text-editor
//! behaviour — then alternates every [`BLINK_INTERVAL`].

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Caret blink half-period (solid for this long, then hidden for this long).
/// 530 ms is the long-standing platform default (Win32 `GetCaretBlinkTime`).
pub(crate) const BLINK_INTERVAL: Duration = Duration::from_millis(530);

thread_local! {
    /// The instant the caret phase last reset to "solid". `None` until the first
    /// [`tick`] anchors it.
    static ANCHOR: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The `(doc_key, container id)` of the editor currently being blinked, so a
    /// focus change can restore the previously-blinked caret to solid (never
    /// leave a blurred editor frozen mid-blink with a hidden caret). Doc-scoped:
    /// container ids collide across documents on one thread (issue #134).
    static TARGET: Cell<Option<(u64, usize)>> = const { Cell::new(None) };
}

/// Reset the blink phase to "solid". Called whenever the caret moves or the
/// document is edited so the caret is solid right after the interaction.
pub(crate) fn reset() {
    ANCHOR.with(|a| a.set(Some(Instant::now())));
}

/// The `(doc_key, container id)` currently being blinked, if any.
pub(crate) fn target() -> Option<(u64, usize)> {
    TARGET.with(|t| t.get())
}

/// Record which editor is being blinked (`None` = none).
pub(crate) fn set_target(key: Option<(u64, usize)>) {
    TARGET.with(|t| t.set(key));
}

/// The current blink phase and the time until the next toggle. Lazily anchors
/// the phase to "now" on first call after a [`reset`]/cold start.
pub(crate) fn tick() -> (bool, Duration) {
    let now = Instant::now();
    let anchor = ANCHOR.with(|a| {
        let cur = a.get().unwrap_or(now);
        a.set(Some(cur));
        cur
    });
    phase_at(anchor, now)
}

/// Pure phase computation (factored out so it is unit-testable without a clock):
/// given the phase `anchor` and the current instant `now`, return whether the
/// caret should currently be visible and the [`Duration`] until the next toggle.
fn phase_at(anchor: Instant, now: Instant) -> (bool, Duration) {
    let elapsed = now.saturating_duration_since(anchor).as_millis();
    let period = BLINK_INTERVAL.as_millis();
    // Even half-periods are visible, odd are hidden.
    let visible = (elapsed / period).is_multiple_of(2);
    let into_phase = elapsed % period;
    let remaining = (period - into_phase) as u64;
    (visible, Duration::from_millis(remaining))
}

/// The current phase anchor — test-only, so view tests can assert which editor's
/// caret move actually re-anchored the (global) blink clock.
#[cfg(test)]
pub(crate) fn anchor_for_test() -> Option<Instant> {
    ANCHOR.with(|a| a.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_is_solid_at_anchor_and_alternates_each_half_period() {
        let a = Instant::now();
        // At the anchor: visible, a full period until the next toggle.
        let (v0, n0) = phase_at(a, a);
        assert!(v0);
        assert_eq!(n0, BLINK_INTERVAL);
        // Just before the first boundary: still visible, a sliver remaining.
        let (v1, n1) = phase_at(a, a + Duration::from_millis(529));
        assert!(v1);
        assert_eq!(n1, Duration::from_millis(1));
        // At the first boundary: hidden, a fresh full period remaining.
        let (v2, n2) = phase_at(a, a + Duration::from_millis(530));
        assert!(!v2);
        assert_eq!(n2, BLINK_INTERVAL);
        // Mid second half-period: hidden.
        let (v3, _) = phase_at(a, a + Duration::from_millis(800));
        assert!(!v3);
        // Second boundary: visible again.
        let (v4, _) = phase_at(a, a + Duration::from_millis(1060));
        assert!(v4);
    }

    #[test]
    fn now_before_anchor_is_clamped_to_solid() {
        // saturating_duration_since guards a non-monotonic / pre-anchor `now`.
        let a = Instant::now();
        let earlier = a;
        let (v, n) = phase_at(a, earlier);
        assert!(v);
        assert_eq!(n, BLINK_INTERVAL);
    }
}
