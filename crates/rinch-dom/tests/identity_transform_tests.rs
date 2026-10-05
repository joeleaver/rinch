//! A `transform` that composes to the identity is still a transform — issue
//! #415.
//!
//! CSS keys a transform's side effects on the **computed value not being
//! `none`**, not on the matrix it composes to: `transform: translateX(0)`
//! creates a stacking context and makes the element the containing block of
//! its absolutely positioned descendants exactly as `translateX(1px)` does.
//! rinch used to ask `TransformValue::is_identity` for both, so a no-op
//! transform did neither. `is_identity` is still what paint and hit testing
//! ask ([`rinch_dom::Node::has_applied_transform`]), so an identity transform
//! creates the context without paying a transform at paint.
//!
//! Every expected answer is **measured in Chrome 153** (`--headless=new`,
//! `CSS1Compat`, `body { margin: 0 }`):
//!
//! | markup | Chrome 153 |
//! |---|---|
//! | `div(relative) > [div(translateX(0)) > div(relative; z-index: 5), div(absolute; z-index: 1)]`, both 100x100 at the origin | `elementFromPoint` answers the `z-index: 1` box: the `z-index: 5` one is trapped in its parent's context |
//! | the same with no transform | the `z-index: 5` box |
//! | `div(rotate(0deg); margin-left: 40px; 200x80) > abs(inset: 0)` | abs `40,y,200,80`; `offsetParent` is the div |
//! | the same with `scale(1)`, and with `translateX(50%) rotate(180deg) translateX(50%) rotate(180deg)` | abs `40,y,200,80` |
//! | the same with `transform: none` | the abs resolves against the initial containing block |
//! | `span(translateX(0)) > abs` | `offsetParent` is `BODY` (#1080 still holds) |
//! | `@keyframes` `translateX(30px)` → `translateX(0)`, `forwards`, finished | still the abs's containing block |
//! | `@keyframes` `translateX(30px)` → `none`, `forwards`, finished | still the containing block — `getComputedStyle` reports `matrix(1, 0, 0, 1, 0, 0)`, the padded list, not `none` |
//! | a `transform` transition back to `none`, finished | no longer the containing block; `getComputedStyle` reports `none` |
//! | `div(translate(0)) > fixed(inset: 0)` | the fixed box fills the div — **not modelled by rinch**, see below |
//!
//! The last row is out of scope here: rinch models no containment of a
//! `position: fixed` box by a transformed ancestor at all, identity or not
//! (`out_of_flow::out_of_flow_kind` answers "the viewport" for every fixed
//! box). That is a separate, older gap and keeps its own issue.
//!
//! No fixture sits on a fixed point: every positive case has a `transform:
//! none` twin answering the other way, and the containing-block fixtures read a
//! size (200x80 against the 800x600 initial containing block) that differs
//! between the two answers.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::stacking::stacking_paint_order;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The identity transforms the issue names, plus #410's cancelling list (its
/// percentage contributions sum to zero for every box size).
const IDENTITIES: &[&str] = &[
    "translateX(0)",
    "translate(0)",
    "rotate(0deg)",
    "scale(1)",
    "translateX(50%) rotate(180deg) translateX(50%) rotate(180deg)",
];

fn el(doc: &mut RinchDocument, tag: &str, parent: NodeId, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

/// `div(transform) > abs(inset: 0)` under the body; returns (div, abs).
fn containing(transform: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = el(
        &mut doc,
        "div",
        body,
        &format!("transform: {transform}; margin-left: 40px; width: 200px; height: 80px"),
    );
    let abs = el(&mut doc, "div", div, "position: absolute; inset: 0");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, div, abs)
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let l = doc.tree.nodes[id.0].layout;
    (l.width, l.height)
}

// ── the predicates ───────────────────────────────────────────────────────────

/// Every identity transform is a stacking context and an absolute containing
/// block — and is still skipped by paint and hit testing, whose question is
/// whether a non-identity matrix applies.
#[test]
fn an_identity_transform_creates_a_context_and_a_containing_block() {
    for tf in IDENTITIES {
        let (doc, div, _) = containing(tf);
        let n = &doc.tree.nodes[div.0];
        assert!(n.creates_stacking_context(), "{tf}: stacking context");
        assert!(
            n.establishes_abs_containing_block(),
            "{tf}: absolute containing block"
        );
        assert!(
            !n.has_applied_transform(),
            "{tf}: paint and hit testing still skip a no-op matrix"
        );
    }
}

/// The negative twin: `transform: none` is neither.
#[test]
fn transform_none_is_neither() {
    let (doc, div, _) = containing("none");
    let n = &doc.tree.nodes[div.0];
    assert!(!n.creates_stacking_context());
    assert!(!n.establishes_abs_containing_block());
}

/// The exclusions already in place hold for an identity transform too: a plain
/// `display: inline` span is not transformable (#1080), and a `display:
/// contents` element generates no box (#994, #1038).
#[test]
fn an_identity_transform_on_an_inline_span_or_a_contents_box_is_neither() {
    for display in ["inline", "contents"] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let p = el(&mut doc, "p", body, "");
        let span = el(
            &mut doc,
            "span",
            p,
            &format!("display: {display}; transform: translateX(0)"),
        );
        let t = doc.create_text("xx");
        doc.append_child(span, t);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let n = &doc.tree.nodes[span.0];
        assert!(!n.creates_stacking_context(), "display: {display}");
        assert!(!n.establishes_abs_containing_block(), "display: {display}");
    }
}

// ── the containing block ─────────────────────────────────────────────────────

/// Chrome: the abs fills the transformed div, 200x80. Without the fix it
/// resolves against the 800x600 initial containing block (#204).
#[test]
fn an_absolute_child_of_an_identity_transform_fills_it() {
    for tf in IDENTITIES {
        let (doc, _, abs) = containing(tf);
        assert_eq!(size(&doc, abs), (200.0, 80.0), "{tf}");
    }
    let (doc, _, abs) = containing("none");
    assert_eq!(
        size(&doc, abs),
        VIEWPORT,
        "with no transform the abs resolves against the initial containing block"
    );
}

/// A transform flipped from `none` to an identity at run time re-syncs the
/// absolute descendant: the cascade's containing-block re-sync sees the change
/// (a viewport change forces the re-layout).
#[test]
fn becoming_an_identity_transform_re_resolves_the_absolute_child() {
    let (mut doc, div, abs) = containing("none");
    assert_eq!(size(&doc, abs), VIEWPORT);
    doc.set_attribute(
        div,
        "style",
        "transform: rotate(0deg); margin-left: 40px; width: 200px; height: 80px",
    );
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(size(&doc, abs), (200.0, 80.0));
    doc.set_attribute(
        div,
        "style",
        "transform: none; margin-left: 40px; width: 200px; height: 80px",
    );
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(size(&doc, abs), VIEWPORT);
}

// ── stacking ─────────────────────────────────────────────────────────────────

/// Chrome: the `z-index: 1` sibling is on top, because the `z-index: 5` box is
/// trapped inside its identity-transformed parent's context. With no transform
/// it is hoisted to the body's sequence and wins.
#[test]
fn a_z_index_inside_an_identity_transform_is_trapped_in_its_context() {
    for (tf, trapped) in [("translateX(0)", true), ("none", false)] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            "div",
            body,
            "position: relative; width: 300px; height: 100px",
        );
        let p = el(
            &mut doc,
            "div",
            c,
            &format!("transform: {tf}; width: 100px; height: 100px"),
        );
        let k = el(
            &mut doc,
            "div",
            p,
            "position: relative; z-index: 5; width: 100px; height: 100px",
        );
        let s = el(
            &mut doc,
            "div",
            c,
            "position: absolute; left: 0; top: 0; z-index: 1; width: 100px; height: 100px",
        );
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let order: Vec<_> = stacking_paint_order(&doc.tree, doc.tree.body_id, 1.0, 0.0, 0.0)
            .iter()
            .map(|e| e.node_id)
            .collect();
        let at = |id: NodeId| order.iter().position(|&n| n == id.0);
        if trapped {
            assert_eq!(at(k), None, "{tf}: z-index 5 stays in its parent's context");
            assert!(
                at(p).unwrap() < at(s).unwrap(),
                "{tf}: the context paints below the z-index 1 sibling: {order:?}"
            );
        } else {
            assert!(
                at(s).unwrap() < at(k).unwrap(),
                "{tf}: the z-index 5 box is hoisted above the z-index 1 one: {order:?}"
            );
        }
    }
}

// ── transitions and animations ───────────────────────────────────────────────

/// A doc whose `.t` div transitions `transform` over 1000ms, laid out once with
/// transitions armed, holding an `inset: 0` abs.
fn transition_doc(from: &str, to: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let css = doc.create_text(&format!(
        ".t {{ width: 200px; height: 80px; transition: transform 1000ms linear; \
         transform: {from}; }} .t.on {{ transform: {to}; }}"
    ));
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "t");
    doc.append_child(body, div);
    el(&mut doc, "div", div, "position: absolute; inset: 0");
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, div)
}

fn transition_start(doc: &mut RinchDocument, div: NodeId) -> f64 {
    doc.set_attribute(div, "class", "t on");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    doc.tree
        .active_transitions
        .get(&div.0)
        .and_then(|t| t.get(&rinch_dom::transition::TransitionProperty::Transform))
        .expect("the class change should start a transform transition")
        .start_time_ms
}

/// Chrome: a transition back to `none` is a containing block while it runs and
/// is not one once it has finished (`getComputedStyle` reports `none`). The
/// completing tick writes the end value straight into the computed style, so
/// it is what has to say `none`.
#[test]
fn a_transition_that_finishes_at_none_stops_being_a_context() {
    let (mut doc, div) = transition_doc("translateX(20px)", "none");
    assert!(doc.tree.nodes[div.0].creates_stacking_context());
    let t0 = transition_start(&mut doc, div);
    rinch_dom::transition::tick_transitions(&mut doc.tree, t0 + 500.0);
    let n = &doc.tree.nodes[div.0];
    assert!(n.creates_stacking_context(), "mid-run: still a transform");
    assert!(n.establishes_abs_containing_block(), "mid-run");
    rinch_dom::transition::tick_transitions(&mut doc.tree, t0 + 1500.0);
    let n = &doc.tree.nodes[div.0];
    assert!(
        n.computed_style.transform.functions.is_empty(),
        "finished: none"
    );
    assert!(!n.creates_stacking_context(), "finished at none");
    assert!(!n.establishes_abs_containing_block(), "finished at none");
    assert!(
        !n.has_applied_transform(),
        "finished at none: nothing to paint"
    );
}

/// The same through the document's own tick, which is what the shells call:
/// the finishing frame stops the div being the containing block, and the abs
/// under it — whose box is baked into its Taffy style — resolves against the
/// initial containing block again with no cascade of its own (Chrome: the abs
/// fills the viewport once the transition has finished).
#[test]
fn the_abs_child_follows_a_transition_that_finishes_at_none() {
    let (mut doc, div) = transition_doc("translateX(20px)", "none");
    let abs = NodeId(doc.tree.nodes[div.0].children[0]);
    transition_start(&mut doc, div);
    assert_eq!(
        size(&doc, abs),
        (200.0, 80.0),
        "mid-run: the div contains it"
    );
    // Back-date the run so the wall-clock tick finishes it.
    for t in doc
        .tree
        .active_transitions
        .get_mut(&div.0)
        .unwrap()
        .values_mut()
    {
        t.start_time_ms -= 10_000.0;
    }
    doc.tick_transitions();
    assert!(!doc.tree.nodes[div.0].establishes_abs_containing_block());
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(size(&doc, abs), VIEWPORT, "finished at none");
}

/// The shell's frame: it drains `dirty_nodes` at every resolve
/// (`take_dirty_nodes`), so on the frame a transition finishes the abs is in
/// no dirty set and **only the tick's own re-sync** reaches it — the abs a
/// direct child, and one level down. (The fixture above leaves the set
/// undrained, where the tick's dirty-node Taffy loop happens to re-bake the
/// abs; this one fails with the re-sync removed.)
#[test]
fn the_tick_re_syncs_the_abs_after_the_shell_drained_the_dirty_set() {
    for deep in [false, true] {
        let (mut doc, div) = transition_doc("translateX(20px)", "none");
        let mut abs = NodeId(doc.tree.nodes[div.0].children[0]);
        if deep {
            let mid = el(&mut doc, "div", div, "width: 50px; height: 30px");
            abs = el(
                &mut doc,
                "div",
                mid,
                "position: absolute; left: 0; top: 0; width: 100%; height: 100%",
            );
        }
        transition_start(&mut doc, div);
        let _ = doc.take_dirty_nodes();
        for t in doc
            .tree
            .active_transitions
            .get_mut(&div.0)
            .unwrap()
            .values_mut()
        {
            t.start_time_ms -= 10_000.0;
        }
        doc.tick_transitions();
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        assert_eq!(size(&doc, abs), VIEWPORT, "finished at none (deep: {deep})");
    }
}

/// The twin: a transition that finishes at `translateX(0)` stays a context.
#[test]
fn a_transition_that_finishes_at_an_identity_stays_a_context() {
    let (mut doc, div) = transition_doc("translateX(20px)", "translateX(0)");
    let t0 = transition_start(&mut doc, div);
    rinch_dom::transition::tick_transitions(&mut doc.tree, t0 + 1500.0);
    let n = &doc.tree.nodes[div.0];
    assert!(!n.computed_style.transform.functions.is_empty());
    assert!(n.creates_stacking_context());
    assert!(n.establishes_abs_containing_block());
}

/// Chrome: an animation filled `forwards` at `translateX(0)` is still the
/// containing block of its abs once it has finished.
#[test]
fn an_animation_filled_at_an_identity_stays_a_context() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let css = doc.create_text(
        "@keyframes k { from { transform: translateX(30px); } to { transform: translateX(0); } } \
         .a { width: 200px; height: 80px; animation: k 1000ms linear forwards; }",
    );
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "a");
    doc.append_child(body, div);
    let abs = el(&mut doc, "div", div, "position: absolute; inset: 0");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let t0 = doc.tree.active_animations[&div.0][0].start_time_ms;
    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 1500.0);
    let n = &doc.tree.nodes[div.0];
    assert!(n.creates_stacking_context());
    assert!(n.establishes_abs_containing_block());
    // A resize forces the re-layout the finished fill needs.
    doc.resolve_layout(VIEWPORT.0 + 1.0, VIEWPORT.1);
    assert_eq!(size(&doc, abs), (200.0, 80.0));
}

/// Chrome: an animation filled `forwards` at a `transform: none` keyframe
/// reports `matrix(1, 0, 0, 1, 0, 0)` — the padded identity list, not `none` —
/// and is still the containing block of its abs. The fill is written through
/// the same path as every frame, which never writes `none`.
#[test]
fn an_animation_filled_at_a_none_keyframe_stays_a_context() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let css = doc.create_text(
        "@keyframes k { from { transform: translateX(30px); } to { transform: none; } } \
         .a { width: 200px; height: 80px; animation: k 1000ms linear forwards; }",
    );
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "a");
    doc.append_child(body, div);
    el(&mut doc, "div", div, "position: absolute; inset: 0");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let t0 = doc.tree.active_animations[&div.0][0].start_time_ms;
    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 1500.0);
    let n = &doc.tree.nodes[div.0];
    assert!(n.creates_stacking_context());
    assert!(n.establishes_abs_containing_block());
}
