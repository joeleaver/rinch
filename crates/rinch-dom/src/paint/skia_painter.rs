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

// ── Device-space rectangles ───────────────────────────────────────────────

/// A half-open rectangle of whole device pixels, `[x0, x1) x [y0, y1)`.
///
/// The painter's bookkeeping unit for *where pixels may have been written*:
/// a clip mask's non-zero area, the part of a layer anything was drawn into.
/// It is always a conservative over-estimate — every rule that uses one is
/// "outside this rect nothing changed", never the converse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeviceRect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl DeviceRect {
    const EMPTY: DeviceRect = DeviceRect {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    fn full(w: u32, h: u32) -> Self {
        DeviceRect {
            x0: 0,
            y0: 0,
            x1: w,
            y1: h,
        }
    }

    fn is_empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    fn area(self) -> u64 {
        if self.is_empty() {
            0
        } else {
            (self.x1 - self.x0) as u64 * (self.y1 - self.y0) as u64
        }
    }

    fn intersect(self, o: DeviceRect) -> DeviceRect {
        let r = DeviceRect {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        };
        if r.is_empty() { Self::EMPTY } else { r }
    }

    fn union(self, o: DeviceRect) -> DeviceRect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        DeviceRect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    /// The device pixels a local-space rect can touch under `ts`, padded by
    /// `pad` device pixels and clamped to a `w x h` surface. The padding is
    /// what covers anti-aliasing and nearest-neighbour rounding at the edges;
    /// it is applied in floating point before the cast, because `as` on a
    /// float saturates and adding to the result of one can overflow.
    ///
    /// A transform that produces a non-finite corner answers the whole
    /// surface: a guess narrower than the truth is the one wrong answer.
    #[allow(clippy::too_many_arguments)]
    fn from_local(
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        ts: Transform,
        pad: f32,
        w: u32,
        h: u32,
    ) -> DeviceRect {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for (px, py) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            let dx = ts.sx * px + ts.kx * py + ts.tx;
            let dy = ts.ky * px + ts.sy * py + ts.ty;
            min_x = min_x.min(dx);
            min_y = min_y.min(dy);
            max_x = max_x.max(dx);
            max_y = max_y.max(dy);
        }
        if !(min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite()) {
            return DeviceRect::full(w, h);
        }
        let c = |v: f32, limit: u32| -> u32 { (v as f64).clamp(0.0, limit as f64) as u32 };
        let r = DeviceRect {
            x0: c((min_x - pad).floor(), w),
            y0: c((min_y - pad).floor(), h),
            x1: c((max_x + pad).ceil(), w),
            y1: c((max_y + pad).ceil(), h),
        };
        if r.is_empty() { DeviceRect::EMPTY } else { r }
    }
}

/// The pixels `shape`, drawn through `ts` onto a `w` x `h` surface, covers
/// completely: anti-aliased coverage exactly 255 on each. `EMPTY` when there
/// is no such rect or the question cannot be answered cheaply.
///
/// Answers only for a `Rect` or `RoundedRect` under a transform that keeps it
/// axis-aligned (no rotation or skew); anything else is `EMPTY`, which only
/// costs the work it would have saved. A pixel `i` spans `[i, i + 1)`, so the
/// fully covered columns of an edge pair `a..b` are `ceil(a)..floor(b)`; one
/// more pixel is given up on each side as margin against the rasteriser's
/// fixed-point edges — except at an edge lying on or past the surface's own,
/// where there is no pixel beyond to be partly covered. A rounded rect
/// answers for its rect inset by its largest radius on every side, which lies
/// inside the rounded shape. Debug builds check every answer this is trusted
/// with against the rasteriser ([`debug_assert_full`]).
fn full_coverage_rect(shape: &PaintShape, ts: Transform, w: u32, h: u32) -> DeviceRect {
    if ts.kx != 0.0 || ts.ky != 0.0 {
        return DeviceRect::EMPTY;
    }
    let (rect, inset) = match shape {
        PaintShape::Rect(rect) => (*rect, 0.0),
        PaintShape::RoundedRect(rr) => {
            let rad = rr.radii();
            let inset = rad
                .top_left
                .max(rad.top_right)
                .max(rad.bottom_right)
                .max(rad.bottom_left);
            (rr.rect(), inset)
        }
        _ => return DeviceRect::EMPTY,
    };
    let (x0, x1) = (rect.x0.min(rect.x1) + inset, rect.x0.max(rect.x1) - inset);
    let (y0, y1) = (rect.y0.min(rect.y1) + inset, rect.y0.max(rect.y1) - inset);
    let (sx, sy) = (ts.sx as f64, ts.sy as f64);
    let (tx, ty) = (ts.tx as f64, ts.ty as f64);
    let (dx0, dx1) = (sx * x0 + tx, sx * x1 + tx);
    let (dy0, dy1) = (sy * y0 + ty, sy * y1 + ty);
    let (dx0, dx1) = (dx0.min(dx1), dx0.max(dx1));
    let (dy0, dy1) = (dy0.min(dy1), dy0.max(dy1));
    if !(dx0.is_finite() && dx1.is_finite() && dy0.is_finite() && dy1.is_finite()) {
        return DeviceRect::EMPTY;
    }
    let lo = |v: f64| if v <= 0.0 { 0.0 } else { v.ceil() + 1.0 };
    let hi = |v: f64, limit: u32| {
        if v >= limit as f64 {
            limit as f64
        } else {
            v.floor() - 1.0
        }
    };
    let (ix0, ix1) = (lo(dx0), hi(dx1, w));
    let (iy0, iy1) = (lo(dy0), hi(dy1, h));
    if !(ix0 < ix1 && iy0 < iy1) {
        return DeviceRect::EMPTY;
    }
    DeviceRect {
        x0: ix0 as u32,
        y0: iy0 as u32,
        x1: ix1 as u32,
        y1: iy1 as u32,
    }
}

/// Debug builds check a [`full_coverage_rect`] answer the painter is about to
/// rely on against the mask itself: every byte of `r` must be 255. Called with
/// a scratch fill of the path for a clip that is skipped, and with the
/// enclosing mask for an intersection that is skipped — so every debug test
/// in the workspace is an oracle for the claim. Nothing pooled or counted is
/// touched.
#[cfg(debug_assertions)]
fn debug_assert_full(mask: &Mask, r: DeviceRect, what: &str) {
    let stride = mask.width() as usize;
    let data = mask.data();
    for y in r.y0 as usize..r.y1 as usize {
        let row = &data[y * stride + r.x0 as usize..y * stride + r.x1 as usize];
        if let Some(i) = row.iter().position(|&c| c != 255) {
            panic!(
                "push_clip: {what} is not fully covered over {r:?}: coverage {} at ({}, {y})",
                row[i],
                r.x0 as usize + i
            );
        }
    }
}

/// Whether `inner` lies inside `outer` (an empty `inner` lies inside anything).
fn device_rect_within(inner: DeviceRect, outer: DeviceRect) -> bool {
    inner.is_empty()
        || (outer.x0 <= inner.x0
            && inner.x1 <= outer.x1
            && outer.y0 <= inner.y0
            && inner.y1 <= outer.y1)
}

/// Anti-aliased coverage can reach the pixel past a geometric edge, and a
/// nearest-sampled pixmap can round one pixel either way; two pixels covers
/// both with a pixel to spare. Bookkeeping only — no pixel depends on it.
const DEVICE_PAD: f32 = 2.0;

// ── Clip masks ────────────────────────────────────────────────────────────

/// A clip mask plus the part of it that can be non-zero.
///
/// tiny-skia takes a mask only at the size of the pixmap it masks, so a mask
/// is always surface-sized; what `bounds` buys is that nothing ever *touches*
/// more of it than the clip covers. Creating one costs nothing (it comes
/// zeroed out of the pool), filling it costs its path, intersecting it with
/// its parent costs its bounds, and returning it to the pool costs zeroing its
/// bounds again. Before, every one of those was a whole-surface pass — the
/// allocation's zeroing included — per clip push (card K24, #F3 of the paint
/// audit).
struct ClipMask {
    mask: Mask,
    /// Outside this rect every byte of `mask` is zero.
    bounds: DeviceRect,
    /// Inside this rect every byte of `mask` is 255 — a conservative
    /// under-estimate, `EMPTY` whenever unknown (#907). A clip whose own
    /// non-zero area lies inside it needs no intersection: `m * 255` rounds
    /// back to `m`.
    full: DeviceRect,
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
    Clip { previous_mask: Option<ClipMask> },
    /// An opacity/blend layer — content drawn to a temporary pixmap.
    ///
    /// The clip mask in force at the push is left in place for the layer's
    /// content (balanced pushes and pops inside the layer restore it before
    /// the matching pop), so nothing about it is saved here.
    Opacity {
        parent_pixmap: Pixmap,
        /// The parent's written-area bookkeeping, restored on pop.
        parent_touched: Option<DeviceRect>,
        opacity: f32,
        /// How the layer is composited back: source-over, or `Plus` for a
        /// [`BlendMode::Plus`] layer (a blurred `text-shadow`'s kernel taps,
        /// #980). `Saturation` is composited source-over, as it always was.
        blend: tiny_skia::BlendMode,
    },
}

// ── Glyph cache ───────────────────────────────────────────────────────────

/// One rasterised glyph, as swash produced it for one font, size, hinting
/// setting and variation instance.
///
/// **No sub-pixel position is in the key, because none is in the image.**
/// This painter asks swash for a glyph at the origin (no `Render::offset`)
/// and places it by translating the finished image with nearest-neighbour
/// sampling, so a glyph at `x = 10.25` and one at `x = 10.75` are the same
/// coverage bytes drawn one sample apart. Caching per sub-pixel bucket would
/// store identical images several times over. If glyphs are ever rasterised
/// at their fractional offset (a quality change: better spacing, different
/// pixels), that offset — bucketed — has to join [`GlyphRunKey`], and
/// `skia_painter_oracle_tests` is what fails when it does not.
///
/// The synthesis a run carries is not in the key either, for the same
/// reason: `draw_glyphs` has never applied `glyph_transform` (faux italic)
/// on this backend, and Parley's faux bold reaches no painter.
struct CachedGlyph {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    /// `true`: `data` is straight-alpha RGBA and the brush is ignored (a COLR
    /// glyph). `false`: `data` is one coverage byte per pixel.
    color: bool,
    data: Vec<u8>,
}

/// Everything about a glyph run that changes a glyph's pixels, except the
/// glyph id.
struct GlyphRunKey {
    /// `Blob::id()` of the font file's bytes: unique per blob for the life of
    /// the process, so a font dropped and another loaded can never alias.
    font_id: u64,
    font_index: u32,
    /// `f32::to_bits` of the physical font size. Exact, not bucketed: two
    /// sizes one ulp apart rasterise differently often enough to matter to a
    /// pixel oracle, and real documents use a handful of sizes.
    size_bits: u32,
    hint: bool,
    coords: Box<[i16]>,
}

struct GlyphRun {
    key: GlyphRunKey,
    /// `None` is a cached *empty* glyph (a space): swash rendered nothing.
    glyphs: std::collections::HashMap<u32, Option<CachedGlyph>>,
}

/// The rasterised-glyph cache, bounded by an approximate byte budget.
///
/// Runs are found by a linear scan (a document uses a few dozen font/size
/// combinations at most, and comparing one is five integer compares plus an
/// usually empty slice), with a remembered last hit because consecutive runs
/// almost always share one. The bound is **clear-on-threshold**: when the
/// accounted bytes exceed [`GLYPH_CACHE_BUDGET`], or the run count
/// [`GLYPH_CACHE_MAX_RUNS`], everything is dropped and the next frame
/// re-rasterises what it draws. An LRU would keep the warm set warmer across
/// that one frame; for a working set that fits the budget — any real UI — the
/// threshold is never reached and the difference never arises.
///
/// The accounting is an estimate: each glyph counts its image bytes plus
/// [`GLYPH_ENTRY_OVERHEAD`] for the hash-map slot, the `CachedGlyph` and the
/// hash map's spare capacity. Measured with a counting allocator on about
/// 24 700 small (9-13px) glyphs: the real heap was the image bytes plus about
/// 118 bytes per entry, so a flat 32 under-counted it 2x, 96 still 1.15x, and
/// 128 over-counts by about 6% (the conservative side). Large glyphs are
/// dominated by their image bytes either way.
#[derive(Default)]
struct GlyphCache {
    runs: Vec<GlyphRun>,
    last: usize,
    bytes: usize,
    /// One swash cache key per font file, so swash's own scaler cache
    /// (outlines, hinting state) is hit across calls. `FontRef::from_index`
    /// mints a fresh key every time, which made every `draw_glyphs` call a
    /// miss there. Cleared with the glyphs: a new font file always opens a
    /// new run, so this map never outgrows [`GLYPH_CACHE_MAX_RUNS`] entries
    /// before the run cap clears both.
    font_keys: std::collections::HashMap<(u64, u32), swash::CacheKey>,
}

/// Accounted bytes before the cache is cleared.
const GLYPH_CACHE_BUDGET: usize = 16 * 1024 * 1024;
/// Per-glyph bookkeeping charged on top of the image bytes: the hash-map
/// slot (`u32` key + `Option<CachedGlyph>`), its share of the map's spare
/// capacity, and the image allocation's rounding. See [`GlyphCache`].
const GLYPH_ENTRY_OVERHEAD: usize = 128;
/// Distinct (font, size, hinting, variation) runs before the cache is cleared.
const GLYPH_CACHE_MAX_RUNS: usize = 512;

impl GlyphCache {
    fn run_index(
        &mut self,
        font_id: u64,
        font_index: u32,
        size: f32,
        hint: bool,
        coords: &[i16],
    ) -> usize {
        let size_bits = size.to_bits();
        let matches = |k: &GlyphRunKey| {
            k.font_id == font_id
                && k.font_index == font_index
                && k.size_bits == size_bits
                && k.hint == hint
                && *k.coords == *coords
        };
        if self.runs.get(self.last).is_some_and(|r| matches(&r.key)) {
            return self.last;
        }
        if let Some(i) = self.runs.iter().position(|r| matches(&r.key)) {
            self.last = i;
            return i;
        }
        if self.runs.len() >= GLYPH_CACHE_MAX_RUNS {
            self.clear();
        }
        self.runs.push(GlyphRun {
            key: GlyphRunKey {
                font_id,
                font_index,
                size_bits,
                hint,
                coords: coords.into(),
            },
            glyphs: std::collections::HashMap::new(),
        });
        self.last = self.runs.len() - 1;
        self.last
    }

    fn clear(&mut self) {
        self.runs.clear();
        self.last = 0;
        self.bytes = 0;
        self.font_keys.clear();
    }
}

// ── Painter statistics ────────────────────────────────────────────────────

/// What the software painter did, for the perf counters.
///
/// The painter has no document to count into, so it counts here and the
/// shell folds a frame's worth into the document's
/// [`PerfCounters`](crate::perf::PerfCounters) with
/// [`TinySkiaPainter::take_stats`]. See `docs/src/guide/performance.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkiaPainterStats {
    /// Glyphs drawn from the rasterised-glyph cache.
    pub glyph_cache_hits: u64,
    /// Glyphs swash had to rasterise (then cached).
    pub glyph_cache_misses: u64,
    /// Clip masks pushed (`push_clip` calls).
    pub clip_masks: u64,
    /// Mask pixels those pushes worked over: the clip's bounds, filled and
    /// intersected. A surface-sized clip costs the surface; a small one its
    /// own area; one that fully covers the clip enclosing it, nothing; one lying
    /// wholly inside the enclosing clip's fully covered area, its fill alone
    /// (#907).
    pub clip_mask_px: u64,
    /// Opacity layers opened (`push_layer` calls that allocated a layer).
    pub layers: u64,
    /// Layer pixels composited back onto their parent.
    pub layer_px: u64,
    /// Surface-sized buffers (masks and layer pixmaps) newly allocated, as
    /// opposed to reused from the pool. Zero in a steady state.
    pub surface_allocs: u64,
    /// Images premultiplied at draw time. Zero for a cached `<img>` or
    /// `background-image` after its first software paint; a live frame
    /// source (`RenderSurface`, video, `GameViewport`) is premultiplied on
    /// every draw — unless it was submitted opaque, and then never.
    pub image_premultiplies: u64,
    /// Opaque images copied straight into the surface row by row, with no
    /// premultiply and no `draw_pixmap` (#361 — a software `GameViewport` or
    /// video frame): a draw with no rotation or skew, a positive scale, destination edges on whole pixels, and a clip that is fully on or fully off wherever the frame lands. Scaled
    /// draws count too; only scale 1 is a plain `memcpy` per row.
    pub opaque_image_copies: u64,
    /// Pooled surface-sized buffers released by
    /// [`TinySkiaPainter::end_frame`] because no recent frame needed that
    /// many at once.
    pub surface_trims: u64,
}

impl SkiaPainterStats {
    /// Add these counts to the current frame of `perf`.
    pub fn add_to(&self, perf: &crate::perf::PerfCounters) {
        use crate::perf::Counter;
        perf.add(Counter::GlyphCacheHits, self.glyph_cache_hits);
        perf.add(Counter::GlyphCacheMisses, self.glyph_cache_misses);
        perf.add(Counter::ClipMasks, self.clip_masks);
        perf.add(Counter::ClipMaskPx, self.clip_mask_px);
        perf.add(Counter::PaintLayers, self.layers);
        perf.add(Counter::LayerPx, self.layer_px);
        perf.add(Counter::PaintSurfaceAllocs, self.surface_allocs);
        perf.add(Counter::ImagePremultiplies, self.image_premultiplies);
        perf.add(Counter::OpaqueImageCopies, self.opaque_image_copies);
        perf.add(Counter::PaintSurfaceTrims, self.surface_trims);
    }
}

/// The `[a, b)` columns of row `y` of `dst` that lie inside `inner` (a
/// clip's fully covered rect, already intersected with `dst`); `(dst.x0,
/// dst.x0)` when the row misses it.
fn inner_span(inner: DeviceRect, y: u32, dst: DeviceRect) -> (u32, u32) {
    if inner.is_empty() || y < inner.y0 || y >= inner.y1 {
        (dst.x0, dst.x0)
    } else {
        (inner.x0, inner.x1)
    }
}

// ── TinySkiaPainter ───────────────────────────────────────────────────────

/// Pooled surface-sized buffers kept per painter. A frame reaches this depth
/// only with that many clips or layers open at once.
const POOL_MAX: usize = 8;

/// How many recent frames' peak buffer use the pool is sized to. A buffer
/// beyond the largest peak in this window is released at the end of a frame,
/// so an app that once nested deeply does not keep the extra surfaces for
/// the rest of the session, while one that alternates between two depths
/// does not reallocate every other frame.
const POOL_TRIM_WINDOW: usize = 8;

/// Live and peak counts of one kind of pooled buffer, and the recent peaks
/// the pool is trimmed to.
#[derive(Default)]
struct PoolUse {
    /// Buffers of this kind currently out of the pool (in the clip stack or
    /// the layer stack).
    live: usize,
    /// The most `live` has been since the last `end_frame`.
    peak: usize,
    /// The peaks of the last [`POOL_TRIM_WINDOW`] frames, newest last.
    recent: std::collections::VecDeque<usize>,
}

impl PoolUse {
    fn acquired(&mut self) {
        self.live += 1;
        self.peak = self.peak.max(self.live);
    }

    fn released(&mut self) {
        self.live = self.live.saturating_sub(1);
    }

    /// Close a frame and answer how many buffers the pool should keep.
    fn end_frame(&mut self) -> usize {
        if self.recent.len() == POOL_TRIM_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(self.peak);
        self.peak = self.live;
        self.recent.iter().copied().max().unwrap_or(0)
    }
}

/// Software rendering backend using tiny-skia.
///
/// Rasterizes directly to an RGBA pixel buffer. All drawing operations are
/// immediate — there is no command recording step.
pub struct TinySkiaPainter {
    pixmap: Pixmap,
    /// Current clip mask (intersection of all active clip layers).
    clip_mask: Option<ClipMask>,
    /// Stack of saved layer states.
    layer_stack: Vec<LayerState>,
    /// Reusable swash scale context for glyph rasterization.
    scale_context: swash::scale::ScaleContext,
    /// Rasterised glyphs, reused across calls and frames.
    glyph_cache: GlyphCache,
    /// Scratch RGBA buffer a glyph is coloured into before it is drawn.
    glyph_scratch: Vec<u8>,
    /// All-zero surface-sized masks ready for reuse.
    mask_pool: Vec<Mask>,
    /// Fully transparent surface-sized pixmaps ready for reuse as layers.
    layer_pool: Vec<Pixmap>,
    /// While a layer is open: the device pixels drawn into it so far. `None`
    /// outside any layer, where nobody needs to know.
    touched: Option<DeviceRect>,
    mask_use: PoolUse,
    layer_use: PoolUse,
    stats: SkiaPainterStats,
    /// Paint the way this painter did before its caches and pools existed.
    /// For the pixel-diff oracle only.
    reference_mode: bool,
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
            glyph_cache: GlyphCache::default(),
            glyph_scratch: Vec::new(),
            mask_pool: Vec::new(),
            layer_pool: Vec::new(),
            touched: None,
            mask_use: PoolUse::default(),
            layer_use: PoolUse::default(),
            stats: SkiaPainterStats::default(),
            reference_mode: false,
        }
    }

    /// Turn every cache and pool off and paint exactly as the painter did
    /// before they existed: every glyph rasterised on every draw, every clip
    /// and layer a fresh surface-sized buffer composited whole, every image
    /// premultiplied per draw.
    ///
    /// Exists so a test can paint the same document both ways in one process
    /// and demand identical pixels (`skia_painter_oracle_tests`). Not for
    /// application use; it only makes things slower.
    #[doc(hidden)]
    pub fn set_reference_mode(&mut self, on: bool) {
        self.reference_mode = on;
    }

    /// The counts since the last [`take_stats`](Self::take_stats).
    pub fn stats(&self) -> SkiaPainterStats {
        self.stats
    }

    /// The counts since the last call, zeroing them.
    pub fn take_stats(&mut self) -> SkiaPainterStats {
        std::mem::take(&mut self.stats)
    }

    /// How many bytes of glyph images the cache holds.
    pub fn glyph_cache_bytes(&self) -> usize {
        self.glyph_cache.bytes
    }

    /// Drop every cached glyph image.
    pub fn clear_glyph_cache(&mut self) {
        self.glyph_cache.clear();
    }

    /// Close a frame: release pooled masks and layer pixmaps beyond the
    /// largest number any of the last [`POOL_TRIM_WINDOW`] frames had open at
    /// once. The shell calls this after each software paint; a painter nobody
    /// calls it on keeps what it pooled (bounded by [`POOL_MAX`] of each)
    /// until it is resized or dropped.
    pub fn end_frame(&mut self) {
        let keep_masks = self.mask_use.end_frame();
        let keep_layers = self.layer_use.end_frame();
        let trimmed = self.mask_pool.len().saturating_sub(keep_masks)
            + self.layer_pool.len().saturating_sub(keep_layers);
        self.mask_pool.truncate(keep_masks);
        self.layer_pool.truncate(keep_layers);
        self.stats.surface_trims += trimmed as u64;
    }

    /// How many (masks, layer pixmaps) the pool holds right now.
    pub fn pooled_buffers(&self) -> (usize, usize) {
        (self.mask_pool.len(), self.layer_pool.len())
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
            self.touched = None;
            self.mask_pool.clear();
            self.layer_pool.clear();
            self.mask_use.live = 0;
            self.layer_use.live = 0;
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
        self.touch_all();
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
        self.touch_all();
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
        self.touch_all();
        self.pixmap.fill(tiny_skia::Color::WHITE);
    }

    /// Fill the entire surface with a fully transparent background.
    pub fn fill_transparent(&mut self) {
        self.touch_all();
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
        self.touch_all();
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
        self.touch_all();

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

    /// Copy an opaque image straight into the surface, if the draw is one a
    /// copy reproduces **byte for byte**: no rotation or skew, a positive
    /// scale, destination edges on whole pixels, and no clip that is partial
    /// anywhere the image lands. Answers `false`, having drawn nothing, for any
    /// draw it cannot reproduce, which `draw_pixmap` then takes.
    ///
    /// Why it is exact: `draw_pixmap` fills the image's transformed rect
    /// without anti-aliasing (so, with whole-pixel edges, exactly the pixels
    /// `x0..x1 × y0..y1`), samples each with `FilterQuality::Nearest` in the
    /// high-precision pipeline — the pixel centre through the inverse
    /// transform, `(x + 0.5) * isx + ((y + 0.5) * ikx + itx)`, clamped to
    /// `[0, w - 1ulp]` and truncated — and blends it source-over, which for an
    /// alpha-255 pixel is the pixel. The same arithmetic, done once per column
    /// and once per row, is below; `skia_painter_oracle_tests` checks it
    /// against `draw_pixmap` over a spread of scales and offsets. At scale 1
    /// it is one `copy_from_slice` per row — for a 1080p `GameViewport` frame
    /// about seven times cheaper than premultiply plus `draw_pixmap` (#361).
    fn copy_opaque_image(&mut self, image: &PaintImage<'_>, ts: Transform) -> bool {
        if ts.kx != 0.0 || ts.ky != 0.0 || ts.sx <= 0.0 || ts.sy <= 0.0 {
            return false;
        }
        let (iw, ih) = (image.width, image.height);
        let row = iw as usize * 4;
        if image.data.len() < row * ih as usize {
            return false;
        }
        let (sw, sh) = (self.pixmap.width(), self.pixmap.height());
        // `draw_pixmap` tiles a surface this big, translating per tile; the
        // copy does not model that.
        if sw > 8191 || sh > 8191 {
            return false;
        }
        // The destination rect's edges, as the fill computes them.
        let whole = |v: f32| {
            let r = v.round();
            ((v - r).abs() <= 1e-3).then_some(r as i64)
        };
        let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
            whole(ts.tx),
            whole(ts.ty),
            whole(iw as f32 * ts.sx + ts.tx),
            whole(ih as f32 * ts.sy + ts.ty),
        ) else {
            return false;
        };
        // The inverse `draw_pixmap` samples through (tiny-skia inverts the
        // pattern's transform, which is `ts`). No skew, so its cross terms are
        // (signed) zeros and each output column's source column is the same
        // on every row.
        let Some(inv) = ts.invert() else {
            return false;
        };
        if inv.kx != 0.0 || inv.ky != 0.0 {
            return false;
        }
        let (cx0, cy0) = (x0.max(0), y0.max(0));
        let (cx1, cy1) = (x1.min(sw as i64), y1.min(sh as i64));
        if cx0 >= cx1 || cy0 >= cy1 {
            return true; // wholly off the surface: nothing to draw
        }
        let mut dst = DeviceRect {
            x0: cx0 as u32,
            y0: cy0 as u32,
            x1: cx1 as u32,
            y1: cy1 as u32,
        };
        // Under a clip, a pixel is written exactly when its mask byte says so,
        // and that is reproducible only where the byte is 0 or 255. Inside the
        // mask's fully covered rect every byte is 255; the band between it and
        // the mask's bounds (a damage rect's one-pixel margin, say) is read,
        // and a single partial byte there declines the copy.
        let mut inner = dst;
        if let Some(m) = &self.clip_mask {
            dst = dst.intersect(m.bounds);
            if dst.is_empty() {
                return true; // wholly clipped away
            }
            inner = dst.intersect(m.full);
            let md = m.mask.data();
            for y in dst.y0..dst.y1 {
                let row_bytes = &md[(y * sw) as usize..][..sw as usize];
                let (a, b) = inner_span(inner, y, dst);
                let partial = |c: &u8| *c != 0 && *c != 255;
                if row_bytes[dst.x0 as usize..a as usize].iter().any(partial)
                    || row_bytes[b as usize..dst.x1 as usize].iter().any(partial)
                {
                    return false; // a partial clip: masked coverage is draw_pixmap's
                }
            }
        }
        self.stats.opaque_image_copies += 1;
        self.touch(dst);

        // Each destination column's source byte offset. At scale 1 that is
        // `x - x0`; otherwise `draw_pixmap`'s nearest sample, below.
        //
        // `ulp_sub` in tiny-skia's `gather_ix`: the last representable value
        // below the exclusive limit.
        let below = |v: f32| f32::from_bits(v.to_bits() - 1);
        let (wmax, hmax) = (below(iw as f32), below(ih as f32));
        let unscaled = ts.sx == 1.0 && ts.sy == 1.0;
        let columns: Vec<usize> = (dst.x0..dst.x1)
            .map(|x| {
                if unscaled {
                    return (x as i64 - x0) as usize * 4;
                }
                // `mad(r, sx, mad(g, kx, tx))` with `g * kx` a zero.
                let u = (x as f32 + 0.5) * inv.sx + inv.tx;
                u.max(0.0).min(wmax) as usize * 4
            })
            .collect();
        let stride = sw as usize * 4;
        let mask = self.clip_mask.as_ref().map(|m| m.mask.data());
        let data = self.pixmap.data_mut();
        for y in dst.y0..dst.y1 {
            let src_y = if unscaled {
                (y as i64 - y0) as usize
            } else {
                // `mad(r, ky, mad(g, sy, ty))` with `r * ky` a zero.
                let v = (y as f32 + 0.5) * inv.sy + inv.ty;
                v.max(0.0).min(hmax) as usize
            };
            let src_row = &image.data[src_y * row..][..row];
            let out_row = &mut data[y as usize * stride..][..stride];
            let (a, b) = if mask.is_some() {
                inner_span(inner, y, dst)
            } else {
                (dst.x0, dst.x1)
            };
            // The fully covered span: one copy at scale 1.
            if a < b {
                let (ca, cb) = ((a - dst.x0) as usize, (b - dst.x0) as usize);
                let out = &mut out_row[a as usize * 4..b as usize * 4];
                if unscaled {
                    out.copy_from_slice(&src_row[columns[ca]..columns[ca] + out.len()]);
                } else {
                    for (px, &c) in out.as_chunks_mut::<4>().0.iter_mut().zip(&columns[ca..cb]) {
                        px.copy_from_slice(&src_row[c..c + 4]);
                    }
                }
            }
            // The band outside it, where the mask is 0 or 255 (checked above).
            if let Some(md) = mask {
                let mrow = &md[(y * sw) as usize..][..sw as usize];
                for x in (dst.x0..a).chain(b.max(dst.x0)..dst.x1) {
                    if mrow[x as usize] == 255 {
                        let c = columns[(x - dst.x0) as usize];
                        out_row[x as usize * 4..][..4].copy_from_slice(&src_row[c..c + 4]);
                    }
                }
            }
        }
        true
    }

    // ── Bookkeeping ────────────────────────────────────────────────────

    fn surface_rect(&self) -> DeviceRect {
        DeviceRect::full(self.pixmap.width(), self.pixmap.height())
    }

    /// Record that `r` may have been written, if a layer is open. Narrowed to
    /// the clip in force, since a masked draw writes nothing outside it.
    #[inline]
    fn touch(&mut self, r: DeviceRect) {
        if let Some(t) = self.touched.as_mut() {
            let r = match &self.clip_mask {
                Some(m) => r.intersect(m.bounds),
                None => r,
            };
            *t = t.union(r);
        }
    }

    fn touch_all(&mut self) {
        let full = self.surface_rect();
        if let Some(t) = self.touched.as_mut() {
            *t = full;
        }
    }

    fn touch_local(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, ts: Transform) {
        if self.touched.is_none() {
            return;
        }
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let r = DeviceRect::from_local(x0, y0, x1, y1, ts, DEVICE_PAD, w, h);
        self.touch(r);
    }

    /// An all-zero surface-sized mask: from the pool when one is there.
    fn acquire_mask(&mut self) -> Mask {
        self.mask_use.acquired();
        if !self.reference_mode
            && let Some(m) = self.mask_pool.pop()
        {
            return m;
        }
        self.stats.surface_allocs += 1;
        Mask::new(self.pixmap.width(), self.pixmap.height()).expect("failed to create clip mask")
    }

    /// Hand a mask back: zero what it may hold, then pool it.
    fn release_mask(&mut self, mut m: ClipMask) {
        self.mask_use.released();
        if self.reference_mode
            || self.mask_pool.len() >= POOL_MAX
            || m.mask.width() != self.pixmap.width()
            || m.mask.height() != self.pixmap.height()
        {
            return;
        }
        let b = m.bounds;
        if !b.is_empty() {
            let stride = m.mask.width() as usize;
            let data = m.mask.data_mut();
            for y in b.y0 as usize..b.y1 as usize {
                data[y * stride + b.x0 as usize..y * stride + b.x1 as usize].fill(0);
            }
        }
        self.mask_pool.push(m.mask);
    }

    /// A copy of `m`: its bounds copied into a pooled mask.
    fn clone_mask(&mut self, m: &ClipMask) -> ClipMask {
        if self.reference_mode {
            self.stats.surface_allocs += 1;
            return ClipMask {
                mask: m.mask.clone(),
                bounds: m.bounds,
                full: m.full,
            };
        }
        let mut mask = self.acquire_mask();
        let b = m.bounds;
        if !b.is_empty() {
            let stride = mask.width() as usize;
            let src = m.mask.data();
            let dst = mask.data_mut();
            for y in b.y0 as usize..b.y1 as usize {
                let row = y * stride;
                dst[row + b.x0 as usize..row + b.x1 as usize]
                    .copy_from_slice(&src[row + b.x0 as usize..row + b.x1 as usize]);
            }
        }
        ClipMask {
            mask,
            bounds: b,
            full: m.full,
        }
    }

    /// A transparent surface-sized pixmap for a layer: from the pool when one
    /// is there.
    fn acquire_layer(&mut self) -> Pixmap {
        self.layer_use.acquired();
        if !self.reference_mode
            && let Some(p) = self.layer_pool.pop()
        {
            return p;
        }
        self.stats.surface_allocs += 1;
        Pixmap::new(self.pixmap.width(), self.pixmap.height())
            .expect("failed to create layer pixmap")
    }

    /// Hand a layer pixmap back: clear what was drawn into it, then pool it.
    fn release_layer(&mut self, mut p: Pixmap, drawn: DeviceRect) {
        self.layer_use.released();
        if self.reference_mode
            || self.layer_pool.len() >= POOL_MAX
            || p.width() != self.pixmap.width()
            || p.height() != self.pixmap.height()
        {
            return;
        }
        if !drawn.is_empty() {
            let stride = p.width() as usize * 4;
            let data = p.data_mut();
            for y in drawn.y0 as usize..drawn.y1 as usize {
                data[y * stride + drawn.x0 as usize * 4..y * stride + drawn.x1 as usize * 4]
                    .fill(0);
            }
        }
        self.layer_pool.push(p);
    }
}

impl TinySkiaPainter {
    /// Draw what follows into a fresh (or pooled, already transparent) layer
    /// until the matching pop composites it back at `opacity` with `blend`.
    /// The clip mask in force stays in force: the layer's content is clipped
    /// exactly as it would be drawn directly.
    fn open_layer(&mut self, opacity: f32, blend: tiny_skia::BlendMode) {
        self.stats.layers += 1;
        let layer = self.acquire_layer();
        let parent_pixmap = std::mem::replace(&mut self.pixmap, layer);
        let parent_touched = self.touched.replace(DeviceRect::EMPTY);

        self.layer_stack.push(LayerState::Opacity {
            parent_pixmap,
            parent_touched,
            opacity,
            blend,
        });
    }
}

impl Painter for TinySkiaPainter {
    fn reset(&mut self) {
        self.pixmap.fill(tiny_skia::Color::TRANSPARENT);
        // Anything still open is dropped, not pooled: no longer live.
        self.clip_mask = None;
        self.layer_stack.clear();
        self.touched = None;
        self.mask_use.live = 0;
        self.layer_use.live = 0;
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
        self.touch_local(
            bounds.left(),
            bounds.top(),
            bounds.right(),
            bounds.bottom(),
            ts,
        );
        let mask = self.clip_mask.as_ref().map(|m| &m.mask);
        self.pixmap.fill_path(&path, &paint, fill_rule, ts, mask);
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
        if self.touched.is_some() {
            // How far a stroke can reach past its path, in path space: half
            // the width, times the miter limit where a miter join can spike
            // (a square cap reaches `sqrt 2` half-widths, which `1.5` covers).
            let reach = sk_stroke.width * 0.5 * sk_stroke.miter_limit.max(1.5);
            let b = path.bounds();
            self.touch_local(
                b.left() - reach,
                b.top() - reach,
                b.right() + reach,
                b.bottom() + reach,
                ts,
            );
        }
        let mask = self.clip_mask.as_ref().map(|m| &m.mask);
        self.pixmap.stroke_path(&path, &paint, &sk_stroke, ts, mask);
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
        let Some(mut font_ref) = swash::FontRef::from_index(font_data, font.index as usize) else {
            return;
        };
        let font_id = font.data.id();
        if !self.reference_mode {
            font_ref.key = *self
                .glyph_cache
                .font_keys
                .entry((font_id, font.index))
                .or_insert(font_ref.key);
        }

        // Resolve brush color (only solid colors for now)
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => AlphaColor::new([0.0, 0.0, 0.0, 1.0]), // fallback to black
        };
        let rgba = color.to_rgba8();
        let (cr, cg, cb, ca) = (rgba.r, rgba.g, rgba.b, rgba.a);

        let ts = affine_to_transform(transform);
        let glyph_ts = glyph_transform.map(affine_to_transform);

        let render = swash::scale::Render::new(&[
            swash::scale::Source::ColorOutline(0),
            swash::scale::Source::Outline,
        ]);
        let rasterise = |s: &mut swash::scale::Scaler<'_>, id: u32| -> Option<CachedGlyph> {
            let image = render.render(s, id as u16)?;
            if image.placement.width == 0 || image.placement.height == 0 {
                return None;
            }
            let (color, data) = match image.content {
                swash::scale::image::Content::Mask => (false, image.data),
                swash::scale::image::Content::Color => (true, image.data),
                swash::scale::image::Content::SubpixelMask => (
                    false,
                    image
                        .data
                        .chunks(4)
                        .map(|px| px.get(3).copied().unwrap_or(0))
                        .collect(),
                ),
            };
            Some(CachedGlyph {
                left: image.placement.left,
                top: image.placement.top,
                width: image.placement.width,
                height: image.placement.height,
                color,
                data,
            })
        };

        let run = if self.reference_mode {
            None
        } else {
            Some(self.glyph_cache.run_index(
                font_id,
                font.index,
                font_size,
                hint,
                normalized_coords,
            ))
        };

        // Built only when something misses: a fully cached run never pays
        // for a scaler (which is where swash sets up hinting state).
        let needs_scaler = match run {
            Some(run) => {
                let cached = &self.glyph_cache.runs[run].glyphs;
                glyphs.iter().any(|g| !cached.contains_key(&g.id))
            }
            None => true,
        };
        let mut scaler = needs_scaler.then(|| {
            self.scale_context
                .builder(font_ref)
                .size(font_size)
                .hint(hint)
                .normalized_coords(normalized_coords)
                .build()
        });

        for glyph in glyphs {
            // Look the glyph up, rasterising it on a miss.
            let owned;
            let cached: Option<&CachedGlyph> = match run {
                Some(run) => {
                    let cache = &mut self.glyph_cache;
                    if !cache.runs[run].glyphs.contains_key(&glyph.id) {
                        self.stats.glyph_cache_misses += 1;
                        let g = scaler.as_mut().and_then(|s| rasterise(s, glyph.id));
                        cache.bytes +=
                            g.as_ref().map_or(0, |g| g.data.len()) + GLYPH_ENTRY_OVERHEAD;
                        cache.runs[run].glyphs.insert(glyph.id, g);
                    } else {
                        self.stats.glyph_cache_hits += 1;
                    }
                    cache.runs[run].glyphs[&glyph.id].as_ref()
                }
                None => {
                    self.stats.glyph_cache_misses += 1;
                    owned = scaler.as_mut().and_then(|s| rasterise(s, glyph.id));
                    owned.as_ref()
                }
            };
            let Some(g) = cached else {
                continue;
            };
            let gx = glyph.x + g.left as f32;
            let gy = glyph.y - g.top as f32;
            if self.touched.is_some() {
                let (w, h) = (self.pixmap.width(), self.pixmap.height());
                let r = DeviceRect::from_local(
                    gx,
                    gy,
                    gx + g.width as f32,
                    gy + g.height as f32,
                    ts,
                    DEVICE_PAD,
                    w,
                    h,
                );
                if let Some(t) = self.touched.as_mut() {
                    let r = match &self.clip_mask {
                        Some(m) => r.intersect(m.bounds),
                        None => r,
                    };
                    *t = t.union(r);
                }
            }
            let mask = self.clip_mask.as_ref().map(|m| &m.mask);
            if g.color {
                blit_color_glyph_into(
                    &mut self.pixmap,
                    mask,
                    &mut self.glyph_scratch,
                    &g.data,
                    g.width,
                    g.height,
                    gx,
                    gy,
                    ts,
                );
            } else {
                blit_alpha_mask_into(
                    &mut self.pixmap,
                    mask,
                    &mut self.glyph_scratch,
                    &g.data,
                    g.width,
                    g.height,
                    gx,
                    gy,
                    [cr, cg, cb, ca],
                    ts,
                );
            }
        }
        let _ = glyph_ts;

        if self.glyph_cache.bytes > GLYPH_CACHE_BUDGET {
            self.glyph_cache.clear();
        }
    }

    fn draw_image(&mut self, image: &PaintImage<'_>, transform: Affine) {
        if image.width == 0 || image.height == 0 {
            return;
        }

        let ts = affine_to_transform(transform);
        if image.opaque && !self.reference_mode && self.copy_opaque_image(image, ts) {
            return;
        }

        // The premultiplied pixels: the image cache's copy when the image came
        // from one (computed on its first software paint and kept), the pixels
        // themselves when every one is opaque, otherwise made here, per draw —
        // a live frame source has no cache to keep it in.
        let cached = if self.reference_mode {
            None
        } else if image.opaque {
            Some(image.data)
        } else {
            image.decoded.map(|d| d.premultiplied())
        };
        let owned;
        let premul: &[u8] = match cached {
            Some(p) => p,
            None => {
                self.stats.image_premultiplies += 1;
                owned = if self.reference_mode {
                    reference_premultiply(image.data)
                } else {
                    crate::image_cache::premultiply_rgba(image.data)
                };
                &owned
            }
        };

        let Some(src) = PixmapRef::from_bytes(premul, image.width, image.height) else {
            return;
        };

        self.touch_local(0.0, 0.0, image.width as f32, image.height as f32, ts);
        let paint = PixmapPaint::default();
        let mask = self.clip_mask.as_ref().map(|m| &m.mask);
        self.pixmap.draw_pixmap(0, 0, src, &paint, ts, mask);
    }

    fn draw_alpha_mask(
        &mut self,
        mask: &[u8],
        width: u32,
        height: u32,
        color: peniko::color::AlphaColor<peniko::color::Srgb>,
        transform: Affine,
    ) {
        if width == 0 || height == 0 || mask.len() != (width * height) as usize {
            return;
        }
        let c = transform.as_coeffs();
        // Not a cache, so taken in reference mode too (whose masks are
        // fresh allocations rather than pooled ones).
        if c[0] == 1.0
            && c[1] == 0.0
            && c[2] == 0.0
            && c[3] == 1.0
            && c[4].fract() == 0.0
            && c[5].fract() == 0.0
        {
            // On whole pixels: the coverage goes into a pooled surface mask,
            // intersected with the clip in force, and the colour is filled
            // through it — tiny-skia's solid-colour mask fill, with no RGBA
            // intermediate at all.
            let (sw, sh) = (self.pixmap.width() as i64, self.pixmap.height() as i64);
            let (ox, oy, w, h) = (c[4] as i64, c[5] as i64, width as i64, height as i64);
            let (x0, y0) = (ox.max(0), oy.max(0));
            let (x1, y1) = ((ox + w).min(sw), (oy + h).min(sh));
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            let mut m = self.acquire_mask();
            {
                let stride = sw as usize;
                let clip = self.clip_mask.as_ref().map(|c| c.mask.data());
                let data = m.data_mut();
                let span = (x1 - x0) as usize;
                for y in y0..y1 {
                    let src_at = ((y - oy) * w + (x0 - ox)) as usize;
                    let src = &mask[src_at..src_at + span];
                    let at = y as usize * stride + x0 as usize;
                    let dst = &mut data[at..at + span];
                    match clip {
                        None => dst.copy_from_slice(src),
                        Some(clip) => {
                            for ((d, &s), &k) in dst.iter_mut().zip(src).zip(&clip[at..at + span]) {
                                *d = ((s as u32 * k as u32 + 127) / 255) as u8;
                            }
                        }
                    }
                }
            }
            let bounds = DeviceRect {
                x0: x0 as u32,
                y0: y0 as u32,
                x1: x1 as u32,
                y1: y1 as u32,
            };
            self.touch(bounds);
            if let Some(rect) =
                tiny_skia::Rect::from_ltrb(x0 as f32, y0 as f32, x1 as f32, y1 as f32)
            {
                let paint = Paint {
                    shader: tiny_skia::Shader::SolidColor(to_skia_color(color)),
                    anti_alias: false,
                    ..Paint::default()
                };
                self.pixmap
                    .fill_rect(rect, &paint, Transform::identity(), Some(&m));
            }
            self.release_mask(ClipMask {
                mask: m,
                bounds,
                full: DeviceRect::EMPTY,
            });
            return;
        }
        // Premultiplied straight from the coverage: one pass, no straight-
        // alpha intermediate to premultiply again.
        let [r, g, b, a] = color.to_rgba8().to_u8_array();
        let a = a as u32;
        let premul = |c: u8, m: u32| ((c as u32 * m + 127) / 255) as u8;
        let mut data = vec![0_u8; mask.len() * 4];
        for (px, &m) in data.chunks_exact_mut(4).zip(mask) {
            if m != 0 {
                let alpha = (m as u32 * a + 127) / 255;
                px.copy_from_slice(&[
                    premul(r, alpha),
                    premul(g, alpha),
                    premul(b, alpha),
                    alpha as u8,
                ]);
            }
        }
        let Some(src) = PixmapRef::from_bytes(&data, width, height) else {
            return;
        };
        let ts = affine_to_transform(transform);
        self.touch_local(0.0, 0.0, width as f32, height as f32, ts);
        let paint = PixmapPaint::default();
        let clip = self.clip_mask.as_ref().map(|m| &m.mask);
        self.pixmap.draw_pixmap(0, 0, src, &paint, ts, clip);
    }

    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape) {
        self.stats.clip_masks += 1;
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
            // passes a `Rect` or a `RoundedRect` — except `build_pixels`' damage
            // clip, a `BezPath` of non-empty whole-pixel rects inside the
            // surface (`DamageRegion::clip_path`) — `clip_shape` builds its rect
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
            self.clip_mask = previous_mask.as_ref().map(|m| self.clone_mask(m));
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
            // not the absence of one: nothing behind it should show, and an
            // all-zero mask is exactly that.
            //
            // This is also the branch that used to leak the enclosing clip, per
            // the rule above; the all-zero mask is stricter than `previous_mask`
            // by construction, so it settles both halves at once.
            let mask = self.acquire_mask();
            self.clip_mask = Some(ClipMask {
                mask,
                bounds: DeviceRect::EMPTY,
                full: DeviceRect::EMPTY,
            });
            self.layer_stack.push(LayerState::Clip { previous_mask });
            return;
        }

        let w = self.pixmap.width();
        let h = self.pixmap.height();
        let ts = affine_to_transform(transform);
        let fill_rule = to_fill_rule(fill);

        // Where this shape's own coverage is exactly 255 (#907); `EMPTY` in
        // reference mode, which therefore never takes either shortcut below.
        let own_full = if self.reference_mode {
            DeviceRect::EMPTY
        } else {
            full_coverage_rect(shape, ts, w, h)
        };

        // A clip that covers everything the enclosing clip lets through
        // changes nothing (#907). Where the enclosing mask can be non-zero
        // (`prev.bounds`), this shape's coverage is exactly 255, and
        // `(255 * p + 127) / 255 == p` for every byte `p`: the intersection
        // *is* the enclosing mask, byte for byte. So nothing is filled,
        // multiplied, copied or pooled, and the pop restores nothing —
        // [`LayerState::Noop`], with the inherited mask put back where
        // `.take()` found it (the rule at the top of this function).
        //
        // This is the partial repaint's common case: the damage clip that
        // `RinchApp::build_pixels` pushes first is a caret or a row, and every
        // scroller or `overflow: hidden` box it sits inside used to fill and
        // intersect its whole box to clip a region it contains — a 320 px
        // caret blink filled 607 320 mask pixels in an editor.
        if let Some(prev) = &previous_mask
            && !own_full.is_empty()
            && device_rect_within(prev.bounds, own_full)
        {
            #[cfg(debug_assertions)]
            {
                let mut scratch = Mask::new(w, h).expect("scratch mask");
                scratch.fill_path(&path, fill_rule, true, ts);
                debug_assert_full(&scratch, prev.bounds, "a skipped clip's own shape");
            }
            self.clip_mask = previous_mask;
            self.layer_stack.push(LayerState::Noop);
            return;
        }

        let mut mask = self.acquire_mask();
        mask.fill_path(&path, fill_rule, true, ts);

        // Where the new mask can be non-zero: the path's device-space bounds.
        //
        // `acquire_mask` hands back a mask of zeroes and `fill_path` writes only
        // inside the path, so every byte outside those bounds is still zero,
        // and zero times whatever the parent mask holds is zero. Multiplying
        // those bytes is arithmetic whose answer is already in the buffer.
        // Running the loop over the whole mask regardless is what made a clip
        // cost the surface rather than the box: at 1080×2460 that is 2.66
        // million multiply-and-divides per nested clip, and the library screen
        // pushes seventeen clips a frame — every `overflow: hidden` box, every
        // scroller, every rounded thumbnail — for about 35ms of a 90ms frame on
        // the moto g stylus 5G. See card K24.
        //
        // The bounds are padded because `fill_path` is called with
        // anti-aliasing on and its coverage can spill into the pixel outside
        // the geometric edge.
        //
        // If the bounds cannot be mapped into device space the whole surface
        // is used. Narrowing on a guess would be the one way to get this
        // wrong — outside the region it walks, the parent mask is never
        // applied, and content paints straight through the enclosing clip. The
        // same bounds decide what `release_mask` zeroes before pooling the
        // mask, so a narrow guess there would also leave coverage behind for
        // the next clip to inherit.
        let path_rect = if self.reference_mode {
            DeviceRect::full(w, h)
        } else {
            DeviceRect::from_local(
                bounds.left(),
                bounds.top(),
                bounds.right(),
                bounds.bottom(),
                ts,
                DEVICE_PAD,
                w,
                h,
            )
        };
        let mut new_bounds = path_rect;
        let mut new_full = own_full;
        let mut worked = path_rect.area();
        if let Some(ref prev) = previous_mask
            && !prev.full.is_empty()
            && device_rect_within(path_rect, prev.full)
        {
            // The converse of the skip above (#907): everywhere this fill can
            // be non-zero the enclosing mask is 255, so the product is the
            // fill itself and the multiply is skipped. A damage rect around a
            // whole scroller is this shape.
            #[cfg(debug_assertions)]
            debug_assert_full(
                &prev.mask,
                path_rect,
                "a skipped intersection's enclosing mask",
            );
            // `own_full` lies inside `path_rect`, so inside `prev.full`:
            // it needs no narrowing.
        } else if let Some(ref prev) = previous_mask {
            // In reference mode, the whole-surface walk the painter used to
            // make before card K24 narrowed it; otherwise the path's bounds.
            let walk = path_rect;
            let stride = w as usize;
            let mask_data = mask.data_mut();
            let prev_data = prev.mask.data();
            for y in walk.y0 as usize..walk.y1 as usize {
                let row = y * stride;
                let m = &mut mask_data[row + walk.x0 as usize..row + walk.x1 as usize];
                let p = &prev_data[row + walk.x0 as usize..row + walk.x1 as usize];
                for (m, p) in m.iter_mut().zip(p.iter()) {
                    *m = ((*m as u16 * *p as u16 + 127) / 255) as u8;
                }
            }
            worked += walk.area();
            // Outside the parent's bounds the parent is zero, so the product is.
            new_bounds = new_bounds.intersect(prev.bounds);
            // 255 only where both are.
            new_full = new_full.intersect(prev.full);
        }
        self.stats.clip_mask_px += worked;

        self.clip_mask = Some(ClipMask {
            mask,
            bounds: new_bounds,
            full: new_full,
        });
        self.layer_stack.push(LayerState::Clip { previous_mask });
    }

    fn push_layer(
        &mut self,
        blend: BlendMode,
        opacity: f32,
        _transform: Affine,
        _bounds: &PaintShape,
    ) {
        // A `Plus` layer at opacity 1 still adds rather than covers, so it
        // never takes the fast path below.
        if !matches!(blend, BlendMode::Plus) && (opacity - 1.0).abs() < f32::EPSILON {
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
            // Saving nothing is both cheaper and the more honest statement —
            // see [`LayerState::Noop`].
            self.layer_stack.push(LayerState::Noop);
            return;
        }

        let blend = match blend {
            BlendMode::Plus => tiny_skia::BlendMode::Plus,
            BlendMode::Normal | BlendMode::Saturation => tiny_skia::BlendMode::SourceOver,
        };
        self.open_layer(opacity, blend);
    }

    fn push_isolated_layer(&mut self, opacity: f32, _transform: Affine, _bounds: &PaintShape) {
        // No near-opaque fast path: what is drawn inside may add (`Plus`)
        // rather than cover, which only an empty layer of its own makes right.
        self.open_layer(opacity, tiny_skia::BlendMode::SourceOver);
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
                if let Some(m) = std::mem::replace(&mut self.clip_mask, previous_mask) {
                    self.release_mask(m);
                }
            }
            LayerState::Opacity {
                mut parent_pixmap,
                parent_touched,
                opacity,
                blend,
            } => {
                // Composite the layer back onto the parent — only the part of
                // it anything was drawn into. Everywhere else the layer is
                // transparent, and `SourceOver` of transparent is the identity.
                let drawn = if self.reference_mode {
                    self.surface_rect()
                } else {
                    self.touched.unwrap_or(DeviceRect::EMPTY)
                };
                let paint = PixmapPaint {
                    opacity,
                    blend_mode: blend,
                    quality: tiny_skia::FilterQuality::Nearest,
                };
                if drawn == self.surface_rect() {
                    parent_pixmap.draw_pixmap(
                        0,
                        0,
                        self.pixmap.as_ref(),
                        &paint,
                        Transform::identity(),
                        None,
                    );
                } else if !drawn.is_empty()
                    && let Some(rect) = tiny_skia::IntRect::from_xywh(
                        drawn.x0 as i32,
                        drawn.y0 as i32,
                        drawn.x1 - drawn.x0,
                        drawn.y1 - drawn.y0,
                    )
                    && let Some(part) = self.pixmap.clone_rect(rect)
                {
                    parent_pixmap.draw_pixmap(
                        drawn.x0 as i32,
                        drawn.y0 as i32,
                        part.as_ref(),
                        &paint,
                        Transform::identity(),
                        None,
                    );
                }
                self.stats.layer_px += drawn.area();

                let layer = std::mem::replace(&mut self.pixmap, parent_pixmap);
                self.release_layer(layer, drawn);
                self.touched = parent_touched;
                // What the layer drew is now drawn into the parent.
                if let Some(t) = self.touched.as_mut() {
                    *t = t.union(drawn);
                }
            }
        }
    }

    fn append(&mut self, other: &Self) {
        self.touch_all();
        let src = other.pixmap.as_ref();
        let paint = PixmapPaint::default();
        self.pixmap
            .draw_pixmap(0, 0, src, &paint, Transform::identity(), None);
    }
}

/// `draw_image`'s premultiply as it was written before the image cache kept a
/// premultiplied copy, for reference mode only.
///
/// A deliberate second copy of [`crate::image_cache::premultiply_rgba`]: if
/// reference mode called that function, a change to its arithmetic would move
/// both sides of `skia_painter_oracle_tests` together and the oracle would
/// pass against it (measured — a `+128` rounding mutant survived until this
/// copy existed).
fn reference_premultiply(data: &[u8]) -> Vec<u8> {
    let mut premul = Vec::with_capacity(data.len());
    for chunk in data.chunks(4) {
        let (r, g, b, a) = (chunk[0], chunk[1], chunk[2], chunk[3]);
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
    premul
}

// ── Glyph blitting helpers ────────────────────────────────────────────────

/// Blit an alpha mask glyph onto `pixmap` with the given straight-alpha
/// colour.
///
/// The glyph is coloured into `scratch` (reused across glyphs, so no
/// allocation per glyph) and drawn with `draw_pixmap()`, so that the full
/// transform (including rotation/skew) is applied by tiny-skia.
#[allow(clippy::too_many_arguments)]
fn blit_alpha_mask_into(
    pixmap: &mut Pixmap,
    clip: Option<&Mask>,
    scratch: &mut Vec<u8>,
    mask_data: &[u8],
    glyph_w: u32,
    glyph_h: u32,
    gx: f32,
    gy: f32,
    [cr, cg, cb, ca]: [u8; 4],
    transform: Transform,
) {
    if glyph_w == 0 || glyph_h == 0 {
        return;
    }
    let n = (glyph_w * glyph_h) as usize;
    scratch.clear();
    scratch.resize(n * 4, 0);

    for (i, &alpha) in mask_data.iter().enumerate().take(n) {
        if alpha == 0 {
            continue;
        }
        // Combine glyph alpha with brush alpha → premultiplied RGBA
        let a = ((alpha as u16 * ca as u16 + 127) / 255) as u8;
        if a == 0 {
            continue;
        }
        let idx = i * 4;
        scratch[idx] = premultiply_channel(cr, a);
        scratch[idx + 1] = premultiply_channel(cg, a);
        scratch[idx + 2] = premultiply_channel(cb, a);
        scratch[idx + 3] = a;
    }

    let Some(src) = PixmapRef::from_bytes(scratch, glyph_w, glyph_h) else {
        return;
    };
    // Compose the transform: first translate to glyph position, then apply the node transform
    let ts = transform.pre_concat(Transform::from_translate(gx, gy));
    pixmap.draw_pixmap(0, 0, src, &PixmapPaint::default(), ts, clip);
}

/// Blit a color (RGBA) glyph onto `pixmap`, converting it to premultiplied
/// alpha in `scratch` first.
#[allow(clippy::too_many_arguments)]
fn blit_color_glyph_into(
    pixmap: &mut Pixmap,
    clip: Option<&Mask>,
    scratch: &mut Vec<u8>,
    rgba_data: &[u8],
    glyph_w: u32,
    glyph_h: u32,
    gx: f32,
    gy: f32,
    transform: Transform,
) {
    if glyph_w == 0 || glyph_h == 0 {
        return;
    }
    let pixel_count = (glyph_w * glyph_h) as usize;
    scratch.clear();
    scratch.resize(pixel_count * 4, 0);

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
            scratch[dst_idx] = sr;
            scratch[dst_idx + 1] = sg;
            scratch[dst_idx + 2] = sb;
            scratch[dst_idx + 3] = 255;
        } else {
            // Convert straight alpha to premultiplied
            scratch[dst_idx] = premultiply_channel(sr, sa);
            scratch[dst_idx + 1] = premultiply_channel(sg, sa);
            scratch[dst_idx + 2] = premultiply_channel(sb, sa);
            scratch[dst_idx + 3] = sa;
        }
    }

    let Some(src) = PixmapRef::from_bytes(scratch, glyph_w, glyph_h) else {
        return;
    };
    let ts = transform.pre_concat(Transform::from_translate(gx, gy));
    pixmap.draw_pixmap(0, 0, src, &PixmapPaint::default(), ts, clip);
}

#[cfg(test)]
impl TinySkiaPainter {
    /// Blit an alpha mask glyph onto the pixmap with the given color.
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
        let clip = self.clip_mask.as_ref().map(|m| &m.mask);
        blit_alpha_mask_into(
            &mut self.pixmap,
            clip,
            &mut self.glyph_scratch,
            mask_data,
            glyph_w,
            glyph_h,
            gx,
            gy,
            [cr, cg, cb, ca],
            transform,
        );
    }

    /// Blit a color (RGBA) glyph onto the pixmap.
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
        let clip = self.clip_mask.as_ref().map(|m| &m.mask);
        blit_color_glyph_into(
            &mut self.pixmap,
            clip,
            &mut self.glyph_scratch,
            rgba_data,
            glyph_w,
            glyph_h,
            gx,
            gy,
            transform,
        );
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
