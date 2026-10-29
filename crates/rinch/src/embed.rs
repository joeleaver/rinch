//! Game engine integration API.
//!
//! This module provides [`RinchContext`] for embedding rinch UI into an
//! existing game engine or custom render loop. The game owns the wgpu
//! `Device`/`Queue`/`Surface` and frame loop; rinch provides UI as a Vello
//! scene that can be composited on top of game content.
//!
//! # Architecture
//!
//! ```text
//! Game loop:
//!   1. Collect platform events
//!   2. ctx.update(&events)        // rinch processes input, updates layout
//!   3. game.render()              // render game scene
//!   4. overlay.render(&device, &queue, &mut ctx) // render rinch UI to texture
//!   5. composite game + UI        // blit both to swapchain
//! ```
//!
//! # Two integration patterns
//!
//! - **Full overlay (HUD)**: Rinch covers the entire window. Use
//!   [`RinchContext::wants_mouse`] / [`wants_keyboard`](RinchContext::wants_keyboard)
//!   to decide whether input goes to the game or UI.
//!
//! - **Split layout**: Rinch renders toolbars/panels around a
//!   [`GameViewport`] hole. Query the viewport rect with
//!   [`RinchContext::viewport_rect`] and render the game into that region.
//!
//! # Input methods (IME)
//!
//! The host owns the window, so it owns the platform input method. After each
//! [`update`](RinchContext::update), [`RinchContext::take_ime_change`] says
//! whether to switch it on (a text field, an editor or an `on_ime` target took
//! focus), move its candidate box (the caret moved), or switch it off; feed
//! the composition back in as `PlatformEvent::Ime` events on the next update.
//!
//! # Example
//!
//! ```ignore
//! use rinch::embed::{RinchContext, RinchContextConfig, RinchOverlayRenderer};
//! use rinch::prelude::*;
//!
//! let mut ctx = RinchContext::new(
//!     RinchContextConfig {
//!         width: 1280,
//!         height: 720,
//!         scale_factor: 1.0,
//!         theme: None,
//!     },
//!     game_ui,
//! );
//!
//! // In your game loop:
//! let actions = ctx.update(&platform_events);
//! let ui = overlay.render(&device, &queue, &mut ctx);
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use peniko::Color;
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRenderer, RendererOptions, Scene};
use wgpu::{
    Device, Extent3d, Queue, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureView, TextureViewDescriptor,
};

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_platform::{AppAction, PlatformEvent};

use crate::app::RinchApp;
use crate::app::hit_testing::hit_test;
pub use crate::app::{ImeChange, ImeState};
/// The rect type of [`ImeState::cursor_area`].
pub use rinch_core::ElementBounds;

// ── LayoutRect ───────────────────────────────────────────────────────────────

/// A layout rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

// ── RinchContextConfig ───────────────────────────────────────────────────────

/// Configuration for creating a [`RinchContext`].
pub struct RinchContextConfig {
    /// Initial viewport width in physical pixels.
    pub width: u32,
    /// Initial viewport height in physical pixels.
    pub height: u32,
    /// Display scale factor (e.g. 2.0 for Retina).
    pub scale_factor: f64,
    /// Optional theme configuration. When `None`, the default theme is used
    /// (if the `theme` feature is enabled).
    pub theme: Option<ThemeProviderProps>,
    /// Fonts (TTF/OTF bytes) to register **before** the component mounts, so the
    /// very first layout/paint pass has glyphs.
    ///
    /// This is the correct hook for environments with no system fonts — most
    /// importantly **wasm32/WebGPU**, where the platform provides no fallback
    /// font and text would otherwise render as zero glyphs. (Registering a font
    /// *after* [`RinchContext::new`] via [`register_font`](RinchContext::register_font)
    /// is too late for the initial mount.)
    pub fonts: Vec<&'static [u8]>,
}

// ── RinchContext ──────────────────────────────────────────────────────────────

/// The next [`RinchContext::sliced_frame`] stamp, shared by every context so
/// that a stamp names one frame of one context (#1976). Starts at 1: `0` is
/// "no frame".
static NEXT_SLICED_STAMP: AtomicU64 = AtomicU64::new(1);

/// Handle to an embedded rinch UI.
///
/// Created once during game initialization. The game loop calls [`update`]
/// each frame with platform events, then reads the Vello [`Scene`] via
/// [`scene`] for compositing.
///
/// [`update`]: RinchContext::update
/// [`scene`]: RinchContext::scene
pub struct RinchContext {
    app: RinchApp,
    size: (u32, u32),
    scale_factor: f64,
    dirty: Arc<AtomicBool>,
    /// When the focused editor's caret next toggles, as of the last
    /// [`update`](RinchContext::update) — see [`RinchContext::next_wake`].
    next_wake: Option<rinch_platform::Instant>,
    /// The IME state [`take_ime_change`](RinchContext::take_ime_change) last
    /// reported, which its next answer is the change from.
    ime_reported: ImeState,
    /// The `RenderSurface` ids in this context's document, as of the last
    /// [`scene`](RinchContext::scene): what its surface pass drives, and what
    /// `update` / `needs_update` ask about a new frame for.
    surface_ids: Vec<usize>,
    /// How many `scene()` calls ran the surface pass — see
    /// [`RinchContext::surface_pass_count`].
    surface_passes: u64,
    /// The stamp of the last frame [`sliced_frame`](Self::sliced_frame)
    /// painted, `0` before the first: unique across every context, so a
    /// [`RinchOverlayRenderer`] that holds a different one holds another
    /// frame's slices, or none (#1976).
    sliced_stamp: u64,
    /// This context's signal-change subscription. Multi-subscriber: several
    /// concurrent contexts (plus a mounted shell/web root) each hold their own,
    /// and dropping this context detaches only its own callback — creating or
    /// dropping context B never silences context A (issue #134).
    _signal_change_sub: rinch_core::SignalChangeSubscription,
}

/// Cross-thread dispatcher for an embedded context: queue only, no wake.
///
/// An embedded host's frame loop is already turning — it calls
/// [`RinchContext::update`] every frame, which drains the queue — so unlike the
/// desktop shell there is nothing here to wake.
fn queue_on_main_thread(f: Box<dyn FnOnce() + Send>) {
    rinch_core::queue_main_callback(f);
}

impl RinchContext {
    /// Create and mount a rinch UI.
    ///
    /// The component function runs immediately, building the initial DOM.
    /// Call this once during game initialization.
    ///
    /// On wasm32 it also installs the browser's timer backend and task
    /// scheduler (issue #1956), so `set_timeout`, `sleep` and `spawn_local`
    /// work with no `rinch-web` root on the page. A timer armed *before* the
    /// first context is created needs `rinch::install_web_runtime()` first.
    pub fn new<F>(config: RinchContextConfig, component: F) -> Self
    where
        F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    {
        let mut app = RinchApp::new(component);

        // Per-document theme (issue #138): generate this context's CSS from its
        // own config and hand it to the app before mount. The thread-global
        // theme slot is never touched — creating a second context can no longer
        // restyle a concurrent one (or a mounted shell/web root).
        #[cfg(feature = "theme")]
        {
            let css = match config.theme {
                Some(ref theme) => crate::generate_theme_css_string(theme),
                None => crate::generate_theme_css_string(&ThemeProviderProps::default()),
            };
            app.set_owned_theme_css(Some(css));
        }
        let _ = &config.theme; // suppress unused warning when theme feature is off

        // Namespace this context's stores/contexts under its document's
        // doc_key: two contexts creating the same store type no longer
        // overwrite each other, and lookups fall back to the thread-global
        // root 0 for stores created outside any context (issue #136).
        app.scope_context_to_doc = true;

        // Register any pre-mount fonts BEFORE mounting so the initial layout/paint
        // pass has glyphs (essential on wasm, which has no system fonts). These are
        // drained onto the document's render font context during mount_component.
        for font in &config.fonts {
            app.register_font_data(font);
        }

        let width = config.width.max(1);
        let height = config.height.max(1);
        let scale_factor = config.scale_factor;

        // Arm cross-thread signal dispatch. Both halves are required and they
        // are required together: `register_main_thread` is what *arms* the
        // check, so registering it without a dispatcher left a background-thread
        // `Signal::send()` / `update_send()` / `run_on_main_thread()` panicking
        // on the calling thread — strictly worse than registering neither, where
        // `is_main_thread()` answers `true` and `send()` degrades to `set()`
        // (issue #172).
        //
        // `_if_unset` so a context created inside a desktop app never displaces
        // the shell's dispatcher, which additionally wakes the winit event loop.
        // Both push onto the same `rinch-core` queue, so either dispatcher feeds
        // both drain sites.
        // Dispatcher first: a worker send between the two would otherwise
        // find a main thread registered and no dispatcher, and panic.
        //
        // Both before the mount (issue #1032, the CI failure of #1663): while no
        // main thread is registered `is_main_thread()` answers `true` on every
        // thread, so a timer armed by the component that came due before the
        // mount finished hopped to "the main thread" inline on the
        // `rinch-timer` thread, found nothing parked there, and was lost.
        rinch_core::set_cross_thread_dispatcher_if_unset(queue_on_main_thread);
        rinch_core::register_main_thread();

        // In a browser, the timer backend and task scheduler (issue #1956).
        // Native rinch-core has a timer thread of its own; a browser has no
        // threads, so without this a `set_timeout` (and a `sleep`, and every
        // `rinch-ws` / `rinch-rtc` timer) armed by this context's UI never
        // fired unless the page also mounted a `rinch-web` root, and a
        // `spawn_local` task was polled only by the next `update()`'s drain of
        // the main-thread queue. Now tasks are polled in a microtask, as under
        // `rinch-web`. Before the mount, so the component's own timers are
        // scheduled. Not the poll drain: `update()` does that once a frame,
        // and a second drain would sample every-frame polls twice.
        #[cfg(target_arch = "wasm32")]
        crate::web_runtime::install_web_scheduling();

        // Mount the component (builds DOM, runs initial layout) at the
        // *logical* viewport — `width`/`height` are physical (see
        // `rinch_platform::to_logical`). The display scale goes in first so
        // the initial style resolution sees the right `device_pixel_ratio`
        // (issue #211).
        app.set_device_pixel_ratio(scale_factor);
        let (logical_w, logical_h) = rinch_platform::to_logical((width, height), scale_factor);
        app.mount_component(logical_w as f32, logical_h as f32);

        // Subscribe to signal changes so the game loop can detect dirty state.
        // A guard-based subscription (not the legacy single slot): each context
        // observes independently, so N contexts can coexist (issue #134).
        let dirty = Arc::new(AtomicBool::new(false));
        let dirty_clone = dirty.clone();
        let signal_change_sub = rinch_core::subscribe_signal_change(move || {
            dirty_clone.store(true, Ordering::Release);
        });

        Self {
            app,
            size: (width, height),
            scale_factor,
            dirty,
            next_wake: None,
            ime_reported: ImeState::default(),
            surface_ids: Vec::new(),
            surface_passes: 0,
            sliced_stamp: 0,
            _signal_change_sub: signal_change_sub,
        }
    }

    /// Process input events and update layout. Call once per frame.
    ///
    /// Returns actions the game should handle (e.g. `SetCursor`, `Exit`).
    /// Input events that rinch doesn't consume are ignored.
    ///
    /// Also runs the housekeeping the desktop event loop does each turn,
    /// including the focused editor's caret blink: an event-driven host should
    /// call `update` again at [`next_wake`](RinchContext::next_wake) even with
    /// no input.
    pub fn update(&mut self, events: &[PlatformEvent]) -> Vec<AppAction> {
        let mut all_actions = Vec::new();

        // One `update` (plus the `scene` that follows it) is one performance
        // frame: close the previous one before this one's work starts.
        self.app.end_perf_frame();

        // Run the per-frame housekeeping the desktop runtime does ahead of each
        // paint, in the same order it does.
        //
        // Closures a background thread queued — `Signal::send()`,
        // `update_send()`, `run_on_main_thread()`, a fired `set_timeout`, an
        // `rinch-http`/`rinch-ws` completion — run first (issue #172). Then due
        // polled signals fire before anything reads them; without this an
        // embedded context never drained the registry at all, so a `poll_signal`
        // created inside a `RinchContext` never fired even once (issue #141).
        //
        // Draining before the events is deliberate: a queued write, a poll write
        // and an event handler can all dirty the tree, and this ordering lets the
        // single layout pass below absorb all of them.
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        // Work a background thread sent the app itself — a plain `<input>`
        // paste whose clipboard read answered (issue #328) — lands with the
        // queued writes, before this frame's events.
        let deferred_ran = self.app.run_deferred_work() > 0;

        // Process each event
        for event in events {
            let actions = self
                .app
                .handle_event(event.clone(), self.size, self.scale_factor);
            all_actions.extend(actions);
        }

        // Blink the focused editor's caret (issue #331) — the tick desktop runs
        // after every event-loop turn. Ahead of the layout pass below, so the
        // toggle's style write is resolved in this same update.
        if self.tick_caret_blink() && !all_actions.contains(&AppAction::RequestRedraw) {
            all_actions.push(AppAction::RequestRedraw);
        }

        // A `RenderSurface` frame submitted since the last scene — from its
        // render callback's previous run, another thread, a decoder — is
        // something to draw. `scene` collects it.
        // So is a surface that wants its render callback run again: a
        // continuous one, or one with a frame requested (#1768).
        if (crate::render_surface::inline_surfaces_dirty(&self.surface_ids)
            || crate::render_surface::surfaces_want_frame(&self.surface_ids))
            && !all_actions.contains(&AppAction::RequestRedraw)
        {
            all_actions.push(AppAction::RequestRedraw);
        }

        // If signals changed, resolve layout
        if self.dirty.swap(false, Ordering::AcqRel) || deferred_ran || self.app.has_dirty_nodes() {
            let (w, h) = self.logical_size();
            if self.app.resolve_and_repaint(w, h) {
                // Deduplicate RequestRedraw
                if !all_actions.contains(&AppAction::RequestRedraw) {
                    all_actions.push(AppAction::RequestRedraw);
                }
            }
        }

        // Process AboutToWait for transitions
        let transition_actions =
            self.app
                .handle_event(PlatformEvent::AboutToWait, self.size, self.scale_factor);
        all_actions.extend(transition_actions);

        all_actions
    }

    /// The last completed performance frame: every [`rinch_dom::perf`]
    /// counter for the previous `update` + `scene`.
    pub fn perf_last_frame(&self) -> rinch_dom::perf::FrameStats {
        self.app.last_frame_perf()
    }

    /// Every performance counter since the context was created or last
    /// [`reset_perf`](Self::reset_perf), the current frame included.
    pub fn perf_total(&self) -> rinch_dom::perf::FrameStats {
        self.app.total_perf()
    }

    /// Zero the performance counters.
    pub fn reset_perf(&mut self) {
        self.app.reset_perf();
    }

    /// Get the current Vello scene. Rebuilds only if dirty.
    ///
    /// The returned scene contains the full rinch UI, for a host that
    /// renders it with its own Vello setup. [`RinchOverlayRenderer::render`]
    /// takes the context instead, and does not use this.
    ///
    /// Each [`GameViewport`]'s box is cleared **in the scene** (#1866), which
    /// is approximate inside a clip: along a clipping ancestor's edges near the
    /// viewport (in each 16px tile the clip's outline crosses), Vello keeps
    /// what the clip saved, so a band of that ancestor's background can show
    /// over the game. `RinchOverlayRenderer::render` cuts the viewports out
    /// after Vello instead, exactly.
    ///
    /// **Use `rinch::vello`.** rinch depends on Vello 0.11 from
    /// `joeleaver/vello-fork` at a pinned commit (#1955), which fixes two
    /// image-atlas bugs of stock Vello 0.11. Build Vello types from `rinch::vello`
    /// (and its `peniko`/`kurbo`), and wgpu types from `rinch::wgpu`, instead of
    /// declaring your own `vello`/`wgpu` dependencies, so the versions can never
    /// drift (for an extra wgpu feature, keep `wgpu = "30"` with just that feature;
    /// Cargo unifies it). No `[patch]` line is needed.
    ///
    /// This context's mounted `RenderSurface`s are driven here, as the desktop
    /// paint drives them: each is told its layout size, its render callback
    /// runs, and its latest frame is painted inline at its box. Call it once
    /// per frame: **every** call runs the render callbacks again, so a second
    /// `scene()` in one frame renders each callback-driven surface twice.
    pub fn scene(&mut self) -> &Scene {
        // `build_scene`'s size reaches `paint_document` as its `viewport`, which
        // is in **logical** pixels — the unit every other caller passes, and the
        // unit it had to settle on once card K43 gave the parameter a job. This
        // one passed `self.size`, which is physical. That failed safe (an
        // already-physical size multiplied by the scale is a rect at least as
        // large as the real target, so the off-window cull and the
        // covers-the-window clip elision simply never fired) but it failed safe
        // by never optimising an embedded view at all, and it left one parameter
        // with two meanings. `logical_size` is the same conversion `update`
        // already uses.
        let (lw, lh) = self.logical_size();
        let surface_pixels = self.prepare_render_surfaces();
        let installed = !surface_pixels.is_empty();
        if installed {
            rinch_dom::paint::set_surface_pixels(Some(surface_pixels));
        }
        self.app
            .build_scene(self.scale_factor, (lw as u32, lh as u32));
        if installed {
            rinch_dom::paint::set_surface_pixels(None);
        }
        self.app.painter.scene()
    }

    /// [`Self::scene`] painted for [`RinchOverlayRenderer::render`] (#1866):
    /// each `GameViewport` split out of the scene as a base layer with no
    /// game, for the split compositor to cut exactly. The frame's slices and
    /// its stamp when it was painted, `None` when the last one is still
    /// current; the scene is `self.app.painter`'s.
    ///
    /// `fed` is the stamp of the frame whose slices the calling renderer
    /// holds (`0`: none). Unless it is this context's last sliced frame, the
    /// frame is painted whether or not anything changed (#1976): a new
    /// renderer (the host's step after a device loss), a renderer that
    /// another one took a turn from, or one that last drew another context
    /// holds no slices of the scene it is about to render, and the cached
    /// scene alone has nothing painted after its viewports.
    fn sliced_frame(
        &mut self,
        fed: u64,
    ) -> Option<(u64, Vec<rinch_dom::paint::slicing::SurfaceSlice>)> {
        if fed == 0 || fed != self.sliced_stamp {
            self.app.request_repaint();
        }
        let (lw, lh) = self.logical_size();
        let surface_pixels = self.prepare_render_surfaces();
        let installed = !surface_pixels.is_empty();
        if installed {
            rinch_dom::paint::set_surface_pixels(Some(surface_pixels));
        }
        let slices = self
            .app
            .build_scene_with(self.scale_factor, (lw as u32, lh as u32), true);
        if installed {
            rinch_dom::paint::set_surface_pixels(None);
        }
        let slices = slices?;
        self.sliced_stamp = NEXT_SLICED_STAMP.fetch_add(1, Ordering::Relaxed);
        Some((self.sliced_stamp, slices))
    }

    /// Drive this context's mounted `RenderSurface`s for this frame and return their
    /// pixels for inline painting — the desktop paint's surface steps, in its
    /// order (issue #331): tell each surface its **physical** layout size, run
    /// the render callbacks (which skip a surface not yet measured), then
    /// collect the frames. A surface that delivered a frame since the last
    /// scene marks the scene dirty; one that did not is still installed, so a
    /// scene rebuilt for some other reason draws its last frame.
    ///
    /// Only the inline path. The compositor path — video, and a surface
    /// registered under a `data-viewport` name — is the host's to composite in
    /// an embedded context, like a `GameViewport` hole, and a
    /// `GpuTextureRegistrar` texture is not read back here: the host owns the
    /// device (see the game-engine guide).
    ///
    /// Only **this** context's surfaces — the ones in its own document. The
    /// registry is thread-global, and a pass over every surface would run
    /// another context's render callbacks and take its fresh frame, leaving
    /// that context's own `scene()` cached with the old one.
    ///
    /// A thread with no inline surface registered skips the pass, and with it
    /// the document walk that finds the surfaces: every step would be a no-op,
    /// and the walk is O(document) on a `scene()` that is otherwise a cache
    /// hit.
    fn prepare_render_surfaces(
        &mut self,
    ) -> std::collections::HashMap<usize, rinch_dom::paint::SurfacePixelData> {
        use crate::render_surface as rs;
        if !rs::any_inline_surface_registered() {
            self.surface_ids.clear();
            return Default::default();
        }
        self.surface_passes += 1;
        let s = self.scale_factor as f32;
        self.surface_ids.clear();
        self.surface_ids = self.app.update_render_surface_layouts(s, false);
        rs::invoke_render_callbacks_for(&self.surface_ids);
        let (pixels, fresh) = rs::collect_surface_pixels_for(&self.surface_ids);
        if !fresh.is_empty() {
            self.app.request_repaint();
        }
        pixels
    }

    /// How many [`scene`](RinchContext::scene) calls have run the
    /// `RenderSurface` pass, which walks the document to find this context's
    /// surfaces. It is skipped while the thread has no surface registered, so
    /// that a `scene()` on an unchanged document stays a cache hit. A
    /// structural test hook, not a stable API.
    #[doc(hidden)]
    pub fn surface_pass_count(&self) -> u64 {
        self.surface_passes
    }

    /// Advance the focused editor's caret blink and record when it next
    /// toggles. Returns whether the caret toggled (a redraw is owed).
    #[cfg(feature = "desktop")]
    fn tick_caret_blink(&mut self) -> bool {
        let focused = self.app.blinking_editor_id();
        match crate::editor::caret_blink_tick(self.app.doc_key(), focused) {
            Some(blink) => {
                self.next_wake = Some(rinch_platform::Instant::now() + blink.next);
                blink.redraw
            }
            None => {
                self.next_wake = None;
                false
            }
        }
    }

    /// No rich-text editor without `desktop`, so nothing blinks.
    #[cfg(not(feature = "desktop"))]
    fn tick_caret_blink(&mut self) -> bool {
        false
    }

    /// When this context next needs an [`update`](RinchContext::update) with
    /// no input, or `None` when nothing is scheduled.
    ///
    /// Today that is the focused editor's caret blink (issue #331), or a
    /// pointer resting over an element with a `title`, whose tooltip shows at
    /// that instant (issue #1646): an
    /// editor's caret toggles every 530 ms while it has the keyboard, and it
    /// is what the desktop runtime arms its only timed wake
    /// (`ControlFlow::WaitUntil`) with. A host that renders continuously can
    /// ignore this — every `update` ticks the blink. An event-driven host
    /// schedules a wake for this instant, or its caret toggles only when some
    /// other event happens to arrive. [`needs_update`](RinchContext::needs_update)
    /// answers `true` once it has passed.
    ///
    /// As of the last `update`: it moves when the caret moves (the blink
    /// restarts solid) and becomes `None` when the editor loses the keyboard or
    /// the host window reports `WindowFocus(false)`. A running transition or
    /// animation is not a wake — `update` answers `RequestRedraw` for each of
    /// its frames instead.
    pub fn next_wake(&self) -> Option<rinch_platform::Instant> {
        match (self.next_wake, self.app.title_tooltip_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Notify rinch the window was resized.
    ///
    /// Call this when the game window size changes. Dimensions are in
    /// physical pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.size = (width, height);
        // Layout is resolved in logical (CSS) pixels — `update` already uses
        // `logical_size()` for exactly this, and paint scales the result back up
        // by `scale_factor`. Re-laying out at the physical size here made the
        // document `scale_factor` times too wide until the next signal change.
        let (lw, lh) = rinch_platform::to_logical((width, height), self.scale_factor);
        self.app.resize_layout(lw, lh);
    }

    /// Update the display scale factor (DPI).
    ///
    /// Pushes the new scale into Stylo's `device_pixel_ratio` (issue #211),
    /// marking the document style-dirty when the value changed — the next
    /// [`update`](RinchContext::update) re-resolves at the new logical size
    /// (physical size ÷ scale), so `resolution`-gated styles and the layout
    /// viewport change together.
    pub fn set_scale_factor(&mut self, scale: f64) {
        self.scale_factor = scale;
        self.app.set_device_pixel_ratio(scale);
    }

    /// Replace this context's theme at runtime.
    ///
    /// Regenerates the per-document theme CSS from `theme` and restyles this
    /// context's document on its next [`update`](RinchContext::update). Other
    /// concurrent contexts — and a mounted shell/web root following the
    /// thread-global theme — are untouched (issue #138).
    ///
    /// **Caveat:** an embedded `ThemeProvider` component with a reactive
    /// `dark_mode_fn`/`primary_color_fn` writes the *thread-global* theme slot,
    /// which embed contexts deliberately ignore — this method is the supported
    /// path for changing an embedded context's theme.
    #[cfg(feature = "theme")]
    pub fn set_theme(&mut self, theme: &ThemeProviderProps) {
        self.app
            .set_owned_theme_css(Some(crate::generate_theme_css_string(theme)));
        // Wake the next update() — resolve_and_repaint detects the owned-CSS
        // change against its cached last_theme_key and does the full restyle,
        // exactly like the shell's thread-slot comparison path.
        self.dirty.store(true, Ordering::Release);
    }

    /// No-op when the theme feature is disabled.
    #[cfg(not(feature = "theme"))]
    pub fn set_theme(&mut self, _theme: &ThemeProviderProps) {}

    /// Query the layout rect of a [`GameViewport`] component by name.
    ///
    /// Returns the absolute position and size in logical pixels, or `None`
    /// if no viewport with that name exists.
    pub fn viewport_rect(&self, name: &str) -> Option<LayoutRect> {
        self.app.viewport_rect(name).map(|(x, y, w, h)| LayoutRect {
            x,
            y,
            width: w,
            height: h,
        })
    }

    /// Returns `true` if the point `(x, y)` (in logical pixels) hits a
    /// rinch UI element rather than a [`GameViewport`] hole.
    ///
    /// When this returns `true`, the game should let rinch handle the
    /// input. When `false`, the game should handle it (e.g. camera,
    /// gameplay).
    pub fn wants_mouse(&self, x: f32, y: f32) -> bool {
        let Some(doc) = self.app.doc() else {
            return false;
        };
        let d = doc.borrow();

        // Hit-test the rinch DOM
        let Some(hit_node_id) = hit_test(&d.tree, x, y) else {
            return false;
        };

        // Walk up from the hit node — if we find a data-viewport attribute,
        // the click is in the game area, not rinch UI.
        let mut current = Some(hit_node_id);
        while let Some(id) = current {
            if let Some(node) = d.tree.get(id) {
                if node.attributes.contains_key("data-viewport") {
                    return false;
                }
                current = node.parent;
            } else {
                break;
            }
        }

        true
    }

    /// Returns `true` if a text input, contenteditable, or a generic
    /// focusable node that registered its own key handling
    /// ([`FocusEntry::on_key`](crate::focus_registry::FocusEntry::on_key),
    /// issue #228) is focused.
    ///
    /// When this returns `true`, the game should route keyboard events to
    /// rinch instead of handling them as game input.
    ///
    /// **A plain `<button>` or `<a href>` the user just clicked does *not*
    /// count** (issue #548). Such a tag is focusable with no `tabindex`
    /// needed (issue #252), so a mouse click alone claims rinch's keyboard
    /// focus — but with no registered [`on_key`][k], the button only ever
    /// consumes Enter/Space through the runtime's own activation path, and
    /// that is not reason enough for a host to give up its own Esc/hotkey
    /// handling. A *registered* generic node — a custom widget that reads
    /// arrow keys or its own shortcuts via `on_key` — still counts, Tab-focused
    /// or mouse-clicked alike: the runtime cannot tell you which keys it
    /// wants without seeing them. See
    /// [`RinchApp::has_focused_key_consumer`](crate::app::RinchApp::has_focused_key_consumer)
    /// for that half on its own, and
    /// [`RinchApp::has_focused_node`](crate::app::RinchApp::has_focused_node) for
    /// "is *any* generic node focused" if you need the coarser question for
    /// something else (it is the wrong one for this).
    ///
    /// [k]: crate::focus_registry::FocusEntry::on_key
    pub fn wants_keyboard(&self) -> bool {
        self.app.has_focused_input()
            || self.app.has_focused_contenteditable()
            || self.app.has_focused_key_consumer()
    }

    /// The IME state the focused target wants: whether the host's platform
    /// input method should be on, and where its candidate box goes, in
    /// **logical** pixels of this context (multiply by the scale factor for
    /// physical). The same answer the desktop shell applies to its window
    /// after every event-loop turn: on for a focused `<input>`/`<textarea>`,
    /// an editor that is not read-only, or a node registered with
    /// [`FocusEntry::on_ime`](crate::focus_registry::FocusEntry::on_ime); off
    /// for anything else and while the host reported the window blurred
    /// (`PlatformEvent::WindowFocus(false)`). The caret area is where the caret
    /// is painted, a field's horizontal scroll and transforms included.
    ///
    /// This is a different question from [`wants_keyboard`](Self::wants_keyboard):
    /// a node that reads keys but takes no text wants the keyboard and no IME.
    pub fn ime_state(&self) -> ImeState {
        self.app.ime_state()
    }

    /// The request that brings the host's input method from what this method
    /// last reported to [`ime_state`](Self::ime_state), or `None` when nothing
    /// changed. Call it after each [`update`](Self::update) and apply the
    /// answer: [`ImeChange::Enable`] / [`ImeChange::Disable`] switch the
    /// platform input method on or off, [`ImeChange::Update`] moves its
    /// candidate box. It is the decision the desktop shell makes for its
    /// window, so a host that applies every answer tracks the IME as a desktop window
    /// does. Composition comes back in through [`update`](Self::update) as
    /// `PlatformEvent::Ime` events, which go to the focused target.
    pub fn take_ime_change(&mut self) -> Option<ImeChange> {
        let desired = self.app.ime_state();
        self.ime_reported.change_to(desired)
    }

    /// Register font data for text rendering **after** the initial mount.
    ///
    /// Useful in environments without system fonts (e.g. WASM, embedded).
    /// The font applies to subsequent layout/paint passes, not the mount that
    /// already happened inside [`new`](RinchContext::new). To have glyphs on the
    /// **first** frame — which is mandatory on wasm/WebGPU, where there is no
    /// system fallback font — supply the font via
    /// [`RinchContextConfig::fonts`] instead.
    ///
    /// The data should be a TrueType (`.ttf`) or OpenType (`.otf`) font file.
    pub fn register_font(&mut self, data: &'static [u8]) {
        self.app.register_font_data(data);
    }

    /// Whether the UI needs a repaint.
    ///
    /// Useful for game engines that want to skip rendering unchanged frames.
    pub fn needs_update(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
            || self.app.has_dirty_nodes()
            || self.app.has_deferred_work()
            || crate::render_surface::inline_surfaces_dirty(&self.surface_ids)
            || crate::render_surface::surfaces_want_frame(&self.surface_ids)
            || self
                .next_wake()
                .is_some_and(|t| rinch_platform::Instant::now() >= t)
    }

    /// Tell rinch the pointer left the UI's surface (the window, or the
    /// region of it the host routes to rinch). There is no `PlatformEvent`
    /// for this: a host must call it, or a `title` tooltip (issue #1646)
    /// shown or waiting when the pointer left stays until the next press,
    /// key or move. Returns whether a frame is owed (a shown tooltip went).
    ///
    /// To turn tooltips off altogether, call
    /// `app_mut().set_title_tooltips(false)`.
    pub fn pointer_left(&mut self) -> bool {
        let shown = self.app.title_tooltip_text().is_some();
        self.app.hide_title_tooltip();
        shown
    }

    /// Access the underlying `RinchApp` for advanced use cases.
    pub fn app(&self) -> &RinchApp {
        &self.app
    }

    /// Mutable access to the underlying `RinchApp`.
    pub fn app_mut(&mut self) -> &mut RinchApp {
        &mut self.app
    }

    fn logical_size(&self) -> (f32, f32) {
        let (w, h) = rinch_platform::to_logical(self.size, self.scale_factor);
        (w as f32, h as f32)
    }
}

impl Drop for RinchContext {
    fn drop(&mut self) {
        // Clear this context's store/context namespace so its stores don't
        // outlive it (issue #136). The thread-global root 0 — and every other
        // context's namespace — is untouched.
        let key = self.app.doc_key();
        if key != 0 {
            rinch_core::clear_context_for_root(key);
        }
    }
}

// ── Debug integration ───────────────────────────────────────────────────────

/// Re-export screenshot utilities for game engine integration.
///
/// The two gates differ because the items do: `encode_png` is plain CPU work,
/// while `capture_texture_rgba` reads back a wgpu texture and is
/// `#[cfg(feature = "gpu")]`. Re-exporting both on `debug` alone meant `rinch`
/// with `debug` + `embed` and *without* `gpu` did not compile at all — a
/// combination nothing in the workspace or CI had ever built, because
/// `cargo test --workspace` unifies `rinch/gpu` on from `examples/game-embed`.
/// Found by naming `debug` on CI's gated-test line for #401's pins.
#[cfg(all(feature = "debug", feature = "gpu"))]
pub use crate::shell::screenshot::capture_texture_rgba;
#[cfg(feature = "debug")]
pub use crate::shell::screenshot::encode_png;

/// A pending screenshot request from the debug server.
///
/// Returned by [`RinchContext::process_debug_commands`]. Call
/// [`respond`](ScreenshotRequest::respond) with the captured pixel data.
#[cfg(feature = "debug")]
pub struct ScreenshotRequest {
    response_tx: std::sync::mpsc::Sender<rinch_debug::DebugResult>,
}

#[cfg(feature = "debug")]
impl ScreenshotRequest {
    /// Send a successful screenshot (raw RGBA pixels → PNG encoded internally).
    pub fn respond(self, width: u32, height: u32, rgba: Vec<u8>) {
        use base64::Engine;
        let png_bytes = crate::shell::screenshot::encode_png(&rgba, width, height);
        let _ = self.response_tx.send(rinch_debug::DebugResult::Bytes {
            data: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
        });
    }

    /// Send an error response.
    pub fn fail(self, message: String) {
        let _ = self
            .response_tx
            .send(rinch_debug::DebugResult::Error { message });
    }
}

#[cfg(feature = "debug")]
impl RinchContext {
    /// Attach the debug server for MCP tool integration.
    ///
    /// The `notify` callback is invoked (from a background thread) when a
    /// debug command arrives. Use it to wake the event loop so commands are
    /// processed promptly. In polling game loops this can be a no-op.
    pub fn attach_debug(
        &mut self,
        app_name: &str,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<()> {
        let (server, rx) = rinch_debug::attach(app_name, notify)?;
        self.app._debug_server = Some(server);
        self.app.debug_cmd_rx = Some(rx);
        Ok(())
    }

    /// Process pending debug commands.
    ///
    /// Non-screenshot commands (DOM queries, clicks, typing, etc.) are
    /// handled internally. Screenshot requests are returned so the game
    /// can capture the composited frame and respond via
    /// [`ScreenshotRequest::respond`].
    ///
    /// Call this once per frame, after rendering but before presenting.
    pub fn process_debug_commands(&mut self) -> Vec<ScreenshotRequest> {
        use rinch_debug::DebugCommandKind;

        let Some(rx) = self.app.debug_cmd_rx.take() else {
            return Vec::new();
        };

        let mut pending = Vec::new();
        while let Ok(cmd) = rx.0.try_recv() {
            pending.push(cmd);
        }

        let mut screenshots = Vec::new();

        for cmd in pending {
            match cmd.kind {
                DebugCommandKind::Screenshot => {
                    screenshots.push(ScreenshotRequest {
                        response_tx: cmd.response_tx,
                    });
                }
                other => {
                    let mut actions = Vec::new();
                    let result = self.app.execute_debug_command(
                        other,
                        &mut actions,
                        self.scale_factor,
                        self.size,
                    );
                    if actions.contains(&AppAction::RequestRedraw) {
                        self.dirty.store(true, Ordering::Release);
                    }
                    let _ = cmd.response_tx.send(result);
                }
            }
        }

        self.app.debug_cmd_rx = Some(rx);
        screenshots
    }
}

// ── RinchOverlayRenderer ─────────────────────────────────────────────────────

/// Convenience helper that renders a Vello [`Scene`] to a GPU texture.
///
/// Game engines with their own Vello setup can skip this and render the
/// scene directly. This struct manages a [`vello::Renderer`] and an
/// intermediate render texture sized to the window.
///
/// **The texture holds straight (non-premultiplied) alpha**, which is what
/// Vello writes: composite it over your game with straight-alpha blending
/// (`wgpu::BlendState::ALPHA_BLENDING`, `src * a + dst * (1 - a)`), or
/// premultiply in your shader first. `PREMULTIPLIED_ALPHA_BLENDING` on the
/// raw texture adds the full colour of every translucent pixel. Where a
/// [`GameViewport`] is, the texture is transparent (#1866).
///
/// **Use `rinch::vello`.** rinch depends on Vello 0.11 from
/// `joeleaver/vello-fork` at a pinned commit (#1955), which fixes two
/// image-atlas bugs of stock Vello 0.11. Build Vello types from `rinch::vello`
/// (and its `peniko`/`kurbo`), and wgpu types from `rinch::wgpu`, instead of
/// declaring your own `vello`/`wgpu` dependencies, so the versions can never
/// drift (for an extra wgpu feature, keep `wgpu = "30"` with just that feature;
/// Cargo unifies it). No `[patch]` line is needed.
///
/// # Usage
///
/// ```ignore
/// let mut overlay = RinchOverlayRenderer::new(&device, 1280, 720, TextureFormat::Rgba8Unorm);
///
/// // Each frame:
/// let view = overlay.render(&device, &queue, &mut ctx);
/// // composite `view` on top of your game scene
/// ```
pub struct RinchOverlayRenderer {
    renderer: VelloRenderer,
    render_texture: Texture,
    width: u32,
    height: u32,
    format: TextureFormat,
    /// The `GameViewport`s split out of the last painted frame (#1866).
    split: crate::split_frame::SplitFrame,
    compositor: Option<crate::split_compositor::SplitCompositor>,
    /// The stamp of the context frame whose slices `split` holds, `0` for
    /// none: a context asked by a renderer holding another stamp paints the
    /// frame again for it (#1976).
    fed: u64,
    /// A failed slice apply was logged and none has succeeded since: a
    /// persistent error logs once, not every frame.
    slice_error_logged: bool,
    /// Where Vello renders the UI while a viewport is split: the split
    /// compositor then writes `render_texture` from it.
    below: Option<Texture>,
}

impl RinchOverlayRenderer {
    /// Create a new overlay renderer from the game engine's wgpu device.
    ///
    /// The `format` should match the texture format you'll composite with
    /// (typically `Rgba8Unorm` or `Bgra8Unorm`).
    pub fn new(device: &Device, width: u32, height: u32, format: TextureFormat) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let renderer = VelloRenderer::new(
            device,
            RendererOptions {
                antialiasing_support: AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("Failed to create Vello renderer");

        let render_texture = Self::create_texture(device, format, width, height);

        Self {
            renderer,
            render_texture,
            width,
            height,
            format,
            split: Default::default(),
            compositor: None,
            fed: 0,
            slice_error_logged: false,
            below: None,
        }
    }

    /// Render `ctx`'s UI to a texture.
    ///
    /// Returns a [`TextureView`] that the game can sample/composite.
    /// The texture has a transparent background so it can be alpha-blended
    /// over the game scene; its alpha is straight, not premultiplied (see the
    /// type's docs).
    ///
    /// Each [`GameViewport`] is the base layer (#1866): the texture is
    /// transparent inside its box (its own `border-radius`, clipped and
    /// transformed as the element is), and what is painted after it — its
    /// children, later siblings, popups — is drawn over it. The box is cut
    /// out by a compositor pass after Vello, from the frame's slices, so it is
    /// exact under every clip around the viewport. (Before #1866 this took
    /// `ctx.scene()`; a scene rendered on its own clears the viewports in the
    /// scene, which is approximate inside clips, see [`RinchContext::scene`].)
    ///
    /// Call it once per frame, in place of `ctx.scene()`: it drives the
    /// context's `RenderSurface`s as `scene()` does.
    ///
    /// A renderer may be replaced (after a device loss: the renderer is
    /// bound to its device, the context is not), and several may take turns
    /// on one context or one on several contexts: a renderer that does not
    /// hold the context's last painted frame has it painted again, so its
    /// first `render` is complete with nothing changed in the document
    /// (#1976). Nothing needs marking dirty.
    pub fn render(
        &mut self,
        device: &Device,
        queue: &Queue,
        ctx: &mut RinchContext,
    ) -> TextureView {
        if let Some((stamp, slices)) = ctx.sliced_frame(self.fed) {
            self.fed = stamp;
            if let Err(e) = self.split.apply(
                device,
                queue,
                &mut self.renderer,
                crate::split_frame::SplitChange {
                    drop: Vec::new(),
                    all: true,
                    add: slices,
                },
            ) {
                if !self.slice_error_logged {
                    tracing::error!("rinch: overlay split slices: {e}");
                    self.slice_error_logged = true;
                }
                self.split.clear();
                // Holding none of the frame's slices: ask for it again.
                self.fed = 0;
            } else {
                self.slice_error_logged = false;
            }
        }
        let scene = ctx.app.painter.scene();
        let view = self
            .render_texture
            .create_view(&TextureViewDescriptor::default());
        let split = !self.split.is_empty();
        let below = if split {
            let (w, h) = (self.width, self.height);
            let below = self
                .below
                .take()
                .filter(|t| t.width() == w && t.height() == h)
                .unwrap_or_else(|| {
                    device.create_texture(&TextureDescriptor {
                        label: Some("rinch overlay below"),
                        size: Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: TextureDimension::D2,
                        format: TextureFormat::Rgba8Unorm,
                        usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                });
            let below_view = below.create_view(&TextureViewDescriptor::default());
            self.below = Some(below);
            Some(below_view)
        } else {
            self.below = None;
            None
        };

        self.renderer
            .render_to_texture(
                device,
                queue,
                scene,
                below.as_ref().unwrap_or(&view),
                &RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: self.width,
                    height: self.height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .expect("Vello render_to_texture failed");

        if let Some(below) = below {
            let format = self.format;
            let compositor = self.compositor.get_or_insert_with(|| {
                crate::split_compositor::SplitCompositor::new(device, format)
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rinch overlay split"),
            });
            self.split.composite_with(
                compositor,
                device,
                &mut encoder,
                &below,
                &view,
                (self.width, self.height),
                &Default::default(),
                true,
            );
            queue.submit(Some(encoder.finish()));
        }

        view
    }

    /// Resize the render target. Call when the window size changes.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.render_texture = Self::create_texture(device, self.format, width, height);
    }

    /// Get the current render texture (e.g. for binding to a shader).
    pub fn texture(&self) -> &Texture {
        &self.render_texture
    }

    /// Current width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Current height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    fn create_texture(device: &Device, format: TextureFormat, width: u32, height: u32) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("rinch overlay texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                // The split compositor writes it (#1866).
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    }
}

// ── GameViewport component ───────────────────────────────────────────────────

/// A transparent placeholder that marks a region for game rendering.
///
/// Place this component in your rinch UI layout wherever you want the game
/// scene to show through. Use [`RinchContext::viewport_rect`] to query
/// the computed layout rect and render your game into that region.
///
/// The viewport is the **base layer** (#1866): its box (its own
/// `border-radius`, clipped and transformed as the element is) is
/// transparent in [`RinchOverlayRenderer::render`]'s texture, so nothing
/// painted beneath it — ancestor backgrounds, earlier siblings — covers the
/// game, and anything painted after it — its children, later siblings,
/// popups — draws over it. ([`RinchContext::scene`] clears it in the scene,
/// approximate inside clips.) A debug build warns once
/// where that cannot hold: the viewport inside an `opacity < 1`, `mask-image`
/// or `filter` group, or under a rotation, scale or skew.
///
/// The component renders as a transparent, **hittable** `div`. Hittability is
/// what routes input: [`RinchContext::wants_mouse`] hit-tests the DOM and walks
/// up from the hit node looking for `data-viewport`, so a pointer that lands on
/// the hole answers "the game owns this" (issue #207). The transparent
/// background and `pointer-events: auto` come from the UA stylesheet rule for
/// `[data-viewport]`, not from an inline style, so passing `style:` (which
/// *replaces* the style attribute) can neither restore nor break them.
///
/// To hand a region back to the UI, declare `pointer-events: none` on the
/// viewport yourself — an author declaration still wins over the UA default.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
/// use rinch::embed::GameViewport;
///
/// #[component]
/// fn game_ui() -> NodeHandle {
///     rsx! {
///         div { style: "display: flex; flex-direction: column; height: 100%;",
///             div { class: "toolbar", "Tools here" }
///             div { style: "display: flex; flex: 1;",
///                 div { style: "width: 200px;", "Side panel" }
///                 GameViewport { name: "main", style: "flex: 1;" }
///             }
///             div { class: "status-bar", "Status" }
///         }
///     }
/// }
/// ```
pub fn game_viewport(__scope: &mut RenderScope, name: &str) -> NodeHandle {
    let div = __scope.create_element("div");
    div.set_attribute("class", "rinch-game-viewport");
    div.set_attribute("data-viewport", name);
    div
}

/// Component struct for `GameViewport` — use in RSX as `GameViewport { name: "main" }`.
#[derive(Debug, Default)]
pub struct GameViewport {
    /// Name used to query this viewport's rect via [`RinchContext::viewport_rect`].
    pub name: String,
}

impl rinch_core::Component for GameViewport {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        game_viewport(scope, &self.name)
    }
}
