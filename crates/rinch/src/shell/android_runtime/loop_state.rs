//! The state the Android loop keeps between turns, and the order one turn
//! does its work in.
//!
//! Each step of a turn is a method on [`LoopState`], in the module for its
//! area: the activity's lifecycle events in `main_events`, input in `input`,
//! the soft keyboard and the IME in `ime`, the text-selection toolbar in
//! `toolbar`, painting in `present`. [`LoopState::turn`] is where the order
//! between them is decided, and it is the order the comments on each step
//! depend on.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use android_activity::{AndroidApp, PollEvent};

use rinch_android::text_action::ToolbarMirror;
use rinch_core::events;
use rinch_platform::PlatformEvent;

#[cfg(feature = "android-gpu")]
use super::GpuContext;
use super::{DEFAULT_FRAME_INTERVAL, REDRAW_PENDING, WindowSurface, earliest_due, process_actions};
use crate::app::RinchApp;
use crate::shell::android_frame;
use crate::shell::android_ime::ImeComposition;
use crate::shell::android_key::KeyTranslator;
use crate::shell::android_key_status::KeyAnswers;
use crate::shell::touch_gesture::{EventClock, TouchGesture};

/// Everything the loop carries from one turn to the next.
///
/// The fields are in drop order, and three of them are placed for it: the
/// surface goes before the GPU context it was built from, and the app goes
/// last, as they did when these were locals of `run_loop` and `app` its
/// parameter.
pub(super) struct LoopState {
    /// The device outlives the window; the swapchain does not. That asymmetry
    /// is the whole lifecycle design of the GPU path and the reason these are
    /// two bindings rather than one — see [`GpuContext`] for what each half
    /// dies of. On the software path there is nothing to keep, so there is only
    /// `surface`.
    pub(super) surface: Option<WindowSurface>,
    #[cfg(feature = "android-gpu")]
    pub(super) gpu: Option<GpuContext>,
    pub(super) mounted: bool,
    pub(super) physical_size: (u32, u32),
    pub(super) scale_factor: f64,
    pub(super) running: bool,
    pub(super) gesture: TouchGesture,
    /// The finger's own clock, which is what makes a fling's launch speed a
    /// speed rather than a per-sample distance. Lives beside the recogniser and
    /// for the same reason: both are per-gesture state that has to survive
    /// between turns of the loop. See `touch_gesture::EventClock`.
    pub(super) event_clock: EventClock,
    /// Hardware-key state that outlives one event: the pending dead key and
    /// how each held key's press was spelled. See `android_key`.
    pub(super) keys: KeyTranslator,
    /// Which Back presses were handed back to the system, so their releases
    /// are too (#3062).
    pub(super) key_answers: KeyAnswers,
    /// The platform's floating text-selection toolbar, as the loop believes it
    /// to be (issue #813). See `ToolbarMirror` for why a belief and a count.
    pub(super) text_toolbar: ToolbarMirror,
    /// Whether the clipboard held anything when the toolbar was last asked
    /// for, which decides its Paste. Asked once per long press rather than per
    /// refresh: see `toolbar_items`.
    pub(super) clipboard_has_clip: bool,
    pub(super) keyboard_visible: bool,
    /// The kind of Enter key the keyboard is currently showing. Mirrors what
    /// the Java side was last told, so the input session is only restarted when
    /// the focused field's kind actually changes — see `ime::set_multiline`.
    pub(super) keyboard_multiline: bool,
    /// The IME's composing region, and the field it belongs to. See
    /// `shell::android_ime` for why the region is mirrored on this side at all.
    pub(super) composition: ImeComposition,
    pub(super) composing_field: Option<u64>,
    pub(super) backgrounded: bool,
    /// A window-focus change seen inside `poll_events`, dispatched after it
    /// returns (`app` is not reachable from the callback). `None` = no change
    /// this turn; the app starts focused, so nothing to send until Android says
    /// otherwise (issue #147).
    pub(super) window_focus_change: Option<bool>,
    /// How long one frame lasts on this panel, and the deadline the loop paces
    /// itself against. Replaced with the display's own answer at `InitWindow`;
    /// 60Hz until then, which is both the commonest panel and the rate the
    /// hard-coded 16ms this replaces was silently assuming.
    pub(super) frame_interval: Duration,
    /// When the display was last asked what a frame costs. See `RATE_RECHECK`.
    pub(super) frame_interval_read: Instant,
    /// When the current iteration's *work* began. Reset just after the poll
    /// returns, so the sleep is never counted against the frame it preceded.
    pub(super) frame_start: Instant,
    /// When to look again for Vello's counters of a frame the GPU has not
    /// finished (#2071). A third clock with no producer to ring the waker.
    pub(super) vello_check_in: Option<Duration>,
    /// Whether the previous iteration put pixels on the glass. This is the
    /// whole of the loop's pacing state — see `android_frame::poll_timeout` for
    /// why it is the right predicate and why it is deliberately the *narrow*
    /// one.
    pub(super) presented: bool,
    /// Whether Android told us, during the poll, that the device configuration
    /// moved under the app. Set in the `ConfigChanged` arm and acted on in
    /// [`LoopState::drain_platform_queues`], beside the other drains — see
    /// there for why it is deferred rather than dispatched where it arrives.
    pub(super) config_changed: bool,
    pub(super) app: RinchApp,
}

impl LoopState {
    pub(super) fn new(app: RinchApp) -> Self {
        Self {
            surface: None,
            #[cfg(feature = "android-gpu")]
            gpu: None,
            mounted: false,
            physical_size: (0, 0),
            scale_factor: 1.0,
            running: true,
            gesture: TouchGesture::new(),
            event_clock: EventClock::new(),
            keys: KeyTranslator::new(),
            key_answers: KeyAnswers::new(),
            text_toolbar: ToolbarMirror::new(),
            clipboard_has_clip: false,
            keyboard_visible: false,
            keyboard_multiline: false,
            composition: ImeComposition::new(),
            composing_field: None,
            backgrounded: false,
            window_focus_change: None,
            frame_interval: DEFAULT_FRAME_INTERVAL,
            frame_interval_read: Instant::now(),
            frame_start: Instant::now(),
            vello_check_in: None,
            presented: false,
            config_changed: false,
            app,
        }
    }

    /// One turn of the loop: sleep until there is something to do, then do
    /// it, in this order.
    ///
    /// Every early `return` is taken either with `running` already false — so
    /// the caller's `while running` ends the loop, as the `break`s this was
    /// written with did — or to wait for the next turn, as its `continue` did.
    pub(super) fn turn(&mut self, android_app: &AndroidApp) {
        self.poll(android_app);

        // The frame's own clock starts once the loop is awake and has work to
        // do, so `poll_timeout` is subtracting the time this iteration spent
        // *working* from the display's deadline and not the time it spent
        // asleep waiting to be given work.
        self.frame_start = Instant::now();
        // Nothing has reached the glass this iteration yet. Every path that
        // leaves before the present block in `paint` therefore falls back to
        // waiting for an event, which is the safe direction: a loop that waits
        // too long is woken by the waker or by the user's next touch, while a
        // loop that waits too little is a phone getting hot in a pocket.
        self.presented = false;

        if !self.running {
            return;
        }

        self.dispatch_window_focus();

        // No surface — wait for InitWindow. The composition is flushed first:
        // `MainEvent::Pause` and the `TerminateWindow` that drops the surface
        // can arrive in one `poll_events`, and skipping the flush here would
        // throw away the very half-typed word it exists to save — and leave
        // `backgrounded` latched until the next frame that has a surface,
        // which is after the resume.
        if self.surface.is_none() {
            self.flush_composition_if_backgrounded();
            return;
        }

        // Process touch / key input — must drain after poll_events returns
        let logical_size = rinch_platform::to_logical(self.physical_size, self.scale_factor);
        let input = self.dispatch_input(android_app);
        self.answer_toolbar_request(&input);
        self.sync_soft_keyboard();
        self.apply_ime_updates();

        // Backgrounding ends the IME session. Android finishes the composition
        // on the connection when it does, but this process is on its way to
        // being frozen and may not be scheduled to see it — so the composition
        // is committed here instead, and a half-typed word survives the way it
        // would in an `EditText`, where the composing characters are already
        // real text in the buffer. Emptying the mirror makes the framework's
        // own `finishComposingText` a no-op rather than a second insertion.
        self.flush_composition_if_backgrounded();

        self.perform_text_actions(input.toolbar_requested);
        self.refresh_text_toolbar(input.toolbar_requested);
        self.drain_platform_queues();

        // The frame clock. Every time-driven thing `RinchApp` owns — CSS
        // transitions, CSS animations, the dirty state the input handlers
        // leave for it to batch — advances here and nowhere else, so the clock
        // runs at whatever rate this loop iterates at. That used to be a flat
        // ~60Hz because the poll waited a flat 16ms; card K37 made it the
        // display's rate while something is moving and nothing at all while
        // nothing is. Before the surface is presented, not after: what it
        // resolves has to be in the pixels this iteration hands to
        // `present_pixels`, and the redraw it asks for has to be visible to
        // the swap in `paint`.
        let frame = android_frame::pump_frame(&mut self.app, self.physical_size, self.scale_factor);
        process_actions(&frame.actions, &mut self.running);
        if !self.running {
            return;
        }

        self.paint(&frame, logical_size);
    }

    /// Sleep until Android, the waker or one of the loop's own deadlines has
    /// something for it, handling each activity event as it is read.
    fn poll(&mut self, android_app: &AndroidApp) {
        let timeout = self.poll_timeout();
        android_app.poll_events(timeout, |event| match event {
            PollEvent::Main(main_event) => self.on_main_event(android_app, main_event),
            PollEvent::Wake => {}
            _ => {}
        });
    }

    fn poll_timeout(&self) -> Option<Duration> {
        // Read `REDRAW_PENDING` here, at the last possible instant before the
        // block, rather than reusing the swap in `paint`: anything that set it
        // after that swap — a cross-thread callback, an effect that ran during
        // the paint — would otherwise wait for an unrelated event.
        android_frame::poll_timeout(
            // Without one the bail in `turn` returns past every drain and past
            // the frame clock, so this iteration will do nothing — and the
            // `REDRAW_PENDING` swap that would clear the flag read on the next
            // line is itself below that bail, so a redraw requested while the
            // surface is gone can never clear. See `poll_timeout`.
            self.surface.is_some(),
            self.presented,
            REDRAW_PENDING.load(Ordering::Acquire),
            self.frame_start.elapsed(),
            self.frame_interval,
            // The two clocks with no producer to ring the waker: a
            // `poll_signal` bridge is sampled by `drain_polls` in
            // `drain_platform_queues`, and `drain_polls` only runs when this
            // loop runs — and a finger held still sends no event, so the
            // long-press deadline passes unobserved unless the loop comes back
            // for it (issue #813; see `TouchGesture::long_press_due`). See
            // `android_frame::poll_timeout`.
            earliest_due(
                earliest_due(
                    rinch_core::reactive::next_poll_due(),
                    self.gesture.long_press_due(Instant::now()),
                ),
                self.vello_check_in,
            ),
        )
    }

    /// Window focus, before input: a key that arrives in the same turn as
    /// the regain belongs to the refocused window. Dispatched *above* the
    /// no-surface bail in [`turn`](Self::turn) for the same reason the
    /// composition flush is: `MainEvent::LostFocus` and the `TerminateWindow`
    /// that drops the surface arrive together when the activity goes to the
    /// background, so deferring the loss to the next frame that has a surface
    /// would defer it past the regain — and this slot holds only the latest
    /// value, so the `GainedFocus` would overwrite it and the focused widget
    /// would never hear `on_focus_lost` at all. `handle_event` needs no
    /// surface.
    fn dispatch_window_focus(&mut self) {
        if let Some(focused) = self.window_focus_change.take() {
            let actions = self.app.handle_event(
                PlatformEvent::WindowFocus(focused),
                self.physical_size,
                self.scale_factor,
            );
            process_actions(&actions, &mut self.running);
        }
    }

    /// What arrived from the platform and from other threads since the last
    /// turn, other than input and the IME.
    fn drain_platform_queues(&mut self) {
        // Drain activity result callbacks (file picker, etc.)
        rinch_android::callback::drain_activity_results();
        rinch_android::callback::drain_permission_results();

        // Drain sensor, location, and lifecycle updates
        rinch_android::sensors::drain_sensor_events();
        rinch_android::location::drain_location();
        rinch_android::lifecycle::drain_lifecycle();

        // And tell the app if the device configuration moved. On the main
        // thread by construction — this is the loop body, which is the thread
        // that owns the signal store — which is the whole requirement, because
        // the handler's job is to write `Signal`s and `Signal::set` panics off
        // the main thread. `std::mem::take` so a handler that somehow causes
        // another `ConfigChanged` to be delivered cannot spin here.
        if std::mem::take(&mut self.config_changed) {
            events::dispatch_configuration_change();
        }

        // And intents the app was handed from outside: a share, a deep link.
        // Unlike the drains above this one keeps what it cannot deliver, so an
        // app that registers its handler a frame or two into startup still
        // receives the share that launched it. See `rinch_android::intent`.
        rinch_android::intent::drain_incoming_intents();

        // Drain cross-thread callbacks
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        // Then the work other threads sent the app itself — a plain `<input>`
        // paste's completion (issue #328). Android's clipboard answers inline,
        // so a toolbar Paste handled earlier in the turn lands here, in the
        // same iteration.
        if self.app.run_deferred_work() > 0 {
            REDRAW_PENDING.store(true, Ordering::Release);
        }
    }
}
