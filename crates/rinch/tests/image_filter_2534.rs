//! #2534: a picture drawn below its natural size matches Chrome's pixels, on
//! both painters.
//!
//! Chrome 157 draws a minified raster `<img>` from a mipmap level (Skia's
//! `SkMipmap`: each level half the one above, each pixel the 2x2 average of
//! the four above it, truncated) and bilinearly from there. The level is
//! the deepest one no smaller than the drawn size on *either* axis, so a
//! picture drawn at 1/3 is the half-size level drawn at 0.67x, and one drawn
//! at 0.6x by 0.3x is the picture itself drawn at that scale. Fitted against
//! Chrome's screenshots (mean / max absolute error per channel) that model is
//! 0.00/0 at 1/2 and 1/8, 0.17/1 at 1/3, 1.11/23 at 0.37 and 1.19/18 at
//! 0.6x0.3; a box filter to the drawn size is 6.5/65 at 1/3, Mitchell or
//! Lanczos-3 7/60-75. Before #2534 rinch box-filtered a copy to the drawn
//! scale rounded up to a quarter octave (#2456) and drew it the rest of the
//! way bilinearly: blurred twice, softer than Chrome.
//!
//! The picture is [`picture`], procedural so that nothing in it depends on a
//! font. Chrome's screenshots of it are
//! `tests/fixtures/image_filter_2534_chrome_<case>.png` (RGB, cropped to the
//! drawn image), taken with
//!
//! ```text
//! google-chrome --headless=new --force-device-scale-factor=1 --hide-scrollbars \
//!     --window-size=400,400 --user-data-dir=<a fresh dir> --screenshot=out.png \
//!     'file://<dir>/page.html'
//! ```
//!
//! where `page.html` is `<body style="margin:0;background:#fff"><img
//! src="pic.png" style="display:block;width:Wpx;height:Hpx">` for each case
//! of [`CASES`], and `pic.png` is written by `write_the_picture` (run it with
//! `RINCH_2534_PICTURE=<dir>/pic.png ... -- --ignored write_the_picture`).
//! `--disable-gpu` gives the same bytes.
//!
//! The GPU half needs an adapter, so it is `#[ignore]`d: run it with
//! `cargo test -p rinch --features gpu --test image_filter_2534 -- --ignored`.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;

/// The picture's side.
const N: u32 = 600;

/// (name, drawn width, drawn height, mean, max): each case's bound on the
/// error against Chrome, the mean and maximum absolute difference per
/// channel over the drawn image, on both painters. Measured (software and
/// Vello alike, to 0.01): 0.00/0, 0.14/1, 1.09/23, 0.00/0 and 1.16/18; the
/// last two cases' residue is bilinear sampling at a slightly different
/// phase from Skia's. Before #2534: 0.18/1, 9.92/99, 8.43/134, 0.31/2 and
/// 16.98/196, the same on both.
const CASES: &[(&str, u32, u32, f64, u8)] = &[
    ("half", 300, 300, 0.05, 1),
    ("third", 200, 200, 0.5, 4),
    ("0.37", 222, 222, 1.5, 30),
    ("eighth", 75, 75, 0.05, 1),
    ("0.6x0.3", 360, 180, 1.5, 25),
];

/// A cheap deterministic hash of a pixel position.
fn hash(x: u32, y: u32, salt: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ salt;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

/// 600x600 opaque RGBA: rows of text-like 5x7 glyphs of 1px strokes, 1px
/// vertical and horizontal lines every 7px, a fan of 1px slanted lines,
/// colour noise, 1px rings, a gradient and 1px and 2px checkerboards.
fn picture() -> Vec<u8> {
    let n = N as usize;
    let mut px = vec![255u8; n * n * 4];
    let mut set = |x: usize, y: usize, c: [u8; 3]| {
        if x < n && y < n {
            px[(y * n + x) * 4..][..3].copy_from_slice(&c);
        }
    };
    // "Text": 12 lines of 70 glyphs, each a 5x7 bitmap in an 8x12 cell.
    for line in 0..12u32 {
        let colour = if line.is_multiple_of(3) {
            [200, 20, 20]
        } else {
            [0, 0, 0]
        };
        for glyph in 0..70u32 {
            if hash(glyph, line, 7).is_multiple_of(6) {
                continue; // a space
            }
            for gy in 0..7u32 {
                for gx in 0..5u32 {
                    if hash(glyph * 8 + gx, line * 12 + gy, 1).is_multiple_of(3) {
                        set(
                            (10 + glyph * 8 + gx) as usize,
                            (10 + line * 18 + gy) as usize,
                            colour,
                        );
                    }
                }
            }
        }
    }
    for k in 0..40 {
        for y in 240..330 {
            set(10 + 7 * k, y, [0, 0, 0]);
        }
    }
    for k in 0..12 {
        for x in 300..590 {
            set(x, 240 + 7 * k, [0, 90, 0]);
        }
    }
    for k in 0..30 {
        let (y0, y1) = (350.0 + 3.0 * k as f64, 350.0 + 9.0 * k as f64);
        for x in 10..290 {
            let y = y0 + (y1 - y0) * (x - 10) as f64 / 280.0;
            set(x, y.round() as usize, [30, 30, 30]);
        }
    }
    for y in 350..450 {
        for x in 300..380 {
            let h = hash(x as u32, y as u32, 3).to_le_bytes();
            set(x, y, [h[0], h[1], h[2]]);
        }
    }
    for y in 340..600 {
        for x in 390..600 {
            let d = ((x as f64 - 490.0).hypot(y as f64 - 470.0) - 4.0) / 6.0;
            if d >= -0.08 && (d - d.round()).abs() < 0.5 / 6.0 && d < 16.0 {
                set(x, y, [120, 0, 120]);
            }
        }
    }
    for y in 480..590 {
        for x in 10..210 {
            let g = ((x - 10) * 255 / 199) as u8;
            set(x, y, [g, 255 - g, 128]);
        }
    }
    for y in 480..600 {
        for x in 220..280 {
            let on = if y < 540 {
                (x + y) % 2
            } else {
                (x / 2 + y / 2) % 2
            };
            let v = if on == 1 { 255 } else { 0 };
            set(x, y, [v, v, v]);
        }
    }
    px
}

/// A page drawing [`picture`] at `w` x `h` at its top left.
fn document(w: u32, h: u32) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "html, body {{ margin: 0; background: #fff; }}
         img {{ display: block; width: {w}px; height: {h}px; }}"
    ));
    doc.tree
        .image_cache
        .insert_decoded("pic.png".to_string(), DecodedImage::new(picture(), N, N));
    let body = doc.body();
    let img = doc.create_element("img");
    doc.set_attribute(img, "src", "pic.png");
    doc.append_child(body, img);
    doc.resolve_layout(400.0, 400.0);
    doc
}

/// Chrome's RGB pixels for case `name`.
fn chrome(name: &str) -> (u32, u32, Vec<u8>) {
    let bytes: &[u8] = match name {
        "half" => include_bytes!("fixtures/image_filter_2534_chrome_half.png"),
        "third" => include_bytes!("fixtures/image_filter_2534_chrome_third.png"),
        "0.37" => include_bytes!("fixtures/image_filter_2534_chrome_0.37.png"),
        "eighth" => include_bytes!("fixtures/image_filter_2534_chrome_eighth.png"),
        "0.6x0.3" => include_bytes!("fixtures/image_filter_2534_chrome_0.6x0.3.png"),
        _ => unreachable!(),
    };
    let mut reader = png::Decoder::new(bytes).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    buf.truncate((info.width * info.height * 3) as usize);
    (info.width, info.height, buf)
}

/// Mean and maximum absolute difference per channel between `pixels` (rows
/// `stride` wide) and Chrome's, over case `name`'s drawn image.
fn error(name: &str, pixels: &[[u8; 4]], stride: u32) -> (f64, u8) {
    let (w, h, want) = chrome(name);
    let (mut sum, mut max) = (0u64, 0u8);
    for y in 0..h {
        for x in 0..w {
            let got = pixels[(y * stride + x) as usize];
            let want = &want[((y * w + x) * 3) as usize..][..3];
            for c in 0..3 {
                let d = got[c].abs_diff(want[c]);
                sum += u64::from(d);
                max = max.max(d);
            }
        }
    }
    (sum as f64 / f64::from(w * h * 3), max)
}

/// Check every case against its bounds.
fn check(paint: impl Fn(&mut RinchDocument) -> Vec<[u8; 4]>) {
    let mut report = Vec::new();
    let mut failed = false;
    for &(name, w, h, bound_mean, bound_max) in CASES {
        let mut doc = document(w, h);
        let (mean, max) = error(name, &paint(&mut doc), 400);
        report.push(format!("{name}: {mean:.2}/{max}"));
        failed |= mean > bound_mean || max > bound_max;
    }
    eprintln!("error against Chrome (mean/max): {}", report.join(", "));
    assert!(!failed, "{report:?} against {CASES:?}");
}

#[cfg(feature = "desktop")]
fn paint_software(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    let mut painter = TinySkiaPainter::new(400, 400);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (400.0, 400.0),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

#[cfg(feature = "desktop")]
#[test]
fn software_minified_pictures_match_chrome() {
    check(paint_software);
}

#[cfg(feature = "gpu")]
#[path = "support/vello_pixels.rs"]
mod vello_pixels;

#[cfg(feature = "gpu")]
fn paint_gpu(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    use rinch_dom::paint::vello_painter::VelloPainter;
    let mut painter = VelloPainter::new();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (400.0, 400.0),
        &mut doc.font_cx,
        &mut cx,
    );
    vello_pixels::render(painter.scene(), 400, 400)
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_minified_pictures_match_chrome() {
    check(paint_gpu);
}

/// Writes [`picture`] as a PNG to `$RINCH_2534_PICTURE`, for Chrome.
#[test]
#[ignore = "writes the picture Chrome is given"]
fn write_the_picture() {
    let path = std::env::var("RINCH_2534_PICTURE").expect("RINCH_2534_PICTURE");
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, N, N);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&picture())
        .unwrap();
}
