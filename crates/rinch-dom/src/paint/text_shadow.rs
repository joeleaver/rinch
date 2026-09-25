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

use super::painter::{BlendMode, PaintImage, PaintShape, Painter};
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
    if !(sigma >= 0.25) {
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

/// The text's own extent relative to its origin, in physical px: its line
/// boxes, grown by how far glyph ink may reach past them.
fn text_extent(layout: &parley::layout::Layout<Brush>, scale: f64) -> Rect {
    let ink = layout_ink_margin(layout) * scale;
    Rect::new(
        -ink,
        -ink,
        layout.full_width().max(layout.width()) as f64 * scale + ink,
        layout.height() as f64 * scale + ink,
    )
}

/// How far, in layout px, glyph ink may reach past `layout`'s line boxes: an
/// italic's overhang, a tall accent, a wavy underline. Half the largest font
/// size, which is generous; it only sizes a mask or a layer's bounds.
fn layout_ink_margin(layout: &parley::layout::Layout<Brush>) -> f64 {
    let mut size = 0.0_f32;
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                size = size.max(run.run().font_size());
            }
        }
    }
    size as f64 * 0.5
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
    let blur = Blur1d::new(sigma);
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
            for (o, p) in out.iter_mut().zip(src.chunks_exact(4)) {
                if p[3] != 0 {
                    *o = p[3] as f32 * (1.0 / 255.0);
                    any = true;
                }
            }
        }
    });
    if !any {
        return true;
    }

    blur_2d(&blur, &mut coverage, w, h);

    let [r, g, b, a] = color.components;
    let (r, g, b) = (
        (r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (b.clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    let alpha = a.clamp(0.0, 1.0) * 255.0;
    let mut rgba = vec![0_u8; w * h * 4];
    for (px, &cov) in rgba.chunks_exact_mut(4).zip(&coverage) {
        let a = (cov * alpha).round().clamp(0.0, 255.0) as u8;
        if a != 0 {
            px.copy_from_slice(&[r, g, b, a]);
        }
    }
    painter.draw_image(
        &PaintImage {
            data: &rgba,
            width: w as u32,
            height: h as u32,
            decoded: None,
            opaque: false,
        },
        image_transform * Affine::translate((area.x0, area.y0)),
    );
    true
}

/// Blur a `w` x `h` coverage mask in place, rows then columns. A row with no
/// coverage stays empty through the row pass, so it is skipped.
#[cfg(feature = "software-renderer")]
fn blur_2d(blur: &Blur1d, mask: &mut [f32], w: usize, h: usize) {
    let mut tmp = Vec::new();
    for row in mask.chunks_exact_mut(w) {
        if row.iter().any(|&v| v != 0.0) {
            blur.apply(row, &mut tmp);
        }
    }
    let mut col = vec![0.0_f32; h];
    for x in 0..w {
        let mut any = false;
        for (y, v) in col.iter_mut().enumerate() {
            *v = mask[y * w + x];
            any |= *v != 0.0;
        }
        if !any {
            continue;
        }
        blur.apply(&mut col, &mut tmp);
        for (y, v) in col.iter().enumerate() {
            mask[y * w + x] = *v;
        }
    }
}

/// Past this `sigma` a 1-D blur is three box blurs rather than a sampled
/// Gaussian kernel: the kernel costs `6 sigma` per sample, the boxes a
/// constant, and above it the boxes are within a level of the Gaussian.
#[cfg(feature = "software-renderer")]
const DIRECT_KERNEL_MAX_SIGMA: f64 = 2.5;

/// A 1-D approximation of a Gaussian blur of standard deviation `sigma`: a
/// sampled kernel out to `3 sigma` for a small `sigma`, three box blurs
/// (Kovesi, "Fast Almost-Gaussian Filtering") for a large one — Skia's own
/// method. Values past either end of a line count as zero.
///
/// The same construction as the inset `box-shadow` blur of PR #1014 (#974),
/// kept here rather than shared while both are unmerged.
#[cfg(feature = "software-renderer")]
enum Blur1d {
    Kernel(Vec<f32>),
    Boxes([usize; 3]),
}

#[cfg(feature = "software-renderer")]
impl Blur1d {
    fn new(sigma: f64) -> Self {
        if sigma <= DIRECT_KERNEL_MAX_SIGMA {
            let radius = (3.0 * sigma).ceil().max(1.0) as i64;
            let mut k: Vec<f64> = (-radius..=radius)
                .map(|i| (-(i as f64).powi(2) / (2.0 * sigma * sigma)).exp())
                .collect();
            let sum: f64 = k.iter().sum();
            k.iter_mut().for_each(|v| *v /= sum);
            Blur1d::Kernel(k.into_iter().map(|v| v as f32).collect())
        } else {
            const N: f64 = 3.0;
            let ideal = (12.0 * sigma * sigma / N + 1.0).sqrt();
            let mut wl = ideal.floor() as i64;
            if wl % 2 == 0 {
                wl -= 1;
            }
            let wl = wl.max(1);
            let wlf = wl as f64;
            let m = ((12.0 * sigma * sigma - N * wlf * wlf - 4.0 * N * wlf - 3.0 * N)
                / (-4.0 * wlf - 4.0))
                .round() as i64;
            let mut widths = [0usize; 3];
            for (i, w) in widths.iter_mut().enumerate() {
                *w = if (i as i64) < m { wl } else { wl + 2 } as usize;
            }
            Blur1d::Boxes(widths)
        }
    }

    /// How far one sample's value spreads, in samples.
    fn reach(&self) -> usize {
        match self {
            Blur1d::Kernel(k) => k.len() / 2,
            Blur1d::Boxes(w) => w.iter().map(|w| w / 2).sum(),
        }
    }

    /// Blur `line` in place; `tmp` is scratch, reused across calls.
    fn apply(&self, line: &mut [f32], tmp: &mut Vec<f32>) {
        let n = line.len();
        tmp.clear();
        tmp.extend_from_slice(line);
        match self {
            Blur1d::Kernel(k) => {
                let r = k.len() / 2;
                for (i, out) in line.iter_mut().enumerate() {
                    let lo = i.saturating_sub(r);
                    let hi = (i + r).min(n - 1);
                    let mut acc = 0.0f32;
                    for (s, v) in tmp[lo..=hi].iter().enumerate() {
                        acc += v * k[lo + s + r - i];
                    }
                    *out = acc;
                }
            }
            Blur1d::Boxes(widths) => {
                for &w in widths {
                    let r = w / 2;
                    if r == 0 {
                        continue;
                    }
                    let norm = 1.0 / w as f32;
                    let mut sum: f32 = tmp.iter().take(r.min(n)).sum();
                    for i in 0..n {
                        if i + r < n {
                            sum += tmp[i + r];
                        }
                        line[i] = sum * norm;
                        if i >= r {
                            sum -= tmp[i - r];
                        }
                    }
                    tmp.copy_from_slice(line);
                }
            }
        }
    }
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
    if !(sigma >= 0.25) || !sigma.is_finite() {
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
