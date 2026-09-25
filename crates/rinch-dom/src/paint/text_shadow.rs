//! Drawing one `text-shadow` (#980, #981).
//!
//! A shadow with no blur is one copy of the text — glyphs, straight
//! decorations and wavy underlines — in the shadow's colour, drawn straight
//! into the painter.
//!
//! A blurred shadow is a **mask**: wherever the software rasteriser is
//! compiled in (`software-renderer`, which every desktop and Android build
//! carries), the shadow's copy is rasterised **once**, opaque, into a scratch
//! pixmap covering the part of it that can be seen, its coverage is blurred by
//! a separable Gaussian of standard deviation `blur / 2`, and the result is
//! drawn as one image in the shadow's colour. Both painters are handed the same
//! image, so Vello and tiny-skia draw the same pixels; Vello has no general
//! blur of its own.
//!
//! The Vello-only build (`embed` without `software-renderer`) has no CPU
//! rasteriser, and falls back to a small **kernel of copies**
//! ([`draw_tapped`]): at most 13 copies, each in a [`BlendMode::Plus`] layer at
//! its weight inside one isolated layer, and at most [`TAP_GLYPH_BUDGET`] glyph
//! copies per paint — past the budget a shadow is drawn unblurred. The first
//! cut of #980 drew every blurred shadow that way with up to 113 copies; a page
//! of text then cost 50–170 times as much per software frame and overflowed
//! wgpu's buffer-binding limit on the GPU (review of PR #1020).
//!
//! The shadow reaches [`REACH_PER_BLUR`] times the blur radius past its
//! offset copy — three standard deviations, where Skia stops — which is what
//! `layer_bounds::text_shadow_reach` and the damage ink allow for.

use std::cell::Cell;

use peniko::Brush;
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Rect};

#[cfg(feature = "software-renderer")]
use super::blur::Blur1d;
use super::painter::{BlendMode, PaintShape, Painter};
use super::text::{TextMask, draw_shadow_copy};

/// How far a blurred `text-shadow` reaches past its offset copy, as a
/// multiple of its blur radius: three standard deviations of a Gaussian whose
/// standard deviation is half the radius.
pub(crate) const REACH_PER_BLUR: f64 = 1.5;

/// The most glyph copies the kernel-of-copies fallback draws in one paint
/// (glyphs × taps, summed over every blurred shadow). Past it a shadow is
/// drawn once, unblurred, which bounds what a page of text can hand Vello.
pub const TAP_GLYPH_BUDGET: usize = 20_000;

/// The largest mask, in pixels, the software rasteriser blurs; a bigger one
/// (a huge transformed text block — a translate-only one is cropped to what
/// can be seen) falls back to the kernel of copies.
#[cfg(feature = "software-renderer")]
const MAX_MASK_PX: usize = 16 << 20;

/// The longest side of a mask: the image goes to Vello's image atlas, whose
/// textures are bounded by wgpu's default 8192 limit.
#[cfg(feature = "software-renderer")]
const MAX_MASK_SIDE: usize = 4096;

thread_local! {
    /// Draw blurred shadows as a kernel of copies even where the software
    /// rasteriser is compiled in — tests pin the Vello-only fallback with it.
    static FORCE_TAPS: Cell<bool> = const { Cell::new(false) };
    /// Glyph copies the kernel-of-copies fallback has drawn this paint.
    static TAP_GLYPHS: Cell<usize> = const { Cell::new(0) };
    /// The pixmap blurred shadows are rasterised into, kept across paints so
    /// its glyph cache stays warm; it only ever grows, to the largest mask
    /// drawn, which is bounded by what can be seen.
    #[cfg(feature = "software-renderer")]
    static MASK_CACHE: std::cell::RefCell<MaskCache> = std::cell::RefCell::new(MaskCache::default());
    #[cfg(feature = "software-renderer")]
    static SCRATCH: std::cell::RefCell<Option<super::skia_painter::TinySkiaPainter>> =
        const { std::cell::RefCell::new(None) };
}

/// Draw blurred `text-shadow`s as a kernel of copies (the Vello-only build's
/// way) even where the software rasteriser is compiled in, on this thread,
/// until set back. Returns the previous setting. For tests.
#[doc(hidden)]
pub fn force_tapped_text_shadows(on: bool) -> bool {
    FORCE_TAPS.with(|f| f.replace(on))
}

/// Forget every blurred shadow mask kept between paints, so the next paint
/// rasterises and blurs each one again. For benchmarks and tests.
#[doc(hidden)]
pub fn clear_text_shadow_cache() {
    #[cfg(feature = "software-renderer")]
    MASK_CACHE.with(|c| *c.borrow_mut() = MaskCache::default());
}

/// Start a paint: the kernel-of-copies budget is per paint.
pub(super) fn begin_paint() {
    TAP_GLYPHS.with(|t| t.set(0));
}

/// Draw one shadow pass of `layout`: glyphs, straight decorations and — given
/// the IFC's `wavy` spans — its wavy underlines, in `color`, blurred by `blur`
/// physical px. `x`/`y` are the (already offset) physical origin; `mask` hides
/// what a `visibility: hidden` element holds (#829).
#[allow(clippy::too_many_arguments)]
pub(super) fn render_text_shadow_pass(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    color: AlphaColor<Srgb>,
    blur: f64,
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
    wavy: Option<&crate::node::InlineLayout>,
) {
    let sigma = blur / 2.0;
    // `sigma` may be NaN; that is no blur too.
    if sigma.is_nan() || sigma < 0.25 {
        let brush = Brush::Solid(color);
        draw_shadow_copy(
            painter,
            layout,
            x,
            y,
            &brush,
            css_transform,
            scale,
            mask,
            wavy,
        );
        return;
    }
    #[cfg(feature = "software-renderer")]
    if !FORCE_TAPS.with(Cell::get)
        && draw_masked(
            painter,
            layout,
            x,
            y,
            color,
            sigma,
            css_transform,
            scale,
            mask,
            wavy,
        )
    {
        return;
    }
    draw_tapped(
        painter,
        layout,
        x,
        y,
        color,
        sigma,
        css_transform,
        scale,
        mask,
        wavy,
    );
}

/// The text's own extent relative to its origin, in physical px: each line
/// from its first glyph run's start to its last one's end and from its top to
/// its bottom (ascent and descent, or the line box when that is taller),
/// grown by how far glyph ink may reach past them.
fn text_extent(layout: &parley::layout::Layout<Brush>, scale: f64) -> Rect {
    let mut r: Option<Rect> = None;
    let mut size = 0.0_f32;
    for line in layout.lines() {
        let m = line.metrics();
        let (mut x0, mut x1) = (f32::INFINITY, f32::NEG_INFINITY);
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                x0 = x0.min(run.offset());
                x1 = x1.max(run.offset() + run.advance());
                size = size.max(run.run().font_size());
            }
        }
        if x0 > x1 {
            continue;
        }
        let top = m.block_min_coord.min(m.baseline - m.ascent);
        let bottom = m.block_max_coord.max(m.baseline + m.descent);
        let line_rect = Rect::new(x0 as f64, top as f64, x1 as f64, bottom as f64);
        r = Some(r.map_or(line_rect, |r| r.union(line_rect)));
    }
    // An italic's overhang, a tall accent, a wavy underline below the
    // descent: a quarter of the largest font size.
    let ink = size as f64 * 0.25;
    let r = r.unwrap_or(Rect::ZERO).inflate(ink, ink);
    Rect::new(r.x0 * scale, r.y0 * scale, r.x1 * scale, r.y1 * scale)
}

// ── The blurred mask (software rasteriser compiled in) ─────────────────────

/// Rasterise the shadow once into a coverage mask over what can be seen of it,
/// blur the mask, and draw it as one image. `false` when the mask would be too
/// big, and the caller falls back to [`draw_tapped`].
///
/// Under a translate-only transform the mask is in device pixels, lined up with
/// the surface so the image lands on whole pixels, and cropped to what can be
/// seen (the surface, the open clips and the damage) grown by the blur's
/// reach — pixels outside it still bleed in. Under any other transform it is
/// built in the text's own (scaled) space and drawn through the transform.
#[cfg(feature = "software-renderer")]
#[allow(clippy::too_many_arguments)]
fn draw_masked(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    color: AlphaColor<Srgb>,
    sigma: f64,
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
    wavy: Option<&crate::node::InlineLayout>,
) -> bool {
    let blur = Blur1d::new(sigma, DIRECT_KERNEL_MAX_SIGMA);
    let pad = blur.reach() as f64;
    let c = css_transform.as_coeffs();
    let translate_only = c[0] == 1.0 && c[1] == 0.0 && c[2] == 0.0 && c[3] == 1.0;
    let (ox, oy, image_transform) = if translate_only {
        (x + c[4], y + c[5], Affine::IDENTITY)
    } else {
        (x, y, css_transform)
    };
    let ext = text_extent(layout, scale);
    let mut area = Rect::new(
        ox + ext.x0 - pad,
        oy + ext.y0 - pad,
        ox + ext.x1 + pad,
        oy + ext.y1 + pad,
    );
    if translate_only && let Some(visible) = super::visible_device_rect() {
        area = area.intersect(visible.inflate(pad, pad));
    }
    let area = Rect::new(
        area.x0.floor(),
        area.y0.floor(),
        area.x1.ceil(),
        area.y1.ceil(),
    );
    if !(area.width() >= 1.0 && area.height() >= 1.0) {
        return true; // nothing of it can be seen
    }
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w.saturating_mul(h) > MAX_MASK_PX || w > MAX_MASK_SIDE || h > MAX_MASK_SIDE {
        return false;
    }

    // Every call the copy would make, hashed in the mask's own space: the
    // same calls rasterise to the same coverage, wherever on the surface the
    // mask lands (a scroll by whole pixels, a repaint of an unchanged page).
    let key = {
        let mut hasher = CallHasher::default();
        draw_shadow_copy(
            &mut hasher,
            layout,
            ox - area.x0,
            oy - area.y0,
            &Brush::Solid(AlphaColor::<Srgb>::from_rgba8(255, 255, 255, 255)),
            Affine::IDENTITY,
            scale,
            mask,
            wavy,
        );
        use std::hash::Hasher;
        hasher.0.write_u64(sigma.to_bits());
        hasher.0.write_usize(w);
        hasher.0.write_usize(h);
        hasher.0.finish()
    };
    let draw_at = image_transform * Affine::translate((area.x0, area.y0));
    let hit = MASK_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let Some(entry) = c.get(key) else {
            return false;
        };
        if let Some(m) = entry {
            painter.draw_alpha_mask(m, w as u32, h as u32, color, draw_at);
        }
        true
    });
    if hit {
        return true;
    }

    // Rasterise: coverage is the alpha of an opaque copy.
    let mut coverage = vec![0.0_f32; w * h];
    let mut any = false;
    SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        let sp =
            s.get_or_insert_with(|| super::skia_painter::TinySkiaPainter::new(w as u32, h as u32));
        if (sp.width() as usize) < w || (sp.height() as usize) < h {
            sp.resize(
                (sp.width() as usize).max(w) as u32,
                (sp.height() as usize).max(h) as u32,
            );
        }
        sp.clear_rect_transparent(0, 0, w as u32, h as u32);
        let white = Brush::Solid(AlphaColor::<Srgb>::from_rgba8(255, 255, 255, 255));
        draw_shadow_copy(
            sp,
            layout,
            ox - area.x0,
            oy - area.y0,
            &white,
            Affine::IDENTITY,
            scale,
            mask,
            wavy,
        );
        let stride = sp.width() as usize;
        let px = sp.pixels();
        for (row, out) in coverage.chunks_exact_mut(w).enumerate() {
            let src = &px[row * stride * 4..(row * stride + w) * 4];
            for (o, p) in out.iter_mut().zip(src.as_chunks::<4>().0) {
                if p[3] != 0 {
                    *o = p[3] as f32 * (1.0 / 255.0);
                    any = true;
                }
            }
        }
    });
    if !any {
        MASK_CACHE.with(|c| c.borrow_mut().insert(key, None));
        return true;
    }
    blur_2d(&blur, &mut coverage, w, h);

    let mask8: Vec<u8> = coverage
        .iter()
        .map(|&c| (c * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect();
    painter.draw_alpha_mask(&mask8, w as u32, h as u32, color, draw_at);
    MASK_CACHE.with(|c| c.borrow_mut().insert(key, Some(mask8)));
    true
}

/// The most bytes of blurred masks kept between paints.
#[cfg(feature = "software-renderer")]
const MASK_CACHE_BYTES: usize = 32 << 20;

/// Blurred shadow masks kept between paints, keyed by the hash of every call
/// that rasterised them ([`CallHasher`]) and the blur and size, least
/// recently used first out past [`MASK_CACHE_BYTES`]. `None` is a mask that
/// came out empty. A page of shadowed text is rasterised and blurred once, not
/// every frame — which matters most on the GPU path, where every frame paints
/// everything.
#[cfg(feature = "software-renderer")]
#[derive(Default)]
struct MaskCache {
    entries: rustc_hash::FxHashMap<u64, (Option<Vec<u8>>, u64)>,
    bytes: usize,
    clock: u64,
}

#[cfg(feature = "software-renderer")]
impl MaskCache {
    fn get(&mut self, key: u64) -> Option<Option<&Vec<u8>>> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(&key).map(|e| {
            e.1 = clock;
            e.0.as_ref()
        })
    }

    fn insert(&mut self, key: u64, mask: Option<Vec<u8>>) {
        let size = mask.as_ref().map_or(0, Vec::len) + 64;
        if size > MASK_CACHE_BYTES {
            return;
        }
        while self.bytes + size > MASK_CACHE_BYTES {
            let Some((&old, _)) = self.entries.iter().min_by_key(|(_, e)| e.1) else {
                break;
            };
            if let Some((m, _)) = self.entries.remove(&old) {
                self.bytes -= m.map_or(0, |m| m.len()) + 64;
            }
        }
        self.clock += 1;
        if let Some((m, _)) = self.entries.insert(key, (mask, self.clock)) {
            self.bytes -= m.map_or(0, |m| m.len()) + 64;
        }
        self.bytes += size;
    }
}

/// A [`Painter`] that draws nothing and hashes every call it is handed, in
/// the terms that decide its pixels: the font file and size, every transform,
/// every glyph and its position, every stroke and its shape.
#[cfg(feature = "software-renderer")]
#[derive(Default)]
struct CallHasher(rustc_hash::FxHasher);

#[cfg(feature = "software-renderer")]
impl CallHasher {
    fn affine(&mut self, a: Affine) {
        use std::hash::Hasher;
        for c in a.as_coeffs() {
            self.0.write_u64(c.to_bits());
        }
    }

    fn shape(&mut self, shape: &PaintShape) {
        use std::hash::Hasher;
        let b = shape.bounding_box();
        for v in [b.x0, b.y0, b.x1, b.y1] {
            self.0.write_u64(v.to_bits());
        }
        if let PaintShape::BezPath(p) = shape {
            for el in p.elements() {
                for pt in [el.end_point()].into_iter().flatten() {
                    self.0.write_u64(pt.x.to_bits());
                    self.0.write_u64(pt.y.to_bits());
                }
            }
        }
    }
}

#[cfg(feature = "software-renderer")]
impl Painter for CallHasher {
    fn reset(&mut self) {}
    fn fill(&mut self, fill: peniko::Fill, transform: Affine, _: &Brush, shape: &PaintShape) {
        use std::hash::Hasher;
        self.0.write_u8(1 + fill as u8);
        self.affine(transform);
        self.shape(shape);
    }
    fn stroke(
        &mut self,
        stroke: &peniko::kurbo::Stroke,
        transform: Affine,
        _: &Brush,
        shape: &PaintShape,
    ) {
        use std::hash::Hasher;
        self.0.write_u8(3);
        self.0.write_u64(stroke.width.to_bits());
        self.affine(transform);
        self.shape(shape);
    }
    fn draw_glyphs(
        &mut self,
        font: &peniko::FontData,
        font_size: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        _: &Brush,
        hint: bool,
        normalized_coords: &[i16],
        glyphs: &[super::painter::PaintGlyph],
    ) {
        use std::hash::Hasher;
        self.0.write_u8(4);
        self.0.write_u64(font.data.id());
        self.0.write_u32(font.index);
        self.0.write_u32(font_size.to_bits());
        self.affine(transform);
        if let Some(g) = glyph_transform {
            self.affine(g);
        }
        self.0.write_u8(hint as u8);
        for c in normalized_coords {
            self.0.write_i16(*c);
        }
        self.0.write_usize(glyphs.len());
        for g in glyphs {
            self.0.write_u32(g.id);
            self.0.write_u32(g.x.to_bits());
            self.0.write_u32(g.y.to_bits());
        }
    }
    fn draw_image(&mut self, _: &super::painter::PaintImage<'_>, transform: Affine) {
        use std::hash::Hasher;
        self.0.write_u8(5);
        self.affine(transform);
    }
    fn push_clip(&mut self, _: peniko::Fill, transform: Affine, shape: &PaintShape) {
        use std::hash::Hasher;
        self.0.write_u8(6);
        self.affine(transform);
        self.shape(shape);
    }
    fn push_layer(&mut self, _: BlendMode, opacity: f32, transform: Affine, shape: &PaintShape) {
        use std::hash::Hasher;
        self.0.write_u8(7);
        self.0.write_u32(opacity.to_bits());
        self.affine(transform);
        self.shape(shape);
    }
    fn pop_layer(&mut self) {
        use std::hash::Hasher;
        self.0.write_u8(8);
    }
}

/// Past this `sigma` a text shadow's blur is three box blurs rather than a
/// sampled Gaussian kernel: the kernel costs `6 sigma` per sample, the boxes
/// a constant. The inset `box-shadow` switches at 2.5; a text shadow is
/// blurred over far more pixels, so it switches sooner.
#[cfg(feature = "software-renderer")]
const DIRECT_KERNEL_MAX_SIGMA: f64 = 1.25;

/// Blur a `w` x `h` coverage mask in place, rows then columns. A row with no
/// coverage stays empty through the row pass, so it is skipped; the column
/// pass sweeps whole rows at a time, so it reads the mask in order.
#[cfg(feature = "software-renderer")]
fn blur_2d(blur: &Blur1d, mask: &mut [f32], w: usize, h: usize) {
    let mut tmp = Vec::new();
    for row in mask.chunks_exact_mut(w) {
        if row.iter().any(|&v| v != 0.0) {
            blur.apply(row, &mut tmp);
        }
    }
    blur.apply_columns(mask, w, h, &mut tmp);
}

// ── The kernel of copies (Vello-only fallback) ─────────────────────────────

/// Draw the shadow as a small Gaussian kernel of copies — each in a
/// [`BlendMode::Plus`] layer at its weight, inside one isolated layer at the
/// shadow colour's alpha — or, once this paint has drawn
/// [`TAP_GLYPH_BUDGET`] glyph copies, as one unblurred copy.
#[allow(clippy::too_many_arguments)]
fn draw_tapped(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    color: AlphaColor<Srgb>,
    sigma: f64,
    css_transform: Affine,
    scale: f64,
    mask: Option<&TextMask>,
    wavy: Option<&crate::node::InlineLayout>,
) {
    let kernel = shadow_kernel(sigma);
    let copies = glyph_count(layout).max(1) * kernel.len();
    let within_budget = TAP_GLYPHS.with(|t| {
        let used = t.get() + copies;
        (used <= TAP_GLYPH_BUDGET).then(|| t.set(used)).is_some()
    });
    if kernel.len() <= 1 || !within_budget {
        let brush = Brush::Solid(color);
        draw_shadow_copy(
            painter,
            layout,
            x,
            y,
            &brush,
            css_transform,
            scale,
            mask,
            wavy,
        );
        return;
    }

    // Everything a copy can draw, grown by the kernel's reach, in the
    // painter's space. Vello clips a layer to its bounds.
    let ext = text_extent(layout, scale);
    let reach = 3.0 * sigma;
    let bounds = PaintShape::Rect(Rect::new(
        x + ext.x0 - reach,
        y + ext.y0 - reach,
        x + ext.x1 + reach,
        y + ext.y1 + reach,
    ));
    let opaque = Brush::Solid(color.with_alpha(1.0));
    painter.push_isolated_layer(color.components[3], css_transform, &bounds);
    for &(dx, dy, weight) in &kernel {
        painter.push_layer(BlendMode::Plus, weight, css_transform, &bounds);
        draw_shadow_copy(
            painter,
            layout,
            x + dx,
            y + dy,
            &opaque,
            css_transform,
            scale,
            mask,
            wavy,
        );
        painter.pop_layer();
    }
    painter.pop_layer();
}

/// How many glyphs `layout` draws.
fn glyph_count(layout: &parley::layout::Layout<Brush>) -> usize {
    let mut n = 0;
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                n += run.glyphs().count();
            }
        }
    }
    n
}

/// The taps of a Gaussian of standard deviation `sigma` (physical px):
/// `(dx, dy, weight)` on a grid of at most 5 x 5 points out to `3 sigma`,
/// kept inside the disc — 13 taps at most. The step is `1.5 sigma`, or one
/// pixel for a small blur. The weights are quantised to multiples of 1/255
/// (the unit a layer's opacity reaches an 8-bit surface in) and sum to
/// exactly 1, so a solid interior stays solid; a tap whose weight rounds to
/// nothing is dropped, and what rounding leaves over goes to the centre.
///
/// A `sigma` below a quarter pixel gives the single tap `(0, 0, 1.0)`.
pub(super) fn shadow_kernel(sigma: f64) -> Vec<(f64, f64, f32)> {
    if sigma.is_nan() || sigma < 0.25 || !sigma.is_finite() {
        return vec![(0.0, 0.0, 1.0)];
    }
    let radius = 3.0 * sigma;
    let step = (radius / 2.0).max(1.0);
    let n = ((radius / step).floor() as i32).min(2);
    let mut taps: Vec<(f64, f64, f64)> = Vec::new();
    for j in -n..=n {
        for i in -n..=n {
            let (dx, dy) = (i as f64 * step, j as f64 * step);
            let d2 = dx * dx + dy * dy;
            if d2 <= radius * radius + 1e-9 {
                taps.push((dx, dy, (-d2 / (2.0 * sigma * sigma)).exp()));
            }
        }
    }
    let total: f64 = taps.iter().map(|t| t.2).sum();
    let mut units: Vec<i64> = taps
        .iter()
        .map(|t| (t.2 / total * 255.0).round() as i64)
        .collect();
    let centre = taps
        .iter()
        .position(|t| t.0 == 0.0 && t.1 == 0.0)
        .expect("the grid holds its centre");
    let left_over = 255 - units.iter().sum::<i64>();
    units[centre] = (units[centre] + left_over).max(1);
    taps.iter()
        .zip(units)
        .filter(|&(_, u)| u > 0)
        .map(|(t, u)| (t.0, t.1, u as f32 / 255.0))
        .collect()
}
