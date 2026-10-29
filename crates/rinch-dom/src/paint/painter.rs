//! Abstract painting trait for backend-agnostic rendering.
//!
//! The `Painter` trait defines the drawing operations needed to render a rinch
//! DOM tree. Two implementations exist:
//!
//! - `VelloPainter` (feature `gpu`) — wraps `vello::Scene` for GPU rendering
//! - `TinySkiaPainter` (default) — software rasterization via `tiny-skia`
//!
//! The trait uses peniko/kurbo types directly for geometry, colors, and brushes.
//! These are lightweight crates (~zero runtime cost) shared with Parley for text
//! layout. The abstraction is at the *rendering* level (how shapes become pixels),
//! not the geometry level.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Circle, Line, Rect, RoundedRect, Stroke};
use peniko::{Brush, Fill, FontData};

// ── Shape abstraction ───────────────────────────────────────────────────────

/// A shape that can be filled or stroked by a `Painter`.
///
/// This avoids the need for `dyn kurbo::Shape` (which isn't object-safe for
/// all uses) by enumerating the shapes actually used in the paint code.
#[derive(Clone, Debug)]
pub enum PaintShape {
    Rect(Rect),
    RoundedRect(RoundedRect),
    BezPath(BezPath),
    Circle(Circle),
    Line(Line),
}

impl PaintShape {
    /// The smallest axis-aligned rect containing the shape, in its own space.
    pub fn bounding_box(&self) -> Rect {
        use peniko::kurbo::Shape;
        match self {
            PaintShape::Rect(r) => r.bounding_box(),
            PaintShape::RoundedRect(r) => r.bounding_box(),
            PaintShape::BezPath(p) => p.bounding_box(),
            PaintShape::Circle(c) => c.bounding_box(),
            PaintShape::Line(l) => l.bounding_box(),
        }
    }

    /// A rect containing the shape, cheaply: a path's control box (its
    /// points, control points included) rather than its tight bounds, which
    /// solve each curve's extrema (#1894 review: 6M instructions a paint
    /// routing ~70 boxes over a split surface). Still conservative.
    pub fn control_bounds(&self) -> Rect {
        match self {
            PaintShape::BezPath(p) => p.control_box(),
            _ => self.bounding_box(),
        }
    }
}

impl From<Rect> for PaintShape {
    fn from(r: Rect) -> Self {
        Self::Rect(r)
    }
}

impl From<RoundedRect> for PaintShape {
    fn from(r: RoundedRect) -> Self {
        Self::RoundedRect(r)
    }
}

impl From<BezPath> for PaintShape {
    fn from(p: BezPath) -> Self {
        Self::BezPath(p)
    }
}

impl From<Circle> for PaintShape {
    fn from(c: Circle) -> Self {
        Self::Circle(c)
    }
}

impl From<Line> for PaintShape {
    fn from(l: Line) -> Self {
        Self::Line(l)
    }
}

impl From<&Rect> for PaintShape {
    fn from(r: &Rect) -> Self {
        Self::Rect(*r)
    }
}

impl From<&RoundedRect> for PaintShape {
    fn from(r: &RoundedRect) -> Self {
        Self::RoundedRect(*r)
    }
}

impl From<&BezPath> for PaintShape {
    fn from(p: &BezPath) -> Self {
        Self::BezPath(p.clone())
    }
}

impl From<&Circle> for PaintShape {
    fn from(c: &Circle) -> Self {
        Self::Circle(*c)
    }
}

impl From<&Line> for PaintShape {
    fn from(l: &Line) -> Self {
        Self::Line(*l)
    }
}

// ── Blend mode ──────────────────────────────────────────────────────────────

/// Blend mode for layer compositing.
///
/// Only the modes actually used in rinch paint code.
#[derive(Clone, Copy, Debug, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    /// Used for CSS `filter: grayscale(...)` approximation.
    Saturation,
    /// Composite by **adding** the layer to what is beneath it (Porter-Duff
    /// `plus`, saturating), instead of covering it. Inside an isolated layer
    /// ([`Painter::push_isolated_layer`]), a sum of `Plus` layers at weights
    /// that total 1 is a weighted average of their contents — which is how a
    /// blurred `text-shadow` is drawn as a Gaussian kernel of copies on both
    /// backends (#980).
    Plus,
    /// Keep what is beneath the layer only where the layer is opaque, scaled
    /// by its alpha (Porter-Duff `destination-in`): the layer is a mask over
    /// what was drawn before it. `mask-image` (#1798) draws its mask into
    /// such a layer inside the layer holding the element. Never elided at
    /// opacity 1.
    DestinationIn,
    /// Erase what is beneath the layer where the layer is opaque, scaled by
    /// its alpha (Porter-Duff `destination-out`): the layer is an eraser. A
    /// `GameViewport` clears its box to transparent this way when paint
    /// reaches it, so the compositor's layer under the UI shows there and
    /// nothing painted before it does (#1866). Never elided at opacity 1.
    DestinationOut,
}

// ── Image data ──────────────────────────────────────────────────────────────

/// Decoded image data ready for painting.
///
/// Always RGBA8 with straight (non-premultiplied) alpha.
pub struct PaintImage<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// The cache entry `data` came from, when it came from one. A backend
    /// that needs the pixels in another form (the software painter wants them
    /// premultiplied) can keep that form on the entry instead of converting
    /// on every draw. `None` for a live frame source.
    pub decoded: Option<&'a crate::image_cache::DecodedImage>,
    /// The shared buffer `data` lives in, for a live frame source (a CPU
    /// `RenderSurface` frame, a video frame). A painter that keeps images
    /// (Vello) shares the buffer and keeps one identity per buffer, so a
    /// frame repainted unchanged is not uploaded again (#1955). `None` for a
    /// decoded image (it has `decoded`) and for pixels made for one draw.
    pub frame: Option<&'a std::sync::Arc<Vec<u8>>>,
    /// Every pixel's alpha is 255, as the producer promises (a live frame
    /// source checks at submit, on its own thread). Opaque pixels are their
    /// own premultiplied form, so the software painter skips premultiplying
    /// them, and may copy them straight into the surface when the draw has
    /// no rotation or skew, a positive scale, destination edges on whole pixels, and a clip that is fully on or fully off wherever the frame lands (#361). `false` promises nothing.
    pub opaque: bool,
}

/// An RGBA8 image, premultiplied or straight ([`Self::premultiplied`]), tiled
/// edge to edge by [`Painter::fill_repeating`] (#468). Only the `embed` build
/// hands a straight-alpha tile (a decoded `url()` image, #1158), and only to
/// Vello; the straight-alpha branches of the other painters are reached by no
/// caller today.
#[derive(Clone, Copy, Debug)]
pub struct RepeatTile<'a> {
    /// `width * height * 4` bytes of RGBA8, rows top to bottom, premultiplied
    /// or straight as [`Self::premultiplied`] says. A `Blob` so a painter that
    /// keeps images (Vello) shares it, not copies.
    pub pixels: &'a peniko::Blob<u8>,
    /// Whether [`Self::pixels`] are premultiplied (a rasterised tile) or
    /// straight (a decoded image handed over as it is, #1158).
    pub premultiplied: bool,
    pub width: u32,
    pub height: u32,
    /// Maps the tile's pixel grid (tile `(0, 0)` covering `0..width`,
    /// `0..height`) into the shape's own space, before the fill's transform.
    pub transform: Affine,
    /// Sample bilinearly (a tile scaled to a fractional size, or under a
    /// transform); otherwise nearest, which is exact for a whole-pixel tile
    /// on the device grid.
    pub smooth: bool,
}

// ── Glyph run ───────────────────────────────────────────────────────────────

/// A single positioned glyph for text rendering.
#[derive(Clone, Copy, Debug)]
pub struct PaintGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

// ── Split surfaces (#1865) ──────────────────────────────────────────────────

/// A GPU surface's frame at its place in paint order, offered to the painter
/// to draw outside the scene (`docs/design/split-layer-gpu-surfaces.md`). All
/// geometry is in device pixels, as paint draws it.
#[derive(Clone, Debug)]
pub struct SurfaceSplit {
    /// The surface's id (`data-render-surface`).
    pub surface_id: usize,
    /// The element's content box, before `node_transform`.
    pub content: Rect,
    /// The element's transform.
    pub node_transform: Affine,
    /// The element's own inner-radius clip, when its corners are rounded.
    pub inner: Option<PaintShape>,
    /// Whether the frame is clipped to `content` (`object-fit` overflowing).
    pub clip_to_content: bool,
    /// The image-to-device transform the in-scene draw would use
    /// (`fit_image`), and the image's size in texels.
    pub image_transform: Affine,
    pub extents: (u32, u32),
    /// Whether the frame's texels are straight alpha (a CPU frame, drawn as
    /// `draw_image` draws one) rather than premultiplied (a registered GPU
    /// texture, #1768).
    pub straight_alpha: bool,
    /// The element's ink in device pixels (its border box grown by its
    /// shadows and outline, transformed): where the paint cull reaches it,
    /// so where damage makes the frame paint it, and split it, again.
    pub ink: Rect,
    /// Set for a `GameViewport`, the **base** layer (#1866): inside `clear`
    /// the game replaces what was painted before it, instead of going over
    /// it (`docs/design/split-layer-gpu-surfaces.md`, *How `GameViewport`
    /// fits on the split path*). `None` for a surface or a video.
    pub base: Option<BaseSplit>,
}

/// What makes a [`SurfaceSplit`] a base layer (#1866).
#[derive(Clone, Debug)]
pub struct BaseSplit {
    /// The viewport's box under its own `border-radius`, in the element's
    /// space (drawn with `node_transform`): where nothing painted beneath it
    /// shows. The image (`image_transform`, `extents`) may cover less of it,
    /// letterboxed.
    pub clear: PaintShape,
    /// Opaque black under the frame inside `clear`, so a letterbox is black
    /// (#354): a CPU frame's. A GPU texture is stretched over the box.
    pub black_backdrop: bool,
}

/// What a painter did with a [`SurfaceSplit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitVerdict {
    /// The painter took the frame out of its scene: the compositor draws it.
    Split,
    /// The caller draws the frame in the scene, as it would have anyway.
    InScene(Refusal),
}

/// Why a surface stays in the scene (#1865, *Fallback to in-scene*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// This painter, or this frame, does not split (software, embed, or a
    /// frame the runtime did not open for slicing).
    Unsupported,
    /// A group effect is open around the surface: opacity below 1, a blend
    /// other than normal, a mask or another isolated layer (D3a).
    GroupEffect,
    /// The element's or the image's transform rotates or skews (D4a).
    Transform,
    /// Its region overlaps an earlier split surface's (D7a).
    Overlap,
    /// The window's split memory would pass its cap with this surface (D6):
    /// the runtime names the surfaces that do not fit
    /// ([`super::vello_painter::VelloPainter::refuse_split`]).
    MemoryCap,
}

// ── The Painter trait ───────────────────────────────────────────────────────

/// Abstract 2D rendering backend.
///
/// Implementations accumulate drawing commands (Vello) or rasterize immediately
/// (tiny-skia). The call pattern mirrors CSS painting order: backgrounds, then
/// borders, then text/content, with clip and opacity layers pushed/popped around
/// groups of operations.
pub trait Painter {
    /// Clear all accumulated drawing state, preparing for a new frame.
    fn reset(&mut self);

    /// Fill a shape with the given brush (solid color or gradient).
    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape);

    /// Fill a shape with a solid color (convenience — avoids Brush allocation).
    fn fill_color(
        &mut self,
        fill: Fill,
        transform: Affine,
        color: AlphaColor<Srgb>,
        shape: &PaintShape,
    ) {
        self.fill(fill, transform, &Brush::Solid(color), shape);
    }

    /// Stroke a shape with the given brush.
    fn stroke(&mut self, stroke: &Stroke, transform: Affine, brush: &Brush, shape: &PaintShape);

    /// Stroke a shape with a solid color (convenience).
    fn stroke_color(
        &mut self,
        stroke: &Stroke,
        transform: Affine,
        color: AlphaColor<Srgb>,
        shape: &PaintShape,
    ) {
        self.stroke(stroke, transform, &Brush::Solid(color), shape);
    }

    /// Draw a run of pre-shaped glyphs.
    ///
    /// The font, size, and brush come from Parley's glyph run styling.
    /// `normalized_coords` are font variation settings.
    #[allow(clippy::too_many_arguments)]
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
    );

    /// Draw an RGBA8 image with the given transform.
    fn draw_image(&mut self, image: &PaintImage<'_>, transform: Affine);

    /// Draw an image whose pixels live in a GPU texture registered with the
    /// backend's renderer (a `RenderSurface` GPU frame, #1768), placed like
    /// [`draw_image`](Self::draw_image): `transform` maps its `width` x `height`
    /// pixel space into the scene. The default draws nothing — only a backend
    /// that renders with the renderer the texture is registered with can draw
    /// it, and no other is ever handed one.
    fn draw_gpu_image(&mut self, image: &peniko::ImageData, transform: Affine) {
        let _ = (image, transform);
    }

    /// Fill `color` through a `width` x `height` coverage mask (one byte per
    /// pixel, 255 = fully covered), placed by `transform` like an image. A
    /// blurred `text-shadow` is drawn this way (#980).
    ///
    /// The default builds a straight-alpha image and calls
    /// [`draw_image`](Self::draw_image); a backend can do it without the
    /// intermediate.
    fn draw_alpha_mask(
        &mut self,
        mask: &[u8],
        width: u32,
        height: u32,
        color: AlphaColor<Srgb>,
        transform: Affine,
    ) {
        let [r, g, b, a] = color.to_rgba8().to_u8_array();
        let mut rgba = vec![0_u8; mask.len() * 4];
        for (px, &m) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(mask) {
            if m != 0 {
                *px = [r, g, b, ((m as u32 * a as u32 + 127) / 255) as u8];
            }
        }
        self.draw_image(
            &PaintImage {
                data: &rgba,
                width,
                height,
                decoded: None,
                frame: None,
                opaque: false,
            },
            transform,
        );
    }

    /// Fill `shape` with `tile` repeated in both directions, sampled nearest
    /// (#468). One call however many repetitions the shape covers, which is
    /// what keeps an `8px` tiled background on a large box from costing one
    /// fill per tile.
    ///
    /// The default draws one `draw_image` per repetition the shape's bounding
    /// box touches, under a clip of `shape` — correct, and linear in the
    /// count; both shipped painters override it with a repeating image
    /// brush / pattern.
    fn fill_repeating(
        &mut self,
        fill: Fill,
        transform: Affine,
        tile: &RepeatTile<'_>,
        shape: &PaintShape,
    ) {
        let (w, h) = (tile.width as f64, tile.height as f64);
        if w <= 0.0 || h <= 0.0 || tile.transform.determinant().abs() < 1e-12 {
            return;
        }
        let unpremultiplied;
        let straight = if tile.premultiplied {
            unpremultiplied = crate::image_cache::unpremultiply_rgba(tile.pixels.data());
            &unpremultiplied[..]
        } else {
            tile.pixels.data()
        };
        let image = PaintImage {
            data: straight,
            width: tile.width,
            height: tile.height,
            decoded: None,
            frame: None,
            opaque: false,
        };
        // The shape's bounds in tile-grid space: which repetitions touch it.
        let bb = tile
            .transform
            .inverse()
            .transform_rect_bbox(shape.bounding_box());
        let (i0, i1) = ((bb.x0 / w).floor() as i64, (bb.x1 / w).ceil() as i64);
        let (j0, j1) = ((bb.y0 / h).floor() as i64, (bb.y1 / h).ceil() as i64);
        self.push_clip(fill, transform, shape);
        for j in j0..j1 {
            for i in i0..i1 {
                let at = Affine::translate((i as f64 * w, j as f64 * h));
                self.draw_image(&image, transform * tile.transform * at);
            }
        }
        self.pop_layer();
    }

    /// Clear `shape` to transparent: erase what has been drawn there, as an
    /// opaque fill composited `destination-out` would, through the open clips
    /// and anti-aliased at its edges (#1866). Inside a layer it erases the
    /// layer's own pixels.
    ///
    /// The default opens a [`BlendMode::DestinationOut`] layer over the
    /// shape's bounding box and fills the shape in it (Vello has no per-draw
    /// blend mode); the software painter erases in place, with no layer.
    fn erase(&mut self, fill: Fill, transform: Affine, shape: &PaintShape) {
        self.push_layer(
            BlendMode::DestinationOut,
            1.0,
            transform,
            &PaintShape::Rect(shape.bounding_box()),
        );
        self.fill_color(fill, transform, AlphaColor::<Srgb>::BLACK, shape);
        self.pop_layer();
    }

    /// Push a clip layer — all subsequent drawing is clipped to the shape.
    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape);

    /// Push an opacity/blend layer.
    ///
    /// Content drawn until the matching `pop_layer()` is composited with the
    /// given opacity and blend mode.
    fn push_layer(
        &mut self,
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        bounds: &PaintShape,
    );

    /// Push a layer that is **always** drawn on its own and composited back
    /// as a unit, source-over at `opacity` — even at opacity 1, where a
    /// backend may skip a [`push_layer`](Self::push_layer) as a no-op. What
    /// is drawn inside can then use [`BlendMode::Plus`] against the layer's
    /// own transparent start rather than against the backdrop. Popped by
    /// [`pop_layer`](Self::pop_layer).
    fn push_isolated_layer(&mut self, opacity: f32, transform: Affine, bounds: &PaintShape) {
        self.push_layer(BlendMode::Normal, opacity, transform, bounds);
    }

    /// Pop the most recent clip or opacity layer.
    fn pop_layer(&mut self);

    /// Offer a GPU surface's frame for the split-layer path (#1865): on
    /// [`SplitVerdict::Split`] the painter has taken the frame and routes
    /// everything painted after it by its place against the surface's region;
    /// the caller then draws nothing for the frame. The default refuses, and
    /// the caller draws the frame in the scene.
    fn split_surface(&mut self, split: &SurfaceSplit) -> SplitVerdict {
        let _ = split;
        SplitVerdict::InScene(Refusal::Unsupported)
    }

    /// Paint is starting `node`, one the runtime asked to place in paint
    /// order against the surfaces split so far (#1865 S4,
    /// `set_paint_order_watch`). The default ignores it.
    fn note_node_start(&mut self, node: crate::node::RawNodeId) {
        let _ = node;
    }

    /// Append all drawing operations from another painter of the same type.
    ///
    /// Used for DnD snapshot overlays. Default implementation is a no-op
    /// for backends that don't support composition of two painter outputs.
    fn append(&mut self, _other: &Self)
    where
        Self: Sized,
    {
        // Default: no-op. VelloPainter overrides with Scene::append.
    }
}
