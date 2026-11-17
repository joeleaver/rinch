//! Colour management of decoded pictures (#2779): a raster file tagged with
//! an ICC profile that is not sRGB is converted to sRGB at decode, as Chrome
//! converts a tagged image to the output colour space (sRGB, since rinch has
//! no display profile). Untagged images are sRGB already, and are left alone.
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
use std::sync::{Arc, LazyLock, Mutex};

use moxcms::{ColorProfile, DataColorSpace, Layout, Transform8BitExecutor, TransformOptions};

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

/// A transform to sRGB, or `None` for a profile that needs none.
type Cached = Option<Arc<Transform8BitExecutor>>;

/// Transforms by profile hash. Bounded: emptied when it reaches
/// [`MAX_TRANSFORMS`] (a transform is a few lookup tables, tens of KB).
static TRANSFORMS: LazyLock<Mutex<HashMap<u64, Cached>>> = LazyLock::new(Default::default);
const MAX_TRANSFORMS: usize = 32;

/// The transform that brings pixels in `icc`'s colour space to sRGB, or
/// `None` when they are sRGB already, the profile does not parse, or it is not
/// an RGB profile (a grey or CMYK profile: the decoder has already made RGB of
/// those pixels by its own rule, and they are left as it made them).
pub(crate) fn to_srgb(icc: &[u8]) -> Cached {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    icc.hash(&mut h);
    let key = h.finish();
    if let Some(cached) = TRANSFORMS.lock().unwrap().get(&key) {
        return cached.clone();
    }
    let made = build(icc);
    let mut map = TRANSFORMS.lock().unwrap();
    if map.len() >= MAX_TRANSFORMS {
        map.clear();
    }
    map.insert(key, made.clone());
    made
}

fn build(icc: &[u8]) -> Cached {
    let src = ColorProfile::new_from_slice(icc).ok()?;
    if src.color_space != DataColorSpace::Rgb {
        return None;
    }
    let transform = src
        .create_transform_8bit(
            Layout::Rgba,
            &ColorProfile::new_srgb(),
            Layout::Rgba,
            TransformOptions::default(),
        )
        .ok()?;
    (!is_identity(transform.as_ref())).then_some(transform)
}

/// Whether `t` leaves every probe colour (each channel one of 0, 64, 128,
/// 192, 255) within one unit of itself.
fn is_identity(t: &Transform8BitExecutor) -> bool {
    const LEVELS: [u8; 5] = [0, 64, 128, 192, 255];
    let mut src = Vec::with_capacity(125 * 4);
    for r in LEVELS {
        for g in LEVELS {
            for b in LEVELS {
                src.extend_from_slice(&[r, g, b, 255]);
            }
        }
    }
    let mut dst = vec![0; src.len()];
    if t.transform(&src, &mut dst).is_err() {
        return false;
    }
    src.iter().zip(&dst).all(|(a, b)| a.abs_diff(*b) <= 1)
}

/// Convert straight-alpha RGBA8 `pixels` (`width` a row) in place with `t`,
/// a row at a time through one row of scratch. Alpha is carried through.
pub(crate) fn convert(t: &Transform8BitExecutor, pixels: &mut [u8], width: u32) {
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
