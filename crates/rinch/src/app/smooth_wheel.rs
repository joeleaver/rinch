//! Smooth wheel scrolling (#2287): a wheel notch glides to where it scrolls
//! to, over a fraction of a second, the way Chrome's does, instead of jumping
//! there in one frame.
//!
//! A *line* wheel (winit's `MouseScrollDelta::LineDelta`, a mouse wheel's
//! notch, reported as [`PlatformEvent::MouseWheelLines`]) animates; a *pixel*
//! wheel (`PixelDelta`: a trackpad, a high-resolution wheel, Android's fling)
//! stays direct, as it is in Chrome, because the device already reports a
//! stream of small moves.
//!
//! # The curve is Chromium's
//!
//! [`ScrollCurve`] is a port of `cc::ScrollOffsetAnimationCurve`
//! (`cc/animation/scroll_offset_animation_curve.cc`), the curve Chromium runs
//! for an animated wheel scroll (`ScrollType::kMouseWheel` →
//! `DurationBehavior::kInverseDelta`, eased by `EaseInOutWithInitialSlope`):
//!
//! - **The ease** is `cubic-bezier(0.42, y1, 0.58, 1)`, CSS's `ease-in-out`
//!   with its first control point's `y` scaled to start at the curve's current
//!   velocity: `y1 = 0.42 × slope`. From rest, `slope = 0` and it is plain
//!   `ease-in-out`.
//! - **The duration** falls as the distance grows: 12 frames (of 60 Hz, so
//!   200 ms) up to 120 px, 6 frames (100 ms) from 480 px, linear between
//!   (`kInverseDeltaRampStartPx` / `kInverseDeltaRampEndPx`,
//!   `kInverseDeltaMinDuration` / `kInverseDeltaMaxDuration`,
//!   `kDurationDivisor = 60`).
//! - **A new notch retargets** the running curve (`UpdateTarget`): it starts a
//!   new segment from where the scroll is now, to the new target, with an
//!   initial slope that keeps the velocity it has, and a duration that is the
//!   smaller of the distance's own and `2.5 ×` the time the remaining distance
//!   would take at the present velocity (`VelocityBasedDurationBound`).
//!
//! Measured in Chromium 153 on Linux (Playwright's build, a non-precise wheel
//! synthesised with `chrome.gpuBenchmarking.smoothScrollByXY` and
//! `MOUSE_INPUT`; `scrollTop` sampled every rAF): a 40 px notch fits
//! `ease-in-out` over 172 ms, 200 px over 154 ms and 600 px over 78 ms, each
//! about 25 ms under the source's 200 / 178 / 100 ms because Chromium takes
//! the time from the input event to the first animation frame off the curve
//! (`SegmentDuration`'s `delayed_by`). rinch starts the curve when it handles
//! the notch, which is that event, so it uses the source's durations as they
//! are. Chrome's samples for the 40 px notch, one per 16.7 ms frame:
//! 1, 3, 7, 13, 19, 26, 31, 36, 39, 40.
//!
//! # Through the existing scroll path
//!
//! Each frame of the animation is one call of
//! [`RinchApp::scroll_vertically_by`] (or its horizontal twin): the scroll
//! write a wheel notch made before, with its device-pixel snapping, its
//! `mark_scrolled` damage and its `onscroll`. No layout and no restyle.
//!
//! # What ends one early
//!
//! The scroll offset is compared, every frame, with what the animation last
//! wrote; anything else that moved it — a `scroll_top` write, a scrollbar
//! drag, the editor revealing its caret, a keyboard scroll, a trackpad —
//! wins, and the animation stops where that put it. A scrollbar press on the
//! animating scroller stops it too, before the thumb moves.

use super::RinchApp;
use rinch_core::dom::DomDocument as _;
use rinch_dom::RinchDocument;

/// Whether a `RinchApp` created from now on, on this thread, animates wheel
/// notches ([`App::smooth_wheel_scroll`](crate::App::smooth_wheel_scroll)
/// calls this before the window's app is created).
pub fn set_smooth_wheel_scroll_default(enabled: bool) {
    SMOOTH_OFF_BY_DEFAULT.set(!enabled);
}

thread_local! {
    static SMOOTH_OFF_BY_DEFAULT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Which offset an animation moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WheelAxis {
    Vertical,
    Horizontal,
}

/// One scroller's animation on one axis.
#[derive(Debug, Clone)]
pub(crate) struct WheelAnimation {
    pub(crate) node: u64,
    pub(crate) axis: WheelAxis,
    pub(crate) curve: ScrollCurve,
    /// The offset this animation last wrote (or found, when it started). A
    /// different one at the next frame means something else scrolled it.
    pub(crate) written: f64,
}

/// The smooth-scroll state of one [`RinchApp`].
#[derive(Debug)]
pub(crate) struct SmoothWheelState {
    pub(crate) enabled: bool,
    pub(crate) running: Vec<WheelAnimation>,
    /// Set while a [`PlatformEvent::MouseWheelLines`](rinch_platform::PlatformEvent::MouseWheelLines)
    /// is being handled as the `MouseWheel` it stands for, so that arm knows
    /// its default scroll animates.
    pub(crate) line_wheel: bool,
    /// The glides' own time, in ms: it moves by the document clock's forward
    /// steps only (`glide_now`), so a wall clock set back mid-glide neither
    /// replays the curve backwards nor stalls it until the clock catches up.
    glide_ms: f64,
    /// The document clock's reading at the last `glide_now`.
    clock_seen: Option<f64>,
}

impl Default for SmoothWheelState {
    fn default() -> Self {
        Self {
            enabled: !SMOOTH_OFF_BY_DEFAULT.get(),
            running: Vec::new(),
            line_wheel: false,
            glide_ms: 0.0,
            clock_seen: None,
        }
    }
}

// --- Chromium's constants (cc/animation/scroll_offset_animation_curve.cc) ---

/// Durations are written in frames of this rate.
const DURATION_DIVISOR: f64 = 60.0;
const INVERSE_DELTA_RAMP_START_PX: f64 = 120.0;
const INVERSE_DELTA_RAMP_END_PX: f64 = 480.0;
const INVERSE_DELTA_MIN_DURATION: f64 = 6.0;
const INVERSE_DELTA_MAX_DURATION: f64 = 12.0;
const INVERSE_DELTA_SLOPE: f64 = (INVERSE_DELTA_MIN_DURATION - INVERSE_DELTA_MAX_DURATION)
    / (INVERSE_DELTA_RAMP_END_PX - INVERSE_DELTA_RAMP_START_PX);
const INVERSE_DELTA_OFFSET: f64 =
    INVERSE_DELTA_MAX_DURATION - INVERSE_DELTA_RAMP_START_PX * INVERSE_DELTA_SLOPE;
const EPSILON: f64 = 0.01;
/// `EaseInOutWithInitialSlope`'s first control point's `x`, and its clamp.
const EASE_X1: f64 = 0.42;
const EASE_X2: f64 = 0.58;
const MAX_SLOPE: f64 = 1000.0;

/// How long a wheel segment of `delta` px takes, in ms: `kInverseDelta`.
pub(crate) fn segment_duration_ms(delta: f64) -> f64 {
    let frames = (INVERSE_DELTA_OFFSET + delta.abs() * INVERSE_DELTA_SLOPE)
        .clamp(INVERSE_DELTA_MIN_DURATION, INVERSE_DELTA_MAX_DURATION);
    frames / DURATION_DIVISOR * 1000.0
}

/// `cubic-bezier(x1, y1, x2, 1)`, with the slope it needs for a retarget.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ease {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

impl Ease {
    /// `EaseInOutWithInitialSlope(slope)`.
    fn with_initial_slope(slope: f64) -> Self {
        let slope = slope.clamp(-MAX_SLOPE, MAX_SLOPE);
        Self {
            x1: EASE_X1,
            y1: slope * EASE_X1,
            x2: EASE_X2,
            y2: 1.0,
        }
    }

    fn bx(&self, t: f64) -> f64 {
        let u = 1.0 - t;
        3.0 * u * u * t * self.x1 + 3.0 * u * t * t * self.x2 + t * t * t
    }

    fn by(&self, t: f64) -> f64 {
        let u = 1.0 - t;
        3.0 * u * u * t * self.y1 + 3.0 * u * t * t * self.y2 + t * t * t
    }

    fn dbx(&self, t: f64) -> f64 {
        let u = 1.0 - t;
        3.0 * u * u * self.x1 + 6.0 * u * t * (self.x2 - self.x1) + 3.0 * t * t * (1.0 - self.x2)
    }

    fn dby(&self, t: f64) -> f64 {
        let u = 1.0 - t;
        3.0 * u * u * self.y1 + 6.0 * u * t * (self.y2 - self.y1) + 3.0 * t * t * (1.0 - self.y2)
    }

    /// The curve's parameter at `x`. `x(t)` is monotonic (both control
    /// points' `x` are in `[0, 1]`), so bisection always converges.
    fn solve(&self, x: f64) -> f64 {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        let mut t = x;
        for _ in 0..8 {
            let err = self.bx(t) - x;
            if err.abs() < 1e-9 {
                return t;
            }
            let d = self.dbx(t);
            if d.abs() < 1e-9 {
                break;
            }
            t -= err / d;
            if !(0.0..=1.0).contains(&t) {
                break;
            }
        }
        t = x;
        for _ in 0..60 {
            if self.bx(t) < x {
                lo = t;
            } else {
                hi = t;
            }
            t = (lo + hi) / 2.0;
        }
        t
    }

    fn value(&self, x: f64) -> f64 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        self.by(self.solve(x))
    }

    /// `dy/dx` at `x`: the normalized velocity.
    fn slope(&self, x: f64) -> f64 {
        let t = self.solve(x.clamp(0.0, 1.0));
        let dx = self.dbx(t);
        if dx.abs() < 1e-9 {
            // At an end where `x` stops moving (`x1 = 0` / `x2 = 1`), which
            // these control points never are.
            return 0.0;
        }
        self.dby(t) / dx
    }
}

/// One axis of Chromium's `ScrollOffsetAnimationCurve` for a wheel. Times are
/// in ms on the document's animation clock.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScrollCurve {
    initial: f64,
    target: f64,
    /// When the current segment started (`last_retarget_`).
    segment_start: f64,
    /// When the current segment ends (`total_animation_duration_`, absolute).
    end: f64,
    ease: Ease,
}

impl ScrollCurve {
    /// A curve from rest at `initial` to `target`, starting at `now`.
    pub(crate) fn new(initial: f64, target: f64, now: f64) -> Self {
        Self {
            initial,
            target,
            segment_start: now,
            end: now + segment_duration_ms(target - initial),
            ease: Ease::with_initial_slope(0.0),
        }
    }

    pub(crate) fn target(&self) -> f64 {
        self.target
    }

    /// The offset at `now`.
    pub(crate) fn value(&self, now: f64) -> f64 {
        let duration = self.end - self.segment_start;
        if now >= self.end || duration <= 0.0 {
            return self.target;
        }
        let x = ((now - self.segment_start) / duration).max(0.0);
        self.initial + (self.target - self.initial) * self.ease.value(x)
    }

    pub(crate) fn is_finished(&self, now: f64) -> bool {
        now >= self.end
    }

    /// `UpdateTarget`: aim at `new_target` from where the curve is at `now`,
    /// keeping its velocity.
    pub(crate) fn update_target(&mut self, now: f64, new_target: f64) {
        if (self.target - new_target).abs() < EPSILON {
            self.target = new_target;
            return;
        }
        let now = now.max(self.segment_start);
        let current = self.value(now);
        let old_delta = self.target - self.initial;
        let new_delta = new_target - current;
        let old_duration = self.end - self.segment_start;
        let old_velocity = if now >= self.end || old_duration <= 0.0 {
            0.0
        } else {
            self.ease.slope((now - self.segment_start) / old_duration)
        };
        let new_duration = segment_duration_ms(new_delta).min(velocity_based_duration_bound(
            old_delta,
            old_velocity,
            old_duration,
            new_delta,
        ));
        // `kEpsilon` is compared with the duration in *seconds* there.
        if new_duration < EPSILON * 1000.0 {
            self.initial = current;
            self.target = new_target;
            self.segment_start = now;
            self.end = now;
            return;
        }
        let new_velocity = if new_delta.abs() < EPSILON || old_duration <= 0.0 {
            0.0
        } else {
            old_velocity * (new_duration / old_duration) * (old_delta / new_delta)
        };
        self.initial = current;
        self.target = new_target;
        self.segment_start = now;
        self.end = now + new_duration;
        self.ease = Ease::with_initial_slope(new_velocity);
    }
}

/// `VelocityBasedDurationBound`, in ms.
fn velocity_based_duration_bound(
    old_delta: f64,
    old_normalized_velocity: f64,
    old_duration: f64,
    new_delta: f64,
) -> f64 {
    if new_delta.abs() < EPSILON {
        return 0.0;
    }
    if old_delta.abs() < EPSILON || old_normalized_velocity.abs() < EPSILON {
        return f64::INFINITY;
    }
    // px per ms.
    let old_true_velocity = old_normalized_velocity * old_delta / old_duration;
    let bound = (new_delta / old_true_velocity) * 2.5;
    if bound < 0.0 { f64::INFINITY } else { bound }
}

/// The scroll range of `id` on `axis`, `0..=max`, snapped as the scroll
/// writes are.
fn max_offset(doc: &RinchDocument, id: u64, axis: WheelAxis) -> f64 {
    let nid = rinch_core::dom::NodeId(id);
    let span = match axis {
        WheelAxis::Vertical => doc.scroll_height(nid) - doc.client_height(nid),
        WheelAxis::Horizontal => doc.scroll_width(nid) - doc.client_width(nid),
    };
    span.max(0.0)
}

/// `id`'s offset on `axis`, and the sub-device-pixel remainder kept with it.
fn precise_offset(doc: &RinchDocument, id: u64, axis: WheelAxis) -> Option<(f64, f64)> {
    let node = doc.tree.get(id)?;
    Some(match axis {
        WheelAxis::Vertical => (node.scroll_offset.1, node.scroll_remainder.1),
        WheelAxis::Horizontal => (node.scroll_offset.0, node.scroll_remainder.0),
    })
}

impl RinchApp {
    /// Turn smooth wheel scrolling on (the default) or off. Off, a wheel
    /// notch scrolls its whole distance in the frame it arrives, as before
    /// #2287; an animation running when it is turned off ends at its target
    /// on the next frame.
    pub fn set_smooth_wheel_scroll(&mut self, enabled: bool) {
        self.smooth_wheel.enabled = enabled;
    }

    /// Whether wheel notches animate.
    pub fn smooth_wheel_scroll_enabled(&self) -> bool {
        self.smooth_wheel.enabled
    }

    /// Whether a wheel scroll animation is running (the frame clock must keep
    /// turning).
    pub fn wheel_scroll_animating(&self) -> bool {
        !self.smooth_wheel.running.is_empty()
    }
}

impl SmoothWheelState {
    /// The glides' time for a document clock reading of `clock` ms: the last
    /// one plus however far the clock moved *forward* since. A step back (the
    /// wall clock corrected) counts as no time, and the next frame carries on
    /// from there.
    pub(crate) fn glide_now(&mut self, clock: f64) -> f64 {
        if let Some(seen) = self.clock_seen {
            self.glide_ms += (clock - seen).max(0.0);
        }
        self.clock_seen = Some(clock);
        self.glide_ms
    }

    /// A line wheel's `delta` content px on `axis` of `id`: start an
    /// animation toward it, or move the running one's target on by it.
    /// Answers whether the target moved (a notch into a scroller whose
    /// animation is already headed for its end moves nothing).
    pub(crate) fn animate(
        &mut self,
        doc: &RinchDocument,
        id: u64,
        axis: WheelAxis,
        delta: f64,
    ) -> bool {
        let Some((offset, remainder)) = precise_offset(doc, id, axis) else {
            return false;
        };
        let now = self.glide_now(doc.animation_now_ms());
        let max = max_offset(doc, id, axis);
        let running = self
            .running
            .iter_mut()
            .find(|a| a.node == id && a.axis == axis && a.written == offset);
        if let Some(anim) = running {
            let target = (anim.curve.target() + delta).clamp(0.0, max);
            if target == anim.curve.target() {
                return false;
            }
            anim.curve.update_target(now, target);
            return true;
        }
        // Nothing runs there, or something else has moved it since (stale).
        self.running.retain(|a| !(a.node == id && a.axis == axis));
        let initial = offset + remainder;
        let target = (initial + delta).clamp(0.0, max);
        if (target - initial).abs() < EPSILON {
            return false;
        }
        self.running.push(WheelAnimation {
            node: id,
            axis,
            curve: ScrollCurve::new(initial, target, now),
            written: offset,
        });
        true
    }
}

impl RinchApp {
    /// One frame of every running wheel animation, from the `AboutToWait`
    /// frame clock: each scroller moves to where its curve is now and fires
    /// its `onscroll`. Answers whether any is still running — whether the
    /// clock must keep turning.
    pub(crate) fn tick_wheel_scroll(&mut self) -> bool {
        if self.smooth_wheel.running.is_empty() {
            return false;
        }
        let Some(doc) = self.doc.clone() else {
            self.smooth_wheel.running.clear();
            return false;
        };
        let dragged = self.scrollbar_drag.as_ref().map(|d| d.node_id);
        let enabled = self.smooth_wheel.enabled;
        let mut fired: Vec<(u64, usize)> = Vec::new();
        let mut moved_any = false;
        {
            let mut d = doc.borrow_mut();
            let now = self.smooth_wheel.glide_now(d.animation_now_ms());
            self.smooth_wheel.running.retain_mut(|anim| {
                let Some((offset, remainder)) = precise_offset(&d, anim.node, anim.axis) else {
                    return false;
                };
                // Something else scrolled it, the scrollbar has it, or a
                // `lock_scroll` (#474) now keeps the user's gestures off it.
                if offset != anim.written
                    || dragged == Some(anim.node)
                    || d.tree.scroll_locked_out(anim.node)
                {
                    return false;
                }
                let finished = !enabled || anim.curve.is_finished(now);
                let to = if enabled {
                    anim.curve.value(now)
                } else {
                    anim.curve.target()
                };
                let to = to.clamp(0.0, max_offset(&d, anim.node, anim.axis));
                let delta = to - (offset + remainder);
                let (moved, handler) = match anim.axis {
                    WheelAxis::Vertical => Self::scroll_vertically_by(&mut d, anim.node, delta),
                    WheelAxis::Horizontal => Self::scroll_horizontally_by(&mut d, anim.node, delta),
                };
                if moved {
                    moved_any = true;
                    if let Some(h) = handler
                        && !fired.iter().any(|(n, _)| *n == anim.node)
                    {
                        fired.push((anim.node, h));
                    }
                }
                anim.written = precise_offset(&d, anim.node, anim.axis).map_or(offset, |p| p.0);
                !finished
            });
        }
        if moved_any {
            self.scene_dirty = true;
        }
        let events: Vec<(usize, rinch_core::events::ScrollEvent)> = {
            let d = doc.borrow();
            fired
                .iter()
                .map(|&(node, h)| (h, Self::scroll_event_for(&d.tree, node)))
                .collect()
        };
        for (h, event) in events {
            use rinch_core::events::{EventHandlerId, dispatch_scroll_event};
            dispatch_scroll_event(EventHandlerId(h), event);
        }
        // A frame that moved something must be drawn even when it was the
        // last one.
        if self.smooth_wheel.running.is_empty() {
            self.smooth_wheel.clock_seen = None;
        }
        moved_any || !self.smooth_wheel.running.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_chromiums_inverse_delta() {
        assert!((segment_duration_ms(40.0) - 200.0).abs() < 1e-9);
        assert!((segment_duration_ms(120.0) - 200.0).abs() < 1e-9);
        assert!((segment_duration_ms(-300.0) - 150.0).abs() < 1e-9);
        assert!((segment_duration_ms(480.0) - 100.0).abs() < 1e-9);
        assert!((segment_duration_ms(5000.0) - 100.0).abs() < 1e-9);
    }

    /// From rest the ease is CSS `ease-in-out`: symmetric, half way at half
    /// time, and starting slowly (the 40 px notch Chrome measured is at 1 px
    /// after a frame).
    #[test]
    fn from_rest_the_ease_is_ease_in_out() {
        let c = ScrollCurve::new(0.0, 40.0, 0.0);
        assert!((c.value(100.0) - 20.0).abs() < 1e-6);
        assert!((c.value(50.0) + c.value(150.0) - 40.0).abs() < 1e-6);
        let first = c.value(1000.0 / 60.0);
        assert!(first > 0.2 && first < 1.0, "{first}");
        assert_eq!(c.value(200.0), 40.0);
        assert!(c.is_finished(200.0) && !c.is_finished(199.0));
    }

    /// Chromium stops a retarget whose new segment would last under
    /// `kEpsilon` *seconds* (10 ms): the curve is at its new target at once.
    /// A fast glide nudged 2px on is such a one (the velocity bound is about
    /// 0.5 ms).
    #[test]
    fn a_retarget_under_ten_ms_lands_at_once() {
        let mut c = ScrollCurve::new(0.0, 600.0, 0.0);
        let t = 50.0;
        let at = c.value(t);
        c.update_target(t, at + 2.0);
        assert_eq!(c.value(t), at + 2.0);
        assert!(c.is_finished(t));
    }

    /// A retarget keeps the position and the velocity continuous.
    #[test]
    fn a_retarget_keeps_position_and_velocity() {
        let mut c = ScrollCurve::new(0.0, 40.0, 0.0);
        let t = 90.0;
        let h = 0.01;
        let before = c.value(t);
        let v_before = (c.value(t) - c.value(t - h)) / h;
        c.update_target(t, 80.0);
        let after = c.value(t);
        let v_after = (c.value(t + h) - c.value(t)) / h;
        assert!((before - after).abs() < 1e-9, "{before} vs {after}");
        assert!(
            (v_before - v_after).abs() / v_before < 0.01,
            "velocity {v_before} → {v_after}"
        );
        assert_eq!(c.value(10_000.0), 80.0);
    }
}
