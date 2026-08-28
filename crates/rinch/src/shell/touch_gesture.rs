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
//! | moves past [`SCROLL_THRESHOLD`] | a scroll | `PointerCancel`, then `MouseWheel` per frame, then momentum |
//! | lifts while still | a tap | `MouseDown`/`MouseUp` (left) at the down point |
//! | stays still past [`LONG_PRESS_TIMEOUT`] | a context menu | `MouseDown`/`MouseUp` (right) |
//!
//! Only the first of those three abandons something. A tap and a long press
//! each *complete* the press they started, so they end with a release; a scroll
//! takes the gesture away from whatever was under the finger, and
//! [`PlatformEvent::PointerCancel`] is how the document is told. Nothing else
//! ends a gesture here — see the note on `PointerCancel` in
//! [`TouchGesture::process`] for why there is no scroll-end event beside it.

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
    ///
    /// # Why a cancel and no scroll-end
    ///
    /// [`PlatformEvent::PointerCancel`] says "the interaction you had is gone",
    /// and the only thing here that takes one away is a scroll claiming the
    /// gesture. The obvious sibling — a "the finger lifted after scrolling"
    /// event — is deliberately absent, on three grounds:
    ///
    /// - Nothing could consume it. A scroll reaches the document as
    ///   `MouseWheel`, which only moves a container that can actually scroll; a
    ///   row that wants to know it was swiped never sees the deltas in the first
    ///   place, so handing it the lift would complete half a gesture.
    /// - The lift is not when the scroll ends. Momentum keeps emitting wheel
    ///   deltas after the finger is gone (see [`Self::tick_momentum`]), so an
    ///   end fired on `Up` would be a lie for as long as the list is still
    ///   coasting — an honest one has to wait for the fling to settle, which is
    ///   a design of its own and not this one.
    /// - It would be a second end-of-gesture channel. The pointer stream this
    ///   recogniser is being grown towards ends with a release the document
    ///   already understands, and two events meaning "the finger is up" is one
    ///   more than anyone can keep in agreement.
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
                            //
                            // The scroll has just taken the gesture over, and
                            // this is the moment the document has to hear about
                            // it: not at the lift, half a second later, but now,
                            // while it still might complete whatever the press
                            // started. Emitted on the crossing frame, so exactly
                            // once per gesture — `Scrolling` never returns to
                            // `Pending`.
                            events.push(PlatformEvent::PointerCancel);
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
                        // End of scroll drag — momentum will be applied in
                        // tick(). No event: the document was told this gesture
                        // was no longer its own back when the scroll claimed it,
                        // and a lift adds nothing to that.
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
                match self.state {
                    // The system took the contact away from an unresolved press
                    // — a parent view claiming the gesture, the app going to the
                    // background. Same message as a scroll taking it over, for
                    // the same reason: the press will never be completed.
                    TouchState::Pending { .. } => events.push(PlatformEvent::PointerCancel),
                    // A long press that already fired still gets its release:
                    // the press pair stays balanced and `:active` cannot stick.
                    // It is not cancelled — the context event *completed*, and
                    // telling the document to tear down the menu the same finger
                    // just opened is the opposite of what happened.
                    TouchState::LongPressed => events.push(PlatformEvent::MouseUp {
                        x,
                        y,
                        button: MouseButton::Right,
                    }),
                    // Already cancelled on the crossing frame; a second one
                    // would be a cancel with nothing left to cancel.
                    TouchState::Scrolling { .. } | TouchState::Idle => {}
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
                PlatformEvent::PointerCancel => "cancel".to_string(),
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
    /// rather than firing into the next frame. No cancel either — a tap
    /// *completes* the press it started, and the asserted sequence is the whole
    /// sequence, so the absence is checked rather than assumed.
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
            ["move 100 100", "cancel", "wheel 100 100 0 30"],
            "the cancel hands the gesture to the scroll; nothing else may be synthesised"
        );

        assert!(
            f.gesture.has_momentum(),
            "the flick must still coast after the lift"
        );
        f.tick(820);
        assert_eq!(f.emitted().len(), 4, "the coast emits a wheel event");
    }

    /// Once the menu is open the gesture belongs to it. Dragging the same finger
    /// away must not scroll the list behind the menu, and the lift must still not
    /// click — the alternative is a menu that appears and then has the page
    /// yanked out from under it by the very finger that opened it.
    ///
    /// Nor may the movement cancel: the press did not get taken away, it
    /// *resolved*, into the context event that fired half a second ago. A cancel
    /// here would tell the document to tear down the menu the same finger just
    /// opened. So the absorbed movement stays absorbed, and the sequence is the
    /// one stage 1 asserted, unchanged.
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

    /// The event this stage exists for, and the two things about it that are
    /// easy to get wrong: *when* it fires and *how often*.
    ///
    /// When: on the frame the finger leaves the slop, not on the first move
    /// (which is still a press) and not on the lift (by which time whatever was
    /// under the finger may already have acted). How often: once, however many
    /// frames the scroll then runs for — the document is told the gesture is no
    /// longer its own, and repeating that says nothing new.
    #[test]
    fn a_press_that_becomes_a_scroll_cancels_once_on_the_frame_it_crosses() {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 100.0, 100.0);

        // 3px is inside the slop — this is still a press that could yet be a tap
        // or a long press, and cancelling it here would kill both.
        f.act(10, TouchAction::Move, 103.0, 100.0);
        assert_eq!(f.emitted(), ["move 100 100"], "no cancel inside the slop");

        // 12px — the scroll claims the gesture on this frame.
        f.act(20, TouchAction::Move, 100.0, 112.0);
        assert_eq!(
            f.emitted(),
            ["move 100 100", "cancel"],
            "the cancel lands on the crossing frame, ahead of the first wheel"
        );

        f.act(30, TouchAction::Move, 100.0, 140.0);
        f.act(40, TouchAction::Move, 100.0, 170.0);
        f.act(50, TouchAction::Up, 100.0, 170.0);
        assert_eq!(
            f.emitted(),
            [
                "move 100 100",
                "cancel",
                "wheel 100 100 0 28",
                "wheel 100 100 0 30"
            ],
            "and the lift adds nothing — the cancel was the whole announcement"
        );
        assert_eq!(
            f.emitted().iter().filter(|e| *e == "cancel").count(),
            1,
            "exactly one cancel per gesture, however long the scroll ran"
        );
    }

    /// A cancelled gesture must leave nothing behind. The next press is an
    /// ordinary press: it taps, it does not re-announce the cancel that ended
    /// the gesture before it, and it does not inherit the flick's momentum.
    #[test]
    fn the_press_after_a_cancelled_one_starts_clean() {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 100.0, 100.0);
        f.act(20, TouchAction::Move, 100.0, 140.0);
        f.act(40, TouchAction::Move, 100.0, 180.0);
        f.act(60, TouchAction::Up, 100.0, 180.0);
        let after_scroll = f.emitted().len();
        assert!(
            f.gesture.has_momentum(),
            "precondition: the flick is still coasting when the next press lands"
        );

        f.act(200, TouchAction::Down, 50.0, 50.0);
        assert!(
            !f.gesture.has_momentum(),
            "a new press stops the coast rather than scrolling under the finger"
        );
        f.act(300, TouchAction::Up, 50.0, 50.0);
        f.tick(5_000);

        assert_eq!(
            &f.emitted()[after_scroll..],
            ["move 50 50", "down Left 50 50", "up Left 50 50"],
            "an ordinary tap, with no cancel leaked from the gesture before it"
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
        assert_eq!(
            f.emitted(),
            ["move 5 5", "cancel"],
            "the system took an unresolved press away — the document has to hear so"
        );

        let mut g = Finger::new();
        g.act(0, TouchAction::Down, 5.0, 5.0);
        g.tick(500);
        g.act(600, TouchAction::Cancel, 5.0, 5.0);
        assert_eq!(g.emitted(), ["move 5 5", "down Right 5 5", "up Right 5 5"]);
    }
}
