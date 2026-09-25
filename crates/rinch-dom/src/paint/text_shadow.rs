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
//! its weight inside one isolated layer; a shadow whose glyphs × taps would
//! pass [`TAP_GLYPHS_PER_SHADOW`] gets a 5-tap cross or no blur, decided by
//! the shadow alone, and [`TAP_GLYPH_BUDGET`] guards a whole paint. The first
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

/// The most glyph copies the kernel-of-copies fallback draws for **one**
/// shadow (its glyphs × taps). A shadow of up to 153 glyphs gets the 13-tap
/// kernel, one of up to 400 a 5-tap cross, a longer one is drawn unblurred —
/// a decision about the element alone, so its blur does not change as other
/// text scrolls on or off screen (review of #1020, round 2, F3).
pub const TAP_GLYPHS_PER_SHADOW: usize = 2_000;

/// The most glyph copies the kernel-of-copies fallback draws in one paint,
/// summed over every blurred shadow; past it a shadow is drawn unblurred. A
/// crash guard, not a style decision — it depends on paint order — and far
/// past what a screen of text reaches under [`TAP_GLYPHS_PER_SHADOW`]: the
/// uncapped kernel overflowed wgpu's buffer-binding limit at 687 000 glyph
/// copies (review of #1020).
pub const TAP_GLYPH_BUDGET: usize = 150_000;

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
    /// How many paints are running on this thread (a paint may nest another);
    /// the per-paint state resets only at the outermost.
    static PAINT_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// Blurred masks rasterised this paint, for `text_shadow_masks_rasterised`.
    static RASTERISED: Cell<u64> = const { Cell::new(0) };
    #[cfg(feature = "software-renderer")]
    static MASK_CACHE: std::cell::RefCell<MaskCache> = std::cell::RefCell::new(MaskCache::default());
    /// The pixmap blurred shadows are rasterised into, kept across paints so
    /// its glyph cache stays warm, and trimmed to the largest mask the last
    /// [`SCRATCH_WINDOW`] paints needed ([`end_paint`]).
    #[cfg(feature = "software-renderer")]
    static SCRATCH: std::cell::RefCell<Option<super::skia_painter::TinySkiaPainter>> =
        const { std::cell::RefCell::new(None) };
    /// The largest mask side each of the last [`SCRATCH_WINDOW`] paints
    /// rasterised, newest last; and this paint's so far.
    #[cfg(feature = "software-renderer")]
    static SCRATCH_USE: std::cell::RefCell<ScratchUse> =
        const { std::cell::RefCell::new((Vec::new(), (0, 0))) };
}

/// The largest mask size each of the last paints needed, newest last, and
/// this paint's so far.
#[cfg(feature = "software-renderer")]
type ScratchUse = (Vec<(u32, u32)>, (u32, u32));

/// How many paints the scratch pixmap's size answers to.
#[cfg(feature = "software-renderer")]
const SCRATCH_WINDOW: usize = 8;

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

/// Start a paint: the kernel-of-copies crash guard and the rasterised count
/// are per paint, and a paint nested in another continues the outer one's.
pub(super) fn begin_paint() {
    let depth = PAINT_DEPTH.with(|d| d.replace(d.get() + 1));
    if depth == 0 {
        TAP_GLYPHS.with(|t| t.set(0));
        RASTERISED.with(|r| r.set(0));
    }
}

/// End a paint begun by [`begin_paint`]. At the outermost, returns how many
/// blurred masks it rasterised (the rest were served from the cache), and
/// trims the scratch pixmap to what the last [`SCRATCH_WINDOW`] paints needed
/// — dropping it when none of them needed one.
pub(super) fn end_paint() -> u64 {
    let depth = PAINT_DEPTH.with(|d| {
        let v = d.get().saturating_sub(1);
        d.set(v);
        v
    });
    if depth != 0 {
        return 0;
    }
    #[cfg(feature = "software-renderer")]
    {
        let need = SCRATCH_USE.with(|u| {
            let mut u = u.borrow_mut();
            let this = std::mem::take(&mut u.1);
            u.0.push(this);
            if u.0.len() > SCRATCH_WINDOW {
                u.0.remove(0);
            }
            u.0.iter()
                .fold((0, 0), |a, &(w, h)| (a.0.max(w), a.1.max(h)))
        });
        SCRATCH.with(|s| {
            let mut s = s.borrow_mut();
            if need == (0, 0) {
                *s = None;
            } else if let Some(sp) = s.as_mut()
                && (sp.width() > need.0 || sp.height() > need.1)
            {
                sp.resize(need.0, need.1);
            }
        });
    }
    RASTERISED.with(|r| r.replace(0))
}

/// The scratch pixmap's size, if there is one. For tests.
#[doc(hidden)]
pub fn text_shadow_scratch_size() -> Option<(u32, u32)> {
    #[cfg(feature = "software-renderer")]
    {
        SCRATCH.with(|s| s.borrow().as_ref().map(|p| (p.width(), p.height())))
    }
    #[cfg(not(feature = "software-renderer"))]
    None
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
    // The mask's frame is anchored to the pixel the text's origin rounds to,
    // not to its exact position, so a sub-pixel move changes the mask's size
    // and origin only when it moves that pixel: the key (which includes the
    // size) then changes only where the glyphs' pixels do. Every glyph pixel
    // is within a pixel of its exact extent from that anchor.
    let (ax, ay) = ((ox - 0.5).ceil(), (oy - 0.5).ceil());
    let mut area = Rect::new(
        ax + ext.x0.floor() - pad - 1.0,
        ay + ext.y0.floor() - pad - 1.0,
        ax + ext.x1.ceil() + pad + 1.0,
        ay + ext.y1.ceil() + pad + 1.0,
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

    // Every call the copy would make, hashed in the mask's own space and in
    // the terms that decide its pixels: the same key rasterises to the same
    // coverage, wherever on the surface the mask lands (a scroll by whole
    // pixels, a repaint of an unchanged page). The copy itself is drawn at
    // its true offset; only the key is quantised ([`CallHasher`]), so a
    // scroll by a fraction of a pixel that moves no glyph to another pixel
    // hits (review of #1020, rounds 2 and 3).
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
        let Some(entry) = c.get(key, (w, h, sigma.to_bits())) else {
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
    RASTERISED.with(|r| r.set(r.get() + 1));
    SCRATCH_USE.with(|u| {
        let mut u = u.borrow_mut();
        u.1 = (u.1.0.max(w as u32), u.1.1.max(h as u32));
    });
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
        MASK_CACHE.with(|c| c.borrow_mut().insert(key, (w, h, sigma.to_bits()), None));
        return true;
    }
    blur_2d(&blur, &mut coverage, w, h);

    let mask8: Vec<u8> = coverage
        .iter()
        .map(|&c| (c * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect();
    painter.draw_alpha_mask(&mask8, w as u32, h as u32, color, draw_at);
    MASK_CACHE.with(|c| {
        c.borrow_mut()
            .insert(key, (w, h, sigma.to_bits()), Some(mask8))
    });
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
///
/// Each entry also carries the width, height and blur it was made for, and a
/// hit is served only when they match: a 64-bit hash collision then costs a
/// miss rather than a wrong mask of another size.
#[cfg(feature = "software-renderer")]
#[derive(Default)]
struct MaskCache {
    entries: rustc_hash::FxHashMap<u64, MaskEntry>,
    bytes: usize,
    clock: u64,
}

/// `(width, height, sigma bits)` a mask was made for.
#[cfg(feature = "software-renderer")]
type MaskShape = (usize, usize, u64);

#[cfg(feature = "software-renderer")]
struct MaskEntry {
    shape: MaskShape,
    mask: Option<Vec<u8>>,
    used: u64,
}

#[cfg(feature = "software-renderer")]
impl MaskEntry {
    fn bytes(&self) -> usize {
        self.mask.as_ref().map_or(0, Vec::len) + 64
    }
}

#[cfg(feature = "software-renderer")]
impl MaskCache {
    fn get(&mut self, key: u64, shape: MaskShape) -> Option<Option<&Vec<u8>>> {
        self.clock += 1;
        let clock = self.clock;
        let e = self.entries.get_mut(&key)?;
        if e.shape != shape {
            return None;
        }
        e.used = clock;
        Some(e.mask.as_ref())
    }

    fn insert(&mut self, key: u64, shape: MaskShape, mask: Option<Vec<u8>>) {
        self.clock += 1;
        let entry = MaskEntry {
            shape,
            mask,
            used: self.clock,
        };
        let size = entry.bytes();
        if size > MASK_CACHE_BYTES {
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes -= old.bytes();
        }
        while self.bytes + size > MASK_CACHE_BYTES {
            let Some((&oldest, _)) = self.entries.iter().min_by_key(|(_, e)| e.used) else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.bytes -= old.bytes();
            }
        }
        self.entries.insert(key, entry);
        self.bytes += size;
    }
}

#[cfg(all(test, feature = "software-renderer"))]
mod cache_tests {
    use super::{MASK_CACHE_BYTES, MaskCache};

    /// The cache holds at most [`MASK_CACHE_BYTES`], and what leaves first
    /// is what was used longest ago — a hit renews an entry.
    ///
    /// Kills: no eviction; evicting the newest; a hit that does not renew.
    #[test]
    fn it_is_bounded_and_evicts_the_least_recently_used() {
        let mut c = MaskCache::default();
        let mb = 1 << 20;
        for k in 0..40u64 {
            c.insert(k, (mb, 1, 0), Some(vec![0; mb]));
            assert!(c.bytes <= MASK_CACHE_BYTES, "{} bytes after {k}", c.bytes);
            // Keep key 0 in use throughout.
            assert!(
                c.get(0, (mb, 1, 0)).is_some(),
                "the entry in use left at {k}"
            );
        }
        assert!(
            c.get(1, (mb, 1, 0)).is_none(),
            "the oldest unused entry is gone"
        );
        assert!(c.get(39, (mb, 1, 0)).is_some(), "the newest is kept");
        assert!(c.bytes <= MASK_CACHE_BYTES);
    }

    /// A hit is served only for the size and blur the mask was made for.
    #[test]
    fn a_hit_needs_the_same_shape() {
        let mut c = MaskCache::default();
        c.insert(7, (4, 4, 1), Some(vec![1; 16]));
        assert!(c.get(7, (4, 4, 1)).is_some());
        assert!(c.get(7, (4, 5, 1)).is_none());
        assert!(c.get(7, (4, 4, 2)).is_none());
    }
}

/// A [`Painter`] that draws nothing and hashes every call it is handed, in
/// the terms that decide its pixels: the font file and size, every transform,
/// every glyph and its position, every stroke and its shape.
#[cfg(feature = "software-renderer")]
#[derive(Default)]
struct CallHasher(rustc_hash::FxHasher);

/// How far from a half pixel a position has to be for [`CallHasher`] to key
/// it by the pixel it rounds to rather than by its exact value. Closer than
/// this, float error in the painter could round it either way.
#[cfg(feature = "software-renderer")]
const TIE_GUARD_PX: f64 = 1.0 / 64.0;

#[cfg(feature = "software-renderer")]
impl CallHasher {
    /// A glyph's device position as the software painter places it: it
    /// blits each glyph image by nearest-neighbour sampling, so the image
    /// lands on the pixel `ceil(p - 0.5)` and the fraction of `p` is in no
    /// pixel. Near a tie the exact value is keyed too, so an ambiguous
    /// position misses rather than reuses the other rounding.
    fn glyph_pos(&mut self, p: f64) {
        use std::hash::Hasher;
        self.0.write_i64((p - 0.5).ceil() as i64);
        let frac = p - p.floor();
        if (frac - 0.5).abs() < TIE_GUARD_PX {
            self.0.write_u64(p.to_bits());
        }
    }

    /// An anti-aliased coordinate (a decoration's stroke or fill), keyed to
    /// a quarter pixel: two copies with one key differ by at most 1/8 px
    /// there, under a blur of at least a quarter pixel.
    fn aa_pos(&mut self, p: f64) {
        use std::hash::Hasher;
        self.0.write_i64((p * 4.0).round() as i64);
    }

    /// The translation of `a` when it is a pure translation.
    fn translation(a: Affine) -> Option<(f64, f64)> {
        let c = a.as_coeffs();
        (c[0] == 1.0 && c[1] == 0.0 && c[2] == 0.0 && c[3] == 1.0).then_some((c[4], c[5]))
    }

    /// A shape drawn under `transform`: its device coordinates to a quarter
    /// pixel when the transform is a translation, else exactly.
    fn placed_shape(&mut self, transform: Affine, shape: &PaintShape) {
        use std::hash::Hasher;
        let Some((tx, ty)) = Self::translation(transform) else {
            self.affine(transform);
            self.shape(shape);
            return;
        };
        self.0.write_u8(0xA0);
        let b = shape.bounding_box();
        for (v, t) in [(b.x0, tx), (b.y0, ty), (b.x1, tx), (b.y1, ty)] {
            self.aa_pos(v + t);
        }
        if let PaintShape::BezPath(p) = shape {
            for el in p.elements() {
                if let Some(pt) = el.end_point() {
                    self.aa_pos(pt.x + tx);
                    self.aa_pos(pt.y + ty);
                }
            }
        }
    }

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
        self.placed_shape(transform, shape);
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
        self.placed_shape(transform, shape);
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
        let place = Self::translation(transform);
        match place {
            Some(_) => self.0.write_u8(0xA1),
            None => self.affine(transform),
        }
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
            match place {
                // The painter adds the image's whole-pixel bearing to this,
                // which moves the rounding with it.
                Some((tx, ty)) => {
                    self.glyph_pos(tx + g.x as f64);
                    self.glyph_pos(ty + g.y as f64);
                }
                None => {
                    self.0.write_u32(g.x.to_bits());
                    self.0.write_u32(g.y.to_bits());
                }
            }
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
/// shadow colour's alpha. The kernel is the full one, a 5-tap cross or none,
/// by the shadow's own glyph count against [`TAP_GLYPHS_PER_SHADOW`]; past
/// the per-paint crash guard [`TAP_GLYPH_BUDGET`] a shadow is drawn once,
/// unblurred.
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
    // The kernel is a function of this shadow alone: the full kernel, a
    // cross, or none, by how many glyph copies each would draw.
    let glyphs = glyph_count(layout).max(1);
    let full = shadow_kernel(sigma, false);
    let kernel = if glyphs * full.len() <= TAP_GLYPHS_PER_SHADOW {
        full
    } else {
        let cross = shadow_kernel(sigma, true);
        if glyphs * cross.len() <= TAP_GLYPHS_PER_SHADOW {
            cross
        } else {
            vec![(0.0, 0.0, 1.0)]
        }
    };
    let copies = glyphs * kernel.len();
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
/// `cross` keeps only the centre and the four taps one step out along the
/// axes (5 taps), for a shadow too long for the full kernel.
///
/// A `sigma` below a quarter pixel gives the single tap `(0, 0, 1.0)`.
pub(super) fn shadow_kernel(sigma: f64, cross: bool) -> Vec<(f64, f64, f32)> {
    if sigma.is_nan() || sigma < 0.25 || !sigma.is_finite() {
        return vec![(0.0, 0.0, 1.0)];
    }
    let radius = 3.0 * sigma;
    let step = (radius / 2.0).max(1.0);
    let n = ((radius / step).floor() as i32).min(if cross { 1 } else { 2 });
    let mut taps: Vec<(f64, f64, f64)> = Vec::new();
    for j in -n..=n {
        for i in -n..=n {
            if cross && i != 0 && j != 0 {
                continue;
            }
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
