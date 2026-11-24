//! The #2368 scene and Chrome 158's pixels of it, shared by the software
//! painter's `fractional_scale_box_snap_2368_tests` and the GPU painter's
//! `gpu_box_snap_2368` (`rinch/tests`, through `#[path]`). What it pins and
//! why: the test file's doc.

#![allow(dead_code)]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

pub const VW: f32 = 420.0;
pub const VH: f32 = 280.0;

const B: &str = "position:absolute;left:13px;top:9px;width:40px;height:30px;box-sizing:border-box";
const BIG: &str = "<div style=\"position:absolute;left:-20px;top:-20px;width:200px;\
                   height:200px;background:#000\"></div>";

/// The scene, one 100x60 cell per case; each box is 40x30 at (13, 9) in its
/// cell, the padded-child and translated-ancestor ones a 20x20 child at 1px.
fn scene() -> String {
    let cell = |l: u32, t: u32, inner: String| {
        format!(
            "<div style=\"position:absolute;left:{l}px;top:{t}px;width:100px;height:60px\">\
             {inner}</div>"
        )
    };
    [
        cell(0, 0, format!("<div style=\"{B};background:#000\"></div>")),
        cell(100, 0, format!("<div style=\"{B};border:3px solid #000;background:#fff\"></div>")),
        cell(200, 0, format!("<div style=\"{B};overflow:hidden\">{BIG}</div>")),
        cell(300, 0, format!("<div style=\"{B};overflow:hidden;border-radius:5px\">{BIG}</div>")),
        cell(
            0,
            100,
            format!(
                "<div style=\"{B};overflow:hidden\"><div style=\"position:absolute;left:4px;\
                 top:4px;width:30px;height:20px;overflow:hidden\">{BIG}</div></div>"
            ),
        ),
        cell(100, 100, format!("<div style=\"{B};background:#000;transform:translateX(0.4px)\"></div>")),
        cell(
            200,
            100,
            format!("<div style=\"{B};background:#000;transform:scale(1.1);transform-origin:0 0\"></div>"),
        ),
        cell(
            300,
            100,
            format!(
                "<div style=\"{B};padding:1px\"><div style=\"width:20px;height:20px;\
                 background:#000\"></div></div>"
            ),
        ),
        cell(
            0,
            200,
            format!(
                "<div style=\"{B};transform:translateX(0.4px)\"><div style=\"position:absolute;\
                 left:1px;top:1px;width:20px;height:20px;background:#000\"></div></div>"
            ),
        ),
        cell(100, 200, format!("<div style=\"{B};border:1px solid #000;background:#fff\"></div>")),
        cell(200, 200, format!("<div style=\"{B};overflow:hidden;border:3px solid #888\">{BIG}</div>")),
        cell(300, 200, format!("<div style=\"{B};background:#000;opacity:0.5\"></div>")),
    ]
    .concat()
}

/// Each case's four probes, as Chrome's dump of this scene gives them: the
/// row across the left edge (8 px from two before it), across the right edge
/// (7 px from three before it), and the columns across the top and bottom.
pub type Probe = (&'static str, [u8; 8], [u8; 7], [u8; 8], [u8; 7]);

const CELLS: [(f64, f64); 12] = [
    (0., 0.),
    (100., 0.),
    (200., 0.),
    (300., 0.),
    (0., 100.),
    (100., 100.),
    (200., 100.),
    (300., 100.),
    (0., 200.),
    (100., 200.),
    (200., 200.),
    (300., 200.),
];

/// Chrome 158 at 1.25x.
pub const CHROME_125: [Probe; 12] = [
    (
        "bg",
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 255, 255, 255, 255],
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "border3",
        [255, 255, 0, 0, 0, 255, 255, 255],
        [0, 0, 0, 255, 255, 255, 255],
        [255, 255, 0, 0, 0, 255, 255, 255],
        [255, 0, 0, 0, 255, 255, 255],
    ),
    (
        "clip",
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 255, 255, 255, 255],
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "clip_r5",
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 255, 255, 255, 255],
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "nested",
        [255, 255, 255, 255, 255, 255, 255, 0],
        [255, 255, 255, 255, 255, 255, 255],
        [255, 255, 255, 255, 255, 255, 255, 0],
        [255, 255, 255, 255, 255, 255, 255],
    ),
    (
        "translate",
        [255, 255, 127, 0, 0, 0, 0, 0],
        [0, 0, 0, 127, 255, 255, 255],
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "scale",
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0],
        [255, 255, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0],
    ),
    (
        "padded_child",
        [255, 255, 255, 255, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
        [255, 255, 255, 255, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "translated_ancestor",
        [255, 255, 255, 255, 127, 0, 0, 0],
        [0, 0, 0, 0, 127, 255, 255],
        [255, 255, 255, 255, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "border1",
        [255, 255, 0, 255, 255, 255, 255, 255],
        [255, 255, 0, 255, 255, 255, 255],
        [255, 255, 0, 255, 255, 255, 255, 255],
        [255, 255, 255, 0, 255, 255, 255],
    ),
    (
        "clip_border3",
        [255, 255, 136, 136, 136, 0, 0, 0],
        [136, 136, 136, 255, 255, 255, 255],
        [255, 255, 136, 136, 136, 0, 0, 0],
        [0, 136, 136, 136, 255, 255, 255],
    ),
    (
        "opacity",
        [255, 255, 127, 127, 127, 127, 127, 127],
        [127, 127, 127, 255, 255, 255, 255],
        [255, 255, 127, 127, 127, 127, 127, 127],
        [127, 127, 127, 127, 255, 255, 255],
    ),
];
/// Chrome 158 at 1.5x.
pub const CHROME_15: [Probe; 12] = [
    (
        "bg",
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "border3",
        [255, 255, 255, 0, 0, 0, 0, 255],
        [0, 0, 0, 0, 255, 255, 255],
        [255, 255, 255, 0, 0, 0, 0, 255],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "clip",
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "clip_r5",
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "nested",
        [255, 255, 255, 255, 255, 255, 255, 255],
        [255, 255, 255, 255, 255, 255, 255],
        [255, 255, 255, 255, 255, 255, 255, 255],
        [255, 255, 255, 255, 255, 255, 255],
    ),
    (
        "translate",
        [255, 255, 255, 153, 0, 0, 0, 0],
        [0, 0, 0, 0, 101, 255, 255],
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 255, 255, 255],
    ),
    (
        "scale",
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0],
        [255, 255, 255, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0],
    ),
    (
        "padded_child",
        [255, 255, 255, 255, 0, 0, 0, 0],
        [0, 0, 0, 255, 255, 255, 255],
        [255, 255, 255, 255, 0, 0, 0, 0],
        [0, 0, 0, 255, 255, 255, 255],
    ),
    (
        "translated_ancestor",
        [255, 255, 255, 255, 153, 0, 0, 0],
        [0, 0, 0, 101, 255, 255, 255],
        [255, 255, 255, 255, 0, 0, 0, 0],
        [0, 0, 0, 255, 255, 255, 255],
    ),
    (
        "border1",
        [255, 255, 255, 0, 255, 255, 255, 255],
        [255, 255, 255, 0, 255, 255, 255],
        [255, 255, 255, 0, 255, 255, 255, 255],
        [255, 255, 255, 0, 255, 255, 255],
    ),
    (
        "clip_border3",
        [255, 255, 255, 136, 136, 136, 136, 0],
        [136, 136, 136, 136, 255, 255, 255],
        [255, 255, 255, 136, 136, 136, 136, 0],
        [136, 136, 136, 136, 255, 255, 255],
    ),
    (
        "opacity",
        [255, 255, 255, 127, 127, 127, 127, 127],
        [127, 127, 127, 127, 255, 255, 255],
        [255, 255, 255, 127, 127, 127, 127, 127],
        [127, 127, 127, 127, 255, 255, 255],
    ),
];

/// The scene laid out at `scale`, as the desktop shell lays it out before it
/// paints there (the device pixel ratio set, so Stylo puts border widths on
/// the device grid).
pub fn document(scale: f64) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.set_device_pixel_ratio(scale as f32);
    let body = doc.body();
    doc.set_attribute(body, "style", "margin:0;background:#fff");
    doc.set_inner_html(body, &scene());
    doc.resolve_layout(VW, VH);
    doc
}

/// The surface size at `scale`, in device px.
pub fn surface(scale: f64) -> (u32, u32) {
    (
        (VW as f64 * scale).round() as u32,
        (VH as f64 * scale).round() as u32,
    )
}

/// The probes of a paint at `scale` (`g`, one gray byte a device pixel, rows
/// `w` long), at the pixels the Chrome table was read at.
fn probes(g: &[u8], w: usize, scale: f64, names: impl Iterator<Item = &'static str>) -> Vec<Probe> {
    let at = |x: usize, y: usize| g[y * w + x];
    let s = scale;
    CELLS
        .iter()
        .zip(names)
        .map(|(&(cx, cy), name)| {
            let child = matches!(name, "padded_child" | "translated_ancestor");
            let (row, col, r, b) = if child {
                (
                    (cy + 20.0) * s,
                    (cx + 20.0) * s,
                    (cx + 34.0) * s,
                    (cy + 30.0) * s,
                )
            } else {
                (
                    (cy + 24.0) * s,
                    (cx + 33.0) * s,
                    (cx + 53.0) * s,
                    (cy + 39.0) * s,
                )
            };
            let (row, col) = (row as usize, col as usize);
            let l0 = ((cx + 13.0) * s) as usize - 2;
            let t0 = ((cy + 9.0) * s) as usize - 2;
            let (r0, b0) = (r as usize - 3, b as usize - 3);
            (
                name,
                std::array::from_fn(|i| at(l0 + i, row)),
                std::array::from_fn(|i| at(r0 + i, row)),
                std::array::from_fn(|i| at(col, t0 + i)),
                std::array::from_fn(|i| at(col, b0 + i)),
            )
        })
        .collect()
}

/// Every probe of a paint at `scale` against Chrome's, each mismatch named. A pixel
/// Chrome fills or leaves (0, 255, or the 3px border's 136) must be within 3
/// levels; a pixel Chrome blends (the translated edges) within 32, since
/// tiny-skia's coverage of a partly covered pixel comes in quarters (a 0.6
/// device px translate at 1.5x is 0.5 covered, 127, where Chrome has 153):
/// that is the edge where Chrome's is, blended as Chrome's is, which is what
/// a snapped transform (a hard edge) or an unsnapped box (the blend a column
/// off) fails.
///
/// Under `scale(1.1)` an edge is off the grid by a sliver: the snapped box is
/// scaled about the box's own (unsnapped) origin, so its top edge is at 10.975
/// at 1.25x and at 14.05 at 1.5x. Vello's analytic coverage draws those
/// slivers (249 for 255, 13 for 0); Chrome's software raster, whose coverage
/// like tiny-skia's comes in vertical quarters, draws neither, so these
/// pixels cannot tell that sliver from an edge exactly on the grid. That
/// case's filled and empty pixels are within 16 (5% of a pixel); its blended
/// ones, and a column-wide error, are still caught.
pub fn mismatches(g: &[u8], w: usize, scale: f64, chrome: &[Probe]) -> Vec<String> {
    let ours = probes(g, w, scale, chrome.iter().map(|p| p.0));
    let close = |case: &str, a: &[u8], b: &[u8]| {
        let hard = if case == "scale" { 16 } else { 3 };
        a.iter().zip(b).all(|(&a, &b)| {
            let blended = !matches!(b, 0 | 136 | 255);
            a.abs_diff(b) <= if blended { 32 } else { hard }
        })
    };
    let mut wrong = Vec::new();
    for (o, c) in ours.iter().zip(chrome) {
        for (side, a, b) in [
            ("left", &o.1[..], &c.1[..]),
            ("right", &o.2[..], &c.2[..]),
            ("top", &o.3[..], &c.3[..]),
            ("bottom", &o.4[..], &c.4[..]),
        ] {
            if !close(c.0, a, b) {
                wrong.push(format!("{} {side}: rinch {a:?}, Chrome {b:?}", c.0));
            }
        }
    }
    wrong
}
