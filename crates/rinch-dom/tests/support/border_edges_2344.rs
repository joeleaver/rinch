//! The #2344 scene and its expected pixels, shared by the software painter's
//! `border_edges_2344_tests` and the GPU painter's `gpu_border_edges_2344`
//! (`rinch/tests`, through `#[path]`).
//!
//! Chrome 156 draws a 1px solid border or outline on whole pixels as solid
//! one-pixel lines on every side (the issue's table), and at scale 2 as
//! solid two-pixel lines. The boxes sit at odd positions with odd sizes, so
//! no edge is on the origin, on an even pixel or on another box's column.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

pub const W: u32 = 200;
pub const H: u32 = 100;

/// The border colour.
pub const BD: [u8; 3] = [128, 128, 128];
/// The boxes' background.
pub const BG: [u8; 3] = [255, 0, 255];
pub const WH: [u8; 3] = [255, 255, 255];

/// (left, top, width, height, border) per box, in CSS px. Widths and heights
/// are the content box (`box-sizing: content-box`).
const BOXES: &[(u32, u32, u32, u32, &str)] = &[
    // The issue's box.
    (161, 61, 16, 16, "1px solid rgb(128, 128, 128)"),
    (13, 7, 21, 11, "1px solid rgb(128, 128, 128)"),
    (47, 29, 9, 23, "1px dashed rgb(128, 128, 128)"),
    (91, 15, 31, 5, "1px solid rgb(128, 128, 128)"),
];

/// (left, top, width, height) of a borderless box with a square
/// `outline: 1px solid` in the border colour, which takes the same stroke.
const OUTLINED: (u32, u32, u32, u32) = (141, 23, 13, 17);

/// The scene, laid out at `W` x `H` CSS px.
pub fn document() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    let mut add = |style: String| {
        let n = doc.create_element("div");
        doc.set_attribute(n, "style", &style);
        doc.append_child(body, n);
    };
    for &(l, t, w, h, border) in BOXES {
        add(format!(
            "position: absolute; left: {l}px; top: {t}px; width: {w}px; height: {h}px; \
             border: {border}; background: rgb(255, 0, 255)"
        ));
    }
    let (l, t, w, h) = OUTLINED;
    add(format!(
        "position: absolute; left: {l}px; top: {t}px; width: {w}px; height: {h}px; \
         outline: 1px solid rgb(128, 128, 128); background: rgb(255, 0, 255)"
    ));
    doc.resolve_layout(W as f32, H as f32);
    doc
}

/// At the middle of each edge: every device pixel of the border (or
/// outline), the page just outside it and the background just inside it.
/// The dashed border's own pixels are not sampled (its dashes fall where
/// they fall), only either side of it.
pub fn expectations(scale: u32) -> Vec<((u32, u32), [u8; 3])> {
    let s = scale;
    let mut out = Vec::new();
    for &(l, t, w, h, border) in BOXES {
        // The border box, in device px.
        let (x0, y0) = (l * s, t * s);
        let (x1, y1) = ((l + w + 2) * s, (t + h + 2) * s);
        let (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
        out.push(((x0 - 1, my), WH));
        out.push(((x1, my), WH));
        out.push(((mx, y0 - 1), WH));
        out.push(((mx, y1), WH));
        out.push(((x0 + s, my), BG));
        out.push(((x1 - s - 1, my), BG));
        out.push(((mx, y0 + s), BG));
        out.push(((mx, y1 - s - 1), BG));
        if !border.contains("dashed") {
            for i in 0..s {
                out.push(((x0 + i, my), BD));
                out.push(((x1 - 1 - i, my), BD));
                out.push(((mx, y0 + i), BD));
                out.push(((mx, y1 - 1 - i), BD));
            }
        }
    }
    // The outline: one CSS px outside the box.
    let (l, t, w, h) = OUTLINED;
    let (x0, y0, x1, y1) = (l * s, t * s, (l + w) * s, (t + h) * s);
    let (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
    for i in 1..=s {
        out.push(((x0 - i, my), BD));
        out.push(((x1 - 1 + i, my), BD));
        out.push(((mx, y0 - i), BD));
        out.push(((mx, y1 - 1 + i), BD));
    }
    out.push(((x0 - s - 1, my), WH));
    out.push(((x1 + s, my), WH));
    out.push(((mx, y0 - s - 1), WH));
    out.push(((mx, y1 + s), WH));
    out.push(((x0, my), BG));
    out.push(((x1 - 1, my), BG));
    out
}

/// One missed expectation: `(device point, got, want)`.
pub type Miss = ((u32, u32), [u8; 3], [u8; 3]);

/// The expectations `px` (RGBA rows `pw` wide) misses by more than 2 levels
/// on any channel.
pub fn misses(px: &[[u8; 4]], pw: u32, scale: u32) -> Vec<Miss> {
    expectations(scale)
        .into_iter()
        .filter_map(|((x, y), want)| {
            let p = px[(y * pw + x) as usize];
            let got = [p[0], p[1], p[2]];
            got.iter()
                .zip(want)
                .any(|(a, b)| a.abs_diff(b) > 2)
                .then_some(((x, y), got, want))
        })
        .collect()
}
