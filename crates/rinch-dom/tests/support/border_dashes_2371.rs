//! The #2371 / #2372 scene and Chrome's pixels for it, shared by the software
//! painter's `border_dashes_2371_tests` and the GPU painter's
//! `gpu_border_dashes_2371` (`rinch/tests`, through `#[path]`).
//!
//! Dashed and dotted borders 1, 2, 3 and 5px wide, on boxes of even and odd
//! sizes, the issue's 40x20 box, per-side styles, and rounded corners. Each
//! box is compared with Chrome 156 (headless, the same page; `chrome.rs` holds
//! the captured rows) along its top border's middle device row and its left
//! border's middle device column, corners included. Where Chrome is dark
//! (`#`) the painter must be under 128, and where Chrome is light (`.`) it
//! must be over 128; an in-between pixel is not judged.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

#[path = "border_dashes_2371_chrome.rs"]
mod chrome;
pub use chrome::{BOXES, CHROME_1X, CHROME_2X};

pub const W: u32 = 200;
pub const H: u32 = 960;

/// The boxes that are rounded. Painters may differ from Chrome along an arc
/// by antialiasing, so these are allowed a few pixels (see `misses`).
pub fn rounded(i: usize) -> bool {
    BOXES[i].4.contains("radius")
}

/// The boxes whose rows are not black on white (the per-side colours), which
/// `misses` skips; `corner_colours` checks them.
pub fn coloured(i: usize) -> bool {
    BOXES[i].4.contains("#f00")
}

pub fn document() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
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
    }
    doc.resolve_layout(W as f32, H as f32);
    doc
}

/// The first border width a box's style names, in CSS px.
fn border_width(css: &str) -> u32 {
    let at = css.find("px").expect("a width");
    css[..at]
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// Expand `"#3 .2 ~1"` into `"###..~"`.
fn expand(runs: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for run in runs.split(' ') {
        let (c, n) = run.split_at(1);
        out.extend(std::iter::repeat_n(c.as_bytes()[0], n.parse().unwrap()));
    }
    out
}

/// The grey level of a pixel (RGBA, on white, black ink).
fn level(p: [u8; 4]) -> u8 {
    ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8
}

/// One box's sampled row and column, as the painter drew them: `(top, left)`
/// grey levels, the same pixels Chrome's strings describe.
pub fn sample(px: &[[u8; 4]], pw: u32, scale: u32, i: usize) -> (Vec<u8>, Vec<u8>) {
    let (x, y, w, h, css) = BOXES[i];
    let s = scale;
    let b = border_width(css) * s;
    let (x0, y0) = (x * s, y * s);
    let top = (0..w * s)
        .map(|k| level(px[((y0 + b / 2) * pw + x0 + k) as usize]))
        .collect();
    let left = (0..h * s)
        .map(|k| level(px[((y0 + k) * pw + x0 + b / 2) as usize]))
        .collect();
    (top, left)
}

/// Render `levels` in Chrome's run notation, for a failure message.
pub fn runs(levels: &[u8]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut cur = 0u8;
    let mut n = 0;
    for &v in levels {
        let c = if v < 64 {
            b'#'
        } else if v > 192 {
            b'.'
        } else {
            b'~'
        };
        if c == cur {
            n += 1;
        } else {
            if n > 0 {
                out.push(format!("{}{n}", cur as char));
            }
            cur = c;
            n = 1;
        }
    }
    out.push(format!("{}{n}", cur as char));
    out.join(" ")
}

/// The pixels where `got` disagrees with Chrome's `want` string: Chrome dark
/// and the painter 128 or lighter, or Chrome light and the painter darker
/// than 128. Returns their indices.
pub fn disagreements(got: &[u8], want: &str) -> Vec<usize> {
    let want = expand(want);
    assert_eq!(got.len(), want.len());
    (0..got.len())
        .filter(|&k| match want[k] {
            b'#' => got[k] >= 128,
            b'.' => got[k] < 128,
            _ => false,
        })
        .collect()
}

/// Every box's disagreements with Chrome at `scale`, as failure lines.
/// A square box must agree on every judged pixel; a rounded one may miss
/// `ROUNDED_SLACK` per row or column (antialiasing on the arcs).
pub fn misses(px: &[[u8; 4]], pw: u32, scale: u32) -> Vec<String> {
    // Measured: at most 3 on tiny-skia (#2371).
    const ROUNDED_SLACK: usize = 3;
    let chrome = if scale == 1 { CHROME_1X } else { CHROME_2X };
    let mut out = Vec::new();
    for i in 0..BOXES.len() {
        if coloured(i) {
            continue;
        }
        let (top, left) = sample(px, pw, scale, i);
        let slack = if rounded(i) { ROUNDED_SLACK } else { 0 };
        for (name, got, want) in [("top", &top, chrome[i].0), ("left", &left, chrome[i].1)] {
            let bad = disagreements(got, want);
            if bad.len() > slack {
                out.push(format!(
                    "scale {scale} box {i} ({}) {name}: {} pixels at {:?}\n   got  {}\n   want {}",
                    BOXES[i].4,
                    bad.len(),
                    bad,
                    runs(got),
                    want
                ));
            }
        }
    }
    out
}

/// Chrome's colours near the four corners of the per-side box (top red
/// dashed, right blue dotted, bottom black solid, left green dashed), each
/// corner split on its diagonal: `(corner, inward x, inward y, colour)` in
/// device px, corners `0..4` clockwise from the top-left. One pixel each side
/// of the diagonal, where Chrome's colour is pure.
const CORNERS_1X: &[(usize, u32, u32, [u8; 3])] = &[
    (0, 1, 0, [255, 0, 0]),
    (0, 0, 1, [0, 170, 0]),
    (1, 1, 0, [255, 0, 0]),
    (1, 0, 1, [0, 0, 255]),
    (2, 1, 0, [0, 0, 0]),
    (2, 0, 1, [0, 0, 255]),
    (3, 1, 0, [0, 0, 0]),
    (3, 0, 1, [0, 170, 0]),
];
const CORNERS_2X: &[(usize, u32, u32, [u8; 3])] = &[
    (0, 2, 0, [255, 0, 0]),
    (0, 0, 2, [0, 170, 0]),
    (1, 2, 0, [255, 0, 0]),
    (1, 1, 2, [0, 0, 255]),
    (2, 2, 0, [0, 0, 0]),
    (2, 1, 2, [0, 0, 255]),
    (3, 2, 0, [0, 0, 0]),
    (3, 0, 2, [0, 170, 0]),
];

/// The per-side box's corner samples that miss Chrome's colour by more than
/// 40 levels on a channel, as failure lines.
pub fn corner_misses(px: &[[u8; 4]], pw: u32, scale: u32) -> Vec<String> {
    let i = (0..BOXES.len()).find(|&i| coloured(i)).unwrap();
    let (x, y, w, h, _) = BOXES[i];
    let s = scale;
    let (x0, y0, x1, y1) = (x * s, y * s, (x + w) * s - 1, (y + h) * s - 1);
    let samples = if scale == 1 { CORNERS_1X } else { CORNERS_2X };
    samples
        .iter()
        .filter_map(|&(corner, dx, dy, want)| {
            let (px_x, px_y) = match corner {
                0 => (x0 + dx, y0 + dy),
                1 => (x1 - dx, y0 + dy),
                2 => (x1 - dx, y1 - dy),
                _ => (x0 + dx, y1 - dy),
            };
            let p = px[(px_y * pw + px_x) as usize];
            let got = [p[0], p[1], p[2]];
            (0..3).any(|c| got[c].abs_diff(want[c]) > 40).then(|| {
                format!(
                    "scale {scale} corner {corner} at ({px_x}, {px_y}): got {got:?}, want {want:?}"
                )
            })
        })
        .collect()
}
