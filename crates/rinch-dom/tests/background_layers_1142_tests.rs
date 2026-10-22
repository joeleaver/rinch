//! #1142 — every `background-image` layer is painted, not only the first.
//!
//! Every expected pixel is Chrome 155's (headless, `--force-device-scale-factor=1`,
//! measured on the same markup: `position: absolute; left: 0; top: 0` on a white
//! page). Before #1142 rinch-dom read the first entry of each background list
//! and dropped the rest, so each fixture painted one layer of several.
//!
//! - `a` — the transparency checkerboard `ColorSwatch` and the colour
//!   picker's alpha slider ship (four layers, one `background-size`, a
//!   four-entry `background-position`). One layer of four drew triangles.
//! - `b` — two gradients and the colour: a translucent layer over an opaque
//!   one over `background-color`, each a different size. Pins the order
//!   (first listed is on top) and that the colour is under every layer.
//! - `c` — an image over a gradient.
//! - `d` — list repetition (css-backgrounds-3 §2.2): a two-entry
//!   `background-size` gives the third layer the first size, a one-entry
//!   `background-repeat` all three.
//! - `e` — a first layer of `none` and a gradient below it.
//! - `g` — three overlapping layers: the order of the lower ones.
//! - `f` — an image named only by a lower layer is requested.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::image::{ImageLoadResult, ImageLoader};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
    let mut painter = TinySkiaPainter::new(100, 100);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (100.0, 100.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

fn mount(style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; {style}"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(100.0, 100.0);
    (doc, node)
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

const CHECKER: &str = "width: 32px; height: 32px; \
    background-image: linear-gradient(45deg, #ccc 25%, transparent 25%), \
    linear-gradient(-45deg, #ccc 25%, transparent 25%), \
    linear-gradient(45deg, transparent 75%, #ccc 75%), \
    linear-gradient(-45deg, transparent 75%, #ccc 75%); \
    background-size: 8px 8px; background-position: 0 0, 0 4px, 4px -4px, -4px 0;";

/// Chrome 155: a 4px checker, `rgb(204, 204, 204)` where `x / 4 + y / 4` is
/// odd and white elsewhere, every pixel of the 32x32 box (no seam on the
/// diagonals the triangles meet on). One layer of four (before #1142) is 448
/// pixels off it.
#[test]
fn a_the_colour_swatch_checkerboard_is_a_checkerboard() {
    let (mut doc, _) = mount(CHECKER);
    let p = paint(&mut doc);
    let mut bad = Vec::new();
    for y in 0..32 {
        for x in 0..32 {
            let want = if (x / 4 + y / 4) % 2 == 1 {
                [204, 204, 204, 255]
            } else {
                [255, 255, 255, 255]
            };
            let got = px(&p, x, y);
            // Known, separate from #1142 (#TIE): two pixels a tile whose
            // centre lies exactly on the first layer's 25% hard stop
            // (`(0.5, 4.5)` and `(1.5, 5.5)` in the tile) take the colour
            // after the stop, white, where Chrome takes `#ccc`.
            let on_the_stop = matches!((x % 8, y % 8), (0, 4) | (1, 5));
            if !close(got, want) && !(on_the_stop && got == [255, 255, 255, 255]) {
                bad.push((x, y, got));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} pixels off the checker: {:?}",
        bad.len(),
        &bad[..bad.len().min(12)]
    );
}

/// Chrome 155: (5,5) blue (layer 2 alone), (15,15) and (25,25) the 50% red
/// over blue `(128, 0, 127)`, (35,35) and (32,5) the green colour, which is
/// under both layers.
#[test]
fn b_two_gradients_over_the_colour_in_list_order() {
    let (mut doc, _) = mount(
        "width: 40px; height: 40px; background-color: rgb(0, 255, 0); \
         background-image: linear-gradient(rgba(255, 0, 0, 0.5), rgba(255, 0, 0, 0.5)), \
         linear-gradient(rgb(0, 0, 255), rgb(0, 0, 255)); \
         background-size: 20px 20px, 30px 30px; background-repeat: no-repeat; \
         background-position: 10px 10px, 0 0;",
    );
    let p = paint(&mut doc);
    for (x, y, want) in [
        (5, 5, [0, 0, 255, 255]),
        (15, 15, [128, 0, 127, 255]),
        (25, 25, [128, 0, 127, 255]),
        (35, 35, [0, 255, 0, 255]),
        (32, 5, [0, 255, 0, 255]),
    ] {
        let got = px(&p, x, y);
        assert!(close(got, want), "({x},{y}): {got:?}, Chrome {want:?}");
    }
}

/// A 4x4 image, its left half opaque magenta and its right half transparent.
fn half_magenta() -> rinch_dom::image_cache::DecodedImage {
    let mut data = Vec::new();
    for _ in 0..4 {
        for x in 0..4 {
            data.extend_from_slice(if x < 2 {
                &[255, 0, 255, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    rinch_dom::image_cache::DecodedImage::new(data, 4, 4)
}

/// Chrome 155 (the same image as a PNG data URI): (9,9) magenta, (11,9) and
/// (9,13) the yellow gradient through and beside the image, (2,2) yellow.
#[test]
fn c_an_image_over_a_gradient() {
    let (mut doc, _) = mount(
        "width: 24px; height: 24px; \
         background-image: url(r1142.png), linear-gradient(to right, rgb(255, 255, 0), rgb(255, 255, 0)); \
         background-repeat: no-repeat; background-position: 8px 8px, 0 0;",
    );
    doc.tree
        .image_cache
        .insert_decoded("r1142.png".to_string(), half_magenta());
    let p = paint(&mut doc);
    for (x, y, want) in [
        (9, 9, [255, 0, 255, 255]),
        (11, 9, [255, 255, 0, 255]),
        (2, 2, [255, 255, 0, 255]),
        (9, 13, [255, 255, 0, 255]),
    ] {
        let got = px(&p, x, y);
        assert!(close(got, want), "({x},{y}): {got:?}, Chrome {want:?}");
    }
}

/// Chrome 155: red 10x10 at 0 0, green 20x20 at 20px 0, and the third layer
/// blue at 45px 0 with the **first** size (10x10): (50,5) blue, (50,15)
/// white. `no-repeat` reaches all three: (5,15) and (15,5) white.
#[test]
fn d_shorter_lists_repeat_to_the_image_count() {
    let (mut doc, _) = mount(
        "width: 60px; height: 30px; \
         background-image: linear-gradient(rgb(255, 0, 0), rgb(255, 0, 0)), \
         linear-gradient(rgb(0, 255, 0), rgb(0, 255, 0)), \
         linear-gradient(rgb(0, 0, 255), rgb(0, 0, 255)); \
         background-size: 10px 10px, 20px 20px; background-repeat: no-repeat; \
         background-position: 0 0, 20px 0, 45px 0;",
    );
    let p = paint(&mut doc);
    for (x, y, want) in [
        (5, 5, [255, 0, 0, 255]),
        (25, 5, [0, 255, 0, 255]),
        (25, 15, [0, 255, 0, 255]),
        (50, 5, [0, 0, 255, 255]),
        (50, 15, [255, 255, 255, 255]),
        (5, 15, [255, 255, 255, 255]),
        (15, 5, [255, 255, 255, 255]),
    ] {
        let got = px(&p, x, y);
        assert!(close(got, want), "({x},{y}): {got:?}, Chrome {want:?}");
    }
}

/// Chrome 155: three opaque layers, each larger than the one above it —
/// (5,5) red, (15,15) green, (25,25) blue, (35,35) the white page. Pins the
/// order of the layers below the first among themselves.
#[test]
fn g_three_layers_stack_in_list_order() {
    let (mut doc, _) = mount(
        "width: 40px; height: 40px; \
         background-image: linear-gradient(rgb(255, 0, 0), rgb(255, 0, 0)), \
         linear-gradient(rgb(0, 255, 0), rgb(0, 255, 0)), \
         linear-gradient(rgb(0, 0, 255), rgb(0, 0, 255)); \
         background-size: 10px 10px, 20px 20px, 30px 30px; background-repeat: no-repeat;",
    );
    let p = paint(&mut doc);
    for (x, y, want) in [
        (5, 5, [255, 0, 0, 255]),
        (15, 15, [0, 255, 0, 255]),
        (25, 25, [0, 0, 255, 255]),
        (35, 35, [255, 255, 255, 255]),
    ] {
        let got = px(&p, x, y);
        assert!(close(got, want), "({x},{y}): {got:?}, Chrome {want:?}");
    }
}

/// Chrome 155: blue.
#[test]
fn e_a_first_layer_of_none_paints_the_layers_below_it() {
    let (mut doc, _) = mount(
        "width: 20px; height: 20px; \
         background-image: none, linear-gradient(rgb(0, 0, 255), rgb(0, 0, 255));",
    );
    let p = paint(&mut doc);
    assert!(close(px(&p, 5, 5), [0, 0, 255, 255]), "{:?}", px(&p, 5, 5));
}

/// A restyle that changes only a lower layer repaints it.
#[test]
fn a_change_to_a_lower_layer_repaints() {
    let (mut doc, node) = mount(
        "width: 20px; height: 20px; \
         background-image: none, linear-gradient(rgb(0, 0, 255), rgb(0, 0, 255));",
    );
    let _ = paint(&mut doc);
    doc.tree.paint_dirty_nodes.clear();
    doc.set_attribute(
        node,
        "style",
        "position: absolute; left: 0; top: 0; width: 20px; height: 20px; \
         background-image: none, linear-gradient(rgb(255, 0, 0), rgb(255, 0, 0));",
    );
    doc.resolve_layout(100.0, 100.0);
    assert!(
        doc.tree.paint_dirty_nodes.contains(&node.0),
        "the node must be repainted"
    );
    let p = paint(&mut doc);
    assert!(close(px(&p, 5, 5), [255, 0, 0, 255]), "{:?}", px(&p, 5, 5));
}

struct CountingLoader(Arc<AtomicUsize>);

impl ImageLoader for CountingLoader {
    fn load(&self, _src: &str) -> ImageLoadResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        ImageLoadResult::Failed("nope".into())
    }
}

#[test]
fn f_an_image_named_only_by_a_lower_layer_is_requested() {
    let mut doc = RinchDocument::new();
    let count = Arc::new(AtomicUsize::new(0));
    doc.tree.image_loader = Some(Arc::new(CountingLoader(count.clone())));
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; \
         background-image: linear-gradient(red, red), url(bg-1142-lower.png)",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.image_cache.contains("bg-1142-lower.png"),
        "a lower layer's url must be loaded"
    );
}
