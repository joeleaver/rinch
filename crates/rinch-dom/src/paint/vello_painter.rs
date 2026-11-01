//! [`Painter`] implementation backed by `vello::Scene` for GPU rendering.

use peniko::kurbo::Affine;
use peniko::{Blob, Brush, Fill, FontData, ImageAlphaType, ImageData, ImageFormat};
use vello::Scene;

use super::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
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
    /// See [`Self::clip_tiles`].
    clip_tiles: u64,
}

/// One clip or layer [`VelloPainter`] has open.
#[derive(Clone, Copy)]
struct OpenLayer {
    /// Its pop has something to pop; `false` for a clip elided as cutting nothing.
    pushed: bool,
    /// Device bounds drawing is confined to while this is open.
    region: Rect,
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
        Self {
            scene: Scene::new(),
            slicing: None,
            node_starts: Default::default(),
            reads_backdrop: false,
            routing: false,
            open: Vec::new(),
            tile_target: EVERYWHERE,
            clip_tiles: 0,
        }
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
    fn opened(&mut self, transform: Affine, shape: &PaintShape) {
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
            exact,
        });
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
        glyph_transform: Option<Affine>,
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
                glyph_transform,
                brush,
                hint,
                normalized_coords,
                glyphs,
            )
        });
    }
}

/// Encode one glyph run into `scene`.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn encode_glyphs(
    scene: &mut Scene,
    font: &FontData,
    font_size: f32,
    transform: Affine,
    glyph_transform: Option<Affine>,
    brush: &Brush,
    hint: bool,
    normalized_coords: &[i16],
    glyphs: &[PaintGlyph],
) {
    scene
        .draw_glyphs(font)
        .font_size(font_size)
        .transform(transform)
        .glyph_transform(glyph_transform)
        .brush(brush)
        .hint(hint)
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

/// Dispatch a fill operation on a `PaintShape` to the appropriate Scene method.
macro_rules! dispatch_fill {
    ($scene:expr, $fill:expr, $transform:expr, $brush:expr, $shape:expr) => {
        dispatch_fill!($scene, $fill, $transform, $brush, None, $shape)
    };
    ($scene:expr, $fill:expr, $transform:expr, $brush:expr, $bt:expr, $shape:expr) => {
        match $shape {
            PaintShape::Rect(r) => $scene.fill($fill, $transform, $brush, $bt, r),
            PaintShape::RoundedRect(r) => $scene.fill($fill, $transform, $brush, $bt, r),
            PaintShape::BezPath(p) => $scene.fill($fill, $transform, $brush, $bt, p),
            PaintShape::Circle(c) => $scene.fill($fill, $transform, $brush, $bt, c),
            PaintShape::Line(l) => $scene.fill($fill, $transform, $brush, $bt, l),
        }
    };
}

/// Dispatch a stroke operation on a `PaintShape` to the appropriate Scene method.
macro_rules! dispatch_stroke {
    ($scene:expr, $stroke:expr, $transform:expr, $brush:expr, $shape:expr) => {
        match $shape {
            PaintShape::Rect(r) => $scene.stroke($stroke, $transform, $brush, None, r),
            PaintShape::RoundedRect(r) => $scene.stroke($stroke, $transform, $brush, None, r),
            PaintShape::BezPath(p) => $scene.stroke($stroke, $transform, $brush, None, p),
            PaintShape::Circle(c) => $scene.stroke($stroke, $transform, $brush, None, c),
            PaintShape::Line(l) => $scene.stroke($stroke, $transform, $brush, None, l),
        }
    };
}

/// Dispatch a clip operation on a `PaintShape` to the appropriate Scene method.
macro_rules! dispatch_clip {
    ($scene:expr, $fill:expr, $transform:expr, $shape:expr) => {
        match $shape {
            PaintShape::Rect(r) => $scene.push_clip_layer($fill, $transform, r),
            PaintShape::RoundedRect(r) => $scene.push_clip_layer($fill, $transform, r),
            PaintShape::BezPath(p) => $scene.push_clip_layer($fill, $transform, p),
            PaintShape::Circle(c) => $scene.push_clip_layer($fill, $transform, c),
            PaintShape::Line(l) => $scene.push_clip_layer($fill, $transform, l),
        }
    };
}

/// Dispatch a push_layer operation on a `PaintShape`.
macro_rules! dispatch_push_layer {
    ($scene:expr, $fill:expr, $mix:expr, $opacity:expr, $transform:expr, $shape:expr) => {
        match $shape {
            PaintShape::Rect(r) => $scene.push_layer($fill, $mix, $opacity, $transform, r),
            PaintShape::RoundedRect(r) => $scene.push_layer($fill, $mix, $opacity, $transform, r),
            PaintShape::BezPath(p) => $scene.push_layer($fill, $mix, $opacity, $transform, p),
            PaintShape::Circle(c) => $scene.push_layer($fill, $mix, $opacity, $transform, c),
            PaintShape::Line(l) => $scene.push_layer($fill, $mix, $opacity, $transform, l),
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
    };
    dispatch_push_layer!(scene, Fill::NonZero, mix, opacity, transform, bounds);
}

/// The device bounds of `shape` under `transform`.
pub(crate) fn shape_bounds(transform: Affine, shape: &PaintShape) -> Rect {
    transform.transform_rect_bbox(shape.control_bounds())
}

impl Painter for VelloPainter {
    fn reset(&mut self) {
        self.scene.reset();
        self.open.clear();
        self.tile_target = EVERYWHERE;
        self.clip_tiles = 0;
        self.slicing = None;
        self.routing = false;
        self.reads_backdrop = false;
    }

    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape) {
        // Vello's fine shader evaluates a gradient at each pixel's top-left
        // corner (it sampled images there too before Vello 0.9, #1027),
        // where CSS and tiny-skia evaluate it at the centre: a 1px hard-stop line of a repeating
        // gradient fell between two corners and was not drawn (#1798).
        let bt = matches!(brush, Brush::Gradient(_))
            .then(|| centre_sampling(transform, Affine::IDENTITY));
        route!(
            self,
            || Some(shape_bounds(transform, shape)),
            |scene| dispatch_fill!(scene, fill, transform, brush, bt, shape)
        );
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
        glyph_transform: Option<Affine>,
        brush: &Brush,
        hint: bool,
        normalized_coords: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        // The hottest draw: with nothing split it is exactly the encode, and
        // the routed draw is a call out of line (#1899 review: through
        // `route!` the glyph path still cost 8.1k instructions a text page).
        if self.routing {
            return self.draw_glyphs_routed(
                font,
                font_size,
                transform,
                glyph_transform,
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
            glyph_transform,
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
        let brush = peniko::ImageBrush::new(image_data);
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
                    exact,
                });
                return;
            }
            self.opened(transform, shape);
            return push_clip_into(&mut self.scene, fill, transform, shape);
        }
        if elide {
            self.open.push(OpenLayer {
                pushed: true,
                region,
                exact,
            });
        } else {
            self.opened(transform, shape);
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
        if matches!(blend, BlendMode::Saturation) {
            self.reads_backdrop = true;
        }
        self.opened(transform, bounds);
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
        self.opened(transform, bounds);
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
        if !self.open.pop().is_none_or(|o| o.pushed) {
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
