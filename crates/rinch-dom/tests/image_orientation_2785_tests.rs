//! #2785: a picture is drawn as its EXIF orientation says.
//!
//! Browsers default to `image-orientation: from-image`: a photograph stored
//! sideways with orientation 6 (a phone held upright) is drawn upright, and
//! its natural size is the turned size. Chrome reads the orientation only
//! where the EXIF comes before the image data: always in a JPEG (`APP1`), in
//! a PNG only from an `eXIf` chunk before `IDAT`, and never in a WebP (whose
//! container puts it after the image data). rinch decoded the stored pixels
//! as they were.
//!
//! Each fixture is made here: a 32x16 picture, white, with an 8x8 red block
//! at its stored top-left, green at its top-right and blue at its
//! bottom-left, encoded by the `image` crate and given an EXIF block by hand.
//! The expected place of each block is worked out from EXIF's own definition
//! of the eight values (where the stored row 0 and column 0 are to be drawn),
//! not from the code under test.

use rinch_core::dom::DomDocument;
use rinch_core::image::{ImageLoadResult, ImageLoader};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::{DecodedImage, decode_image_bytes};
use std::sync::Arc;

const SW: u32 = 32;
const SH: u32 = 16;
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// The stored picture, before any orientation.
fn stored() -> image::RgbImage {
    image::RgbImage::from_fn(SW, SH, |x, y| {
        let (l, t, r, b) = (x < 8, y < 8, x >= SW - 8, y >= SH - 8);
        image::Rgb(if l && t {
            RED
        } else if r && t {
            GREEN
        } else if l && b {
            BLUE
        } else {
            [255, 255, 255]
        })
    })
}

/// Where the stored pixel `(x, y)` is drawn under EXIF orientation `o`, and
/// the drawn size. From the EXIF definition: 2 mirrors left-right, 3 turns
/// half round, 4 mirrors top-bottom, 5 transposes, 6 turns a quarter
/// clockwise, 7 transverses, 8 turns a quarter anticlockwise.
fn drawn_at(o: u8, x: u32, y: u32) -> ((u32, u32), (u32, u32)) {
    let (w, h) = (SW, SH);
    match o {
        1 => ((x, y), (w, h)),
        2 => ((w - 1 - x, y), (w, h)),
        3 => ((w - 1 - x, h - 1 - y), (w, h)),
        4 => ((x, h - 1 - y), (w, h)),
        5 => ((y, x), (h, w)),
        6 => ((h - 1 - y, x), (h, w)),
        7 => ((h - 1 - y, w - 1 - x), (h, w)),
        8 => ((y, w - 1 - x), (h, w)),
        _ => unreachable!(),
    }
}

/// A TIFF structure (little-endian, or big-endian with `big`) whose first IFD
/// holds an orientation of `o`, after another tag so the scan has to walk.
fn tiff(o: u16, big: bool) -> Vec<u8> {
    let u16b = |v: u16| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let u32b = |v: u32| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let mut t = Vec::new();
    t.extend_from_slice(if big { b"MM\0*" } else { b"II*\0" });
    t.extend_from_slice(&u32b(8));
    t.extend_from_slice(&u16b(2));
    // ImageDescription is not read; a SHORT here would do as well.
    for (tag, kind, value) in [(0x0131u16, 3u16, 7u16), (0x0112, 3, o)] {
        t.extend_from_slice(&u16b(tag));
        t.extend_from_slice(&u16b(kind));
        t.extend_from_slice(&u32b(1));
        t.extend_from_slice(&u16b(value));
        t.extend_from_slice(&[0, 0]);
    }
    t.extend_from_slice(&u32b(0));
    t
}

/// A JPEG of the stored picture with an `APP1` EXIF segment of orientation
/// `o` right after `SOI` (before the encoder's own `APP0`).
fn jpeg(o: u16) -> Vec<u8> {
    let mut plain = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 100)
        .encode_image(&stored())
        .unwrap();
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend(tiff(o, o.is_multiple_of(2)));
    let mut out = plain[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend(payload);
    out.extend_from_slice(&plain[2..]);
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = (data.len() as u32).to_be_bytes().to_vec();
    c.extend_from_slice(kind);
    c.extend_from_slice(data);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(data);
    c.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    c
}

/// The stored picture as a PNG.
fn plain_png() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    stored()
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

/// A PNG of the stored picture with an `eXIf` chunk of orientation `o`,
/// before `IDAT` (straight after `IHDR`) or after it (just before `IEND`).
fn png(o: u16, before_idat: bool) -> Vec<u8> {
    let plain = plain_png();
    // Signature (8) + IHDR (4 + 4 + 13 + 4).
    let after_ihdr = 8 + 25;
    assert_eq!(&plain[after_ihdr - 21..after_ihdr - 17], b"IHDR");
    let iend = plain.len() - 12;
    assert_eq!(&plain[iend + 4..iend + 8], b"IEND");
    let at = if before_idat { after_ihdr } else { iend };
    let mut out = plain[..at].to_vec();
    out.extend(png_chunk(b"eXIf", &tiff(o, !o.is_multiple_of(2))));
    out.extend_from_slice(&plain[at..]);
    out
}

/// A WebP (lossless, extended format) of the stored picture with an `EXIF`
/// chunk of orientation `o` after the image data, as the container places it.
fn webp(o: u16) -> Vec<u8> {
    let mut simple = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut simple)
        .encode(stored().as_raw(), SW, SH, image::ExtendedColorType::Rgb8)
        .unwrap();
    assert_eq!(&simple[8..12], b"WEBP");
    let vp8l = &simple[12..];
    assert_eq!(&vp8l[..4], b"VP8L");
    let riff_chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut c = kind.to_vec();
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(data);
        if !data.len().is_multiple_of(2) {
            c.push(0);
        }
        c
    };
    // VP8X: flags (EXIF = 0x08), 3 reserved bytes, canvas width - 1 and
    // height - 1 as 24-bit little-endian numbers.
    let mut vp8x = vec![0x08, 0, 0, 0];
    vp8x.extend_from_slice(&(SW - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(SH - 1).to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    body.extend(riff_chunk(b"VP8X", &vp8x));
    body.extend_from_slice(vp8l);
    body.extend(riff_chunk(b"EXIF", &tiff(o, false)));
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

fn px(img: &DecodedImage, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * img.width + x) * 4) as usize;
    [img.data[i], img.data[i + 1], img.data[i + 2]]
}

/// The decoded picture is `o`'s drawn size and has each coloured block where
/// `o` draws it (sampled at the centre of each block, within `tol` a channel:
/// JPEG is lossy).
fn assert_oriented(img: &DecodedImage, o: u8, tol: i32, case: &str) {
    let (_, (w, h)) = drawn_at(o, 0, 0);
    assert_eq!((img.width, img.height), (w, h), "{case}: size");
    for (sx, sy, colour, name) in [
        (4, 4, RED, "red"),
        (SW - 5, 4, GREEN, "green"),
        (4, SH - 5, BLUE, "blue"),
    ] {
        let ((x, y), _) = drawn_at(o, sx, sy);
        let got = px(img, x, y);
        let off = (0..3)
            .map(|c| (i32::from(got[c]) - i32::from(colour[c])).abs())
            .max()
            .unwrap();
        assert!(
            off <= tol,
            "{case}: {name} block expected at ({x}, {y}), found {got:?}"
        );
    }
}

#[test]
fn a_jpeg_is_drawn_in_each_of_the_eight_orientations() {
    for o in 1..=8u8 {
        let img = decode_image_bytes(&jpeg(u16::from(o))).unwrap();
        assert_oriented(&img, o, 40, &format!("JPEG orientation {o}"));
    }
}

#[test]
fn a_png_with_exif_before_its_image_data_is_drawn_in_each_orientation() {
    for o in 1..=8u8 {
        let img = decode_image_bytes(&png(u16::from(o), true)).unwrap();
        assert_oriented(&img, o, 0, &format!("PNG orientation {o}"));
    }
}

#[test]
fn a_png_with_exif_after_its_image_data_is_drawn_as_stored() {
    let bytes = png(6, false);
    let img = decode_image_bytes(&bytes).unwrap();
    assert_oriented(&img, 1, 0, "PNG eXIf after IDAT");
}

#[test]
fn a_webp_with_exif_is_drawn_as_stored() {
    let bytes = webp(6);
    // Positive control: the EXIF is in the file and readable, so the WebP
    // decoder itself reports orientation 6. rinch ignores it, as Chrome and
    // Firefox do.
    let mut decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&bytes)).unwrap();
    assert_eq!(
        image::ImageDecoder::orientation(&mut decoder).unwrap(),
        image::metadata::Orientation::Rotate90
    );
    let img = decode_image_bytes(&bytes).unwrap();
    assert_oriented(&img, 1, 0, "WebP with EXIF orientation 6");
}

#[test]
fn a_file_with_no_exif_is_drawn_as_stored() {
    let img = decode_image_bytes(&plain_png()).unwrap();
    assert_oriented(&img, 1, 0, "PNG, no EXIF");
}

/// Answers every source with the same bytes.
struct Bytes(Vec<u8>);

impl ImageLoader for Bytes {
    fn load(&self, _src: &str) -> ImageLoadResult {
        ImageLoadResult::Loaded(self.0.clone())
    }
}

/// An `<img>` with no size of its own takes the oriented size as its natural
/// size: a 32x16 stored picture with orientation 6 lays out 16x32.
#[test]
fn an_img_lays_out_at_the_oriented_size() {
    for (o, want) in [(1u16, (32.0, 16.0)), (6, (16.0, 32.0)), (7, (16.0, 32.0))] {
        let mut doc = RinchDocument::new();
        doc.load_css("html, body { margin: 0 }");
        doc.tree.image_loader = Some(Arc::new(Bytes(jpeg(o))));
        let body = doc.body();
        let img = doc.create_element("img");
        doc.set_attribute(img, "src", &format!("photo-{o}.jpg"));
        doc.set_attribute(img, "style", "display: block");
        doc.append_child(body, img);
        let start = std::time::Instant::now();
        let mut wide = false;
        loop {
            wide = !wide;
            doc.resolve_layout(if wide { 201.0 } else { 200.0 }, 100.0);
            if doc
                .tree
                .image_cache
                .get(&format!("photo-{o}.jpg"))
                .is_some()
            {
                doc.resolve_layout(200.0, 100.0);
                break;
            }
            assert!(start.elapsed().as_secs() < 10, "the picture never landed");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let l = doc.tree.get(img.0).unwrap().layout;
        assert_eq!((l.width, l.height), want, "orientation {o}");
    }
}
