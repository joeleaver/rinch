//! Minified copies of decoded images (#2456): a raster image drawn well
//! below its natural size is drawn from a copy box-filtered down to the size
//! it covers on the device, not sampled straight from its full pixels.
//!
//! Both painters sample an `<img>` from its own pixels: the software painter
//! nearest-neighbour, Vello bilinearly, neither with mipmaps. Drawn at 0.6x,
//! the samples are 1.67 source pixels apart, so a 1px line falls between two
//! of them and is dropped (nearest) or kept at a sixth of its contrast
//! (bilinear). Thin strokes of small text baked into a picture — the bars of
//! an E, the tail of a $ — go with it. Chrome filters a minified image with
//! mipmaps (Skia's `kMedium` filter quality), where every source pixel
//! counts.
//!
//! A box filter at the drawn size counts every source pixel by the area it
//! covers on the device, which is what a mipmap pyramid approximates. The
//! copy is drawn at about 1:1, so the painter's own sampling no longer
//! decides which source pixels survive. It is made in premultiplied space,
//! as Skia filters (a transparent pixel's colour does not bleed), in the
//! pixels' own sRGB encoding, as Chrome does.
//!
//! Only a draw scaled below [`MINIFY_BELOW`] on some axis is minified: from
//! there up to 1x the bilinear samples are close enough that every line keeps
//! most of its contrast. `image-rendering: pixelated`/`crisp-edges` asks for
//! nearest-neighbour and is never minified.
//!
//! The copies live in a per-thread cache like the SVG rasters'
//! ([`crate::svg_image`]): keyed by the image's identity and the copy's size,
//! held within [`MINIFY_BUDGET_BYTES`], let go after [`MINIFY_IDLE_FRAMES`]
//! painted frames unused. Each copy is a [`DecodedImage`] of its own, so the
//! software painter keeps its premultiplied form and Vello one atlas image
//! for it across frames. A frame makes copies of at most
//! [`MAX_MINIFY_WORK_PER_FRAME`] source pixels; past that (an image scaled by
//! an animation makes a new size every frame) it draws the full image, as
//! before.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use peniko::kurbo::Affine;

use crate::image_cache::DecodedImage;

/// A draw is minified when it scales the image below this on either axis.
pub const MINIFY_BELOW: f64 = 0.75;

/// The bytes every minified copy on one thread may hold together: 64 MB,
/// charged at 8 bytes a pixel (the copy, and the painter's premultiplied copy
/// or GPU blob of it), as the SVG rasters are.
pub const MINIFY_BUDGET_BYTES: usize = 64 << 20;

/// Painted frames a copy may go unused before it is let go.
pub const MINIFY_IDLE_FRAMES: u64 = 60;

/// The most source pixels one painted frame box-filters into new copies:
/// 16M, about 80 ms at the 5 ns a source pixel measured on a 4000x3000
/// picture (7 ns at 1920x1080, 11 ns at 284x442 — 1.4 ms; release build,
/// #2456). An image that would pass it is drawn from its full pixels this
/// frame.
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

/// The size a draw of `image` under `transform` (image pixels to device)
/// is minified to, or `None` when it is not minified: a vector image, an
/// empty one, a draw no axis of which is scaled below [`MINIFY_BELOW`].
/// An axis scaled up keeps its natural size.
pub fn target_size(image: &DecodedImage, transform: Affine) -> Option<(u32, u32)> {
    if image.vector().is_some() || image.width == 0 || image.height == 0 {
        return None;
    }
    let [a, b, c, d, _, _] = transform.as_coeffs();
    let (sx, sy) = (a.hypot(b), c.hypot(d));
    if !(sx.is_finite() && sy.is_finite()) || !(sx < MINIFY_BELOW || sy < MINIFY_BELOW) {
        return None;
    }
    let axis = |n: u32, s: f64| -> u32 {
        if s >= 1.0 {
            n
        } else {
            ((f64::from(n) * s).round() as u32).clamp(1, n)
        }
    };
    let size = (axis(image.width, sx), axis(image.height, sy));
    (size != (image.width, image.height)).then_some(size)
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
        if c.work + src_px > MAX_MINIFY_WORK_PER_FRAME {
            return Some(None);
        }
        c.work += src_px;
        None
    });
    if let Some(hit) = hit {
        return hit;
    }
    let pixels = box_downscale(&image.data, image.width, image.height, w, h);
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

/// For each of `dst` output pixels along an axis of `src` input pixels
/// (`dst <= src`), the first input pixel it covers and the share of the
/// output each covered input pixel has (summing to 1): the input interval
/// `[i * src / dst, (i + 1) * src / dst)` by overlap.
fn box_weights(src: u32, dst: u32) -> Vec<(usize, Vec<f32>)> {
    let ratio = f64::from(src) / f64::from(dst);
    (0..dst)
        .map(|i| {
            let a = f64::from(i) * ratio;
            let b = (f64::from(i + 1) * ratio).min(f64::from(src));
            let start = a.floor() as usize;
            let end = (b.ceil() as usize).min(src as usize);
            let weights = (start..end)
                .map(|s| {
                    let lo = a.max(s as f64);
                    let hi = b.min(s as f64 + 1.0);
                    ((hi - lo).max(0.0) / ratio) as f32
                })
                .collect();
            (start, weights)
        })
        .collect()
}

/// Straight-alpha RGBA8 `src` (`sw` x `sh`) box-filtered to `dw` x `dh`
/// (each no larger than the source's), straight alpha. The filter runs on
/// premultiplied values, so a transparent pixel adds no colour; one output
/// row at a time, so it needs one row of floats beside the output.
pub fn box_downscale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    debug_assert!(dw <= sw && dh <= sh && dw > 0 && dh > 0);
    debug_assert_eq!(src.len(), sw as usize * sh as usize * 4);
    let cols = box_weights(sw, dw);
    let rows = box_weights(sh, dh);
    let sw = sw as usize;
    let mut acc = vec![0f32; sw * 4];
    let mut out = Vec::with_capacity(dw as usize * dh as usize * 4);
    for (row0, row_weights) in &rows {
        acc.fill(0.0);
        for (k, &wy) in row_weights.iter().enumerate() {
            let at = (row0 + k) * sw * 4;
            let line = &src[at..at + sw * 4];
            for (px, a) in line
                .as_chunks::<4>()
                .0
                .iter()
                .zip(acc.as_chunks_mut::<4>().0)
            {
                let alpha = f32::from(px[3]) * wy;
                a[0] += f32::from(px[0]) * alpha;
                a[1] += f32::from(px[1]) * alpha;
                a[2] += f32::from(px[2]) * alpha;
                a[3] += alpha;
            }
        }
        for (col0, col_weights) in &cols {
            let mut sum = [0f32; 4];
            for (k, &wx) in col_weights.iter().enumerate() {
                let a = &acc[(col0 + k) * 4..(col0 + k) * 4 + 4];
                for (s, v) in sum.iter_mut().zip(a) {
                    *s += v * wx;
                }
            }
            let alpha = sum[3];
            if alpha <= 0.0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            for &c in &sum[..3] {
                out.push((c / alpha + 0.5).clamp(0.0, 255.0) as u8);
            }
            out.push((alpha + 0.5).clamp(0.0, 255.0) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_cover_each_output_once() {
        for (s, d) in [(300, 180), (442, 278), (7, 3), (5, 5)] {
            let w = box_weights(s, d);
            assert_eq!(w.len(), d as usize);
            for (_, ws) in &w {
                let sum: f32 = ws.iter().sum();
                assert!((sum - 1.0).abs() < 1e-4, "{s}->{d}: {sum}");
            }
            // Every input pixel is counted in full across the outputs.
            let mut per_input = vec![0f32; s as usize];
            for (start, ws) in &w {
                for (k, v) in ws.iter().enumerate() {
                    per_input[start + k] += v * s as f32 / d as f32;
                }
            }
            assert!(per_input.iter().all(|v| (v - 1.0).abs() < 1e-3), "{s}->{d}");
        }
    }

    #[test]
    fn transparent_pixels_add_no_colour() {
        // A red opaque pixel beside a transparent *green* one, halved: the
        // result is red at half alpha, not a red-green blend.
        let src = [255, 0, 0, 255, 0, 255, 0, 0];
        assert_eq!(box_downscale(&src, 2, 1, 1, 1), vec![255, 0, 0, 128]);
    }

    #[test]
    fn target_size_by_axis() {
        let img = DecodedImage::new(vec![0; 300 * 200 * 4], 300, 200);
        assert_eq!(target_size(&img, Affine::scale(0.6)), Some((180, 120)));
        // At or above the threshold: drawn as it is.
        assert_eq!(target_size(&img, Affine::scale(0.75)), None);
        // One axis minified, the other scaled up: that one keeps its size.
        assert_eq!(
            target_size(&img, Affine::scale_non_uniform(0.5, 2.0)),
            Some((150, 200))
        );
        // A rotation does not change the scale.
        assert_eq!(
            target_size(&img, Affine::rotate(0.7) * Affine::scale(0.5)),
            Some((150, 100))
        );
    }
}
