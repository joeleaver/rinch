//! #2311: a decoded `<img>` is drawn in its **content box**, as a browser
//! draws a replaced element's content: inside its padding and border,
//! fitted there by `object-fit`, and clipped to the content edge's curve
//! when the box is rounded. Before, the software and GPU painters fitted the
//! picture into the border box, so it covered the padding, and a
//! `border-radius` rounded neither the background, the border nor the
//! picture.
//!
//! The markup is Chrome 156's oracle page (`--force-device-scale-factor=1`,
//! `image-rendering: pixelated` so every texel is a solid run), and every
//! expectation below is a pixel Chrome drew. The paddings are uneven (7, 3, 5,
//! 9) so no side sits on another's value, and the content box is not centred
//! in the border box. `gpu_img_content_box_2311` (in `rinch`) holds the GPU
//! painter to the same pixels.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 400;
const H: u32 = 160;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

const R: [u8; 3] = [255, 0, 0];
const G: [u8; 3] = [0, 255, 0];
const B: [u8; 3] = [0, 0, 255];
const WH: [u8; 3] = [255, 255, 255];
/// The boxes' background, which shows in their padding.
const BG: [u8; 3] = [255, 0, 255];
/// Their border.
const BD: [u8; 3] = [128, 128, 128];

const CSS: &str = "html, body { margin: 0; background: #fff; }
.a { position: absolute; display: block; image-rendering: pixelated; }
.pb { padding: 7px 3px 5px 9px; border: 4px solid rgb(128, 128, 128);
      background: rgb(255, 0, 255); width: 40px; height: 20px; }
.r { padding: 6px; border: 4px solid rgb(128, 128, 128); background: rgb(255, 0, 255);
     width: 60px; height: 60px; border-radius: 20px; }";

/// (class, src, style) per `<img>`.
const BOXES: &[(&str, &str, &str)] = &[
    ("a pb", "q.png", "left: 10px; top: 10px"),
    (
        "a pb",
        "q.png",
        "left: 100px; top: 10px; object-fit: contain",
    ),
    ("a pb", "q.png", "left: 190px; top: 10px; object-fit: cover"),
    ("a pb", "g.png", "left: 280px; top: 10px; object-fit: none"),
    ("a r", "q.png", "left: 10px; top: 70px"),
    ("a r", "g.png", "left: 190px; top: 70px; object-fit: cover"),
];

/// `q.png`: 2x2, red green / blue white. `g.png`: 80x40, its left half red
/// and its right half green.
fn images() -> [(&'static str, DecodedImage); 2] {
    let wide: Vec<[u8; 4]> = (0..80 * 40)
        .map(|i| if i % 80 < 40 { RED } else { GREEN })
        .collect();
    [
        (
            "q.png",
            DecodedImage::new([RED, GREEN, BLUE, WHITE].concat(), 2, 2),
        ),
        ("g.png", DecodedImage::new(wide.concat(), 80, 40)),
    ]
}

fn render() -> Vec<[u8; 4]> {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    for (url, image) in images() {
        doc.tree.image_cache.insert_decoded(url.to_string(), image);
    }
    let body = doc.body();
    for &(class, src, style) in BOXES {
        let n = doc.create_element("img");
        doc.set_attribute(n, "class", class);
        doc.set_attribute(n, "src", src);
        doc.set_attribute(n, "style", style);
        doc.append_child(body, n);
    }
    doc.resolve_layout(W as f32, H as f32);
    let mut painter = TinySkiaPainter::new(W, H);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

/// Chrome 156's pixels for [`BOXES`], `((x, y), rgb)`.
pub const CHROME: &[((u32, u32), [u8; 3])] = &[
    // `fill`: border 10..14, left padding 14..23, content 23..63 x 21..41.
    ((11, 30), BD),
    ((18, 30), BG),
    ((22, 21), BG),
    ((23, 21), R),
    ((61, 22), G),
    ((24, 39), B),
    ((62, 40), WH),
    ((63, 40), BG),
    // `contain`: the 20x20 picture centred in the content box, the
    // letterbox the background.
    ((118, 30), BG),
    ((122, 30), BG),
    ((123, 30), R),
    ((124, 22), R),
    ((142, 22), G),
    ((143, 22), BG),
    // `cover`: 40x40 centred, cut at the content box, not the border box.
    ((205, 20), BG),
    ((205, 21), R),
    ((205, 30), R),
    ((205, 31), B),
    ((205, 40), B),
    ((205, 41), BG),
    ((244, 30), BG),
    // `none`: 80x40 at natural size, centred and cut at the content box.
    ((292, 30), BG),
    ((293, 30), R),
    ((312, 30), R),
    ((313, 30), G),
    ((300, 17), BG),
    // Rounded (radius 20px, border 4px, padding 6px: the content edge's
    // radius is 10px): the corners outside the border's curve are the page,
    // the content box's corner is the background, its inside the picture.
    ((10, 70), WH),
    ((11, 71), WH),
    ((20, 80), BG),
    ((21, 81), BG),
    ((25, 85), R),
    ((75, 135), WH),
    ((79, 139), BG),
    // Rounded and `cover`: 120x60, cut to the content edge's curve.
    ((200, 80), BG),
    ((205, 110), R),
    ((229, 110), R),
    ((230, 110), G),
    ((255, 110), G),
];

#[test]
fn an_img_is_drawn_in_its_content_box_as_chrome_draws_it() {
    let px = render();
    let at = |x: u32, y: u32| {
        let p = px[(y * W + x) as usize];
        [p[0], p[1], p[2]]
    };
    let bad: Vec<_> = CHROME
        .iter()
        .filter(|&&((x, y), want)| at(x, y).iter().zip(want).any(|(a, b)| a.abs_diff(b) > 2))
        .map(|&((x, y), want)| ((x, y), at(x, y), want))
        .collect();
    assert!(bad.is_empty(), "(point, tiny-skia, chrome): {bad:?}");
}
