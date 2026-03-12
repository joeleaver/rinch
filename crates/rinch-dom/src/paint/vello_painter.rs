//! [`Painter`] implementation backed by `vello::Scene` for GPU rendering.

use peniko::kurbo::Affine;
use peniko::{Blob, Brush, Fill, FontData, ImageAlphaType, ImageData, ImageFormat};
use vello::Scene;

use super::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};

/// GPU rendering backend that wraps a [`vello::Scene`].
///
/// Drawing commands are accumulated in the scene and later submitted
/// to the Vello rendering pipeline for GPU execution.
pub struct VelloPainter {
    scene: Scene,
}

impl VelloPainter {
    /// Create a new `VelloPainter` with an empty scene.
    pub fn new() -> Self {
        Self {
            scene: Scene::new(),
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

impl Default for VelloPainter {
    fn default() -> Self {
        Self::new()
    }
}

/// Dispatch a fill operation on a `PaintShape` to the appropriate Scene method.
macro_rules! dispatch_fill {
    ($scene:expr, $fill:expr, $transform:expr, $brush:expr, $shape:expr) => {
        match $shape {
            PaintShape::Rect(r) => $scene.fill($fill, $transform, $brush, None, r),
            PaintShape::RoundedRect(r) => $scene.fill($fill, $transform, $brush, None, r),
            PaintShape::BezPath(p) => $scene.fill($fill, $transform, $brush, None, p),
            PaintShape::Circle(c) => $scene.fill($fill, $transform, $brush, None, c),
            PaintShape::Line(l) => $scene.fill($fill, $transform, $brush, None, l),
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

impl Painter for VelloPainter {
    fn reset(&mut self) {
        self.scene.reset();
    }

    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape) {
        dispatch_fill!(self.scene, fill, transform, brush, shape);
    }

    fn stroke(
        &mut self,
        stroke: &peniko::kurbo::Stroke,
        transform: Affine,
        brush: &Brush,
        shape: &PaintShape,
    ) {
        dispatch_stroke!(self.scene, stroke, transform, brush, shape);
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
        self.scene
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

    fn draw_image(&mut self, image: &PaintImage<'_>, transform: Affine) {
        let blob: Blob<u8> = Blob::from(image.data.to_vec());
        let image_data = ImageData {
            data: blob,
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: image.width,
            height: image.height,
        };
        self.scene.draw_image(&image_data, transform);
    }

    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape) {
        dispatch_clip!(self.scene, fill, transform, shape);
    }

    fn push_layer(
        &mut self,
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        bounds: &PaintShape,
    ) {
        let mix = match blend {
            BlendMode::Normal => peniko::Mix::Normal,
            BlendMode::Saturation => peniko::Mix::Saturation,
        };
        dispatch_push_layer!(self.scene, Fill::NonZero, mix, opacity, transform, bounds);
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    fn append(&mut self, other: &Self) {
        self.scene.append(&other.scene, None);
    }
}
