//! #2779: a picture tagged with an ICC profile is converted to sRGB.
//!
//! Chrome colour-manages images: a file tagged Display P3 or Adobe RGB is
//! converted to the output colour space (sRGB, with no display profile), and
//! an untagged one is taken as sRGB. rinch drew every file's values as sRGB,
//! so a wide-gamut photograph looked duller and off in hue.
//!
//! The fixtures are made here by the `image` crate's encoders, which embed a
//! profile (`ImageEncoder::set_icc_profile`: JPEG `APP2`, PNG `iCCP`, WebP
//! `ICCP`); the profiles are `moxcms`'s own Display P3 and sRGB, encoded. The
//! expected sRGB values are worked out here from the published Display P3
//! and sRGB primaries (D65) and the sRGB transfer curve both spaces use, not
//! from the CMS under test.
#![cfg(feature = "color-management")]

use image::ImageEncoder;
use rinch_dom::image_cache::decode_image_bytes;
use rinch_dom::image_color::set_color_management;
use std::sync::Mutex;

/// The opt-out test flips a process-wide switch; every test holds this.
static SWITCH: Mutex<()> = Mutex::new(());

fn p3_profile() -> Vec<u8> {
    moxcms::ColorProfile::new_display_p3().encode().unwrap()
}

fn srgb_profile() -> Vec<u8> {
    moxcms::ColorProfile::new_srgb().encode().unwrap()
}

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

/// A Display P3 colour as sRGB, clipped to the sRGB gamut.
fn p3_to_srgb(c: [u8; 3]) -> [u8; 3] {
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
    let lin = c.map(|v| to_linear(f64::from(v) / 255.0));
    let mul = |m: [[f64; 3]; 3], v: [f64; 3]| {
        [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
    };
    let srgb = mul(XYZ_TO_SRGB, mul(P3_TO_XYZ, lin));
    srgb.map(|v| (from_linear(v) * 255.0).round() as u8)
}

/// The colours: pure P3 red (outside sRGB: it clips to sRGB red), half red,
/// half green, and a warm mid colour, the last at half alpha. Each a 2x2
/// block of a 8x2 picture.
const COLOURS: [[u8; 4]; 4] = [
    [255, 0, 0, 255],
    [128, 0, 0, 255],
    [0, 128, 0, 255],
    [200, 150, 100, 128],
];

fn picture() -> image::RgbaImage {
    image::RgbaImage::from_fn(8, 2, |x, _| image::Rgba(COLOURS[x as usize / 2]))
}

fn png(icc: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = image::codecs::png::PngEncoder::new(&mut out);
    if let Some(icc) = icc {
        enc.set_icc_profile(icc).unwrap();
    }
    let p = picture();
    enc.write_image(&p, 8, 2, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

fn webp(icc: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
    if let Some(icc) = icc {
        enc.set_icc_profile(icc).unwrap();
    }
    let p = picture();
    enc.write_image(&p, 8, 2, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

/// A 16x16 JPEG of one colour, quality 100.
fn jpeg(icc: Option<Vec<u8>>, c: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 100);
    if let Some(icc) = icc {
        enc.set_icc_profile(icc).unwrap();
    }
    let p = image::RgbImage::from_pixel(16, 16, image::Rgb(c));
    enc.write_image(&p, 16, 16, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

/// What the `image` crate decodes `bytes` to with no colour management: the
/// pixels as rinch drew them before #2779.
fn unmanaged(bytes: &[u8]) -> Vec<u8> {
    image::load_from_memory(bytes)
        .unwrap()
        .to_rgba8()
        .into_raw()
}

fn assert_near(got: [u8; 3], want: [u8; 3], tol: u8, case: &str) {
    assert!(
        (0..3).all(|c| got[c].abs_diff(want[c]) <= tol),
        "{case}: got {got:?}, want {want:?}"
    );
}

/// The profile is in each file and the `image` crate reads it back, so a
/// test that finds pixels unchanged is not passing for want of a profile.
#[test]
fn the_fixtures_carry_their_profile() {
    use image::ImageDecoder;
    let p3 = p3_profile();
    let bytes = png(Some(p3.clone()));
    let mut d = image::codecs::png::PngDecoder::new(std::io::Cursor::new(&bytes)).unwrap();
    assert_eq!(d.icc_profile().unwrap().as_ref(), Some(&p3));
    let bytes = webp(Some(p3.clone()));
    let mut d = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&bytes)).unwrap();
    assert_eq!(d.icc_profile().unwrap().as_ref(), Some(&p3));
    let bytes = jpeg(Some(p3.clone()), [1, 2, 3]);
    let mut d = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&bytes)).unwrap();
    assert_eq!(d.icc_profile().unwrap().as_ref(), Some(&p3));
}

/// PNG (`iCCP`) and WebP (`ICCP`), lossless: each P3 colour comes out as its
/// sRGB value within 2 a channel, and alpha is carried through.
#[test]
fn a_display_p3_png_or_webp_is_converted_to_srgb() {
    let _g = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
    for (name, bytes) in [
        ("PNG", png(Some(p3_profile()))),
        ("WebP", webp(Some(p3_profile()))),
    ] {
        let img = decode_image_bytes(&bytes).unwrap();
        assert_eq!((img.width, img.height), (8, 2));
        for (i, c) in COLOURS.iter().enumerate() {
            let at = i * 2 * 4;
            let got = [img.data[at], img.data[at + 1], img.data[at + 2]];
            let want = p3_to_srgb([c[0], c[1], c[2]]);
            assert_near(got, want, 2, &format!("{name} {c:?}"));
            assert_eq!(img.data[at + 3], c[3], "{name} {c:?}: alpha");
        }
    }
    // Off the fixed point: half P3 red is a different sRGB red.
    assert_eq!(p3_to_srgb([255, 0, 0]), [255, 0, 0]);
    assert!(p3_to_srgb([128, 0, 0])[0] > 135);
}

/// JPEG (`APP2`): the colour as the JPEG decodes it untagged, converted.
#[test]
fn a_display_p3_jpeg_is_converted_to_srgb() {
    let _g = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
    for c in [[200, 150, 100], [128, 0, 0], [40, 160, 220]] {
        let stored = unmanaged(&jpeg(None, c));
        let stored = [stored[0], stored[1], stored[2]];
        let img = decode_image_bytes(&jpeg(Some(p3_profile()), c)).unwrap();
        let got = [img.data[0], img.data[1], img.data[2]];
        assert_near(got, p3_to_srgb(stored), 2, &format!("JPEG {c:?}"));
    }
}

/// An sRGB-tagged or untagged file decodes to exactly the pixels it did
/// before #2779, in every format.
#[test]
fn srgb_tagged_and_untagged_files_are_unchanged() {
    let _g = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
    let c = [200, 150, 100];
    for (name, bytes) in [
        ("PNG sRGB", png(Some(srgb_profile()))),
        ("PNG untagged", png(None)),
        ("WebP sRGB", webp(Some(srgb_profile()))),
        ("WebP untagged", webp(None)),
        ("JPEG sRGB", jpeg(Some(srgb_profile()), c)),
        ("JPEG untagged", jpeg(None, c)),
    ] {
        let img = decode_image_bytes(&bytes).unwrap();
        assert!(*img.data == *unmanaged(&bytes), "{name}: pixels changed");
    }
}

/// `set_color_management(false)` decodes a tagged file as untagged.
#[test]
fn colour_management_can_be_turned_off() {
    let _g = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
    /// Turns colour management back on when dropped, so a panic while it is
    /// off does not leave it off for the tests that run after this one.
    struct On;
    impl Drop for On {
        fn drop(&mut self) {
            set_color_management(true);
        }
    }
    let bytes = png(Some(p3_profile()));
    let off = {
        let _on = On;
        set_color_management(false);
        decode_image_bytes(&bytes).unwrap()
    };
    assert!(rinch_dom::image_color::color_management());
    assert!(*off.data == *unmanaged(&bytes));
    let on = decode_image_bytes(&bytes).unwrap();
    assert!(*on.data != *unmanaged(&bytes));
}

/// The cost at decode of a 3840x2160 Display P3 PNG, against the same file
/// untagged. Run with `--release --ignored --nocapture`.
#[test]
#[ignore]
fn measure_the_cost_of_converting_a_large_picture() {
    let _g = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
    let (w, h) = (3840u32, 2160u32);
    let p = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
    });
    let encode = |icc: Option<Vec<u8>>| {
        let mut out = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new_with_quality(
            &mut out,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        );
        if let Some(icc) = icc {
            enc.set_icc_profile(icc).unwrap();
        }
        enc.write_image(&p, w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    };
    let tagged = encode(Some(p3_profile()));
    let plain = encode(None);
    let time = |bytes: &[u8]| {
        let mut best = f64::MAX;
        for _ in 0..7 {
            let t = std::time::Instant::now();
            std::hint::black_box(decode_image_bytes(bytes).unwrap());
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        best
    };
    let (a, b) = (time(&plain), time(&tagged));
    println!(
        "3840x2160 PNG decode: untagged {a:.1} ms, Display P3 {b:.1} ms (+{:.1} ms)",
        b - a
    );
}
