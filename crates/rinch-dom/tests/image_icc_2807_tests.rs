//! #2807: a 16-bit ICC-tagged picture is converted at 16 bits, and a grey
//! picture's grey ICC profile is applied.
//!
//! #2779 converted tagged pictures to sRGB with an 8-bit transform after the
//! decode was reduced to RGBA8, so a 16-bit Display P3 gradient was rounded
//! to 8 bits twice (before and after the transform) and banded; and a grey
//! profile was not applied at all, so a grey picture tagged with a linear (or
//! any non-sRGB) grey curve was drawn as if its values were sRGB. Chrome
//! converts the 16-bit values and rounds once, and applies the grey curve.
//!
//! The expected values are worked out here from the published Display P3 and
//! sRGB primaries and the transfer curves, in `f64`, not from the CMS under
//! test.
#![cfg(feature = "color-management")]

use image::ImageEncoder;
use rinch_dom::image_cache::decode_image_bytes;

fn to_linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// A Display P3 colour (each channel 0..=1) as sRGB in 0..=255, unrounded,
/// clipped to the sRGB gamut.
fn p3_to_srgb(c: [f64; 3]) -> [f64; 3] {
    const P3_TO_XYZ: [[f64; 3]; 3] = [
        [0.486_570_9, 0.265_667_7, 0.198_217_3],
        [0.228_974_6, 0.691_738_5, 0.079_286_9],
        [0.0, 0.045_113_4, 1.043_944_4],
    ];
    const XYZ_TO_SRGB: [[f64; 3]; 3] = [
        [3.240_454_2, -1.537_138_5, -0.498_531_4],
        [-0.969_266_0, 1.876_010_8, 0.041_556_0],
        [0.055_643_4, -0.204_025_9, 1.057_225_2],
    ];
    let lin = c.map(to_linear);
    let mul = |m: [[f64; 3]; 3], v: [f64; 3]| {
        [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
    };
    mul(XYZ_TO_SRGB, mul(P3_TO_XYZ, lin)).map(|v| from_linear(v) * 255.0)
}

/// Encode a PNG of `samples` (native-endian `u16`s, as `image` takes them)
/// with `icc` in its `iCCP` chunk.
fn png16(w: u32, h: u32, samples: &[u16], ty: image::ExtendedColorType, icc: Vec<u8>) -> Vec<u8> {
    let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_ne_bytes()).collect();
    png8(w, h, &bytes, ty, icc)
}

fn png8(w: u32, h: u32, bytes: &[u8], ty: image::ExtendedColorType, icc: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = image::codecs::png::PngEncoder::new(&mut out);
    enc.set_icc_profile(icc).unwrap();
    enc.write_image(bytes, w, h, ty).unwrap();
    out
}

fn p3_profile() -> Vec<u8> {
    moxcms::ColorProfile::new_display_p3().encode().unwrap()
}

fn grey_profile(gamma: f32) -> Vec<u8> {
    moxcms::ColorProfile::new_gray_with_gamma(gamma)
        .encode()
        .unwrap()
}

/// The P3 gradient: 4096 steps from black to a P3 orange
/// (`GRADIENT_END`, inside the sRGB gamut), 16 bits a channel. Each 16-bit
/// step is a sixteenth of an 8-bit one, so most steps lie between 8-bit
/// codes: rounding to 8 bits first moves them.
const STEPS: u32 = 4096;
const GRADIENT_END: [f64; 3] = [0.85, 0.55, 0.30];

fn gradient_samples() -> Vec<u16> {
    (0..STEPS)
        .flat_map(|i| {
            let t = f64::from(i) / f64::from(STEPS - 1);
            GRADIENT_END.map(|e| (e * t * 65535.0).round() as u16)
        })
        .collect()
}

/// The sRGB value of gradient step `i` from its 16-bit P3 samples, unrounded.
fn gradient_reference(samples: &[u16], i: usize) -> [f64; 3] {
    let s = &samples[i * 3..i * 3 + 3];
    p3_to_srgb([0, 1, 2].map(|c| f64::from(s[c]) / 65535.0))
}

/// Each converted step is within 1 of the sRGB value worked out from its
/// 16-bit samples, and the steps rise monotonically in every channel. The
/// 8-bit path (#2779) misses the reference by up to 2 and its error is
/// several times larger on average.
#[test]
fn a_16_bit_display_p3_gradient_is_converted_at_16_bits() {
    let samples = gradient_samples();
    let bytes = png16(
        STEPS,
        1,
        &samples,
        image::ExtendedColorType::Rgb16,
        p3_profile(),
    );
    let img = decode_image_bytes(&bytes).unwrap();
    assert_eq!((img.width, img.height), (STEPS, 1));
    let mut worst = 0f64;
    let mut total = 0f64;
    for i in 0..STEPS as usize {
        let want = gradient_reference(&samples, i);
        for (c, want) in want.iter().enumerate() {
            let got = f64::from(img.data[i * 4 + c]);
            let err = (got - want).abs();
            worst = worst.max(err);
            total += err;
            if i > 0 {
                assert!(
                    img.data[i * 4 + c] >= img.data[(i - 1) * 4 + c],
                    "step {i} channel {c} falls"
                );
            }
        }
        assert_eq!(img.data[i * 4 + 3], 255, "step {i}: alpha");
    }
    let mean = total / f64::from(STEPS * 3);
    // Rounding once to the nearest code is within 0.5; the CMS's own tables
    // add a little.
    assert!(worst <= 1.0, "worst error {worst:.3}");
    assert!(mean <= 0.3, "mean error {mean:.3}");
    // Off the fixed point: the gradient's colours move in conversion.
    let end = gradient_reference(&samples, STEPS as usize - 1);
    assert!((end[0] - 0.85 * 255.0).abs() > 10.0, "{end:?}");
}

/// A 16-bit RGBA picture keeps its alpha (rounded to 8 bits) and converts
/// its colour at 16 bits.
#[test]
fn a_16_bit_rgba_picture_keeps_its_alpha() {
    // Two colours between 8-bit codes, at alphas whose nearest 8-bit value
    // is not their high byte: 0xFF00 is 254.0 (high byte 255), 0x01C0 is
    // 1.74 (high byte 1), so rounding and truncating give different answers.
    let samples: Vec<u16> = vec![
        0x8080, 0x6040, 0x2010, 0xFF00, //
        0xC0C0, 0x3000, 0x9050, 0x01C0,
    ];
    let bytes = png16(
        2,
        1,
        &samples,
        image::ExtendedColorType::Rgba16,
        p3_profile(),
    );
    let img = decode_image_bytes(&bytes).unwrap();
    for (px, alpha) in [(0usize, 254u8), (1, 2)] {
        let s = &samples[px * 4..px * 4 + 3];
        let want = p3_to_srgb([0, 1, 2].map(|c| f64::from(s[c]) / 65535.0));
        for (c, want) in want.iter().enumerate() {
            let got = f64::from(img.data[px * 4 + c]);
            assert!(
                (got - want).abs() <= 1.0,
                "pixel {px} channel {c}: got {got}, want {want:.2}"
            );
        }
        assert_eq!(img.data[px * 4 + 3], alpha, "pixel {px}: alpha");
    }
}

/// A 16-bit grey+alpha PNG with a linear grey profile: the grey goes
/// through the curve at 16 bits and the alpha is carried through, rounded to
/// 8 bits (alphas chosen as in `a_16_bit_rgba_picture_keeps_its_alpha`).
#[test]
fn a_16_bit_grey_alpha_picture_keeps_its_alpha() {
    let samples: Vec<u16> = vec![0x0203, 0xFF00, 0x4321, 0x01C0, 0x9000, 0x8000];
    let bytes = png16(
        3,
        1,
        &samples,
        image::ExtendedColorType::La16,
        grey_profile(1.0),
    );
    let img = decode_image_bytes(&bytes).unwrap();
    for (px, alpha) in [(0usize, 254u8), (1, 2), (2, 128)] {
        let g = samples[px * 2];
        let want = from_linear(f64::from(g) / 65535.0) * 255.0;
        let got = &img.data[px * 4..px * 4 + 4];
        for c in 0..3 {
            assert!(
                (f64::from(got[c]) - want).abs() <= 1.0,
                "pixel {px}: got {got:?}, want {want:.2}"
            );
        }
        assert_eq!(got[3], alpha, "pixel {px}: alpha");
    }
}

/// An 8-bit grey PNG tagged with a linear grey profile (gamma 1.0): each
/// grey value `v` is linear light `v / 255`, so it draws as the sRGB
/// encoding of that, the same in R, G and B. Alpha is carried through.
#[test]
fn a_grey_profile_is_applied_to_a_grey_picture() {
    let greys: [u8; 6] = [0, 16, 64, 128, 200, 255];
    // Grey + alpha, the last two at partial alpha.
    let mut la = Vec::new();
    for (i, g) in greys.iter().enumerate() {
        la.extend_from_slice(&[*g, if i >= 4 { 100 } else { 255 }]);
    }
    let bytes = png8(6, 1, &la, image::ExtendedColorType::La8, grey_profile(1.0));
    let img = decode_image_bytes(&bytes).unwrap();
    for (i, g) in greys.iter().enumerate() {
        let want = from_linear(f64::from(*g) / 255.0) * 255.0;
        let px = &img.data[i * 4..i * 4 + 4];
        for c in 0..3 {
            assert!(
                (f64::from(px[c]) - want).abs() <= 1.0,
                "grey {g}: got {px:?}, want {want:.2}"
            );
        }
        assert_eq!(px[3], la[i * 2 + 1], "grey {g}: alpha");
    }
    // Off the fixed point: linear 64 is sRGB 137, not 64.
    assert!(from_linear(64.0 / 255.0) * 255.0 > 130.0);
}

/// 16-bit grey PNGs (no alpha) with gamma grey profiles, converted at 16
/// bits: each value within 1 of `encode_srgb((v / 65535) ^ gamma)`. The
/// second case is a ramp through the darkest 8-bit codes of a linear
/// profile, where the sRGB curve is steepest: one 8-bit grey code there is
/// about 13 sRGB codes, so a picture reduced to 8 bits before the transform
/// misses most of the ramp's values.
#[test]
fn a_16_bit_grey_picture_with_a_gamma_profile() {
    let full: Vec<u16> = (0..256u32).map(|i| (i * 257 + 0x40) as u16).collect();
    let dark: Vec<u16> = (0..256u32).map(|i| (i * 8 + 3) as u16).collect();
    for (gamma, greys) in [(2.2f32, full), (1.0, dark)] {
        let bytes = png16(
            256,
            1,
            &greys,
            image::ExtendedColorType::L16,
            grey_profile(gamma),
        );
        let img = decode_image_bytes(&bytes).unwrap();
        for (i, g) in greys.iter().enumerate() {
            let want = from_linear((f64::from(*g) / 65535.0).powf(f64::from(gamma))) * 255.0;
            let px = &img.data[i * 4..i * 4 + 4];
            for c in 0..3 {
                assert!(
                    (f64::from(px[c]) - want).abs() <= 1.0,
                    "gamma {gamma} grey {g}: got {px:?}, want {want:.2}"
                );
            }
            assert_eq!(px[3], 255);
        }
    }
}

/// A grey picture tagged with the sRGB curve as its grey curve, or not
/// tagged, decodes to exactly the `image` crate's plain decode.
#[test]
fn an_srgb_grey_profile_or_none_leaves_grey_pixels_alone() {
    let mut srgb_grey = moxcms::ColorProfile::new_gray_with_gamma(1.0);
    srgb_grey.gray_trc = moxcms::ColorProfile::new_srgb().red_trc;
    let srgb_grey = srgb_grey.encode().unwrap();
    let greys: Vec<u8> = (0..=255).collect();
    let tagged = png8(256, 1, &greys, image::ExtendedColorType::L8, srgb_grey);
    let mut untagged = Vec::new();
    image::codecs::png::PngEncoder::new(&mut untagged)
        .write_image(&greys, 256, 1, image::ExtendedColorType::L8)
        .unwrap();
    for (name, bytes) in [("sRGB grey", tagged), ("untagged", untagged)] {
        let img = decode_image_bytes(&bytes).unwrap();
        let plain = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert!(*img.data == *plain.as_raw(), "{name}: pixels changed");
    }
}

/// Writes the fixtures to `$RINCH_2807_OUT` with rinch's decode of each,
/// for the comparison with Chrome. Run with `--ignored`.
#[test]
#[ignore]
fn write_fixtures_for_chrome() {
    let dir = std::path::PathBuf::from(std::env::var("RINCH_2807_OUT").unwrap());
    let samples = gradient_samples();
    let files = [
        (
            "p3-gradient-16.png",
            png16(
                STEPS,
                1,
                &samples,
                image::ExtendedColorType::Rgb16,
                p3_profile(),
            ),
        ),
        (
            "grey-linear-8.png",
            png8(
                256,
                1,
                &(0..=255).collect::<Vec<u8>>(),
                image::ExtendedColorType::L8,
                grey_profile(1.0),
            ),
        ),
        (
            "grey-gamma22-16.png",
            png16(
                256,
                1,
                &(0..256u32)
                    .map(|i| (i * 257 + 0x40) as u16)
                    .collect::<Vec<_>>(),
                image::ExtendedColorType::L16,
                grey_profile(2.2),
            ),
        ),
    ];
    for (name, bytes) in files {
        std::fs::write(dir.join(name), &bytes).unwrap();
        let img = decode_image_bytes(&bytes).unwrap();
        std::fs::write(dir.join(format!("{name}.rinch.rgba")), &*img.data).unwrap();
    }
}
