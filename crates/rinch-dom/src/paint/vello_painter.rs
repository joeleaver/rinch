//! [`Painter`] implementation backed by `vello::Scene` for GPU rendering.

use peniko::kurbo::{Affine, Point, Vec2};
use peniko::{Blob, Brush, Fill, FontData, ImageAlphaType, ImageData, ImageFormat};
use vello::Scene;

use super::painter::{
    BlendMode, GlyphSynthesis, ImageSampling, PaintGlyph, PaintImage, PaintShape, Painter,
};
use super::slicing::{Open, Slicing, SurfaceSlice};
use peniko::kurbo::Rect;

/// GPU rendering backend that wraps a [`vello::Scene`].
///
/// Drawing commands are accumulated in the scene and later submitted
/// to the Vello rendering pipeline for GPU execution.
pub struct VelloPainter {
    scene: Scene,
    /// Open on a frame the runtime paints for the split-layer path (#1865).
    slicing: Option<Slicing>,
    /// See [`Self::node_starts`].
    node_starts: std::collections::HashMap<crate::node::RawNodeId, usize>,
    /// This scene holds a layer that blends with its backdrop (grayscale's
    /// saturation): appended over a split surface, it taints it (D5a).
    reads_backdrop: bool,
    /// [`Painter::set_coarse_curves`]: fills leave their curves to Vello.
    coarse_curves: bool,
    /// A surface was split this frame, so draws are routed by their bounds.
    /// A plain field, so a draw on a frame with none tests one bool and is
    /// otherwise the draw itself (#1899 review: the routing generic cost
    /// 8.1k instructions a text page with no surface).
    routing: bool,
    /// The clips and layers open, each with whether its pop reaches the scene
    /// and the region drawing is confined to with it open: see
    /// [`Painter::push_clip`] here.
    open: Vec<OpenLayer>,
    /// The render target tiles are counted inside: see [`Self::set_tile_target`].
    tile_target: Rect,
    /// How many of `open` are layers, not clips: subpixel (LCD) text is drawn
    /// only while none is (#2286 stage 3).
    layers_open: u32,
    /// See [`Self::set_subpixel_text`].
    subpixel_text: bool,
    /// See [`Self::clip_tiles`].
    clip_tiles: u64,
    /// The glyph masks text is drawn from (#2286): the thread's atlas, shared
    /// with every painter on it, unless [`Self::set_glyph_atlas`] gave this
    /// painter its own.
    #[cfg(feature = "glyph-atlas")]
    atlas: super::glyph_atlas::SharedAtlas,
    /// This painter's id in `atlas` (its frames protect what they drew).
    #[cfg(feature = "glyph-atlas")]
    atlas_painter: u32,
    /// See [`Self::take_glyph_stats`].
    glyph_stats: GlyphStats,
    /// See [`Self::set_glyph_atlas_enabled`].
    #[cfg(feature = "glyph-atlas")]
    atlas_enabled: bool,
    /// Runs changing size frame after frame, drawn as outlines meanwhile.
    #[cfg(feature = "glyph-atlas")]
    churn: super::glyph_atlas::SizeChurn,
    /// Scratch reused across glyph runs.
    #[cfg(feature = "glyph-atlas")]
    glyph_scratch: GlyphScratch,
}

/// What the Vello painter's glyph draws did, since the last
/// [`VelloPainter::take_glyph_stats`] (the `glyph_atlas_*` and
/// `glyph_outline_fallbacks` counters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphStats {
    /// The atlas's hits, misses, uploads and evictions.
    #[cfg(feature = "glyph-atlas")]
    pub atlas: super::glyph_atlas::AtlasStats,
    /// Glyphs drawn as outlines: every glyph without the `glyph-atlas`
    /// feature; with it, the ones the atlas does not draw.
    pub outline_fallbacks: u64,
}

impl GlyphStats {
    /// Add these counts to the current frame of `perf`.
    pub fn add_to(&self, perf: &crate::perf::PerfCounters) {
        #[cfg(feature = "glyph-atlas")]
        self.atlas.add_to(perf);
        perf.add(
            crate::perf::Counter::GlyphOutlineFallbacks,
            self.outline_fallbacks,
        );
    }
}

/// One glyph the atlas draws: its mask's image, at a whole device pixel.
#[cfg(feature = "glyph-atlas")]
struct MaskDraw {
    image: ImageData,
    x: f64,
    y: f64,
    /// A subpixel mask (the fork's mask mode 2).
    lcd: bool,
}

#[cfg(feature = "glyph-atlas")]
#[derive(Default)]
struct GlyphScratch {
    keys: Vec<(u32, u8)>,
    places: Vec<super::glyph_atlas::Placement>,
    draws: Vec<MaskDraw>,
    outlines: Vec<PaintGlyph>,
}

/// One clip or layer [`VelloPainter`] has open.
#[derive(Clone, Copy)]
struct OpenLayer {
    /// Its pop has something to pop; `false` for a clip elided as cutting nothing.
    pushed: bool,
    /// Device bounds drawing is confined to while this is open.
    region: Rect,
    /// A layer (opacity, blend, isolation), not a clip: subpixel text is
    /// grey inside it (#2286 stage 3).
    layer: bool,
    /// `region` is exactly what the open clips leave, not just its bounds:
    /// every one of them is an axis-aligned rect. Only then is a clip judged
    /// against it: a rounded clip leaves less than its bounds.
    exact: bool,
}

/// No confinement: before any clip is open.
const EVERYWHERE: Rect = Rect::new(
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
    f64::INFINITY,
    f64::INFINITY,
);

/// Vello's tile edge (`TILE_WIDTH`, `TILE_HEIGHT` in its shaders), in device
/// pixels.
const VELLO_TILE_PX: f64 = 16.0;

/// The tiles a clip or layer with device bounds `bounds` costs in a render to
/// `target`: the 16px tiles its bounds cover once clipped to the target, as
/// `tile_alloc.wgsl` rounds them (down at the near edges, up at the far ones).
/// An empty box, in or clipped by the target, costs none.
///
/// An estimate of what a clip or layer costs, not Vello's `bump.tile`: see
/// [`VelloPainter::clip_tiles`].
pub(crate) fn tile_cost(bounds: Rect, target: Rect) -> u64 {
    let b = bounds.intersect(target);
    if b.width() <= 0.0 || b.height() <= 0.0 {
        return 0;
    }
    let span = |near: f64, far: f64| {
        ((far / VELLO_TILE_PX).ceil() - (near / VELLO_TILE_PX).floor()) as u64
    };
    span(b.x0, b.x1).saturating_mul(span(b.y0, b.y1))
}

/// Whether clipping to `shape` under `transform` can remove nothing from
/// `region` (device bounds drawing is already confined to): an axis-aligned
/// rect, or rounded rect, that holds all of it. Conservative: `false` for any
/// other shape or transform.
fn clip_holds(transform: Affine, shape: &PaintShape, region: Rect) -> bool {
    let [sx, b, c, sy, _, _] = transform.as_coeffs();
    if b != 0.0 || c != 0.0 || !region.is_finite() {
        return false;
    }
    let holds = |outer: Rect, inner: Rect| {
        outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
    };
    match shape {
        PaintShape::Rect(r) => holds(transform.transform_rect_bbox(*r), region),
        PaintShape::RoundedRect(rr) => {
            // A point of the rect lies outside the rounded shape only inside a
            // corner square of the radius' size: a region clear of the corners
            // in either axis is inside.
            let outer = transform.transform_rect_bbox(rr.rect());
            let radii = rr.radii();
            let r = radii
                .top_left
                .max(radii.top_right)
                .max(radii.bottom_left)
                .max(radii.bottom_right)
                * sx.abs().max(sy.abs());
            holds(outer, region)
                && (holds(outer.inset((-r, 0.0)), region) || holds(outer.inset((0.0, -r)), region))
        }
        _ => false,
    }
}

/// Paint `$body` (with `$s` the scene) where it belongs: on a frame with no
/// split surface, the body itself on the window's scene, its bounds never
/// worked out; else routed by its device bounds (`$bounds()`). A macro, not a
/// generic method: the no-surface path then builds no closure at all.
macro_rules! route {
    ($self:ident, $bounds:expr, |$s:ident| $body:expr) => {
        if !$self.routing {
            let $s = &mut $self.scene;
            $body;
        } else {
            $self.draw_routed(&$bounds, &|$s: &mut Scene| {
                $body;
            })
        }
    };
}

impl VelloPainter {
    /// Create a new `VelloPainter` with an empty scene.
    pub fn new() -> Self {
        #[cfg(feature = "glyph-atlas")]
        let atlas = super::glyph_atlas::thread_atlas();
        #[cfg(feature = "glyph-atlas")]
        let atlas_painter = atlas.borrow_mut().register_painter();
        Self {
            scene: Scene::new(),
            slicing: None,
            node_starts: Default::default(),
            reads_backdrop: false,
            coarse_curves: false,
            routing: false,
            open: Vec::new(),
            tile_target: EVERYWHERE,
            layers_open: 0,
            subpixel_text: true,
            clip_tiles: 0,
            #[cfg(feature = "glyph-atlas")]
            atlas_painter,
            #[cfg(feature = "glyph-atlas")]
            atlas,
            glyph_stats: GlyphStats::default(),
            #[cfg(feature = "glyph-atlas")]
            atlas_enabled: true,
            #[cfg(feature = "glyph-atlas")]
            churn: Default::default(),
            #[cfg(feature = "glyph-atlas")]
            glyph_scratch: GlyphScratch::default(),
        }
    }

    /// What this painter's glyph draws did since the last call, zeroing it:
    /// the atlas's hits, misses, uploads and evictions, and the glyphs drawn
    /// as outlines. The shell adds the window painter's to each GPU frame.
    pub fn take_glyph_stats(&mut self) -> GlyphStats {
        std::mem::take(&mut self.glyph_stats)
    }

    /// Draw text from `atlas` instead of the thread's.
    #[cfg(feature = "glyph-atlas")]
    pub fn set_glyph_atlas(&mut self, atlas: super::glyph_atlas::SharedAtlas) {
        self.atlas.borrow_mut().forget_painter(self.atlas_painter);
        self.atlas_painter = atlas.borrow_mut().register_painter();
        self.atlas = atlas;
    }

    /// The atlas this painter draws text from.
    #[cfg(feature = "glyph-atlas")]
    pub fn glyph_atlas(&self) -> &super::glyph_atlas::SharedAtlas {
        &self.atlas
    }

    /// Whether this painter may draw subpixel (LCD) text where the thread's
    /// policy asks for it (the default), or, with `false`, draws grey text
    /// whatever the policy (#2286 stage 3). For a scene drawn somewhere its
    /// pixel grid is not known, such as a snapshot composited under a
    /// transform.
    pub fn set_subpixel_text(&mut self, on: bool) {
        self.subpixel_text = on;
    }

    /// Whether a glyph run drawn now may be subpixel text: the painter allows
    /// it and no layer is open. Inside an opacity, blend or isolated layer
    /// the destination is the layer's own pixels, which are not opaque and
    /// are composited later, so text there is grey, as Skia and Chrome draw
    /// it. Clips do not stop it: the fork's per-channel composite reads the
    /// pixel under the clip (`fine.wgsl`'s `lcd_composite`).
    pub fn subpixel_text_allowed(&self) -> bool {
        self.subpixel_text && self.layers_open == 0
    }

    /// Draw text from the glyph-mask atlas (the default) or, with `false`,
    /// every glyph as Vello's outlines, as before #2286: for text whose size
    /// animates every frame (each size is a new set of masks), or to compare.
    #[cfg(feature = "glyph-atlas")]
    pub fn set_glyph_atlas_enabled(&mut self, on: bool) {
        self.atlas_enabled = on;
    }

    /// The render target this frame's scene will be drawn to, in device
    /// pixels: [`Self::clip_tiles`] counts inside it. Call after
    /// [`Painter::reset`], which forgets it, and after
    /// [`Self::begin_slicing`], before painting. With none, nothing is cut:
    /// a snapshot, which is drawn into someone else's frame later.
    pub fn set_tile_target(&mut self, target: Rect) {
        self.tile_target = target;
        if let Some(slicing) = &mut self.slicing {
            slicing.tile_target = target;
        }
    }

    /// An estimate of the tiles the clips and layers encoded for this frame
    /// ask Vello for: [`tile_cost`] of each one's device bounding box, summed.
    /// Vello allocates a clip's or layer's tiles over its whole bounds,
    /// whatever it holds, its tile and ptcl buffers are fixed, and a frame
    /// past them is dropped (`perf::Counter::VelloClipTiles`).
    ///
    /// Counted: what reaches the window's scene, the inverse clip of the split
    /// path each time it opens, and what a slice keeps: its above scenes that
    /// have draws, replayed clips included, and its mask scenes. Not counted: a
    /// clip [`Painter::push_clip`] elides, and the above scene of a slice
    /// nothing was drawn over, neither of which is rendered. A surface's slices
    /// are separate Vello renders, each with those buffers to itself, added
    /// here as a frame's total cost.
    ///
    /// **Not Vello's `bump.tile`.** Fills and glyph runs are left out, and
    /// their tiles are in `bump.tile`. A box is cut by the target only, where
    /// Vello also cuts it by the bounds of the clips around it, so a clip
    /// nested in a smaller one is overstated; a curved shape's bounds are its
    /// control points'. An appended scene adds what its own painter counted,
    /// not cut by this target.
    ///
    /// Read after [`Self::take_slices`], which adds the split path's part.
    /// [`Painter::reset`] zeroes it.
    pub fn clip_tiles(&self) -> u64 {
        self.clip_tiles
    }

    /// The region drawing is confined to now, and whether it is exact.
    fn region(&self) -> (Rect, bool) {
        self.open
            .last()
            .map_or((EVERYWHERE, true), |o| (o.region, o.exact))
    }

    /// Record a clip or layer pushed into the window's scene, confining
    /// drawing to `shape` under `transform` as well.
    fn opened(&mut self, transform: Affine, shape: &PaintShape, layer: bool) {
        let (region, exact) = self.region();
        let bounds = shape_bounds(transform, shape);
        let region = region.intersect(bounds);
        self.clip_tiles = self
            .clip_tiles
            .saturating_add(tile_cost(bounds, self.tile_target));
        let [_, b, c, _, _, _] = transform.as_coeffs();
        let exact = exact && matches!(shape, PaintShape::Rect(_)) && b == 0.0 && c == 0.0;
        self.open.push(OpenLayer {
            pushed: true,
            region,
            layer,
            exact,
        });
        self.layers_open += u32::from(layer);
    }

    /// Open this frame for the split-layer path (#1865): GPU surfaces offered
    /// through [`Painter::split_surface`] may leave the scene. `window` is the
    /// render target in device pixels. Call after [`Painter::reset`], before
    /// painting; collect with [`Self::take_slices`].
    pub fn begin_slicing(&mut self, window: Rect) {
        self.slicing = Some(Slicing::new(window));
        self.routing = false;
    }

    /// Refuse to split `ids` this frame ([`super::painter::Refusal::MemoryCap`]):
    /// the runtime's memory cap (#1865 D6). Call after [`Self::begin_slicing`];
    /// a frame not opened for slicing splits nothing anyway.
    pub fn refuse_split(&mut self, ids: &[usize]) {
        if let Some(slicing) = &mut self.slicing {
            slicing.refused = ids.to_vec();
        }
    }

    /// The surfaces this frame took out of the scene, in paint order, and the
    /// end of slicing for the frame. Empty when the frame was not opened.
    pub fn take_slices(&mut self) -> Vec<SurfaceSlice> {
        self.routing = false;
        match self.slicing.take() {
            Some(mut slicing) => {
                self.clip_tiles = self.clip_tiles.saturating_add(slicing.kept_tiles());
                self.node_starts = std::mem::take(&mut slicing.starts);
                slicing.finish(&mut self.scene)
            }
            None => {
                self.node_starts.clear();
                Vec::new()
            }
        }
    }

    /// Where the last sliced frame started each watched node
    /// (`paint::set_paint_order_watch`), as the number of surfaces split
    /// before its first start (#1865 S4): a node started after the `i`th
    /// split (0-based) has a count past `i`, every time it starts, since the
    /// count only grows through a paint. Filled by [`Self::take_slices`].
    pub fn node_starts(&self) -> &std::collections::HashMap<crate::node::RawNodeId, usize> {
        &self.node_starts
    }

    /// Append `other`'s scene under `transform`, routed like any draw on a
    /// sliced frame (bounds unknown: every region it may meet). The runtime's
    /// drag ghost (#1865). A ghost that blends with its backdrop (grayscale)
    /// over a split surface taints it, as a layer painted in place would.
    pub fn append_scene(&mut self, other: &VelloPainter, transform: Affine) {
        if other.reads_backdrop
            && let Some(slicing) = &mut self.slicing
        {
            slicing.taint_all();
        }
        self.clip_tiles = self.clip_tiles.saturating_add(other.clip_tiles);
        route!(self, || None, |s| s.append(&other.scene, Some(transform)));
    }

    /// A draw on a frame with a split surface, routed by its `bounds`
    /// ([`route!`]).
    #[inline(never)]
    fn draw_routed(&mut self, bounds: &dyn Fn() -> Option<Rect>, op: &dyn Fn(&mut Scene)) {
        let Self { slicing, scene, .. } = self;
        match slicing {
            Some(slicing) => slicing.draw(scene, bounds(), op),
            None => op(scene),
        }
    }

    /// Record and push the entry `make` builds on a sliced frame (the callers
    /// push straight into the scene otherwise, with no copy of the shape).
    /// Out of line, so a frame not opened for slicing pays one test.
    #[inline(never)]
    fn push_sliced(&mut self, make: impl FnOnce() -> Open) {
        if let Some(slicing) = &mut self.slicing {
            slicing.push(&mut self.scene, make());
        }
    }

    /// Pop on a sliced frame.
    #[inline(never)]
    fn pop_sliced(&mut self) {
        if let Some(slicing) = &mut self.slicing {
            slicing.pop(&mut self.scene);
        }
    }

    /// Borrow the underlying scene (read-only).
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Borrow the underlying scene (mutable).
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }
}

impl VelloPainter {
    /// [`Painter::draw_glyphs`] on a frame with a split surface: routed by
    /// the run's bounds, its origins grown by two ems for the glyphs' extent
    /// under any glyph transform (synthetic skew), then mapped.
    #[allow(clippy::too_many_arguments)]
    #[cold]
    #[inline(never)]
    fn draw_glyphs_routed(
        &mut self,
        font: &FontData,
        font_size: f32,
        transform: Affine,
        synthesis: GlyphSynthesis,
        brush: &Brush,
        hint: bool,
        normalized_coords: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        let bounds = || {
            let mut it = glyphs.iter();
            let first = it.next()?;
            let mut r = Rect::new(
                first.x as f64,
                first.y as f64,
                first.x as f64,
                first.y as f64,
            );
            for g in it {
                r = r.union_pt((g.x as f64, g.y as f64));
            }
            let em = 2.0 * font_size as f64;
            Some(transform.transform_rect_bbox(r.inflate(em, em)))
        };
        self.draw_routed(&bounds, &|scene: &mut Scene| {
            encode_glyphs(
                scene,
                font,
                font_size,
                transform,
                synthesis,
                brush,
                hint,
                normalized_coords,
                glyphs,
            )
        });
    }
}

#[cfg(feature = "glyph-atlas")]
impl VelloPainter {
    /// Draw a glyph run from the atlas (#2286): each glyph the mask for its
    /// device size and sub-pixel bucket, an image at a whole device pixel
    /// painted in the brush colour through the fork's tinted alpha mask. A
    /// colour glyph in the run is drawn as Vello draws it (outlines, COLR,
    /// bitmap). `false`: the atlas does not draw this run at all (see
    /// [`super::glyph_atlas`]), and the caller draws outlines.
    #[allow(clippy::too_many_arguments)]
    fn draw_atlas_glyphs(
        &mut self,
        font: &FontData,
        font_size: f32,
        transform: Affine,
        synthesis: GlyphSynthesis,
        brush: &Brush,
        hint: bool,
        normalized_coords: &[i16],
        glyphs: &[PaintGlyph],
    ) -> bool {
        use super::glyph_atlas::{
            ATLAS_MAX_FONT_PX, Looked, MaskKind, RunSpec, atlas_scale, place,
        };
        let Brush::Solid(tint) = brush else {
            return false;
        };
        // A faux italic draws as outlines, Vello's glyph transform; a faux
        // bold is rasterised emboldened (#2296).
        if synthesis.skew != 0.0 {
            return false;
        }
        let Some(scale) = atlas_scale(transform) else {
            return false;
        };
        let size = (font_size as f64 * scale) as f32;
        if !(size > 0.0 && size <= ATLAS_MAX_FONT_PX) {
            return false;
        }
        // The thread's policy (#2286 stage 1), as `encode_glyphs` reads it:
        // the app's escape hatch, hinting, and light text's stem darkening in
        // device pixels, which is the mask's own scale here.
        let policy = super::text_rasterization::thread_policy();
        if policy.gpu_glyph_outlines {
            return false;
        }
        let spec = RunSpec {
            font,
            size,
            hint: hint && policy.hint(),
            embolden: policy.run_darkening_px(brush, font_size, transform),
            // The mask is rasterised at the device size, so the faux bold
            // takes the device amount too.
            synthetic_bold: (synthesis.embolden as f64 * scale) as f32,
            skew: 0.0,
            coords: normalized_coords,
            // Subpixel masks where the policy asks for them and the painter
            // can draw them here (#2286 stage 3); the fork composites them
            // per channel only over an opaque pixel, and grey elsewhere.
            lcd: if self.subpixel_text_allowed() {
                policy.lcd_mode()
            } else {
                None
            },
        };
        // A colour face is Vello's to draw (COLR, bitmaps): not rasterised
        // here just to be thrown away. A run whose size is animating draws
        // as outlines until it settles.
        if self.atlas.borrow_mut().face_has_color(font) || self.churn.animating(&spec) {
            return false;
        }
        let mut scratch = std::mem::take(&mut self.glyph_scratch);
        scratch.keys.clear();
        scratch.places.clear();
        scratch.draws.clear();
        scratch.outlines.clear();
        for g in glyphs {
            let p = transform * peniko::kurbo::Point::new(g.x as f64, g.y as f64);
            let pl = place(p.x, p.y);
            scratch.keys.push((g.id, pl.bucket));
            scratch.places.push(pl);
        }
        let tint = *tint;
        let routing = self.routing;
        {
            let GlyphScratch {
                keys,
                places,
                draws,
                outlines,
            } = &mut scratch;
            let scene = &mut self.scene;
            let read = self.atlas.borrow_mut().lookup(
                &spec,
                keys,
                true,
                &mut self.glyph_stats.atlas,
                |i, looked| match looked {
                    Looked::Mask(m)
                        if matches!(m.kind, MaskKind::Coverage | MaskKind::Subpixel) =>
                    {
                        if let Some(image) = m.gpu_image() {
                            let (x, y) =
                                ((places[i].x + m.left) as f64, (places[i].y - m.top) as f64);
                            let lcd = m.kind == MaskKind::Subpixel;
                            if routing {
                                // Routed below, once the run's bounds are known.
                                draws.push(MaskDraw {
                                    image: image.clone(),
                                    x,
                                    y,
                                    lcd,
                                });
                            } else {
                                // The common frame: straight into the scene, the
                                // image borrowed (the encoding takes its own
                                // reference for the resolve).
                                encode_mask(scene, image, tint, x, y, lcd);
                            }
                        }
                    }
                    // A colour mask (a face without COLR/CBDT/sbix tables that
                    // still gave one) or a glyph swash could not render.
                    Looked::Mask(_) | Looked::Failed => outlines.push(glyphs[i]),
                    Looked::Empty => {}
                },
            );
            if !read {
                // swash cannot read the font. Vello's outlines, as before
                // the atlas, if skrifa (Vello's reader) can: its `draw_glyphs`
                // panics on a font it cannot read, so not otherwise.
                self.glyph_scratch = scratch;
                let data: &[u8] = font.data.as_ref();
                return skrifa::FontRef::from_index(data, font.index).is_err();
            }
        }
        self.glyph_stats.outline_fallbacks += scratch.outlines.len() as u64;
        let encode = |scene: &mut Scene| {
            for d in &scratch.draws {
                encode_mask(scene, &d.image, tint, d.x, d.y, d.lcd);
            }
            if !scratch.outlines.is_empty() {
                encode_glyphs(
                    scene,
                    font,
                    font_size,
                    transform,
                    synthesis,
                    brush,
                    hint,
                    normalized_coords,
                    &scratch.outlines,
                );
            }
        };
        if routing {
            let bounds = || {
                let mut r: Option<Rect> = None;
                for d in &scratch.draws {
                    let b = Rect::new(
                        d.x,
                        d.y,
                        d.x + d.image.width as f64,
                        d.y + d.image.height as f64,
                    );
                    r = Some(r.map_or(b, |r| r.union(b)));
                }
                if !scratch.outlines.is_empty() {
                    let em = 2.0 * font_size as f64;
                    for g in &scratch.outlines {
                        let o = transform.transform_rect_bbox(
                            Rect::new(g.x as f64, g.y as f64, g.x as f64, g.y as f64)
                                .inflate(em, em),
                        );
                        r = Some(r.map_or(o, |r| r.union(o)));
                    }
                }
                r
            };
            self.draw_routed(&bounds, &encode);
        } else {
            encode(&mut self.scene);
        }
        self.glyph_scratch = scratch;
        true
    }
}

/// Encode one atlas mask: its image, at the whole device pixel `(x, y)`,
/// painted in `tint` (the fork's tinted alpha mask), sampled nearest, so the
/// texels are copied as they are. A subpixel mask (`lcd`) is the fork's mask
/// mode 2, composited per channel over an opaque pixel (#2286 stage 3).
#[cfg(feature = "glyph-atlas")]
#[inline]
fn encode_mask(
    scene: &mut Scene,
    image: &ImageData,
    tint: peniko::Color,
    x: f64,
    y: f64,
    lcd: bool,
) {
    let brush = peniko::ImageBrushRef::from(image).with_quality(peniko::ImageQuality::Low);
    let at = Affine::translate((x, y));
    if lcd {
        scene.draw_image_lcd_mask(brush, tint, at);
    } else {
        scene.draw_image_mask(brush, tint, at);
    }
}

/// Encode one glyph run into `scene`, under this thread's text-rendering
/// policy (#2286): hinted only where the policy hints, and light text
/// thickened by its stem darkening through Vello's `font_embolden`
/// (`kurbo::expand_path` on each outline, cached per amount).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn encode_glyphs(
    scene: &mut Scene,
    font: &FontData,
    font_size: f32,
    transform: Affine,
    synthesis: GlyphSynthesis,
    brush: &Brush,
    hint: bool,
    normalized_coords: &[i16],
    glyphs: &[PaintGlyph],
) {
    let policy = super::text_rasterization::thread_policy();
    let hint = hint && policy.hint();
    // The darkening is in device pixels. Vello expands the outline in the
    // space it draws the glyph in: with hinting and a uniform-scale transform
    // it folds the scale into the font size and draws at device size
    // (`vello_encoding::resolve`, the `GlyphRun` patch, compares the f32
    // matrix this way), otherwise at `font_size`, scaled by `transform`
    // afterwards.
    //
    // A faux bold (#2296) is in the run's own units, so it is scaled up where
    // the scale is folded in, and adds to the darkening.
    let device = policy.run_darkening_px(brush, font_size, transform) as f64;
    let faux = synthesis.embolden as f64;
    let darkening = if device == 0.0 && faux == 0.0 {
        0.0
    } else {
        let [a, b, c, d, _, _] = transform.as_coeffs();
        let folds = hint && a as f32 == d as f32 && b as f32 == 0.0 && c as f32 == 0.0;
        let scale = super::text_rasterization::device_scale(transform);
        if folds || scale == 0.0 {
            device + faux * scale
        } else {
            device / scale + faux
        }
    };
    scene
        .draw_glyphs(font)
        .font_size(font_size)
        .transform(transform)
        .glyph_transform(synthesis.glyph_transform())
        .brush(brush)
        .hint(hint)
        .font_embolden(vello::FontEmbolden::new(peniko::kurbo::Diagonal2::new(
            darkening, darkening,
        )))
        .normalized_coords(normalized_coords)
        .draw(
            Fill::NonZero,
            glyphs.iter().map(|g| vello::Glyph {
                id: g.id,
                x: g.x,
                y: g.y,
            }),
        );
}

#[cfg(feature = "glyph-atlas")]
impl Drop for VelloPainter {
    fn drop(&mut self) {
        if let Ok(mut a) = self.atlas.try_borrow_mut() {
            a.forget_painter(self.atlas_painter);
        }
    }
}

impl Default for VelloPainter {
    fn default() -> Self {
        Self::new()
    }
}

/// The brush transform that makes a sampler that looks at each pixel's
/// top-left CORNER see its CENTRE, for a brush whose brush-to-device map
/// would otherwise be `transform * brush` (#1027, #1798).
///
/// Vello's fine shader (`fine.wgsl`) evaluates a gradient at the pixel's
/// top-left corner (`my_xy`), where CSS and tiny-skia use its centre. Moving
/// the brush half a DEVICE pixel up and left, `T(-0.5) * transform * brush`,
/// puts the centre where the shader looks, under any scale, rotation or
/// skew; the shape is unaffected, being drawn with `transform` alone. A
/// scene appended under a translation keeps it (translations commute); one
/// appended under a scale would not, and none is.
///
/// **Images do not take it any more.** Vello 0.7 sampled images at the corner
/// too, and this shift was how rinch sampled them at the centre; Vello 0.9
/// (linebender/vello#1606) moved its image arm (`CMD_IMAGE`) to the pixel
/// centre, so since the upgrade to Vello 0.11 (#1955) an image brush is drawn
/// with its own transform, which gives the same image-space sample points as
/// before. The split-layer compositor's own shader (`split_compositor.rs`,
/// `fs_surface`) still looks at the corner, so `slicing` keeps the shift for it.
///
/// A degenerate `transform` (which draws nothing) keeps `brush`.
pub(crate) fn centre_sampling(transform: Affine, brush: Affine) -> Affine {
    let det = transform.determinant();
    if !det.is_finite() || det == 0.0 {
        return brush;
    }
    transform.inverse() * Affine::translate((-0.5, -0.5)) * transform * brush
}

thread_local! {
    /// Each live frame buffer's blob, by buffer address, held weakly: the
    /// blob holds the buffer, so the address cannot be reused while the blob
    /// lives, and a dead entry is skipped and pruned.
    static FRAME_BLOBS: std::cell::RefCell<std::collections::HashMap<usize, peniko::WeakBlob<u8>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The blob for a live frame buffer: the same one while any holder (a scene,
/// Vello's atlas) keeps it, so a frame repainted unchanged keeps one Vello
/// image and is not uploaded again; a new frame (a new buffer) is a new
/// image. Shares the buffer, copies nothing (#1955).
pub(crate) fn frame_blob(frame: &std::sync::Arc<Vec<u8>>) -> Blob<u8> {
    let key = std::sync::Arc::as_ptr(frame) as usize;
    FRAME_BLOBS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(blob) = cache.get(&key).and_then(|weak| weak.upgrade())
            && std::ptr::eq(blob.data().as_ptr(), frame.as_ptr())
            && blob.len() == frame.len()
        {
            return blob;
        }
        if cache.len() >= 64 {
            cache.retain(|_, weak| weak.upgrade().is_some());
        }
        let blob = Blob::new(frame.clone());
        cache.insert(key, blob.downgrade());
        blob
    })
}

/// The most, in device px, a curve `VelloPainter` hands Vello strays from the
/// true curve (#2321). Vello flattens every curve itself at a fixed 0.25
/// device px (`flatten.wgsl`), and each chord lies inside a convex arc, so a
/// rounded corner covered up to 64 levels less than its area (52 measured at
/// 1x, a 5px corner). A pixel's coverage error is at most the distance the
/// polygon strays, so at 0.03 a corner pixel is within about 8 levels of the
/// area (measured 3–8). Lines are not flattened again, and Vello's area
/// coverage of a polygon is exact, so this is the whole error. Glyphs never
/// come through here (`draw_glyphs` hands Vello outlines, or atlas masks, of
/// its own).
///
/// Why 0.03: each line costs about 130 instructions to work out and encode,
/// and the corner's lines go as `1 / sqrt(tolerance)`. At 0.02 the 140
/// rounded boxes of `vello_paint::split_surface_140_boxes` cost 6.6% more
/// than Vello's own curves; at 0.03 they cost the same (−0.1%), at 0.04 5%
/// less but up to 12 levels off. Measured by sweeping it (review of #2380).
pub(crate) const CURVE_TOLERANCE: f64 = 0.03;

/// `shape` with its curves flattened to lines at most [`CURVE_TOLERANCE`]
/// device px from them under `transform`, or `None` when it has none to
/// flatten (a rect, a line, a square rounded rect, a path of lines, or a
/// transform that collapses it). The lines stay in `shape`'s own space, so
/// the transform, a brush transform and a stroke's width mean what they did.
///
/// It is a [`Shape`](peniko::kurbo::Shape) whose path elements are worked
/// out as Vello's encoder reads them ([`FlatIter`]), not a path built first:
/// no polygon is written to memory and read back.
pub(crate) fn fine(transform: Affine, shape: &PaintShape) -> Option<Flat<'_>> {
    use peniko::kurbo::PathEl;
    use std::f64::consts::{FRAC_PI_2, TAU};
    let curved = match shape {
        PaintShape::Rect(_) | PaintShape::Line(_) => false,
        PaintShape::RoundedRect(r) => {
            let r = r.radii();
            r.top_left > 0.0 || r.top_right > 0.0 || r.bottom_right > 0.0 || r.bottom_left > 0.0
        }
        PaintShape::Circle(c) => c.radius > 0.0,
        PaintShape::BezPath(p) => p
            .elements()
            .iter()
            .any(|e| matches!(e, PathEl::QuadTo(..) | PathEl::CurveTo(..))),
    };
    if !curved {
        return None;
    }
    // The transform's largest stretch: a local error of `t` is at most
    // `t * stretch` device px.
    let [a, b, c, d, _, _] = transform.as_coeffs();
    let (sum, det) = (a * a + b * b + c * c + d * d, a * d - b * c);
    let stretch = ((sum + (sum * sum - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt();
    if !(stretch.is_finite() && stretch > 0.0) {
        return None;
    }
    let tolerance = CURVE_TOLERANCE / stretch;
    let bounds = shape.control_bounds().inflate(tolerance, tolerance);
    let mut ops = Ops::default();
    match shape {
        PaintShape::BezPath(p) => {
            return Some(Flat {
                ops,
                path: Some(p.elements()),
                tolerance,
                bounds,
            });
        }
        PaintShape::RoundedRect(rr) => {
            // As kurbo's `RoundedRect` path runs, so a dash pattern stroked
            // along it has the same phase on both painters: from the
            // top-left corner's start on the left edge `(x0, y0 + r)`, round
            // that corner, then clockwise on screen.
            let Rect { x0, y0, x1, y1 } = rr.rect();
            let r = rr.radii();
            let (up, right, down, left) = (
                Vec2::new(0.0, -1.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(-1.0, 0.0),
            );
            // Each corner: its radius, its centre, and the directions it runs
            // from and to.
            let corners = [
                (
                    r.top_left,
                    Point::new(x0 + r.top_left, y0 + r.top_left),
                    [left, up],
                ),
                (
                    r.top_right,
                    Point::new(x1 - r.top_right, y0 + r.top_right),
                    [up, right],
                ),
                (
                    r.bottom_right,
                    Point::new(x1 - r.bottom_right, y1 - r.bottom_right),
                    [right, down],
                ),
                (
                    r.bottom_left,
                    Point::new(x0 + r.bottom_left, y1 - r.bottom_left),
                    [down, left],
                ),
            ];
            ops.push(Op::Move(Point::new(x0, y0 + r.top_left)));
            // The corners usually share a radius: work its polygon out once.
            let mut last: Option<(f64, ArcPolygon)> = None;
            for (i, (rad, centre, [from, to])) in corners.into_iter().enumerate() {
                if rad > 0.0 {
                    let poly = match last {
                        Some((r, p)) if r == rad => p,
                        _ => ArcPolygon::new(rad, tolerance, FRAC_PI_2),
                    };
                    last = Some((rad, poly));
                    if i > 0 {
                        ops.push(Op::Line(centre + from * rad));
                    }
                    ops.push(Op::ArcOp(ArcOp {
                        centre,
                        from,
                        end: centre + to * rad,
                        poly,
                    }));
                } else if i > 0 {
                    ops.push(Op::Line(centre));
                }
            }
            ops.push(Op::Close);
        }
        PaintShape::Circle(c) => {
            let east = Vec2::new(1.0, 0.0);
            let start = c.center + east * c.radius;
            ops.push(Op::Move(start));
            ops.push(Op::ArcOp(ArcOp {
                centre: c.center,
                from: east,
                end: start,
                poly: ArcPolygon::new(c.radius, tolerance, TAU),
            }));
            ops.push(Op::Close);
        }
        PaintShape::Rect(_) | PaintShape::Line(_) => return None,
    }
    Some(Flat {
        ops,
        path: None,
        tolerance,
        bounds,
    })
}

/// [`fine`]'s lines as a path (tests, and anything that needs them twice).
#[cfg(test)]
pub(crate) fn flattened(transform: Affine, shape: &PaintShape) -> Option<peniko::kurbo::BezPath> {
    use peniko::kurbo::Shape;
    fine(transform, shape).map(|f| f.path_elements(0.0).collect())
}

/// A curved [`PaintShape`] flattened lazily: see [`fine`]. A path's own
/// elements when `path` is set, else a rounded rect's or circle's outline as
/// `ops`.
pub(crate) struct Flat<'a> {
    ops: Ops,
    path: Option<&'a [peniko::kurbo::PathEl]>,
    tolerance: f64,
    bounds: Rect,
}

/// At most a move, four lines and four arcs, and a close.
#[derive(Clone, Copy, Default)]
struct Ops {
    ops: [Op; 10],
    len: usize,
}

impl Ops {
    fn push(&mut self, op: Op) {
        self.ops[self.len] = op;
        self.len += 1;
    }
}

#[derive(Clone, Copy, Default)]
enum Op {
    #[default]
    Close,
    Move(Point),
    Line(Point),
    ArcOp(ArcOp),
}

/// A circular arc about `centre` from the unit direction `from` to the point
/// `end` (on the circle), drawn as `poly`'s chords.
#[derive(Clone, Copy)]
struct ArcOp {
    centre: Point,
    from: Vec2,
    end: Point,
    poly: ArcPolygon,
}

/// The polygon an arc of radius `r` sweeping `sweep` radians is drawn as,
/// within `tolerance` of it.
///
/// A chord across angle `a` between two points on the circle lies up to
/// `s = r (1 - cos(a / 2))` inside it, and the polygon of such chords covers
/// less than the circle everywhere. Moving the vertices out to
/// `2r / (1 + cos(a / 2))` puts them about `s / 2` outside and each chord's
/// middle as far inside, so `a` may be the angle whose `s` is twice the
/// tolerance (about 1.4x fewer chords), and the coverage errors either side
/// partly cancel. The vertices sit half a step in from the arc's ends, so the
/// two end chords (from an end on the circle to the first vertex outside it)
/// stray no further.
#[derive(Clone, Copy)]
struct ArcPolygon {
    /// Vertices between the arc's ends (0: one chord is within tolerance).
    n: usize,
    /// Their distance from the centre.
    radius: f64,
    /// `sin_cos` of half the angle between two of them.
    half: (f64, f64),
}

impl ArcPolygon {
    /// `sweep` is signed: positive turns from +x towards +y (clockwise on
    /// screen).
    fn new(r: f64, tolerance: f64, sweep: f64) -> Self {
        let none = Self {
            n: 0,
            radius: r,
            half: (0.0, 1.0),
        };
        // `x = 2 tolerance / r` is `1 - cos(a / 2)` for the widest chord. An
        // arc that sags no more than the tolerance is one chord.
        let x = 2.0 * tolerance / r;
        if r.is_nan() || r <= 0.0 || x >= 1.0 || 0.5 * x >= 1.0 - (sweep / 2.0).cos() {
            return none;
        }
        // `acos(1 - x) >= sqrt(2x) (1 + x / 12)`: no wider than allowed.
        let a = 2.0 * (2.0 * x).sqrt() * (1.0 + x / 12.0);
        // A negative sweep turns the other way: `half`'s sine carries it.
        let n = ((sweep.abs() / a).ceil() as usize).clamp(1, 4096);
        let half = (sweep / (2 * n) as f64).sin_cos();
        Self {
            n,
            radius: 2.0 * r / (1.0 + half.1),
            half,
        }
    }
}

/// `v` turned by the angle whose `sin_cos` is `(sin, cos)`.
fn rotate(v: Vec2, (sin, cos): (f64, f64)) -> Vec2 {
    Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

impl peniko::kurbo::Shape for Flat<'_> {
    type PathElementsIter<'iter>
        = FlatIter<'iter>
    where
        Self: 'iter;

    fn path_elements(&self, _tolerance: f64) -> FlatIter<'_> {
        FlatIter {
            ops: &self.ops,
            path: self.path,
            index: 0,
            current: Point::ZERO,
            start: Point::ZERO,
            emit: Emit::Idle,
            tolerance: self.tolerance,
            quarter: None,
        }
    }

    fn area(&self) -> f64 {
        peniko::kurbo::BezPath::from_vec(self.path_elements(0.0).collect()).area()
    }

    fn perimeter(&self, accuracy: f64) -> f64 {
        peniko::kurbo::BezPath::from_vec(self.path_elements(0.0).collect()).perimeter(accuracy)
    }

    fn winding(&self, pt: Point) -> i32 {
        peniko::kurbo::BezPath::from_vec(self.path_elements(0.0).collect()).winding(pt)
    }

    fn bounding_box(&self) -> Rect {
        self.bounds
    }
}

/// [`Flat`]'s path elements, worked out one at a time: its subpaths, their
/// start points and their direction are the source's, and its lines are
/// kept as they are.
///
/// A path's cubic that is a circular arc (what kurbo makes of a
/// `RoundedRect`'s or `BorderRadii`'s circular corners, so a solid rounded
/// border's ring and a shadow's shape: endpoints on one circle, tangent to
/// it, equal arms, and its middle on it too) is drawn as [`ArcPolygon`]'s
/// chords, as a rounded rect is. Any other quad or cubic is cut into `n`
/// equal steps of its parameter, `n` by Wang's formula (`n^2 >= d(d - 1) M /
/// (8 tol)`, `M` the largest second difference of its control points), which
/// bounds the distance from the curve to its chords by the tolerance with no
/// recursion.
pub(crate) struct FlatIter<'a> {
    ops: &'a Ops,
    path: Option<&'a [peniko::kurbo::PathEl]>,
    index: usize,
    current: Point,
    start: Point,
    emit: Emit,
    tolerance: f64,
    /// The last circular quarter's polygon, by radius and turn.
    quarter: Option<(f64, f64, ArcPolygon)>,
}

/// The lines of the curve being emitted.
#[derive(Clone, Copy)]
enum Emit {
    Idle,
    /// `left` more vertices at `radius` from `centre`, the next in direction
    /// `v`, each turned by `turn` from the last; then `end`.
    Arc {
        centre: Point,
        v: Vec2,
        turn: (f64, f64),
        radius: f64,
        left: usize,
        end: Point,
    },
    /// The points at `t = i / n` of the Bezier `pts` (a quad when `quad`),
    /// up to `n - 1`; then its end.
    Curve {
        pts: [Vec2; 4],
        quad: bool,
        i: usize,
        n: usize,
    },
}

impl FlatIter<'_> {
    fn start_arc(&mut self, arc: ArcOp) {
        let (sin, cos) = arc.poly.half;
        self.emit = Emit::Arc {
            centre: arc.centre,
            v: rotate(arc.from, arc.poly.half),
            turn: (2.0 * sin * cos, 2.0 * cos * cos - 1.0),
            radius: arc.poly.radius,
            left: arc.poly.n,
            end: arc.end,
        };
        self.current = arc.end;
    }
}

impl Iterator for FlatIter<'_> {
    type Item = peniko::kurbo::PathEl;

    fn next(&mut self) -> Option<peniko::kurbo::PathEl> {
        use peniko::kurbo::PathEl;
        loop {
            match &mut self.emit {
                Emit::Idle => {}
                Emit::Arc {
                    centre,
                    v,
                    turn,
                    radius,
                    left,
                    end,
                } => {
                    if *left > 0 {
                        *left -= 1;
                        let p = *centre + *v * *radius;
                        *v = rotate(*v, *turn);
                        return Some(PathEl::LineTo(p));
                    }
                    let end = *end;
                    self.emit = Emit::Idle;
                    return Some(PathEl::LineTo(end));
                }
                Emit::Curve { pts, quad, i, n } => {
                    let [a, b, c, d] = *pts;
                    if *i < *n {
                        let t = *i as f64 / *n as f64;
                        let mt = 1.0 - t;
                        *i += 1;
                        let p = if *quad {
                            a * (mt * mt) + b * (2.0 * mt * t) + c * (t * t)
                        } else {
                            a * (mt * mt * mt)
                                + b * (3.0 * mt * mt * t)
                                + c * (3.0 * mt * t * t)
                                + d * (t * t * t)
                        };
                        return Some(PathEl::LineTo(p.to_point()));
                    }
                    let end = if *quad { c } else { d };
                    self.emit = Emit::Idle;
                    return Some(PathEl::LineTo(end.to_point()));
                }
            }
            let el = match self.path {
                None => {
                    if self.index >= self.ops.len {
                        return None;
                    }
                    let op = self.ops.ops[self.index];
                    self.index += 1;
                    match op {
                        Op::Move(p) => PathEl::MoveTo(p),
                        Op::Line(p) => PathEl::LineTo(p),
                        Op::Close => PathEl::ClosePath,
                        Op::ArcOp(arc) => {
                            self.start_arc(arc);
                            continue;
                        }
                    }
                }
                Some(els) => {
                    let el = *els.get(self.index)?;
                    self.index += 1;
                    el
                }
            };
            match el {
                PathEl::MoveTo(p) => {
                    (self.current, self.start) = (p, p);
                    return Some(el);
                }
                PathEl::LineTo(p) => {
                    self.current = p;
                    return Some(el);
                }
                PathEl::ClosePath => {
                    self.current = self.start;
                    return Some(el);
                }
                PathEl::QuadTo(p1, p2) => {
                    let (a, b, c) = (self.current.to_vec2(), p1.to_vec2(), p2.to_vec2());
                    let n = wang_steps(0.25 * (a - 2.0 * b + c).hypot(), self.tolerance);
                    self.emit = Emit::Curve {
                        pts: [a, b, c, c],
                        quad: true,
                        i: 1,
                        n,
                    };
                    self.current = p2;
                }
                PathEl::CurveTo(p1, p2, p3) => {
                    if let Some((centre, r, from, sweep)) =
                        circular_arc(self.current, p1, p2, p3, self.tolerance)
                    {
                        let quarter_turn = (sweep.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-9;
                        let poly = match self.quarter {
                            Some((qr, qs, p)) if quarter_turn && qr == r && qs == sweep => p,
                            _ => {
                                let p = ArcPolygon::new(r, self.tolerance, sweep);
                                if quarter_turn {
                                    self.quarter = Some((r, sweep, p));
                                }
                                p
                            }
                        };
                        self.start_arc(ArcOp {
                            centre,
                            from,
                            end: p3,
                            poly,
                        });
                    } else {
                        let (a, b, c, d) = (
                            self.current.to_vec2(),
                            p1.to_vec2(),
                            p2.to_vec2(),
                            p3.to_vec2(),
                        );
                        let m = (a - 2.0 * b + c).hypot().max((b - 2.0 * c + d).hypot());
                        self.emit = Emit::Curve {
                            pts: [a, b, c, d],
                            quad: false,
                            i: 1,
                            n: wang_steps(0.75 * m, self.tolerance),
                        };
                        self.current = p3;
                    }
                }
            }
        }
    }
}

/// Wang's step count for a curve whose `d(d - 1) / 8 * M` is `k`.
fn wang_steps(k: f64, tolerance: f64) -> usize {
    let n = (k / tolerance).sqrt().ceil();
    if n.is_finite() {
        (n as usize).clamp(1, 4096)
    } else {
        1
    }
}

/// The circle the cubic `(p0, p1, p2, p3)` is an arc of, as `(centre,
/// radius, from, sweep)` (`from` the unit direction of its start, `sweep`
/// signed as [`ArcPolygon::new`] takes it), or `None` when it is not one to
/// within `tolerance` (local px), or sweeps more than a quarter turn.
///
/// The arc is drawn as the circle, so the cubic must be the circle's own
/// cubic: its arms each `4/3 tan(sweep / 4) r` long (kurbo's, which puts its
/// middle on the circle) to within the tolerance, tangent at both ends, and
/// its middle on the circle. A cubic wider than a quarter turn is left to
/// Wang's steps: one cubic is too far from a circle there, and past a half
/// turn the angle between the ends no longer says which way round it goes
/// (review of #2380). The sweep's sign is the start tangent's.
fn circular_arc(
    p0: Point,
    p1: Point,
    p2: Point,
    p3: Point,
    tolerance: f64,
) -> Option<(Point, f64, Vec2, f64)> {
    use std::f64::consts::FRAC_PI_2;
    let (t0, t1, d) = (p1 - p0, p3 - p2, p3 - p0);
    let (l0, l1) = (t0.hypot(), t1.hypot());
    if l0.is_nan() || l0 <= 0.0 || l1.is_nan() || l1 <= 0.0 {
        return None;
    }
    // The centre is on the normal at `p0`, on the side `p3` is, as far from
    // `p3` as from `p0`: `r = |d|^2 / (2 u.d)`.
    let mut u = Vec2::new(-t0.y, t0.x) / l0;
    if u.dot(d) < 0.0 {
        u = -u;
    }
    let ud = u.dot(d);
    if ud.is_nan() || ud <= 0.0 {
        return None;
    }
    let r = d.hypot2() / (2.0 * ud);
    let centre = p0 + u * r;
    let (a, b) = (p0 - centre, p3 - centre);
    // Which way round: the start tangent's turn about the centre. The end
    // must be reached turning that way, by no more than a quarter turn (and
    // a hair, for rounding).
    let turn = a.cross(t0).signum();
    let (cross, dot) = (a.cross(b), a.dot(b));
    if cross * turn <= 0.0 || dot < -1e-6 * r * r {
        return None;
    }
    let sweep = if dot.abs() < 1e-12 * r * r {
        FRAC_PI_2
    } else {
        cross.abs().atan2(dot).min(FRAC_PI_2)
    };
    // The circle's own cubic, to within the tolerance: tangent at the end,
    // both arms the right length, and the middle on the circle.
    let arm = if sweep == FRAC_PI_2 {
        4.0 / 3.0 * (std::f64::consts::FRAC_PI_8).tan() * r
    } else {
        4.0 / 3.0 * (sweep / 4.0).tan() * r
    };
    let mid =
        ((p0.to_vec2() + 3.0 * p1.to_vec2() + 3.0 * p2.to_vec2() + p3.to_vec2()) / 8.0).to_point();
    if t1.dot(b).abs() > tolerance * l1
        || (l0 - arm).abs() > tolerance
        || (l1 - arm).abs() > tolerance
        || ((mid - centre).hypot() - r).abs() > tolerance
    {
        return None;
    }
    Some((centre, r, a / r, sweep * turn))
}

/// Dispatch a fill operation on a `PaintShape` to the appropriate Scene
/// method: its curves flattened by [`fine`] unless `coarse`.
macro_rules! dispatch_fill {
    ($scene:expr, $fill:expr, $transform:expr, $brush:expr, $shape:expr) => {
        dispatch_fill!($scene, $fill, $transform, $brush, None, $shape)
    };
    ($scene:expr, $fill:expr, $transform:expr, $brush:expr, $bt:expr, $shape:expr) => {
        dispatch_fill!($scene, $fill, $transform, $brush, $bt, $shape, coarse: false)
    };
    ($scene:expr, $fill:expr, $transform:expr, $brush:expr, $bt:expr, $shape:expr, coarse: $coarse:expr) => {
        match if $coarse { None } else { fine($transform, $shape) } {
            Some(f) => $scene.fill($fill, $transform, $brush, $bt, &f),
            None => match $shape {
                PaintShape::Rect(r) => $scene.fill($fill, $transform, $brush, $bt, r),
                PaintShape::RoundedRect(r) => $scene.fill($fill, $transform, $brush, $bt, r),
                PaintShape::BezPath(p) => $scene.fill($fill, $transform, $brush, $bt, p),
                PaintShape::Circle(c) => $scene.fill($fill, $transform, $brush, $bt, c),
                PaintShape::Line(l) => $scene.fill($fill, $transform, $brush, $bt, l),
            },
        }
    };
}

/// Dispatch a stroke operation on a `PaintShape` to the appropriate Scene
/// method, its curves flattened by [`fine`].
macro_rules! dispatch_stroke {
    ($scene:expr, $stroke:expr, $transform:expr, $brush:expr, $shape:expr) => {
        match fine($transform, $shape) {
            Some(f) => $scene.stroke($stroke, $transform, $brush, None, &f),
            None => match $shape {
                PaintShape::Rect(r) => $scene.stroke($stroke, $transform, $brush, None, r),
                PaintShape::RoundedRect(r) => $scene.stroke($stroke, $transform, $brush, None, r),
                PaintShape::BezPath(p) => $scene.stroke($stroke, $transform, $brush, None, p),
                PaintShape::Circle(c) => $scene.stroke($stroke, $transform, $brush, None, c),
                PaintShape::Line(l) => $scene.stroke($stroke, $transform, $brush, None, l),
            },
        }
    };
}

/// Dispatch a clip operation on a `PaintShape` to the appropriate Scene
/// method, its curves flattened by [`fine`].
macro_rules! dispatch_clip {
    ($scene:expr, $fill:expr, $transform:expr, $shape:expr) => {
        match fine($transform, $shape) {
            Some(f) => $scene.push_clip_layer($fill, $transform, &f),
            None => match $shape {
                PaintShape::Rect(r) => $scene.push_clip_layer($fill, $transform, r),
                PaintShape::RoundedRect(r) => $scene.push_clip_layer($fill, $transform, r),
                PaintShape::BezPath(p) => $scene.push_clip_layer($fill, $transform, p),
                PaintShape::Circle(c) => $scene.push_clip_layer($fill, $transform, c),
                PaintShape::Line(l) => $scene.push_clip_layer($fill, $transform, l),
            },
        }
    };
}

/// Dispatch a push_layer operation on a `PaintShape`, its curves flattened
/// by [`fine`].
macro_rules! dispatch_push_layer {
    ($scene:expr, $fill:expr, $mix:expr, $opacity:expr, $transform:expr, $shape:expr) => {
        match fine($transform, $shape) {
            Some(f) => $scene.push_layer($fill, $mix, $opacity, $transform, &f),
            None => match $shape {
                PaintShape::Rect(r) => $scene.push_layer($fill, $mix, $opacity, $transform, r),
                PaintShape::RoundedRect(r) => {
                    $scene.push_layer($fill, $mix, $opacity, $transform, r)
                }
                PaintShape::BezPath(p) => $scene.push_layer($fill, $mix, $opacity, $transform, p),
                PaintShape::Circle(c) => $scene.push_layer($fill, $mix, $opacity, $transform, c),
                PaintShape::Line(l) => $scene.push_layer($fill, $mix, $opacity, $transform, l),
            },
        }
    };
}

/// Fill `shape` with `brush` into `scene`.
pub(crate) fn fill_into(
    scene: &mut Scene,
    fill: Fill,
    transform: Affine,
    brush: &Brush,
    shape: &PaintShape,
) {
    dispatch_fill!(scene, fill, transform, brush, shape);
}

/// Push `shape` as a clip into `scene`.
pub(crate) fn push_clip_into(scene: &mut Scene, fill: Fill, transform: Affine, shape: &PaintShape) {
    dispatch_clip!(scene, fill, transform, shape);
}

/// Push a `blend` layer at `opacity` into `scene`.
pub(crate) fn push_layer_into(
    scene: &mut Scene,
    blend: BlendMode,
    opacity: f32,
    transform: Affine,
    bounds: &PaintShape,
) {
    let mix = match blend {
        BlendMode::Normal => peniko::BlendMode::from(peniko::Mix::Normal),
        BlendMode::Saturation => peniko::BlendMode::from(peniko::Mix::Saturation),
        BlendMode::Plus => peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::Plus),
        BlendMode::DestinationIn => {
            peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::DestIn)
        }
        BlendMode::DestinationOut => {
            peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::DestOut)
        }
        BlendMode::Mix(mode) => peniko::BlendMode::from(peniko_mix(mode)),
    };
    dispatch_push_layer!(scene, Fill::NonZero, mix, opacity, transform, bounds);
}

/// Vello's mix for a compositing-1 blend mode (#2221).
fn peniko_mix(mode: super::painter::MixMode) -> peniko::Mix {
    use super::painter::MixMode as M;
    match mode {
        M::Multiply => peniko::Mix::Multiply,
        M::Screen => peniko::Mix::Screen,
        M::Overlay => peniko::Mix::Overlay,
        M::Darken => peniko::Mix::Darken,
        M::Lighten => peniko::Mix::Lighten,
        M::ColorDodge => peniko::Mix::ColorDodge,
        M::ColorBurn => peniko::Mix::ColorBurn,
        M::HardLight => peniko::Mix::HardLight,
        M::SoftLight => peniko::Mix::SoftLight,
        M::Difference => peniko::Mix::Difference,
        M::Exclusion => peniko::Mix::Exclusion,
        M::Hue => peniko::Mix::Hue,
        M::Saturation => peniko::Mix::Saturation,
        M::Color => peniko::Mix::Color,
        M::Luminosity => peniko::Mix::Luminosity,
    }
}

/// The device bounds of `shape` under `transform`.
pub(crate) fn shape_bounds(transform: Affine, shape: &PaintShape) -> Rect {
    transform.transform_rect_bbox(shape.control_bounds())
}

/// The brush `draw_image` fills with (#2098): Vello's default sampler
/// (`ImageQuality::Medium`, bilinear) for `ImageSampling::Auto`, exactly the
/// brush it was before, and `ImageQuality::Low` — nearest-neighbour — for
/// `image-rendering: pixelated` / `crisp-edges`. The quality is a brush
/// parameter, not part of the image Vello keeps in its atlas (keyed by the
/// blob), so one image drawn both ways is uploaded once.
fn image_brush(image: ImageData, sampling: ImageSampling) -> peniko::ImageBrush {
    let brush = peniko::ImageBrush::new(image);
    match sampling {
        ImageSampling::Auto => brush,
        ImageSampling::Nearest => brush.with_quality(peniko::ImageQuality::Low),
    }
}

impl Painter for VelloPainter {
    fn reset(&mut self) {
        self.scene.reset();
        self.open.clear();
        self.layers_open = 0;
        self.tile_target = EVERYWHERE;
        self.clip_tiles = 0;
        self.slicing = None;
        self.coarse_curves = false;
        self.routing = false;
        self.reads_backdrop = false;
        #[cfg(feature = "glyph-atlas")]
        {
            self.glyph_stats.atlas.evictions += self
                .atlas
                .borrow_mut()
                .begin_painter_frame(self.atlas_painter);
            self.churn.begin_frame();
        }
    }

    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape) {
        // Vello's fine shader evaluates a gradient at each pixel's top-left
        // corner (it sampled images there too before Vello 0.9, #1027),
        // where CSS and tiny-skia evaluate it at the centre: a 1px hard-stop line of a repeating
        // gradient fell between two corners and was not drawn (#1798).
        let bt = matches!(brush, Brush::Gradient(_))
            .then(|| centre_sampling(transform, Affine::IDENTITY));
        let coarse = self.coarse_curves;
        route!(
            self,
            || Some(shape_bounds(transform, shape)),
            |scene| dispatch_fill!(scene, fill, transform, brush, bt, shape, coarse: coarse)
        );
    }

    fn set_coarse_curves(&mut self, coarse: bool) {
        self.coarse_curves = coarse;
    }

    fn stroke(
        &mut self,
        stroke: &peniko::kurbo::Stroke,
        transform: Affine,
        brush: &Brush,
        shape: &PaintShape,
    ) {
        route!(
            self,
            || {
                // The stroke's reach past its path (half the width, more at
                // a miter join or a square cap), under a bound on the
                // transform's largest stretch.
                let [a, b, c, d, _, _] = transform.as_coeffs();
                let grow = super::svg::stroke_outset(stroke)
                    * (a.abs() + b.abs() + c.abs() + d.abs())
                    + 1.0;
                Some(shape_bounds(transform, shape).inflate(grow, grow))
            },
            |scene| dispatch_stroke!(scene, stroke, transform, brush, shape)
        );
    }

    fn draw_glyphs(
        &mut self,
        font: &FontData,
        font_size: f32,
        transform: Affine,
        synthesis: GlyphSynthesis,
        brush: &Brush,
        hint: bool,
        normalized_coords: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        // From the glyph-mask atlas wherever it can draw the run (#2286).
        #[cfg(feature = "glyph-atlas")]
        if self.atlas_enabled
            && self.draw_atlas_glyphs(
                font,
                font_size,
                transform,
                synthesis,
                brush,
                hint,
                normalized_coords,
                glyphs,
            )
        {
            return;
        }
        self.glyph_stats.outline_fallbacks += glyphs.len() as u64;
        // The hottest outline draw: with nothing split it is exactly the
        // encode, and the routed draw is a call out of line (#1899 review:
        // through `route!` the glyph path still cost 8.1k instructions a text
        // page).
        if self.routing {
            return self.draw_glyphs_routed(
                font,
                font_size,
                transform,
                synthesis,
                brush,
                hint,
                normalized_coords,
                glyphs,
            );
        }
        encode_glyphs(
            &mut self.scene,
            font,
            font_size,
            transform,
            synthesis,
            brush,
            hint,
            normalized_coords,
            glyphs,
        );
    }

    fn draw_image(&mut self, image: &PaintImage<'_>, transform: Affine) {
        // One blob per image, not per paint: Vello keys its atlas on the
        // blob's id (#1955). A decoded image keeps its blob; a live frame's
        // buffer maps to one blob while it lives; pixels made for this draw
        // alone get a blob of their own.
        let blob: Blob<u8> = match (image.decoded, image.frame) {
            (Some(decoded), _) => decoded.blob(),
            (None, Some(frame)) => frame_blob(frame),
            (None, None) => Blob::from(image.data.to_vec()),
        };
        let image_data = ImageData {
            data: blob,
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: image.width,
            height: image.height,
        };
        // `Scene::draw_image`. Vello samples an image at each pixel's centre
        // (since Vello 0.9), as CSS and tiny-skia do (#1027).
        let rect = peniko::kurbo::Rect::new(0.0, 0.0, image.width as f64, image.height as f64);
        let brush = image_brush(image_data, image.sampling);
        route!(
            self,
            || Some(transform.transform_rect_bbox(rect)),
            |scene| scene.fill(Fill::NonZero, transform, &brush, None, &rect)
        );
    }

    fn draw_gpu_image(&mut self, image: &ImageData, transform: Affine) {
        // As `draw_image`, sampled at each pixel's centre (#1027). The pixels
        // come from the registered texture.
        let rect = peniko::kurbo::Rect::new(0.0, 0.0, image.width as f64, image.height as f64);
        let brush = peniko::ImageBrush::new(image.clone());
        route!(
            self,
            || Some(transform.transform_rect_bbox(rect)),
            |scene| scene.fill(Fill::NonZero, transform, &brush, None, &rect)
        );
    }

    fn fill_repeating(
        &mut self,
        fill: Fill,
        transform: Affine,
        tile: &crate::paint::painter::RepeatTile<'_>,
        shape: &PaintShape,
    ) {
        let image = ImageData {
            data: tile.pixels.clone(),
            format: ImageFormat::Rgba8,
            alpha_type: if tile.premultiplied {
                ImageAlphaType::AlphaPremultiplied
            } else {
                ImageAlphaType::Alpha
            },
            width: tile.width,
            height: tile.height,
        };
        // A tile whose texels are device pixels is placed on whole device
        // pixels and sampled nearest, as before #1151. Vello clamps its
        // bilinear taps at the image's edge after `Extend::Repeat`
        // (`fine.wgsl`, `CMD_IMAGE`), so at a fractional phase the pixel on a
        // boundary between tiles takes one side's texel while the inner edges
        // blend: during a sub-pixel animation the stripes pulse in width
        // (4.5–5.5px for a 10px tile) and brightness (±9%), which looks worse
        // than whole-pixel steps. The software pattern wraps its taps and
        // keeps the fraction.
        let mut tile_transform = tile.transform;
        let mut smooth = tile.smooth;
        let total = (transform * tile.transform).as_coeffs();
        let det = transform.determinant();
        if smooth
            && det.is_finite()
            && det != 0.0
            && (total[0] - 1.0).abs() < 1e-9
            && total[1].abs() < 1e-9
            && total[2].abs() < 1e-9
            && (total[3] - 1.0).abs() < 1e-9
        {
            let stepped = Affine::translate((total[4].round(), total[5].round()));
            tile_transform = transform.inverse() * stepped;
            smooth = false;
        }
        let quality = if smooth {
            peniko::ImageQuality::Medium
        } else {
            peniko::ImageQuality::Low
        };
        let brush: Brush = peniko::ImageBrush::new(image)
            .with_extend(peniko::Extend::Repeat)
            .with_quality(quality)
            .into();
        // Sampled at each pixel's centre by Vello itself (#1027, #1955).
        let brush_transform = Some(tile_transform);
        route!(
            self,
            || Some(shape_bounds(transform, shape)),
            |scene| dispatch_fill!(scene, fill, transform, &brush, brush_transform, shape)
        );
    }

    /// A clip that holds the whole region drawing is already confined to cuts
    /// nothing, and is not pushed (judged only while every open clip is an
    /// axis-aligned rect, so the region is exact). Vello allocates a clip's
    /// tiles over its whole bounds and blends every one of them twice, whatever
    /// it removes; a nested paint sequence re-opening a window-sized clip of an
    /// outer one (a pane inside panes inside a window, each its own stacking
    /// context) paid a window of tiles each time, cutting nothing. Its pop is
    /// matched, and pops nothing. On the split-layer path it is still handed to
    /// the split, which reasons over the whole chain, and only the window's
    /// scene skips it, so a frame opened for slicing that splits nothing
    /// encodes as an unsliced one does.
    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape) {
        let (region, exact) = self.region();
        let elide = exact && clip_holds(transform, shape, region);
        if self.slicing.is_none() {
            if elide {
                self.open.push(OpenLayer {
                    pushed: false,
                    region,
                    layer: false,
                    exact,
                });
                return;
            }
            self.opened(transform, shape, false);
            return push_clip_into(&mut self.scene, fill, transform, shape);
        }
        if elide {
            self.open.push(OpenLayer {
                pushed: true,
                region,
                layer: false,
                exact,
            });
        } else {
            self.opened(transform, shape, false);
        }
        self.push_sliced(|| Open::Clip {
            fill,
            transform,
            shape: shape.clone(),
            below: !elide,
        });
    }

    fn push_layer(
        &mut self,
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        bounds: &PaintShape,
    ) {
        if matches!(blend, BlendMode::Saturation | BlendMode::Mix(_)) {
            self.reads_backdrop = true;
        }
        self.opened(transform, bounds, true);
        if self.slicing.is_none() {
            return push_layer_into(&mut self.scene, blend, opacity, transform, bounds);
        }
        self.push_sliced(|| Open::Layer {
            blend,
            opacity,
            transform,
            bounds: bounds.clone(),
        });
    }

    fn push_isolated_layer(&mut self, opacity: f32, transform: Affine, bounds: &PaintShape) {
        self.opened(transform, bounds, true);
        if self.slicing.is_none() {
            return push_layer_into(
                &mut self.scene,
                BlendMode::Normal,
                opacity,
                transform,
                bounds,
            );
        }
        self.push_sliced(|| Open::Isolated {
            opacity,
            transform,
            bounds: bounds.clone(),
        });
    }

    fn pop_layer(&mut self) {
        // An elided clip's pop pops nothing.
        let popped = self.open.pop();
        if let Some(o) = popped
            && o.layer
        {
            self.layers_open -= 1;
        }
        if !popped.is_none_or(|o| o.pushed) {
            return;
        }
        if self.slicing.is_none() {
            return self.scene.pop_layer();
        }
        self.pop_sliced();
    }

    fn split_surface(
        &mut self,
        split: &super::painter::SurfaceSplit,
    ) -> super::painter::SplitVerdict {
        let verdict = match &mut self.slicing {
            Some(slicing) => slicing.split(&mut self.scene, split),
            None => super::painter::SplitVerdict::InScene(super::painter::Refusal::Unsupported),
        };
        self.routing |= verdict == super::painter::SplitVerdict::Split;
        verdict
    }

    fn note_node_start(&mut self, node: crate::node::RawNodeId) {
        if let Some(slicing) = &mut self.slicing {
            // The first start is the earliest: the count only grows.
            let n = slicing.slices.len();
            slicing.starts.entry(node).or_insert(n);
        }
    }

    fn append(&mut self, other: &Self) {
        // Bounds unknown: routed as across every region. A scene that blends
        // with its backdrop taints every split surface (D5a).
        if other.reads_backdrop
            && let Some(slicing) = &mut self.slicing
        {
            slicing.taint_all();
        }
        self.clip_tiles = self.clip_tiles.saturating_add(other.clip_tiles);
        let scene = &other.scene;
        route!(self, || None, |s| s.append(scene, None));
    }
}

#[cfg(test)]
mod sampling_tests {
    //! What Vello 0.11's fine shader samples for the images this painter
    //! encodes (#1027, #1955). There is no GPU in CI, so the shader's image arm
    //! (`fine.wgsl`, `CMD_IMAGE`) is modelled here on the CPU, in `f32`, from
    //! the transform the scene actually encodes for the brush. The model is
    //! checked against a real GPU by the `#[ignore]`d
    //! `gpu_alpha_tests::vello_draw_image_*` fixtures in
    //! `crates/rinch/src/shell/compositor.rs`.
    use super::*;
    use crate::paint::painter::{PaintImage, Painter, RepeatTile};
    use peniko::kurbo::Rect;

    /// A 4x2 image of distinct opaque greys, so any two neighbours differ.
    const W: u32 = 4;
    const H: u32 = 2;
    const TEXELS: [u8; 8] = [10, 200, 40, 250, 120, 0, 230, 70];

    fn rgba() -> Vec<u8> {
        TEXELS.iter().flat_map(|&g| [g, g, g, 255]).collect()
    }

    /// The last transform the scene encoded: the brush's (the image's
    /// image-to-device map) when the draw carries a brush transform, and the
    /// shape's otherwise, which is then the brush's too.
    fn brush_transform(p: &VelloPainter) -> ([f32; 4], [f32; 2]) {
        let t = p.scene().encoding().transforms.last().expect("a transform");
        (t.matrix, t.translation)
    }

    /// `fine.wgsl`'s `CMD_IMAGE` arm for an image at atlas offset 0 with
    /// `Extend::Pad`, for the device pixel whose top-left corner is
    /// `(x, y)`. Since Vello 0.9 the shader's `my_xy` is the pixel's centre,
    /// `(x + 0.5, y + 0.5)` (Vello 0.7 used the corner). Returns the grey
    /// channel.
    fn shader_sample(
        (m, t): ([f32; 4], [f32; 2]),
        x: f32,
        y: f32,
        medium: bool,
        repeat: bool,
    ) -> f32 {
        // transform_inverse
        let inv_det = 1.0 / (m[0] * m[3] - m[1] * m[2]);
        let im = [
            inv_det * m[3],
            -inv_det * m[1],
            -inv_det * m[2],
            inv_det * m[0],
        ];
        let it = [
            -(im[0] * t[0] + im[2] * t[1]),
            -(im[1] * t[0] + im[3] * t[1]),
        ];
        let (ew, eh) = (W as f32, H as f32);
        let (x, y) = (x + 0.5, y + 0.5);
        let mut u = im[0] * x + im[2] * y + it[0];
        let mut v = im[1] * x + im[3] * y + it[1];
        let extend = |t: f32, max: f32| {
            if repeat {
                (t / max).fract().rem_euclid(1.0) * max
            } else {
                t.clamp(0.0, max)
            }
        };
        u = extend(u, ew);
        v = extend(v, eh);
        let texel = |ix: f32, iy: f32| TEXELS[(iy as usize) * W as usize + ix as usize] as f32;
        if !medium {
            let (cu, cv) = (u.clamp(0.0, ew - 1.0), v.clamp(0.0, eh - 1.0));
            return texel(cu.floor(), cv.floor());
        }
        let (u, v) = (u - 0.5, v - 0.5);
        let (cu, cv) = (u.clamp(0.0, ew - 1.0), v.clamp(0.0, eh - 1.0));
        let (fu, fv) = (u - u.floor(), v - v.floor());
        let a = texel(cu.floor(), cv.floor());
        let b = texel(cu.floor(), cv.ceil());
        let c = texel(cu.ceil(), cv.floor());
        let d = texel(cu.ceil(), cv.ceil());
        let lerp = |p: f32, q: f32, s: f32| p + (q - p) * s;
        lerp(lerp(a, b, fv), lerp(c, d, fv), fu)
    }

    /// Bilinear sampling at the pixel's CENTRE, CSS's and tiny-skia's rule,
    /// computed in `f64` straight from the image-to-device map: the image
    /// point under the centre, less half a texel, interpolated between the
    /// four texels around it (edges clamped).
    fn centre_bilinear(image_to_device: Affine, x: f64, y: f64) -> f64 {
        let p = image_to_device.inverse() * peniko::kurbo::Point::new(x + 0.5, y + 0.5);
        let (u, v) = (p.x - 0.5, p.y - 0.5);
        let texel = |ix: f64, iy: f64| {
            let ix = ix.clamp(0.0, W as f64 - 1.0) as usize;
            let iy = iy.clamp(0.0, H as f64 - 1.0) as usize;
            TEXELS[iy * W as usize + ix] as f64
        };
        let (u0, v0) = (u.floor(), v.floor());
        let (fu, fv) = (u - u0, v - v0);
        let top = texel(u0, v0) * (1.0 - fu) + texel(u0 + 1.0, v0) * fu;
        let bottom = texel(u0, v0 + 1.0) * (1.0 - fu) + texel(u0 + 1.0, v0 + 1.0) * fu;
        top * (1.0 - fv) + bottom * fv
    }

    fn draw(transform: Affine) -> VelloPainter {
        let data = rgba();
        let mut p = VelloPainter::new();
        p.draw_image(
            &PaintImage {
                data: &data,
                width: W,
                height: H,
                decoded: None,
                frame: None,
                opaque: true,
                sampling: Default::default(),
            },
            transform,
        );
        p
    }

    /// The issue's case: scale 1 at a whole-pixel origin reproduces every
    /// texel, as tiny-skia does. Before #1027 Vello read each pixel between
    /// it and its left/top neighbour (the 175 between 153 and 198 of the
    /// issue's table).
    #[test]
    fn an_image_at_a_whole_pixel_origin_samples_its_texels_exactly() {
        let p = draw(Affine::translate((150.0, 37.0)));
        let bt = brush_transform(&p);
        for iy in 0..H {
            for ix in 0..W {
                let got = shader_sample(bt, 150.0 + ix as f32, 37.0 + iy as f32, true, false);
                let want = TEXELS[(iy * W + ix) as usize] as f32;
                assert!(
                    (got - want).abs() < 0.01,
                    "texel ({ix}, {iy}): sampled {got}, the texel is {want}"
                );
            }
        }
    }

    /// Off the fixed point: scaled, rotated and at a fractional origin the
    /// shader samples at each pixel's centre. (A half-pixel shift applied in
    /// IMAGE space rather than device space is right at scale 1 only; this
    /// is the fixture that tells the two apart.)
    #[test]
    fn a_scaled_or_rotated_image_is_sampled_at_pixel_centres() {
        let cases = [
            Affine::translate((10.0, 4.0)) * Affine::scale(2.0),
            Affine::translate((10.3, 4.6)) * Affine::scale_non_uniform(1.5, 2.5),
            Affine::translate((20.0, 3.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2),
        ];
        for m in cases {
            let p = draw(m);
            let bt = brush_transform(&p);
            let bounds = m.transform_rect_bbox(Rect::new(0.0, 0.0, W as f64, H as f64));
            let mut checked = 0;
            for y in bounds.y0.ceil() as i32..bounds.y1.floor() as i32 {
                for x in bounds.x0.ceil() as i32..bounds.x1.floor() as i32 {
                    let got = shader_sample(bt, x as f32, y as f32, true, false) as f64;
                    let want = centre_bilinear(m, x as f64, y as f64);
                    assert!(
                        (got - want).abs() < 0.05,
                        "{m:?} pixel ({x}, {y}): sampled {got}, centre sample {want}"
                    );
                    checked += 1;
                }
            }
            assert!(checked >= 8, "{m:?}: only {checked} pixels checked");
        }
    }

    /// `fill_repeating`'s smooth tile goes through the same shader arm.
    #[test]
    fn a_smooth_repeating_tile_is_sampled_at_pixel_centres() {
        let pixels = peniko::Blob::from(rgba());
        let tile_transform = Affine::scale(1.5);
        // Off the fixed point: a device scale on the fill transform (DPR
        // 1.25), where a shift in the shape's space is not a device half-pixel.
        let fill_transform = Affine::translate((7.0, 3.0)) * Affine::scale(1.25);
        let mut p = VelloPainter::new();
        p.fill_repeating(
            Fill::NonZero,
            fill_transform,
            &RepeatTile {
                pixels: &pixels,
                width: W,
                height: H,
                transform: tile_transform,
                smooth: true,
                premultiplied: true,
            },
            &PaintShape::Rect(Rect::new(0.0, 0.0, 6.0, 3.0)),
        );
        let bt = brush_transform(&p);
        let m = fill_transform * tile_transform;
        // Inside the first tile, away from its wrap edges (where repeat and
        // the edge-clamped reference differ).
        for (x, y) in [(8, 4), (9, 4), (10, 4), (11, 4), (9, 5)] {
            let got = shader_sample(bt, x as f32, y as f32, true, true) as f64;
            let want = centre_bilinear(m, x as f64, y as f64);
            assert!(
                (got - want).abs() < 0.05,
                "pixel ({x}, {y}): sampled {got}, centre sample {want}"
            );
        }
    }

    /// #1151, GPU half: a repeating background at a fractional position is
    /// placed on a whole device pixel on the Vello brush and sampled nearest,
    /// as before #1151 (the software pattern keeps the fraction). Vello clamps
    /// its bilinear taps at the image's edge after `Extend::Repeat`, so a
    /// fractional phase blended the inner edges but not the boundary between
    /// tiles, and a sub-pixel animation pulsed the stripe width. A red/blue
    /// 10px tile at `0.5px`, DPR 1: the tile starts on pixel 1, so pixel 5's
    /// centre is texel 4.5 (nearest: texel 4, red). Kept fractional, it read
    /// 5.0, and blended there.
    #[cfg(feature = "software-renderer")]
    #[test]
    fn a_background_at_a_fractional_position_steps_on_the_gpu() {
        use rinch_core::dom::DomDocument;
        let mut doc = crate::RinchDocument::new();
        doc.load_css("html, body { margin: 0; }");
        let body = doc.body();
        let node = doc.create_element("div");
        doc.set_attribute(
            node,
            "style",
            "position: absolute; left: 0; top: 0; width: 60px; height: 20px; \
             background: linear-gradient(to right, red 0 50%, blue 50% 100%) 0.5px 0 / 10px 10px",
        );
        doc.append_child(body, node);
        doc.resolve_layout(100.0, 40.0);
        let mut p = VelloPainter::new();
        let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        crate::paint::paint_document(
            &doc.tree,
            &mut p,
            1.0,
            (100.0, 40.0),
            &mut doc.font_cx,
            &mut layout_cx,
        );
        let (m, t) = brush_transform(&p);
        let inv_det = 1.0 / (m[0] * m[3] - m[1] * m[2]);
        let (a, c) = (inv_det * m[3], -inv_det * m[2]);
        let u = |x: f32, y: f32| a * x + c * y - (a * t[0] + c * t[1]);
        // The centre of device pixel 5 (row 7), where Vello samples, maps to
        // texel 4.5; nearest reads its floor.
        let at = u(5.5, 7.5);
        assert!(
            (at - 4.5).abs() < 1e-3,
            "pixel 5's centre is texel {at}; 4.5 is the tile placed on pixel 1"
        );
        assert!((m[0] - 1.0).abs() < 1e-6 && m[1] == 0.0 && m[2] == 0.0);
    }
}

#[cfg(test)]
mod blob_identity_tests {
    //! Vello keys its image atlas on the blob's id, and Vello 0.9+ keeps an
    //! image resident across renders: an image must reach it as the same
    //! blob on every paint, or each repaint is a new image (#1955).
    use super::*;
    use crate::image_cache::DecodedImage;
    use crate::paint::painter::{PaintImage, Painter};
    use std::sync::Arc;
    use vello::low_level::vello_encoding::Patch;

    fn image_ids(p: &VelloPainter) -> Vec<u64> {
        p.scene()
            .encoding()
            .resources
            .patches
            .iter()
            .filter_map(|patch| match patch {
                Patch::Image { image, .. } => Some(image.data.id()),
                _ => None,
            })
            .collect()
    }

    fn paint(image: PaintImage<'_>) -> Vec<u64> {
        let mut p = VelloPainter::new();
        p.draw_image(&image, Affine::IDENTITY);
        image_ids(&p)
    }

    fn pixels() -> Vec<u8> {
        vec![255; 2 * 2 * 4]
    }

    /// A decoded image (an `<img>`, a background image) is one blob for
    /// good, painter after painter.
    #[test]
    fn a_decoded_image_is_the_same_blob_on_every_paint() {
        let decoded = DecodedImage::new(pixels(), 2, 2);
        let image = || PaintImage {
            data: &decoded.data,
            width: 2,
            height: 2,
            decoded: Some(&decoded),
            frame: None,
            opaque: true,
            sampling: Default::default(),
        };
        let (first, second) = (paint(image()), paint(image()));
        assert_eq!(first.len(), 1);
        assert_eq!(first, second);
    }

    /// A live frame is one blob while its buffer is the same one, shared
    /// rather than copied; a new frame (a new buffer) is a new blob.
    #[test]
    fn a_live_frame_is_one_blob_per_buffer() {
        let frame = Arc::new(pixels());
        let image = |f: &Arc<Vec<u8>>| {
            let mut p = VelloPainter::new();
            p.draw_image(
                &PaintImage {
                    data: f,
                    width: 2,
                    height: 2,
                    decoded: None,
                    frame: Some(f),
                    opaque: true,
                    sampling: Default::default(),
                },
                Affine::IDENTITY,
            );
            p
        };
        let kept = image(&frame);
        let again = image(&frame);
        assert_eq!(image_ids(&kept), image_ids(&again));
        let shared = kept
            .scene()
            .encoding()
            .resources
            .patches
            .iter()
            .any(|patch| {
                matches!(patch, Patch::Image { image, .. }
                if std::ptr::eq(image.data.data().as_ptr(), frame.as_ptr()))
            });
        assert!(shared, "the frame's buffer is shared, not copied");
        let next = Arc::new(pixels());
        assert_ne!(image_ids(&image(&next)), image_ids(&kept));
    }

    /// Pixels made for one draw (a shadow mask) carry no identity.
    #[test]
    fn pixels_made_per_draw_get_a_blob_each() {
        let data = pixels();
        let image = || PaintImage {
            data: &data,
            width: 2,
            height: 2,
            decoded: None,
            frame: None,
            opaque: true,
            sampling: Default::default(),
        };
        assert_ne!(paint(image()), paint(image()));
    }
}

#[cfg(test)]
mod clip_elision_in_painter_tests {
    use super::*;
    use peniko::kurbo::RoundedRect;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> PaintShape {
        PaintShape::Rect(Rect::new(x0, y0, x1, y1))
    }

    fn pushed(p: &VelloPainter) -> Vec<bool> {
        p.open.iter().map(|o| o.pushed).collect()
    }

    /// A clip holding the whole region drawing is already confined to is not
    /// pushed, and its pop pops nothing.
    #[test]
    fn a_clip_that_cuts_nothing_is_not_pushed_and_its_pop_balances() {
        let mut p = VelloPainter::new();
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(-10.0, -10.0, 900.0, 900.0),
        );
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(100.0, 100.0, 200.0, 200.0),
        );
        assert_eq!(pushed(&p), vec![true, false, false, true]);
        assert_eq!(
            p.scene.encoding().n_clips,
            2,
            "two clips begun on the scene"
        );
        for _ in 0..4 {
            p.pop_layer();
        }
        assert!(p.open.is_empty());
        assert_eq!(p.scene.encoding().n_clips, 4, "two begun, two ended");
    }

    /// A rounded clip cuts its corners: elided only when the region is clear
    /// of them. A rotated clip is never judged, and nothing is judged inside a
    /// clip that leaves less than its bounds.
    #[test]
    fn rounded_and_rotated_clips_are_judged_conservatively() {
        let rounded = PaintShape::RoundedRect(RoundedRect::new(0.0, 0.0, 800.0, 600.0, 20.0));
        let mut p = VelloPainter::new();
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &rounded);
        assert_eq!(
            pushed(&p),
            vec![true, true],
            "the region reaches the corners"
        );
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        assert_eq!(
            pushed(&p),
            vec![true, true, true],
            "inside a rounded clip nothing is judged"
        );

        let mut p = VelloPainter::new();
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(100.0, 100.0, 200.0, 200.0),
        );
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &rounded);
        assert_eq!(
            pushed(&p),
            vec![true, false],
            "a region clear of the corners"
        );
        p.push_clip(
            Fill::NonZero,
            Affine::rotate(0.3),
            &rect(-5000.0, -5000.0, 5000.0, 5000.0),
        );
        assert_eq!(
            pushed(&p),
            vec![true, false, true],
            "a rotated clip is pushed"
        );
        p.push_layer(
            BlendMode::Normal,
            0.5,
            Affine::IDENTITY,
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        assert_eq!(
            pushed(&p),
            vec![true, false, true, true],
            "a layer is always pushed"
        );
    }
}

#[cfg(test)]
mod tile_cost_tests {
    use super::*;
    use crate::paint::painter::{Refusal, SplitVerdict, SurfaceSplit};

    const WINDOW: Rect = Rect::new(0.0, 0.0, 800.0, 600.0);

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> PaintShape {
        PaintShape::Rect(Rect::new(x0, y0, x1, y1))
    }

    fn painter() -> VelloPainter {
        let mut p = VelloPainter::new();
        p.set_tile_target(WINDOW);
        p
    }

    /// Off the tile grid on every side: 15..33 touches columns 0, 1 and 2.
    /// On it, a far edge at a tile's end does not touch the next.
    #[test]
    fn a_box_costs_every_tile_it_touches() {
        let cost = |x0, y0, x1, y1| tile_cost(Rect::new(x0, y0, x1, y1), WINDOW);
        assert_eq!(cost(15.0, 31.0, 33.0, 33.0), 3 * 2);
        assert_eq!(cost(16.0, 32.0, 32.0, 48.0), 1);
        assert_eq!(cost(16.5, 32.5, 31.5, 47.5), 1);
    }

    /// The target cuts first, so a box only part on it pays only that part,
    /// and the plane pays the target, which is 50x38 here.
    #[test]
    fn the_target_cuts_a_box() {
        assert_eq!(tile_cost(EVERYWHERE, WINDOW), 1900);
        assert_eq!(tile_cost(Rect::new(-40.0, -40.0, 17.0, 17.0), WINDOW), 4);
        // Past the far corner of an 809x601 target, whose last tiles are short.
        let target = Rect::new(0.0, 0.0, 809.0, 601.0);
        assert_eq!(tile_cost(Rect::new(790.0, 592.0, 900.0, 700.0), target), 2);
    }

    #[test]
    fn a_box_with_no_area_costs_nothing() {
        let cost = |r| tile_cost(r, WINDOW);
        assert_eq!(cost(Rect::new(20.0, 20.0, 20.0, 400.0)), 0);
        assert_eq!(cost(Rect::new(20.0, 20.0, 400.0, 20.0)), 0);
        assert_eq!(cost(Rect::new(900.0, 0.0, 1000.0, 100.0)), 0, "off target");
        assert_eq!(tile_cost(Rect::new(0.0, 0.0, 100.0, 100.0), Rect::ZERO), 0);
    }

    /// With no target nothing is cut, and the plane saturates.
    #[test]
    fn with_no_target_a_plane_saturates() {
        assert_eq!(tile_cost(EVERYWHERE, EVERYWHERE), u64::MAX);
    }

    /// A clip and each kind of layer pay for their device bounds; an elided
    /// clip pays nothing, and a scale reaches the bounds.
    #[test]
    fn the_painter_counts_what_it_pushes_and_not_what_it_elides() {
        let mut p = painter();
        let t = Affine::IDENTITY;
        p.push_clip(Fill::NonZero, t, &rect(8.0, 8.0, 408.0, 208.0));
        assert_eq!(p.clip_tiles(), 26 * 13, "x tiles 0..26, y tiles 0..13");
        p.push_clip(Fill::NonZero, t, &rect(0.0, 0.0, 800.0, 600.0));
        assert_eq!(p.clip_tiles(), 26 * 13, "the elided clip is free");
        p.push_layer(BlendMode::Normal, 0.5, t, &rect(0.0, 0.0, 32.0, 16.0));
        assert_eq!(p.clip_tiles(), 26 * 13 + 2);
        p.push_isolated_layer(0.5, t, &rect(0.0, 16.0, 16.0, 32.0));
        assert_eq!(p.clip_tiles(), 26 * 13 + 2 + 1);
        for _ in 0..4 {
            p.pop_layer();
        }
        p.push_clip(Fill::NonZero, Affine::scale(2.0), &rect(0.0, 0.0, 8.5, 8.5));
        assert_eq!(p.clip_tiles(), 26 * 13 + 2 + 1 + 4, "17 device px: 2x2");
        p.pop_layer();
        p.reset();
        assert_eq!(p.clip_tiles(), 0);
    }

    /// A rotated clip pays for its axis-aligned device box: a 32-square turned
    /// 45 degrees about its centre at (64, 64) spans 45.25 pixels.
    #[test]
    fn a_rotated_clip_pays_for_its_device_box() {
        let mut p = painter();
        let t = Affine::rotate_about(std::f64::consts::FRAC_PI_4, (64.0, 64.0));
        p.push_clip(Fill::NonZero, t, &rect(48.0, 48.0, 80.0, 80.0));
        // 41.37..86.63: tiles 2..6 on each axis.
        assert_eq!(p.clip_tiles(), 4 * 4);
    }

    /// An appended scene (a drag ghost) adds what its painter counted.
    #[test]
    fn an_appended_scene_brings_its_count() {
        let mut ghost = VelloPainter::new();
        ghost.push_clip(Fill::NonZero, Affine::IDENTITY, &rect(0.0, 0.0, 16.0, 32.0));
        ghost.pop_layer();
        let mut p = painter();
        p.append_scene(&ghost, Affine::translate((5.0, 5.0)));
        assert_eq!(p.clip_tiles(), 2);
        p.append(&ghost);
        assert_eq!(p.clip_tiles(), 4);
    }

    fn split(p: &mut VelloPainter, content: Rect) {
        let verdict = p.split_surface(&SurfaceSplit {
            surface_id: 1,
            content,
            node_transform: Affine::IDENTITY,
            inner: None,
            clip_to_content: false,
            image_transform: Affine::translate((content.x0, content.y0)),
            extents: (content.width() as u32, content.height() as u32),
            straight_alpha: false,
            ink: content,
            base: None,
        });
        assert_eq!(verdict, SplitVerdict::Split);
    }

    fn solid() -> Brush {
        Brush::Solid(peniko::Color::WHITE)
    }

    /// Opened for slicing with nothing to split, a frame counts as an unsliced
    /// one does: an elided clip stays out of the window's scene.
    #[test]
    fn a_sliced_frame_counts_only_the_clips_in_its_scene() {
        let mut p = VelloPainter::new();
        p.begin_slicing(WINDOW);
        p.set_tile_target(WINDOW);
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(8.0, 8.0, 408.0, 208.0),
        );
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &rect(0.0, 0.0, 800.0, 600.0),
        );
        p.pop_layer();
        p.pop_layer();
        assert!(p.take_slices().is_empty());
        assert_eq!(p.clip_tiles(), 26 * 13);
    }

    /// The inverse clip costs the target each time a draw across a surface's
    /// region opens it: it reaches past the window, and its hole saves nothing.
    #[test]
    fn the_inverse_clip_costs_the_target_each_time_it_opens() {
        let mut p = VelloPainter::new();
        p.begin_slicing(WINDOW);
        p.set_tile_target(WINDOW);
        split(&mut p, Rect::new(160.0, 160.0, 320.0, 320.0));
        let across = rect(100.0, 100.0, 200.0, 200.0);
        for _ in 0..2 {
            p.fill(Fill::NonZero, Affine::IDENTITY, &solid(), &across);
            // A push closes the inverse clip, so the next draw opens it again.
            p.push_clip(Fill::NonZero, Affine::IDENTITY, &rect(0.0, 0.0, 16.0, 16.0));
            p.pop_layer();
        }
        let slices = p.take_slices();
        assert_eq!(slices.len(), 1);
        // Two inverse clips, two clips in below, and the two pushes mirrored
        // into `above_in` (which keeps the draws): tile (0, 0) each.
        assert_eq!(p.clip_tiles(), 2 * 1900 + 2 + 2);
        // A partial target cuts the inverse clip.
        let mut p = VelloPainter::new();
        p.begin_slicing(WINDOW);
        p.set_tile_target(Rect::new(0.0, 0.0, 320.0, 320.0));
        split(&mut p, Rect::new(160.0, 160.0, 320.0, 320.0));
        p.fill(Fill::NonZero, Affine::IDENTITY, &solid(), &across);
        p.take_slices();
        assert_eq!(p.clip_tiles(), 20 * 20);
    }

    /// A slice's above scene nothing was drawn in is dropped, not rendered, so
    /// the pushes mirrored into it cost nothing; one with a draw pays, cut by
    /// the target it is rendered to: R's far corner, not the window.
    #[test]
    fn a_slice_pays_only_for_a_scene_it_keeps() {
        let region = Rect::new(160.0, 160.0, 320.0, 320.0);
        let big = rect(8.0, 8.0, 780.0, 580.0);
        let mut p = VelloPainter::new();
        p.begin_slicing(WINDOW);
        p.set_tile_target(WINDOW);
        split(&mut p, region);
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &big);
        p.pop_layer();
        let window_cost = tile_cost(Rect::new(8.0, 8.0, 780.0, 580.0), WINDOW);
        p.take_slices();
        assert_eq!(p.clip_tiles(), window_cost, "above_in has no draw");

        let mut p = VelloPainter::new();
        p.begin_slicing(WINDOW);
        p.set_tile_target(WINDOW);
        split(&mut p, region);
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &big);
        p.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &solid(),
            &rect(200.0, 200.0, 210.0, 210.0),
        );
        p.pop_layer();
        p.take_slices();
        assert_eq!(p.clip_tiles(), window_cost + 20 * 20);
    }

    /// What closes after the split starts `above_out` with what is still open
    /// replayed, an elided clip included: the slices render it as before.
    #[test]
    fn a_replayed_clip_pays_in_above_out() {
        let mut p = VelloPainter::new();
        p.begin_slicing(WINDOW);
        p.set_tile_target(WINDOW);
        let outer = rect(0.0, 0.0, 800.0, 600.0);
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &outer);
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &outer);
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &outer);
        assert_eq!(p.clip_tiles(), 1900, "the two inside it are elided");
        split(&mut p, Rect::new(160.0, 160.0, 320.0, 320.0));
        p.pop_layer();
        p.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &solid(),
            &rect(200.0, 200.0, 210.0, 210.0),
        );
        p.pop_layer();
        p.pop_layer();
        let slices = p.take_slices();
        assert!(slices[0].above_out.is_some());
        assert_eq!(
            p.clip_tiles(),
            1900 + 2 * 20 * 20,
            "two replayed, in 320x320"
        );
    }

    /// A clip that is not a whole-pixel rect makes the ancestors a mask scene,
    /// which is rendered too.
    #[test]
    fn a_mask_scene_pays_for_its_clips() {
        let mut p = VelloPainter::new();
        p.begin_slicing(WINDOW);
        p.set_tile_target(WINDOW);
        let ring = PaintShape::RoundedRect(peniko::kurbo::RoundedRect::new(
            150.0, 150.0, 330.0, 330.0, 8.0,
        ));
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &ring);
        let in_scene = 12 * 12;
        assert_eq!(p.clip_tiles(), in_scene, "tiles 9..21");
        split(&mut p, Rect::new(160.0, 160.0, 320.0, 320.0));
        p.pop_layer();
        p.take_slices();
        // In the mask, cut by R's far corner: tiles 9..20.
        assert_eq!(p.clip_tiles(), in_scene + 11 * 11);
    }

    /// A split the painter refuses leaves the surface in the scene, and adds
    /// no mask.
    #[test]
    fn a_refused_split_adds_nothing() {
        let mut p = VelloPainter::new();
        p.begin_slicing(WINDOW);
        p.set_tile_target(WINDOW);
        p.refuse_split(&[1]);
        let content = Rect::new(160.0, 160.0, 320.0, 320.0);
        let verdict = p.split_surface(&SurfaceSplit {
            surface_id: 1,
            content,
            node_transform: Affine::IDENTITY,
            inner: None,
            clip_to_content: false,
            image_transform: Affine::translate((160.0, 160.0)),
            extents: (160, 160),
            straight_alpha: false,
            ink: content,
            base: None,
        });
        assert_eq!(verdict, SplitVerdict::InScene(Refusal::MemoryCap));
        assert!(p.take_slices().is_empty());
        assert_eq!(p.clip_tiles(), 0);
    }
}

#[cfg(test)]
mod flattened_tests {
    //! #2321: what `flattened` hands Vello in place of a curve.
    use super::*;
    use peniko::kurbo::{BezPath, Circle, Line, PathEl, Point, RoundedRect, Shape};

    /// The vertices of a flattened path (it holds no curves).
    fn vertices(p: &BezPath) -> Vec<Point> {
        p.elements()
            .iter()
            .filter_map(|e| match e {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
                PathEl::ClosePath => None,
                e => panic!("a curve survived: {e:?}"),
            })
            .collect()
    }

    /// The most any point of a chord between two vertices in `corner`
    /// (consecutive in `pts`) strays from the circle of radius `r` about `c`,
    /// either way.
    fn worst_deviation(pts: &[Point], corner: impl Fn(Point) -> bool, c: Point, r: f64) -> f64 {
        pts.windows(2)
            .filter(|w| corner(w[0]) && corner(w[1]))
            .flat_map(|w| (0..=32).map(move |i| w[0].lerp(w[1], i as f64 / 32.0)))
            .map(|p| ((p - c).hypot() - r).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_rounded_corner_is_within_the_tolerance_in_device_px() {
        // A radius-8 corner under scale 1, 2 and 3 (and off the origin): no
        // point of the polygon strays more than the tolerance from the arc, in
        // device px, and the arc starts and ends on the straight edges.
        for s in [1.0, 2.0, 3.0] {
            let rr = RoundedRect::new(11.0, 7.0, 51.0, 37.0, 8.0);
            let p = flattened(Affine::scale(s), &PaintShape::RoundedRect(rr)).unwrap();
            let in_corner = |v: Point| v.x <= 19.0 && v.y <= 15.0;
            let all = vertices(&p);
            let corner = all.iter().filter(|v| in_corner(**v)).count();
            assert!(all.contains(&Point::new(11.0, 15.0)) && all.contains(&Point::new(19.0, 7.0)));
            let c = Point::new(19.0, 15.0);
            let dev = worst_deviation(&all, in_corner, c, 8.0) * s;
            assert!(dev <= CURVE_TOLERANCE * 1.001, "{s}x: {dev} device px off");
            // And not far finer than asked (each chord costs encoding).
            assert!(dev >= CURVE_TOLERANCE * 0.4, "{s}x: {dev} (over-flattened)");
            assert!(corner >= 4, "{s}x: {corner} vertices");
        }
    }

    #[test]
    fn a_curved_path_is_within_the_tolerance() {
        // A circle as a path of cubics (kurbo's), flattened by kurbo.
        let circle = Circle::new((20.0, 20.0), 8.0);
        let p = PaintShape::BezPath(circle.to_path(1e-6));
        let all = vertices(&flattened(Affine::scale(2.0), &p).unwrap());
        let dev = worst_deviation(&all, |_| true, circle.center, 8.0) * 2.0;
        assert!(dev <= CURVE_TOLERANCE * 1.01, "{dev}");
    }

    #[test]
    fn a_rotated_or_stretched_transform_divides_by_its_largest_stretch() {
        // scale(1, 4) stretches y fourfold; a 45deg rotation stretches by 1.
        let c = Circle::new((0.0, 0.0), 5.0);
        let n = |t: Affine| vertices(&flattened(t, &PaintShape::Circle(c)).unwrap()).len();
        assert_eq!(
            n(Affine::rotate(std::f64::consts::FRAC_PI_4)),
            n(Affine::IDENTITY)
        );
        assert_eq!(
            n(Affine::scale_non_uniform(1.0, 4.0)),
            n(Affine::scale(4.0))
        );
        assert!(n(Affine::scale(4.0)) > n(Affine::IDENTITY));
    }

    #[test]
    fn shapes_with_no_curves_are_left_alone() {
        let t = Affine::scale(2.0);
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(flattened(t, &PaintShape::Rect(r)).is_none());
        assert!(flattened(t, &PaintShape::Line(Line::new((0.0, 0.0), (5.0, 5.0)))).is_none());
        assert!(flattened(t, &PaintShape::RoundedRect(RoundedRect::from_rect(r, 0.0))).is_none());
        assert!(flattened(t, &PaintShape::BezPath(r.to_path(0.1))).is_none());
        // A collapsed transform draws nothing either way.
        let curved = PaintShape::RoundedRect(RoundedRect::from_rect(r, 3.0));
        assert!(flattened(Affine::scale(0.0), &curved).is_none());
        assert!(flattened(t, &curved).is_some());
    }

    #[test]
    fn a_path_keeps_its_subpaths_and_its_lines() {
        // Two subpaths, one a curve and one already lines: both survive, the
        // line's points exactly.
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.quad_to((10.0, 0.0), (10.0, 10.0));
        p.close_path();
        p.move_to((20.0, 0.0));
        p.line_to((30.0, 0.0));
        p.line_to((30.0, 7.0));
        p.close_path();
        let f = flattened(Affine::IDENTITY, &PaintShape::BezPath(p.clone())).unwrap();
        let moves = f
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::MoveTo(_)))
            .count();
        assert_eq!(moves, 2);
        let v = vertices(&f);
        assert!(v.ends_with(&[
            Point::new(20.0, 0.0),
            Point::new(30.0, 0.0),
            Point::new(30.0, 7.0)
        ]));
        assert!(
            (f.area() - p.area()).abs() < 0.5,
            "{} vs {}",
            f.area(),
            p.area()
        );
    }

    /// The signed area of a polygon given by its vertices (y down: positive
    /// is clockwise on screen).
    fn signed_area(v: &[Point]) -> f64 {
        v.iter()
            .zip(v.iter().cycle().skip(1))
            .map(|(a, b)| a.x * b.y - b.x * a.y)
            .sum::<f64>()
            / 2.0
    }

    fn move_tos(p: &BezPath) -> Vec<Point> {
        p.elements()
            .iter()
            .filter_map(|e| match e {
                PathEl::MoveTo(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_rounded_rect_runs_as_kurbos_does() {
        // Same start, same direction, same corner order: a dash pattern
        // stroked along either has the same phase (review of #2380: starting
        // at the top edge moved every dash by a quarter arc).
        let rr = RoundedRect::new(11.0, 7.0, 51.0, 37.0, (7.0, 3.0, 11.0, 6.0));
        let ours = flattened(Affine::IDENTITY, &PaintShape::RoundedRect(rr)).unwrap();
        let kurbo = rr.to_path(1e-6);
        let ends = |p: &BezPath| -> Vec<Point> {
            p.elements().iter().filter_map(|e| e.end_point()).collect()
        };
        let (ov, kv) = (ends(&ours), ends(&kurbo));
        assert_eq!(ov[0], kv[0], "first point");
        assert_eq!(ov[0], Point::new(11.0, 14.0));
        // Every point of kurbo's path on the rect's edges (where its arcs
        // start and end) is one of ours, in order.
        let on_edge = |p: &&Point| p.x == 11.0 || p.x == 51.0 || p.y == 7.0 || p.y == 37.0;
        let mut it = ov.iter();
        for k in kv.iter().filter(on_edge) {
            assert!(it.any(|o| (*o - *k).hypot() < 1e-9), "{k:?} out of order");
        }
        assert!(signed_area(&vertices(&ours)) > 0.0);
        // And the dashes land where kurbo's do, to within the tolerance.
        let dashes = |p: &BezPath| {
            move_tos(&peniko::kurbo::dash(p.elements().iter().copied(), 0.0, &[3.0, 2.0]).collect())
        };
        let (od, kd) = (dashes(&ours), dashes(&kurbo));
        assert_eq!(od.len(), kd.len());
        for (o, k) in od.iter().zip(&kd) {
            assert!((*o - *k).hypot() < 0.1, "dash at {o:?}, kurbo's at {k:?}");
        }
    }

    #[test]
    fn a_flattened_path_keeps_its_start_and_winding() {
        // A path of general cubics and a quad (Wang's steps), and the same
        // reversed: the first point, every subpath's start and the sign of
        // the area are kept (#2371's dashed borders root their own paths).
        let mut p = BezPath::new();
        p.move_to((3.0, 20.0));
        p.curve_to((3.0, 4.0), (30.0, 9.0), (40.0, 2.0));
        p.quad_to((60.0, 20.0), (40.0, 30.0));
        p.curve_to((20.0, 40.0), (10.0, 35.0), (3.0, 20.0));
        p.close_path();
        for path in [p.clone(), p.reverse_subpaths()] {
            let f = flattened(Affine::scale(2.0), &PaintShape::BezPath(path.clone())).unwrap();
            assert_eq!(f.elements()[0], path.elements()[0]);
            assert_eq!(move_tos(&f), move_tos(&path));
            // Chords inside a convex curve lose at most the tolerance (in
            // local px: half the device tolerance at scale 2) along its length.
            let (a, b) = (signed_area(&vertices(&f)), path.area());
            let most = CURVE_TOLERANCE / 2.0 * path.perimeter(1e-6);
            assert!(
                a.signum() == b.signum() && (a - b).abs() < most,
                "{a} vs {b} (at most {most} apart)"
            );
        }
        // Wang's steps are within the tolerance of the curve.
        let c = peniko::kurbo::CubicBez::new((3.0, 20.0), (3.0, 4.0), (30.0, 9.0), (40.0, 2.0));
        let mut one = BezPath::new();
        one.move_to(c.p0);
        one.curve_to(c.p1, c.p2, c.p3);
        let f = vertices(&flattened(Affine::IDENTITY, &PaintShape::BezPath(one)).unwrap());
        use peniko::kurbo::{ParamCurve, ParamCurveNearest};
        for i in 0..=200 {
            let q = c.eval(i as f64 / 200.0);
            let d = f
                .windows(2)
                .map(|w| Line::new(w[0], w[1]).nearest(q, 1e-9).distance_sq.sqrt())
                .fold(f64::INFINITY, f64::min);
            assert!(
                d <= CURVE_TOLERANCE * 1.001,
                "{d} at t = {}",
                i as f64 / 200.0
            );
        }
    }

    #[test]
    fn a_rings_arcs_are_drawn_as_circular_polygons() {
        // A solid rounded border's ring: kurbo's rounded rect with the
        // inner one wound against it. Both outlines' arcs are recognised as
        // circular (vertices a good part of the tolerance outside the
        // circle, as `ArcPolygon` puts them; kurbo's cubic is only 0.03% of
        // the radius out, which is where Wang's steps would put them),
        // within the tolerance, and the ring keeps both subpaths' starts and
        // their directions.
        let outer = RoundedRect::new(10.0, 10.0, 60.0, 40.0, 6.0).to_path(0.1);
        let inner = RoundedRect::new(11.0, 11.0, 59.0, 39.0, 5.0).to_path(0.1);
        let mut ring = outer.clone();
        ring.extend(inner.reverse_subpaths());
        let f = flattened(Affine::IDENTITY, &PaintShape::BezPath(ring.clone())).unwrap();
        assert_eq!(move_tos(&f), move_tos(&ring));
        assert!(
            (f.area() - ring.area()).abs() < 0.2,
            "{} vs {}",
            f.area(),
            ring.area()
        );
        let c = Point::new(16.0, 16.0);
        let v = vertices(&f);
        let outside = v
            .iter()
            .filter(|v| v.x < 16.0 && v.y < 16.0 && (**v - c).hypot() > 6.0 + CURVE_TOLERANCE / 4.0)
            .count();
        assert!(outside > 0, "not drawn as an ArcPolygon");
        let corner = |v: Point| v.x <= 16.0 && v.y <= 16.0 && (v - c).hypot() > 5.5;
        let dev = worst_deviation(&v, corner, c, 6.0);
        assert!(dev <= CURVE_TOLERANCE * 1.01, "{dev}");
    }

    /// The most any vertex or chord middle of `poly` is from the curve `c`
    /// (measured to a fine polyline of it).
    fn off_curve(poly: &[Point], c: peniko::kurbo::CubicBez) -> f64 {
        use peniko::kurbo::{ParamCurve, ParamCurveNearest};
        let fine: Vec<Point> = (0..=4000).map(|i| c.eval(i as f64 / 4000.0)).collect();
        let dist = |q: Point| {
            fine.windows(2)
                .map(|w| Line::new(w[0], w[1]).nearest(q, 1e-12).distance_sq)
                .fold(f64::INFINITY, f64::min)
                .sqrt()
        };
        poly.iter()
            .copied()
            .chain(poly.windows(2).map(|w| w[0].midpoint(w[1])))
            .map(dist)
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_cubic_past_a_quarter_turn_follows_the_cubic() {
        // A single cubic built as the circle's own for 100, 180 and 200
        // degrees (both ways round, at several rotations and radii) is not
        // taken for an arc: past a quarter turn one cubic is no circle, and
        // past a half turn the ends' angle names the other way round (review
        // of #2380: 200 degrees was drawn as its 160-degree complement).
        // Its lines stay within the tolerance of the cubic itself.
        for deg in [100.0f64, 180.0, 200.0] {
            for turn in [1.0, -1.0] {
                for rot in (0..12).map(|i| i as f64 * 30.0 + 10.0) {
                    for r in [5.0, 20.0, 100.0] {
                        let sweep = deg.to_radians() * turn;
                        let (a0, a1) = (rot.to_radians(), rot.to_radians() + sweep);
                        let k = 4.0 / 3.0 * (sweep / 4.0).tan() * r;
                        let c = Point::new(50.0, 50.0);
                        let at = |a: f64| c + Vec2::new(a.cos(), a.sin()) * r;
                        let tan = |a: f64| Vec2::new(-a.sin(), a.cos());
                        let cubic = peniko::kurbo::CubicBez::new(
                            at(a0),
                            at(a0) + tan(a0) * k,
                            at(a1) - tan(a1) * k,
                            at(a1),
                        );
                        assert!(
                            circular_arc(cubic.p0, cubic.p1, cubic.p2, cubic.p3, CURVE_TOLERANCE)
                                .is_none(),
                            "{deg} deg ({turn}) at {rot}, r {r}: taken for an arc"
                        );
                        let mut p = BezPath::new();
                        p.move_to(cubic.p0);
                        p.curve_to(cubic.p1, cubic.p2, cubic.p3);
                        let v = vertices(
                            &flattened(Affine::IDENTITY, &PaintShape::BezPath(p)).unwrap(),
                        );
                        let off = off_curve(&v, cubic);
                        assert!(
                            off <= CURVE_TOLERANCE * 1.05,
                            "{deg} deg ({turn}) at {rot}, r {r}: {off} px off the cubic"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_large_near_arc_follows_the_cubic() {
        // An SVG quarter arc whose arms are 0.2% long, at radii 300 and
        // 1000: within a thousandth of the radius of a circle's, but drawn
        // as the circle it would be 0.13-0.45 px off the cubic. It goes
        // through Wang's steps instead. kurbo's own quarter arc, at those
        // radii and both ways round, is still recognised.
        for r in [300.0, 1000.0] {
            let k = 4.0 / 3.0 * std::f64::consts::FRAC_PI_8.tan() * r;
            let (p0, p3) = (Point::new(0.0, r), Point::new(r, 0.0));
            let exact = (p0, p0 + Vec2::new(0.0, -k), p3 + Vec2::new(-k, 0.0), p3);
            assert!(circular_arc(exact.0, exact.1, exact.2, exact.3, CURVE_TOLERANCE).is_some());
            assert!(circular_arc(exact.3, exact.2, exact.1, exact.0, CURVE_TOLERANCE).is_some());
            let k = k * 1.002;
            let near = peniko::kurbo::CubicBez::new(
                p0,
                p0 + Vec2::new(0.0, -k),
                p3 + Vec2::new(-k, 0.0),
                p3,
            );
            assert!(circular_arc(near.p0, near.p1, near.p2, near.p3, CURVE_TOLERANCE).is_none());
            let mut p = BezPath::new();
            p.move_to(near.p0);
            p.curve_to(near.p1, near.p2, near.p3);
            let v = vertices(&flattened(Affine::IDENTITY, &PaintShape::BezPath(p)).unwrap());
            let off = off_curve(&v, near);
            assert!(
                off <= CURVE_TOLERANCE * 1.05,
                "r {r}: {off} px off the cubic"
            );
        }
        // And a rounded rect's kurbo path (both of a ring's windings) still
        // is, at any radius: see `a_rings_arcs_are_drawn_as_circular_polygons`.
        let big = RoundedRect::new(0.0, 0.0, 2000.0, 2000.0, 900.0).to_path(0.1);
        let PathEl::CurveTo(p1, p2, p3) = big.elements()[1] else {
            panic!("{:?}", big.elements()[1]);
        };
        let PathEl::MoveTo(p0) = big.elements()[0] else {
            panic!()
        };
        assert!(circular_arc(p0, p1, p2, p3, CURVE_TOLERANCE).is_some());
    }

    #[test]
    fn a_curve_after_a_close_starts_where_the_subpath_did() {
        // `ClosePath` returns to the subpath's start; a curve after it with
        // no `MoveTo` starts there (not at the last point before the close).
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((30.0, 0.0));
        p.line_to((30.0, 20.0));
        p.close_path();
        p.quad_to((40.0, 40.0), (10.0, 30.0));
        let v = vertices(&flattened(Affine::IDENTITY, &PaintShape::BezPath(p)).unwrap());
        let quad = peniko::kurbo::QuadBez::new((0.0, 0.0), (40.0, 40.0), (10.0, 30.0)).raise();
        // The vertices after the close (the quad's), with its start.
        let after: Vec<Point> = std::iter::once(Point::ZERO)
            .chain(v[3..].iter().copied())
            .collect();
        let off = off_curve(&after, quad);
        assert!(off <= CURVE_TOLERANCE * 1.05, "{off} px off the quad");
    }
}
