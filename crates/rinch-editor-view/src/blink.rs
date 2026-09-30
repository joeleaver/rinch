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
//!
//! There is one clock **per document** (keyed by
//! [`DomDocument::doc_key`](rinch_core::dom::DomDocument::doc_key)), not one per
//! thread: a desktop window and its DevTools panel, or several embedded
//! `RinchContext`s, share a thread and each has a keyboard of its own. A single
//! thread-wide clock was retargeted by every document that ticked it, so two
//! documents that each had a focused editor restored each other's caret to
//! solid on every tick and neither blinked (issue #1149, the #134 rule).

use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Caret blink half-period (solid for this long, then hidden for this long).
/// 530 ms is the long-standing platform default (Win32 `GetCaretBlinkTime`).
pub(crate) const BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// One document's blink clock.
struct Clock {
    doc_key: u64,
    /// The container id of the editor being blinked in this document, so a
    /// focus change can restore the previously-blinked caret to solid (never
    /// leave a blurred editor frozen mid-blink with a hidden caret).
    target: usize,
    /// The instant the caret phase last reset to "solid". `None` until the
    /// first [`tick`] anchors it.
    anchor: Option<Instant>,
}

thread_local! {
    /// One entry per document that is blinking a caret. A document with no
    /// blink target has no entry, so the list is as long as the number of
    /// documents on the thread with a focused editor — one or two in practice,
    /// which is why it is a `Vec` and not a map.
    static CLOCKS: RefCell<Vec<Clock>> = const { RefCell::new(Vec::new()) };
}

fn with_clock<R>(doc_key: u64, f: impl FnOnce(Option<&mut Clock>) -> R) -> R {
    CLOCKS.with(|c| f(c.borrow_mut().iter_mut().find(|c| c.doc_key == doc_key)))
}

/// Reset `doc_key`'s blink phase to "solid". Called whenever its blinking
/// caret moves or its document is edited, so the caret is solid right after
/// the interaction. A document with no blink target has no phase to reset.
pub(crate) fn reset(doc_key: u64) {
    with_clock(doc_key, |c| {
        if let Some(c) = c {
            c.anchor = Some(Instant::now());
        }
    });
}

/// The container id of the editor `doc_key` is blinking, if any.
pub(crate) fn target(doc_key: u64) -> Option<usize> {
    with_clock(doc_key, |c| c.map(|c| c.target))
}

/// Record which editor `doc_key` is blinking (`None` = none), with its phase
/// restarted at "solid".
pub(crate) fn set_target(doc_key: u64, target: Option<usize>) {
    CLOCKS.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|c| c.doc_key != doc_key);
        if let Some(target) = target {
            c.push(Clock {
                doc_key,
                target,
                anchor: Some(Instant::now()),
            });
        }
    });
}

/// Drop `doc_key`'s clock if it is blinking `container_id` — the editor
/// unmounted, so nothing is left to blink or restore.
pub(crate) fn forget(doc_key: u64, container_id: usize) {
    CLOCKS.with(|c| {
        c.borrow_mut()
            .retain(|c| !(c.doc_key == doc_key && c.target == container_id));
    });
}

/// `doc_key`'s current blink phase and the time until its next toggle. Lazily
/// anchors the phase to "now" if it has none yet. A document with no blink
/// target is at the start of a phase.
pub(crate) fn tick(doc_key: u64) -> (bool, Duration) {
    let now = Instant::now();
    let anchor = with_clock(doc_key, |c| match c {
        Some(c) => *c.anchor.get_or_insert(now),
        None => now,
    });
    phase_at(anchor, now)
}

/// How many documents hold a clock (see [`crate::blink_clock_count`]).
pub(crate) fn clock_count() -> usize {
    CLOCKS.with(|c| c.borrow().len())
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

/// `doc_key`'s phase anchor — test-only, so view tests can assert which
/// editor's caret move actually re-anchored which document's blink clock.
#[cfg(test)]
pub(crate) fn anchor_for_test(doc_key: u64) -> Option<Instant> {
    with_clock(doc_key, |c| c.and_then(|c| c.anchor))
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

    /// Two documents keep two clocks (#1149): setting, resetting or forgetting
    /// one document's target leaves the other's alone, even when both name the
    /// same container id (ids collide across documents, #134).
    #[test]
    fn each_document_keeps_its_own_clock() {
        set_target(101, Some(7));
        let a0 = anchor_for_test(101).expect("a target anchors its phase");
        std::thread::sleep(Duration::from_millis(2));
        set_target(102, Some(7));
        assert_eq!(target(101), Some(7), "b's target displaced a's");
        assert_eq!(target(102), Some(7));
        assert_eq!(anchor_for_test(101), Some(a0), "b's target reset a's phase");

        reset(102);
        assert_eq!(
            anchor_for_test(101),
            Some(a0),
            "b's reset reached a's phase"
        );

        set_target(102, None);
        assert_eq!(target(101), Some(7), "clearing b cleared a");
        assert_eq!(target(102), None);

        forget(101, 8);
        assert_eq!(
            target(101),
            Some(7),
            "forgot an editor that is not the target"
        );
        forget(101, 7);
        assert_eq!(target(101), None);
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
