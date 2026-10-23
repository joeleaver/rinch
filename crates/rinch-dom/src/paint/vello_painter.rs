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

/// The brush transform that makes Vello sample an image at each pixel's
/// CENTRE, for an image brush whose image-to-device map would otherwise be
/// `transform * brush` (#1027).
///
/// Vello 0.7's fine shader (`fine.wgsl`, `CMD_IMAGE`) maps the pixel's
/// top-left corner into the image, where CSS and tiny-skia map its centre:
/// its bilinear arm then reads half a texel up and left of the right point
/// (an image at a whole-pixel origin comes out averaged with its
/// neighbours), and its nearest arm picks the texel under the corner. Moving
/// the image half a DEVICE pixel up and left, `T(-0.5) * transform * brush`,
/// puts the centre where the shader looks, under any scale, rotation or
/// skew; the shape is unaffected, being drawn with `transform` alone. A
/// scene appended under a translation keeps it (translations commute); one
/// appended under a scale would not, and none is.
///
/// A degenerate `transform` (which draws nothing) keeps `brush`.
fn centre_sampling(transform: Affine, brush: Affine) -> Affine {
    let det = transform.determinant();
    if !det.is_finite() || det == 0.0 {
        return brush;
    }
    transform.inverse() * Affine::translate((-0.5, -0.5)) * transform * brush
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

impl Painter for VelloPainter {
    fn reset(&mut self) {
        self.scene.reset();
    }

    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape) {
        // Vello's fine shader evaluates a gradient at each pixel's top-left
        // corner, as it samples an image (#1027), where CSS and tiny-skia
        // evaluate it at the centre: a 1px hard-stop line of a repeating
        // gradient fell between two corners and was not drawn (#1798).
        let bt = matches!(brush, Brush::Gradient(_))
            .then(|| centre_sampling(transform, Affine::IDENTITY));
        dispatch_fill!(self.scene, fill, transform, brush, bt, shape);
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
        // `Scene::draw_image` is this fill with no brush transform; the
        // brush transform moves the sample to the pixel's centre (#1027).
        let rect = peniko::kurbo::Rect::new(0.0, 0.0, image.width as f64, image.height as f64);
        let brush = peniko::ImageBrush::new(image_data);
        self.scene.fill(
            Fill::NonZero,
            transform,
            &brush,
            Some(centre_sampling(transform, Affine::IDENTITY)),
            &rect,
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
        let brush_transform = Some(centre_sampling(transform, tile_transform));
        match shape {
            PaintShape::Rect(r) => self.scene.fill(fill, transform, &brush, brush_transform, r),
            PaintShape::RoundedRect(r) => {
                self.scene.fill(fill, transform, &brush, brush_transform, r)
            }
            PaintShape::BezPath(p) => self.scene.fill(fill, transform, &brush, brush_transform, p),
            PaintShape::Circle(c) => self.scene.fill(fill, transform, &brush, brush_transform, c),
            PaintShape::Line(l) => self.scene.fill(fill, transform, &brush, brush_transform, l),
        }
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
            BlendMode::Normal => peniko::BlendMode::from(peniko::Mix::Normal),
            BlendMode::Saturation => peniko::BlendMode::from(peniko::Mix::Saturation),
            BlendMode::Plus => peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::Plus),
            BlendMode::DestinationIn => {
                peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::DestIn)
            }
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

#[cfg(test)]
mod sampling_tests {
    //! What Vello 0.7's fine shader samples for the images this painter
    //! encodes (#1027). There is no GPU in CI, so the shader's image arm
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
    /// `(x, y)` — the shader's `my_xy` is the pixel's corner, not its centre.
    /// Returns the grey channel.
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
        // The corner of device pixel 5 (row 7) maps to its centre's texel
        // coordinate (`centre_sampling`); nearest reads its floor.
        let at = u(5.0, 7.0);
        assert!(
            (at - 4.5).abs() < 1e-3,
            "pixel 5's centre is texel {at}; 4.5 is the tile placed on pixel 1"
        );
        assert!((m[0] - 1.0).abs() < 1e-6 && m[1] == 0.0 && m[2] == 0.0);
    }
}
