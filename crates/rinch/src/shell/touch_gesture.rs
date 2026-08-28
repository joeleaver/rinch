//! The Android touch recogniser: one finger's motion stream translated into the
//! pointer events [`RinchApp`](crate::app::RinchApp) already speaks.
//!
//! It lives here rather than in `android_runtime` because that module only
//! compiles for `target_os = "android"`, and this translation is the part worth
//! testing. Once [`TouchAction`] stands in for `android_activity`'s
//! `MotionAction` — an Android-target-only dependency — the recogniser is a
//! small state machine over a point and a clock with no Android left in it, and
//! a synthetic sequence on the host is the only way to keep it honest.
//!
//! A gesture ends as exactly one of three things, and the state machine is what
//! guarantees the "exactly":
//!
//! | the finger | becomes | emitted |
//! |---|---|---|
//! | moves past [`SCROLL_THRESHOLD`] | a scroll | `MouseWheel` per frame, then momentum |
//! | lifts while still | a tap | `MouseDown`/`MouseUp` (left) at the down point |
//! | stays still past [`LONG_PRESS_TIMEOUT`] | a context menu | `MouseDown`/`MouseUp` (right) |

use std::time::{Duration, Instant};

use rinch_platform::{MouseButton, PlatformEvent};

/// How far the finger may travel before the press stops being a press.
///
/// Coordinates reach the recogniser already divided by the display's scale
/// factor, so this is 8 *CSS* pixels — the same number Android itself uses for
/// `ViewConfiguration.getScaledTouchSlop()` (8dp).
pub(crate) const SCROLL_THRESHOLD: f32 = 8.0;

const MOMENTUM_FRICTION: f32 = 0.95;
const MOMENTUM_MIN_VELOCITY: f32 = 0.5;

/// How long a still finger must stay down to mean "context menu".
///
/// `ViewConfiguration.getLongPressTimeout()`, which every Android widget has
/// used since API 1 and which is therefore the only duration a phone user has
/// been taught. Shortening it would steal presses from taps that happen to
/// linger; lengthening it would make the menu feel unreachable.
const LONG_PRESS_TIMEOUT: Duration = Duration::from_millis(500);

/// The motion actions the recogniser distinguishes.
///
/// `android_runtime` maps `android_activity::input::MotionAction` onto this;
/// everything it does not name arrives as [`TouchAction::Other`] and is ignored,
/// which is what the original `_ => {}` arm did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TouchAction {
    Down,
    Move,
    Up,
    Cancel,
    HoverMove,
    Other,
}

enum TouchState {
    Idle,
    /// Finger down, still within [`SCROLL_THRESHOLD`] of where it landed and not
    /// yet held long enough to be a long press. `down_at` is what the long-press
    /// deadline is measured from, so the timer lives and dies with this state:
    /// leaving `Pending` — by scrolling, by lifting, by cancelling — cancels it
    /// without any separate bookkeeping.
    Pending {
        x: f32,
        y: f32,
        down_at: Instant,
    },
    /// Finger is dragging — emit scroll events.
    Scrolling {
        last_x: f32,
        last_y: f32,
    },
    /// The long press already fired its context event. The rest of this gesture
    /// belongs to the menu that just opened: further movement must not scroll
    /// the list underneath it, and the lift must not click through it.
    LongPressed,
}

pub(crate) struct TouchGesture {
    state: TouchState,
    /// Velocity for momentum scrolling (pixels per frame).
    velocity_x: f32,
    velocity_y: f32,
    /// Where to send scroll events (the initial touch point).
    scroll_origin: (f32, f32),
}

impl TouchGesture {
    pub(crate) fn new() -> Self {
        Self {
            state: TouchState::Idle,
            velocity_x: 0.0,
            velocity_y: 0.0,
            scroll_origin: (0.0, 0.0),
        }
    }

    /// `now` is passed in rather than read from the clock so that the whole
    /// recogniser stays a pure function of its inputs, and a test can hold a
    /// finger down for 500ms without sleeping.
    pub(crate) fn process(
        &mut self,
        action: TouchAction,
        x: f32,
        y: f32,
        now: Instant,
        events: &mut Vec<PlatformEvent>,
    ) {
        match action {
            TouchAction::Down => {
                self.velocity_x = 0.0;
                self.velocity_y = 0.0;
                self.scroll_origin = (x, y);
                self.state = TouchState::Pending { x, y, down_at: now };
                events.push(PlatformEvent::MouseMove { x, y });
            }
            TouchAction::Move => {
                match self.state {
                    TouchState::Pending {
                        x: start_x,
                        y: start_y,
                        ..
                    } => {
                        let dx = x - start_x;
                        let dy = y - start_y;
                        if dx.abs() > SCROLL_THRESHOLD || dy.abs() > SCROLL_THRESHOLD {
                            // Crossed threshold — switch to scrolling, which
                            // also drops the pending long press.
                            self.state = TouchState::Scrolling {
                                last_x: x,
                                last_y: y,
                            };
                        }
                    }
                    TouchState::Scrolling { last_x, last_y } => {
                        let delta_x = (x - last_x) as f64;
                        let delta_y = (y - last_y) as f64;
                        self.velocity_x = (x - last_x) * 0.8 + self.velocity_x * 0.2;
                        self.velocity_y = (y - last_y) * 0.8 + self.velocity_y * 0.2;
                        self.state = TouchState::Scrolling {
                            last_x: x,
                            last_y: y,
                        };
                        let (ox, oy) = self.scroll_origin;
                        events.push(PlatformEvent::MouseWheel {
                            x: ox,
                            y: oy,
                            delta_x,
                            delta_y,
                        });
                    }
                    // The menu is already open under the finger; scrolling what
                    // is behind it is never what was asked for.
                    TouchState::LongPressed => {}
                    TouchState::Idle => {
                        events.push(PlatformEvent::MouseMove { x, y });
                    }
                }
            }
            TouchAction::Up => {
                match self.state {
                    TouchState::Pending { x, y, .. } => {
                        // Didn't exceed threshold and didn't outlast the
                        // long-press deadline — this was a tap
                        events.push(PlatformEvent::MouseDown {
                            x,
                            y,
                            button: MouseButton::Left,
                        });
                        events.push(PlatformEvent::MouseUp {
                            x,
                            y,
                            button: MouseButton::Left,
                        });
                    }
                    TouchState::Scrolling { .. } => {
                        // End of scroll drag — momentum will be applied in tick()
                    }
                    // The context event was dispatched half a second ago. The
                    // release only closes the press it opened — no left-button
                    // pair, so the long press cannot also activate whatever it
                    // was held over.
                    TouchState::LongPressed => {
                        events.push(PlatformEvent::MouseUp {
                            x,
                            y,
                            button: MouseButton::Right,
                        });
                    }
                    TouchState::Idle => {}
                }
                self.state = TouchState::Idle;
            }
            TouchAction::Cancel => {
                // A cancelled gesture never becomes a tap, but a long press that
                // already fired still gets its release: the press pair stays
                // balanced and `:active` cannot stick.
                if matches!(self.state, TouchState::LongPressed) {
                    events.push(PlatformEvent::MouseUp {
                        x,
                        y,
                        button: MouseButton::Right,
                    });
                }
                self.state = TouchState::Idle;
                self.velocity_x = 0.0;
                self.velocity_y = 0.0;
            }
            TouchAction::HoverMove => {
                events.push(PlatformEvent::MouseMove { x, y });
            }
            TouchAction::Other => {}
        }
    }

    /// Fire the context event for a press that has been held still past
    /// [`LONG_PRESS_TIMEOUT`].
    ///
    /// Called once per event-loop iteration alongside [`Self::tick_momentum`].
    /// The loop already polls on a 16ms timeout, so the menu opens within a
    /// frame of the deadline whether or not the finger is producing events.
    ///
    /// The right-button press is the whole synthesis: `RinchApp` routes it
    /// through `dispatch_oncontextmenu`, the one path a desktop right-click
    /// takes. Nothing here knows whether a handler was found — when there is
    /// none, the press falls through to the ordinary click dispatch exactly as a
    /// desktop right-click does, and the release below still ends the gesture.
    pub(crate) fn tick_long_press(&mut self, now: Instant, events: &mut Vec<PlatformEvent>) {
        let TouchState::Pending { x, y, down_at } = self.state else {
            return;
        };
        if now.duration_since(down_at) < LONG_PRESS_TIMEOUT {
            return;
        }
        events.push(PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Right,
        });
        self.state = TouchState::LongPressed;
    }

    /// Whether momentum scrolling still owes the window a frame.
    pub(crate) fn has_momentum(&self) -> bool {
        self.velocity_x.abs() > MOMENTUM_MIN_VELOCITY
            || self.velocity_y.abs() > MOMENTUM_MIN_VELOCITY
    }

    /// Generate momentum scroll events. Returns true if still animating.
    pub(crate) fn tick_momentum(&mut self, events: &mut Vec<PlatformEvent>) -> bool {
        if matches!(self.state, TouchState::Scrolling { .. }) {
            // Still touching — don't apply momentum
            return false;
        }
        if self.velocity_x.abs() < MOMENTUM_MIN_VELOCITY
            && self.velocity_y.abs() < MOMENTUM_MIN_VELOCITY
        {
            self.velocity_x = 0.0;
            self.velocity_y = 0.0;
            return false;
        }

        let (ox, oy) = self.scroll_origin;
        events.push(PlatformEvent::MouseWheel {
            x: ox,
            y: oy,
            delta_x: self.velocity_x as f64,
            delta_y: self.velocity_y as f64,
        });

        self.velocity_x *= MOMENTUM_FRICTION;
        self.velocity_y *= MOMENTUM_FRICTION;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One short line per event, so a test can assert the *whole* emitted
    /// sequence. The failures that matter here are extra events — a long press
    /// that also clicks, a scroll that also opens a menu — and only an exact
    /// sequence catches those.
    fn summarize(events: &[PlatformEvent]) -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                PlatformEvent::MouseMove { x, y } => format!("move {x} {y}"),
                PlatformEvent::MouseDown { x, y, button } => format!("down {button:?} {x} {y}"),
                PlatformEvent::MouseUp { x, y, button } => format!("up {button:?} {x} {y}"),
                PlatformEvent::MouseWheel {
                    x,
                    y,
                    delta_x,
                    delta_y,
                } => format!("wheel {x} {y} {delta_x} {delta_y}"),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// A finger driven off one base instant, so a test reads as a script and
    /// holding still for half a second costs nothing.
    struct Finger {
        gesture: TouchGesture,
        t0: Instant,
        events: Vec<PlatformEvent>,
    }

    impl Finger {
        fn new() -> Self {
            Self {
                gesture: TouchGesture::new(),
                t0: Instant::now(),
                events: Vec::new(),
            }
        }

        fn act(&mut self, ms: u64, action: TouchAction, x: f32, y: f32) {
            let now = self.t0 + Duration::from_millis(ms);
            self.gesture.process(action, x, y, now, &mut self.events);
        }

        /// One turn of the event loop, which is where both timers are driven —
        /// same order as `collect_input_events`.
        fn tick(&mut self, ms: u64) {
            let now = self.t0 + Duration::from_millis(ms);
            self.gesture.tick_long_press(now, &mut self.events);
            self.gesture.tick_momentum(&mut self.events);
        }

        /// Everything emitted since the gesture began.
        fn emitted(&self) -> Vec<String> {
            summarize(&self.events)
        }
    }

    /// The gesture this whole change exists for. Holding a still finger past the
    /// deadline synthesises the right-button press `dispatch_oncontextmenu`
    /// listens for, and — the half that is easy to get wrong — the lift that
    /// follows emits no left-button pair, so the long press cannot also activate
    /// whatever it was held over.
    #[test]
    fn a_still_press_held_past_the_deadline_is_a_context_menu_and_never_a_click() {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 40.0, 60.0);
        f.tick(499);
        assert_eq!(
            f.emitted(),
            ["move 40 60"],
            "the deadline is 500ms; 499 is still a tap in waiting"
        );

        f.tick(500);
        f.act(900, TouchAction::Up, 40.0, 60.0);
        f.tick(920);
        assert_eq!(
            f.emitted(),
            ["move 40 60", "down Right 40 60", "up Right 40 60"]
        );
    }

    /// The behaviour that already worked must keep working: a press released
    /// before the deadline is the tap it always was, and the timer dies with it
    /// rather than firing into the next frame.
    #[test]
    fn a_press_released_before_the_deadline_is_still_a_tap() {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 12.0, 24.0);
        f.tick(200);
        f.act(300, TouchAction::Up, 12.0, 24.0);
        f.tick(5_000);
        assert_eq!(
            f.emitted(),
            ["move 12 24", "down Left 12 24", "up Left 12 24"]
        );
    }

    /// A finger that leaves the slop is a scroll, and a scroll is not a press:
    /// however long it is then held, no context event may appear. Momentum
    /// survives the lift, which is what keeps the flick working.
    #[test]
    fn a_press_that_became_a_scroll_never_becomes_a_context_menu() {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 100.0, 100.0);
        // Past the 8px slop — this is now a scroll, and the pending press is gone.
        f.act(20, TouchAction::Move, 100.0, 130.0);
        f.act(40, TouchAction::Move, 100.0, 160.0);
        // Held far past the long-press deadline without lifting.
        f.tick(600);
        f.tick(700);
        f.act(800, TouchAction::Up, 100.0, 160.0);
        assert_eq!(
            f.emitted(),
            ["move 100 100", "wheel 100 100 0 30"],
            "the wheel event is the scroll; nothing else may be synthesised"
        );

        assert!(
            f.gesture.has_momentum(),
            "the flick must still coast after the lift"
        );
        f.tick(820);
        assert_eq!(f.emitted().len(), 3, "the coast emits a wheel event");
    }

    /// Once the menu is open the gesture belongs to it. Dragging the same finger
    /// away must not scroll the list behind the menu, and the lift must still not
    /// click — the alternative is a menu that appears and then has the page
    /// yanked out from under it by the very finger that opened it.
    #[test]
    fn movement_after_the_context_menu_has_fired_is_absorbed() {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 100.0, 100.0);
        f.tick(500);
        f.act(520, TouchAction::Move, 100.0, 200.0);
        f.act(540, TouchAction::Move, 100.0, 300.0);
        f.act(560, TouchAction::Up, 100.0, 300.0);
        f.tick(600);
        assert_eq!(
            f.emitted(),
            ["move 100 100", "down Right 100 100", "up Right 100 300"],
            "no wheel, no click — and the release lands where the finger left"
        );
    }

    /// The two actions that are not part of a press: a hovering pointer (a stylus
    /// or a mouse) still moves the cursor, and everything the recogniser does not
    /// name is ignored rather than guessed at.
    #[test]
    fn a_hover_moves_the_pointer_and_an_unnamed_action_is_ignored() {
        let mut f = Finger::new();
        f.act(0, TouchAction::HoverMove, 7.0, 9.0);
        f.act(10, TouchAction::Other, 7.0, 9.0);
        f.tick(5_000);
        assert_eq!(f.emitted(), ["move 7 9"]);
    }

    /// A cancelled press is neither a tap nor a menu. A cancel *after* the menu
    /// opened still closes the press pair, so a `data-onmousedown` is never left
    /// hanging and `:active` cannot stick.
    #[test]
    fn a_cancelled_press_is_neither_a_tap_nor_a_context_menu() {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 5.0, 5.0);
        f.act(100, TouchAction::Cancel, 5.0, 5.0);
        f.tick(5_000);
        assert_eq!(f.emitted(), ["move 5 5"]);

        let mut g = Finger::new();
        g.act(0, TouchAction::Down, 5.0, 5.0);
        g.tick(500);
        g.act(600, TouchAction::Cancel, 5.0, 5.0);
        assert_eq!(g.emitted(), ["move 5 5", "down Right 5 5", "up Right 5 5"]);
    }
}
