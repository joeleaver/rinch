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
//! | moves past [`SCROLL_THRESHOLD`] | a scroll | `PointerCancel`, then a `MouseWheel` per move, then momentum |
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

/// How much of its speed a fling keeps across one [`MOMENTUM_TICK`], and the
/// speed below which it has stopped. Both are in pixels per *reference tick*,
/// not per second and not per frame — see [`MOMENTUM_TICK`] for why that
/// distinction is the whole point of this trio of constants.
const MOMENTUM_FRICTION: f32 = 0.95;
const MOMENTUM_MIN_VELOCITY: f32 = 0.5;

/// The tick these numbers were tuned against: 60Hz.
///
/// **The failure that put this here.** Until card K39 there was no such
/// constant, because there did not need to be one: [`Self::tick_momentum`] was
/// called once per turn of the event loop, the Android loop turned every 16ms
/// because its poll said so, and "per tick" and "per 60Hz frame" were the same
/// sentence. Card K37 then paced that loop to the panel instead of to a
/// hard-coded 16ms, the moto g stylus 5G it was measured on runs at 120Hz, and
/// every fling in every app on this framework silently started travelling the
/// same distance in half the time.
///
/// That is not a subtle effect and it does not show up in a frame-rate number,
/// which is why K37 shipped without noticing it and K39 was raised as "119fps
/// that does not feel like 120". Measured on that handset, three identical
/// flicks of the library list, counting frames and milliseconds from the last
/// touch to the last frame the fling produced:
///
/// ```text
///            coast frames   coast duration
///   120Hz      106/87/81     1206/717/679 ms
///    60Hz      104/85/86    2111/1429/1449 ms
/// ```
///
/// The frame counts are the same and the durations are exactly doubled, which
/// is the signature of a curve measured in ticks rather than in time: the
/// friction had been applied 85 times either way, and all that changed was how
/// long 85 of them took. A list that stops in 700ms instead of 1.4s reads as
/// grabby and abrupt next to the system's own scrollers, at any frame rate.
///
/// So the tick is now a duration and the decay is raised to the power of how
/// many of them have actually elapsed. 60Hz is the reference because that is
/// the rate `MOMENTUM_FRICTION = 0.95` was chosen at; picking any other number
/// here would re-tune the fling as a side effect of describing it.
const MOMENTUM_TICK: Duration = Duration::from_nanos(16_666_667);

/// The most ticks one call is allowed to advance the fling by.
///
/// A frame that took 200ms — a stall, a sheet rasterising, the app coming back
/// from the background with a fling still in flight — would otherwise resolve
/// twelve ticks at once and jump the list a screenful in a single frame. Four
/// is a 15fps frame: past that the fling is already visibly broken and the
/// honest thing is to under-shoot rather than to teleport.
const MOMENTUM_MAX_STEPS: f32 = 4.0;

/// How many finger positions are kept behind the fling's launch speed.
///
/// This is storage, not policy: [`VELOCITY_WINDOW`] is the horizon that decides
/// what the speed is measured over, and this only bounds how far back the
/// storage can reach. Eight samples span seven intervals, which is 117ms at
/// 60Hz — longer than the horizon, so the horizon does the trimming — and 58ms
/// at the 120Hz Android actually feeds this app (see [`EventClock`] for why 120
/// and not the 238 the panel's digitiser is capable of). A shorter span is a
/// noisier speed, not a wrong one, and the reason the fling does not care about
/// the report rate is that the estimate divides a distance by the duration it
/// took: halve the rate and both halves halve together.
///
/// It does have to be more than two. A speed read off the newest pair alone is
/// measured across a single reporting interval — 8.3ms of finger, or 4.2ms if
/// the normalisation is ever switched off — and that little travel is mostly the
/// digitiser's own quantisation rather than the flick.
///
/// Small enough to be a fixed array in the recogniser rather than an allocation
/// on the input path.
const SAMPLE_WINDOW: usize = 8;

/// The trailing window the fling's launch speed is measured over.
///
/// **Velocity used to be measured in pixels per move event**, which is a unit
/// with no time in it. `velocity = (x - last_x) * 0.8 + velocity * 0.2` is an
/// exponential average of per-*sample* distances, and it was then handed to
/// [`TouchGesture::tick_momentum`] as though it were pixels per 60Hz tick — true
/// only on a digitiser that happens to report at exactly 60Hz, which is a
/// digitiser nobody has shipped in a decade. On one reporting twice that, every
/// sample covers half the distance, so the same flick of the same list left
/// behind half the fling. That is the same mistake card K39 found in the *decay*
/// — a curve written in ticks rather than in seconds — one term earlier in the
/// same expression, and K39 fixed only the half that a frame-rate number could
/// show.
///
/// **The measurement, on the moto g stylus 5G.** The same scripted flick,
/// injected at two touch report rates, coast distance from the lift to the frame
/// the list stopped on:
///
/// ```text
///                     pixels per event     pixels per second
///   120Hz touch             283px                596px
///   240Hz touch             233px                594px
/// ```
///
/// 21% apart before and 0.3% apart after. A flick is a speed, and a speed does
/// not know what the digitiser's polling interval is; the left-hand column is
/// what it costs to write one down in a unit that does.
///
/// **Re-measured after the resampler was taken back out**, because a number
/// taken alongside a mechanism that has since been deleted is a number about
/// something else. Same handset, same GPU build, a scripted 1600px/s flick fed
/// through the recogniser at three report rates, coast distance from the lift
/// to the frame the fling settled on: 523.9px at 60Hz, 523.4px at 120Hz,
/// 524.0px at 240Hz — 0.1% across the three. The correction did not depend on
/// the resampler and did not leave with it.
///
/// So the launch speed is a distance over a duration, taken across the real
/// samples inside this window. `VelocityTracker`'s horizon in Android is 100ms
/// and there is no reason to disagree with it. One property comes with the
/// window rather than with the arithmetic, and it matters more than the
/// arithmetic does: **a finger that stopped does not fling.** If the digitiser
/// kept reporting while the finger sat still, the window fills with samples that
/// do not move and the speed goes to zero. If it stopped reporting altogether,
/// the newest sample falls out of the window and the speed goes to zero that way
/// instead. The old per-event average did neither — it held the speed of a
/// gesture that had been over for the better part of a second, and the list took
/// off from under a stationary finger the moment it lifted.
/// `a_press_that_became_a_scroll_never_becomes_a_context_menu` is where that is
/// pinned.
///
/// **This replaced the estimator, not only its unit, and that is a change of
/// feel at 60Hz too.** Worth stating flatly, because the rest of this comment
/// is about a unit conversion and it would be easy to read the whole change as
/// one. The old `(x - last_x) * 0.8 + velocity * 0.2` weights its samples
/// 0.8 / 0.16 / 0.032 — effectively the last three, dominated by the final one —
/// where this takes a plain chord across the whole window. For a finger moving
/// at a *constant* speed the two agree exactly, which is why the reference
/// configuration is untouched; for any finger whose speed varies they do not,
/// at 60Hz as much as anywhere. Measured on the host at a 60Hz panel and a 60Hz
/// digitiser, coast distance before against after:
///
/// ```text
///   constant speed, no jitter            657px -> 657px     +0.0%  (exact)
///   decelerating 3000 -> 1000px/s        356px -> 457px    +28.4%
///   accelerating 1000 -> 3000px/s        954px -> 856px    -10.2%
/// ```
///
/// The two shape rows use `v(u) = 2000 * (1 - shape * (u/0.25 - 0.5))` over a
/// 250ms drag; the magnitude is a function of that ramp, so read the sign and
/// the order of magnitude rather than the digits.
///
/// **Jittered sampling is deliberately not a row in that table**, because it is
/// not a shift — it is noise, and quoting a single draw of it as a percentage
/// would misdescribe it. Over 300 seeds at a 60Hz digitiser with each sample
/// displaced by up to a quarter of an interval:
///
/// ```text
///   old (per-event average)    mean 654px   sd 102px   range -27% .. +58%
///   new (chord over the window) mean 657px   sd   0px
/// ```
///
/// The signed difference straddles zero — the new estimate is the *larger* one
/// in 52% of seeds — so the honest statement is not "jitter makes flicks 5%
/// longer" but the stronger one: **the old estimate carried ±100px of coast
/// that was purely an artefact of when the digitiser happened to sample, and
/// this one carries none.** That is the same property as the noise-rejection
/// argument below, measured a second way.
///
/// Intended, and the reason is **noise rejection** — which is worth stating
/// carefully, because the obvious-sounding reason is false.
///
/// It is tempting to credit the window with "a finger that stopped does not
/// fling". It does not own that property: [`Self::sample_velocity`]'s staleness
/// guard does, and the guard is in front of the window and independent of its
/// width. Measured — a finger at 2000px/s that stops dead, then the same
/// estimator narrowed to the newest *pair*:
///
/// ```text
///                                       window       newest pair
///   digitiser goes silent, 17ms         2000px/s        2000px/s
///   digitiser goes silent, 150ms           0px/s           0px/s
///   still reporting, 17ms of stillness  1667px/s           0px/s
///   still reporting, 50ms of stillness  1000px/s           0px/s
/// ```
///
/// The pair is *identical* when the digitiser falls silent — both are the
/// guard — and strictly **better** when it keeps reporting a stationary finger,
/// where the window is still averaging in motion that has stopped. So the
/// narrow estimator has the stopped-finger property too, and has it sooner.
///
/// What the window actually buys is immunity to sampling noise, which is large
/// and is the honest justification. One 6px blip on the final report costs a
/// 2000px/s estimate 5% across eight samples and **36%** across the newest pair
/// (`one_noisy_sample_cannot_own_the_launch_speed`), and over 300 jittered
/// seeds the old per-event average carries a standard deviation of 102px of
/// coast where this carries **zero** (see the table above). A flick should not
/// depend on which microsecond the digitiser happened to sample.
///
/// The old average failed for a different reason again: it had no time in it at
/// all, so it held the speed of a gesture that had been over for the better
/// part of a second.
///
/// **Where this does disagree with `VelocityTracker`**, since the horizon is
/// borrowed from it: AOSP fits a curve across the horizon to estimate the
/// velocity *at the last sample*, where this takes a first-to-last chord, which
/// is the window's *mean*. So a finger decelerating into the lift launches at
/// more than the speed it was actually going — measured, 397px of coast where
/// its terminal 933px/s warrants about 311px. That is a knowingly cruder
/// estimator, not an oversight; a least-squares fit is a change of its own
/// shape and would re-tune the feel a second time in the same PR.
///
/// It changes what a flick does on any digitiser not reporting at 60Hz as well,
/// and it is supposed to. [`MOMENTUM_FRICTION`] is the knob if the corrected
/// flings overshoot.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

/// How long a still finger must stay down to mean "context menu".
///
/// `ViewConfiguration.getLongPressTimeout()`, which every Android widget has
/// used since API 1 and which is therefore the only duration a phone user has
/// been taught. Shortening it would steal presses from taps that happen to
/// linger; lengthening it would make the menu feel unreachable.
const LONG_PRESS_TIMEOUT: Duration = Duration::from_millis(500);
/// How old an event may map before [`EventClock`] stops believing its anchor.
///
/// Half a second is far longer than any plausible input latency, so an event
/// that maps further back than this says the *anchor* has stopped describing
/// the offset between the two clocks — the loop was away (the app was
/// backgrounded mid-gesture), or the device's clock jumped. Re-anchoring is the
/// answer to both, and it is the only thing this constant does.
///
/// **It does not detect a wrong unit, and no threshold could.** [`EventClock`]
/// uses only *differences* between event timestamps — that is the whole of why
/// it is safe without asserting what base the device counts in — so the
/// absolute epoch is invisible to it by construction, and a coarser unit
/// (milliseconds read as nanoseconds) makes those differences 10⁶ times
/// *smaller*, not larger. There is no unit mismatch that dates two samples
/// further apart than they really are.
///
/// What a coarser unit actually does is collapse the span the launch speed
/// divides by, which is why [`MAX_FLING_VELOCITY`] exists: measured, a 64ms
/// flick whose timestamps are milliseconds read as nanoseconds asked for a
/// single 41,666,668px frame before that clamp was added. See
/// `a_clock_running_in_the_wrong_unit_cannot_teleport_the_list`.
const MAX_EVENT_AGE: Duration = Duration::from_millis(500);

/// The fastest a fling may leave the finger, in pixels per second.
///
/// `ViewConfiguration.getScaledMaximumFlingVelocity()` — 8000dp/s, the number
/// Android has clamped its own flings to since API 1. Coordinates reach this
/// module already divided by the display's scale factor, so 8000 here is 8000
/// *dp* and the two agree.
///
/// **Defence in depth, not a live bug.** The launch speed is now a distance
/// divided by a measured duration ([`VELOCITY_WINDOW`]), and a quotient is
/// unbounded when its denominator is wrong: on `main` the estimate was a
/// per-event distance and could never exceed a screen's width, whereas this one
/// is as large as the span is small. `MotionEvent::event_time()` is documented
/// as `java.lang.System.nanoTime()` nanoseconds and the `ndk` crate says so
/// too, so on a conforming device the span is tens of milliseconds and the
/// estimate stays in the low thousands of dp/s.
///
/// It is **not** claimed that no one can flick this fast — AOSP picked 8000
/// precisely because people can, and a hard flick on a large screen is exactly
/// the gesture that gets there. A flick at or past the ceiling is clamped to it
/// on Android too, which is the behaviour being matched rather than a corner
/// being written off. The clamp is
/// here because the cost of being wrong about a platform contract should be a
/// fast fling rather than the list teleporting to its end, and because
/// [`MOMENTUM_MAX_STEPS`] already spends four lines making exactly that
/// argument about a stalled frame. A clamp that is never reached costs one
/// comparison per lift.
const MAX_FLING_VELOCITY: f32 = 8000.0;

/// Puts `MotionEvent::event_time()` onto the loop's own clock, without assuming
/// the two are the same clock.
///
/// **Why the finger's own timestamps are worth this much code.** The launch
/// speed of a fling is a distance divided by a duration — see
/// [`VELOCITY_WINDOW`] — and until this existed the duration was a fiction.
/// Every event drained in one turn of the loop was stamped with the instant the
/// *loop woke up*, so a whole batch of samples carried the same time,
/// [`TouchGesture::push_sample`] collapsed them into one, and the span the
/// distance was divided by was quantised to the frame grid rather than measured
/// off the finger. On a 120Hz panel that is an error of up to 8.3ms on a span of
/// about 58ms. Measured against a digitiser jittering by a quarter of its own
/// interval, loop-stamped samples left 26% of variation in the estimate where
/// event-stamped ones leave 2%.
///
/// **What the handset actually reports**, because this was first written on a
/// guess and the guess was wrong. Across three real gestures on the moto g
/// stylus 5G — 836 position reports — the digitiser's own interval is 4.21ms
/// p50, about 238Hz, corroborated against `/proc/interrupts`. That is *twice*
/// the panel, not half of it. Android then normalises the stream to the
/// display's rate before the app is handed anything, so what arrives here is
/// 120Hz of real samples carrying real times; a 60Hz stream passes through as
/// 60Hz and nothing is ever upsampled. The timestamps are worth reading. The
/// report rate is not worth defending against — see the note on the `Scrolling`
/// arm of [`TouchGesture::process`] for the machinery that came off once that
/// was measured rather than assumed.
///
/// **Why it does not simply convert.** `AMotionEvent_getEventTime` is documented
/// as `java.lang.System.nanoTime()`, and `std::time::Instant` on Android is
/// `CLOCK_MONOTONIC`, and those are the same clock — but "are the same clock" is
/// a fact about a platform, not a fact this file can check, and a shell that
/// silently renders nonsense when it is wrong is a bad trade for the four lines
/// it saves. So no absolute conversion is ever done. Only *differences* within
/// the event clock are used, anchored to one instant the loop observed itself,
/// which is meaningful whatever base the device is counting in.
///
/// **And the anchor corrects itself.** Input latency is never negative: an
/// event exists before the loop reads it. So the anchor with the *smallest*
/// latency is the best estimate of the offset between the two clocks, and any
/// event whose mapped instant lands after the instant the loop observed it has
/// just proved itself the better anchor. Re-anchoring on exactly those events
/// converges on the minimum latency and stays there. An event that maps to more
/// than [`MAX_EVENT_AGE`] ago has not proved anything except that the anchor is
/// stale or the base is not what was assumed, and re-anchoring is the answer to
/// both.
///
/// **One case is deliberately given up on**, and it is worth naming because it
/// looks like a bug from the outside. On the very *first* turn of the loop after
/// an anchor is taken, that anchor pins its own event to the instant the loop
/// read it — a latency of zero, the most optimistic reading possible — so every
/// later event in the same batch maps into the future, is judged the better
/// anchor in turn, and collapses onto the same instant. The batch loses its
/// internal spacing exactly once, and a gesture that both began and ended inside
/// that one turn would have its launch speed measured across a span shorter than
/// it really was. It is the cheapest of the available wrong answers: a flick
/// lasts longer than one turn of the loop, and every turn after the first has an
/// anchor from a previous one to measure against, so the spacing survives —
/// which is the case this spends its life in, and the case
/// `the_event_clock_preserves_the_spacing_between_samples` asserts.
#[derive(Default)]
pub(crate) struct EventClock {
    /// An instant the loop observed, and the device's own timestamp for the
    /// event it observed at that instant.
    anchor: Option<(Instant, i64)>,
}

impl EventClock {
    pub(crate) fn new() -> Self {
        Self { anchor: None }
    }

    /// The instant, on the loop's clock, at which the event stamped `event_ns`
    /// was generated. `now` is when the loop observed it.
    pub(crate) fn instant_for(&mut self, now: Instant, event_ns: i64) -> Instant {
        let (anchor_at, anchor_ns) = *self.anchor.get_or_insert((now, event_ns));
        let offset = event_ns - anchor_ns;
        let mapped = if offset >= 0 {
            anchor_at.checked_add(Duration::from_nanos(offset as u64))
        } else {
            anchor_at.checked_sub(Duration::from_nanos(offset.unsigned_abs()))
        };

        match mapped {
            // The ordinary case: the event happened somewhere between the last
            // time the loop looked and now.
            Some(t) if t <= now && now.duration_since(t) <= MAX_EVENT_AGE => t,
            // Either this event beat the anchor's latency, or the anchor no
            // longer describes anything. Both are answered by believing this
            // event instead, and both leave the caller with `now` — which is
            // exactly what it had before this type existed.
            _ => {
                self.anchor = Some((now, event_ns));
                now
            }
        }
    }
}

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
    /// Finger is dragging — every move it makes emits the distance it covered.
    ///
    /// It used to carry `last_x` / `last_y` for the wheel event to subtract
    /// from. That position is now the newest entry in
    /// [`TouchGesture::samples`], which the launch speed needs kept anyway: two
    /// records of where the finger last was are one more than can be held in
    /// agreement, and of the two the sample ring is the one that has to exist.
    Scrolling,
    /// The long press already fired its context event. The rest of this gesture
    /// belongs to the menu that just opened: further movement must not scroll
    /// the list underneath it, and the lift must not click through it.
    LongPressed,
}

pub(crate) struct TouchGesture {
    state: TouchState,
    /// Velocity for momentum scrolling, in pixels per [`MOMENTUM_TICK`].
    ///
    /// Still per-tick, because that is the unit [`MOMENTUM_FRICTION`] was tuned
    /// in and K39's curve is written in — but it is now *derived* from a
    /// pixels-per-second estimate rather than being a per-sample distance
    /// wearing the same name. See [`VELOCITY_WINDOW`].
    velocity_x: f32,
    velocity_y: f32,
    /// Where to send scroll events (the initial touch point).
    scroll_origin: (f32, f32),
    /// When the fling was last advanced, so the next advance knows how much
    /// time it owes. `None` between flings: the first tick of a new one has no
    /// previous tick to measure from and is charged exactly one
    /// [`MOMENTUM_TICK`], which is what the loop would have done before K39
    /// anyway.
    last_momentum: Option<Instant>,
    /// The recent finger positions, oldest first, and how many of the slots are
    /// filled. Two jobs, and it is worth being clear that the second is the one
    /// that justifies the array: the newest entry is what a `Move` subtracts
    /// from to get its wheel delta, and the whole window is what the launch
    /// speed is measured across at the lift. See [`VELOCITY_WINDOW`].
    ///
    /// Each is stamped with the instant the digitiser sampled the finger, not
    /// the instant the loop woke up and read it — [`EventClock`] is what
    /// recovers the difference, and why it is worth recovering.
    samples: [Option<(Instant, f32, f32)>; SAMPLE_WINDOW],
    sample_count: usize,
}

impl TouchGesture {
    pub(crate) fn new() -> Self {
        Self {
            state: TouchState::Idle,
            velocity_x: 0.0,
            velocity_y: 0.0,
            scroll_origin: (0.0, 0.0),
            last_momentum: None,
            samples: [None; SAMPLE_WINDOW],
            sample_count: 0,
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
                self.last_momentum = None;
                self.forget_samples();
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
                            // The crossing point is where the drag starts from,
                            // so the slop the finger spent getting here is not
                            // also scrolled — the same place the old code
                            // started subtracting from.
                            self.state = TouchState::Scrolling;
                            self.push_sample(now, x, y);
                        }
                    }
                    // **A move draws itself, the moment it arrives**, and it
                    // is worth saying why that plain sentence needed defending.
                    //
                    // Card K40 briefly did not do this. It held the samples
                    // back and let a once-per-frame resampler decide how far
                    // the content had got to, on the theory that the digitiser
                    // reported more slowly than the 120Hz loop consumed, so
                    // that half of all presented frames carried no new finger
                    // position and the list advanced on the digitiser's clock
                    // while the pictures advanced on the panel's. It is a real
                    // failure mode and Android's own `InputConsumer` resamples
                    // for exactly it. It is not this handset's.
                    //
                    // Measured instead of assumed, the theory did not survive.
                    // The moto g stylus 5G's digitiser reports every 4.21ms at
                    // the median — about 238Hz, twice the panel and not half of
                    // it — and Android normalises that stream to the display's
                    // rate before the app is handed anything, so a `Move`
                    // arrives per frame and never less often. The share of
                    // presented frames on which the list did not move was 0.0%
                    // at every touch rate tried, *before* any fix: the thing
                    // the resampler existed to remove does not occur here. (A
                    // later run, after the removal, saw a few per cent of still
                    // frames — but only because the app's own frame took 18ms
                    // rather than the panel's 8.3, so the loop occasionally
                    // turned twice inside one touch interval. That is a paint
                    // that is too slow, and no amount of resampling the finger
                    // makes a frame arrive.) What
                    // it did do was cost 12ms of deliberate lag — 4 to 13px of
                    // a 349px drag, a list that visibly trails the finger — and
                    // make the 60Hz case on the GPU path worse rather than
                    // better, 18.0% of per-frame variation becoming 36.5% with
                    // a growing three-frame beat. So it came out. See
                    // [`EventClock`] for the numbers and how they were taken;
                    // the velocity rewrite in [`VELOCITY_WINDOW`] is the half
                    // of that card which measured true and stayed.
                    //
                    // The delta is read off the newest sample rather than a
                    // `last_x` kept beside it, so there is exactly one record
                    // of where the finger was and the launch speed is computed
                    // from the same positions the drag drew.
                    TouchState::Scrolling => {
                        let last = self.newest_position();
                        self.push_sample(now, x, y);
                        if let Some((last_x, last_y)) = last {
                            let (ox, oy) = self.scroll_origin;
                            events.push(PlatformEvent::MouseWheel {
                                x: ox,
                                y: oy,
                                delta_x: (x - last_x) as f64,
                                delta_y: (y - last_y) as f64,
                            });
                        }
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
                    TouchState::Scrolling => {
                        // The launch speed, measured here and nowhere else,
                        // because this is the last instant at which the samples
                        // that describe the flick still exist — `forget_samples`
                        // below is what ends the gesture. See
                        // [`VELOCITY_WINDOW`].
                        let (vx, vy) = self.sample_velocity(now);
                        self.velocity_x = vx;
                        self.velocity_y = vy;

                        // End of scroll drag — momentum will be applied in
                        // tick(). No event: the document was told this gesture
                        // was no longer its own back when the scroll claimed it,
                        // and a lift adds nothing to that. Nor is there any
                        // travel left owing: every move drew itself as it
                        // arrived, so the picture is already where the finger
                        // is and the fling starts from there.
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
                self.forget_samples();
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
                    TouchState::Scrolling | TouchState::Idle => {}
                }
                self.state = TouchState::Idle;
                self.velocity_x = 0.0;
                self.velocity_y = 0.0;
                self.forget_samples();
            }
            TouchAction::HoverMove => {
                events.push(PlatformEvent::MouseMove { x, y });
            }
            TouchAction::Other => {}
        }
    }

    // ── The finger's recent history ──────────────────────────────────────────

    /// Forget where the finger was. Called wherever a gesture ends or begins,
    /// so that one drag's samples can never be measured against the next one's
    /// — two fingers on opposite sides of the screen, a second apart, would
    /// otherwise describe one very fast flick between them.
    fn forget_samples(&mut self) {
        self.samples = [None; SAMPLE_WINDOW];
        self.sample_count = 0;
    }

    /// Where the finger was when it was last reported, or `None` before the
    /// first sample of a drag.
    fn newest_position(&self) -> Option<(f32, f32)> {
        let (_, x, y) = self.samples[..self.sample_count].last().copied()??;
        Some((x, y))
    }

    /// Record where the finger is, keeping the most recent [`SAMPLE_WINDOW`]
    /// positions.
    ///
    /// The same-timestamp arm below is what keeps the window's timestamps
    /// strictly increasing, which is the invariant [`Self::sample_velocity`]
    /// divides by. Two samples the device stamped identically *replace* one
    /// another rather than stacking up, and nothing is lost by that — a sample
    /// is an absolute position, so the newer one already carries the older
    /// one's travel, and a wheel delta taken against it is the same distance
    /// either way. It is rare now that [`EventClock`] gives each event its own
    /// time; it was every batch back when they were all stamped with the
    /// instant the loop woke.
    fn push_sample(&mut self, now: Instant, x: f32, y: f32) {
        if let Some((t, _, _)) = self.samples[self.sample_count.saturating_sub(1)] {
            if t == now {
                self.samples[self.sample_count - 1] = Some((now, x, y));
                return;
            }
        }
        if self.sample_count == SAMPLE_WINDOW {
            self.samples.rotate_left(1);
            self.sample_count -= 1;
        }
        self.samples[self.sample_count] = Some((now, x, y));
        self.sample_count += 1;
    }

    /// How fast the finger was actually moving, in pixels per [`MOMENTUM_TICK`],
    /// from the real samples inside [`VELOCITY_WINDOW`].
    ///
    /// Zero when the newest sample is older than the window, which is a finger
    /// that has stopped being reported; zero as well when the window is full of
    /// samples that do not move, which is a finger that has stopped moving. The
    /// two cases arrive by different routes and want the same answer.
    fn sample_velocity(&self, now: Instant) -> (f32, f32) {
        let filled = &self.samples[..self.sample_count];
        let Some(&Some((newest_at, newest_x, newest_y))) = filled.last() else {
            return (0.0, 0.0);
        };
        if now.saturating_duration_since(newest_at) > VELOCITY_WINDOW {
            return (0.0, 0.0);
        }
        let Some(&(oldest_at, oldest_x, oldest_y)) = filled
            .iter()
            .flatten()
            .find(|(t, _, _)| newest_at.saturating_duration_since(*t) <= VELOCITY_WINDOW)
        else {
            return (0.0, 0.0);
        };
        let span = newest_at.saturating_duration_since(oldest_at).as_secs_f32();
        if span <= 0.0 {
            return (0.0, 0.0);
        }
        // A distance over a duration, said in the per-tick unit the fling is
        // tuned in — which is the whole correction. See [`VELOCITY_WINDOW`].
        let per_tick = MOMENTUM_TICK.as_secs_f32() / span;
        // Clamped, because a quotient is only as bounded as its denominator and
        // `span` comes from the device's own clock. See [`MAX_FLING_VELOCITY`]:
        // on a conforming device this never engages. Per axis, as Android's own
        // `computeCurrentVelocity(units, maxVelocity)` does.
        let ceiling = MAX_FLING_VELOCITY * MOMENTUM_TICK.as_secs_f32();
        (
            ((newest_x - oldest_x) * per_tick).clamp(-ceiling, ceiling),
            ((newest_y - oldest_y) * per_tick).clamp(-ceiling, ceiling),
        )
    }

    /// Fire the context event for a press that has been held still past
    /// [`LONG_PRESS_TIMEOUT`].
    ///
    /// Called once per event-loop iteration alongside [`Self::tick_momentum`].
    /// The loop turns at least once per displayed frame while a finger is down,
    /// so the menu opens within a frame of the deadline whether or not the
    /// finger is producing events. (It used to say "the loop polls on a 16ms
    /// timeout"; card K37 took that timeout away, and card K39 is the one that
    /// found out what else had been quietly resting on it.)
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
    ///
    /// **`now` is what card K39 added and why.** This used to advance the fling
    /// by exactly one step per call, which was correct for as long as the caller
    /// was a loop turning at a fixed 60Hz — and card K37 stopped it being one.
    /// The fling then ran at whatever rate the panel happened to offer: the same
    /// flick coasted for 1.4 seconds on a 60Hz screen and 0.7 on a 120Hz one,
    /// same distance, same number of frames, half the time. See
    /// [`MOMENTUM_TICK`] for the measurement.
    ///
    /// So the step is now `elapsed / MOMENTUM_TICK` rather than 1, the friction
    /// is raised to that power, and the wheel delta is multiplied by it. At
    /// exactly the reference tick that is exactly the arithmetic this replaced
    /// — one step, one multiplication by 0.95 — so the curve the constants
    /// describe is unchanged and only its independence from the frame rate is
    /// new. At 120Hz it is two half-steps per 60Hz frame, which take the same
    /// time and travel within about 1% of the same distance, but do it with
    /// twice as many pictures.
    ///
    /// **About 1%, not exactly.** The scheme emits `v * steps` and decays
    /// *after*, which is a left Riemann sum, so a finer sampling of the same
    /// curve integrates slightly under it: measured on the host, one flick
    /// coasts 656.7px at 60Hz, 648.8px at 120Hz and 643.0px at 480Hz — a 1.2%
    /// shortfall at 120Hz converging on 2.2%. The *duration* is unaffected,
    /// because the stopping condition counts decay steps and `steps` sums to
    /// the same total either way. Both are far inside the factor of two this
    /// replaces, and pinned at a 5% tolerance in
    /// `the_same_fling_lasts_the_same_time_at_any_refresh_rate`; the reason to
    /// write the number down rather than say "the same" is that a later reader
    /// measuring 648.8 against 656.7 should find it already accounted for.
    pub(crate) fn tick_momentum(&mut self, now: Instant, events: &mut Vec<PlatformEvent>) -> bool {
        if matches!(self.state, TouchState::Scrolling) {
            // Still touching — don't apply momentum
            self.last_momentum = None;
            return false;
        }
        if self.velocity_x.abs() < MOMENTUM_MIN_VELOCITY
            && self.velocity_y.abs() < MOMENTUM_MIN_VELOCITY
        {
            self.velocity_x = 0.0;
            self.velocity_y = 0.0;
            self.last_momentum = None;
            return false;
        }

        // How much of the fling this call is responsible for. The first call of
        // a fling has no previous tick to subtract, and charging it one whole
        // step is both the old behaviour and the honest one: the lift it
        // follows happened within the last frame, not at some knowable earlier
        // instant.
        let steps = match self.last_momentum {
            Some(last) => (now.saturating_duration_since(last).as_secs_f32()
                / MOMENTUM_TICK.as_secs_f32())
            .clamp(0.0, MOMENTUM_MAX_STEPS),
            None => 1.0,
        };
        self.last_momentum = Some(now);

        let (ox, oy) = self.scroll_origin;
        events.push(PlatformEvent::MouseWheel {
            x: ox,
            y: oy,
            delta_x: (self.velocity_x * steps) as f64,
            delta_y: (self.velocity_y * steps) as f64,
        });

        // `powf`, not a repeated multiply, because `steps` is fractional at any
        // refresh rate that is not the reference one — at 120Hz every tick is
        // half a step. Geometric decay is what makes that meaningful: 0.95^0.5
        // twice is 0.95 once, so the curve does not depend on how finely it is
        // sampled.
        let decay = MOMENTUM_FRICTION.powf(steps);
        self.velocity_x *= decay;
        self.velocity_y *= decay;
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

        /// A motion event *and* the turn of the loop that consumed it, in that
        /// order, which is how `collect_input_events` runs: the drain first,
        /// then the clocks.
        fn act(&mut self, ms: u64, action: TouchAction, x: f32, y: f32) {
            let now = self.t0 + Duration::from_millis(ms);
            self.gesture.process(action, x, y, now, &mut self.events);
            self.gesture.tick_long_press(now, &mut self.events);
            self.gesture.tick_momentum(now, &mut self.events);
        }

        /// One turn of the event loop with nothing in the queue, which is where
        /// both clocks are driven — same order as `collect_input_events`.
        fn tick(&mut self, ms: u64) {
            let now = self.t0 + Duration::from_millis(ms);
            self.gesture.tick_long_press(now, &mut self.events);
            self.gesture.tick_momentum(now, &mut self.events);
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
    /// however long it is then held, no context event may appear.
    ///
    /// **It also no longer flings, and that is the velocity rewrite rather than
    /// a regression.** The finger here moves 30px in 20ms and then holds
    /// perfectly still for three quarters of a second before lifting. The old
    /// velocity estimate was an average of per-*event* distances that only moved
    /// when an event arrived, so after the hold it still held the speed of a
    /// gesture that had been over for 760ms, and the list took off from under a
    /// stationary finger. A rate measured against the clock decays to nothing
    /// across that hold — which is what every native scroller does and what a
    /// person expects: you stop, then you let go, and nothing moves. The flick
    /// that *is* a flick is asserted by
    /// `the_same_fling_lasts_the_same_time_at_any_refresh_rate` and
    /// `a_fling_leaves_at_the_same_speed_whatever_the_touch_report_rate`.
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
            f.emitted().iter().filter(|e| *e == "cancel").count(),
            1,
            "the cancel hands the gesture to the scroll, exactly once"
        );
        assert!(
            !f.emitted()
                .iter()
                .any(|e| e.starts_with("down") || e.starts_with("up")),
            "and nothing else may be synthesised: {:?}",
            f.emitted()
        );

        // The finger crossed the slop at y=130 and stopped at y=160. Every
        // pixel of that 30 has to reach the list and no pixel more: a wheel
        // delta is the distance between two absolute positions, so travel can
        // be neither created nor destroyed on the way through, and that is the
        // property that makes this safe to put between a finger and a scroll
        // container.
        let travelled: f64 = f
            .events
            .iter()
            .filter_map(|e| match e {
                PlatformEvent::MouseWheel { delta_y, .. } => Some(*delta_y),
                _ => None,
            })
            .sum();
        assert!(
            (travelled - 30.0).abs() < 0.01,
            "30px of finger must be 30px of list, not {travelled:.2}px"
        );
        assert!(
            !f.gesture.has_momentum(),
            "a finger that stopped 760ms before it lifted has no flick left in it"
        );
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
        let before_lift = f.events.len();
        f.act(50, TouchAction::Up, 100.0, 170.0);

        assert_eq!(
            f.emitted().iter().filter(|e| *e == "cancel").count(),
            1,
            "exactly one cancel per gesture, however long the scroll ran"
        );
        assert!(
            !f.emitted()
                .iter()
                .any(|e| e.starts_with("down") || e.starts_with("up")),
            "and the lift adds nothing — the cancel was the whole announcement"
        );

        // The drag does not trail the finger, stated as a number so that
        // anything reintroducing a latency has to come past this line.
        //
        // The finger crossed the slop at y=112 and reached y=170 — 58px. All 58
        // are on the list by the time it lifts, because each move drew itself as
        // it arrived. Card K40's resampler drew the list where the finger had
        // been 12ms earlier, which left 36px of this drag unrendered at the lift
        // and made this assertion read 22.4px; on the handset the same lag was 4
        // to 13px of a 349px drag, and it was visible as a list that lagged
        // behind the fingertip. It bought nothing measurable in return, so it
        // came off. See the `Scrolling` arm of `process`.
        let dragged: f64 = f.events[..before_lift]
            .iter()
            .filter_map(|e| match e {
                PlatformEvent::MouseWheel { delta_y, .. } => Some(*delta_y),
                _ => None,
            })
            .sum();
        assert!(
            (dragged - 58.0).abs() < 0.1,
            "58px of finger must be 58px of list by the time it lifts, not \
             {dragged:.1}px"
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
    /// The fling, driven at one refresh rate and then at another, and the whole
    /// of card K39 in one assertion.
    ///
    /// Returns how far the list travelled and how long the coast lasted, given
    /// a loop that turns every `tick_ms`. Before K39 these two runs disagreed
    /// by a factor of two on the duration and agreed exactly on the distance,
    /// because the friction was applied once per *call* and a call was worth
    /// however long the panel felt like making it.
    fn fling(tick_ms: u64) -> (f64, u64) {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 100.0, 900.0);
        // Five moves of 40px, far enough past SCROLL_THRESHOLD to be a scroll
        // and fast enough to leave real velocity behind.
        for i in 1..=5 {
            f.act(i * 8, TouchAction::Move, 100.0, 900.0 - 40.0 * i as f32);
        }
        f.act(48, TouchAction::Up, 100.0, 700.0);

        let mut travelled = 0.0;
        let mut t = 48;
        let before = f.events.len();
        loop {
            t += tick_ms;
            f.tick(t);
            if !f.gesture.has_momentum() {
                break;
            }
            assert!(t < 20_000, "a fling that never settles is a hung list");
        }
        for e in &f.events[before..] {
            if let PlatformEvent::MouseWheel { delta_y, .. } = e {
                travelled += delta_y.abs();
            }
        }
        (travelled, t - 48)
    }

    /// A fling is a distance and a duration, and neither of them is the
    /// display's business.
    ///
    /// Card K37 paced the Android loop to the panel instead of to a hard-coded
    /// 16ms, which was right, and turned every fling in the framework into a
    /// function of the refresh rate, which was not. On the moto g stylus 5G the
    /// same flick of the library list coasted for 1.4 seconds at 60Hz and 0.7
    /// at 120Hz — same frames, same distance, half the time — and a list that
    /// stops twice as fast as the one in every other app on the phone is what
    /// "119fps that does not feel like 120" turned out to be made of.
    ///
    /// This is the laptop-side version of that measurement, in the shape cards
    /// K15 and K20 ask for: a thing found on hardware becomes a test that fails
    /// without one.
    #[test]
    fn the_same_fling_lasts_the_same_time_at_any_refresh_rate() {
        let (d60, t60) = fling(16);
        let (d120, t120) = fling(8);
        let (d30, t30) = fling(33);

        assert!(
            t120.abs_diff(t60) <= 24 && t30.abs_diff(t60) <= 40,
            "the coast must last the same wall-clock time at 60Hz ({t60}ms), \
             120Hz ({t120}ms) and 30Hz ({t30}ms) — before card K39 the 120Hz \
             run was half the 60Hz one"
        );
        let spread = (d120 - d60).abs().max((d30 - d60).abs()) / d60;
        assert!(
            spread < 0.05,
            "and it must cover the same distance: 60Hz {d60:.0}px, 120Hz \
             {d120:.0}px, 30Hz {d30:.0}px"
        );
        assert!(
            d60 > 100.0,
            "a flick that travels {d60:.0}px is not testing a fling"
        );
    }

    /// The clamp, which exists so that a stall does not become a teleport.
    ///
    /// A loop that missed 200ms — a sheet rasterising, an app returning from
    /// the background with a fling still in flight — owes twelve ticks. Paying
    /// all twelve in one frame moves the list a screenful between two pictures,
    /// which is worse than the stall it is compensating for.
    #[test]
    fn a_stalled_frame_pays_at_most_four_ticks_of_fling() {
        let mut f = Finger::new();
        f.act(0, TouchAction::Down, 100.0, 900.0);
        for i in 1..=5 {
            f.act(i * 8, TouchAction::Move, 100.0, 900.0 - 40.0 * i as f32);
        }
        f.act(48, TouchAction::Up, 100.0, 700.0);
        f.tick(64);
        let before = f.events.len();
        f.tick(1_064); // a full second of nothing
        let PlatformEvent::MouseWheel { delta_y, .. } = f.events[before] else {
            panic!("the stalled frame still owes one wheel event");
        };
        // The finger covered 40px every 8ms, which is 5000px/s, which is 83px
        // per 60Hz tick — the unit MOMENTUM_MAX_STEPS counts in. This number
        // used to be written as a flat 40, the per-*sample* distance, and the
        // two agreed only because the old velocity estimate confused the two
        // units (see VELOCITY_WINDOW). The clamp is the same clamp; only the unit
        // it is measured in has been corrected.
        let ceiling = 40.0 / 0.008 * MOMENTUM_TICK.as_secs_f64() * f64::from(MOMENTUM_MAX_STEPS);
        assert!(
            delta_y.abs() < ceiling + 1.0,
            "one frame after a one-second stall moved the list {delta_y:.0}px, \
             past the {ceiling:.0}px that four ticks of this fling are worth"
        );
    }

    // ── The launch speed, and the unit it is written in ──────────────────────
    //
    // What used to be here was a simulation of per-frame *smoothness*: a finger
    // at a constant speed, sampled at one rate and consumed at another, with the
    // per-frame content movement read out of the wheel events and asserted to be
    // even. It went with the resampler, and it went for the reason the resampler
    // did — the premise underneath it turned out not to describe this handset.
    // It assumed a digitiser slower than the loop; the moto g stylus 5G reports
    // at about 238Hz, twice the panel, normalised by Android to the panel's rate
    // before the app is handed anything. Measured on the device, the share of
    // presented frames on which the list did not move was 0.0% at every touch
    // rate, before any fix at all. A test whose premise is false does not become
    // true by passing. See [`EventClock`].
    //
    // What did survive the device is below: a flick is a speed, and a speed must
    // not depend on how often the digitiser was asked.

    /// A deterministic jitter source, so "a realistic touch stream" is the same
    /// stream on every run and a regression is a regression rather than a
    /// coincidence.
    struct Jitter(u64);

    impl Jitter {
        /// Signed, in [-1, 1].
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f64) / ((1u64 << 30) as f64) - 1.0
        }
    }

    /// One flick and the coast that follows it, simulated the way the shell
    /// actually runs: a finger moving at a constant speed, sampled by the
    /// digitiser at `touch_hz`, drained by a loop turning at `frame_hz`, lifted,
    /// and then left to settle. Returns how far the list travelled *after* the
    /// lift, which is the whole of what the launch speed decides.
    ///
    /// `jitter` is the fraction of a sample interval each sample may arrive
    /// early or late by. The finger's position is a function of the *real*
    /// instant it was sampled at, so a jittered sample is a true reading taken
    /// at an odd moment rather than a corrupted one — which is exactly the case
    /// a time-based velocity is supposed to be immune to, and exactly the case
    /// a per-event one is not.
    fn flick_distance(touch_hz: f64, frame_hz: f64, speed_px_s: f64, jitter: f64) -> f64 {
        const DRAG_SECONDS: f64 = 0.25;
        let t0 = Instant::now();
        let at = |secs: f64| t0 + Duration::from_nanos((secs * 1e9) as u64);

        let mut rng = Jitter(0x5eed_1234);
        let mut samples: Vec<(f64, f32)> = (0..(DRAG_SECONDS * touch_hz) as usize)
            .map(|k| {
                // A deliberate fraction of a sample interval, so the digitiser
                // and the loop are never accidentally in phase.
                let nominal = (0.37 + k as f64) / touch_hz;
                let t = (nominal + rng.next() * jitter / touch_hz).max(0.0);
                (t, (1000.0 - speed_px_s * t) as f32)
            })
            .collect();
        samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        let mut gesture = TouchGesture::new();
        let mut events = Vec::new();
        gesture.process(TouchAction::Down, 300.0, 1000.0, at(0.0), &mut events);

        // The drag. One turn of the loop per frame: drain everything that has
        // arrived, then run the clocks. Exactly `collect_input_events`.
        let mut next = 0usize;
        let mut frame = 0usize;
        while frame as f64 / frame_hz <= DRAG_SECONDS {
            let now = frame as f64 / frame_hz;
            while next < samples.len() && samples[next].0 <= now {
                // Stamped with the instant the digitiser sampled the finger,
                // which is what [`EventClock`] recovers from the `MotionEvent`
                // on a device, and not with the instant the loop woke up.
                gesture.process(
                    TouchAction::Move,
                    300.0,
                    samples[next].1,
                    at(samples[next].0),
                    &mut events,
                );
                next += 1;
            }
            gesture.tick_long_press(at(now), &mut events);
            gesture.tick_momentum(at(now), &mut events);
            frame += 1;
        }

        let lift = frame as f64 / frame_hz;
        let last_y = samples.last().expect("a flick has samples").1;
        gesture.process(TouchAction::Up, 300.0, last_y, at(lift), &mut events);

        // Only the coast is measured. The drag's own travel is the same distance
        // at every rate by construction — a wheel delta is the gap between two
        // absolute positions — and it is pinned in
        // `a_press_that_becomes_a_scroll_cancels_once_on_the_frame_it_crosses`.
        events.clear();
        let mut t = lift;
        loop {
            t += 1.0 / frame_hz;
            gesture.tick_momentum(at(t), &mut events);
            if !gesture.has_momentum() {
                break;
            }
            assert!(t - lift < 20.0, "a fling that never settles is a hung list");
        }
        events
            .iter()
            .filter_map(|e| match e {
                PlatformEvent::MouseWheel { delta_y, .. } => Some(delta_y.abs()),
                _ => None,
            })
            .sum()
    }

    /// **The correction the handset confirmed, in the shape cards K15 and K20
    /// ask for: a thing found on hardware becomes a test that fails without
    /// one.**
    ///
    /// The launch speed used to be an exponential average of per-*move-event*
    /// distances — `(x - last_x) * 0.8 + velocity * 0.2` — handed to the fling
    /// as though it were pixels per 60Hz tick. That is only true on a digitiser
    /// reporting at exactly 60Hz. Ask a faster one and every sample covers less
    /// ground, the average lands lower, and the same flick of the same list
    /// travels a shorter way for no reason a person could name.
    ///
    /// On the moto g stylus 5G, the same scripted flick injected at two report
    /// rates coasted 283px at 120Hz and 233px at 240Hz — 21% apart. Measured as
    /// a distance over a duration it coasts 596px and 594px, 0.3% apart. This is
    /// the laptop-side version of that: the same finger, at the same speed,
    /// reported at three rates, must throw the list the same distance.
    #[test]
    fn a_fling_leaves_at_the_same_speed_whatever_the_touch_report_rate() {
        const SPEED: f64 = 2000.0;
        for jitter in [0.0, 0.25] {
            let d60 = flick_distance(60.0, 120.0, SPEED, jitter);
            let d120 = flick_distance(120.0, 120.0, SPEED, jitter);
            let d240 = flick_distance(240.0, 120.0, SPEED, jitter);

            assert!(
                d120 > 100.0,
                "a flick that coasts {d120:.0}px is not testing a fling"
            );
            let spread = (d60 - d120).abs().max((d240 - d120).abs()) / d120;
            assert!(
                spread < 0.05,
                "jitter {jitter}: the same {SPEED}px/s flick coasted {d60:.0}px \
                 at a 60Hz digitiser, {d120:.0}px at 120Hz and {d240:.0}px at \
                 240Hz. A flick is a speed; the digitiser's polling interval is \
                 not part of it."
            );
        }
    }

    // ── The event clock ──────────────────────────────────────────────────────

    /// The property the launch speed actually rests on: whatever base the
    /// device counts in, two events 5ms apart in that base come out 5ms apart on
    /// the loop's clock. A speed is a distance divided by one of those gaps, so
    /// the gap is the part that has to be right.
    ///
    /// The absolute answer is deliberately not asserted, because [`EventClock`]
    /// deliberately does not know it. It anchors on one observation and measures
    /// everything else against that, which is what makes it correct without a
    /// claim about `CLOCK_MONOTONIC` that this file has no way to check.
    #[test]
    fn the_event_clock_preserves_the_spacing_between_samples() {
        let mut clock = EventClock::new();
        let t0 = Instant::now();
        // A base with no relationship to anything: the device is 40 seconds into
        // whatever it is counting.
        let base = 40_000_000_000i64;

        // One earlier turn of the loop, which is where the anchor comes from.
        assert_eq!(clock.instant_for(t0, base), t0);

        // A later turn carrying a batch of three samples 5ms apart, read 1ms
        // after the newest of them — the ordinary shape of a digitiser
        // reporting faster than the panel refreshes.
        let now = t0 + Duration::from_millis(21);
        let a = clock.instant_for(now, base + 10_000_000);
        let b = clock.instant_for(now, base + 15_000_000);
        let c = clock.instant_for(now, base + 20_000_000);

        assert_eq!(b.duration_since(a), Duration::from_millis(5));
        assert_eq!(c.duration_since(b), Duration::from_millis(5));
        assert_eq!(
            now.duration_since(c),
            Duration::from_millis(1),
            "and the newest of the batch keeps the latency it was read with"
        );
    }

    /// Input latency is never negative, so the event that maps closest to the
    /// instant it was read is the best evidence about the offset between the two
    /// clocks — and re-anchoring on it is how [`EventClock`] converges on the
    /// truth instead of inheriting whatever the first event happened to cost.
    ///
    /// Without this the first event's latency is added to every sample for the
    /// rest of the gesture. That is harmless to a *difference* between two
    /// samples, which is all the velocity needs — but it is not harmless to the
    /// staleness check at the lift, which compares the newest sample against the
    /// instant the loop is running at, and a constant offset there is a flick
    /// wrongly judged to have gone cold.
    #[test]
    fn the_event_clock_re_anchors_on_the_event_with_the_least_latency() {
        let mut clock = EventClock::new();
        let t0 = Instant::now();
        let base = 40_000_000_000i64;

        // The first event the loop sees is 20ms late — a slow first frame, a
        // scheduler hiccup. The anchor believes it was generated exactly now.
        let first = clock.instant_for(t0, base);
        assert_eq!(first, t0);

        // 20ms later the loop reads an event generated 20ms after the first,
        // i.e. one that arrived instantly. Under the original anchor it would
        // map to `t0 + 20ms`, which is exactly now — so this event proves it has
        // the smaller latency and becomes the anchor.
        let now = t0 + Duration::from_millis(20);
        let second = clock.instant_for(now, base + 20_000_000);
        assert_eq!(second, now);

        // And from here the offset is the better one: an event 5ms before this
        // one dates 5ms back, not 25ms back.
        let third = clock.instant_for(now, base + 15_000_000);
        assert_eq!(now.duration_since(third), Duration::from_millis(5));
    }

    /// An anchor that has stopped describing the offset between the two clocks
    /// must be abandoned, and the fallback is exactly the behaviour this type
    /// replaced: the instant the loop read the event.
    ///
    /// This is what makes a *stale* anchor self-healing — the app backgrounded
    /// mid-gesture, or the device's clock jumped — rather than poisoning the
    /// rest of the session.
    ///
    /// **It was called `..._refuses_a_timestamp_from_another_base`, and it never
    /// tested one.** What it drives is a backwards jump of a second within the
    /// *same* base, which is a different thing and the thing the code actually
    /// handles. A wrong *unit* cannot be detected here at all: [`EventClock`]
    /// compares only differences, so the epoch is invisible to it, and a
    /// coarser unit shrinks those differences rather than enlarging them. That
    /// case is real, it is not caught by any threshold, and it is pinned in
    /// `a_clock_running_in_the_wrong_unit_cannot_teleport_the_list` against the
    /// clamp that does bound it.
    #[test]
    fn the_event_clock_re_anchors_after_a_backwards_jump() {
        let mut clock = EventClock::new();
        let t0 = Instant::now();
        let base = 40_000_000_000i64;
        assert_eq!(clock.instant_for(t0, base), t0);

        let now = t0 + Duration::from_millis(8);
        // A whole second earlier — far outside MAX_EVENT_AGE, and far outside
        // any latency a touch event has ever had.
        assert_eq!(clock.instant_for(now, base - 1_000_000_000), now);
        // Having re-anchored, the clock is usable again immediately rather than
        // poisoned for the rest of the gesture.
        let next = clock.instant_for(now, base - 1_000_000_000 + 4_000_000);
        assert_eq!(next, now);
        let back = clock.instant_for(
            now + Duration::from_millis(8),
            base - 1_000_000_000 + 8_000_000,
        );
        assert_eq!(back.duration_since(now), Duration::from_millis(4));
    }

    // ── What the launch speed is measured across ─────────────────────────────
    //
    // The three tests above drive a finger at a *constant* speed, and that is a
    // fixed point: a first-to-last chord over a constant-velocity finger returns
    // the exact true speed for **any** choice of samples, however many and
    // however timed. It is what makes them good tests of the *unit* — they are
    // immune to the report rate and to jitter by construction, which is the
    // property under test — and it is also why they say nothing at all about
    // which samples the chord is taken across. Shrinking `VELOCITY_WINDOW` to
    // 20ms, cutting `SAMPLE_WINDOW` to 2, or dropping the horizon trim
    // altogether each leaves every one of them passing with the identical
    // number to four significant figures.
    //
    // So the two below move off that fixed point. Both take their expected
    // value from the finger rather than from the estimator: the speed asserted
    // is one the test *scripted*, not one the implementation reported.

    /// The launch speed is read across the window, so one bad sample cannot own
    /// it.
    ///
    /// [`SAMPLE_WINDOW`]'s doc argues that the window "does have to be more than
    /// two", because a speed read off the newest pair alone spans a single
    /// reporting interval and "that little travel is mostly the digitiser's own
    /// quantisation rather than the flick". That was an argument with nothing
    /// behind it — `SAMPLE_WINDOW = 2` passed the whole suite.
    ///
    /// Here it is as a measurement. A finger travelling at a known 2000px/s,
    /// reported at 120Hz, whose **last sample only** is displaced 6px by the
    /// digitiser. The true speed is 2000px/s because the test drove it there;
    /// how close the estimate lands is then a fact about the window:
    ///
    /// ```text
    ///   across 8 samples (58ms)     2103px/s     5% high   <- as shipped
    ///   across a 20ms window        2360px/s    18% high
    ///   across the newest pair      2720px/s    36% high
    /// ```
    #[test]
    fn one_noisy_sample_cannot_own_the_launch_speed() {
        const TRUE_SPEED: f32 = 2000.0;
        const REPORT: f64 = 1.0 / 120.0;
        let t0 = Instant::now();
        let at = |secs: f64| t0 + Duration::from_nanos((secs * 1e9) as u64);

        let mut g = TouchGesture::new();
        let mut ev = Vec::new();
        g.process(TouchAction::Down, 300.0, 1000.0, at(0.0), &mut ev);
        for k in 1..=8 {
            let t = k as f64 * REPORT;
            let mut y = 1000.0 - TRUE_SPEED * t as f32;
            if k == 8 {
                // The digitiser's own quantisation on the final report — the
                // one sample a two-sample estimate has no way to outvote.
                y -= 6.0;
            }
            g.process(TouchAction::Move, 300.0, y, at(t), &mut ev);
        }
        let lift = 8.0 * REPORT;
        g.process(
            TouchAction::Up,
            300.0,
            1000.0 - TRUE_SPEED * lift as f32 - 6.0,
            at(lift),
            &mut ev,
        );

        // The first momentum tick is charged exactly one step, so its wheel
        // delta *is* the launch speed in pixels per MOMENTUM_TICK.
        ev.clear();
        g.tick_momentum(at(lift + REPORT), &mut ev);
        let PlatformEvent::MouseWheel { delta_y, .. } = ev[0] else {
            panic!("the lift owes a fling");
        };
        let launched = delta_y.abs() as f32 / MOMENTUM_TICK.as_secs_f32();
        let error = (launched - TRUE_SPEED).abs() / TRUE_SPEED;
        assert!(
            error < 0.10,
            "a finger scripted at {TRUE_SPEED}px/s with one 6px blip on its last \
             report launched at {launched:.0}px/s ({:.0}% out). Read across the \
             newest pair alone that blip is a third of the answer; the window is \
             what outvotes it.",
            error * 100.0
        );
    }

    /// A burst of speed older than [`VELOCITY_WINDOW`] is not part of the flick.
    ///
    /// The horizon exists to bound how far back the chord reaches, and nothing
    /// exercised it: at the 120Hz the handset reports at, eight samples span
    /// 58ms and the 100ms horizon never trims anything, so `find(…)` and "the
    /// oldest stored sample" are the same sample. It only bites at a report rate
    /// slow enough for the ring to outrun the horizon.
    ///
    /// So: a 60Hz digitiser, where eight samples span 117ms. The finger jerks
    /// 100px in one interval and then travels at a steady 500px/s for the whole
    /// 100ms before it lifts. The jerk is outside the horizon, so the flick is a
    /// 500px/s flick. Reaching past the horizon instead reads 1286px/s and
    /// throws the list two and a half times as far.
    #[test]
    fn a_burst_older_than_the_horizon_is_not_part_of_the_flick() {
        const SETTLED_SPEED: f32 = 500.0;
        const REPORT: f64 = 1.0 / 60.0;
        let t0 = Instant::now();
        let at = |secs: f64| t0 + Duration::from_nanos((secs * 1e9) as u64);

        let mut g = TouchGesture::new();
        let mut ev = Vec::new();
        g.process(TouchAction::Down, 300.0, 1000.0, at(0.0), &mut ev);

        // k=1 crosses the slop and starts the drag; k=1 -> k=2 is the burst;
        // everything after it is the steady speed the flick is actually made of.
        let mut y = 990.0f32;
        g.process(TouchAction::Move, 300.0, y, at(REPORT), &mut ev);
        y -= 100.0;
        g.process(TouchAction::Move, 300.0, y, at(2.0 * REPORT), &mut ev);
        for k in 3..=8 {
            y -= SETTLED_SPEED * REPORT as f32;
            g.process(TouchAction::Move, 300.0, y, at(k as f64 * REPORT), &mut ev);
        }

        let lift = 8.0 * REPORT;
        g.process(TouchAction::Up, 300.0, y, at(lift), &mut ev);
        ev.clear();
        g.tick_momentum(at(lift + REPORT), &mut ev);
        let PlatformEvent::MouseWheel { delta_y, .. } = ev[0] else {
            panic!("the lift owes a fling");
        };
        let launched = delta_y.abs() as f32 / MOMENTUM_TICK.as_secs_f32();
        let error = (launched - SETTLED_SPEED).abs() / SETTLED_SPEED;
        assert!(
            error < 0.20,
            "the finger's speed for the whole {:?} before the lift was \
             {SETTLED_SPEED}px/s, and the 100px jerk before that is older than \
             the horizon — but it launched at {launched:.0}px/s.",
            VELOCITY_WINDOW
        );
    }

    /// A clock counting in the wrong unit costs a fast fling, not the whole list.
    ///
    /// The failure mode [`MAX_EVENT_AGE`] used to claim it caught. It does not
    /// catch it and no threshold could: [`EventClock`] compares only
    /// *differences*, so the epoch cannot reach it, and a coarser unit —
    /// milliseconds read as nanoseconds — makes those differences 10⁶ times
    /// smaller rather than larger. Every sample then lands within nanoseconds of
    /// its neighbour, the span the launch speed divides by collapses, and the
    /// quotient goes up by the same factor.
    ///
    /// Measured before [`MAX_FLING_VELOCITY`] was added, this exact gesture —
    /// 160px of finger over 64ms, which is a perfectly ordinary flick — asked
    /// for a single wheel event of **41,666,668px** and 823 million pixels of
    /// coast. The clamp is what turns "the list teleports to its end" into "the
    /// list flings fast", which is the same trade [`MOMENTUM_MAX_STEPS`] makes
    /// for a stalled frame.
    ///
    /// It is a guard against being wrong about a platform contract, not against
    /// anything observed: `MotionEvent::event_time()` is documented as
    /// `java.lang.System.nanoTime()` nanoseconds. Hence the deliberately loose
    /// bound — the assertion is "bounded at all", not a number to tune.
    #[test]
    fn a_clock_running_in_the_wrong_unit_cannot_teleport_the_list() {
        let mut clock = EventClock::new();
        let t0 = Instant::now();
        // The device counts in milliseconds since boot; we read it as nanoseconds.
        let ms_base = 40_000_000i64;
        let mut g = TouchGesture::new();
        let mut ev = Vec::new();
        for k in 0..=8u64 {
            let real_ms = k * 8;
            let sampled_at = clock.instant_for(
                t0 + Duration::from_millis(real_ms),
                ms_base + real_ms as i64,
            );
            let action = if k == 0 {
                TouchAction::Down
            } else {
                TouchAction::Move
            };
            g.process(action, 300.0, 1000.0 - 20.0 * k as f32, sampled_at, &mut ev);
        }
        let lift_at = clock.instant_for(t0 + Duration::from_millis(64), ms_base + 64);
        g.process(TouchAction::Up, 300.0, 840.0, lift_at, &mut ev);

        let ceiling = (MAX_FLING_VELOCITY * MOMENTUM_TICK.as_secs_f32()) as f64
            * f64::from(MOMENTUM_MAX_STEPS);
        ev.clear();
        let mut coast = 0.0f64;
        let mut t = 64u64;
        for _ in 0..5_000 {
            t += 8;
            g.tick_momentum(t0 + Duration::from_millis(t), &mut ev);
            for e in ev.drain(..) {
                if let PlatformEvent::MouseWheel { delta_y, .. } = e {
                    assert!(
                        delta_y.abs() <= ceiling + 1.0,
                        "one frame moved the list {:.0}px, past the {ceiling:.0}px \
                         that {MAX_FLING_VELOCITY}px/s of fling is worth. Before \
                         the clamp this gesture asked for 41,666,668px.",
                        delta_y.abs()
                    );
                    coast += delta_y.abs();
                }
            }
            if !g.has_momentum() {
                break;
            }
        }
        assert!(
            coast < 4_000.0,
            "the whole coast was {coast:.0}px — bounded per frame but not overall"
        );
    }
}
