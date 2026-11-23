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

thread_local! {
    /// When the event loop last retired a lost device (the event loop's
    /// thread only).
    static RETIRED_AT: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Note that the event loop retired the device at `now` (#1788).
pub(super) fn note_retired(now: Instant) {
    RETIRED_AT.with(|r| r.set(Some(now)));
}

/// Whether `now` is within a device loss's recovery: less than
/// [`GPU_REBUILD_LIMIT`] since the device was retired. A window shown or made
/// then tries for its renderer and, when the GPU will not start yet, waits
/// for the rebuild rather than taking the startup rule
/// ([`ShellWindow::start_renderer_with`]).
pub(super) fn gpu_recovering(now: Instant) -> bool {
    RETIRED_AT.with(|r| r.get().is_some_and(|at| now < at + GPU_REBUILD_LIMIT))
}

/// When the device was last retired, if ever.
fn retired_at() -> Option<Instant> {
    RETIRED_AT.with(std::cell::Cell::get)
}

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
        Self::since(pending, lost_frames, now, now)
    }

    /// A rebuild of a recovery that `started` earlier (the device's
    /// retirement): its limit counts from then. Tried first at `next`.
    pub(super) fn since(
        pending: Vec<WindowRole>,
        lost_frames: Vec<WindowRole>,
        started: Instant,
        next: Instant,
    ) -> Self {
        Self {
            pending,
            lost_frames,
            started,
            delay: GPU_REBUILD_FIRST_DELAY,
            next,
        }
    }

    /// Take window `role`, shown or made during the recovery with no
    /// renderer yet (#1788): tried with the others, and redrawn at the end.
    pub(super) fn adopt(&mut self, role: WindowRole) {
        if !self.pending.contains(&role) {
            self.pending.push(role);
        }
        if !self.lost_frames.contains(&role) {
            self.lost_frames.push(role);
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
    #[cfg(test)]
    pub(super) fn run(
        &mut self,
        now: Instant,
        try_window: impl FnMut(WindowRole, bool) -> bool,
    ) -> bool {
        self.run_shown_first(now, &[], try_window)
    }

    /// `Self::run`, with the windows in `shown` (the ones on screen) tried
    /// before the others (#2896): a shown window publishes the device it
    /// makes, and a hidden window built after it shares that device, where
    /// one built first would make a private device its show then refuses.
    /// So a hidden window is not tried at all while a shown one is still
    /// waiting (before the limit), lest the GPU come back between the two.
    pub(super) fn run_shown_first(
        &mut self,
        now: Instant,
        shown: &[WindowRole],
        mut try_window: impl FnMut(WindowRole, bool) -> bool,
    ) -> bool {
        if now < self.next {
            return self.pending.is_empty();
        }
        let deadline = self.started + GPU_REBUILD_LIMIT;
        let last = now >= deadline;
        // Stable: shown windows first, each group in its own order.
        self.pending.sort_by_key(|role| !shown.contains(role));
        let mut shown_waits = false;
        self.pending.retain(|&role| {
            let on_screen = shown.contains(&role);
            if !on_screen && shown_waits && !last {
                return true;
            }
            let settled = try_window(role, last);
            shown_waits |= on_screen && !settled;
            !settled
        });
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
        self.adopt_waiting_windows(Instant::now());
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
    /// paints nothing until then, and asks for no redraws.
    pub(super) fn awaits_gpu(&self, role: WindowRole) -> bool {
        if self.gpu_rebuild.as_ref().is_some_and(|r| r.awaits(role)) {
            return true;
        }
        let window = match role {
            WindowRole::Main => Some(&self.main),
            WindowRole::DevTools => self.devtools.as_ref(),
            WindowRole::Other(id) => self.others.iter().find(|w| w.handle == id),
        };
        window.is_some_and(|w| w.awaits_gpu)
    }

    /// Hand the rebuild the windows shown or made during the recovery whose
    /// renderer would not start yet ([`ShellWindow::start_renderer_with`]):
    /// they are tried with it, starting one if it was over (its limit still
    /// counted from the device's retirement).
    pub(super) fn adopt_waiting_windows(&mut self, now: Instant) {
        let waiting: Vec<WindowRole> = self
            .roles()
            .filter(|&role| std::mem::take(&mut self.w(role).awaits_gpu))
            .collect();
        if waiting.is_empty() {
            return;
        }
        let rebuild = self.gpu_rebuild.get_or_insert_with(|| {
            GpuRebuild::since(
                Vec::new(),
                Vec::new(),
                retired_at().unwrap_or(now),
                now + GPU_REBUILD_FIRST_DELAY,
            )
        });
        for role in waiting {
            rebuild.adopt(role);
        }
    }

    /// Drop every window's renderer and the device, and start rebuilding the
    /// windows on a new one. Every renderer goes first (each holds its
    /// surface, Vello's renderer, the compositors and the split slices, all
    /// on the old device), the shown windows' and the ones hidden windows
    /// keep for their next show (#2545) alike, then the device itself and the
    /// surfaces' GPU targets and registered textures; then each shown window
    /// that presented on the GPU builds its renderer
    /// ([`Self::run_gpu_rebuild`]), the first making (or adopting) the next
    /// device, which it publishes, and each hidden window prepared on the GPU
    /// builds one with no surface (#2891).
    fn replace_gpu_device(&mut self) {
        // The shown windows that presented on the GPU, and every window
        // that had a frame on the old device (those and the hidden ones that
        // kept a renderer).
        let (mut on_gpu, mut lost_frames) = (Vec::new(), Vec::new());
        for role in self.roles() {
            let w = self.w(role);
            let parked = w.parked_renderer.take().is_some();
            let shown = w.renderer.take().is_some();
            // A hidden window the app prepared on the GPU is rebuilt too, so
            // it is prepared again on the new device though no shown window
            // makes one (#2891: a second loss during a rebuild left it
            // unprepared).
            if shown || (parked && w.window.is_none() && w.prepared.is_some()) {
                on_gpu.push(role);
            }
            if shown || parked {
                lost_frames.push(role);
            }
        }
        GPU.retire();
        note_retired(Instant::now());
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
    /// again, so its show still finds a drawn frame: on the renderer the
    /// rebuild built it ([`ShellWindow::rebuild_hidden_with`], #2824/#2891),
    /// else (the GPU did not come back in time) by the startup rule, with
    /// software where its show would fall back to it; any other hidden window
    /// builds its renderer at its next show. A window that presents with
    /// software lost nothing and does nothing; a window closed since is
    /// skipped. The windows' frames still out
    /// to Vello went with their renderers, so the lost-frame look
    /// (`look_at_frames`, #2781) has nothing of the old device left to ask.
    pub(super) fn run_gpu_rebuild(&mut self, now: Instant) {
        let Some(mut rebuild) = self.gpu_rebuild.take() else {
            return;
        };
        let shown: Vec<WindowRole> = rebuild
            .pending
            .iter()
            .copied()
            .filter(|&role| self.open_w(role).is_some_and(|w| w.window.is_some()))
            .collect();
        if !rebuild.run_shown_first(now, &shown, |role, last| self.rebuild_window(role, last)) {
            self.gpu_rebuild = Some(rebuild);
            return;
        }
        for role in rebuild.lost_frames {
            // A window closed since is gone (#2891).
            let Some(w) = self.open_w(role) else {
                continue;
            };
            // Nothing of the old frames survives: the next one is full.
            w.app.mark_scene_dirty();
            w.redraw_lost_frame();
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
        // A window closed since is settled (#2891).
        let Some(w) = self.open_w(role) else {
            return true;
        };
        // So is one shown during the rebuild that has its renderer already
        // (#2891: it was built a second one).
        if w.renderer.is_some() {
            return true;
        }
        let Some(window) = w.window.as_ref().map(|w| w.window.clone()) else {
            let choice = w.prepared_with;
            return match w.rebuild_hidden_with(last, |physical| {
                WgpuRenderer::try_new_offscreen(physical.0, physical.1, choice)
            }) {
                Ok(renderer) => {
                    w.note_renderer_built(renderer.is_some());
                    if renderer.is_some() {
                        w.parked_renderer = renderer;
                    }
                    true
                }
                Err(e) => {
                    tracing::info!(
                        "rinch: no GPU renderer yet for the prepared hidden window after the \
                         device loss ({e}); trying again"
                    );
                    false
                }
            };
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

impl ShellWindow {
    /// [`RinchRuntime::rebuild_window`] for a window with no OS window: only
    /// one the app prepared (on the GPU before the loss, or during the
    /// recovery when the GPU would not start then: #2824) waits for a
    /// renderer: `build` makes one for the size the
    /// preparation was painted at, and it is answered (`Ok(Some)`) to be
    /// parked for the preparation the rebuild's end paints again, and for the
    /// show. `Err`: the GPU will not start yet, try again. Any other hidden
    /// window is settled at once (`Ok(None)`): it builds its renderer at its
    /// show; and so is this one at the rebuild's `last` try, when the
    /// rebuild's end prepares it by the startup rule.
    pub(super) fn rebuild_hidden_with<R>(
        &mut self,
        last: bool,
        build: impl FnOnce((u32, u32)) -> Result<R, crate::shell::desktop::GpuStartError>,
    ) -> Result<Option<R>, String> {
        use crate::shell::desktop::GpuStartError;
        let Some((scale, _)) = self.prepared else {
            return Ok(None);
        };
        if last || self.parked_renderer.is_some() {
            return Ok(None);
        }
        let (w, h) = self.laid_out_size().unwrap_or((1, 1));
        let physical = (
            ((w as f64 * scale).round() as u32).max(1),
            ((h as f64 * scale).round() as u32).max(1),
        );
        match build(physical) {
            Ok(renderer) => Ok(Some(renderer)),
            // The show presents with software: nothing to prepare on the GPU.
            Err(GpuStartError::DeclinedCpuAdapter(_)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
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

    /// #1788 re-review: a window shown during the recovery (a tray app hidden
    /// at the loss) whose first device request fails does not take the
    /// startup rule (a panic, or software for good): it waits, the rebuild
    /// takes it, and it is built on the new device at the rebuild's next try.
    #[test]
    fn a_window_shown_during_the_recovery_waits_and_is_rebuilt_on_the_new_device() {
        fn component(scope: &mut RenderScope) -> NodeHandle {
            scope.create_element("div")
        }
        let mut rt = RinchRuntime::new("t", 800, 600, component, &[]);
        let t0 = Instant::now();
        note_retired(t0);
        let shown = t0 + Duration::from_millis(300);
        // The show's request: the driver is still resetting.
        let started = rt.main.start_renderer_with(
            shown,
            |_| Err::<&str, _>("no adapter yet".to_string()),
            |_| panic!("the startup rule decided during the recovery"),
            "waiting",
        );
        assert_eq!(started, "waiting");
        assert!(rt.awaits_gpu(WindowRole::Main), "it paints nothing");

        rt.adopt_waiting_windows(shown);
        let rebuild = rt.gpu_rebuild.as_mut().expect("the rebuild takes it");
        assert!(rebuild.awaits(WindowRole::Main));
        assert_eq!(rebuild.next(), shown + GPU_REBUILD_FIRST_DELAY);
        let mut tries = Vec::new();
        // The device is back at the next try.
        let done = rebuild.run(shown + GPU_REBUILD_FIRST_DELAY, |role, last| {
            tries.push((role, last));
            true
        });
        assert!(done);
        assert_eq!(
            tries,
            [(WindowRole::Main, false)],
            "tried, not by the startup rule"
        );
        assert!(!rt.main.awaits_gpu, "taken by the rebuild");
    }

    /// Outside a recovery the startup rule is the one that decides.
    #[test]
    fn outside_a_recovery_a_window_takes_the_startup_rule() {
        fn component(scope: &mut RenderScope) -> NodeHandle {
            scope.create_element("div")
        }
        let mut rt = RinchRuntime::new("t", 800, 600, component, &[]);
        let t0 = Instant::now();
        note_retired(t0);
        let started = rt.main.start_renderer_with(
            t0 + GPU_REBUILD_LIMIT,
            |_| -> Result<&str, String> { panic!("tried during no recovery") },
            |_| "startup rule",
            "waiting",
        );
        assert_eq!(started, "startup rule");
        assert!(!rt.awaits_gpu(WindowRole::Main));
    }

    /// #2824: a hidden window the app prepares during the recovery
    /// (`prepare_current_window`) whose first device request fails does not
    /// take the startup rule (a frame prepared with software, or none where
    /// the show would panic): it waits, the rebuild takes it, a later try
    /// builds its renderer, and the rebuild's end prepares it again on it.
    #[test]
    fn a_window_prepared_during_the_recovery_waits_and_is_rebuilt_on_the_new_device() {
        use super::super::main_window::OffscreenStart;
        use crate::shell::renderer::Renderer;
        fn component(scope: &mut RenderScope) -> NodeHandle {
            scope.create_element("div")
        }
        // `Auto` (the startup rule falls back) and `Gpu` (it panics at the
        // show) alike.
        for choice in [Renderer::Auto, Renderer::Gpu] {
            let mut rt = RinchRuntime::new("t", 800, 600, component, &[]);
            let t0 = Instant::now();
            note_retired(t0);
            let prepared = t0 + Duration::from_millis(300);
            // The preparation's request: the driver is still resetting.
            let started = rt.main.start_offscreen_with(prepared, choice, false, |_| {
                Err::<&str, _>("no adapter yet".to_string().into())
            });
            assert_eq!(started, OffscreenStart::Waits, "{choice:?}");
            assert!(rt.awaits_gpu(WindowRole::Main), "{choice:?}");
            // What `prepare_hidden_as` records after it.
            rt.main.prepared = Some((1.5, 0));

            rt.adopt_waiting_windows(prepared);
            let mut rebuild = rt.gpu_rebuild.take().expect("the rebuild takes it");
            assert!(rebuild.awaits(WindowRole::Main), "{choice:?}");
            assert!(!rt.main.awaits_gpu, "taken by the rebuild");
            // The rebuild's first try fails too; the second finds the device.
            let mut sizes = Vec::new();
            let mut device_back = false;
            let mut try_hidden = |w: &mut ShellWindow, last: bool| {
                let back = std::mem::replace(&mut device_back, true);
                match w.rebuild_hidden_with(last, |physical| {
                    sizes.push(physical);
                    if back {
                        Ok("renderer")
                    } else {
                        Err("still resetting".to_string().into())
                    }
                }) {
                    Ok(built) => {
                        assert_eq!(built, Some("renderer"), "{choice:?}");
                        true
                    }
                    Err(_) => false,
                }
            };
            let at = rebuild.next();
            assert!(!rebuild.run(at, |_, last| try_hidden(&mut rt.main, last)));
            assert!(rebuild.awaits(WindowRole::Main), "it waits again");
            let at = rebuild.next();
            assert!(rebuild.run(at, |_, last| try_hidden(&mut rt.main, last)));
            assert_eq!(sizes.len(), 2, "two tries, {choice:?}");
            // At the scale the preparation was painted at.
            let laid_out = rt.main.laid_out_size().unwrap_or((1, 1));
            let expect = (
                ((laid_out.0 as f64 * 1.5).round() as u32).max(1),
                ((laid_out.1 as f64 * 1.5).round() as u32).max(1),
            );
            assert_eq!(sizes[1], expect);
        }
    }

    /// #2824: outside a recovery the preparation keeps the startup rule: with
    /// software where the show falls back, nothing where it panics; and a
    /// hidden window that was not prepared waits for no renderer.
    #[test]
    fn outside_a_recovery_the_preparation_takes_the_startup_rule() {
        use super::super::main_window::OffscreenStart;
        use crate::shell::renderer::Renderer;
        fn component(scope: &mut RenderScope) -> NodeHandle {
            scope.create_element("div")
        }
        let mut rt = RinchRuntime::new("t", 800, 600, component, &[]);
        let t0 = Instant::now();
        note_retired(t0);
        let after = t0 + GPU_REBUILD_LIMIT;
        let fail = |_: &mut ShellWindow| Err::<&str, _>("no adapter".to_string().into());
        assert_eq!(
            rt.main
                .start_offscreen_with(after, Renderer::Auto, false, fail),
            OffscreenStart::Software
        );
        assert_eq!(
            rt.main
                .start_offscreen_with(after, Renderer::Gpu, false, fail),
            OffscreenStart::Nothing
        );
        assert_eq!(
            rt.main
                .start_offscreen_with(after, Renderer::Auto, true, fail),
            OffscreenStart::Nothing,
            "a device the app configured"
        );
        assert!(!rt.awaits_gpu(WindowRole::Main));
        // Never prepared: settled at once, nothing built.
        let built = rt
            .main
            .rebuild_hidden_with(false, |_| -> Result<&str, _> { panic!("built") });
        assert_eq!(built, Ok(None));
    }

    /// #2891: a window closed while the rebuild still waits for it (pending,
    /// and among the lost frames) is skipped; it used to panic in
    /// `RinchRuntime::w` ("an event for an open window").
    #[test]
    fn a_window_closed_during_the_rebuild_is_skipped() {
        fn component(scope: &mut RenderScope) -> NodeHandle {
            scope.create_element("div")
        }
        let mut rt = RinchRuntime::new("t", 800, 600, component, &[]);
        let t0 = Instant::now();
        let gone = WindowRole::Other(4242);
        rt.gpu_rebuild = Some(GpuRebuild::new(
            vec![gone],
            vec![gone, WindowRole::DevTools],
            t0,
        ));
        rt.run_gpu_rebuild(t0);
        assert!(rt.gpu_rebuild.is_none(), "settled");
        // And a closed main window.
        rt.main_closed = true;
        rt.gpu_rebuild = Some(GpuRebuild::new(
            vec![WindowRole::Main],
            vec![WindowRole::Main],
            t0,
        ));
        rt.main.prepared = Some((1.0, 0));
        rt.run_gpu_rebuild(t0);
        assert!(rt.gpu_rebuild.is_none(), "settled");
    }

    /// #2891: the rebuild's end prepares a hidden window again on the
    /// renderer the rebuild built it, though no window published a device
    /// (`try_new_offscreen` publishes none, and every window is hidden).
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_device_loss_a_prepared_hidden_window_is_prepared_again_on_its_rebuilt_renderer_2891() {
        use crate::shell::renderer::Renderer;
        fn component(scope: &mut RenderScope) -> NodeHandle {
            scope.create_element("div")
        }
        let prepared_frames = |rt: &RinchRuntime| {
            rt.main
                .app
                .doc
                .as_ref()
                .expect("mounted")
                .borrow()
                .tree
                .perf
                .prepared()
        };
        let mut rt = RinchRuntime::new("t", 800, 600, component, &[]);
        rt.main.app.mount_component(800.0, 600.0);
        // As a preparation during the recovery leaves it: waiting, nothing
        // parked (the request for a device failed).
        rt.main.prepared = Some((1.0, 0));
        rt.main.prepared_with = Renderer::Gpu;
        rt.main.awaits_gpu = true;
        let now = Instant::now();
        note_retired(now);
        rt.adopt_waiting_windows(now);
        let before = prepared_frames(&rt);
        rt.run_gpu_rebuild(now + Duration::from_millis(200));
        assert!(rt.gpu_rebuild.is_none(), "rebuilt");
        assert!(rt.main.parked_renderer.is_some(), "on the GPU, parked");
        assert_eq!(prepared_frames(&rt), before + 1, "prepared again");
    }

    /// #2896: a hidden prepared window is rebuilt only after the shown ones
    /// (whatever their order of roles), so it shares the device a shown
    /// window publishes; while a shown window still waits for the GPU, the
    /// hidden one is not tried, and at the limit both are.
    #[test]
    fn hidden_windows_are_rebuilt_after_the_shown_ones() {
        let t0 = Instant::now();
        let shown = WindowRole::Other(7);
        let hidden = WindowRole::Main;
        let mut rebuild = GpuRebuild::new(vec![hidden, shown], vec![hidden, shown], t0);
        let mut tries = Vec::new();
        // The first try: the GPU is not back for the shown window.
        assert!(!rebuild.run_shown_first(t0, &[shown], |role, last| {
            tries.push((role, last));
            false
        }));
        assert_eq!(tries, [(shown, false)], "the hidden one is not tried");
        assert!(rebuild.awaits(hidden) && rebuild.awaits(shown));
        // The second: back; the shown window publishes, then the hidden one.
        tries.clear();
        let at = rebuild.next();
        assert!(rebuild.run_shown_first(at, &[shown], |role, last| {
            tries.push((role, last));
            true
        }));
        assert_eq!(tries, [(shown, false), (hidden, false)]);

        // At the limit every window is tried, by the startup rule.
        let mut rebuild = GpuRebuild::new(vec![hidden, shown], Vec::new(), t0);
        tries.clear();
        assert!(
            rebuild.run_shown_first(t0 + GPU_REBUILD_LIMIT, &[shown], |role, last| {
                tries.push((role, last));
                role == hidden || last
            })
        );
        assert_eq!(tries, [(shown, true), (hidden, true)]);
    }
}
