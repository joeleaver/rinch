//! A `display: contents` element establishes no containing block — issue #994.
//!
//! css-display-3 §2.5: a `display: contents` element generates no box, so it
//! cannot be the containing block of anything, whatever its `position` says —
//! and `transform` does not apply to it at all. `Node::establishes_abs_containing_block`
//! used to key on `position`/`transform` alone, so a
//! `display: contents; position: relative` wrapper answered `true`, and every
//! consumer of that predicate went wrong the same way:
//!
//! * `out_of_flow::out_of_flow_kind` stopped its walk at the wrapper and handed
//!   the box back to Taffy, whose parent after the contents splice is the
//!   wrapper's own parent — so `inset: 0` filled that parent instead of the
//!   initial containing block;
//! * `stacking::Collector::span` set `cb_depth` at the wrapper, so an
//!   `overflow` box between the wrapper and the real containing block clipped
//!   an absolute CSS says escapes it;
//! * the Taffy re-sync in `style_resolution` (a node that starts or stops being
//!   a containing block re-syncs its absolute descendants) did not fire for a
//!   `display` flip between `block` and `contents`.
//!
//! Every expected number is **measured in Chrome 153** (`--headless=new`,
//! `CSS1Compat`, `body { margin: 0 }`), not derived:
//!
//! | markup | Chrome 153 |
//! |---|---|
//! | `div(margin: 100px 0 0 50px; 200x100) > contents+relative > abs(inset: 0)` | abs `0,0,vw,vh` |
//! | `div(margin: 40px 0 0 60px; 100x100; overflow: hidden) > contents+relative > abs(10,20 300x250)` | abs `10,20,300,250`; `elementFromPoint(200,200)` is the abs (outside the clip) |
//! | `div(margin-left: 500px; 50x50; overflow: hidden) > contents+translateX(5px) > abs(left: 430px; top: 7px)` | abs `430,7,100,100` — the transform does nothing |
//!
//! No fixture sits on a fixed point: every parent is off the page origin (so a
//! parent-relative box and an ICB box differ in position as well as size), and
//! every sibling case with a *block* wrapper is asserted alongside to prove the
//! same markup does resolve against the wrapper when it has a box.

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::node::RawNodeId;
use rinch_dom::stacking::{PaintOrder, stacking_paint_order};

fn div(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

fn on_screen(doc: &RinchDocument, node: NodeId) -> (f64, f64) {
    rinch_dom::paint::compute_absolute_position(&doc.tree, node.0, 1.0)
}

fn size(doc: &RinchDocument, node: NodeId) -> (f32, f32) {
    let l = doc.tree.get(node.0).unwrap().layout;
    (l.width, l.height)
}

/// body > outer(offset, 200x100) > wrapper(`wrapper_style`) > abs(inset: 0).
fn issue_fixture(wrapper_style: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let outer = div(
        &mut doc,
        body,
        "margin: 100px 0 0 50px; width: 200px; height: 100px",
    );
    let wrapper = div(&mut doc, outer, wrapper_style);
    let abs = div(&mut doc, wrapper, "position: absolute; inset: 0");
    doc.resolve_layout(800.0, 600.0);
    (doc, outer, wrapper, abs)
}

// ── out_of_flow_kind: the box's own size and place ───────────────────────────

/// The issue as filed.
#[test]
fn a_positioned_contents_wrapper_is_not_the_containing_block() {
    let (doc, outer, _wrapper, abs) = issue_fixture("display: contents; position: relative");
    assert_eq!(
        on_screen(&doc, outer),
        (50.0, 100.0),
        "premise: off the origin"
    );
    assert_eq!(
        size(&doc, abs),
        (800.0, 600.0),
        "Chrome 153: the wrapper generates no box, so the absolute resolves \
         against the initial containing block — not the 200x100 div the \
         contents splice made its Taffy parent"
    );
    assert_eq!(on_screen(&doc, abs), (0.0, 0.0));
}

/// `transform` does not apply to a `display: contents` element, so it does not
/// make one a containing block either.
#[test]
fn a_transformed_contents_wrapper_is_not_the_containing_block() {
    let (doc, _outer, _wrapper, abs) =
        issue_fixture("display: contents; transform: translateX(5px)");
    assert_eq!(size(&doc, abs), (800.0, 600.0));
    assert_eq!(on_screen(&doc, abs), (0.0, 0.0));
}

/// The controls: the same markup with a wrapper that **has** a box resolves
/// against it, and a plain contents wrapper was already right. Without the
/// first, a fixture that always answers "the viewport" would pass above.
#[test]
fn controls_a_boxed_positioned_wrapper_and_an_unpositioned_contents_one() {
    let (doc, _outer, _wrapper, abs) = issue_fixture("position: relative; height: 60px");
    assert_eq!(
        size(&doc, abs),
        (200.0, 60.0),
        "a block wrapper with `position: relative` is the containing block"
    );
    assert_eq!(on_screen(&doc, abs), (50.0, 100.0));

    let (doc, _outer, _wrapper, abs) = issue_fixture("display: contents");
    assert_eq!(size(&doc, abs), (800.0, 600.0));
    assert_eq!(on_screen(&doc, abs), (0.0, 0.0));
}

/// A wrapper that flips between `block` and `contents` starts or stops being a
/// containing block with no change to its `position`. The cascade's re-sync of
/// absolute descendants keys on the predicate's before/after answer, so it has
/// to see the flip — in both directions.
#[test]
fn a_display_flip_on_a_positioned_wrapper_moves_the_absolute() {
    let (mut doc, _outer, wrapper, abs) = issue_fixture("position: relative; height: 60px");
    assert_eq!(size(&doc, abs), (200.0, 60.0), "premise");

    doc.set_attribute(
        wrapper,
        "style",
        "display: contents; position: relative; height: 60px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        size(&doc, abs),
        (800.0, 600.0),
        "block -> contents: the wrapper stopped being a containing block"
    );
    assert_eq!(on_screen(&doc, abs), (0.0, 0.0));

    doc.set_attribute(wrapper, "style", "position: relative; height: 60px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        size(&doc, abs),
        (200.0, 60.0),
        "contents -> block: and started again"
    );
    assert_eq!(on_screen(&doc, abs), (50.0, 100.0));
}

// ── Collector::span: the clip chain ──────────────────────────────────────────

fn body_order(doc: &RinchDocument) -> PaintOrder {
    stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0)
}

fn chain_of(order: &PaintOrder, node: RawNodeId) -> Vec<(f64, f64, f64, f64)> {
    let entry = order
        .iter()
        .find(|e| e.node_id == node)
        .unwrap_or_else(|| panic!("node {node} is not an entry of this sequence: {order:?}"));
    order
        .clips_for(entry)
        .iter()
        .map(|c| (c.rect.x0, c.rect.y0, c.rect.x1, c.rect.y1))
        .collect()
}

/// body(static) > clip(overflow: hidden, 100x100 at 60,40) > wrapper > abs.
fn clip_fixture(wrapper_style: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clip = div(
        &mut doc,
        body,
        "margin: 40px 0 0 60px; width: 100px; height: 100px; overflow: hidden",
    );
    let wrapper = div(&mut doc, clip, wrapper_style);
    let abs = div(
        &mut doc,
        wrapper,
        "position: absolute; left: 10px; top: 20px; width: 300px; height: 250px; \
         background-color: rgb(255, 0, 0)",
    );
    doc.resolve_layout(800.0, 600.0);
    (doc, clip, abs)
}

#[test]
fn an_overflow_box_above_a_positioned_contents_wrapper_does_not_clip_the_absolute() {
    let (doc, _clip, abs) = clip_fixture("display: contents; position: relative");
    assert_eq!(on_screen(&doc, abs), (10.0, 20.0), "Chrome 153: at the ICB");
    assert_eq!(
        chain_of(&body_order(&doc), abs.0),
        Vec::<(f64, f64, f64, f64)>::new(),
        "the containing block is the ICB, so the static `overflow: hidden` box \
         is not in the absolute's containing-block chain and does not clip it"
    );
}

/// The control: a wrapper with a box is the containing block and sits below
/// the clip, so the clip is in the chain. Without it, a collector that drops
/// every chain would pass the test above.
#[test]
fn control_an_overflow_box_above_a_boxed_positioned_wrapper_clips() {
    let (doc, _clip, abs) = clip_fixture("position: relative");
    assert_eq!(
        chain_of(&body_order(&doc), abs.0),
        vec![(60.0, 40.0, 160.0, 140.0)]
    );
}

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    const NOTHING: [u8; 4] = [0, 0, 0, 0];
    const RED: [u8; 4] = [255, 0, 0, 255];

    fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
        let mut painter = TinySkiaPainter::new(400, 400);
        let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (800.0, 600.0),
            &mut doc.font_cx,
            &mut layout_cx,
        );
        painter
    }

    fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * painter.width() + x) * 4) as usize;
        let d = painter.pixels();
        [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
    }

    /// The same escape, in pixels. The clip is x in [60, 160), y in [40, 140);
    /// the absolute covers x in [10, 310), y in [20, 270).
    #[test]
    fn the_absolute_paints_outside_the_overflow_box() {
        let (mut doc, ..) = clip_fixture("display: contents; position: relative");
        let painter = paint(&mut doc);
        assert_eq!(
            pixel_at(&painter, 100, 100),
            RED,
            "inside the clip and the absolute: the positive control"
        );
        assert_eq!(
            pixel_at(&painter, 200, 200),
            RED,
            "outside the clip, inside the absolute — Chrome 153's \
             `elementFromPoint(200, 200)` answers the absolute here"
        );
        assert_eq!(pixel_at(&painter, 350, 300), NOTHING, "outside both");
    }

    /// The off-window cull (`layer_bounds::subtree_is_entirely_outside`, #562)
    /// asks the same question of a **stacking context** it is about to skip: an
    /// absolute inside escapes a clipper only when that clipper sits below its
    /// containing block. Here the stacking context (`opacity: 0.5`, which is no
    /// containing block) and the clipper sit entirely below the 600px window,
    /// and the absolute — which lives at the ICB, on screen — is hoisted into
    /// that context's sequence. Treat the wrapper as a containing block and the
    /// cull reads the absolute as bounded by the off-window clip and skips the
    /// whole context.
    #[test]
    fn an_offscreen_stacking_context_is_not_culled_past_an_escaping_absolute() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0");
        div(&mut doc, body, "height: 700px");
        let sc = div(&mut doc, body, "opacity: 0.5");
        let clip = div(
            &mut doc,
            sc,
            "margin-left: 60px; width: 100px; height: 100px; overflow: hidden",
        );
        let wrapper = div(&mut doc, clip, "display: contents; position: relative");
        let abs = div(
            &mut doc,
            wrapper,
            "position: absolute; left: 10px; top: 20px; width: 300px; height: 250px; \
             background-color: rgb(255, 0, 0)",
        );
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(on_screen(&doc, abs), (10.0, 20.0), "premise: on screen");
        assert!(
            on_screen(&doc, sc).1 >= 600.0 && on_screen(&doc, clip).1 >= 600.0,
            "premise: the stacking context's own box and the clipper are below \
             the window, so only its hoisted absolute can reach the screen"
        );
        let painter = paint(&mut doc);
        let px = pixel_at(&painter, 100, 100);
        assert!(
            px[0] > 0 && px[3] > 0,
            "the absolute escapes the off-window clipper, so the stacking context \
             holding it must not be culled: got {px:?}"
        );
    }
}
