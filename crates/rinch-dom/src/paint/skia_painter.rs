//! Software rendering backend using tiny-skia.
//!
//! Implements the [`Painter`] trait for CPU-based rasterization.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, PathEl, Shape, Stroke as KurboStroke};
use peniko::{Brush, Fill, FontData, Gradient, GradientKind};

use tiny_skia::{
    Color as SkColor, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, PixmapRef, SpreadMode, Stroke as SkStroke, Transform,
};

use super::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};

// ── Conversion helpers ────────────────────────────────────────────────────

/// Convert a kurbo `Affine` to a tiny-skia `Transform`.
///
/// kurbo Affine stores `[sx, ky, kx, sy, tx, ty]` which maps directly
/// to tiny-skia's `Transform::from_row(sx, ky, kx, sy, tx, ty)`.
fn affine_to_transform(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(
        c[0] as f32,
        c[1] as f32,
        c[2] as f32,
        c[3] as f32,
        c[4] as f32,
        c[5] as f32,
    )
}

/// Premultiply one straight-alpha colour channel by `alpha`, **rounding to
/// nearest** rather than truncating (#461, #473).
///
/// The exact value is `c * alpha / 255`. `(c * alpha + 127) / 255` in integer
/// arithmetic is that value rounded to nearest, exactly and with no float:
/// `floor((x + 127) / 255)` and `floor((x + 127.5) / 255)` cannot differ for an
/// integer `x`, and a tie would need `2 * c * alpha` to be an odd multiple of
/// `255`, which an even number is not. It agrees with `draw_image`'s
/// `(c as f32 * af + 0.5) as u8` over the whole domain.
///
/// It is **not** the `+128` that `blit_rgba` uses. `+128` is *not* a
/// tie-breaking rule — since no tie exists in this domain, round-half-up and
/// round-to-nearest are the same function, so "round-half-up" would mean `+127`.
/// What `+128` does is round up at a fraction of `127/255 ≈ 0.498`, i.e. it
/// applies round-to-nearest against a threshold biased low by `1/510`, and so
/// disagrees at `c * alpha ≡ 127 (mod 255)` — 128 of the 65 536 pairs. Both
/// facts are pinned by
/// `the_helper_agrees_with_the_float_spelling_and_not_with_plus_128`.
///
/// **The premultiplied invariant survives the rounding**, which is what tiny-skia
/// validates: `c <= 255` gives `c * alpha / 255 <= alpha`, and rounding a value
/// that is at most `alpha` cannot exceed `alpha` because the tie case does not
/// arise. Proved over the whole 256x256 domain, against tiny-skia's own
/// `PremultipliedColorU8::from_rgba`, by `rounding_never_breaks_the_premultiplied_invariant`.
#[inline]
fn premultiply_channel(c: u8, alpha: u8) -> u8 {
    ((c as u16 * alpha as u16 + 127) / 255) as u8
}

/// Convert a peniko `AlphaColor<Srgb>` to a tiny-skia `Color`.
fn to_skia_color(c: AlphaColor<Srgb>) -> SkColor {
    let rgba = c.to_rgba8();
    SkColor::from_rgba8(rgba.r, rgba.g, rgba.b, rgba.a)
}

/// Convert a peniko `Fill` rule to tiny-skia `FillRule`.
fn to_fill_rule(fill: Fill) -> FillRule {
    match fill {
        Fill::NonZero => FillRule::Winding,
        Fill::EvenOdd => FillRule::EvenOdd,
    }
}

/// Convert a peniko `Brush` to a tiny-skia `Paint`.
///
/// Returns `None` for image brushes (not supported).
fn brush_to_paint(brush: &Brush) -> Option<Paint<'static>> {
    match brush {
        Brush::Solid(color) => Some(Paint {
            shader: tiny_skia::Shader::SolidColor(to_skia_color(*color)),
            anti_alias: true,
            ..Paint::default()
        }),
        Brush::Gradient(gradient) => gradient_to_paint(gradient),
        Brush::Image(_) => {
            // Image brushes not supported in software renderer
            None
        }
    }
}

/// Convert a peniko `Gradient` to a tiny-skia `Paint`.
fn gradient_to_paint(gradient: &Gradient) -> Option<Paint<'static>> {
    let stops: Vec<GradientStop> = gradient
        .stops
        .iter()
        .map(|s| {
            let color = s.color.to_alpha_color::<Srgb>();
            GradientStop::new(s.offset, to_skia_color(color))
        })
        .collect();

    if stops.is_empty() {
        return None;
    }

    let spread = match gradient.extend {
        peniko::Extend::Pad => SpreadMode::Pad,
        peniko::Extend::Repeat => SpreadMode::Repeat,
        peniko::Extend::Reflect => SpreadMode::Reflect,
    };

    let shader = match &gradient.kind {
        GradientKind::Linear(linear) => {
            let start = tiny_skia::Point {
                x: linear.start.x as f32,
                y: linear.start.y as f32,
            };
            let end = tiny_skia::Point {
                x: linear.end.x as f32,
                y: linear.end.y as f32,
            };
            LinearGradient::new(start, end, stops, spread, Transform::identity())?
        }
        GradientKind::Radial(radial) => {
            // tiny-skia RadialGradient: center of end circle, radius of end circle,
            // center of start circle, radius of start circle
            let start = tiny_skia::Point {
                x: radial.start_center.x as f32,
                y: radial.start_center.y as f32,
            };
            let end = tiny_skia::Point {
                x: radial.end_center.x as f32,
                y: radial.end_center.y as f32,
            };
            tiny_skia::RadialGradient::new(
                start,
                end,
                radial.end_radius,
                stops,
                spread,
                Transform::identity(),
            )?
        }
        GradientKind::Sweep(_) => {
            // Sweep gradients not supported by tiny-skia; fall back to first stop color
            let first = gradient.stops.first()?;
            let color = first.color.to_alpha_color::<Srgb>();
            tiny_skia::Shader::SolidColor(to_skia_color(color))
        }
    };

    Some(Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    })
}

/// Convert a `PaintShape` to a tiny-skia `Path`.
fn shape_to_path(shape: &PaintShape) -> Option<Path> {
    match shape {
        PaintShape::Rect(r) => {
            let rect =
                tiny_skia::Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32)?;
            Some(PathBuilder::from_rect(rect))
        }
        PaintShape::RoundedRect(rr) => {
            // Use kurbo's path_elements to convert rounded rect to path commands
            let mut pb = PathBuilder::new();
            for el in rr.path_elements(0.1) {
                match el {
                    PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
                    PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
                    PathEl::QuadTo(p1, p2) => {
                        pb.quad_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32)
                    }
                    PathEl::CurveTo(p1, p2, p3) => pb.cubic_to(
                        p1.x as f32,
                        p1.y as f32,
                        p2.x as f32,
                        p2.y as f32,
                        p3.x as f32,
                        p3.y as f32,
                    ),
                    PathEl::ClosePath => pb.close(),
                }
            }
            pb.finish()
        }
        PaintShape::BezPath(bp) => {
            let mut pb = PathBuilder::new();
            for el in bp.elements() {
                match *el {
                    PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
                    PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
                    PathEl::QuadTo(p1, p2) => {
                        pb.quad_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32)
                    }
                    PathEl::CurveTo(p1, p2, p3) => pb.cubic_to(
                        p1.x as f32,
                        p1.y as f32,
                        p2.x as f32,
                        p2.y as f32,
                        p3.x as f32,
                        p3.y as f32,
                    ),
                    PathEl::ClosePath => pb.close(),
                }
            }
            pb.finish()
        }
        PaintShape::Circle(c) => {
            PathBuilder::from_circle(c.center.x as f32, c.center.y as f32, c.radius as f32)
        }
        PaintShape::Line(l) => {
            let mut pb = PathBuilder::new();
            pb.move_to(l.p0.x as f32, l.p0.y as f32);
            pb.line_to(l.p1.x as f32, l.p1.y as f32);
            pb.finish()
        }
    }
}

/// Convert a kurbo `Stroke` to a tiny-skia `Stroke`.
fn to_skia_stroke(stroke: &KurboStroke) -> SkStroke {
    let line_cap = match stroke.start_cap {
        peniko::kurbo::Cap::Butt => LineCap::Butt,
        peniko::kurbo::Cap::Round => LineCap::Round,
        peniko::kurbo::Cap::Square => LineCap::Square,
    };
    let line_join = match stroke.join {
        peniko::kurbo::Join::Bevel => LineJoin::Bevel,
        peniko::kurbo::Join::Miter => LineJoin::Miter,
        peniko::kurbo::Join::Round => LineJoin::Round,
    };
    let dash = if !stroke.dash_pattern.is_empty() {
        let pattern: Vec<f32> = stroke.dash_pattern.iter().map(|d| *d as f32).collect();
        tiny_skia::StrokeDash::new(pattern, stroke.dash_offset as f32)
    } else {
        None
    };
    SkStroke {
        width: stroke.width as f32,
        miter_limit: stroke.miter_limit as f32,
        line_cap,
        line_join,
        dash,
    }
}

// ── Layer state ───────────────────────────────────────────────────────────

/// Saved state for clip/layer operations.
enum LayerState {
    /// A push that changed nothing, so its pop restores nothing.
    ///
    /// The stack has to stay balanced whatever a push decided to do, so a
    /// fast path that skips its work still pushes — and this is the only
    /// spelling of "skipped" that is true. `Clip { previous_mask: None }` is
    /// *not* a way to say it: that is the claim that nothing was clipping,
    /// and popping it installs that claim over whatever really was. See
    /// [`TinySkiaPainter::push_layer`]'s near-opaque branch (#560) and the
    /// rule stated in [`TinySkiaPainter::push_clip`].
    Noop,
    /// A clip layer — just a saved mask to restore on pop.
    Clip { previous_mask: Option<Mask> },
    /// An opacity/blend layer — content drawn to a temporary pixmap.
    Opacity {
        parent_pixmap: Pixmap,
        parent_mask: Option<Mask>,
        opacity: f32,
    },
}

// ── TinySkiaPainter ───────────────────────────────────────────────────────

/// Software rendering backend using tiny-skia.
///
/// Rasterizes directly to an RGBA pixel buffer. All drawing operations are
/// immediate — there is no command recording step.
pub struct TinySkiaPainter {
    pixmap: Pixmap,
    /// Current clip mask (intersection of all active clip layers).
    clip_mask: Option<Mask>,
    /// Stack of saved layer states.
    layer_stack: Vec<LayerState>,
    /// Reusable swash scale context for glyph rasterization.
    scale_context: swash::scale::ScaleContext,
}

impl TinySkiaPainter {
    /// Create a new painter with the given dimensions.
    ///
    /// Panics if width or height is 0.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            pixmap: Pixmap::new(width.max(1), height.max(1)).expect("invalid pixmap dimensions"),
            clip_mask: None,
            layer_stack: Vec::new(),
            scale_context: swash::scale::ScaleContext::new(),
        }
    }

    /// Resize the painting surface, clearing all content.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if width != self.pixmap.width() || height != self.pixmap.height() {
            self.pixmap = Pixmap::new(width, height).expect("invalid pixmap dimensions");
            self.clip_mask = None;
            self.layer_stack.clear();
        }
    }

    /// Get the raw RGBA pixel data (straight alpha, 4 bytes per pixel).
    ///
    /// Note: tiny-skia stores premultiplied alpha internally. This method
    /// returns the raw premultiplied data. For presentation to softbuffer,
    /// the caller may need to convert to the expected format.
    pub fn pixels(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// Get mutable access to the raw premultiplied pixel data.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        self.pixmap.data_mut()
    }

    /// Width of the surface in pixels.
    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    /// Height of the surface in pixels.
    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// Clear a rectangular region to white.
    ///
    /// Used for dirty region caching: only the changed area is cleared
    /// before repainting, preserving unchanged pixels from the previous frame.
    pub fn clear_rect_white(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.clear_rect_rgba(x, y, w, h, 255, 255, 255, 255);
    }

    /// Clear a rectangular region to black.
    pub fn clear_rect_black(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.clear_rect_rgba(x, y, w, h, 0, 0, 0, 255);
    }

    /// Clear a rectangular region to a specific premultiplied RGBA color.
    #[allow(clippy::too_many_arguments)]
    fn clear_rect_rgba(&mut self, x: u32, y: u32, w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) {
        let pw = self.pixmap.width();
        let ph = self.pixmap.height();
        let x1 = (x + w).min(pw);
        let y1 = (y + h).min(ph);
        let x0 = x.min(x1);
        let y0 = y.min(y1);

        let data = self.pixmap.data_mut();
        for row in y0..y1 {
            let row_start = (row * pw + x0) as usize * 4;
            let row_end = (row * pw + x1) as usize * 4;
            let row_data = &mut data[row_start..row_end];
            for pixel in row_data.as_chunks_mut::<4>().0 {
                pixel[0] = r;
                pixel[1] = g;
                pixel[2] = b;
                pixel[3] = a;
            }
        }
    }

    /// Fill the entire surface with an opaque white background.
    pub fn fill_white(&mut self) {
        self.pixmap.fill(tiny_skia::Color::WHITE);
    }

    /// Fill the entire surface with a fully transparent background.
    pub fn fill_transparent(&mut self) {
        self.pixmap.fill(tiny_skia::Color::TRANSPARENT);
    }

    /// Clear a rectangular region to fully transparent.
    pub fn clear_rect_transparent(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.clear_rect_rgba(x, y, w, h, 0, 0, 0, 0);
    }

    /// Fill the entire surface with an opaque black background.
    ///
    /// Used as the base layer when composite surfaces are present,
    /// matching the GPU compositor's black clear color.
    pub fn fill_black(&mut self) {
        self.pixmap.fill(tiny_skia::Color::BLACK);
    }

    /// Blit RGBA pixel data into a destination rectangle on the surface.
    ///
    /// The source pixels are scaled to fit the destination rect. Pixels outside
    /// the surface bounds are clipped. Used for compositing RenderSurface frames
    /// (video, custom renderers) in the software path.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_rgba(
        &mut self,
        pixels: &[u8],
        src_w: u32,
        src_h: u32,
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
    ) {
        if src_w == 0 || src_h == 0 || dst_w <= 0.0 || dst_h <= 0.0 {
            return;
        }
        let expected = (src_w * src_h * 4) as usize;
        if pixels.len() < expected {
            return;
        }

        // Create a pixmap from the source RGBA data.
        // tiny-skia expects premultiplied alpha, so premultiply in-place.
        let mut premul = pixels[..expected].to_vec();
        for chunk in premul.as_chunks_mut::<4>().0 {
            let a = chunk[3] as u16;
            if a == 0 {
                chunk[0] = 0;
                chunk[1] = 0;
                chunk[2] = 0;
            } else if a < 255 {
                chunk[0] = ((chunk[0] as u16 * a + 128) / 255) as u8;
                chunk[1] = ((chunk[1] as u16 * a + 128) / 255) as u8;
                chunk[2] = ((chunk[2] as u16 * a + 128) / 255) as u8;
            }
        }

        let Some(src_pixmap) = PixmapRef::from_bytes(&premul, src_w, src_h) else {
            return;
        };

        // Compute transform: scale source to destination size, then translate
        let sx = dst_w / src_w as f32;
        let sy = dst_h / src_h as f32;
        let transform = Transform::from_scale(sx, sy).post_translate(dst_x, dst_y);

        self.pixmap.draw_pixmap(
            0,
            0,
            src_pixmap,
            &PixmapPaint {
                opacity: 1.0,
                blend_mode: tiny_skia::BlendMode::SourceOver,
                quality: tiny_skia::FilterQuality::Bilinear,
            },
            transform,
            None,
        );
    }
}

impl Painter for TinySkiaPainter {
    fn reset(&mut self) {
        self.pixmap.fill(tiny_skia::Color::TRANSPARENT);
        self.clip_mask = None;
        self.layer_stack.clear();
    }

    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape) {
        // A fully transparent solid fill writes no pixel — `SourceOver` at
        // alpha 0 leaves the destination exactly as it found it — so
        // rasterising it is cost with no output, and the cost is proportional
        // to the shape. The guard belongs here rather than at any one caller:
        // `transparent` is `background-color`'s initial value, so it arrives
        // from every element on every page, and it arrives the same way from
        // border colours, text-decoration and the render-surface backdrop.
        if let Brush::Solid(color) = brush
            && color.components[3] <= 0.0
        {
            return;
        }
        let Some(paint) = brush_to_paint(brush) else {
            return;
        };
        let Some(path) = shape_to_path(shape) else {
            return;
        };
        // Skip degenerate paths that tiny-skia can't fill (warns on zero-area).
        // Use || since a zero-height horizontal line or zero-width vertical line
        // has no fillable area either.
        let bounds = path.bounds();
        if bounds.width() < 0.001 || bounds.height() < 0.001 {
            return;
        }
        let ts = affine_to_transform(transform);
        let fill_rule = to_fill_rule(fill);
        self.pixmap
            .fill_path(&path, &paint, fill_rule, ts, self.clip_mask.as_ref());
    }

    fn stroke(
        &mut self,
        stroke: &KurboStroke,
        transform: Affine,
        brush: &Brush,
        shape: &PaintShape,
    ) {
        let Some(paint) = brush_to_paint(brush) else {
            return;
        };
        let Some(path) = shape_to_path(shape) else {
            return;
        };
        // Skip empty paths — stroke_path internally converts to fill_path
        // which warns on zero-area results
        if path.bounds().width() < 0.001 && path.bounds().height() < 0.001 {
            return;
        }
        let ts = affine_to_transform(transform);
        let sk_stroke = to_skia_stroke(stroke);
        self.pixmap
            .stroke_path(&path, &paint, &sk_stroke, ts, self.clip_mask.as_ref());
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
        let font_data: &[u8] = font.data.as_ref();
        let Some(font_ref) = swash::FontRef::from_index(font_data, font.index as usize) else {
            return;
        };

        // Resolve brush color (only solid colors for now)
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => AlphaColor::new([0.0, 0.0, 0.0, 1.0]), // fallback to black
        };
        let rgba = color.to_rgba8();
        let (cr, cg, cb, ca) = (rgba.r, rgba.g, rgba.b, rgba.a);

        // Rasterize all glyphs first to avoid borrow conflict with self
        let rendered: Vec<_> = {
            let mut scaler = self
                .scale_context
                .builder(font_ref)
                .size(font_size)
                .hint(hint)
                .normalized_coords(normalized_coords)
                .build();

            let render = swash::scale::Render::new(&[
                swash::scale::Source::ColorOutline(0),
                swash::scale::Source::Outline,
            ]);

            glyphs
                .iter()
                .filter_map(|glyph| {
                    let image = render.render(&mut scaler, glyph.id as u16)?;
                    if image.placement.width == 0 || image.placement.height == 0 {
                        return None;
                    }
                    let gx = glyph.x + image.placement.left as f32;
                    let gy = glyph.y - image.placement.top as f32;
                    Some((image, gx, gy))
                })
                .collect()
        };

        let ts = affine_to_transform(transform);
        let glyph_ts = glyph_transform.map(affine_to_transform);

        for (image, gx, gy) in &rendered {
            match image.content {
                swash::scale::image::Content::Mask => {
                    self.blit_alpha_mask(
                        &image.data,
                        image.placement.width,
                        image.placement.height,
                        *gx,
                        *gy,
                        cr,
                        cg,
                        cb,
                        ca,
                        ts,
                        glyph_ts,
                    );
                }
                swash::scale::image::Content::Color => {
                    self.blit_color_glyph(
                        &image.data,
                        image.placement.width,
                        image.placement.height,
                        *gx,
                        *gy,
                        ts,
                        glyph_ts,
                    );
                }
                swash::scale::image::Content::SubpixelMask => {
                    let alpha_data: Vec<u8> = image
                        .data
                        .chunks(4)
                        .map(|px| px.get(3).copied().unwrap_or(0))
                        .collect();
                    self.blit_alpha_mask(
                        &alpha_data,
                        image.placement.width,
                        image.placement.height,
                        *gx,
                        *gy,
                        cr,
                        cg,
                        cb,
                        ca,
                        ts,
                        glyph_ts,
                    );
                }
            }
        }
    }

    fn draw_image(&mut self, image: &PaintImage<'_>, transform: Affine) {
        if image.width == 0 || image.height == 0 {
            return;
        }

        // Convert straight alpha RGBA to premultiplied (tiny-skia requirement)
        let mut premul = Vec::with_capacity(image.data.len());
        for chunk in image.data.chunks(4) {
            let r = chunk[0];
            let g = chunk[1];
            let b = chunk[2];
            let a = chunk[3];
            if a == 255 {
                premul.extend_from_slice(&[r, g, b, a]);
            } else if a == 0 {
                premul.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let af = a as f32 / 255.0;
                premul.push((r as f32 * af + 0.5) as u8);
                premul.push((g as f32 * af + 0.5) as u8);
                premul.push((b as f32 * af + 0.5) as u8);
                premul.push(a);
            }
        }

        let Some(src) = PixmapRef::from_bytes(&premul, image.width, image.height) else {
            return;
        };

        let ts = affine_to_transform(transform);
        let paint = PixmapPaint::default();
        self.pixmap
            .draw_pixmap(0, 0, src, &paint, ts, self.clip_mask.as_ref());
    }

    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape) {
        let previous_mask = self.clip_mask.take();

        // Both give-up branches below share one rule, and it is a rule about
        // this painter rather than about these two branches:
        //
        // > **A fast path that skips clipping work must still preserve the clip
        // > it inherited.**
        //
        // That is the painter-level cousin of #547's "giving up is not a licence
        // to narrow", and it is stated as the rule rather than as a count of
        // instances because the count kept being wrong. `.take()` above left
        // `self.clip_mask` at `None`, which is not "no new clip" but "no clip at
        // all", so a give-up did not merely fail to add a clip — it dropped the
        // *enclosing* one for the whole subtree, and content painted straight
        // through an ancestor's `overflow: hidden`. Measured, in review of #540,
        // and worth stating because it is strictly worse than the symptom the
        // bug was reported for: in a real software frame the outermost clip is
        // the dirty-region clip (`RinchApp::build_pixels`), so `previous_mask`
        // is `Some(..)` for every DOM clip in a partial repaint.
        //
        // [`Self::push_layer`]'s near-opaque fast path was the **third**
        // instance of the same rule broken the same way — `previous_mask: None`
        // pushed without taking the mask — and is fixed under #560. It is fixed
        // the other way round from these two, and the difference is worth
        // knowing before adding a fourth fast path: a give-up *here* has already
        // taken the mask, so the cheap repair is to put it back, while a
        // near-opaque layer never took it, so the cheap repair is to save
        // nothing at all ([`LayerState::Noop`]). Both satisfy the rule; only one
        // of them is free at each site.
        let Some(path) = shape_to_path(shape) else {
            // Hardening, not a fix for anything reachable — say so rather than
            // let it read as a closed defect. No `push_clip` call site in the
            // workspace can produce a shape `shape_to_path` refuses: every one
            // passes a `Rect` or a `RoundedRect`, `clip_shape` builds its rect
            // from non-negative layout dimensions so it is never inverted, and
            // `select.rs` clamps its own with `.max(text_x)`. Reaching here needs
            // a geometrically inverted or non-finite rect. Degenerate rects,
            // zero-radius circles and single-segment paths all *build* fine and
            // land in the branch below instead — checked, not assumed.
            //
            // Keeping the enclosing mask is the answer that is right whatever
            // the unbuildable shape meant. Blanking would be a guess about a
            // shape we could not read, and this is the file where a guess about
            // an unmappable region is already called out as the one way to get
            // clipping wrong (see the intersect note below).
            self.clip_mask = previous_mask.clone();
            self.layer_stack.push(LayerState::Clip { previous_mask });
            return;
        };

        let bounds = path.bounds();
        if bounds.width() < 0.001 || bounds.height() < 0.001 {
            // Card K51: a degenerate clip path is not the same thing as no clip
            // at all, and treating them alike is what let a `height: 0`
            // `overflow: hidden` box (card J1's collapsed group) go on painting
            // its rows at full size, in their old position, forever. A path
            // whose bounds round to nothing is the *strictest* clip there is,
            // not the absence of one: nothing behind it should show, and
            // `Mask::new` already hands back exactly that — a mask of zeroes —
            // so installing it costs nothing an ordinary clip wasn't already
            // going to pay a few lines below.
            //
            // This is also the branch that used to leak the enclosing clip, per
            // the rule above; the all-zero mask is stricter than `previous_mask`
            // by construction, so it settles both halves at once.
            let w = self.pixmap.width();
            let h = self.pixmap.height();
            let mask = Mask::new(w, h).expect("failed to create clip mask");
            self.clip_mask = Some(mask);
            self.layer_stack.push(LayerState::Clip { previous_mask });
            return;
        }

        let w = self.pixmap.width();
        let h = self.pixmap.height();
        let mut mask = Mask::new(w, h).expect("failed to create clip mask");
        let ts = affine_to_transform(transform);
        let fill_rule = to_fill_rule(fill);
        mask.fill_path(&path, fill_rule, true, ts);

        // If there was a previous mask, intersect with it — but only over the
        // part of the surface the new clip path actually reaches.
        //
        // `Mask::new` hands back a mask of zeroes and `fill_path` writes only
        // inside the path, so every byte outside the path's device-space bounds
        // is still zero, and zero times whatever the parent mask holds is zero.
        // Multiplying those bytes is arithmetic whose answer is already in the
        // buffer. Running the loop over the whole mask regardless is what made
        // a clip cost the surface rather than the box: at 1080×2460 that is
        // 2.66 million multiply-and-divides per nested clip, and the library
        // screen pushes seventeen clips a frame — every `overflow: hidden` box,
        // every scroller, every rounded thumbnail — for about 35ms of a 90ms
        // frame on the moto g stylus 5G. See card K24.
        //
        // The bounds are padded by a pixel because `fill_path` is called with
        // anti-aliasing on and its coverage can spill into the pixel outside
        // the geometric edge. The padding is done in floating point, before
        // the cast: `as i64` saturates, so adding to the result of one can
        // overflow.
        //
        // If the bounds cannot be mapped into device space the whole surface
        // is walked. Narrowing on a guess would be the one way to get this
        // wrong — outside the region it walks, the parent mask is never
        // applied, and content paints straight through the enclosing clip.
        if let Some(ref prev) = previous_mask {
            let px = |v: f32, limit: u32| -> usize { (v as f64).clamp(0.0, limit as f64) as usize };
            let (x0, x1, y0, y1) = match bounds.transform(ts) {
                Some(device) => (
                    px((device.left() - 1.0).floor(), w),
                    px((device.right() + 1.0).ceil(), w),
                    px((device.top() - 1.0).floor(), h),
                    px((device.bottom() + 1.0).ceil(), h),
                ),
                None => (0, w as usize, 0, h as usize),
            };
            let stride = w as usize;
            let mask_data = mask.data_mut();
            let prev_data = prev.data();
            for y in y0..y1 {
                let row = y * stride;
                let m = &mut mask_data[row + x0..row + x1];
                let p = &prev_data[row + x0..row + x1];
                for (m, p) in m.iter_mut().zip(p.iter()) {
                    *m = ((*m as u16 * *p as u16 + 127) / 255) as u8;
                }
            }
        }

        self.clip_mask = Some(mask);
        self.layer_stack.push(LayerState::Clip { previous_mask });
    }

    fn push_layer(
        &mut self,
        _blend: BlendMode,
        opacity: f32,
        _transform: Affine,
        _bounds: &PaintShape,
    ) {
        if (opacity - 1.0).abs() < f32::EPSILON {
            // Near-opaque: compositing a layer back at this alpha changes no
            // pixel, so no layer is allocated. The push still has to happen —
            // `pop_layer` is called unconditionally by the caller — and what it
            // pushes has to say *truthfully* that nothing was changed.
            //
            // > **A fast path that skips clipping work must still preserve the
            // > clip it inherited.** Pushing `previous_mask: None` is a claim
            // > that nothing was clipping — not a way of saying "I did not need
            // > to change anything". The two are indistinguishable at the push
            // > and catastrophic at the pop.
            //
            // `Clip { previous_mask: None }` stood here and was the second
            // claim while meaning the first: this branch never `.take()`s the
            // mask, so the clip it inherited is still in force for everything
            // drawn inside the layer, and then `pop_layer` installs the `None`
            // over it. Everything painted after the pop, until something else
            // pushes or pops a clip, is unclipped — and in a partial repaint
            // that includes the dirty-region clip, which `RinchApp::build_pixels`
            // pushes around the whole of `paint_document`.
            //
            // Measured from CSS, not inferred: `opacity: 0.99999994` (the one
            // f32 below `1.0` inside `f32::EPSILON` of it) on the first of two
            // stacking-context children of a `50x50` `overflow: hidden` box lets
            // the *second* one paint at full size outside it, while `opacity: 1`
            // and `opacity: 0.5` both clip.
            //
            // **`opacity` is the exotic way in and not the one to fix this for.**
            // `paint_node`'s `filter: grayscale(...)` arm opens a
            // `BlendMode::Saturation` layer at the filter's own amount, so
            // `filter: grayscale(1)` arrives here at exactly `1.0`. That needs
            // no unusual float and no stacking context: two plain in-flow
            // siblings of one `overflow: hidden` box, the first greyed, and the
            // second painted a hundred pixels outside it. Both are in
            // `opacity_layer_clip_tests`.
            //
            // Saving the mask instead, the way [`Self::push_clip`]'s give-up
            // branches do, would also be correct and is the wrong trade here:
            // it means cloning a full-surface `Mask` (2.66MB at 1080x2460, per
            // the cost note in `push_clip`) on the one path whose entire
            // purpose is to cost nothing. Saving nothing is both cheaper and
            // the more honest statement — see [`LayerState::Noop`].
            self.layer_stack.push(LayerState::Noop);
            return;
        }

        // Save current pixmap and mask, draw to a fresh pixmap
        let w = self.pixmap.width();
        let h = self.pixmap.height();
        let parent_pixmap = std::mem::replace(
            &mut self.pixmap,
            Pixmap::new(w, h).expect("failed to create layer pixmap"),
        );
        let parent_mask = self.clip_mask.take();

        // Restore clip mask on the new layer (clone it)
        if let Some(ref m) = parent_mask {
            self.clip_mask = Some(m.clone());
        }

        self.layer_stack.push(LayerState::Opacity {
            parent_pixmap,
            parent_mask,
            opacity,
        });
    }

    fn pop_layer(&mut self) {
        let Some(state) = self.layer_stack.pop() else {
            return;
        };

        match state {
            // Nothing was saved because nothing was changed (`push_layer`'s
            // near-opaque branch). Writing anything to `clip_mask` here — a
            // `None` above all — would be inventing state this push never took.
            LayerState::Noop => {}
            LayerState::Clip { previous_mask } => {
                self.clip_mask = previous_mask;
            }
            LayerState::Opacity {
                mut parent_pixmap,
                parent_mask,
                opacity,
            } => {
                // Composite the layer pixmap back onto the parent with opacity
                let layer_ref = self.pixmap.as_ref();
                let paint = PixmapPaint {
                    opacity,
                    blend_mode: tiny_skia::BlendMode::SourceOver,
                    quality: tiny_skia::FilterQuality::Nearest,
                };
                parent_pixmap.draw_pixmap(0, 0, layer_ref, &paint, Transform::identity(), None);

                self.pixmap = parent_pixmap;
                self.clip_mask = parent_mask;
            }
        }
    }

    fn append(&mut self, other: &Self) {
        let src = other.pixmap.as_ref();
        let paint = PixmapPaint::default();
        self.pixmap
            .draw_pixmap(0, 0, src, &paint, Transform::identity(), None);
    }
}

// ── Glyph blitting helpers ────────────────────────────────────────────────

impl TinySkiaPainter {
    /// Blit an alpha mask glyph onto the pixmap with the given color.
    ///
    /// Uses a temporary pixmap + `draw_pixmap()` so that the full transform
    /// (including rotation/skew) is applied by tiny-skia.
    #[allow(clippy::too_many_arguments)]
    fn blit_alpha_mask(
        &mut self,
        mask_data: &[u8],
        glyph_w: u32,
        glyph_h: u32,
        gx: f32,
        gy: f32,
        cr: u8,
        cg: u8,
        cb: u8,
        ca: u8,
        transform: Transform,
        _glyph_transform: Option<Transform>,
    ) {
        if glyph_w == 0 || glyph_h == 0 {
            return;
        }

        // Build a temporary RGBA pixmap from the alpha mask + brush color
        let mut glyph_pm = match Pixmap::new(glyph_w, glyph_h) {
            Some(pm) => pm,
            None => return,
        };
        let glyph_pixels = glyph_pm.data_mut();

        for (i, &alpha) in mask_data
            .iter()
            .enumerate()
            .take((glyph_w * glyph_h) as usize)
        {
            if alpha == 0 {
                continue;
            }
            // Combine glyph alpha with brush alpha → premultiplied RGBA
            let a = ((alpha as u16 * ca as u16 + 127) / 255) as u8;
            if a == 0 {
                continue;
            }
            let idx = i * 4;
            glyph_pixels[idx] = premultiply_channel(cr, a);
            glyph_pixels[idx + 1] = premultiply_channel(cg, a);
            glyph_pixels[idx + 2] = premultiply_channel(cb, a);
            glyph_pixels[idx + 3] = a;
        }

        // Compose the transform: first translate to glyph position, then apply the node transform
        let glyph_offset = Transform::from_translate(gx, gy);
        let ts = transform.pre_concat(glyph_offset);

        let paint = PixmapPaint::default();
        self.pixmap
            .draw_pixmap(0, 0, glyph_pm.as_ref(), &paint, ts, self.clip_mask.as_ref());
    }

    /// Blit a color (RGBA) glyph onto the pixmap.
    ///
    /// Uses a temporary pixmap + `draw_pixmap()` so that the full transform
    /// (including rotation/skew) is applied by tiny-skia.
    #[allow(clippy::too_many_arguments)]
    fn blit_color_glyph(
        &mut self,
        rgba_data: &[u8],
        glyph_w: u32,
        glyph_h: u32,
        gx: f32,
        gy: f32,
        transform: Transform,
        _glyph_transform: Option<Transform>,
    ) {
        if glyph_w == 0 || glyph_h == 0 {
            return;
        }

        // Build a temporary pixmap from the RGBA glyph data (convert to premultiplied)
        let mut glyph_pm = match Pixmap::new(glyph_w, glyph_h) {
            Some(pm) => pm,
            None => return,
        };
        let glyph_pixels = glyph_pm.data_mut();

        let pixel_count = (glyph_w * glyph_h) as usize;
        for i in 0..pixel_count.min(rgba_data.len() / 4) {
            let src_idx = i * 4;
            let sr = rgba_data[src_idx];
            let sg = rgba_data[src_idx + 1];
            let sb = rgba_data[src_idx + 2];
            let sa = rgba_data[src_idx + 3];

            if sa == 0 {
                continue;
            }

            let dst_idx = i * 4;
            if sa == 255 {
                glyph_pixels[dst_idx] = sr;
                glyph_pixels[dst_idx + 1] = sg;
                glyph_pixels[dst_idx + 2] = sb;
                glyph_pixels[dst_idx + 3] = 255;
            } else {
                // Convert straight alpha to premultiplied
                glyph_pixels[dst_idx] = premultiply_channel(sr, sa);
                glyph_pixels[dst_idx + 1] = premultiply_channel(sg, sa);
                glyph_pixels[dst_idx + 2] = premultiply_channel(sb, sa);
                glyph_pixels[dst_idx + 3] = sa;
            }
        }

        // Compose the transform: first translate to glyph position, then apply the node transform
        let glyph_offset = Transform::from_translate(gx, gy);
        let ts = transform.pre_concat(glyph_offset);

        let paint = PixmapPaint::default();
        self.pixmap
            .draw_pixmap(0, 0, glyph_pm.as_ref(), &paint, ts, self.clip_mask.as_ref());
    }
}

// ── Premultiply rounding (#461, #473) ─────────────────────────────────────

/// Both glyph blitters premultiply a straight-alpha colour channel by the
/// pixel's alpha, and both used to do it with a bare `as u8`, which truncates.
/// That is a one-sided error — it can only ever move a channel down — so every
/// antialiased glyph edge pixel came out up to one level short.
///
/// **Which means one level DARKER, not thinner.** Only the colour channels
/// moved; `alpha` is written straight through by both blitters and was never
/// touched. A premultiplied pixel whose channels drop while its alpha holds is
/// not closer to transparent, it is closer to black:
/// `out = src_premul + dst * (255 - a) / 255`, so a lower `src_premul` darkens
/// the result over any background. Measured on opaque backgrounds, same
/// document, only the premultiply swapped: `#111` on white 230.4746 → 230.4434,
/// `#eee` on black 24.5254 → 24.4937 — darker both times. Dark text on a light
/// page therefore looked *heavier*, not thinner; "thinner" holds only for
/// light-on-dark.
///
/// **And it is not observable.** The shift is at most one level of 255 on
/// antialiased channels alone — about 0.2% of full scale, on a few percent of
/// the frame. What the fix buys is the removal of a provable one-sided bias and
/// the first pixel oracle glyph paint has ever had, not a visible difference.
///
/// `glyph_premultiply_tests.rs` is that end-to-end oracle, over a real glyph
/// edge; this module is the font-free half, and it is what covers the
/// **colour**-glyph site, which needs a COLR face no CI host is guaranteed to
/// have. (COLR is the whole of it: `draw_glyphs` builds its swash `Render` from
/// `ColorOutline` and `Outline` only, with no `Source::ColorBitmap`, so a
/// CBDT/CBLC or sbix emoji face paints **nothing** on this backend — a separate
/// pre-existing bug, filed from the #794 review, not something these fixtures
/// cover or claim to.)
///
/// The two blitters are tested through their own private methods rather than
/// through `draw_glyphs`, for the same reason: a synthetic mask is the only way
/// to choose the alphas, and choosing them is the whole point. A real edge's
/// alphas are whatever the font and the hinter produced, and about half of them
/// sit where truncation and rounding agree — the fixed-point trap.
///
/// # Which fixture kills which mutant
///
/// Measured, one run each, against the five fixtures here plus the one in
/// `glyph_premultiply_tests.rs`, and re-run in full after the oracle's face was
/// pinned (which changes the margins, not the verdicts):
///
/// | mutation | killed by |
/// |---|---|
/// | the helper truncates (#461's bug) | all six |
/// | the helper uses `+128` | all six — but the oracle only by 9 of 6 822 samples; the exhaustive fixture here is what guarantees it |
/// | the helper `ceil`s | all six |
/// | the helper uses `+126` | all six |
/// | the helper uses `(ca + 127) >> 8` (the classic fast premultiply) | all six |
/// | the **mask site** alone re-truncates, helper intact | `the_mask_glyph_premultiply_rounds`, `a_translucent_brush_rounds_too`, and the pixel oracle |
/// | the mask site writes `alpha = premultiply_channel(a, a)` | the same two, and the pixel oracle's `invalid` arm |
/// | the **colour site** alone re-truncates, helper intact | `the_colour_glyph_premultiply_rounds` **only** |
/// | the colour site's `sa == 255` branch halves every channel | `the_colour_glyph_premultiply_rounds` **only**, and only since `every_alpha` reached 255 |
///
/// The last two rows are the reason this module exists: no document fixture
/// paints a COLR glyph, so either of those regressing is invisible to every
/// other test in the workspace, the pixel oracle included. The last row is also
/// the third fixed point this work turned up — see `every_alpha`.
///
/// One mutation is **equivalent** and cannot be killed by anything, so a future
/// run should not chase it: `premultiply_channel` is symmetric in its two
/// arguments, so swapping them changes no value.
#[cfg(test)]
mod premultiply_tests {
    use super::*;
    use tiny_skia::PremultipliedColorU8;

    /// The straight-alpha brush these fixtures use. None of the three channels
    /// is `0` or `255`: at either of those `c * a / 255` is an integer for
    /// every `a`, truncation and rounding agree, and the fixture would sit on
    /// the fixed point and pass against the bug.
    ///
    /// All three are also **coprime to 255**, which is a second fixed point and
    /// a less obvious one. `c * a mod 255` only ever takes multiples of
    /// `gcd(c, 255)`, so a channel like `200` (`gcd = 5`) reaches just 51 of the
    /// 255 possible fractions and can never produce one of `127/255` — the
    /// single fraction at which round-half-up and round-to-nearest disagree.
    /// A fixture built on `200, 150, 100` is blind to that mutant by
    /// construction; measured, and the reason these are `199, 151, 101`.
    const CR: u8 = 199;
    const CG: u8 = 151;
    const CB: u8 = 101;

    /// What the sites did before #461: `(c as f32 * (a as f32 / 255.0)) as u8`.
    fn truncating_premultiply(c: u8, alpha: u8) -> u8 {
        let af = alpha as f32 / 255.0;
        (c as f32 * af) as u8
    }

    /// Round-to-nearest, arrived at a **different way** from the implementation
    /// — `f64` and `.round()` rather than integer `+127` — so that the blitter
    /// fixtures below compare against an independent value.
    ///
    /// Asserting them against `premultiply_channel` instead makes them pin only
    /// *"the site calls the helper"*: measured, a `ceil` mutation of the helper
    /// left both of them green, because both sides of the comparison moved
    /// together. The helper's own correctness is pinned exhaustively by
    /// `rounding_never_breaks_the_premultiplied_invariant`; these pin the sites.
    fn rounded_premultiply(c: u8, alpha: u8) -> u8 {
        (c as f64 * alpha as f64 / 255.0).round() as u8
    }

    /// Step 2 of #461: *prove* rather than argue that tiny-skia still accepts
    /// the rounded values. The domain is 256x256, so it is checked entire
    /// rather than sampled.
    ///
    /// `PremultipliedColorU8::from_rgba` is tiny-skia's own validation — it
    /// answers `None` for any channel above its alpha — so this asserts against
    /// the library's rule, not against a restatement of it.
    #[test]
    fn rounding_never_breaks_the_premultiplied_invariant() {
        let mut discriminating = 0usize;
        for alpha in 0..=255u8 {
            for c in 0..=255u8 {
                let v = premultiply_channel(c, alpha);

                assert!(
                    v <= alpha,
                    "premultiplied channel {v} exceeds its alpha {alpha} (c = {c})"
                );
                assert!(
                    PremultipliedColorU8::from_rgba(v, v, v, alpha).is_some(),
                    "tiny-skia rejects the premultiplied pixel ({v}, {alpha}) for c = {c}"
                );

                // Rounded to nearest: at most half a level from the exact value.
                let exact = c as f64 * alpha as f64 / 255.0;
                assert!(
                    (v as f64 - exact).abs() <= 0.5,
                    "premultiply_channel({c}, {alpha}) = {v}, exact = {exact}"
                );

                if v != truncating_premultiply(c, alpha) {
                    discriminating += 1;
                }
            }
        }

        // Positive control. Without it a `premultiply_channel` that truncated
        // would still satisfy every assertion above except the 0.5 bound, and a
        // future rewrite of that bound into something looser would go green on
        // an empty claim. 31 770 of the 65 536 cases separate the two rules.
        assert!(
            discriminating > 30_000,
            "only {discriminating} of 65536 (c, alpha) pairs separate rounding from \
             truncation — the fixture is no longer discriminating"
        );
    }

    /// Where the helper stands against the two rounding spellings this file
    /// already had, measured rather than assumed — the first draft of this test
    /// asserted it agreed with both and was wrong about one of them.
    ///
    /// `draw_image`'s `(x as f32 * af + 0.5) as u8`: identical, all 65 536
    /// pairs. So that site could be folded onto the helper with no pixel
    /// changing, which this PR deliberately does not do — it is correct today,
    /// and changing correct code is how a text-rendering PR grows a second,
    /// unreviewed surface.
    ///
    /// `blit_rgba`'s `(x * a + 128) / 255`: **not** identical — and the reason
    /// is not the one it looks like. It is tempting to call `+128`
    /// "round-half-up", but this domain contains **no ties at all** (a tie needs
    /// `2 * c * alpha` to be an odd multiple of 255), so round-half-up and
    /// round-to-nearest are the same function here and `+128` would have to
    /// equal `+127`. What `+128` really does is move the rounding *threshold*
    /// down by `1/510`: it rounds up at a fraction of `127/255 = 0.498`, one
    /// level too eagerly, at the 128 pairs where `c * alpha ≡ 127 (mod 255)`.
    /// Error `0.502`, in the opposite direction to #461's truncation. A
    /// sub-level imprecision on an image path with no coverage, not the glyph
    /// bug, so it is recorded here rather than changed.
    ///
    /// Either way this pins #473's framing: the glyph sites were the only
    /// premultiplies in the file that had dropped the rounding term altogether.
    #[test]
    fn the_helper_agrees_with_the_float_spelling_and_not_with_plus_128() {
        let mut plus_128_differs = 0usize;
        for alpha in 0..=255u8 {
            for c in 0..=255u8 {
                let helper = premultiply_channel(c, alpha);

                let draw_image_spelling = {
                    let af = alpha as f32 / 255.0;
                    (c as f32 * af + 0.5) as u8
                };
                assert_eq!(
                    helper, draw_image_spelling,
                    "integer and float rounding disagree at (c = {c}, alpha = {alpha})"
                );

                let blit_rgba_spelling = ((c as u16 * alpha as u16 + 128) / 255) as u8;
                if helper != blit_rgba_spelling {
                    plus_128_differs += 1;
                    assert_eq!(
                        c as u16 * alpha as u16 % 255,
                        127,
                        "+128 differs somewhere other than a 127/255 fraction, at \
                         (c = {c}, alpha = {alpha})"
                    );
                    assert_eq!(
                        blit_rgba_spelling,
                        helper + 1,
                        "+128 differs by something other than rounding up, at \
                         (c = {c}, alpha = {alpha})"
                    );
                }
            }
        }
        assert_eq!(
            plus_128_differs, 128,
            "the +128 spelling's disagreement with round-to-nearest changed"
        );
    }

    /// Every alpha a mask can carry, in one row, so no alpha the rounding could
    /// be wrong at is left out — and so the fixture cannot sit on a fixed point
    /// by accident.
    ///
    /// **`255` is in the range even though rounding cannot be wrong there**, and
    /// that is the point: `blit_color_glyph` has a separate `sa == 255` branch
    /// that copies the channels straight through and never reaches the helper,
    /// so `255` covers a *branch* rather than an arithmetic case. It was
    /// `1..=254` in the first round, and a mutant that halved every channel
    /// inside that branch survived all six fixtures (#794 review, mutant J).
    /// The pixel oracle cannot cover it either — it skips fully opaque pixels by
    /// construction, since they separate no rounding rule.
    fn every_alpha() -> Vec<u8> {
        (1..=255u8).collect()
    }

    fn painted_row(painter: &TinySkiaPainter, len: usize) -> Vec<[u8; 4]> {
        let d = painter.pixels();
        (0..len)
            .map(|x| [d[x * 4], d[x * 4 + 1], d[x * 4 + 2], d[x * 4 + 3]])
            .collect()
    }

    /// Site 1: the mask path — an ordinary (non-colour) glyph combined with the
    /// brush colour. Every glyph of Latin text on the software backend takes it.
    #[test]
    fn the_mask_glyph_premultiply_rounds() {
        let mask = every_alpha();
        let mut painter = TinySkiaPainter::new(mask.len() as u32, 1);
        painter.blit_alpha_mask(
            &mask,
            mask.len() as u32,
            1,
            0.0,
            0.0,
            CR,
            CG,
            CB,
            255,
            Transform::identity(),
            None,
        );

        let row = painted_row(&painter, mask.len());
        let mut moved = 0usize;
        for (i, &alpha) in mask.iter().enumerate() {
            let px = row[i];
            assert_eq!(px[3], alpha, "alpha changed at column {i}");
            for (ch, &c) in [CR, CG, CB].iter().enumerate() {
                assert_eq!(
                    px[ch],
                    rounded_premultiply(c, alpha),
                    "channel {ch} at alpha {alpha}: truncation gives {}, rounding gives {}",
                    truncating_premultiply(c, alpha),
                    rounded_premultiply(c, alpha)
                );
                if rounded_premultiply(c, alpha) != truncating_premultiply(c, alpha) {
                    moved += 1;
                }
            }
        }

        // Positive control: this fixture would pass unchanged against the
        // truncating code if no alpha it sampled separated the two rules.
        // 381 of the 765 channel samples do (alpha 255 is not one of them: it
        // agrees, which is the point of including it — see `every_alpha`).
        assert!(
            moved > 300,
            "only {moved} of {} channel samples separate rounding from truncation",
            mask.len() * 3
        );
    }

    /// Site 2: the **COLR** path, which carries its own straight-alpha RGBA and
    /// ignores the brush. Not the embedded-bitmap path, which does not exist:
    /// `draw_glyphs` asks swash for `ColorOutline` and `Outline` and never
    /// `ColorBitmap`, so a CBDT/CBLC or sbix face paints nothing at all here
    /// (measured in the #794 review, filed separately).
    ///
    /// It is reachable from a document fixture on a host that happens to have a
    /// COLR face installed, and unreachable on one that does not — which is the
    /// same thing as saying no fixture may depend on it. Hence driving the
    /// blitter directly.
    ///
    /// This is the **only** thing in the workspace that kills a regression of
    /// either branch of this method, the `sa == 255` fast path included; see
    /// `every_alpha`.
    #[test]
    fn the_colour_glyph_premultiply_rounds() {
        let alphas = every_alpha();
        let mut rgba = Vec::with_capacity(alphas.len() * 4);
        for &a in &alphas {
            rgba.extend_from_slice(&[CR, CG, CB, a]);
        }

        let mut painter = TinySkiaPainter::new(alphas.len() as u32, 1);
        painter.blit_color_glyph(
            &rgba,
            alphas.len() as u32,
            1,
            0.0,
            0.0,
            Transform::identity(),
            None,
        );

        let row = painted_row(&painter, alphas.len());
        let mut moved = 0usize;
        for (i, &alpha) in alphas.iter().enumerate() {
            let px = row[i];
            assert_eq!(px[3], alpha, "alpha changed at column {i}");
            for (ch, &c) in [CR, CG, CB].iter().enumerate() {
                assert_eq!(
                    px[ch],
                    rounded_premultiply(c, alpha),
                    "channel {ch} at alpha {alpha}: truncation gives {}, rounding gives {}",
                    truncating_premultiply(c, alpha),
                    rounded_premultiply(c, alpha)
                );
                if rounded_premultiply(c, alpha) != truncating_premultiply(c, alpha) {
                    moved += 1;
                }
            }
        }
        // Positive control, as above: 381 of the 765 channel samples separate
        // the two rules on this path too.
        assert!(
            moved > 300,
            "only {moved} of {} channel samples separate rounding from truncation",
            alphas.len() * 3
        );
    }

    /// The brush's own alpha is folded into the mask before the premultiply, so
    /// a translucent brush is a second, independent way to reach a partial
    /// alpha — and the one that is not the fixed point `ca == 255` the two
    /// fixtures above sit on.
    ///
    /// **The `(coverage * CA + 127) / 255` below is a deliberate second copy of
    /// the combine in `blit_alpha_mask`, not a self-reference.** This fixture
    /// owns it; the src line it mirrors is a different line of code, so a
    /// mutation there fails the `px[3]` assertion. (Only the *channel*
    /// assertions go through `rounded_premultiply`, which is derived
    /// independently for the reason on that function.)
    ///
    /// **What it deliberately does not assert:** the combine rounds, and then
    /// the channel is rounded against that already-rounded alpha, so relative
    /// to the exact `c * coverage * ca / 255²` a channel can be about a level
    /// out. That double rounding is pre-existing and untouched by #461 — this
    /// fixture asserts against the rounded `a`, which is what the premultiply is
    /// actually given.
    #[test]
    fn a_translucent_brush_rounds_too() {
        let mask = every_alpha();
        const CA: u8 = 137;
        let mut painter = TinySkiaPainter::new(mask.len() as u32, 1);
        painter.blit_alpha_mask(
            &mask,
            mask.len() as u32,
            1,
            0.0,
            0.0,
            CR,
            CG,
            CB,
            CA,
            Transform::identity(),
            None,
        );

        let row = painted_row(&painter, mask.len());
        for (i, &coverage) in mask.iter().enumerate() {
            let alpha = ((coverage as u16 * CA as u16 + 127) / 255) as u8;
            if alpha == 0 {
                continue;
            }
            let px = row[i];
            assert_eq!(px[3], alpha, "combined alpha at column {i}");
            for (ch, &c) in [CR, CG, CB].iter().enumerate() {
                assert_eq!(
                    px[ch],
                    rounded_premultiply(c, alpha),
                    "channel {ch} at coverage {coverage} (combined alpha {alpha})"
                );
            }
        }
    }
}
