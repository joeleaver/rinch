//! The #2719 conic-gradient scene and Chrome's pixels for it, shared by the
//! software painter's `conic_gradient_2719_tests` and the GPU painter's
//! `gpu_conic_gradient_2719` (`rinch/tests`, through `#[path]`).
//!
//! Each box is sampled on a 6x6 grid (the centre of each sixth of its border
//! box, floored to a device pixel). `conic_gradient_2719_chrome.rs` holds
//! Chrome 157's pixel at every sample whose 3x3 neighbourhood in Chrome's
//! screenshot is smooth (no channel spreads more than 48 levels): a sample
//! on a hard stop, a box edge or a gradient's centre, where a pixel is
//! decided by antialiasing or by which side of an edge it falls, is left out.
//! Captured headless, `--force-device-scale-factor=1`, from the page
//! `conic_gradient_2719_tests::page_html` writes (the same boxes); regenerate
//! with the ignored test
//! `conic_gradient_2719_tests::write_chrome_page`, then sample as above.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

#[path = "conic_gradient_2719_chrome.rs"]
mod chrome;
pub use chrome::CHROME;

pub const W: u32 = 520;
pub const H: u32 = 430;

/// Each box: left, top, width, height (the border box, CSS px) and its CSS.
/// Sizes and positions are off round numbers.
pub const BOXES: &[(u32, u32, u32, u32, &str)] = &[
    // 0: two colours, all defaults.
    (7, 9, 97, 83, "background: conic-gradient(red, blue)"),
    // 1: `from`.
    (
        113,
        9,
        101,
        79,
        "background: conic-gradient(from 37deg, #f80, #08f, #f80)",
    ),
    // 2: `at <percentage>`, many stops.
    (
        223,
        9,
        93,
        87,
        "background: conic-gradient(at 30% 65%, red, yellow, lime, aqua, blue, magenta, red)",
    ),
    // 3: `from <turn> at <length>`, hard stops at angles.
    (
        327,
        9,
        103,
        77,
        "background: conic-gradient(from 0.15turn at 23px 41px, #e33 0deg 47deg, \
         #3e3 47deg 133deg, #33e 133deg)",
    ),
    // 4: hard stops at percentages.
    (
        7,
        107,
        89,
        91,
        "background: conic-gradient(#000 0% 12.5%, #fff 12.5% 37%, #c60 37% 81%, #06c 81%)",
    ),
    // 5: repeating, first stop at 0deg.
    (
        113,
        107,
        99,
        85,
        "background: repeating-conic-gradient(#222 0deg 17deg, #eee 17deg 34deg)",
    ),
    // 6: repeating, smooth, `from` and `at`, first stop past 0deg.
    (
        223,
        107,
        97,
        93,
        "background: repeating-conic-gradient(from 11deg at 60% 40%, red 10deg, \
         blue 35deg, red 60deg)",
    ),
    // 7: through transparent, over a background colour (premultiplied).
    (
        327,
        107,
        91,
        83,
        "background: conic-gradient(transparent, rgba(255, 128, 0, 0.8)) rgb(40, 60, 200)",
    ),
    // 8: two conic layers.
    (
        7,
        213,
        95,
        89,
        "background: conic-gradient(from 90deg, rgba(255, 0, 0, 0.6), rgba(0, 0, 255, 0.6)), \
         conic-gradient(at 20% 80%, #ff0, #0ff)",
    ),
    // 9: a repeating tile: a checkerboard, sized and positioned.
    (
        113,
        213,
        97,
        87,
        "background: conic-gradient(#fd0 25%, #222 0 50%, #fd0 0 75%, #222 0); \
         background-size: 23px 19px; background-position: 7px 5px",
    ),
    // 10: one tile, sized and positioned, over a colour.
    (
        223,
        213,
        101,
        91,
        "background: conic-gradient(red, blue) no-repeat rgb(230, 230, 150); \
         background-size: 41px 37px; background-position: 70% 30%",
    ),
    // 11: a pie chart, clipped round.
    (
        327,
        213,
        87,
        87,
        "border-radius: 50%; background: conic-gradient(#e44 0 62%, #333 62%)",
    ),
    // 12: a gauge: a dial under a dark hub.
    (
        7,
        317,
        107,
        107,
        "border-radius: 50%; background: radial-gradient(circle, #111 0 52%, transparent 53%), \
         conic-gradient(from 240deg, #0c6, #fc0 120deg, #f33 200deg, #444 200deg 240deg, \
         transparent 240deg)",
    ),
    // 13: stops outside the turn, cut at its edges.
    (
        123,
        317,
        93,
        81,
        "background: conic-gradient(from 200deg, red -30deg, lime 170deg, blue 400deg)",
    ),
    // 14: a border: positioned in the padding box, tiled under the border.
    (
        227,
        317,
        99,
        85,
        "border: 7px solid rgba(0, 0, 0, 0.3); \
         background: conic-gradient(from 45deg, #f0f, #0ff, #f0f); background-size: 50% 50%",
    ),
    // 15: three stops on one angle.
    (
        337,
        317,
        87,
        79,
        "background: conic-gradient(red 40%, blue 40%, lime 40%)",
    ),
    // 16: repeating, a full turn long, starting past 0deg.
    (
        437,
        9,
        79,
        77,
        "background: repeating-conic-gradient(red 10deg, blue 190deg, red 370deg)",
    ),
];

/// The scene, laid out at `W` x `H`, and each box's node.
pub fn document() -> (RinchDocument, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    let mut ids = Vec::new();
    for &(l, t, w, h, css) in BOXES {
        let n = doc.create_element("div");
        doc.set_attribute(
            n,
            "style",
            &format!(
                "position: absolute; box-sizing: border-box; left: {l}px; top: {t}px; \
                 width: {w}px; height: {h}px; {css}"
            ),
        );
        doc.append_child(body, n);
        ids.push(n);
    }
    doc.resolve_layout(W as f32, H as f32);
    (doc, ids)
}

/// Every Chrome sample the painter's pixels (`px`, RGBA rows `W` wide)
/// miss by more than `tol` in a channel, as `box (x, y): painter vs Chrome`.
pub fn misses(px: &[[u8; 4]], tol: u8) -> Vec<String> {
    let mut bad = Vec::new();
    for &(b, x, y, want) in CHROME {
        let got = px[(y * W + x) as usize];
        if got[..3].iter().zip(want).any(|(g, w)| g.abs_diff(w) > tol) {
            bad.push(format!(
                "box {b} ({x}, {y}): {:?} vs Chrome {want:?}  [{}]",
                &got[..3],
                BOXES[b].4
            ));
        }
    }
    bad
}

/// How many of Chrome's samples each box has (a box with none tests
/// nothing).
pub fn samples_per_box() -> Vec<usize> {
    let mut n = vec![0; BOXES.len()];
    for &(b, ..) in CHROME {
        n[b] += 1;
    }
    n
}
