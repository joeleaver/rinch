//! Review of #1754 (#1151): Chrome 155 values (headless, --force-device-scale-factor=1,
//! 200x80 window, `body { margin: 0 }`), read at the stated device pixel.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const RB: &str = "linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";
const RBV: &str = "linear-gradient(to bottom, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";

fn render(style: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(node, "style", &format!("position: absolute; {style}"));
    doc.append_child(body, node);
    doc.resolve_layout(200.0, 80.0);
    let mut painter = TinySkiaPainter::new(200, 80);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (200.0, 80.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn rgb(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

fn check(p: &TinySkiaPainter, pts: &[(u32, u32, [u8; 3])]) {
    let bad: Vec<String> = pts
        .iter()
        .filter(|(x, y, c)| (0..3).any(|k| (rgb(p, *x, *y)[k] as i32 - c[k] as i32).abs() > 3))
        .map(|(x, y, c)| format!("({x},{y}): got {:?}, Chrome {c:?}", rgb(p, *x, *y)))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

const R: [u8; 3] = [255, 0, 0];
const B: [u8; 3] = [0, 0, 255];

/// A box at a fractional layout position: Chrome snaps the box (10.5 -> 11)
/// and its tiles start on that pixel, crisp. #1754 keeps the box's fraction
/// in the tile phase and blends every tile edge (pixel 15: (128, 0, 127)).
#[test]
fn a_box_at_a_fractional_layout_position_tiles_from_its_snapped_origin() {
    let p = render(&format!(
        "left: 10.5px; top: 0; width: 60px; height: 20px; background: {RB} 0 0 / 10px 10px"
    ));
    check(
        &p,
        &[
            (11, 7, R),
            (15, 7, R),
            (16, 7, B),
            (20, 7, B),
            (21, 7, R),
            (25, 7, R),
        ],
    );
}

/// Off the half-pixel point: `top: 19.2px` (a 1.2em line above it) with
/// integer `background-position`; Chrome's rows are crisp from 19.
#[test]
fn a_box_below_a_fractional_line_keeps_crisp_rows() {
    let p = render(&format!(
        "left: 0; top: 19.2px; width: 60px; height: 20px; background: {RBV} 0 0 / 10px 10px"
    ));
    check(
        &p,
        &[(0, 20, R), (0, 23, R), (0, 24, B), (0, 28, B), (0, 29, R)],
    );
}

/// The box's fraction is snapped away and the position's kept: left 10.5px,
/// position 0.25px puts the tile at 11.25 in Chrome.
#[test]
fn the_position_fraction_survives_the_box_snap() {
    let p = render(&format!(
        "left: 10.5px; top: 0; width: 60px; height: 20px; background: {RB} 0.25px 0 / 10px 10px"
    ));
    check(
        &p,
        &[
            (11, 7, [191, 0, 64]),
            (12, 7, R),
            (16, 7, [64, 0, 191]),
            (17, 7, B),
            (21, 7, [191, 0, 64]),
        ],
    );
}

/// A transformed box (DeviceSnap::None) keeps bilinear sampling: kills the
/// `on_grid: Self::None => true` mutant, which no #1754 fixture covers.
#[test]
fn a_scaled_box_keeps_its_fraction() {
    let p = render(&format!(
        "left: 0; top: 0; width: 60px; height: 20px; transform: scale(1.5); transform-origin: 0 0; background: {RB} 0.5px 0 / 10px 10px"
    ));
    check(
        &p,
        &[
            (0, 7, [64, 0, 191]),
            (1, 7, R),
            (7, 7, [191, 0, 64]),
            (8, 7, B),
            (15, 7, [64, 0, 191]),
            (22, 7, [191, 0, 64]),
        ],
    );
}
