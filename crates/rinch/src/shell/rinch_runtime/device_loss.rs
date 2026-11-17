//! Replacing the shared GPU device after a loss (#1788): the event loop's
//! half of `shell::gpu_device`.

use std::time::{Duration, Instant};

use super::*;
use crate::shell::gpu_device::{self, GPU};

/// How long the windows' rebuild after a device loss keeps trying before the
/// startup rule decides (a fallback to software, or a panic): a TDR or a
/// driver reset can take a few seconds before a new device can be made.
pub(super) const GPU_REBUILD_LIMIT: Duration = Duration::from_secs(5);
/// The wait before the rebuild's second try; each later wait is twice the
/// last, up to [`GPU_REBUILD_MAX_DELAY`], and never past the limit.
pub(super) const GPU_REBUILD_FIRST_DELAY: Duration = Duration::from_millis(100);
pub(super) const GPU_REBUILD_MAX_DELAY: Duration = Duration::from_secs(1);

/// The windows still to be rebuilt on a new device after a loss (#1788), and
/// when to try them again.
#[derive(Debug)]
pub(super) struct GpuRebuild {
    /// The windows that presented on the GPU and have no renderer yet. They
    /// paint nothing until rebuilt.
    pending: Vec<WindowRole>,
    /// Every window that had a frame on the old device: drawn again once the
    /// rebuild is over.
    lost_frames: Vec<WindowRole>,
    started: Instant,
    delay: Duration,
    next: Instant,
}

impl GpuRebuild {
    pub(super) fn new(
        pending: Vec<WindowRole>,
        lost_frames: Vec<WindowRole>,
        now: Instant,
    ) -> Self {
        Self {
            pending,
            lost_frames,
            started: now,
            delay: GPU_REBUILD_FIRST_DELAY,
            next: now,
        }
    }

    /// When to try again: the event loop wakes then.
    pub(super) fn next(&self) -> Instant {
        self.next
    }

    /// Whether window `role` waits for its renderer.
    pub(super) fn awaits(&self, role: WindowRole) -> bool {
        self.pending.contains(&role)
    }

    /// Try each pending window with `try_window(role, last)`, which answers
    /// whether the window is settled (rebuilt, or past saving); `last` is
    /// set once the limit has passed, when the startup rule decides and
    /// every window settles. The ones that are not wait for the next try,
    /// after a wait twice the last. Answers whether none is left. Before
    /// [`Self::next`] it tries nothing.
    pub(super) fn run(
        &mut self,
        now: Instant,
        mut try_window: impl FnMut(WindowRole, bool) -> bool,
    ) -> bool {
        if now < self.next {
            return self.pending.is_empty();
        }
        let deadline = self.started + GPU_REBUILD_LIMIT;
        let last = now >= deadline;
        self.pending.retain(|&role| !try_window(role, last));
        if !self.pending.is_empty() {
            self.next = (now + self.delay).min(deadline);
            self.delay = (self.delay * 2).min(GPU_REBUILD_MAX_DELAY);
        }
        self.pending.is_empty()
    }
}

impl RinchRuntime {
    /// Replace the shared GPU device when it is due (lost, or an app given
    /// `App::external_gpu` handed over another), go on with a rebuild that is
    /// waiting to try again, then tell the app's listeners of a device that
    /// changed since they were last told, once every window is rebuilt.
    /// Called at every wake, before every redraw and in `AboutToWait`, so a
    /// loss is dealt with before the next frame is drawn on the dead device.
    pub(super) fn settle_gpu_device(&mut self) {
        GPU.poll_published();
        if GPU.take_due() {
            self.replace_gpu_device();
        }
        self.run_gpu_rebuild(Instant::now());
        if self.gpu_rebuild.is_some() {
            return;
        }
        if let Some(handle) = GPU.take_announcement() {
            gpu_device::announce(handle.as_ref());
        }
    }

    /// When a rebuild after a device loss tries again: a wake the event loop
    /// arms.
    pub(super) fn gpu_rebuild_deadline(&self) -> Option<Instant> {
        self.gpu_rebuild.as_ref().map(GpuRebuild::next)
    }

    /// Whether window `role` waits for its renderer after a device loss: it
    /// paints nothing until then.
    pub(super) fn awaits_gpu(&self, role: WindowRole) -> bool {
        self.gpu_rebuild.as_ref().is_some_and(|r| r.awaits(role))
    }

    /// Drop every window's renderer and the device, and start rebuilding the
    /// windows on a new one. Every renderer goes first (each holds its
    /// surface, Vello's renderer, the compositors and the split slices, all
    /// on the old device), the shown windows' and the ones hidden windows
    /// keep for their next show (#2545) alike, then the device itself and the
    /// surfaces' GPU targets and registered textures; then each shown window
    /// that presented on the GPU builds its renderer
    /// ([`Self::run_gpu_rebuild`]), the first making (or adopting) the next
    /// device, which it publishes.
    fn replace_gpu_device(&mut self) {
        // The shown windows that presented on the GPU, and every window
        // that had a frame on the old device (those and the hidden ones that
        // kept a renderer).
        let (mut on_gpu, mut lost_frames) = (Vec::new(), Vec::new());
        for role in self.roles() {
            let w = self.w(role);
            let parked = w.parked_renderer.take().is_some();
            let shown = w.renderer.take().is_some();
            if shown {
                on_gpu.push(role);
            }
            if shown || parked {
                lost_frames.push(role);
            }
        }
        GPU.retire();
        crate::render_surface::forget_gpu_device();
        // A rebuild still trying for the device before this one: its windows
        // are in this one's (their renderers are `None`, so not in `on_gpu`).
        if let Some(earlier) = self.gpu_rebuild.take() {
            for role in earlier.pending {
                if !on_gpu.contains(&role) {
                    on_gpu.push(role);
                }
            }
            for role in earlier.lost_frames {
                if !lost_frames.contains(&role) {
                    lost_frames.push(role);
                }
            }
        }
        self.gpu_rebuild = Some(GpuRebuild::new(on_gpu, lost_frames, Instant::now()));
    }

    /// Try the windows of the rebuild after a device loss, when it is time.
    ///
    /// The device can be slow to come back: a driver reset or a TDR is still
    /// recovering when the loss is reported, and a request for a new device
    /// then fails. So a window whose renderer will not start is tried again,
    /// after 100 ms and then twice as long each time (at most a second
    /// apart), for up to [`GPU_REBUILD_LIMIT`], and paints nothing meanwhile.
    /// Only then does the startup rule decide: an app that chose the GPU or
    /// configured the device itself (`Renderer::Gpu`, `App::gpu_config`,
    /// `App::external_gpu`) panics, and any other presents with software for
    /// the rest of the session. DevTools presents with software then.
    ///
    /// Once every window is rebuilt, every frame on the old device is gone,
    /// as a frame Vello lost is (#2071), and each window draws again the way
    /// it does for that ([`ShellWindow::redraw_lost_frame`]): a shown window
    /// redraws, in full; a hidden window the app prepared (#2633) is prepared
    /// again on the new device, so its show still finds a drawn frame (when
    /// no shown window made a new device, it is left to build its renderer
    /// at its show, as an unprepared one does); any other hidden window
    /// builds its renderer at its next show. A window that presents with
    /// software lost nothing and does nothing. The windows' frames still out
    /// to Vello went with their renderers, so the lost-frame look
    /// (`look_at_frames`, #2781) has nothing of the old device left to ask.
    pub(super) fn run_gpu_rebuild(&mut self, now: Instant) {
        let Some(mut rebuild) = self.gpu_rebuild.take() else {
            return;
        };
        if !rebuild.run(now, |role, last| self.rebuild_window(role, last)) {
            self.gpu_rebuild = Some(rebuild);
            return;
        }
        let on_new_device = GPU.handle().is_some();
        for role in rebuild.lost_frames {
            let w = self.w(role);
            // Nothing of the old frames survives: the next one is full.
            w.app.mark_scene_dirty();
            if w.window.is_some() || on_new_device {
                w.redraw_lost_frame();
            }
        }
        tracing::info!(
            "rinch: GPU device replaced (generation {})",
            crate::shell::desktop::gpu_generation()
        );
    }

    /// Build window `role`'s renderer on the new device; `last`: by the
    /// startup rule if the GPU will not start. Answers whether the window is
    /// settled: rebuilt, presenting with software, or gone from the screen
    /// (a hidden window builds its renderer at its show).
    fn rebuild_window(&mut self, role: WindowRole, last: bool) -> bool {
        let w = self.w(role);
        let Some(window) = w.window.as_ref().map(|w| w.window.clone()) else {
            return true;
        };
        if role == WindowRole::DevTools {
            // DevTools presents as it was opened: with `WgpuRenderer::try_new`.
            let size = window.surface_size();
            match WgpuRenderer::try_new(&*window, size.width.max(1), size.height.max(1)) {
                Ok(renderer) => w.renderer = Some(renderer),
                Err(e) if last => tracing::warn!(
                    "DevTools: GPU renderer unavailable after the device loss ({e}); \
                     using software"
                ),
                Err(e) => {
                    tracing::debug!("DevTools: no GPU renderer yet after the device loss ({e})");
                    return false;
                }
            }
            return true;
        }
        let transparent = w.gpu_surface_transparent();
        let started = if last {
            w.start_gpu_renderer(&*window, transparent)
        } else {
            match w.try_start_gpu_renderer(&*window, transparent) {
                Ok(started) => started,
                Err(e) => {
                    tracing::info!(
                        "rinch: no GPU renderer yet after the device loss ({e}); trying again"
                    );
                    return false;
                }
            }
        };
        let (renderer, built) = started;
        w.note_renderer_built(built);
        w.renderer_started(renderer);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1788 review: a rebuild whose first device request fails (a driver
    /// reset still recovering) tries again after the first delay, and the
    /// window is rebuilt then; nothing is decided by the startup rule.
    #[test]
    fn a_rebuild_whose_first_try_fails_tries_again_and_succeeds() {
        let t0 = Instant::now();
        let mut rebuild = GpuRebuild::new(vec![WindowRole::Main], vec![WindowRole::Main], t0);
        let mut tries = Vec::new();
        // The injected failure: the first request finds no device.
        let mut device_back = false;
        let mut try_window = |role: WindowRole, last: bool, tries: &mut Vec<_>| {
            tries.push((role, last));
            let ok = device_back;
            device_back = true;
            ok
        };
        assert!(!rebuild.run(t0, |r, l| try_window(r, l, &mut tries)));
        assert!(
            rebuild.awaits(WindowRole::Main),
            "it waits, painting nothing"
        );
        assert_eq!(rebuild.next(), t0 + GPU_REBUILD_FIRST_DELAY);
        // Not before its time.
        assert!(
            !rebuild.run(t0 + Duration::from_millis(50), |r, l| try_window(
                r, l, &mut tries
            ))
        );
        assert!(rebuild.run(t0 + GPU_REBUILD_FIRST_DELAY, |r, l| try_window(
            r, l, &mut tries
        )));
        assert!(!rebuild.awaits(WindowRole::Main));
        assert_eq!(
            tries,
            [(WindowRole::Main, false), (WindowRole::Main, false)],
            "two tries, neither by the startup rule"
        );
    }

    /// A device that never comes back: the waits double up to a second, and
    /// once the limit has passed the last try is by the startup rule, which
    /// settles the window.
    #[test]
    fn a_rebuild_that_keeps_failing_ends_with_the_startup_rule_at_the_limit() {
        let t0 = Instant::now();
        let mut rebuild = GpuRebuild::new(vec![WindowRole::Main], Vec::new(), t0);
        let mut waits = Vec::new();
        let mut now = t0;
        let mut lasts = Vec::new();
        loop {
            let done = rebuild.run(now, |_, last| {
                lasts.push(last);
                last
            });
            if done {
                break;
            }
            waits.push(rebuild.next() - now);
            now = rebuild.next();
            assert!(waits.len() < 20, "the rebuild ends");
        }
        let ms = |v: u64| Duration::from_millis(v);
        assert_eq!(
            waits,
            [
                ms(100),
                ms(200),
                ms(400),
                ms(800),
                ms(1000),
                ms(1000),
                ms(1000),
                ms(500)
            ]
        );
        assert_eq!(now, t0 + GPU_REBUILD_LIMIT, "the last try is at the limit");
        assert_eq!(lasts.iter().filter(|l| **l).count(), 1);
        assert!(*lasts.last().unwrap(), "and it is the startup rule's");
    }
}
