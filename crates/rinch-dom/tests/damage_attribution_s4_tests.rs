//! Which node `compute_damage_attributed` places a change at (#1865 S4, and
//! the #1923 review). A change is placed by the start of the node whose paint
//! draws it, unless a negatively z-indexed descendant is hoisted out of that
//! node into an ancestor's negative layer, painted before the node starts:
//! then it is not placed at all, and the runtime re-renders below.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;

const VP: (f32, f32) = (200.0, 120.0);

/// A page with `x` (styled `x_style`) holding `child` (styled `child_style`,
/// or no child), painted once so the next change has a painted state.
fn page(x_style: &str, child_style: Option<&str>) -> (RinchDocument, NodeId) {
    let mut d = RinchDocument::new();
    d.load_css("html, body { margin: 0 }");
    let body = d.body();
    let x = d.create_element("div");
    d.set_attribute(x, "style", x_style);
    d.append_child(body, x);
    if let Some(cs) = child_style {
        let c = d.create_element("div");
        d.set_attribute(c, "style", cs);
        d.append_child(x, c);
    }
    d.resolve_layout(VP.0, VP.1);
    let mut p = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&d.tree, &mut p, 1.0, VP, &mut d.font_cx, &mut cx);
    d.tree.consume_paint_dirty();
    (d, x)
}

/// Restyle `x` and attribute the frame's damage: the node `x`'s change was
/// placed at.
fn restyle(d: &mut RinchDocument, x: NodeId, style: &str) -> Option<Option<u64>> {
    d.set_attribute(x, "style", style);
    d.resolve_layout(VP.0, VP.1);
    let (_, by_node) =
        rinch_dom::paint::compute_damage_attributed(&d.tree, 1.0, VP.0 as f64, VP.1 as f64);
    by_node
        .iter()
        .find(|n| n.node.is_none() || n.node == Some(x.0))
        .map(|n| n.node)
}

const X: &str = "position: absolute; left: 100px; top: 60px; width: 10px; height: 10px";
const NEG: &str = "position: relative; z-index: -1; width: 20px; height: 20px; \
                   background: rgb(40, 200, 90)";

/// The #1923 review's defect: `x` is no stacking context, so its negative-z
/// child is hoisted into the page's negative layer. Clipping `x` changes the
/// child's pixels, which paint before `x` starts: not placed.
#[test]
fn a_change_reaching_a_hoisted_negative_z_child_is_not_placed() {
    let (mut d, x) = page(X, Some(NEG));
    let placed = restyle(&mut d, x, &format!("{X}; overflow: hidden"));
    assert_eq!(placed, Some(None), "placed at {placed:?}");
}

/// A stacking context holds its negative layer: the child paints after `x`
/// starts, so the change is placed at `x`.
#[test]
fn a_stacking_context_holds_its_negative_z_child() {
    let x_ctx = format!("{X}; z-index: 0");
    let (mut d, x) = page(&x_ctx, Some(NEG));
    let placed = restyle(&mut d, x, &format!("{x_ctx}; overflow: hidden"));
    assert_eq!(placed, Some(Some(x.0)));
}

/// No negative-z descendant: placed at `x`.
#[test]
fn a_plain_box_is_placed_at_itself() {
    let (mut d, x) = page(
        X,
        Some("width: 20px; height: 20px; background: rgb(0, 0, 0)"),
    );
    let placed = restyle(&mut d, x, &format!("{X}; background: rgb(250, 0, 0)"));
    assert_eq!(placed, Some(Some(x.0)));
}

/// The walk for a hoisted descendant is bounded: past its budget it answers
/// "not placed" (which only re-renders below) instead of walking a big
/// panel's whole subtree every frame. `split_hoist_walk_nodes` counts it.
#[test]
fn the_hoist_walk_is_bounded() {
    let (mut d, x) = page(X, None);
    for _ in 0..1000 {
        let c = d.create_element("div");
        d.set_attribute(c, "style", "width: 1px; height: 1px");
        d.append_child(x, c);
    }
    d.resolve_layout(VP.0, VP.1);
    let mut p = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&d.tree, &mut p, 1.0, VP, &mut d.font_cx, &mut cx);
    d.tree.consume_paint_dirty();
    let before = d.tree.perf.get(Counter::SplitHoistWalkNodes);
    let placed = restyle(&mut d, x, &format!("{X}; background: rgb(250, 0, 0)"));
    let walked = d.tree.perf.get(Counter::SplitHoistWalkNodes) - before;
    assert_eq!(placed, Some(None), "past the budget: not placed");
    assert!(walked <= 257, "walked {walked} nodes");
    assert!(walked > 0, "the walk was counted");
}
