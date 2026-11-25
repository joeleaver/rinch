//! The activity's lifecycle events: the window arriving, going and changing
//! size, the device configuration moving, pause, resume and focus.
//!
//! Each is handled inside `poll_events`, which `android-activity` calls with
//! the Java thread that raised the event blocked until it returns. What can be
//! deferred to the loop body is: these handlers only record it.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use android_activity::{AndroidApp, MainEvent};

use rinch_platform::PlatformEvent;

#[cfg(feature = "android-gpu")]
use super::GpuContext;
#[cfg(not(feature = "android-gpu"))]
use super::SoftSurface;
use super::loop_state::LoopState;
use super::{DEFAULT_FRAME_INTERVAL, REDRAW_PENDING, process_actions};

impl LoopState {
    pub(super) fn on_main_event(&mut self, android_app: &AndroidApp, main_event: MainEvent<'_>) {
        match main_event {
            MainEvent::InitWindow { .. } => self.on_init_window(android_app),
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
                // binding, the paint block cannot run without it, and this
                // line empties it before the paint block is reached. There
                // is no second handle to go stale.
                //
                // `gpu` — the device, the queue and vello's compiled
                // pipelines — deliberately survives. Pressing Home
                // destroys the window and nothing else; rebuilding the
                // renderer on the way back would cost the better part of a
                // second of black screen for a device Vulkan never
                // invalidated.
                self.surface = None;
            }
            MainEvent::WindowResized { .. } => self.on_window_resized(android_app),
            MainEvent::ConfigChanged { .. } => self.on_config_changed(),
            MainEvent::Resume { .. } => {
                rinch_android::lifecycle::notify_resumed();
            }
            MainEvent::Pause => {
                rinch_android::lifecycle::notify_paused();
                // Flushed in `turn`, where the viewport this loop dispatches
                // against is in scope.
                self.backgrounded = true;
            }
            // Android's answer to `PlatformEvent::WindowFocus` (issue
            // #147): the activity keeps its in-document focus claim across
            // a focus loss — a notification shade, a permission dialog —
            // and the focused widget is told, then told again when focus
            // returns. Distinct from `Resume`/`Pause`, which are lifecycle:
            // an activity can be resumed but unfocused.
            MainEvent::GainedFocus => {
                self.window_focus_change = Some(true);
            }
            MainEvent::LostFocus => {
                self.window_focus_change = Some(false);
            }
            MainEvent::Destroy => {
                self.running = false;
            }
            _ => {}
        }
    }

    fn on_init_window(&mut self, android_app: &AndroidApp) {
        if let Some(native_window) = android_app.native_window() {
            let w = native_window.width() as u32;
            let h = native_window.height() as u32;
            self.physical_size = (w, h);

            let dpi = rinch_android::display::density_dpi().unwrap_or_else(|| {
                let config = android_app.config();
                config.density().unwrap_or(160) as i32
            }) as f64;
            self.scale_factor = dpi / 160.0;

            // What this panel calls a frame. Read here rather
            // than at startup because there is no display to ask
            // about until there is a window on it. It is *also*
            // re-read once a second while frames are being drawn
            // (see `RATE_RECHECK`), and it has to be, because the
            // answer on this line is only true at this instant:
            // the same handset answers 120Hz on a cold start and
            // 60Hz on the `InitWindow` that follows a press of
            // Home. See `display::refresh_rate_hz`.
            self.frame_interval = rinch_android::display::refresh_rate_hz()
                .map(|hz| Duration::from_secs_f32(1.0 / hz))
                .unwrap_or(DEFAULT_FRAME_INTERVAL);
            self.frame_interval_read = Instant::now();

            log::info!(
                "InitWindow: {}x{} physical, density={dpi}, scale={:.2}, \
                 frame={:.2}ms",
                w,
                h,
                self.scale_factor,
                self.frame_interval.as_secs_f64() * 1000.0
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
            self.surface = None;

            #[cfg(not(feature = "android-gpu"))]
            {
                self.surface = Some(SoftSurface::new(&native_window, w, h));
            }
            #[cfg(feature = "android-gpu")]
            {
                self.surface = GpuContext::attach(&mut self.gpu, &native_window, w, h);
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
            self.app.set_surface_size(self.physical_size);
            self.app.set_device_pixel_ratio(self.scale_factor);
            if !self.mounted {
                let (lw, lh) = rinch_platform::to_logical((w, h), self.scale_factor);
                self.app.set_text_scale(self.scale_factor as f32);
                self.app.mount_component(lw as f32, lh as f32);
                self.mounted = true;
            }

            REDRAW_PENDING.store(true, Ordering::Release);
        }
    }

    fn on_window_resized(&mut self, android_app: &AndroidApp) {
        if let Some(native_window) = android_app.native_window() {
            let w = native_window.width() as u32;
            let h = native_window.height() as u32;
            self.physical_size = (w, h);

            let (lw, lh) = rinch_platform::to_logical((w, h), self.scale_factor);
            // Published with the `Resized` (`window_physical_size`).
            self.app.set_surface_size(self.physical_size);

            // The `Resized` payload is the *logical* viewport layout
            // is resolved at; `handle_event`'s `window_size` is the
            // *physical* surface size it derives that viewport from
            // (see `RinchApp::layout_viewport`). Passing the logical
            // size for both divided by the scale factor twice.
            let actions = self.app.handle_event(
                PlatformEvent::Resized {
                    width: lw,
                    height: lh,
                },
                self.physical_size,
                self.scale_factor,
            );
            process_actions(&actions, &mut self.running);

            if let Some(ref mut s) = self.surface {
                s.resize(w, h);
            }

            REDRAW_PENDING.store(true, Ordering::Release);
        }
    }

    fn on_config_changed(&mut self) {
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
        self.config_changed = true;
        // The whole point of the event is that what should be on
        // the glass has changed, so ask for a frame. The handler's
        // signal writes would normally schedule this themselves;
        // asking here means an app that re-reads the platform
        // *without* writing a signal — one that only calls
        // `set_light_system_bars`, say — still repaints against
        // the new configuration.
        REDRAW_PENDING.store(true, Ordering::Release);
    }
}
