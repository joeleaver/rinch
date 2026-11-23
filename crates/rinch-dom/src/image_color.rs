//! Colour management of decoded pictures (#2779): a raster file tagged with
//! an ICC profile that is not sRGB is converted to sRGB at decode, as Chrome
//! converts a tagged image to the output colour space (sRGB, since rinch has
//! no display profile). Untagged images are sRGB already, and are left alone.
//!
//! A grey picture tagged with a grey profile is converted too (#2807): its
//! grey+alpha values go through the grey curve to sRGB and come out as RGBA.
//! A picture decoded at 16 bits a channel is converted at 16 bits and rounded
//! to 8 once, after the transform (#2807), so a smooth wide-gamut gradient
//! does not band.
//!
//! The profile comes from where each decoder keeps it: a JPEG's `APP2`
//! `ICC_PROFILE` segments, a PNG's `iCCP` chunk, a WebP's `ICCP` chunk. The
//! CMS is [`moxcms`] (pure Rust, builds for wasm32). A profile is parsed and
//! its transform built once, then kept by the profile's hash
//! (`TRANSFORMS`): most photographs carry one of a handful of profiles, so
//! the next picture with the same one pays a hash of its bytes. A profile
//! whose transform leaves sRGB as it is (an sRGB profile, or one that agrees
//! with sRGB within a unit on every probe colour) is remembered as `None`, so
//! an sRGB-tagged picture costs no pass over its pixels.
//!
//! Off with [`set_color_management`]`(false)` (process-wide), for an app that
//! hands over files whose pixels it has already converted. Pixels handed over
//! decoded (`ImageLoadResult::Decoded`, `register_rgba`) are never touched.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use moxcms::{
    ColorProfile, DataColorSpace, Layout, Transform8BitExecutor, Transform16BitExecutor,
    TransformOptions,
};

static ENABLED: AtomicBool = AtomicBool::new(true);

/// Turn colour management of decoded pictures on or off for the process (on
/// by default). Off, an ICC-tagged file's pixels are drawn as if they were
/// sRGB, as before #2779. It applies to decodes from then on; a picture
/// already decoded keeps the pixels it was decoded with (reload it to decode
/// it again).
pub fn set_color_management(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Whether decoded pictures are colour-managed ([`set_color_management`]).
pub fn color_management() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The conversion of one profile's pixels to sRGB.
///
/// An RGB profile converts RGBA to RGBA; a grey profile converts grey+alpha
/// to RGBA (#2807), the step that also expands a grey picture to colour. The
/// 8-bit transform is built with the conversion; the 16-bit one, for a
/// picture decoded at 16 bits a channel (#2807), when the first such picture
/// asks for it (its tables are 65536 entries).
pub(crate) struct ToSrgb {
    profile: ColorProfile,
    /// The source layout: [`Layout::Rgba`] or [`Layout::GrayAlpha`].
    src: Layout,
    t8: Arc<Transform8BitExecutor>,
    t16: OnceLock<Option<Arc<Transform16BitExecutor>>>,
}

impl ToSrgb {
    fn is_gray(&self) -> bool {
        self.src == Layout::GrayAlpha
    }

    fn t16(&self) -> Option<&Transform16BitExecutor> {
        self.t16
            .get_or_init(|| {
                self.profile
                    .create_transform_16bit(
                        self.src,
                        &ColorProfile::new_srgb(),
                        Layout::Rgba,
                        TransformOptions::default(),
                    )
                    .ok()
            })
            .as_deref()
    }
}

/// A conversion to sRGB, or `None` for a profile that needs none.
type Cached = Option<Arc<ToSrgb>>;

/// Conversions by profile hash. Bounded: emptied when it reaches
/// [`MAX_TRANSFORMS`]. An entry is its transforms' lookup tables (tens of KB
/// for 8 bits, a few hundred once a 16-bit one is built) and the parsed
/// profile, kept to build the 16-bit transform; a profile with LUT tags
/// (`A2B`/`B2A`) carries those tables too, which can run to megabytes.
static TRANSFORMS: LazyLock<Mutex<HashMap<u64, Cached>>> = LazyLock::new(Default::default);
const MAX_TRANSFORMS: usize = 32;

/// The conversion that brings pixels in `icc`'s colour space to sRGB, or
/// `None` when they are sRGB already, the profile does not parse, or it is
/// neither an RGB nor a grey profile (a CMYK profile: the decoder has already
/// made RGB of those pixels by its own rule, and they are left as it made
/// them).
pub(crate) fn to_srgb(icc: &[u8]) -> Cached {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    icc.hash(&mut h);
    let key = h.finish();
    if let Some(cached) = TRANSFORMS.lock().unwrap().get(&key) {
        return cached.clone();
    }
    let made = build(icc).map(Arc::new);
    let mut map = TRANSFORMS.lock().unwrap();
    if map.len() >= MAX_TRANSFORMS {
        map.clear();
    }
    map.insert(key, made.clone());
    made
}

fn build(icc: &[u8]) -> Option<ToSrgb> {
    let profile = ColorProfile::new_from_slice(icc).ok()?;
    let src = match profile.color_space {
        DataColorSpace::Rgb => Layout::Rgba,
        DataColorSpace::Gray => Layout::GrayAlpha,
        _ => return None,
    };
    let t8 = profile
        .create_transform_8bit(
            src,
            &ColorProfile::new_srgb(),
            Layout::Rgba,
            TransformOptions::default(),
        )
        .ok()?;
    if is_identity(t8.as_ref(), src) {
        return None;
    }
    Some(ToSrgb {
        profile,
        src,
        t8,
        t16: OnceLock::new(),
    })
}

/// Whether `t` leaves every probe colour within one unit of itself: for RGB,
/// each channel one of 0, 64, 128, 192, 255; for grey, every grey level
/// (which must come out as that grey in R, G and B).
fn is_identity(t: &Transform8BitExecutor, src_layout: Layout) -> bool {
    const LEVELS: [u8; 5] = [0, 64, 128, 192, 255];
    let (src, want): (Vec<u8>, Vec<u8>) = if src_layout == Layout::GrayAlpha {
        let src = (0..=255u8).flat_map(|g| [g, 255]).collect();
        let want = (0..=255u8).flat_map(|g| [g, g, g, 255]).collect();
        (src, want)
    } else {
        let mut src = Vec::with_capacity(125 * 4);
        for r in LEVELS {
            for g in LEVELS {
                for b in LEVELS {
                    src.extend_from_slice(&[r, g, b, 255]);
                }
            }
        }
        (src.clone(), src)
    };
    let mut dst = vec![0; want.len()];
    if t.transform(&src, &mut dst).is_err() {
        return false;
    }
    want.iter().zip(&dst).all(|(a, b)| a.abs_diff(*b) <= 1)
}

/// Convert straight-alpha RGBA8 `pixels` (`width` a row) in place with `t`,
/// a row at a time through one row of scratch. Alpha is carried through.
fn convert_rgba8(t: &Transform8BitExecutor, pixels: &mut [u8], width: u32) {
    let row = width as usize * 4;
    if row == 0 {
        return;
    }
    let mut scratch = vec![0u8; row];
    for line in pixels.chunks_exact_mut(row) {
        if t.transform(line, &mut scratch).is_ok() {
            line.copy_from_slice(&scratch);
        }
    }
}

/// A 16-bit sample rounded to the nearest 8-bit one.
fn to_u8(v: u16) -> u8 {
    ((u32::from(v) * 255 + 32767) / 65535) as u8
}

/// Run `t` over `src` (`width` pixels a row of `src_ch` samples each, which
/// `widen` turns into the transform's source layout, `t_ch` samples a pixel),
/// a row at a time, into the RGBA8 `out`: each row is widened into scratch,
/// transformed to RGBA at the transform's depth, and `narrow`ed to 8 bits.
#[allow(clippy::too_many_arguments)]
fn convert_rows<S: Copy, T: Copy + Default>(
    src: &[S],
    width: usize,
    src_ch: usize,
    t_ch: usize,
    widen: impl Fn(&[S], &mut [T]),
    transform: impl Fn(&[T], &mut [T]) -> bool,
    narrow: impl Fn(T) -> u8,
    out: &mut [u8],
) {
    if width == 0 {
        return;
    }
    let mut wide = vec![T::default(); width * t_ch];
    let mut rgba = vec![T::default(); width * 4];
    for (line, dst) in src
        .chunks_exact(width * src_ch)
        .zip(out.chunks_exact_mut(width * 4))
    {
        for (px, w) in line.chunks_exact(src_ch).zip(wide.chunks_exact_mut(t_ch)) {
            widen(px, w);
        }
        if transform(&wide, &mut rgba) {
            for (d, s) in dst.iter_mut().zip(&rgba) {
                *d = narrow(*s);
            }
        } else {
            // The row unconverted, as it would be drawn untagged.
            for (d, w) in dst
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(wide.chunks_exact(t_ch))
            {
                let px = if t_ch == 2 {
                    [w[0], w[0], w[0], w[1]]
                } else {
                    [w[0], w[1], w[2], w[3]]
                };
                for (d, s) in d.iter_mut().zip(px) {
                    *d = narrow(s);
                }
            }
        }
    }
}

/// The decoded `img` as straight-alpha RGBA8 in sRGB, converted by `t`.
///
/// An RGB profile converts a picture decoded at 16 bits a channel (or in
/// floating point) with the 16-bit transform, and rounds to 8 bits once,
/// after it (#2807: rounding first and again after an 8-bit transform bands
/// a smooth gradient); an 8-bit picture is reduced to RGBA8 and converted in
/// place. A grey profile converts a grey picture's grey+alpha values
/// straight to RGBA, at 8 or 16 bits as decoded. A picture whose channels do
/// not match its profile's (grey pixels with an RGB profile, colour pixels
/// with a grey one) is converted as RGBA with an RGB profile and left as
/// decoded with a grey one.
pub(crate) fn convert(t: &ToSrgb, img: image::DynamicImage) -> image::RgbaImage {
    use image::DynamicImage as D;
    let (w, h) = (img.width(), img.height());
    let width = w as usize;
    let out = || image::RgbaImage::new(w, h);
    let is_16 = |img: &D| {
        matches!(
            img,
            D::ImageLuma16(_)
                | D::ImageLumaA16(_)
                | D::ImageRgb16(_)
                | D::ImageRgba16(_)
                | D::ImageRgb32F(_)
                | D::ImageRgba32F(_)
        )
    };
    let is_grey = |img: &D| {
        matches!(
            img,
            D::ImageLuma8(_) | D::ImageLumaA8(_) | D::ImageLuma16(_) | D::ImageLumaA16(_)
        )
    };
    if t.is_gray() {
        if !is_grey(&img) {
            return img.into_rgba8();
        }
        if is_16(&img)
            && let Some(t16) = t.t16()
        {
            let mut rgba = out();
            let run = |s: &[u16], ch: usize, rgba: &mut [u8]| {
                convert_rows(
                    s,
                    width,
                    ch,
                    2,
                    |p: &[u16], w: &mut [u16]| {
                        w[0] = p[0];
                        w[1] = if ch == 2 { p[1] } else { u16::MAX };
                    },
                    |a, b| t16.transform(a, b).is_ok(),
                    to_u8,
                    rgba,
                )
            };
            match &img {
                D::ImageLuma16(b) => run(b.as_raw(), 1, &mut rgba),
                D::ImageLumaA16(b) => run(b.as_raw(), 2, &mut rgba),
                _ => run(img.to_luma_alpha16().as_raw(), 2, &mut rgba),
            }
            return rgba;
        }
        let mut rgba = out();
        let run = |s: &[u8], ch: usize, rgba: &mut [u8]| {
            convert_rows(
                s,
                width,
                ch,
                2,
                |p: &[u8], w: &mut [u8]| {
                    w[0] = p[0];
                    w[1] = if ch == 2 { p[1] } else { u8::MAX };
                },
                |a, b| t.t8.transform(a, b).is_ok(),
                |v| v,
                rgba,
            )
        };
        match &img {
            D::ImageLuma8(b) => run(b.as_raw(), 1, &mut rgba),
            D::ImageLumaA8(b) => run(b.as_raw(), 2, &mut rgba),
            _ => run(img.to_luma_alpha8().as_raw(), 2, &mut rgba),
        }
        return rgba;
    }
    if is_16(&img)
        && let Some(t16) = t.t16()
    {
        let mut rgba = out();
        let run = |s: &[u16], ch: usize, rgba: &mut [u8]| {
            convert_rows(
                s,
                width,
                ch,
                4,
                |p: &[u16], w: &mut [u16]| {
                    w[..3].copy_from_slice(&p[..3]);
                    w[3] = if ch == 4 { p[3] } else { u16::MAX };
                },
                |a, b| t16.transform(a, b).is_ok(),
                to_u8,
                rgba,
            )
        };
        match &img {
            D::ImageRgb16(b) => run(b.as_raw(), 3, &mut rgba),
            D::ImageRgba16(b) => run(b.as_raw(), 4, &mut rgba),
            _ => run(img.to_rgba16().as_raw(), 4, &mut rgba),
        }
        return rgba;
    }
    let mut rgba = img.into_rgba8();
    convert_rgba8(t.t8.as_ref(), &mut rgba, w);
    rgba
}

/// The ICC profile of a JPEG: its `APP2` `ICC_PROFILE` segments, joined in
/// their sequence order, walking the marker segments up to the scan. `None`
/// for none, or a set with a missing or repeated part.
pub(crate) fn jpeg_icc_profile(bytes: &[u8]) -> Option<Vec<u8>> {
    const TAG: &[u8] = b"ICC_PROFILE\0";
    let mut parts: Vec<(u8, &[u8])> = Vec::new();
    let mut count = 0u8;
    crate::image_metadata::jpeg_segments(bytes, |marker, data| {
        if marker == 0xE2
            && let Some([seq, n, chunk @ ..]) = data.strip_prefix(TAG)
        {
            count = *n;
            parts.push((*seq, chunk));
        }
        true
    });
    if parts.is_empty() || count == 0 || parts.len() != usize::from(count) {
        return None;
    }
    parts.sort_by_key(|p| p.0);
    if parts
        .iter()
        .enumerate()
        .any(|(i, p)| usize::from(p.0) != i + 1)
    {
        return None;
    }
    Some(parts.iter().flat_map(|p| p.1.iter().copied()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_srgb_profile_needs_no_transform() {
        let srgb = ColorProfile::new_srgb().encode().unwrap();
        assert!(to_srgb(&srgb).is_none());
    }

    #[test]
    fn a_display_p3_profile_needs_one() {
        let p3 = ColorProfile::new_display_p3().encode().unwrap();
        assert!(to_srgb(&p3).is_some());
    }

    #[test]
    fn a_profile_that_does_not_parse_is_left_alone() {
        assert!(to_srgb(b"not a profile").is_none());
    }

    /// One profile in three `APP2` segments, out of order.
    #[test]
    fn a_jpeg_profile_in_several_segments_is_joined_in_order() {
        let seg = |seq: u8, data: &[u8]| {
            let mut payload = b"ICC_PROFILE\0".to_vec();
            payload.extend_from_slice(&[seq, 3]);
            payload.extend_from_slice(data);
            let mut s = vec![0xFF, 0xE2];
            s.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
            s.extend(payload);
            s
        };
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend(seg(2, b"cd"));
        jpeg.extend(seg(1, b"ab"));
        jpeg.extend(seg(3, b"ef"));
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
        assert_eq!(jpeg_icc_profile(&jpeg).as_deref(), Some(&b"abcdef"[..]));
        // A part missing is no profile.
        let mut short = vec![0xFF, 0xD8];
        short.extend(seg(1, b"ab"));
        short.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        assert_eq!(jpeg_icc_profile(&short), None);
    }
}
