//! #468 — `background-size`, `background-position` and `background-repeat`
//! are painted, and `background-position` animates.
//!
//! The striped `Progress` bar and the `Skeleton` pulse both animate
//! `background-position` over a gradient sized by `background-size`. None of
//! the three properties reached `ComputedStyle`, so paint stretched the
//! gradient over the whole border box and the keyframes had nothing to write
//! into: the animation was registered, "running", and moved no pixel.
//!
//! # The oracle
//!
//! Every static expectation below was measured in Chrome 153 (headless,
//! device scale 1) on the same markup: a 100x20 box whose image is a
//! hard-stopped two-colour gradient, `red 0 50%, blue 50% 100%`, so each tile
//! reads as a red half then a blue half and a sampled pixel says which tile,
//! and which half of it, covers that column. Runs are `[start, end)` in CSS px:
//!
//! | case | Chrome 153 |
//! |---|---|
//! | `40px 20px` at `30px 0`, `no-repeat` | W 0-30, R 30-50, B 50-70, W 70-100 |
//! | `40px 20px` at `30px 0`, `repeat` | R 0-10, B 10-30, R 30-50, B 50-70, R 70-90, B 90-100 |
//! | `50% 100%` at `100% 0`, `no-repeat` | W 0-50, R 50-75, B 75-100 |
//! | `background-color` under a `no-repeat` tile | G 0-30, R 30-50, B 50-70, G 70-100 |
//! | `200% 100%` at `-50% 0` (the Skeleton's shape) | B 0-50, R 50-100 |
//! | `40px 20px` at `right 10px top 0`, `no-repeat` | W 0-50, R 50-70, B 70-90, W 90-100 |
//!
//! Every position is **off** zero on purpose: at `0 0` with `auto` size the
//! tile is the box and the unfixed paint (the gradient stretched over the
//! box) is already right, so a fixture there passes with the fix absent.
//! Samples sit mid-run, clear of the anti-aliased stop edges.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 200.0;
const VH: f32 = 100.0;

const GRADIENT: &str =
    "background-image: linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";

fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

/// `R`, `B`, `G` or `W` for a pure red, blue, green or white pixel; `?` for
/// anything else (an edge, or a colour the fixture did not ask for).
fn class_at(painter: &TinySkiaPainter, x: u32, y: u32) -> char {
    let idx = ((y * painter.width() + x) * 4) as usize;
    let d = painter.pixels();
    match (d[idx], d[idx + 1], d[idx + 2], d[idx + 3]) {
        (255, 0, 0, 255) => 'R',
        (0, 0, 255, 255) => 'B',
        (0, 255, 0, 255) => 'G',
        (_, _, _, 0) | (255, 255, 255, 255) => 'W',
        _ => '?',
    }
}

/// A 100x20 box at the viewport's top-left with `style` after the gradient,
/// on a white page, laid out and painted.
fn one_box(style: &str) -> TinySkiaPainter {
    let (mut doc, _) = mount("", style);
    paint(&mut doc)
}

fn mount(css: &str, style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "html, body {{ margin: 0; background: rgb(255, 255, 255); }} {css}"
    ));
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!(
            "position: absolute; left: 0; top: 0; width: 100px; height: 20px; {GRADIENT}; {style}"
        ),
    );
    doc.append_child(body, node);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VW, VH);
    (doc, node)
}

/// The class of each sampled column on the box's middle row.
fn row(painter: &TinySkiaPainter, xs: &[u32]) -> String {
    xs.iter().map(|&x| class_at(painter, x, 10)).collect()
}

#[test]
fn a_sized_tile_is_placed_at_its_position_without_repeating() {
    let p = one_box(
        "background-size: 40px 20px; background-position: 30px 0; background-repeat: no-repeat",
    );
    assert_eq!(
        row(&p, &[15, 40, 60, 85]),
        "WRBW",
        "Chrome 153: W 0-30, R 30-50, B 50-70, W 70-100"
    );
}

#[test]
fn a_sized_tile_repeats_both_ways_from_its_position() {
    let p = one_box("background-size: 40px 20px; background-position: 30px 0");
    assert_eq!(
        row(&p, &[5, 20, 40, 60, 80, 95]),
        "RBRBRB",
        "Chrome 153: R 0-10, B 10-30, R 30-50, B 50-70, R 70-90, B 90-100"
    );
}

#[test]
fn a_percentage_position_is_a_share_of_the_free_space() {
    let p = one_box(
        "background-size: 50% 100%; background-position: 100% 0; background-repeat: no-repeat",
    );
    assert_eq!(
        row(&p, &[25, 62, 88]),
        "WRB",
        "Chrome 153: W 0-50, R 50-75, B 75-100"
    );
}

#[test]
fn the_background_colour_is_painted_under_the_image() {
    let p = one_box(
        "background-color: rgb(0, 255, 0); background-size: 40px 20px; \
         background-position: 30px 0; background-repeat: no-repeat",
    );
    assert_eq!(
        row(&p, &[15, 40, 60, 85]),
        "GRBG",
        "Chrome 153: G 0-30, R 30-50, B 50-70, G 70-100"
    );
}

#[test]
fn a_negative_percentage_on_an_oversized_tile_shifts_it_right() {
    let p = one_box("background-size: 200% 100%; background-position: -50% 0");
    assert_eq!(row(&p, &[25, 75]), "BR", "Chrome 153: B 0-50, R 50-100");
}

#[test]
fn an_edge_offset_position_measures_from_that_edge() {
    let p = one_box(
        "background-size: 40px 20px; background-position: right 10px top 0; background-repeat: no-repeat",
    );
    assert_eq!(
        row(&p, &[25, 60, 80, 95]),
        "WRBW",
        "Chrome 153: W 0-50, R 50-70, B 70-90, W 90-100"
    );
}

// =============================================================================
// Animation
// =============================================================================

/// The node's first animation, and its start time.
fn start_of(doc: &RinchDocument, node: NodeId) -> f64 {
    doc.tree.active_animations[&node.0][0].start_time_ms
}

/// The `Progress` shape: a length stop to a zero stop, on a sized tile.
/// At a quarter of the way, `40px → 0` is `30px`, which is the repeating
/// fixture's placement above.
#[test]
fn a_length_background_position_animation_moves_the_tile() {
    let (mut doc, node) = mount(
        "@keyframes k468-stripes { from { background-position: 40px 0; } to { background-position: 0 0; } }
         .s { animation: k468-stripes 1000ms linear infinite; }",
        "background-size: 40px 20px",
    );
    doc.set_attribute(node, "class", "s");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(
        doc.tree.active_animations.get(&node.0).map(Vec::len),
        Some(1)
    );
    let t0 = start_of(&doc, node);

    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 250.0);
    let p = paint(&mut doc);
    assert_eq!(
        row(&p, &[5, 20, 40, 60, 80, 95]),
        "RBRBRB",
        "a quarter of the way from 40px to 0 is 30px: the repeating fixture's placement"
    );

    // Off the first sample, so a stuck value cannot pass both.
    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 750.0);
    let p = paint(&mut doc);
    // 10px: tiles at -30..10, 10..50, 50..90, 90..130.
    assert_eq!(
        row(&p, &[5, 20, 40, 60, 80, 95]),
        "BRBRBR",
        "three quarters of the way is 10px"
    );
}

/// The `Skeleton` shape: percentage stops on an oversized tile.
/// `200% → -200%` at 62.5% is `-50%`, the fixture measured above.
#[test]
fn a_percentage_background_position_animation_moves_the_tile() {
    let (mut doc, node) = mount(
        "@keyframes k468-pulse { 0% { background-position: 200% 0; } 100% { background-position: -200% 0; } }
         .s { animation: k468-pulse 1000ms linear infinite; }",
        "background-size: 200% 100%",
    );
    doc.set_attribute(node, "class", "s");
    doc.resolve_layout(VW + 1.0, VH);
    let t0 = start_of(&doc, node);

    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 625.0);
    let p = paint(&mut doc);
    assert_eq!(row(&p, &[25, 75]), "BR", "-50%: B 0-50, R 50-100");

    // 50% → 0%: offset 0, the tile's red half on screen.
    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 500.0);
    let p = paint(&mut doc);
    assert_eq!(
        row(&p, &[25, 75]),
        "RR",
        "0%: the tile's left (red) half covers the box"
    );
}

/// An animated background moves paint, not boxes: a tick marks the node
/// paint-dirty and leaves layout alone (the frame must not re-run Taffy).
#[test]
fn a_background_position_tick_dirties_paint_only() {
    let (mut doc, node) = mount(
        "@keyframes k468-p { from { background-position: 40px 0; } to { background-position: 0 0; } }
         .s { animation: k468-p 1000ms linear infinite; }",
        "background-size: 40px 20px",
    );
    doc.set_attribute(node, "class", "s");
    doc.resolve_layout(VW + 1.0, VH);
    let t0 = start_of(&doc, node);
    doc.tree.paint_dirty_nodes.clear();
    doc.tree.dirty_nodes.clear();

    assert!(rinch_dom::animation::tick_animations(
        &mut doc.tree,
        t0 + 300.0
    ));
    assert!(
        doc.tree.paint_dirty_nodes.contains(&node.0),
        "the tick moved the tile, so the node must repaint"
    );
    assert!(
        !doc.tree.dirty_nodes.contains(&node.0),
        "background-position is paint-only; the tick must not owe a layout"
    );
}
