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
            for pixel in row_data.chunks_exact_mut(4) {
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
        for chunk in premul.chunks_exact_mut(4) {
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

        let Some(path) = shape_to_path(shape) else {
            // Can't create path — push a no-op layer so pop_layer still works
            self.layer_stack.push(LayerState::Clip { previous_mask });
            return;
        };

        let bounds = path.bounds();
        if bounds.width() < 0.001 || bounds.height() < 0.001 {
            self.layer_stack.push(LayerState::Clip { previous_mask });
            return;
        }

        let w = self.pixmap.width();
        let h = self.pixmap.height();
        let mut mask = Mask::new(w, h).expect("failed to create clip mask");
        let ts = affine_to_transform(transform);
        let fill_rule = to_fill_rule(fill);
        mask.fill_path(&path, fill_rule, true, ts);

        // If there was a previous mask, intersect with it
        if let Some(ref prev) = previous_mask {
            let mask_data = mask.data_mut();
            let prev_data = prev.data();
            for (m, p) in mask_data.iter_mut().zip(prev_data.iter()) {
                *m = ((*m as u16 * *p as u16 + 127) / 255) as u8;
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
            // Fully opaque — just push a no-op clip layer to keep the stack balanced
            self.layer_stack.push(LayerState::Clip {
                previous_mask: None,
            });
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
            let af = a as f32 / 255.0;
            let idx = i * 4;
            glyph_pixels[idx] = (cr as f32 * af) as u8;
            glyph_pixels[idx + 1] = (cg as f32 * af) as u8;
            glyph_pixels[idx + 2] = (cb as f32 * af) as u8;
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
                let af = sa as f32 / 255.0;
                glyph_pixels[dst_idx] = (sr as f32 * af) as u8;
                glyph_pixels[dst_idx + 1] = (sg as f32 * af) as u8;
                glyph_pixels[dst_idx + 2] = (sb as f32 * af) as u8;
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
