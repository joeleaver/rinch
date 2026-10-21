//! A box laid out `0x0` still paints its descendants (#1696).
//!
//! CSS paints a box's overflowing content whatever the box's own size: a
//! `width: 0; height: 0` box draws no background or border (there is no area
//! to fill) but everything inside it that reaches past it is drawn, at the
//! box's origin, under its transform and its opacity, and clipped by its own
//! `overflow` — a `0x0` `overflow: hidden` box clips everything. Chrome 153
//! does all of that.
//!
//! `paint_node` returned from a box zero in **both** dimensions without
//! visiting its children (a box zero in one was walked since #142). A
//! positioned descendant of a `0x0` box that is not a stacking context is
//! hoisted to an outer sequence and painted anyway, which is what hid it; an
//! in-flow descendant, and every descendant of a `0x0` **stacking context**,
//! is an entry of that box's own sequence and was never drawn. The shape it
//! was found in is the `Editor` overlay slot (#1677): `position: absolute;
//! width: 0; height: 0; z-index: 2`, whose marker children were hit-tested
//! where they were laid out and drawn nowhere.
//!
//! Every assertion here reads pixels of the software painter, in a region
//! where the correct output is provably one colour, except
//! [`the_gpu_painter_records_the_child`], which reads the scene the GPU
//! painter records (the paint walk is the same code for both; what Vello does
//! with the scene is not testable without a GPU).
//!
//! The fixed points stepped off: the box is never at the origin (`left: 20px;
//! top: 30px`), the child is offset from it on both axes, the scale is 1 and
//! 2, and the transform and opacity are not identities.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 300.0;
const VH: f32 = 300.0;

const GREEN: [u8; 3] = [0, 128, 0];

fn paint(doc: &mut RinchDocument, scale: f64) -> TinySkiaPainter {
    let mut painter = TinySkiaPainter::new((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn pixel_at(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

/// Opaque `rgb` pixels in `[x0, x1) x [y0, y1)`.
fn count_rgb(p: &TinySkiaPainter, (x0, x1, y0, y1): (u32, u32, u32, u32), rgb: [u8; 3]) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = pixel_at(p, x, y);
            if px[..3] == rgb && px[3] == 255 {
                n += 1;
            }
        }
    }
    n
}

/// Pixels in the region with any green at all.
fn count_any_green(p: &TinySkiaPainter, (x0, x1, y0, y1): (u32, u32, u32, u32)) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            if pixel_at(p, x, y)[1] > 0 {
                n += 1;
            }
        }
    }
    n
}

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

/// body (margin 0) > container (relative, 200x200 at 10,10) > box (`box_style`,
/// at `left: 20px; top: 30px`, `0x0`) > child (absolute, 18x18 green at
/// `left: 10px; top: 12px`): the child is at (40, 52)–(58, 70).
fn slot_doc(box_style: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let container = el(
        &mut doc,
        body,
        "position: relative; left: 10px; top: 10px; width: 200px; height: 200px",
    );
    let bx = el(
        &mut doc,
        container,
        &format!("position: absolute; left: 20px; top: 30px; width: 0; height: 0; {box_style}"),
    );
    let child = el(
        &mut doc,
        bx,
        "position: absolute; left: 10px; top: 12px; width: 18px; height: 18px; \
         background-color: rgb(0, 128, 0)",
    );
    doc.resolve_layout(VW, VH);
    (doc, bx, child)
}

const CHILD: (u32, u32, u32, u32) = (40, 58, 52, 70);
const WINDOW: (u32, u32, u32, u32) = (0, 300, 0, 300);

/// The overlay-slot shape: a `0x0` absolutely positioned stacking context
/// (`z-index: 2`). Its child is an entry of its own sequence; it was drawn
/// nowhere.
#[test]
fn a_zero_by_zero_stacking_context_paints_its_child() {
    let (mut doc, bx, _) = slot_doc("z-index: 2");
    let l = doc.tree.get(bx.0).unwrap().layout;
    assert_eq!(
        (l.width, l.height),
        (0.0, 0.0),
        "precondition: the box is 0x0"
    );
    let p = paint(&mut doc, 1.0);
    assert_eq!(
        count_rgb(&p, CHILD, GREEN),
        18 * 18,
        "the child is drawn whole"
    );
    assert_eq!(
        count_any_green(&p, WINDOW),
        18 * 18,
        "and nowhere else (it is drawn at the box's origin, not the container's)"
    );
}

/// The same at scale 2: the box's origin and the child's offset both scale.
#[test]
fn a_zero_by_zero_stacking_context_paints_its_child_at_scale_two() {
    let (mut doc, _, _) = slot_doc("z-index: 2");
    let p = paint(&mut doc, 2.0);
    assert_eq!(count_rgb(&p, (80, 116, 104, 140), GREEN), 36 * 36);
    assert_eq!(count_any_green(&p, (0, 600, 0, 600)), 36 * 36);
}

/// An in-flow child of a static `0x0` box: no stacking context, nothing
/// hoisted, so it is in the box's own (tree-order) walk.
#[test]
fn a_static_zero_by_zero_box_paints_its_in_flow_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let pad = el(&mut doc, body, "padding: 20px 0 0 30px");
    let bx = el(&mut doc, pad, "width: 0; height: 0");
    el(
        &mut doc,
        bx,
        "width: 50px; height: 40px; background-color: rgb(0, 128, 0)",
    );
    doc.resolve_layout(VW, VH);
    let l = doc.tree.get(bx.0).unwrap().layout;
    assert_eq!((l.width, l.height), (0.0, 0.0), "precondition");
    let p = paint(&mut doc, 1.0);
    assert_eq!(count_rgb(&p, (30, 80, 20, 60), GREEN), 50 * 40);
    assert_eq!(count_any_green(&p, WINDOW), 50 * 40);
}

/// The box's own transform moves its descendants.
#[test]
fn a_zero_by_zero_box_keeps_its_transform() {
    let (mut doc, _, _) = slot_doc("z-index: 2; transform: translate(40px, 25px)");
    let p = paint(&mut doc, 1.0);
    assert_eq!(count_rgb(&p, (80, 98, 77, 95), GREEN), 18 * 18);
    assert_eq!(count_any_green(&p, WINDOW), 18 * 18);
}

/// The box's own opacity fades its descendants (one group, not per child).
#[test]
fn a_zero_by_zero_box_keeps_its_opacity() {
    let (mut doc, _, _) = slot_doc("opacity: 0.5");
    let p = paint(&mut doc, 1.0);
    let px = pixel_at(&p, 49, 61);
    assert!(
        (120..=136).contains(&px[3]) && px[1] > 0 && px[0] == 0 && px[2] == 0,
        "half-faded green, got {px:?}"
    );
    assert_eq!(count_any_green(&p, WINDOW), 18 * 18);
}

/// A `0x0` `overflow: hidden` box clips everything inside it — an in-flow
/// child and a positioned one it contains (it is `position: absolute`, so the
/// absolute child's containing block).
#[test]
fn a_zero_by_zero_overflow_hidden_box_clips_everything() {
    let (mut doc, bx, _) = slot_doc("z-index: 2; overflow: hidden");
    el(
        &mut doc,
        bx,
        "width: 30px; height: 30px; background-color: rgb(0, 128, 0)",
    );
    doc.resolve_layout(VW + 1.0, VH);
    let p = paint(&mut doc, 1.0);
    assert_eq!(count_any_green(&p, WINDOW), 0);
}

/// Hidden is hidden: a `display: none` box (also `0x0`) paints nothing.
#[test]
fn a_display_none_box_paints_nothing() {
    let (mut doc, _, _) = slot_doc("z-index: 2; display: none");
    let p = paint(&mut doc, 1.0);
    assert_eq!(count_any_green(&p, WINDOW), 0);
}

/// `visibility: hidden` on the box hides the child, which inherits it.
#[test]
fn a_visibility_hidden_zero_by_zero_box_paints_nothing() {
    let (mut doc, _, _) = slot_doc("z-index: 2; visibility: hidden");
    let p = paint(&mut doc, 1.0);
    assert_eq!(count_any_green(&p, WINDOW), 0);
}

/// The GPU painter is handed the same draw: the paint walk is shared, so the
/// scene records one more solid fill with the child than with the child
/// hidden.
#[test]
fn the_gpu_painter_records_the_child() {
    use rinch_dom::paint::vello_painter::VelloPainter;
    fn color_draws(doc: &mut RinchDocument) -> usize {
        let mut vp = VelloPainter::new();
        let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut vp,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut layout_cx,
        );
        // `DrawTag::COLOR`.
        vp.scene()
            .encoding()
            .draw_tags
            .iter()
            .filter(|t| t.0 == 0x44)
            .count()
    }
    let (mut doc, _, child) = slot_doc("z-index: 2");
    let with = color_draws(&mut doc);
    doc.set_style(child, "display", "none");
    doc.resolve_layout(VW + 1.0, VH);
    let without = color_draws(&mut doc);
    assert_eq!(with, without + 1, "the child's background is in the scene");
}

/// A `0x0` `<svg>` or `<img>` does not render an HTML child (Chrome 155 draws
/// neither): `zero_box_paints_children` leaves both out, as their full-size
/// arms never walk their children.
#[test]
fn a_zero_by_zero_svg_or_img_paints_no_html_child() {
    for tag in ["svg", "img"] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0");
        let pad = el(&mut doc, body, "padding: 20px 0 0 30px");
        let host = doc.create_element(tag);
        doc.set_attribute(host, "style", "display: block; width: 0; height: 0");
        doc.append_child(pad, host);
        el(
            &mut doc,
            host,
            "width: 50px; height: 40px; background-color: rgb(0, 128, 0)",
        );
        doc.resolve_layout(VW, VH);
        let p = paint(&mut doc, 1.0);
        assert_eq!(count_any_green(&p, WINDOW), 0, "<{tag}>'s HTML child");
    }
}

/// A `0x0` render surface with no frame paints its children as any `0x0` box
/// does (its arm paints a frameless surface as an ordinary element).
#[test]
fn a_zero_by_zero_render_surface_without_a_frame_paints_its_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let pad = el(&mut doc, body, "padding: 20px 0 0 30px");
    let surf = el(&mut doc, pad, "width: 0; height: 0");
    doc.set_attribute(surf, "data-render-surface", "987654");
    el(
        &mut doc,
        surf,
        "width: 50px; height: 40px; background-color: rgb(0, 128, 0)",
    );
    doc.resolve_layout(VW, VH);
    let p = paint(&mut doc, 1.0);
    assert_eq!(count_rgb(&p, (30, 80, 20, 60), GREEN), 50 * 40);
}

/// A `0x0` box that clips holds nothing visible, so its subtree is not
/// walked: 500 rows under it cost the box's own visit, not 500.
#[test]
fn a_zero_by_zero_clipping_box_does_not_walk_its_subtree() {
    use rinch_dom::perf::Counter;
    for style in [
        "overflow: hidden",
        "overflow: clip",
        "overflow: hidden; z-index: 2; position: relative",
    ] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let bx = el(&mut doc, body, &format!("width: 0; height: 0; {style}"));
        for _ in 0..500 {
            el(
                &mut doc,
                bx,
                "height: 4px; background-color: rgb(0, 128, 0)",
            );
        }
        doc.resolve_layout(VW, VH);
        doc.tree.perf.reset();
        let p = paint(&mut doc, 1.0);
        assert_eq!(count_any_green(&p, WINDOW), 0, "{style}");
        let visits = doc.tree.perf.get(Counter::PaintNodesVisited);
        assert!(visits <= 4, "{style}: {visits} paint visits");
    }
}

/// …except a stacking context's `position: fixed` descendant, which its clip
/// does not reach (#545): it is still drawn.
#[test]
fn a_zero_by_zero_clipping_stacking_context_still_paints_a_fixed_descendant() {
    let (mut doc, bx, _) = slot_doc("z-index: 2; overflow: hidden");
    el(
        &mut doc,
        bx,
        "position: fixed; left: 100px; top: 150px; width: 20px; height: 10px; \
         background-color: rgb(0, 128, 0)",
    );
    doc.resolve_layout(VW + 1.0, VH);
    let p = paint(&mut doc, 1.0);
    assert_eq!(count_rgb(&p, (100, 120, 150, 160), GREEN), 200);
    assert_eq!(
        count_any_green(&p, WINDOW),
        200,
        "the contained absolute is clipped"
    );
}
