//! Per-frame performance counters.
//!
//! Every [`NodeTree`](crate::NodeTree) carries a [`PerfCounters`] in its
//! `perf` field: a fixed array of plain `u64` counters, one per [`Counter`],
//! bumped by the style, layout, text, paint and hit-testing code as it runs.
//! They are always compiled in — an increment is a load and a store into a
//! `Cell` the tree already owns, with no allocation and no branch on a flag —
//! so a performance fix can be proven with a deterministic before/after
//! number instead of a timing that moves with the host's load.
//!
//! The counters are split into a **current frame** and two accumulators.
//! Whoever owns the frame loop — the desktop shell after each present, the
//! embed context after each `update`, a test whenever it likes — calls
//! [`PerfCounters::end_frame`], which folds the current frame into the
//! running total, keeps it as the last completed frame and starts a new one.
//! A document driven by nothing (a bare `RinchDocument` in a unit test) never
//! ends a frame, so [`PerfCounters::frame`] then simply accumulates
//! everything since the last [`PerfCounters::reset`].
//!
//! The `Time*Ns` counters are wall-clock nanoseconds summed over the frame.
//! They are measured with one `Instant::now()` pair per *phase call* — each
//! `resolve_styles`, `apply_stylo_styles_to_taffy`, `resolve_layout`, paint
//! and present — and never per node. A phase can run several times a frame
//! (every DOM insertion calls the two style phases synchronously), so that is
//! a handful of clock reads per insertion, not one per frame. They are the only
//! counters whose values are not deterministic.
//!
//! `RINCH_PERF` (any value) prints one summary line per ended frame to
//! stderr: the phase times and every non-zero counter. The variable is read
//! once per process ([`perf_env_enabled`]).
//!
//! Guide: `docs/src/guide/performance.md`.

use std::cell::Cell;
use std::fmt;
use std::sync::OnceLock;

macro_rules! define_counters {
    ($( $(#[doc = $doc:literal])* $var:ident = $name:literal, )*) => {
        /// One per-frame performance counter. See the [module docs](self).
        ///
        /// The discriminant indexes [`PerfCounters`]' array; `name()` is the
        /// stable `snake_case` key used in `RINCH_PERF` output and the
        /// `perf_stats` debug command's JSON.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum Counter {
            $( $(#[doc = $doc])* $var, )*
        }

        impl Counter {
            /// Every counter, in declaration (and array) order.
            pub const ALL: &'static [Counter] = &[$(Counter::$var,)*];
            /// How many counters there are.
            pub const COUNT: usize = Self::ALL.len();

            /// The stable `snake_case` name of this counter.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Counter::$var => $name,)*
                }
            }

            /// Look a counter up by its [`name`](Self::name).
            pub fn from_name(name: &str) -> Option<Counter> {
                match name {
                    $($name => Some(Counter::$var),)*
                    _ => None,
                }
            }
        }
    };
}

define_counters! {
    // ── Style ──────────────────────────────────────────────────────────
    /// `resolve_styles` calls, including the synchronous ones every DOM
    /// insertion makes.
    StyleResolves = "style_resolves",
    /// Elements that went through a full selector match and cascade.
    ElementsCascaded = "elements_cascaded",
    /// Nodes the style walk visited, cached ones included. The gap between
    /// this and `elements_cascaded` is the walk's overhead.
    StyleNodesVisited = "style_nodes_visited",
    /// `::before` / `::after` resolution attempts (two per cascaded element).
    PseudoElementPasses = "pseudo_element_passes",
    /// Full-document restyle *requests*, any reason. Each is also counted
    /// under one of the `full_restyle_*` reasons below. Two requests can land
    /// in one walk (a theme restyle that also changes the root font-size counts
    /// twice), so this can exceed the number of full walks actually made.
    FullRestyles = "full_restyles",
    /// ...because the viewport size changed (`resolve_layout`).
    FullRestyleViewport = "full_restyle_viewport",
    /// ...because the theme or another whole-document style input changed
    /// (`recompute_all_styles_full`).
    FullRestyleTheme = "full_restyle_theme",
    /// ...because a `<style>` element's text changed (`maybe_load_style_css`).
    FullRestyleStylesheet = "full_restyle_stylesheet",
    /// ...because the device pixel ratio changed.
    FullRestyleDpr = "full_restyle_dpr",
    /// ...because the root element's font-size (the `rem` basis) changed.
    FullRestyleRootFontSize = "full_restyle_root_font_size",
    /// `resolve_styles` walked the whole document from `<html>` because no
    /// style root was recorded or the first layout has not completed yet.
    FullStyleWalks = "full_style_walks",
    /// Nodes `apply_stylo_styles_to_taffy` processed.
    TaffyStyleSyncs = "taffy_style_syncs",
    /// ...of which the resulting Taffy style differed and was written.
    TaffyStyleChanges = "taffy_style_changes",

    // ── Text ───────────────────────────────────────────────────────────
    /// Parley layouts built by the Taffy measure function for an IFC root.
    ShapeMeasureIfc = "shape_measure_ifc",
    /// Parley layouts built by the Taffy measure function for a text leaf.
    ShapeMeasureText = "shape_measure_text",
    /// Parley layouts built by `build_ifc_layouts` (the layouts paint uses).
    ShapeIfcBuild = "shape_ifc_build",
    /// Parley layouts built while sizing an atomic inline
    /// (`inline-block` / `inline-flex` / `inline-grid`).
    ShapeAtomicInline = "shape_atomic_inline",
    /// `text-overflow: ellipsis` truncations. Each one shapes several
    /// candidate layouts (a binary search over the prefix length).
    EllipsisBuilds = "ellipsis_builds",
    /// Parley layouts built by paint itself: every `<input>` / `<textarea>`
    /// value and `<select>` label, every frame they are painted, plus the
    /// fallback for text with no cached layout. (Query-time shaping — hit
    /// testing an input, the MCP text tools — is not counted.)
    ShapePaint = "shape_paint",
    /// IFC measures served from `ifc_measure_cache` without shaping.
    IfcMeasureCacheHits = "ifc_measure_cache_hits",
    /// Per-root invalidations of the IFC measure cache by a restyle or a
    /// content change (`NodeTree::forget_ifc_measures`, O(1) each).
    IfcMeasureInvalidations = "ifc_measure_invalidations",
    /// IFC roots the structural pass found new or changed (their content
    /// signature moved), and so dropped the measures and paint layout of.
    /// The roots it left alone keep both.
    IfcSignatureChanges = "ifc_signature_changes",

    // ── Layout ─────────────────────────────────────────────────────────
    /// `resolve_layout` calls.
    LayoutResolves = "layout_resolves",
    /// ...that skipped Taffy because nothing layout-affecting changed and
    /// no text needed reshaping (paint-only).
    LayoutSkippedPaintOnly = "layout_skipped_paint_only",
    /// ...that skipped Taffy but rebuilt dirty IFC text layouts.
    LayoutSkippedTextOnly = "layout_skipped_text_only",
    /// Whole-document IFC structural setup passes (the `ifc_dirty` branch:
    /// `sync_display_contents` + `setup_inline_formatting_contexts` +
    /// `sync_text_contexts` + `compute_inline_block_layouts`). Same semantics as
    /// the never-reset `NodeTree::ifc_setup_passes` counter (#875), per frame.
    IfcSetupPasses = "ifc_setup_passes",
    /// Root Taffy computes (`run_taffy_compute`). Mirrors
    /// `NodeTree::taffy_computes`.
    TaffyRootComputes = "taffy_root_computes",
    /// Taffy measure-function invocations during root computes.
    TaffyMeasureCalls = "taffy_measure_calls",
    /// Standalone Taffy computes sizing an atomic inline outside the root
    /// compute.
    InlineBlockComputes = "inline_block_computes",
    /// Extra root computes the `calc(%, px)` fixpoint ran.
    CalcFixpointPasses = "calc_fixpoint_passes",

    // ── Paint ──────────────────────────────────────────────────────────
    /// Frames actually painted (a scene or pixel buffer rebuilt).
    PaintFrames = "paint_frames",
    /// Redraws served from the cached scene/pixels with nothing dirty.
    PaintCachedFrames = "paint_cached_frames",
    /// Software frames repainted inside a dirty region.
    RepaintPartial = "repaint_partial",
    /// Full repaints, any reason. Each is also counted under one
    /// `repaint_full_*` reason below.
    RepaintFull = "repaint_full",
    /// ...the first frame (no previous pixels to keep).
    RepaintFullFirstFrame = "repaint_full_first_frame",
    /// ...the surface was resized.
    RepaintFullResize = "repaint_full_resize",
    /// ...the scene was dirty but no dirty region came out: either no node
    /// was paint-dirty, or the paint-dirty nodes produced no rect (none had a
    /// layout box).
    RepaintFullNoDirtyNodes = "repaint_full_no_dirty_nodes",
    /// ...the dirty region covered half the surface or more.
    RepaintFullRegionTooLarge = "repaint_full_region_too_large",
    /// ...the dirty region had zero area.
    RepaintFullEmptyRegion = "repaint_full_empty_region",
    /// ...a drag ghost or the inspect highlight was on screen.
    RepaintFullOverlay = "repaint_full_overlay",
    /// ...the theme CSS changed.
    RepaintFullTheme = "repaint_full_theme",
    /// ...the previous frame was invalidated for a reason not listed above.
    RepaintFullInvalidated = "repaint_full_invalidated",
    /// ...the GPU backend, which always re-encodes the whole scene.
    RepaintFullGpu = "repaint_full_gpu",
    /// Area of the software dirty region repainted, in physical pixels
    /// (the whole surface for a full repaint).
    RepaintedPx = "repainted_px",
    /// Area of the surface, in physical pixels, summed per painted frame —
    /// `repainted_px / surface_px` is the repainted fraction.
    SurfacePx = "surface_px",
    /// Nodes `paint_node` visited.
    PaintNodesVisited = "paint_nodes_visited",
    /// Stacking-order sequences built (`stacking_paint_order`), by paint and
    /// hit testing alike.
    StackingOrderBuilds = "stacking_order_builds",
    /// Software painter: glyphs drawn from its rasterised-glyph cache.
    GlyphCacheHits = "glyph_cache_hits",
    /// Software painter: glyphs it had to rasterise (and then cached).
    GlyphCacheMisses = "glyph_cache_misses",
    /// Software painter: clip masks pushed.
    ClipMasks = "clip_masks",
    /// Software painter: mask pixels those clips were filled and intersected
    /// over — each clip's own bounds, not the surface.
    ClipMaskPx = "clip_mask_px",
    /// Software painter: opacity layers opened.
    PaintLayers = "paint_layers",
    /// Software painter: layer pixels composited back onto their parent —
    /// the part of each layer something was drawn into.
    LayerPx = "layer_px",
    /// Software painter: surface-sized masks and layer pixmaps newly
    /// allocated rather than reused from its pool. Zero in a steady state.
    PaintSurfaceAllocs = "paint_surface_allocs",
    /// Software painter: images premultiplied at draw time. A cached `<img>`
    /// or `background-image` is premultiplied once, on its first software
    /// paint; a live frame source every draw.
    ImagePremultiplies = "image_premultiplies",

    // ── Input ──────────────────────────────────────────────────────────
    /// Hit tests run (`RinchApp::hit_test`).
    HitTests = "hit_tests",
    /// Nodes the hit tests visited.
    HitTestNodesVisited = "hit_test_nodes_visited",
    /// Subtree extents the hit tester computed so it could skip subtrees
    /// nowhere near the pointer. Memoised until the tree or its layout
    /// changes, so a run of pointer moves over a still document computes each
    /// once.
    HitExtentsComputed = "hit_extents_computed",

    // ── Reactive (folded in by the shell from rinch-core's counters) ───
    /// Effect bodies run. A memo's invalidation marker runs through the same
    /// runner, so each memo invalidated counts one; its lazy recompute does
    /// not. Per thread: a DevTools window on the same thread
    /// adds its own effects, and the frame-time signals the shell writes for
    /// it, to every frame while it is visible.
    EffectRuns = "effect_runs",
    /// Signal change notifications.
    SignalNotifies = "signal_notifies",
    /// `ReRender` native events queued to the desktop event loop.
    RerenderEventsQueued = "rerender_events_queued",

    // ── Phase timings (wall clock, nanoseconds) ────────────────────────
    /// Style resolution and Taffy style sync.
    TimeStyleNs = "time_style_ns",
    /// Layout, excluding the style time spent inside it.
    TimeLayoutNs = "time_layout_ns",
    /// ...of which the IFC structural setup pass.
    TimeIfcSetupNs = "time_ifc_setup_ns",
    /// ...of which root Taffy computes (measure-time shaping included).
    TimeTaffyComputeNs = "time_taffy_compute_ns",
    /// ...of which `build_ifc_layouts` (paint-layout shaping).
    TimeBuildIfcNs = "time_build_ifc_ns",
    /// Building the scene or pixel buffer.
    TimePaintNs = "time_paint_ns",
    /// Presenting: the GPU raster + submit, or the softbuffer copy.
    TimePresentNs = "time_present_ns",
}

/// Why a frame was repainted in full. Stored by the shell when it throws its
/// previous frame away, and turned into the matching `repaint_full_*`
/// [`Counter`] when the frame is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullRepaintReason {
    FirstFrame,
    Resize,
    NoDirtyNodes,
    RegionTooLarge,
    EmptyRegion,
    Overlay,
    Theme,
    Invalidated,
    Gpu,
}

impl FullRepaintReason {
    /// The `repaint_full_*` counter for this reason.
    pub const fn counter(self) -> Counter {
        match self {
            Self::FirstFrame => Counter::RepaintFullFirstFrame,
            Self::Resize => Counter::RepaintFullResize,
            Self::NoDirtyNodes => Counter::RepaintFullNoDirtyNodes,
            Self::RegionTooLarge => Counter::RepaintFullRegionTooLarge,
            Self::EmptyRegion => Counter::RepaintFullEmptyRegion,
            Self::Overlay => Counter::RepaintFullOverlay,
            Self::Theme => Counter::RepaintFullTheme,
            Self::Invalidated => Counter::RepaintFullInvalidated,
            Self::Gpu => Counter::RepaintFullGpu,
        }
    }
}

/// Why the whole document was restyled; see the `full_restyle_*` counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullRestyleReason {
    Viewport,
    Theme,
    Stylesheet,
    Dpr,
    RootFontSize,
}

impl FullRestyleReason {
    /// The `full_restyle_*` counter for this reason.
    pub const fn counter(self) -> Counter {
        match self {
            Self::Viewport => Counter::FullRestyleViewport,
            Self::Theme => Counter::FullRestyleTheme,
            Self::Stylesheet => Counter::FullRestyleStylesheet,
            Self::Dpr => Counter::FullRestyleDpr,
            Self::RootFontSize => Counter::FullRestyleRootFontSize,
        }
    }
}

/// A snapshot of every [`Counter`]: one frame's worth, or a running total.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    values: [u64; Counter::COUNT],
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            values: [0; Counter::COUNT],
        }
    }
}

impl FrameStats {
    /// The value of one counter.
    #[inline]
    pub fn get(&self, counter: Counter) -> u64 {
        self.values[counter as usize]
    }

    /// Every counter with its value, in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = (Counter, u64)> + '_ {
        Counter::ALL.iter().map(|&c| (c, self.get(c)))
    }

    /// True when every counter is zero.
    pub fn is_empty(&self) -> bool {
        self.values.iter().all(|&v| v == 0)
    }

    /// Add `other` into `self`, counter by counter.
    pub fn accumulate(&mut self, other: &FrameStats) {
        for (a, b) in self.values.iter_mut().zip(other.values.iter()) {
            *a = a.wrapping_add(*b);
        }
    }

    /// `self - earlier`, counter by counter (saturating), for a delta
    /// between two snapshots of the same accumulator.
    pub fn since(&self, earlier: &FrameStats) -> FrameStats {
        let mut out = FrameStats::default();
        for (i, v) in out.values.iter_mut().enumerate() {
            *v = self.values[i].saturating_sub(earlier.values[i]);
        }
        out
    }

    /// Every counter as a JSON object keyed by [`Counter::name`].
    pub fn to_json(&self) -> serde_json::Value {
        let map: serde_json::Map<String, serde_json::Value> = self
            .iter()
            .map(|(c, v)| (c.name().to_string(), serde_json::Value::from(v)))
            .collect();
        serde_json::Value::Object(map)
    }

    /// One line: the phase times in milliseconds, then every non-zero
    /// counter. What `RINCH_PERF` prints per frame.
    pub fn summary_line(&self) -> String {
        use std::fmt::Write;
        let ms = |c: Counter| self.get(c) as f64 / 1e6;
        let mut s = format!(
            "style {:.2}ms layout {:.2}ms paint {:.2}ms present {:.2}ms |",
            ms(Counter::TimeStyleNs),
            ms(Counter::TimeLayoutNs),
            ms(Counter::TimePaintNs),
            ms(Counter::TimePresentNs),
        );
        for (c, v) in self.iter() {
            let headline = matches!(
                c,
                Counter::TimeStyleNs
                    | Counter::TimeLayoutNs
                    | Counter::TimePaintNs
                    | Counter::TimePresentNs
            );
            if v == 0 || headline {
                continue;
            }
            if c.name().starts_with("time_") {
                let _ = write!(s, " {}={:.2}ms", c.name(), v as f64 / 1e6);
            } else {
                let _ = write!(s, " {}={}", c.name(), v);
            }
        }
        s
    }
}

impl fmt::Debug for FrameStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut m = f.debug_map();
        for (c, v) in self.iter() {
            if v != 0 {
                m.entry(&c.name(), &v);
            }
        }
        m.finish()
    }
}

/// The counters a [`NodeTree`](crate::NodeTree) carries. See the
/// [module docs](self).
///
/// The current frame lives in `Cell`s so the read-only paths (paint and hit
/// testing take `&NodeTree`) can count too.
pub struct PerfCounters {
    current: [Cell<u64>; Counter::COUNT],
    last_frame: FrameStats,
    total: FrameStats,
    frames: u64,
}

impl Default for PerfCounters {
    fn default() -> Self {
        Self {
            current: std::array::from_fn(|_| Cell::new(0)),
            last_frame: FrameStats::default(),
            total: FrameStats::default(),
            frames: 0,
        }
    }
}

impl fmt::Debug for PerfCounters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PerfCounters")
            .field("frame", &self.frame())
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

impl PerfCounters {
    /// Add one to `counter` in the current frame.
    #[inline]
    pub fn bump(&self, counter: Counter) {
        self.add(counter, 1);
    }

    /// Add `n` to `counter` in the current frame.
    #[inline]
    pub fn add(&self, counter: Counter, n: u64) {
        let cell = &self.current[counter as usize];
        cell.set(cell.get().wrapping_add(n));
    }

    /// Count one full-document restyle under `reason`.
    #[inline]
    pub fn full_restyle(&self, reason: FullRestyleReason) {
        self.bump(Counter::FullRestyles);
        self.bump(reason.counter());
    }

    /// Count one full repaint under `reason`.
    #[inline]
    pub fn full_repaint(&self, reason: FullRepaintReason) {
        self.bump(Counter::RepaintFull);
        self.bump(reason.counter());
    }

    /// Add the time elapsed since `start` to a `Time*Ns` counter.
    #[inline]
    pub fn add_elapsed(&self, counter: Counter, start: web_time::Instant) {
        self.add(counter, start.elapsed().as_nanos() as u64);
    }

    /// The current (not yet ended) frame's value of one counter.
    #[inline]
    pub fn get(&self, counter: Counter) -> u64 {
        self.current[counter as usize].get()
    }

    /// A snapshot of the current (not yet ended) frame.
    pub fn frame(&self) -> FrameStats {
        let mut out = FrameStats::default();
        for (v, c) in out.values.iter_mut().zip(self.current.iter()) {
            *v = c.get();
        }
        out
    }

    /// The last frame [`end_frame`](Self::end_frame) completed.
    pub fn last_frame(&self) -> FrameStats {
        self.last_frame
    }

    /// Every ended frame plus the current one, since the last
    /// [`reset`](Self::reset).
    pub fn total(&self) -> FrameStats {
        let mut t = self.total;
        t.accumulate(&self.frame());
        t
    }

    /// How many frames have ended since the last [`reset`](Self::reset).
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// End the current frame: fold it into the total, keep it as
    /// [`last_frame`](Self::last_frame), zero the current frame, and return
    /// it. Prints [`FrameStats::summary_line`] to stderr when `RINCH_PERF`
    /// is set.
    pub fn end_frame(&mut self) -> FrameStats {
        let frame = self.frame();
        for c in &self.current {
            c.set(0);
        }
        self.total.accumulate(&frame);
        self.last_frame = frame;
        self.frames += 1;
        if perf_env_enabled() {
            eprintln!("[PERF] frame {}: {}", self.frames, frame.summary_line());
        }
        frame
    }

    /// Zero everything: the current frame, the last frame, the total and the
    /// frame count.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Whether `RINCH_PERF` is set. Read once per process and cached.
pub fn perf_env_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("RINCH_PERF").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for &c in Counter::ALL {
            assert!(seen.insert(c.name()), "duplicate counter name {}", c.name());
            assert_eq!(Counter::from_name(c.name()), Some(c));
            assert_eq!(Counter::ALL[c as usize], c, "discriminant is the index");
        }
    }

    #[test]
    fn end_frame_moves_the_frame_into_last_and_total() {
        let mut p = PerfCounters::default();
        p.add(Counter::ElementsCascaded, 3);
        p.bump(Counter::HitTests);
        assert_eq!(p.frame().get(Counter::ElementsCascaded), 3);
        let f = p.end_frame();
        assert_eq!(f.get(Counter::ElementsCascaded), 3);
        assert_eq!(p.frame().get(Counter::ElementsCascaded), 0);
        assert_eq!(p.last_frame().get(Counter::HitTests), 1);
        p.add(Counter::ElementsCascaded, 2);
        assert_eq!(p.total().get(Counter::ElementsCascaded), 5);
        assert_eq!(p.frames(), 1);
        p.reset();
        assert!(p.total().is_empty());
        assert_eq!(p.frames(), 0);
    }

    #[test]
    fn json_has_every_counter() {
        let mut p = PerfCounters::default();
        p.bump(Counter::RepaintFullGpu);
        let j = p.end_frame().to_json();
        let obj = j.as_object().unwrap();
        assert_eq!(obj.len(), Counter::COUNT);
        assert_eq!(obj["repaint_full_gpu"], 1);
    }
}
