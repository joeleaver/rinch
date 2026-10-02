//! A `display: none` element with inline children must not become an IFC
//! root over its own hidden subtree — #509 — and a node whose own display
//! isn't `none` but sits under an ancestor that is must not either, which is
//! what made text under a `display: none` ancestor get a dead Parley
//! `InlineLayout` rebuilt on every layout pass that touched the document,
//! unboundedly, for as long as the subtree existed (#826).
//!
//! `setup_inline_formatting_contexts`' root-candidacy loop already skips
//! `display: contents` the same way (it generates no box either); these
//! tests pin the two new skips the same way `ifc_classifier_tests.rs` pins
//! the rest of that loop's decisions — against the registry the loop fills
//! and the `InlineLayout` the build step actually produces, plus a perf
//! counter for the cost claim.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::perf::Counter;
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let el = doc.create_element(tag);
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) -> NodeId {
    let t = doc.create_text(text);
    doc.append_child(parent, t);
    t
}

fn ifc_root_of(doc: &RinchDocument, node: NodeId) -> Option<usize> {
    doc.tree.get(node.0).unwrap().ifc_root
}

/// Toggle `node`'s width between two values and re-resolve, so the pass
/// cannot early-return on `!layout_dirty` (brief rule #5) without otherwise
/// touching the hidden subtree under test.
fn repaint_pass(doc: &mut RinchDocument, driver: NodeId, wide: bool) {
    doc.set_attribute(
        driver,
        "style",
        if wide {
            "width: 290px"
        } else {
            "width: 280px"
        },
    );
    doc.resolve_layout(VW, VH);
}

/// #509, direct case: a `display: none` block with inline (text) children is
/// never classified as an IFC root over its own subtree — the root-candidacy
/// loop must skip it exactly as it skips `display: contents`.
///
/// Kills: dropping the `DisplayValue::None` skip added next to the
/// `Contents` one in the root-candidacy loop — `hidden` lands in
/// `ifc_root_registry` and `ifc_root_of(&doc, text)` reads
/// `Some(hidden.0)`.
#[test]
fn a_hidden_block_with_inline_children_is_not_an_ifc_root() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let hidden = child_of(&mut doc, body, "div", "display: none");
    let text = text_in(&mut doc, hidden, "hello");
    doc.resolve_layout(VW, VH);

    assert!(
        !doc.tree.ifc_root_registry.contains(&hidden.0),
        "a display:none block must not register as an IFC root over its \
         own subtree (#509)"
    );
    assert_eq!(
        ifc_root_of(&doc, hidden),
        None,
        "the hidden block itself carries no root mark"
    );
    assert_eq!(
        ifc_root_of(&doc, text),
        None,
        "its text child carries no mark from a root that can never paint it"
    );
}

/// #826, the ancestor case: the node that would become an IFC root is not
/// itself `display: none` — only an ancestor several levels up is — and must
/// still be excluded, or its text pays a Parley rebuild on every layout pass
/// that touches the document, growing with the hidden subtree's size, for as
/// long as it exists.
///
/// Shape follows the issue's repro: a `display: none` panel holding rows of
/// `div{display:flex} > span > "text"` — the flex row blockifies its `span`
/// child (every flex item is block-level), so the blockified span, not the
/// row and not the panel, is the actual root candidate, and its own
/// `display` is never `none`.
///
/// Kills: dropping `has_display_none_ancestor`'s call at the `ifc_roots.push`
/// site (or hard-coding it to `false`) — the span registers as a root and
/// `shape_ifc_build` climbs every pass instead of staying flat at the
/// positive control's own non-zero value.
#[test]
fn a_hidden_ancestors_descendant_is_not_an_ifc_root_and_builds_nothing() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    // The repro's own forcing function: an unrelated driver whose width
    // toggle keeps every pass from early-returning on `!layout_dirty`.
    let driver = child_of(&mut doc, body, "div", "width: 280px");

    let panel = child_of(&mut doc, body, "div", "display: none");
    let mut spans = Vec::new();
    for _ in 0..4 {
        let row = child_of(&mut doc, panel, "div", "display: flex");
        let span = child_of(&mut doc, row, "span", "");
        text_in(&mut doc, span, "Select all");
        spans.push(span);
    }

    doc.resolve_layout(VW, VH); // warm-up, matching the issue's methodology

    // Positive control: a *shown* sibling subtree of the same shape proves
    // `shape_ifc_build` fires at all, so a 0 below is the fix, not a probe
    // that never ran (brief rule #6).
    let shown_row = child_of(&mut doc, body, "div", "display: flex");
    let shown_span = child_of(&mut doc, shown_row, "span", "");
    text_in(&mut doc, shown_span, "Select all");
    repaint_pass(&mut doc, driver, true);
    let control = doc.tree.perf.get(Counter::ShapeIfcBuild);
    assert!(
        control > 0,
        "positive control: the shown twin must build at least once, or this \
         counter never fires and a later 0 proves nothing"
    );
    for &span in &spans {
        assert!(
            !doc.tree.ifc_root_registry.contains(&span.0),
            "a descendant of a display:none ancestor must not register as \
             an IFC root (#826)"
        );
        assert_eq!(ifc_root_of(&doc, span), None);
    }

    doc.tree.perf.reset();
    for i in 0..5 {
        repaint_pass(&mut doc, driver, i % 2 == 0);
    }
    assert_eq!(
        doc.tree.perf.get(Counter::ShapeIfcBuild),
        0,
        "the hidden rows' text must never be rebuilt — their IFC root was \
         never even classified"
    );
    for &span in &spans {
        assert!(!doc.tree.ifc_root_registry.contains(&span.0));
    }
}

/// Showing a previously-hidden subtree re-establishes its IFC root and lays
/// it out exactly as a document built already-shown would — the marks are
/// cleared while hidden (above) and re-established when shown, not merely
/// left stale.
#[test]
fn showing_a_previously_hidden_subtree_lays_out_like_a_fresh_one() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = child_of(
        &mut doc,
        body,
        "div",
        "display: none; font-size: 16px; line-height: 20px; width: 200px;",
    );
    let span = child_of(&mut doc, panel, "span", "");
    text_in(&mut doc, span, "Select all");
    doc.resolve_layout(VW, VH);
    assert_eq!(ifc_root_of(&doc, span), None, "hidden: no root yet");

    doc.set_attribute(
        panel,
        "style",
        "display: block; font-size: 16px; line-height: 20px; width: 200px;",
    );
    doc.resolve_layout(VW, VH);
    // `span` is `display: inline` — a flowed inline element owns no box of
    // its own (it carries no Taffy node once an IFC claims it), so compare
    // the IFC root's own box, which Taffy sizes from the line it lays out.
    let shown_layout = doc.tree.get(panel.0).unwrap().layout;

    // A document built already-shown, for comparison.
    let mut fresh = RinchDocument::new();
    let fresh_body = fresh.body();
    let fresh_panel = child_of(
        &mut fresh,
        fresh_body,
        "div",
        "display: block; font-size: 16px; line-height: 20px; width: 200px;",
    );
    let fresh_span = child_of(&mut fresh, fresh_panel, "span", "");
    text_in(&mut fresh, fresh_span, "Select all");
    fresh.resolve_layout(VW, VH);

    assert_eq!(
        ifc_root_of(&fresh, fresh_span),
        Some(fresh_panel.0),
        "sanity: the fresh document's span is flowed by its panel"
    );
    assert_eq!(
        ifc_root_of(&doc, span),
        Some(panel.0),
        "shown: the span becomes an IFC root member again once its ancestor \
         is no longer display:none"
    );
    let fresh_layout = fresh.tree.get(fresh_panel.0).unwrap().layout;
    assert_eq!(
        (shown_layout.width, shown_layout.height),
        (fresh_layout.width, fresh_layout.height),
        "the shown subtree lays out identically to one built already-shown"
    );
    assert!(
        shown_layout.width > 0.0 && shown_layout.height > 0.0,
        "sanity: the comparison above isn't two zeroes agreeing by accident"
    );
}
