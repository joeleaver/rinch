//! Android runtime using android-activity (NativeActivity), presenting
//! straight into the `ANativeWindow` the Activity owns.
//!
//! Bypasses winit entirely for direct control over the Android Activity
//! lifecycle, touch input, and surface management. Uses the same
//! platform-agnostic [`RinchApp`] as the desktop shell.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use android_activity::AndroidApp;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;
use rinch_platform::AppAction;

use crate::app::{RinchApp, TextContextMenuPresentation};

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

    let mut state = LoopState::new(app);
    while state.running {
        state.turn(&android_app);
    }
}

/// How soon the loop looks again for Vello's counters of a frame the GPU has
/// not finished (#2071).
#[cfg(feature = "android-gpu")]
const VELLO_CHECK_INTERVAL: Duration = Duration::from_millis(8);

// ── The loop's state and the order of a turn ─────────────────────────────────

mod loop_state;
use loop_state::LoopState;

// ── Lifecycle events ─────────────────────────────────────────────────────────

mod main_events;

// ── Painting ─────────────────────────────────────────────────────────────────

mod present;

// ── The soft keyboard and the IME ────────────────────────────────────────────

mod ime;

// ── Input translation ────────────────────────────────────────────────────────

mod input;

// ── The window surface ───────────────────────────────────────────────────────

/// Whatever this build presents through, named once so `LoopState` can hold
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
