//! Android runtime using android-activity (NativeActivity), presenting
//! straight into the `ANativeWindow` the Activity owns.
//!
//! Bypasses winit entirely for direct control over the Android Activity
//! lifecycle, touch input, and surface management. Uses the same
//! platform-agnostic [`RinchApp`] as the desktop shell.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use android_activity::{AndroidApp, MainEvent, PollEvent};

use rinch_android::text_action::{TextAction, TextActionEvent, ToolbarMirror};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;
use rinch_platform::{AppAction, MouseButton, PlatformEvent};

use crate::app::{RinchApp, TextContextMenuPresentation, TextEditAction};
use crate::shell::android_frame;
use crate::shell::android_ime::{ImeAction, ImeComposition};
use crate::shell::android_key::KeyTranslator;
use crate::shell::android_key_status::KeyAnswers;
use crate::shell::touch_gesture::{EventClock, TouchGesture};

// ── Cross-thread dispatch ────────────────────────────────────────────────────

static REDRAW_PENDING: AtomicBool = AtomicBool::new(false);

/// One frame at 60Hz, used until the window exists and the display can be
/// asked what it actually runs at, and as the answer when it refuses to say.
///
/// 60Hz is the conservative guess in the direction that matters: guessing too
/// slow caps the app, guessing too fast only moves the wait from this loop's
/// timeout into the present's own back-pressure. It is also, exactly, what the
/// `Duration::from_millis(16)` this replaces was assuming without saying so.
const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

/// How often the loop re-asks the display what a frame costs, while it is
/// drawing them.
///
/// **This is not paranoia; it was measured happening.** Android varies the
/// panel's refresh rate to save power and does not tell an app about it. On the
/// moto g stylus 5G the shell logged `frame=8.33ms` on a cold start and
/// `frame=16.67ms` on the very next `InitWindow` after a press of Home and a
/// return — same app, same window, same panel, idled down to 60Hz in the second
/// or two the activity was away. Reading the rate once per window would have
/// left the app paced for 60fps on a screen willing to show 120 for the rest of
/// its life, which is the exact fault K37 exists to remove, arrived at by a
/// different road.
///
/// A second is short enough that nobody feels a mode change and long enough
/// that the JNI call is nothing — and it only happens while frames are being
/// presented, so an idle app makes no calls at all and the sleeping loop above
/// stays asleep.
const RATE_RECHECK: Duration = Duration::from_secs(1);

/// Queue a cross-thread closure and ask for a frame.
///
/// The queue itself lives in `rinch-core` so every host shares one
/// ([`rinch_core::queue_main_callback`], issue #172); what this shell adds is
/// the redraw request that gets the loop back around to drain it.
fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    rinch_core::queue_main_callback(f);
    REDRAW_PENDING.store(true, Ordering::Release);
    // The flag alone was enough while the loop polled with a 16ms timeout,
    // because a loop that wakes sixty-two times a second finds a flag within
    // sixteen milliseconds whether or not anyone told it to look. Since card
    // K37 the loop sleeps until something happens, and this is one of the
    // things that happen: `set_timeout`'s shared timer thread, an image that
    // finished decoding, and every `run_on_main_thread` in the process arrive
    // through here. Without the wake they arrive and wait for the user to
    // touch the screen.
    rinch_android::wake::wake_main();
}

// ── Entry points ─────────────────────────────────────────────────────────────

/// Mount `component` and take over the thread with the Android event loop.
///
/// The seam [`crate::App::run_android`] drives; it has already applied the
/// theme. Everything public goes through the builder, so there is exactly one
/// Android startup path.
pub(crate) fn run_component<F>(
    android_app: AndroidApp,
    component: F,
    fonts: &[crate::font::AppFont],
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    // Ahead of `run_loop`, which is what eventually calls `mount_component`:
    // the faces are in the font context before the first measurement, not
    // after it. This matters more here than on desktop — an Android device's
    // font list is whatever its OEM shipped, and the `monospace` slot in
    // particular is one fontique leaves empty (see the #322 repair in
    // `rinch_dom::fonts`).
    let mut app = RinchApp::new(component);
    app.register_app_fonts(fonts);
    // The window's size, scale and focus reach `rinch::window` (#1770).
    app.set_publishes_window_state(true);
    run_loop(android_app, app);
}

/// Run a rinch application on Android with default theme.
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).run_android(android_app)`"
)]
pub fn run<F>(android_app: AndroidApp, title: &str, _width: u32, _height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::App::new(component)
        .title(title)
        .run_android(android_app);
}

/// Run a rinch application on Android with theme configuration.
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).theme(theme).run_android(android_app)`"
)]
pub fn run_with_theme<F>(
    android_app: AndroidApp,
    title: &str,
    _width: u32,
    _height: u32,
    component: F,
    theme: ThemeProviderProps,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::App::new(component)
        .title(title)
        .theme(theme)
        .run_android(android_app);
}

// ── Main loop ────────────────────────────────────────────────────────────────

fn run_loop(android_app: AndroidApp, mut app: RinchApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("rinch"),
    );

    // Dispatcher first, as on desktop: a worker send between the two would
    // otherwise find a main thread registered and no dispatcher, and panic.
    rinch_core::set_cross_thread_dispatcher(dispatch_to_main_thread);
    rinch_core::register_main_thread();
    // No window role beyond an ordinary window on Android (#2659).
    crate::window_roles::publish(crate::window::WindowRoles::NONE);
    rinch_core::set_on_signal_change(|| {
        REDRAW_PENDING.store(true, Ordering::Release);
    });

    rinch_android::init(&android_app);

    // A context-menu gesture on a text field is answered with the platform's
    // floating text-selection toolbar rather than the runtime's DOM menu
    // (issue #813): `handle_event` prepares the caret and selection and hands
    // back `AppAction::ShowTextContextMenu`, and the loop below does the rest.
    app.set_text_context_menu_presentation(TextContextMenuPresentation::Shell);

    // #[cfg(feature = "android-gpu")]
    // gpu_diagnostic::run_tests();

    events::clear_handlers();
    rinch_core::clear_context();

    // The device outlives the window; the swapchain does not. That asymmetry
    // is the whole lifecycle design of the GPU path and the reason these are
    // two bindings rather than one — see [`GpuContext`] for what each half
    // dies of. On the software path there is nothing to keep, so there is only
    // `surface`.
    #[cfg(feature = "android-gpu")]
    let mut gpu: Option<GpuContext> = None;
    let mut surface: Option<WindowSurface> = None;
    let mut mounted = false;
    let mut physical_size = (0u32, 0u32);
    let mut scale_factor = 1.0f64;
    let mut running = true;
    let mut gesture = TouchGesture::new();
    // The finger's own clock, which is what makes a fling's launch speed a
    // speed rather than a per-sample distance. Lives beside the recogniser and
    // for the same reason: both are per-gesture state that has to survive
    // between turns of the loop. See `touch_gesture::EventClock`.
    let mut event_clock = EventClock::new();
    // Hardware-key state that outlives one event: the pending dead key and
    // how each held key's press was spelled. See `android_key`.
    let mut keys = KeyTranslator::new();
    // Which Back presses were handed back to the system, so their releases
    // are too (#3062).
    let mut key_answers = KeyAnswers::new();
    // The platform's floating text-selection toolbar, as the loop believes it
    // to be (issue #813). See `ToolbarMirror` for why a belief and a count.
    let mut text_toolbar = ToolbarMirror::new();
    // Whether the clipboard held anything when the toolbar was last asked
    // for, which decides its Paste. Asked once per long press rather than per
    // refresh: see `toolbar_items`.
    let mut clipboard_has_clip = false;
    let mut keyboard_visible = false;
    // The kind of Enter key the keyboard is currently showing. Mirrors what
    // the Java side was last told, so the input session is only restarted when
    // the focused field's kind actually changes — see `ime::set_multiline`.
    let mut keyboard_multiline = false;
    // The IME's composing region, and the field it belongs to. See
    // `shell::android_ime` for why the region is mirrored on this side at all.
    let mut composition = ImeComposition::new();
    let mut composing_field: Option<u64> = None;
    let mut backgrounded = false;
    // A window-focus change seen inside `poll_events`, dispatched after it
    // returns (`app` is not reachable from the callback). `None` = no change
    // this turn; the app starts focused, so nothing to send until Android says
    // otherwise (issue #147).
    let mut window_focus_change: Option<bool> = None;
    // How long one frame lasts on this panel, and the deadline the loop paces
    // itself against. Replaced with the display's own answer at `InitWindow`;
    // 60Hz until then, which is both the commonest panel and the rate the
    // hard-coded 16ms this replaces was silently assuming.
    let mut frame_interval = DEFAULT_FRAME_INTERVAL;
    // When the display was last asked what a frame costs. See `RATE_RECHECK`.
    let mut frame_interval_read = Instant::now();
    // When the current iteration's *work* began. Reset just after the poll
    // returns, so the sleep is never counted against the frame it preceded.
    let mut frame_start = Instant::now();
    // When to look again for Vello's counters of a frame the GPU has not
    // finished (#2071). A third clock with no producer to ring the waker.
    #[cfg_attr(not(feature = "android-gpu"), allow(unused_mut))]
    let mut vello_check_in: Option<Duration> = None;
    // Whether the previous iteration put pixels on the glass. This is the
    // whole of the loop's pacing state — see `android_frame::poll_timeout` for
    // why it is the right predicate and why it is deliberately the *narrow*
    // one.
    let mut presented = false;
    // Whether Android told us, during the poll below, that the device
    // configuration moved under the app. Set in the `ConfigChanged` arm and
    // acted on further down, beside the other drains — see there for why it is
    // deferred rather than dispatched where it arrives.
    let mut config_changed = false;

    while running {
        // Read `REDRAW_PENDING` here, at the last possible instant before the
        // block, rather than reusing the swap further down: anything that set
        // it after that swap — a cross-thread callback, an effect that ran
        // during the paint — would otherwise wait for an unrelated event.
        let timeout = android_frame::poll_timeout(
            // Without one the bail below `continue`s past every drain and past
            // the frame clock, so this iteration will do nothing — and the
            // `REDRAW_PENDING` swap that would clear the flag read on the next
            // line is itself below that bail, so a redraw requested while the
            // surface is gone can never clear. See `poll_timeout`.
            surface.is_some(),
            presented,
            REDRAW_PENDING.load(Ordering::Acquire),
            frame_start.elapsed(),
            frame_interval,
            // The two clocks with no producer to ring the waker: a
            // `poll_signal` bridge is sampled by `drain_polls` below, and
            // `drain_polls` only runs when this loop runs — and a finger held
            // still sends no event, so the long-press deadline passes
            // unobserved unless the loop comes back for it (issue #813; see
            // `TouchGesture::long_press_due`). See `android_frame::poll_timeout`.
            earliest_due(
                earliest_due(
                    rinch_core::reactive::next_poll_due(),
                    gesture.long_press_due(Instant::now()),
                ),
                vello_check_in,
            ),
        );
        android_app.poll_events(timeout, |event| match event {
            PollEvent::Main(main_event) => match main_event {
                MainEvent::InitWindow { .. } => {
                    if let Some(native_window) = android_app.native_window() {
                        let w = native_window.width() as u32;
                        let h = native_window.height() as u32;
                        physical_size = (w, h);

                        let dpi = rinch_android::display::density_dpi().unwrap_or_else(|| {
                            let config = android_app.config();
                            config.density().unwrap_or(160) as i32
                        }) as f64;
                        scale_factor = dpi / 160.0;

                        // What this panel calls a frame. Read here rather
                        // than at startup because there is no display to ask
                        // about until there is a window on it. It is *also*
                        // re-read once a second while frames are being drawn
                        // (see `RATE_RECHECK`), and it has to be, because the
                        // answer on this line is only true at this instant:
                        // the same handset answers 120Hz on a cold start and
                        // 60Hz on the `InitWindow` that follows a press of
                        // Home. See `display::refresh_rate_hz`.
                        frame_interval = rinch_android::display::refresh_rate_hz()
                            .map(|hz| Duration::from_secs_f32(1.0 / hz))
                            .unwrap_or(DEFAULT_FRAME_INTERVAL);
                        frame_interval_read = Instant::now();

                        log::info!(
                            "InitWindow: {}x{} physical, density={dpi}, scale={scale_factor:.2}, \
                             frame={:.2}ms",
                            w,
                            h,
                            frame_interval.as_secs_f64() * 1000.0
                        );

                        // The old surface is dropped *first*, and on its own
                        // line, because `surface = <new>` would build the new
                        // one while the old was still alive. On the software
                        // path that is merely wasteful; on the GPU path two
                        // swapchains on one `ANativeWindow` is
                        // `VK_ERROR_NATIVE_WINDOW_IN_USE_KHR`, and the window
                        // stays un-drawable for the rest of the process.
                        // `InitWindow` twice with no `TerminateWindow` between
                        // is not the documented sequence, but the sequence a
                        // phone actually delivers is not something this file
                        // gets to assume.
                        surface = None;

                        #[cfg(not(feature = "android-gpu"))]
                        {
                            surface = Some(SoftSurface::new(&native_window, w, h));
                        }
                        #[cfg(feature = "android-gpu")]
                        {
                            surface = GpuContext::attach(&mut gpu, &native_window, w, h);
                        }

                        // Push the display scale into Stylo (issue #211).
                        // Before the mount, so the initial style resolution
                        // sees the right `resolution` media features; on a
                        // re-`InitWindow` with a changed density it restyles
                        // the live document (no-op when unchanged).
                        //
                        // The native window's own size
                        // (`window_physical_size`) first: the scale is
                        // published with it, in one batch, on a re-`InitWindow`
                        // as at the mount.
                        app.set_surface_size(physical_size);
                        app.set_device_pixel_ratio(scale_factor);
                        if !mounted {
                            let (lw, lh) = rinch_platform::to_logical((w, h), scale_factor);
                            app.set_text_scale(scale_factor as f32);
                            app.mount_component(lw as f32, lh as f32);
                            mounted = true;
                        }

                        REDRAW_PENDING.store(true, Ordering::Release);
                    }
                }
                MainEvent::TerminateWindow { .. } => {
                    // Everything that can touch the window dies here, inside
                    // the callback, before `poll_events` returns.
                    //
                    // That timing is not a nicety. `android-activity` delivers
                    // this event from `onNativeWindowDestroyed`, and the
                    // Android thread that raised it is *blocked* until this
                    // loop has acknowledged it — so a swapchain dropped here
                    // is a swapchain destroyed while the `ANativeWindow` is
                    // still alive, which is the only order Vulkan accepts. A
                    // swapchain dropped one iteration later would be destroyed
                    // against a window the system had already freed.
                    //
                    // And it is the *only* way to present, which is what makes
                    // "render into a dead surface" unreachable rather than
                    // unlikely: the sole `GpuSurface` in the process is this
                    // binding, the paint block below cannot run without it,
                    // and this line empties it before the paint block is
                    // reached. There is no second handle to go stale.
                    //
                    // `gpu` — the device, the queue and vello's compiled
                    // pipelines — deliberately survives. Pressing Home
                    // destroys the window and nothing else; rebuilding the
                    // renderer on the way back would cost the better part of a
                    // second of black screen for a device Vulkan never
                    // invalidated.
                    surface = None;
                }
                MainEvent::WindowResized { .. } => {
                    if let Some(native_window) = android_app.native_window() {
                        let w = native_window.width() as u32;
                        let h = native_window.height() as u32;
                        physical_size = (w, h);

                        let (lw, lh) = rinch_platform::to_logical((w, h), scale_factor);
                        // Published with the `Resized` (`window_physical_size`).
                        app.set_surface_size(physical_size);

                        // The `Resized` payload is the *logical* viewport layout
                        // is resolved at; `handle_event`'s `window_size` is the
                        // *physical* surface size it derives that viewport from
                        // (see `RinchApp::layout_viewport`). Passing the logical
                        // size for both divided by the scale factor twice.
                        let actions = app.handle_event(
                            PlatformEvent::Resized {
                                width: lw,
                                height: lh,
                            },
                            physical_size,
                            scale_factor,
                        );
                        process_actions(&actions, &mut running);

                        if let Some(ref mut s) = surface {
                            s.resize(w, h);
                        }

                        REDRAW_PENDING.store(true, Ordering::Release);
                    }
                }
                MainEvent::ConfigChanged { .. } => {
                    // Everything a rinch app reads about the device — the
                    // night-mode flag, the accent colour off the wallpaper,
                    // the safe-area insets, the font scale, the display
                    // density — it reads once, at mount, because until this
                    // arm existed there was no moment at which it could
                    // sensibly be re-read. So an app's idea of the platform
                    // was fixed at the instant it started and every one of
                    // those readings quietly rotted: flip the system theme
                    // with the app in the foreground and it stayed light.
                    //
                    // Deferred rather than dispatched here, and the reason is
                    // the same one `MainEvent::Pause` defers through
                    // `lifecycle::notify_paused`. This closure is running
                    // inside `poll_events`, which `android-activity` invokes
                    // with the Java thread that raised the event *blocked*
                    // waiting for it to return, and the handler on the other
                    // end of this is arbitrary app code: it will write
                    // `Signal`s, it may kick off a re-render, and card K32's
                    // keyboard work is a standing reminder that app code
                    // reached from a platform callback likes to call back
                    // into the platform. Holding a Java thread hostage for
                    // that is a deadlock waiting for the one app that does it.
                    //
                    // Deferring also coalesces, and the `bool` rather than a
                    // counter is the point: a handler's job is to re-read the
                    // platform, and re-reading it twice for two events that
                    // arrived in the same poll gets the same answer for a
                    // second round of JNI calls. Measured on the moto g stylus
                    // 5G, one `cmd uimode night yes` raises this exactly once,
                    // so today the coalescing costs nothing and buys nothing —
                    // but a rotation or a font-scale change is a configuration
                    // change *and* a resize, and a handset that decides to
                    // split one of those in two is not something this file
                    // gets to assume it will never meet.
                    config_changed = true;
                    // The whole point of the event is that what should be on
                    // the glass has changed, so ask for a frame. The handler's
                    // signal writes would normally schedule this themselves;
                    // asking here means an app that re-reads the platform
                    // *without* writing a signal — one that only calls
                    // `set_light_system_bars`, say — still repaints against
                    // the new configuration.
                    REDRAW_PENDING.store(true, Ordering::Release);
                }
                MainEvent::Resume { .. } => {
                    rinch_android::lifecycle::notify_resumed();
                }
                MainEvent::Pause => {
                    rinch_android::lifecycle::notify_paused();
                    // Flushed below, where the viewport this loop dispatches
                    // against is in scope.
                    backgrounded = true;
                }
                // Android's answer to `PlatformEvent::WindowFocus` (issue
                // #147): the activity keeps its in-document focus claim across
                // a focus loss — a notification shade, a permission dialog —
                // and the focused widget is told, then told again when focus
                // returns. Distinct from `Resume`/`Pause`, which are lifecycle:
                // an activity can be resumed but unfocused.
                MainEvent::GainedFocus => {
                    window_focus_change = Some(true);
                }
                MainEvent::LostFocus => {
                    window_focus_change = Some(false);
                }
                MainEvent::Destroy => {
                    running = false;
                }
                _ => {}
            },
            PollEvent::Wake => {}
            _ => {}
        });

        // The frame's own clock starts once the loop is awake and has work to
        // do, so `poll_timeout` is subtracting the time this iteration spent
        // *working* from the display's deadline and not the time it spent
        // asleep waiting to be given work.
        frame_start = Instant::now();
        // Nothing has reached the glass this iteration yet. Every path that
        // leaves before the present block below therefore falls back to
        // waiting for an event, which is the safe direction: a loop that waits
        // too long is woken by the waker or by the user's next touch, while a
        // loop that waits too little is a phone getting hot in a pocket.
        presented = false;

        if !running {
            break;
        }

        // Window focus, before input: a key that arrives in the same turn as
        // the regain belongs to the refocused window. Dispatched *above* the
        // no-surface bail below for the same reason the composition flush is:
        // `MainEvent::LostFocus` and the `TerminateWindow` that drops the
        // surface arrive together when the activity goes to the background, so
        // deferring the loss to the next frame that has a surface would defer
        // it past the regain — and this slot holds only the latest value, so
        // the `GainedFocus` would overwrite it and the focused widget would
        // never hear `on_focus_lost` at all. `handle_event` needs no surface.
        if let Some(focused) = window_focus_change.take() {
            let actions = app.handle_event(
                PlatformEvent::WindowFocus(focused),
                physical_size,
                scale_factor,
            );
            process_actions(&actions, &mut running);
        }

        // No surface — wait for InitWindow. The composition is flushed first:
        // `MainEvent::Pause` and the `TerminateWindow` that drops the surface
        // can arrive in one `poll_events`, and skipping the flush here would
        // throw away the very half-typed word it exists to save — and leave
        // `backgrounded` latched until the next frame that has a surface,
        // which is after the resume.
        if surface.is_none() {
            if std::mem::take(&mut backgrounded) {
                for action in composition.finish_composing_text() {
                    apply_ime_action(&mut app, action, physical_size, scale_factor, &mut running);
                }
            }
            continue;
        }

        // Process touch / key input — must drain after poll_events returns
        let logical_size = rinch_platform::to_logical(physical_size, scale_factor);
        // Each event is dispatched as it is read: a key's answer to the system
        // (handled, or handed back) is what the app made of it (#3062).
        let mut input_events = Vec::new();
        let mut toolbar_requested = false;
        input::drain_input_events(
            &android_app,
            &mut gesture,
            &mut event_clock,
            scale_factor,
            Instant::now(),
            &mut keys,
            &mut key_answers,
            &mut |event| {
                let actions = app.handle_event(event.clone(), physical_size, scale_factor);
                toolbar_requested |= actions
                    .iter()
                    .any(|a| matches!(a, AppAction::ShowTextContextMenu));
                process_actions(&actions, &mut running);
                input_events.push(event);
                app.key_down_unused()
            },
        );

        // A long press on a text field asks for the platform's floating
        // text-selection toolbar (issue #813). The press itself — a right
        // button, from `tick_long_press` — was dispatched just above under
        // `TextContextMenuPresentation::Shell`, so the field it landed on
        // holds the input claim and its caret is placed by the seam's caret
        // rule; the toolbar floats beside the state the seam reports. Any
        // other press, a key and a scroll take an open toolbar down, as they
        // do over a platform text view.
        let pressed_elsewhere = input_events.iter().any(|e| {
            matches!(
                e,
                PlatformEvent::MouseDown {
                    button: MouseButton::Left,
                    ..
                } | PlatformEvent::KeyDown { .. }
                    | PlatformEvent::MouseWheel { .. }
            )
        });
        if toolbar_requested {
            // The platform convention: a long press selects the word under
            // the finger (a press inside an existing selection keeps it), and
            // an empty field — or a press on whitespace — keeps its caret and
            // offers only Paste.
            app.select_word_at_caret();
            clipboard_has_clip = rinch_android::clipboard::has_text();
            if let Some(state) = app.text_edit_state() {
                push_text_toolbar(
                    &mut text_toolbar,
                    &state,
                    scale_factor,
                    clipboard_has_clip,
                    true,
                );
            }
        } else if pressed_elsewhere && text_toolbar.finish() {
            rinch_android::text_action::finish_toolbar();
        }

        // What Enter means in the focused field, pushed before the keyboard is
        // raised so the session it opens already draws the right key. A
        // `<textarea>` wants a newline key; everything else wants what it had.
        // Android only reads this when a session starts, and rinch's own
        // field-to-field moves are invisible to it, so the change has to be
        // announced here or not at all.
        let needs_multiline = app.focused_input_is_multiline();
        if needs_multiline != keyboard_multiline
            && rinch_android::ime::set_multiline(needs_multiline)
        {
            // Advance the mirror only for a push that landed: a dropped one
            // would leave it claiming a kind the keyboard was never told
            // about, and nothing else ever pushes this. A failure retries on
            // the next turn of the loop.
            keyboard_multiline = needs_multiline;
        }

        // Show/hide soft keyboard based on input focus
        let needs_keyboard = app.has_focused_input() || app.has_focused_contenteditable();
        if needs_keyboard != keyboard_visible {
            keyboard_visible = needs_keyboard;
            if needs_keyboard {
                rinch_android::ime::show_keyboard();
            } else {
                rinch_android::ime::hide_keyboard();
            }
        }

        // What the IME did since the last frame, in the order it did it. Taken
        // *before* the focus check below, which needs to be able to throw the
        // batch away: every call in it was made against the field that was
        // focused when the keyboard made it.
        let mut updates = rinch_android::ime::drain_updates();

        // The focused field changed while the IME was composing into the old
        // one. The focus arbiter has already committed that composition into
        // the field that lost focus (`set_focus_target_deferred`'s
        // compositionend-before-blur), so nothing is inserted here — but the
        // keyboard still believes it is composing, and would deliver the same
        // word again into the field that just gained focus. Restarting its
        // input session is the only way to say so; Android cannot see the move
        // itself, because one `RinchInputView` holds focus throughout.
        let focused_field = app.focused_input_node();
        if focused_field != composing_field {
            composing_field = focused_field;
            if composition.abandon() {
                // Emptying the mirror is not enough on its own: the touch that
                // moved focus was dispatched earlier in *this* iteration, so
                // anything the keyboard queued before it is still in hand and
                // would now be applied to the field that just gained focus — a
                // `setComposingText` re-opening the region there, and the
                // `finishComposingText` that `restart_input` provokes then
                // inserting the same word a second time, into the wrong field.
                // The whole batch belongs to the session that just ended.
                updates.clear();
                rinch_android::ime::restart_input();
            }
        }

        // `android_ime` turns the `InputConnection` call stream into the
        // actions below; it holds the composing region and every rule about
        // when one ends.
        for update in updates {
            let actions = match update {
                rinch_android::ime::ImeUpdate::SetComposingText {
                    text,
                    new_cursor_position,
                } => composition.set_composing_text(text, new_cursor_position),
                rinch_android::ime::ImeUpdate::FinishComposingText => {
                    composition.finish_composing_text()
                }
                rinch_android::ime::ImeUpdate::CommitText(text) => composition.commit_text(text),
                rinch_android::ime::ImeUpdate::DeleteSurroundingText { before, after } => {
                    composition.delete_surrounding_text(before, after)
                }
            };
            // An edit from the soft keyboard takes the toolbar down too:
            // typing collapses the selection it was showing for. Judged by
            // what the IME *did*, not by whether it called: a focus move
            // restarts the input session, the keyboard answers with a
            // `finishComposingText` that composes nothing, and a toolbar
            // that just went up for that very press must survive it.
            let edited = actions.iter().any(|a| match a {
                ImeAction::Insert(text) => !text.is_empty(),
                ImeAction::Preedit { text, .. } => !text.is_empty(),
                ImeAction::Delete { .. } => true,
            });
            if edited && text_toolbar.finish() {
                rinch_android::text_action::finish_toolbar();
            }
            for action in actions {
                apply_ime_action(&mut app, action, physical_size, scale_factor, &mut running);
            }
        }

        // Backgrounding ends the IME session. Android finishes the composition
        // on the connection when it does, but this process is on its way to
        // being frozen and may not be scheduled to see it — so the composition
        // is committed here instead, and a half-typed word survives the way it
        // would in an `EditText`, where the composing characters are already
        // real text in the buffer. Emptying the mirror makes the framework's
        // own `finishComposingText` a no-op rather than a second insertion.
        if std::mem::take(&mut backgrounded) {
            for action in composition.finish_composing_text() {
                apply_ime_action(&mut app, action, physical_size, scale_factor, &mut running);
            }
        }

        // What the platform did with the text actions since the last turn: a
        // toolbar item tapped, an IME's own `performContextMenuAction`, or the
        // toolbar going away. Performed here, on the loop, through the seam —
        // which runs exactly the code the keyboard shortcut runs (issue #813).
        for event in rinch_android::text_action::drain_text_action_events() {
            match event {
                TextActionEvent::ToolbarDismissed => text_toolbar.dismissed(),
                TextActionEvent::Perform { action, source } => {
                    log::info!("text action: {action:?} from {source:?}");
                    // Cut, Copy and Paste end the toolbar's interaction, from
                    // either source. A toolbar item has finished the mode on
                    // the Java side already — before reporting the item, so
                    // its dismissal is normally drained by now — and the
                    // mirror only has to stop believing in it, or the refresh
                    // below would re-prepare it and, on the UI thread, start a
                    // new one (PR #819 review, F1). An IME request finished
                    // nothing, so the loop asks. Not when a fresh toolbar was
                    // requested this very turn: that one is newer than
                    // anything this drain holds.
                    if !toolbar_requested && text_toolbar.performing(action, source) {
                        rinch_android::text_action::finish_toolbar();
                    }
                    let actions = app.perform_text_edit(match action {
                        TextAction::Cut => TextEditAction::Cut,
                        TextAction::Copy => TextEditAction::Copy,
                        TextAction::Paste => TextEditAction::Paste,
                        TextAction::SelectAll => TextEditAction::SelectAll,
                    });
                    process_actions(&actions, &mut running);
                }
            }
        }

        // Keep an open toolbar current: the field's state is re-read every
        // turn, so Cut and Copy appear the moment Select all made a selection,
        // the toolbar follows a caret the user moved, and it goes when the
        // field loses the keyboard. `ToolbarMirror` makes this cost nothing
        // while nothing changed. A refresh never starts a toolbar (see
        // `push_text_toolbar`). Skipped on a long-press turn, which pushed the
        // state it just read.
        if text_toolbar.is_shown() && !toolbar_requested {
            match app.text_edit_state() {
                Some(state) => push_text_toolbar(
                    &mut text_toolbar,
                    &state,
                    scale_factor,
                    clipboard_has_clip,
                    false,
                ),
                None => {
                    if text_toolbar.finish() {
                        rinch_android::text_action::finish_toolbar();
                    }
                }
            }
        }

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
        if std::mem::take(&mut config_changed) {
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
        // so a toolbar Paste handled above lands here, in the same iteration.
        if app.run_deferred_work() > 0 {
            REDRAW_PENDING.store(true, Ordering::Release);
        }

        // The frame clock. Every time-driven thing `RinchApp` owns — CSS
        // transitions, CSS animations, the dirty state the input handlers
        // leave for it to batch — advances here and nowhere else, so the clock
        // runs at whatever rate this loop iterates at. That used to be a flat
        // ~60Hz because the poll above waited a flat 16ms; card K37 made it
        // the display's rate while something is moving and nothing at all
        // while nothing is. Before the surface is presented, not after: what
        // it resolves has to be in the pixels this iteration hands to
        // `present_pixels`, and the redraw it asks for has to be visible to
        // the swap below.
        let frame = android_frame::pump_frame(&mut app, physical_size, scale_factor);
        process_actions(&frame.actions, &mut running);
        if !running {
            break;
        }

        // Check if app has pending layout (signal changes create pending updates)
        let has_momentum = gesture.has_momentum();
        let redraw = REDRAW_PENDING.swap(false, Ordering::AcqRel);
        // `pending_layout` also covers a finished image decode, which has
        // dirtied no node (`RinchApp::has_pending_images`) — without that this
        // loop spins past the decode forever and the `<img>` paints as a blank
        // card. Found on a moto g stylus showing a rasterised PDF page out of
        // app-private storage: the page was on disk, the box was the right
        // size, and the pixels only arrived when something else happened to
        // make the screen redraw.
        let pending = frame.pending_layout;
        let needs_paint = redraw || pending || has_momentum || frame.needs_paint;
        // Whether this iteration is going to try to put pixels on the glass.
        // Named because the present's *outcome* is only meaningful beside it —
        // see the `retry_owed_frame` call below the block.
        let attempted = needs_paint && mounted;

        if attempted {
            // Only on frames that are actually being drawn, and at most once a
            // second — see `RATE_RECHECK` for the mode change that made this
            // necessary.
            if frame_interval_read.elapsed() >= RATE_RECHECK {
                frame_interval_read = Instant::now();
                if let Some(hz) = rinch_android::display::refresh_rate_hz() {
                    frame_interval = Duration::from_secs_f32(1.0 / hz);
                }
            }

            if pending {
                app.resolve_and_repaint(logical_size.0 as f32, logical_size.1 as f32);
            }

            #[cfg(feature = "android-gpu")]
            if let (Some(ctx), Some(s)) = (&mut gpu, &mut surface) {
                let scene = app.build_scene(scale_factor, logical_size);
                let present_start = std::time::Instant::now();
                presented = s.present_scene(&mut ctx.renderer, scene);
                app.perf_add(
                    rinch_dom::perf::Counter::TimePresentNs,
                    present_start.elapsed().as_nanos() as u64,
                );
            }

            #[cfg(not(feature = "android-gpu"))]
            if let Some(ref mut s) = surface {
                let (pixels, w, h) = app.build_pixels(scale_factor, logical_size, false);
                let present_start = std::time::Instant::now();
                presented = s.present_pixels(pixels, w, h);
                app.perf_add(
                    rinch_dom::perf::Counter::TimePresentNs,
                    present_start.elapsed().as_nanos() as u64,
                );
            }

            // One owed frame is one performance frame (`rinch_dom::perf`),
            // as one redraw is on the desktop; this is also what prints the
            // `RINCH_PERF` line on Android.
            app.end_perf_frame();
        }

        // A frame that overflowed Vello's buffers went out undrawn, and nothing
        // else would draw it again on a screen at rest (#2071). This path
        // renders the whole scene every time, so a redraw is all it takes.
        #[cfg(feature = "android-gpu")]
        if let (Some(ctx), Some(s)) = (&mut gpu, &surface) {
            vello_check_in = None;
            match ctx.renderer.check_frames(&s.device) {
                vello::FrameCheck::Settled => {}
                vello::FrameCheck::Pending => {
                    vello_check_in = Some(VELLO_CHECK_INTERVAL);
                }
                vello::FrameCheck::Lost => REDRAW_PENDING.store(true, Ordering::Release),
            }
        }

        // The frame was owed and did not reach the glass, so ask for it again
        // (issue #563). Nothing else will: `redraw` was swapped out of
        // `REDRAW_PENDING` above, the layout `pending` named was resolved by
        // the `resolve_and_repaint` just inside the block, `build_scene` /
        // `build_pixels` clear `scene_dirty` *before* the present, and a
        // running CSS transition marks its node dirty without ever queueing a
        // redraw *action* — so the loop's next `poll_timeout`
        // would see `presented == false` with no wake pending, answer `None`,
        // and sleep until something unrelated happened to arrive. The re-arm
        // costs one display frame of sleep and clears itself on the first
        // success; see `android_frame::retry_owed_frame` for the sequence.
        //
        // **The clock reset is half the repair, not a tidy-up.** `poll_timeout`
        // answers `frame_interval - spent`, and the `spent` it is handed is
        // `frame_start.elapsed()` — this whole iteration, paint included. A
        // paint that overran the display's frame (85.2ms against an 8.33ms
        // panel in K37's own software `sheet` row) saturates that to
        // `Duration::ZERO`, so without this line the retry is immediate and the
        // loop spins at whatever rate the CPU allows, failing every time: the
        // exact busy-wait the `bool` returns of `present_pixels` /
        // `present_scene` exist to prevent, reintroduced by the cure. Restart
        // the clock here and the retry is measured from the arm instead, so it
        // gets a whole frame. A frame that *reached the glass* still gets
        // `ZERO` when it overran, and should — that is catching up, not failing
        // faster.
        if android_frame::retry_owed_frame(attempted, presented) {
            REDRAW_PENDING.store(true, Ordering::Release);
            frame_start = Instant::now();
        }
    }
}

/// How soon the loop looks again for Vello's counters of a frame the GPU has
/// not finished (#2071).
#[cfg(feature = "android-gpu")]
const VELLO_CHECK_INTERVAL: Duration = Duration::from_millis(8);

// ── IME ──────────────────────────────────────────────────────────────────────

mod ime;
use ime::apply_ime_action;

// ── Input translation ────────────────────────────────────────────────────────

mod input;

// ── The window surface ───────────────────────────────────────────────────────

/// Whatever this build presents through, named once so the loop above can hold
/// a single binding and the lifecycle can be reasoned about in one place.
///
/// The two implementations are not variants of a runtime choice — they are two
/// different builds. A `--features android-gpu` binary has no `SoftSurface`
/// compiled into it at all, and the shipping binary has no wgpu. That matters
/// here for a reason beyond code size: both presenters take the *same*
/// `ANativeWindow`, and Android lets a window be connected to exactly one
/// producer API at a time. `ANativeWindow_lock` connects it to the CPU
/// producer; creating a Vulkan swapchain connects it to
/// `NATIVE_WINDOW_API_EGL`. Building both and picking one at runtime would
/// mean the loser of that race silently failing every frame, so the choice is
/// made by the compiler and only one of them exists.
#[cfg(not(feature = "android-gpu"))]
type WindowSurface = SoftSurface;
#[cfg(feature = "android-gpu")]
type WindowSurface = GpuSurface;

#[cfg(not(feature = "android-gpu"))]
mod soft_surface;
#[cfg(not(feature = "android-gpu"))]
use soft_surface::SoftSurface;

// ── GPU surface (wgpu + vello) ──────────────────────────────────────────

#[cfg(feature = "android-gpu")]
mod gpu_context;
#[cfg(feature = "android-gpu")]
use gpu_context::GpuContext;
#[cfg(feature = "android-gpu")]
mod gpu_surface;
#[cfg(feature = "android-gpu")]
use gpu_surface::GpuSurface;

// ── GPU diagnostic tests ────────────────────────────────────────────────

// A bring-up diagnostic, run by hand: uncomment the `gpu_diagnostic::run_tests()`
// call in `run_loop` to log whether this device's Vulkan driver can do what
// Vello needs (compute, storage textures, a render). Nothing calls it otherwise,
// so it is dead code by design rather than by accident (#372).
#[cfg(feature = "android-gpu")]
#[allow(dead_code)]
mod gpu_diagnostic;

// ── Action processing ────────────────────────────────────────────────────────

/// The sooner of two "come back in" deadlines, where `None` is "no reason to".
fn earliest_due(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

// ── The text-selection toolbar (issue #813) ──────────────────────────────────

mod toolbar;
use toolbar::push_text_toolbar;

fn process_actions(actions: &[AppAction], running: &mut bool) {
    for action in actions {
        match action {
            AppAction::RequestRedraw => {
                REDRAW_PENDING.store(true, Ordering::Release);
            }
            AppAction::Exit => {
                *running = false;
            }
            AppAction::SetMinimized(_)
            | AppAction::SetMaximized(_)
            | AppAction::SetVisible(_)
            | AppAction::DragWindow
            | AppAction::DragResizeWindow(_)
            | AppAction::SetCursor(_)
            | AppAction::ToggleDevTools => {}
            // This shell asks for `TextContextMenuPresentation::Shell` at
            // startup; the loop answers the action by scanning each event's
            // actions for it (`toolbar_requested`) and showing the platform
            // toolbar, so there is nothing to do here (issue #813).
            AppAction::ShowTextContextMenu => {}
        }
    }
}
