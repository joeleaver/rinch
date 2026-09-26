//! `transform` does not apply to a non-atomic `display: inline` element —
//! issue #1080.
//!
//! CSS Transforms 1 §1: a *transformable element* is an element whose layout is
//! governed by the CSS box model and which is a block-level or **atomic**
//! inline-level element (or a replaced one). A plain `<span>` is none of those,
//! so its `transform` does nothing at all: it moves nothing, it establishes no
//! containing block for its absolute descendants, and it creates no stacking
//! context. rinch used to answer "yes" to all three, off
//! `computed_style.transform.is_identity` alone.
//!
//! Every expected number is **measured in Chrome 153** (`--headless`,
//! `CSS1Compat`, `* { box-sizing: border-box; border-width: 0; margin: 0;
//! padding: 0 }`, `font: 16px/20px sans-serif`), not derived:
//!
//! | markup | Chrome 153 |
//! |---|---|
//! | static 200x100 `overflow: auto` scroller, 100x50 div, then `span(translateX(1px)) > ("x", abs(left: 0; top: 300px; 10x10))` | `scrollHeight` 100; the abs's `offsetParent` is `BODY` |
//! | the same, scroller `position: relative` | `scrollHeight` 310 |
//! | the same, span `display: inline-block; translateX(1px)` | `scrollHeight` 360; `offsetParent` is the span |
//! | `div(relative) > "ab" + span(translateX(30px)) > ("xy", inline-block 20x10)` | the inline-block at `x = 33.8` — exactly where it is with no transform |
//! | `div(static) > "ab" + span(translateX(30px)) > abs(left: 0; top: 0; 5x5)` | abs `0,0,5,5`; `offsetParent` `BODY` |
//! | `getComputedStyle(span).transform` | `matrix(1, 0, 0, 1, 30, 0)` — the computed value is kept |
//!
//! So the computed value stays what the author wrote (`get_computed_styles`
//! reports it, as Chrome's `getComputedStyle` does); only its *effect* is gone.
//! The rule is the one [`rinch_dom::Node::clips_overflow`] already applies to
//! `overflow`: a non-atomic inline element is not a box the property applies to.
//!
//! No fixture sits on a fixed point: every transform is a non-zero translate
//! whose displacement would show, and every negative case has a positive twin
//! (an atomic inline, or a positioned span) proving the same markup does
//! respond when the property applies.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::{content_extents, scrollbars};

const VIEWPORT: (f32, f32) = (800.0, 600.0);
const ABS: &str = "position: absolute; left: 0; top: 300px; width: 10px; height: 10px";

fn el(doc: &mut RinchDocument, tag: &str, parent: NodeId, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// The issue's fixture: a 200x100 scroller holding a 100x50 div and then
/// `span(span_style) > ("x", abs)`. Returns the scroller, the span and the abs.
fn scroller(scroller_style: &str, span_style: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let s = el(
        &mut doc,
        "div",
        body,
        &format!(
            "width: 200px; height: 100px; overflow: auto; font-size: 16px; line-height: 20px; {scroller_style}"
        ),
    );
    el(&mut doc, "div", s, "width: 100px; height: 50px");
    let span = el(&mut doc, "span", s, span_style);
    text(&mut doc, span, "x");
    let abs = el(&mut doc, "span", span, ABS);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, s, span, abs)
}

fn vertical_range(doc: &RinchDocument, s: NodeId) -> (f64, Option<f64>) {
    let (_, h) = content_extents(&doc.tree, s.0);
    let bar = scrollbars(&doc.tree, s.0, 1.0).vertical.map(|t| t.max_scroll);
    (h, bar)
}

// ── the predicates ───────────────────────────────────────────────────────────

/// A transformed plain span is no containing block and no stacking context —
/// but its computed `transform` is still what the author wrote.
#[test]
fn a_transformed_inline_span_is_no_containing_block_and_no_stacking_context() {
    let (doc, _, span, _) = scroller("", "transform: translateX(1px)");
    let n = doc.tree.get(span.0).unwrap();
    assert!(
        !n.computed_style.transform.is_identity,
        "the computed value is kept (Chrome: getComputedStyle reports the matrix)"
    );
    assert!(!n.establishes_abs_containing_block());
    assert!(!n.creates_stacking_context());
}

/// Positive twins: the same transform on an `inline-block` is both, and a
/// `position: relative` span is still a containing block (the `position` arm is
/// untouched).
#[test]
fn an_atomic_inline_or_a_positioned_span_still_is_one() {
    let (doc, _, ib, _) = scroller("", "display: inline-block; transform: translateX(1px)");
    let n = doc.tree.get(ib.0).unwrap();
    assert!(n.establishes_abs_containing_block());
    assert!(n.creates_stacking_context());

    let (doc, _, span, _) = scroller("", "position: relative");
    assert!(
        doc.tree
            .get(span.0)
            .unwrap()
            .establishes_abs_containing_block()
    );
}

// ── the scroll range: the phantom bar the issue was filed for ────────────────

/// Chrome 200x100, no bar. Fails at `ab5116d1`: extent 310, a 210px bar.
#[test]
fn a_transformed_span_gives_a_static_scroller_no_range() {
    let (doc, s, _, _) = scroller("", "transform: translateX(1px)");
    let (h, bar) = vertical_range(&doc, s);
    assert!(h <= 100.0, "extent {h}");
    assert_eq!(bar, None);
}

/// Chrome 200x310: a positioned scroller is the box's containing block, so it
/// counts the box whatever the span says.
#[test]
fn a_positioned_scroller_still_counts_the_box() {
    let (doc, s, _, _) = scroller("position: relative", "transform: translateX(1px)");
    let (h, bar) = vertical_range(&doc, s);
    assert!(h >= 310.0, "extent {h}");
    assert!(bar.is_some(), "bar {bar:?}");
}

// Chrome's third row (an `inline-block` with the same transform: 200x360) is
// not pinned here as a scroll range: rinch measures a scroller's range from its
// child boxes and stops at the first one, so a box whose containing block is
// that inline-block is in nobody's range — issue #770, independent of this
// one. `an_atomic_inline_or_a_positioned_span_still_is_one` pins the half of
// that row this issue is about: the inline-block *is* the containing block.

// ── out_of_flow_kind: the #204 ICB placement ─────────────────────────────────

/// `body > div(margin: 100px 0 0 50px; 200x100) > span(translateX(30px)) >
/// abs(inset: 0)`: the span is not the containing block and neither is the
/// static div, so the box fills the initial containing block (800x600) — the
/// shape of Chrome's `abs 0,0,5,5 / offsetParent BODY` row. The
/// `position: relative` twin is the span's own box (not the viewport).
#[test]
fn an_absolute_under_a_transformed_span_resolves_against_the_viewport() {
    let build = |span_style: &str| {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0");
        let outer = el(
            &mut doc,
            "div",
            body,
            "margin: 100px 0 0 50px; width: 200px; height: 100px; font-size: 16px; line-height: 20px",
        );
        let span = el(&mut doc, "span", outer, span_style);
        text(&mut doc, span, "xy");
        let abs = el(&mut doc, "div", span, "position: absolute; inset: 0");
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let l = doc.tree.get(abs.0).unwrap().layout;
        (l.width, l.height)
    };
    assert_eq!(build("transform: translateX(30px)"), (800.0, 600.0));
    assert_ne!(build("position: relative"), (800.0, 600.0));
}

// ── paint: the transform moves nothing ───────────────────────────────────────

/// `div(relative) > "ab" + span(translateX(30px)) > ("xy", inline-block
/// 20x10)`: Chrome puts the inline-block exactly where it is with no
/// transform. rinch's paint position (`compute_absolute_position_and_transform`,
/// which paint, hit testing and the damage walk all share) must agree: the
/// same x as the untransformed twin, and no transform on the chain.
#[test]
fn a_transformed_span_does_not_move_its_inline_block_child() {
    let build = |span_style: &str| {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0");
        let d = el(
            &mut doc,
            "div",
            body,
            "position: relative; height: 60px; font-size: 16px; line-height: 20px",
        );
        text(&mut doc, d, "ab");
        let span = el(&mut doc, "span", d, span_style);
        text(&mut doc, span, "xy");
        let ib = el(
            &mut doc,
            "span",
            span,
            "display: inline-block; width: 20px; height: 10px",
        );
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let (x, y, t) =
            rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, ib.0, 1.0);
        let p = t * peniko::kurbo::Point::new(x, y);
        (p.x, p.y)
    };
    let plain = build("");
    let transformed = build("transform: translateX(30px)");
    assert!(plain.0 > 0.0, "positive control: the child sits after text");
    assert_eq!(transformed, plain);
    // Twin: on an inline-block span the translate does move it.
    let atomic = build("display: inline-block; transform: translateX(30px)");
    assert!(
        (atomic.0 - plain.0 - 30.0).abs() < 0.5,
        "atomic {atomic:?} vs plain {plain:?}"
    );
}

// ── the cascade's containing-block re-sync ───────────────────────────────────

/// An absolute's viewport size is baked into its Taffy style, so a node that
/// starts or stops being its containing block owes it a re-sync. With a
/// `transform` declared all along, a `display` flip between `inline` and
/// `inline-block` is such a change — and the cascade must see it, although the
/// transform itself did not move. Both directions, each against a fresh
/// layout of the final state.
#[test]
fn a_display_flip_under_a_standing_transform_resyncs_the_absolute() {
    const T: &str = "transform: translateX(30px); width: 120px; height: 40px";
    let build = |display: &str| {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0");
        let outer = el(
            &mut doc,
            "div",
            body,
            "margin: 100px 0 0 50px; width: 200px; height: 100px; font-size: 16px; line-height: 20px",
        );
        let span = el(&mut doc, "span", outer, &format!("display: {display}; {T}"));
        text(&mut doc, span, "xy");
        let abs = el(&mut doc, "div", span, "position: absolute; inset: 0");
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        (doc, span, abs)
    };
    let size = |doc: &RinchDocument, n: NodeId| {
        let l = doc.tree.get(n.0).unwrap().layout;
        (l.width, l.height)
    };
    for (from, to) in [("inline", "inline-block"), ("inline-block", "inline")] {
        let (fresh, _, fresh_abs) = build(to);
        let want = size(&fresh, fresh_abs);
        let (mut doc, span, abs) = build(from);
        let before = size(&doc, abs);
        assert_ne!(before, want, "{from} → {to}: the flip must change the box");
        doc.set_attribute(span, "style", &format!("display: {to}; {T}"));
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        assert_eq!(size(&doc, abs), want, "{from} → {to}");
    }
}

// ── damage: where the last paint put it ──────────────────────────────────────

/// The damage walk replays each node's *painted* state (`PaintedState`,
/// written when a paint consumes the region), so it must record no transform
/// for the span either: the inline-block's previous painted rect is its
/// untransformed box, exactly where paint drew it. Kills a `PaintedState` that
/// still records the span's transform (the rect lands 30px right).
#[test]
fn the_painted_state_records_no_transform_for_an_inline_span() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let d = el(
        &mut doc,
        "div",
        body,
        "position: relative; height: 60px; font-size: 16px; line-height: 20px",
    );
    text(&mut doc, d, "ab");
    let span = el(&mut doc, "span", d, "transform: translateX(30px)");
    text(&mut doc, span, "xy");
    let ib = el(
        &mut doc,
        "span",
        span,
        "display: inline-block; width: 20px; height: 10px",
    );
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    doc.tree.consume_paint_dirty();
    assert!(
        doc.tree.get(span.0).unwrap().painted.is_some(),
        "positive control: the span's painted state was recorded"
    );
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, ib.0, 1.0);
    let r = rinch_dom::paint::previous_painted_rect(&doc.tree, ib.0, 1.0).expect("painted");
    assert!(x > 0.0, "positive control: the child sits after text");
    assert_eq!((r.x0, r.y0, r.width(), r.height()), (x, y, 20.0, 10.0));
}
