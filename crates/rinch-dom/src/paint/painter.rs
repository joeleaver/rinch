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
}

// ── Image data ──────────────────────────────────────────────────────────────

/// Decoded image data ready for painting.
///
/// Always RGBA8 with straight (non-premultiplied) alpha.
pub struct PaintImage<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
}

// ── Glyph run ───────────────────────────────────────────────────────────────

/// A single positioned glyph for text rendering.
#[derive(Clone, Copy, Debug)]
pub struct PaintGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
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

    /// Pop the most recent clip or opacity layer.
    fn pop_layer(&mut self);

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
