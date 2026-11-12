//! Minified copies of decoded images (#2456, #2534): a raster image drawn
//! at half its natural size or less is drawn from a mipmap level of it, as Chrome
//! draws it, not sampled straight from its full pixels.
//!
//! Both painters sample an `<img>` from its own pixels: the software painter
//! nearest-neighbour, Vello bilinearly, neither with mipmaps. Drawn at 0.4x,
//! the samples are 2.5 source pixels apart, so a 1px line falls between two
//! of them and is dropped. Thin strokes of small text baked into a picture —
//! the bars of an E, the tail of a $ — go with it.
//!
//! Chrome 157 draws a minified picture from a mipmap level, on its software
//! and GPU rasters alike, as its image decode cache (cc's `MipMapUtil`)
//! makes one: level `L` is `ceil(side / 2^L)` on each side, the deepest
//! level still no smaller than the drawn size on *both* axes (so the less
//! reduced axis picks it), and it is filled by Skia scaling the picture to
//! that size, which itself samples Skia's `SkMipmap` (each level half the
//! one above rounding down, each pixel the 2x2 sum of the four above it
//! premultiplied and truncated, 1-2-1 across an odd side) bilinearly. When
//! every side halves exactly the two agree and the level is the `SkMipmap`
//! level itself. The draw then samples the level bilinearly. Measured
//! against Chrome's screenshots (#2534, `rinch/tests/image_filter_2534.rs`)
//! that model is exact at 1/2 and 1/8 and within about 1.4 a channel on
//! average everywhere else measured, odd sides included. A box filter to the
//! drawn size, and Mitchell or Lanczos-3 resampling, are 6 to 7 a channel
//! off at 1/3: they blur what Chrome's level-then-bilinear keeps sharp. #2456
//! first box-filtered to the drawn scale rounded up to a quarter octave;
//! that was softer than Chrome (#2534). Halving an odd side down to the
//! level instead of scaling to `ceil` was 13 a channel off (#2629 review).
//!
//! So [`minified`] makes the level ([`mip_level`], [`level_size`],
//! [`mip_downscale`]) of a draw scaled to about 1/2 or less on both axes,
//! and the painter draws it the rest of the way bilinearly
//! (`ImageSampling::Smooth`
//! (crate::paint::painter::ImageSampling::Smooth), the software painter
//! included). A draw at level 0 samples the image itself, bilinearly when
//! some axis is below 1x ([`MINIFY_BELOW`], [`smooth`]): the software
//! painter's own `Auto` is nearest, which at 0.6x drops a 1px line in three.
//! A picture whose scale moves continuously (a zoom, a
//! `transform` transition) makes a new copy only every octave.
//! `image-rendering: pixelated`/`crisp-edges` asks for nearest-neighbour and
//! is never minified.
//!
//! The copies live in a per-thread cache like the SVG rasters'
//! ([`crate::svg_image`]): keyed by the image's identity and the copy's size,
//! held within [`MINIFY_BUDGET_BYTES`], let go after [`MINIFY_IDLE_FRAMES`]
//! painted frames unused. Each copy is a [`DecodedImage`] of its own, so the
//! software painter keeps its premultiplied form and Vello one atlas image
//! for it across frames. A frame makes copies of at most
//! [`MAX_MINIFY_WORK_PER_FRAME`] source pixels; an image past that is drawn
//! from its full pixels this frame, counted in [`refusals`], and the paint
//! asks for it to be painted again on the next frame (`NodeTree`'s
//! `minify_deferred`), so a page of large pictures is filtered over a few
//! frames rather than left aliased until something else repaints.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use peniko::kurbo::Affine;

use crate::image_cache::DecodedImage;

/// A draw that scales the image below this on either axis samples it
/// bilinearly ([`smooth`]), from its mipmap level when it has one. 0.75
/// before the #2629 review, which left 0.75x to 1x nearest on the software
/// painter where Chrome is bilinear.
pub const MINIFY_BELOW: f64 = 1.0;

/// The bytes every minified copy on one thread may hold together: 64 MB,
/// charged at 8 bytes a pixel (the copy, and the painter's premultiplied copy
/// or GPU blob of it), as the SVG rasters are.
pub const MINIFY_BUDGET_BYTES: usize = 64 << 20;

/// Painted frames a copy may go unused before it is let go.
pub const MINIFY_IDLE_FRAMES: u64 = 60;

/// The most source pixels one painted frame filters into new copies: 16M.
/// An opaque picture's level costs about 17 instructions a source pixel
/// (`rinch-bench`'s `full_paint::minified_picture_first` less
/// `minified_picture_cached`, #2534), under half of #2456's box filter. An
/// image that would pass it is drawn from its full pixels this frame.
pub const MAX_MINIFY_WORK_PER_FRAME: u64 = 16 << 20;

struct Entry {
    image: Arc<DecodedImage>,
    bytes: usize,
    last_frame: u64,
}

struct Cache {
    entries: HashMap<(u64, u32, u32), Entry>,
    bytes: usize,
    frame: u64,
    /// Source pixels filtered this frame.
    work: u64,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache {
        entries: HashMap::new(),
        bytes: 0,
        frame: 0,
        work: 0,
    });
    /// How many copies this thread has made (a test's count).
    static MADE: Cell<u64> = const { Cell::new(0) };
    /// How many draws the frame budget refused.
    static REFUSALS: Cell<u64> = const { Cell::new(0) };
    static WORK_BUDGET: Cell<u64> = const { Cell::new(MAX_MINIFY_WORK_PER_FRAME) };
}

/// How many draws on this thread the per-frame budget has refused a copy
/// (each drawn from the full image instead). A paint that sees it move asks
/// for the node again ([`crate::node::NodeTree`]'s `minify_deferred`), and a
/// background tile drawn while it moved is not cached.
pub fn refusals() -> u64 {
    REFUSALS.with(Cell::get)
}

/// The guard [`test_work_budget`] returns.
#[doc(hidden)]
pub struct WorkBudgetGuard(u64);

impl Drop for WorkBudgetGuard {
    fn drop(&mut self) {
        WORK_BUDGET.with(|b| b.set(self.0));
    }
}

/// Set this thread's per-frame work budget (a test's), restored when the
/// guard drops.
#[doc(hidden)]
pub fn test_work_budget(px: u64) -> WorkBudgetGuard {
    WorkBudgetGuard(WORK_BUDGET.with(|b| b.replace(px)))
}

/// One painted frame on the cache's clock: copies not drawn for
/// [`MINIFY_IDLE_FRAMES`] frames are let go, and the frame's work budget
/// starts again.
pub fn begin_frame() {
    CACHE.with_borrow_mut(|c| {
        c.frame += 1;
        c.work = 0;
        let frame = c.frame;
        let mut freed = 0;
        c.entries.retain(|_, e| {
            let keep = frame - e.last_frame <= MINIFY_IDLE_FRAMES;
            if !keep {
                freed += e.bytes;
            }
            keep
        });
        c.bytes -= freed;
    });
}

/// How many minified copies this thread has made.
#[doc(hidden)]
pub fn made() -> u64 {
    MADE.with(Cell::get)
}

/// The bytes this thread's copies are charged.
#[doc(hidden)]
pub fn held_bytes() -> usize {
    CACHE.with_borrow(|c| c.bytes)
}

/// The scale of each axis of `transform` (image pixels to device): the
/// lengths of its columns, so a rotation or skew does not change them.
fn axis_scales(transform: Affine) -> (f64, f64) {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    (a.hypot(b), c.hypot(d))
}

/// `n / 2^level` rounded up, at least 1: a side of mipmap level `level`.
fn ceil_shift(n: u32, level: u32) -> u32 {
    (((u64::from(n) + (1u64 << level) - 1) >> level) as u32).max(1)
}

/// The mipmap level a draw of a `w` x `h` image under `transform` samples,
/// as Chrome's image decode cache picks it (cc's `MipMapUtil`): the deepest
/// level whose size ([`level_size`]) is still no smaller than the drawn
/// size on both axes, so it is chosen by the less reduced axis (a draw at
/// exactly 1/2 of an even side is level 1 at 1:1). 0 (the image itself)
/// for a draw above about 1/2 on some axis, or an unusable one.
pub fn mip_level(w: u32, h: u32, transform: Affine) -> u32 {
    let (sx, sy) = axis_scales(transform);
    if !(sx.is_finite() && sy.is_finite()) || w == 0 || h == 0 {
        return 0;
    }
    // The drawn size, less a hair so a scale computed as 299/599 still
    // counts 299 as reached.
    let (dw, dh) = (f64::from(w) * sx - 1e-6, f64::from(h) * sy - 1e-6);
    let mut level = 0;
    while level < 31
        && (ceil_shift(w, level) > 1 || ceil_shift(h, level) > 1)
        && f64::from(ceil_shift(w, level + 1)) >= dw
        && f64::from(ceil_shift(h, level + 1)) >= dh
    {
        level += 1;
    }
    level
}

/// The size of mipmap level `level` of a `w` x `h` image, as Chrome sizes
/// it: each side `side / 2^level` rounded **up**, at least 1. (Skia's own
/// `SkMipmap` rounds down; Chrome scales the picture to this size with it,
/// [`mip_downscale`].)
pub fn level_size(w: u32, h: u32, level: u32) -> (u32, u32) {
    (ceil_shift(w, level), ceil_shift(h, level))
}

/// The size of the copy a draw of `image` under `transform` (image pixels to
/// device) is drawn from: its [`mip_level`]'s size, or `None` when it is
/// drawn from the image itself: a vector image, an empty one, a draw at
/// level 0.
pub fn target_size(image: &DecodedImage, transform: Affine) -> Option<(u32, u32)> {
    if image.vector().is_some() || image.width == 0 || image.height == 0 {
        return None;
    }
    let level = mip_level(image.width, image.height, transform);
    let size = level_size(image.width, image.height, level);
    (size != (image.width, image.height)).then_some(size)
}

/// Whether a draw of a raster image under `transform` should sample it
/// bilinearly: it scales some axis below [`MINIFY_BELOW`] (1x, less a
/// hair). Chrome samples a minified picture bilinearly, from its level or
/// from the picture itself; the software painter's own `Auto` is nearest,
/// which drops whole source pixels there (11.6 a channel off Chrome on
/// average at 0.76x, #2629 review).
pub fn smooth(transform: Affine) -> bool {
    let (sx, sy) = axis_scales(transform);
    sx < MINIFY_BELOW - 1e-6 || sy < MINIFY_BELOW - 1e-6
}

/// The copy of `image` to draw under `transform`, when the draw is
/// minified ([`target_size`]): made on its first draw at that size and kept.
/// `None` draws `image` itself.
pub fn minified(image: &DecodedImage, transform: Affine) -> Option<Arc<DecodedImage>> {
    let (w, h) = target_size(image, transform)?;
    let key = (image.id(), w, h);
    let src_px = u64::from(image.width) * u64::from(image.height);
    let hit = CACHE.with_borrow_mut(|c| {
        let frame = c.frame;
        if let Some(e) = c.entries.get_mut(&key) {
            e.last_frame = frame;
            return Some(Some(e.image.clone()));
        }
        if c.work + src_px > WORK_BUDGET.with(Cell::get) {
            REFUSALS.with(|r| r.set(r.get() + 1));
            return Some(None);
        }
        c.work += src_px;
        None
    });
    if let Some(hit) = hit {
        return hit;
    }
    let pixels = mip_downscale(&image.data, image.width, image.height, w, h);
    let copy = Arc::new(DecodedImage::new(pixels, w, h));
    MADE.with(|m| m.set(m.get() + 1));
    let bytes = w as usize * h as usize * 8;
    CACHE.with_borrow_mut(|c| {
        // Least recently used first, never one this frame has drawn.
        let frame = c.frame;
        while c.bytes + bytes > MINIFY_BUDGET_BYTES {
            let Some(oldest) = c
                .entries
                .iter()
                .filter(|(_, e)| e.last_frame < frame)
                .min_by_key(|(_, e)| e.last_frame)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(e) = c.entries.remove(&oldest) {
                c.bytes -= e.bytes;
            }
        }
        c.bytes += bytes;
        c.entries.insert(
            key,
            Entry {
                image: copy.clone(),
                bytes,
                last_frame: frame,
            },
        );
    });
    Some(copy)
}

/// Straight-alpha RGBA8 `src` (`sw` x `sh`) scaled down to `dw` x `dh` (a
/// [`level_size`]), straight alpha, as Chrome fills a mipmap level: Skia
/// scales the picture to that size from its own `SkMipmap` level
/// `floor(log2(1 / s))` (`s` the larger of the two scales), bilinearly.
/// That `SkMipmap` level is built premultiplied, halving level by level
/// (each side rounded down), each output pixel the sum of its 2x2 source
/// pixels shifted right by 2 (truncated); a side of odd length is filtered
/// 1-2-1 over the three source pixels centred on the pair, shifted by one
/// more; a side of length 1 stays 1. When the level has the size asked for
/// (every side halved exactly) the bilinear step is the identity and is
/// skipped; otherwise (an odd side, #2629 review) the level is resampled
/// with pixel-centre bilinear taps, edges clamped.
pub fn mip_downscale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    debug_assert_eq!(src.len(), sw as usize * sh as usize * 4);
    debug_assert!(dw > 0 && dh > 0 && dw <= sw && dh <= sh);
    // An opaque image's premultiplied form is itself, and every level of it
    // stays opaque: no premultiply, no unpremultiply.
    let opaque = src.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
    let mut level: std::borrow::Cow<'_, [u8]> = if opaque {
        std::borrow::Cow::Borrowed(src)
    } else {
        std::borrow::Cow::Owned(
            src.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    let a = u32::from(p[3]);
                    // Skia's SkMulDiv255Round.
                    let mul = |c: u8| -> u8 {
                        let prod = u32::from(c) * a + 128;
                        ((prod + (prod >> 8)) >> 8) as u8
                    };
                    [mul(p[0]), mul(p[1]), mul(p[2]), p[3]]
                })
                .collect(),
        )
    };
    let s = (f64::from(dw) / f64::from(sw)).max(f64::from(dh) / f64::from(sh));
    let halvings = if s < 1.0 {
        (-s.log2() + 1e-9).floor() as u32
    } else {
        0
    };
    let (mut w, mut h) = (sw as usize, sh as usize);
    for _ in 0..halvings {
        if w == 1 && h == 1 {
            break;
        }
        level = std::borrow::Cow::Owned(halve(&level, w, h));
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
    }
    let mut out = if (w, h) == (dw as usize, dh as usize) {
        level.into_owned()
    } else {
        bilinear(&level, w, h, dw as usize, dh as usize)
    };
    if !opaque {
        for p in out.as_chunks_mut::<4>().0 {
            let a = u32::from(p[3]);
            if a == 0 {
                *p = [0; 4];
                continue;
            }
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    out
}

/// RGBA8 `src` (`w` x `h`) resampled to `dw` x `dh` bilinearly: output
/// pixel `i`'s centre maps to source `(i + 0.5) * w / dw - 0.5`, clamped to
/// the image, between the two nearest source pixels, with weights in
/// 1/256ths; rounded.
fn bilinear(src: &[u8], w: usize, h: usize, dw: usize, dh: usize) -> Vec<u8> {
    let taps = |n: usize, d: usize| -> Vec<(usize, usize, u32)> {
        let ratio = n as f64 / d as f64;
        (0..d)
            .map(|i| {
                let x = ((i as f64 + 0.5) * ratio - 0.5).clamp(0.0, (n - 1) as f64);
                let x0 = x.floor() as usize;
                let f = ((x - x0 as f64) * 256.0).round() as u32;
                (x0, (x0 + 1).min(n - 1), f)
            })
            .collect()
    };
    let cols = taps(w, dw);
    let rows = taps(h, dh);
    let px = src.as_chunks::<4>().0;
    let mut out = vec![0u8; dw * dh * 4];
    let mut top = vec![[0u32; 4]; dw];
    let mut bottom = vec![[0u32; 4]; dw];
    // Each source row's horizontal pass, kept while consecutive output rows
    // share it.
    let horizontal = |row: usize, into: &mut [[u32; 4]]| {
        let line = &px[row * w..][..w];
        for (o, &(x0, x1, f)) in into.iter_mut().zip(&cols) {
            let (a, b) = (line[x0], line[x1]);
            for c in 0..4 {
                o[c] = u32::from(a[c]) * (256 - f) + u32::from(b[c]) * f;
            }
        }
    };
    let (mut have_top, mut have_bottom) = (usize::MAX, usize::MAX);
    for (orow, &(y0, y1, f)) in out.as_chunks_mut::<4>().0.chunks_mut(dw).zip(&rows) {
        if have_top != y0 {
            if have_bottom == y0 {
                std::mem::swap(&mut top, &mut bottom);
                have_bottom = usize::MAX;
            } else {
                horizontal(y0, &mut top);
            }
            have_top = y0;
        }
        if have_bottom != y1 {
            horizontal(y1, &mut bottom);
            have_bottom = y1;
        }
        for ((o, t), b) in orow.iter_mut().zip(&top).zip(&bottom) {
            for c in 0..4 {
                o[c] = ((t[c] * (256 - f) + b[c] * f + 32768) >> 16) as u8;
            }
        }
    }
    out
}

/// One mipmap level of premultiplied RGBA8 `src` (`w` x `h`, not both 1):
/// each side halved rounding down (at least 1), each output pixel the sum of
/// its sources shifted right (truncated), as `SkMipmap`'s downsamplers: 2x2
/// for even sides, 1-2-1 across an odd one, 1 along a side of length 1.
fn halve(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    if w.is_multiple_of(2) && h.is_multiple_of(2) {
        // The common case, written so it vectorises: 2x2 sums of bytes.
        for (oy, row) in out.chunks_exact_mut(nw * 4).enumerate() {
            let r0 = &src[2 * oy * w * 4..][..w * 4];
            let r1 = &src[(2 * oy + 1) * w * 4..][..w * 4];
            for ((o, a), b) in row
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(r0.as_chunks::<8>().0)
                .zip(r1.as_chunks::<8>().0)
            {
                for c in 0..4 {
                    let sum = u16::from(a[c])
                        + u16::from(a[4 + c])
                        + u16::from(b[c])
                        + u16::from(b[4 + c]);
                    o[c] = (sum >> 2) as u8;
                }
            }
        }
        return out;
    }
    // Taps along a side of length `n`: the weights from the pair's first
    // source pixel, and their shift.
    let taps = |n: usize| -> (&'static [u32], u32) {
        match n {
            1 => (&[1], 0),
            n if n.is_multiple_of(2) => (&[1, 1], 1),
            _ => (&[1, 2, 1], 2),
        }
    };
    let (tx, shift_x) = taps(w);
    let (ty, shift_y) = taps(h);
    let shift = shift_x + shift_y;
    for oy in 0..nh {
        let y0 = if h == 1 { 0 } else { oy * 2 };
        for ox in 0..nw {
            let x0 = if w == 1 { 0 } else { ox * 2 };
            let mut sum = [0u32; 4];
            for (j, &wy) in ty.iter().enumerate() {
                let row = &src[(y0 + j) * w * 4..];
                for (i, &wx) in tx.iter().enumerate() {
                    let p = &row[(x0 + i) * 4..][..4];
                    for c in 0..4 {
                        sum[c] += u32::from(p[c]) * wx * wy;
                    }
                }
            }
            out[(oy * nw + ox) * 4..][..4].copy_from_slice(&sum.map(|v| (v >> shift) as u8));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pixels_add_no_colour() {
        // A red opaque pixel beside a transparent *green* one, drawn to
        // 1x1 (bilinear: the other axis is not reduced): red at half alpha,
        // not a red-green blend.
        let src = [255, 0, 0, 255, 0, 255, 0, 0];
        assert_eq!(mip_downscale(&src, 2, 1, 1, 1), vec![255, 0, 0, 128]);
        // The same through the 2x2 path: opaque red, transparent green,
        // half-alpha blue (premultiplied 128) and transparent black sum to
        // (255, 0, 128, 383) >> 2 = (63, 0, 32, 95), unpremultiplied.
        let src = [255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 128, 0, 0, 0, 0];
        assert_eq!(mip_downscale(&src, 2, 2, 1, 1), vec![169, 0, 86, 95]);
    }

    /// Each level is the 2x2 sum shifted right by 2 (truncated, so 1+1+1+2
    /// is 1, not 1.25 rounded), level by level; an odd side is 1-2-1; a
    /// size no halving reaches is resampled bilinearly from the level.
    #[test]
    fn levels_truncate_like_skia() {
        let grey = |v: &[u8]| -> Vec<u8> { v.iter().flat_map(|&g| [g, g, g, 255]).collect() };
        let rows = |row: &[u8], n: usize| -> Vec<u8> { grey(&row.repeat(n)) };
        // 2x2 -> 1x1: (1 + 1 + 1 + 2) >> 2 = 1.
        assert_eq!(mip_downscale(&grey(&[1, 1, 1, 2]), 2, 2, 1, 1), grey(&[1]));
        // 4x4 -> 1x1 by two levels: (10+13+10+13)>>2 = 11, (20+27+20+27)>>2
        // = 23, then (11 + 23 + 11 + 23) >> 2 = 17; one average is 17.5.
        assert_eq!(
            mip_downscale(&rows(&[10, 13, 20, 27], 4), 4, 4, 1, 1),
            grey(&[17])
        );
        // 3x3 -> 1x1: 1-2-1 both ways, (0 + 200 + 255) * 4 >> 4 = 113.
        assert_eq!(
            mip_downscale(&rows(&[0, 100, 255], 3), 3, 3, 1, 1),
            grey(&[113])
        );
        // 5x5 -> 2x2: output i is 1-2-1 over 2i, 2i+1, 2i+2.
        assert_eq!(
            mip_downscale(&rows(&[0, 40, 80, 120, 160], 5), 5, 5, 2, 2),
            grey(&[40, 120, 40, 120])
        );
        // 5x5 -> 3x3 (level 1 of an odd side, `ceil(5 / 2)`): no halving
        // reaches it, so the picture is resampled bilinearly at centres
        // 1/3, 2 and 11/3: 13.3, 80, 146.7.
        assert_eq!(
            mip_downscale(&rows(&[0, 40, 80, 120, 160], 5), 5, 5, 3, 3),
            rows(&[13, 80, 147], 3)
        );
    }

    /// A copy not drawn for `MINIFY_IDLE_FRAMES` painted frames is let go,
    /// and its bytes with it; one drawn in time is kept.
    #[test]
    fn idle_copies_are_let_go() {
        let img = DecodedImage::new(vec![255; 100 * 100 * 4], 100, 100);
        begin_frame();
        let held = held_bytes();
        assert!(minified(&img, Affine::scale(0.4)).is_some());
        assert_eq!(held_bytes(), held + 50 * 50 * 8);
        for _ in 0..MINIFY_IDLE_FRAMES {
            begin_frame();
        }
        assert_eq!(held_bytes(), held + 50 * 50 * 8, "kept until idle");
        let made_before = made();
        assert!(minified(&img, Affine::scale(0.4)).is_some());
        assert_eq!(made(), made_before, "a hit renews it");
        for _ in 0..=MINIFY_IDLE_FRAMES {
            begin_frame();
        }
        assert_eq!(held_bytes(), held, "let go");
        assert!(minified(&img, Affine::scale(0.4)).is_some());
        assert_eq!(made(), made_before + 1, "made again");
    }

    /// The budget refuses a copy past it within a frame, and starts again
    /// with the next frame.
    #[test]
    fn the_work_budget_is_per_frame() {
        let _budget = test_work_budget(100 * 100);
        let a = DecodedImage::new(vec![255; 100 * 100 * 4], 100, 100);
        let b = DecodedImage::new(vec![255; 100 * 100 * 4], 100, 100);
        begin_frame();
        let (made0, refused0) = (made(), refusals());
        assert!(minified(&a, Affine::scale(0.4)).is_some());
        assert!(minified(&b, Affine::scale(0.4)).is_none(), "over budget");
        assert_eq!((made(), refusals()), (made0 + 1, refused0 + 1));
        // A copy already made costs nothing.
        assert!(minified(&a, Affine::scale(0.4)).is_some());
        begin_frame();
        assert!(minified(&b, Affine::scale(0.4)).is_some(), "the next frame");
        assert_eq!(made(), made0 + 2);
    }

    #[test]
    fn target_size_by_axis() {
        let img = DecodedImage::new(vec![0; 300 * 200 * 4], 300, 200);
        // Above 1/2: level 0, drawn from the image itself.
        assert_eq!(target_size(&img, Affine::scale(0.6)), None);
        assert_eq!(target_size(&img, Affine::scale(0.51)), None);
        // (1/4, 1/2]: level 1, the half-size image; (1/8, 1/4]: level 2.
        assert_eq!(target_size(&img, Affine::scale(0.5)), Some((150, 100)));
        assert_eq!(target_size(&img, Affine::scale(0.49)), Some((150, 100)));
        assert_eq!(
            target_size(&img, Affine::scale(1.0 / 3.0)),
            Some((150, 100))
        );
        assert_eq!(target_size(&img, Affine::scale(0.25)), Some((75, 50)));
        assert_eq!(target_size(&img, Affine::scale(0.2)), Some((75, 50)));
        // Each side halved rounding up (as Chrome sizes a level), never
        // below 1.
        assert_eq!(target_size(&img, Affine::scale(0.1)), Some((38, 25)));
        let odd = DecodedImage::new(vec![0; 599 * 597 * 4], 599, 597);
        // 299x298 of 599x597 is reached by level 1, 300x299.
        assert_eq!(
            target_size(
                &odd,
                Affine::scale_non_uniform(299.0 / 599.0, 298.0 / 597.0)
            ),
            Some((300, 299))
        );
        assert_eq!(target_size(&img, Affine::scale(0.001)), Some((1, 1)));
        // The level is the larger axis scale's, as Skia picks it: 0.6x by
        // 0.3x samples the image itself, 0.3x by 0.1x the half-size level.
        assert_eq!(target_size(&img, Affine::scale_non_uniform(0.6, 0.3)), None);
        assert_eq!(
            target_size(&img, Affine::scale_non_uniform(0.3, 0.1)),
            Some((150, 100))
        );
        assert_eq!(target_size(&img, Affine::scale_non_uniform(0.3, 2.0)), None);
        // A continuous zoom from 0.30x to 0.49x makes 1 level, not 20.
        let levels: std::collections::HashSet<_> = (0..20)
            .map(|i| target_size(&img, Affine::scale(0.30 + 0.01 * i as f64)))
            .collect();
        assert_eq!(levels.len(), 1, "{levels:?}");
        // A rotation does not change the scale.
        assert_eq!(
            target_size(&img, Affine::rotate(0.7) * Affine::scale(0.4)),
            Some((150, 100))
        );
        // Bilinear below 1x on either axis.
        assert!(smooth(Affine::scale(0.76)));
        assert!(smooth(Affine::scale_non_uniform(2.0, 0.6)));
        assert!(!smooth(Affine::scale(1.0)));
        assert!(!smooth(Affine::scale(1.5)));
    }
}
