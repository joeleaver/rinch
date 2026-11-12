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
//! Chrome 157 draws a minified picture from Skia's mipmaps (`SkMipmap`), on
//! its software and GPU rasters alike: each level is half the one above,
//! each of its pixels the 2x2 average of the four above it (premultiplied,
//! truncated; a 1-2-1 filter across an odd row or column), and the draw
//! samples the deepest level no smaller than the drawn size on *either* axis
//! bilinearly. Measured against Chrome's screenshots (#2534) that model is
//! exact at 1/2 and 1/8 and within 0.2 a channel on average at 1/3. A box
//! filter to the drawn size, and Mitchell or Lanczos-3 resampling, are 6 to
//! 7 a channel off at 1/3: they blur what Chrome's level-then-bilinear keeps
//! sharp. #2456 first box-filtered to the drawn scale rounded up to a quarter
//! octave; that was softer than Chrome (#2534).
//!
//! So [`minified`] makes the level ([`mip_level`], [`target_size`]) of a draw
//! scaled to 1/2 or less on both axes, and the painter draws it the rest of the
//! way bilinearly (`ImageSampling::Smooth`
//! (crate::paint::painter::ImageSampling::Smooth), the software painter
//! included). A draw at level 0 (some axis above 1/2) samples the image
//! itself, bilinearly when some axis is below [`MINIFY_BELOW`] ([`smooth`]):
//! the software painter's own `Auto` is nearest, which at 0.6x drops a 1px
//! line in three. A picture whose scale moves continuously (a zoom, a
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
/// bilinearly ([`smooth`]), from its mipmap level when it has one.
pub const MINIFY_BELOW: f64 = 0.75;

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

/// The mipmap level a draw under `transform` samples: the deepest one no
/// smaller than the drawn size on either axis, `floor(log2(1 / s))` for `s`
/// the larger of the two axis scales, as Skia picks it (a draw at exactly
/// 1/2 is level 1 at 1:1). 0 (the image itself) for a draw above 1/2 on
/// some axis, or an unusable one.
pub fn mip_level(transform: Affine) -> u32 {
    let (sx, sy) = axis_scales(transform);
    let s = sx.max(sy);
    if !(s.is_finite() && s > 0.0 && s < 1.0) {
        return 0;
    }
    // Clamped so the shift below stays in range; a level past the image's
    // own size is 1x1 anyway.
    ((-s.log2() + 1e-9).floor() as u32).min(31)
}

/// The size of mipmap level `level` of a `w` x `h` image: each side halved
/// `level` times, rounding down, at least 1.
pub fn level_size(w: u32, h: u32, level: u32) -> (u32, u32) {
    ((w >> level).max(1), (h >> level).max(1))
}

/// The size of the copy a draw of `image` under `transform` (image pixels to
/// device) is drawn from: its [`mip_level`]'s size, or `None` when it is
/// drawn from the image itself: a vector image, an empty one, a draw at
/// level 0.
pub fn target_size(image: &DecodedImage, transform: Affine) -> Option<(u32, u32)> {
    if image.vector().is_some() || image.width == 0 || image.height == 0 {
        return None;
    }
    let size = level_size(image.width, image.height, mip_level(transform));
    (size != (image.width, image.height)).then_some(size)
}

/// Whether a draw of a raster image under `transform` should sample it
/// bilinearly: it scales some axis below [`MINIFY_BELOW`]. Chrome samples a
/// minified picture bilinearly from its level; the software painter's own
/// `Auto` is nearest, which drops whole source pixels there.
pub fn smooth(transform: Affine) -> bool {
    let (sx, sy) = axis_scales(transform);
    sx < MINIFY_BELOW || sy < MINIFY_BELOW
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

/// Straight-alpha RGBA8 `src` (`sw` x `sh`) reduced to its mipmap level of
/// size `dw` x `dh` ([`level_size`]), straight alpha, as Skia's `SkMipmap`
/// builds it: premultiplied, then halved level by level, each output pixel
/// the sum of its 2x2 source pixels shifted right by 2 (truncated). A side of
/// odd length is filtered 1-2-1 over the three source pixels centred on the
/// pair, shifted by one more; a side of length 1 stays 1.
pub fn mip_downscale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    debug_assert_eq!(src.len(), sw as usize * sh as usize * 4);
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
    let (mut w, mut h) = (sw as usize, sh as usize);
    while (w, h) != (dw as usize, dh as usize) && (w > 1 || h > 1) {
        level = std::borrow::Cow::Owned(halve(&level, w, h));
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
    }
    let mut out = level.into_owned();
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
        // A red opaque pixel beside a transparent *green* one, halved: the
        // result is red at half alpha (truncated, as Skia's), not a
        // red-green blend.
        let src = [255, 0, 0, 255, 0, 255, 0, 0];
        assert_eq!(mip_downscale(&src, 2, 1, 1, 1), vec![255, 0, 0, 127]);
        // The same through the 2x2 path: opaque red, transparent green,
        // half-alpha blue (premultiplied 128) and transparent black sum to
        // (255, 0, 128, 383) >> 2 = (63, 0, 32, 95), unpremultiplied.
        let src = [255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 128, 0, 0, 0, 0];
        assert_eq!(mip_downscale(&src, 2, 2, 1, 1), vec![169, 0, 86, 95]);
    }

    /// Each level is the 2x2 sum shifted right by 2 (truncated, so 1+1+1+2
    /// is 1, not 1.25 rounded), level by level; an odd side is 1-2-1.
    #[test]
    fn levels_truncate_like_skia() {
        let grey = |v: &[u8]| -> Vec<u8> { v.iter().flat_map(|&g| [g, g, g, 255]).collect() };
        // 2x2 -> 1x1: (1 + 1 + 1 + 2) >> 2 = 1.
        assert_eq!(mip_downscale(&grey(&[1, 1, 1, 2]), 2, 2, 1, 1), grey(&[1]));
        // 4x1 -> 1x1 by two levels: (10+13)>>1 = 11, (20+27)>>1 = 23, then
        // (11 + 23) >> 1 = 17; one 4-wide average would be 17.5.
        assert_eq!(
            mip_downscale(&grey(&[10, 13, 20, 27]), 4, 1, 1, 1),
            grey(&[17])
        );
        // 3x1 -> 1x1: (1*0 + 2*100 + 1*255) >> 2 = 113.
        assert_eq!(
            mip_downscale(&grey(&[0, 100, 255]), 3, 1, 1, 1),
            grey(&[113])
        );
        // 5x1 -> 2x1: output i is 1-2-1 over 2i, 2i+1, 2i+2.
        assert_eq!(
            mip_downscale(&grey(&[0, 40, 80, 120, 160]), 5, 1, 2, 1),
            grey(&[40, 120])
        );
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
        // Each side halved rounding down, never below 1.
        assert_eq!(target_size(&img, Affine::scale(0.1)), Some((37, 25)));
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
        // Bilinear below 0.75x on either axis.
        assert!(smooth(Affine::scale(0.74)));
        assert!(smooth(Affine::scale_non_uniform(2.0, 0.6)));
        assert!(!smooth(Affine::scale(0.75)));
    }
}
