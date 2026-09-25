//! A `display: contents` element is neither a stacking context nor a positioned
//! layer, and does not clip — issue #1038.
//!
//! css-display-3 §2.5: a `display: contents` element generates no box. Every
//! property that makes a *box* a stacking context (`z-index` on a positioned
//! box, `opacity < 1`, a non-`none` `transform`, `position: fixed | sticky`),
//! that puts it in CSS 2.1 Appendix E step 8 (positioned, `z-index: auto`), or
//! that clips it (`overflow`) therefore does nothing on one. #994 fixed the
//! containing-block half (`establishes_abs_containing_block`); this file pins
//! the rest. `Node::creates_stacking_context` and `stacking::is_positioned_z_auto`
//! used to read `position`, `z-index`, `opacity` and `transform` off such an
//! element as if it had a box, and `Node::clips_overflow` read its `overflow`:
//!
//! * the collector does not descend into a stacking context, and the wrapper's
//!   own entry carried the clipper above it in its chain, pushed around its
//!   whole subtree — so an absolute or fixed box inside was clipped by a
//!   clipper CSS says it escapes (paint and hit testing alike);
//! * a positioned wrapper was hoisted to step 8 (or, with a negative `z-index`,
//!   step 3), so its in-flow children painted out of tree order;
//! * a clipping wrapper's clip is its `0x0` layout rect, so a descendant that
//!   reached it through the collector's chain (paint) or hit testing's
//!   `check_children` gate was clipped away entirely.
//!
//! Every expected answer is **measured in Chrome 153** (`--headless=new`,
//! `CSS1Compat`, `body { margin: 0 }`, a 400x400 window, both
//! `elementFromPoint` and a screenshot pixel), not derived:
//!
//! | markup | Chrome 153 at the probe |
//! |---|---|
//! | `clip(60,40 100x100; overflow: hidden) > W > abs(10,20 300x250; red)`, W = `contents` + each of `position: relative; z-index: 1` / `transform: translateX(5px)` / `opacity: 0.5` / `position: sticky` / `position: fixed` | (200,200): the abs, `rgb(255,0,0)` — full strength under `opacity: 0.5` too |
//! | the same with W = `contents; transform: translateX(5px)` and a `position: fixed` child at 10,20 | (200,200): the fixed box, red |
//! | `W(contents; position: relative) > t(h 50; red)`, then `s(h 50; margin-top: -25px; blue)` | (10,35): `s`, blue |
//! | the same with W `z-index: 5`, `t` `relative; z-index: 1`, `s` `relative; z-index: 2` | (10,35): `s`, blue |
//! | `s(h 50; blue)`, then `W(contents; position: relative; z-index: -1) > t(h 50; margin-top: -25px; red)` | (10,35): `t`, red |
//! | `div(100x100) > W(contents; overflow: hidden [; opacity: 0.5]) > t(relative; 300x60; red)` | (250,30): `t`, red |
//!
//! No probe sits where the broken and fixed answers agree: every clipped probe
//! is outside the clipper and inside the absolute, and every order probe is in
//! the overlap of two boxes of different colours. Each row has a control with a
//! **boxed** wrapper carrying the same style, which the fix must leave alone.

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::stacking::{PaintOrder, paints_at_stacking_root, stacking_paint_order};

const RED: &str = "background-color: rgb(255, 0, 0)";
const BLUE: &str = "background-color: rgb(0, 0, 255)";
const CLIP: &str = "margin: 40px 0 0 60px; width: 100px; height: 100px; overflow: hidden";
const ABS: &str = "position: absolute; left: 10px; top: 20px; width: 300px; height: 250px";

/// The six wrapper styles of the issue's table. Each makes a *box* a stacking
/// context or a positioned layer, and each is inert on `display: contents`.
const WRAPPERS: &[&str] = &[
    "position: relative",
    "position: relative; z-index: 1",
    "transform: translateX(5px)",
    "opacity: 0.5",
    "position: sticky",
    "position: fixed",
];

fn div(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

fn new_doc() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    (doc, body)
}

/// body > clip > wrapper(`wrapper`) > target(`target`, red).
fn clip_fixture(wrapper: &str, target: &str) -> (RinchDocument, NodeId, NodeId) {
    let (mut doc, body) = new_doc();
    let clip = div(&mut doc, body, CLIP);
    let w = div(&mut doc, clip, wrapper);
    let t = div(&mut doc, w, &format!("{target}; {RED}"));
    doc.resolve_layout(800.0, 600.0);
    (doc, w, t)
}

fn body_order(doc: &RinchDocument) -> PaintOrder {
    stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0)
}

fn chain_of(order: &PaintOrder, node: NodeId) -> Option<Vec<(f64, f64, f64, f64)>> {
    let entry = order.iter().find(|e| e.node_id == node.0)?;
    Some(
        order
            .clips_for(entry)
            .iter()
            .map(|c| (c.rect.x0, c.rect.y0, c.rect.x1, c.rect.y1))
            .collect(),
    )
}

// ── The predicates ───────────────────────────────────────────────────────────

#[test]
fn a_contents_element_is_never_a_stacking_context_or_a_positioned_layer() {
    for style in WRAPPERS {
        let (doc, w, _) = clip_fixture(&format!("display: contents; {style}"), ABS);
        let n = doc.tree.get(w.0).unwrap();
        assert!(
            !n.creates_stacking_context(),
            "`display: contents; {style}` generates no box to stack"
        );
        assert!(
            !paints_at_stacking_root(n),
            "`display: contents; {style}` is not hoisted to any sequence"
        );
    }
}

/// The control: every one of those styles does make a **boxed** wrapper a
/// stacking context or a positioned layer. Without it, a predicate that
/// answered `false` everywhere would pass the test above.
#[test]
fn control_the_same_styles_on_a_boxed_wrapper_still_stack() {
    for style in WRAPPERS {
        let (doc, w, _) = clip_fixture(style, ABS);
        assert!(
            paints_at_stacking_root(doc.tree.get(w.0).unwrap()),
            "a block wrapper with `{style}` is hoisted"
        );
    }
    let (doc, w, _) = clip_fixture("opacity: 0.5", ABS);
    assert!(doc.tree.get(w.0).unwrap().creates_stacking_context());
}

#[test]
fn a_contents_element_does_not_clip() {
    let (doc, w, _) = clip_fixture("display: contents; overflow: hidden", "height: 10px");
    assert!(!doc.tree.get(w.0).unwrap().clips_overflow());
    let (doc, w, _) = clip_fixture("overflow: hidden", "height: 10px");
    assert!(
        doc.tree.get(w.0).unwrap().clips_overflow(),
        "control: a boxed wrapper does"
    );
}

// ── The clip chain ───────────────────────────────────────────────────────────

/// The absolute is an entry of the **body's** sequence, with an empty chain:
/// its containing block is the initial one (#994), so the static clipper above
/// the wrapper is not in its containing-block chain. Before the fix, under
/// every wrapper but the plain `relative` one, it was not an entry of the body
/// at all — it was owned by the wrapper's own sequence, and the wrapper's
/// entry carried the clipper.
#[test]
fn an_absolute_under_any_contents_wrapper_escapes_the_clipper_above_it() {
    for style in WRAPPERS {
        let (doc, _w, abs) = clip_fixture(&format!("display: contents; {style}"), ABS);
        assert_eq!(
            chain_of(&body_order(&doc), abs),
            Some(vec![]),
            "`display: contents; {style}`: the absolute escapes the clipper"
        );
    }
}

/// A fixed child under a `contents; transform` wrapper: the transform does not
/// apply, so the viewport is its containing block and nothing clips it.
#[test]
fn a_fixed_box_under_a_transformed_contents_wrapper_escapes_the_clipper() {
    let (doc, _w, fixed) = clip_fixture(
        "display: contents; transform: translateX(5px)",
        "position: fixed; left: 10px; top: 20px; width: 300px; height: 250px",
    );
    assert_eq!(chain_of(&body_order(&doc), fixed), Some(vec![]));
}

/// The control: a boxed `relative` wrapper *is* the containing block and sits
/// inside the clipper, so the clipper is in the chain.
#[test]
fn control_an_absolute_under_a_boxed_relative_wrapper_is_clipped() {
    let (doc, _w, abs) = clip_fixture("position: relative", ABS);
    assert_eq!(
        chain_of(&body_order(&doc), abs),
        Some(vec![(60.0, 40.0, 160.0, 140.0)])
    );
}

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    const NOTHING: [u8; 4] = [0, 0, 0, 0];
    const FULL_RED: [u8; 4] = [255, 0, 0, 255];
    const FULL_BLUE: [u8; 4] = [0, 0, 255, 255];

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

    /// The issue's table in pixels. The clipper is x in [60, 160), y in
    /// [40, 140); the absolute covers x in [10, 310), y in [20, 270).
    /// (200, 200) is outside the first and inside the second.
    #[test]
    fn the_absolute_paints_outside_the_clipper_under_every_contents_wrapper() {
        for style in WRAPPERS {
            let (mut doc, ..) = clip_fixture(&format!("display: contents; {style}"), ABS);
            let painter = paint(&mut doc);
            assert_eq!(
                pixel_at(&painter, 100, 100),
                FULL_RED,
                "`{style}`: inside both — the positive control, and at full \
                 strength: Chrome 153 applies no `opacity` to a contents element"
            );
            assert_eq!(
                pixel_at(&painter, 200, 200),
                FULL_RED,
                "`display: contents; {style}`: Chrome 153 paints the absolute here"
            );
            assert_eq!(pixel_at(&painter, 350, 300), NOTHING, "outside both");
        }
    }

    #[test]
    fn a_fixed_box_under_a_transformed_contents_wrapper_paints_outside_the_clipper() {
        let (mut doc, ..) = clip_fixture(
            "display: contents; transform: translateX(5px)",
            "position: fixed; left: 10px; top: 20px; width: 300px; height: 250px",
        );
        let painter = paint(&mut doc);
        assert_eq!(pixel_at(&painter, 200, 200), FULL_RED);
        assert_eq!(
            pixel_at(&painter, 312, 200),
            NOTHING,
            "and untranslated: the transform does not apply"
        );
    }

    /// The control for `opacity`: on a boxed wrapper it does apply, so the
    /// full-strength assertion above is not something any wrapper produces.
    #[test]
    fn control_a_boxed_translucent_wrapper_fades_its_children() {
        let (mut doc, ..) = clip_fixture("opacity: 0.5; position: relative", ABS);
        let px = pixel_at(&paint(&mut doc), 100, 100);
        assert!(px[3] > 0 && px[3] < 255, "faded: {px:?}");
    }

    /// body > W(`wrapper`) > t(h 50; `t_style`; red), then s(h 50; margin-top:
    /// -25px; `s_style`; blue). The two overlap on y in [25, 50).
    fn order_fixture(wrapper: &str, t_style: &str, s_style: &str) -> [u8; 4] {
        let (mut doc, body) = new_doc();
        let w = div(&mut doc, body, wrapper);
        div(&mut doc, w, &format!("height: 50px; {t_style}; {RED}"));
        div(
            &mut doc,
            body,
            &format!("height: 50px; margin-top: -25px; {s_style}; {BLUE}"),
        );
        doc.resolve_layout(800.0, 600.0);
        pixel_at(&paint(&mut doc), 10, 35)
    }

    /// A positioned contents wrapper is not hoisted to step 8, so its in-flow
    /// child paints in tree order — under the later sibling.
    #[test]
    fn a_positioned_contents_wrappers_in_flow_child_paints_in_tree_order() {
        assert_eq!(
            order_fixture("display: contents; position: relative", "", ""),
            FULL_BLUE
        );
        assert_eq!(
            order_fixture("position: relative", "", ""),
            FULL_RED,
            "control: a boxed relative wrapper is hoisted to step 8, over `s`"
        );
    }

    /// A contents wrapper's `z-index` scopes nothing: its child's `z-index: 1`
    /// is compared with the sibling's `2` in the body's context.
    #[test]
    fn a_contents_wrappers_z_index_does_not_scope_its_childrens() {
        let (t, s) = (
            "position: relative; z-index: 1",
            "position: relative; z-index: 2",
        );
        assert_eq!(
            order_fixture("display: contents; position: relative; z-index: 5", t, s),
            FULL_BLUE
        );
        assert_eq!(
            order_fixture("position: relative; z-index: 5", t, s),
            FULL_RED,
            "control: a boxed wrapper at z 5 lifts its whole subtree over `s`"
        );
    }

    /// A negative `z-index` does not sink a contents wrapper to step 3.
    #[test]
    fn a_negative_z_index_does_not_sink_a_contents_wrapper() {
        let paint_neg = |wrapper: &str| {
            let (mut doc, body) = new_doc();
            div(&mut doc, body, &format!("height: 50px; {BLUE}"));
            let w = div(&mut doc, body, wrapper);
            div(
                &mut doc,
                w,
                &format!("height: 50px; margin-top: -25px; {RED}"),
            );
            doc.resolve_layout(800.0, 600.0);
            pixel_at(&paint(&mut doc), 10, 35)
        };
        assert_eq!(
            paint_neg("display: contents; position: relative; z-index: -1"),
            FULL_RED
        );
        assert_eq!(
            paint_neg("position: relative; z-index: -1"),
            FULL_BLUE,
            "control: a boxed wrapper at z -1 paints before the in-flow sibling"
        );
    }

    /// `overflow: hidden` on a contents element clips nothing. The child is a
    /// `position: relative` box, so it reaches paint as an entry of the body's
    /// sequence and carries the wrapper in its chain — whose `0x0` rect used to
    /// clip it away entirely. With `opacity: 0.5` as well, the wrapper used to
    /// be a stacking context whose own clip is not in its own sequence, which
    /// hid the defect; this row is the one the stacking half of the fix alone
    /// would have regressed.
    #[test]
    fn overflow_on_a_contents_element_clips_nothing() {
        for extra in ["", "; opacity: 0.5"] {
            let (mut doc, body) = new_doc();
            let outer = div(&mut doc, body, "width: 100px; height: 100px");
            let w = div(
                &mut doc,
                outer,
                &format!("display: contents; overflow: hidden{extra}"),
            );
            div(
                &mut doc,
                w,
                &format!("position: relative; width: 300px; height: 60px; {RED}"),
            );
            doc.resolve_layout(800.0, 600.0);
            let painter = paint(&mut doc);
            assert_eq!(pixel_at(&painter, 10, 30), FULL_RED, "`{extra}`: inside");
            assert_eq!(
                pixel_at(&painter, 250, 30),
                FULL_RED,
                "`{extra}`: past the 100px parent — Chrome 153 paints `t` here"
            );
        }
    }
}
