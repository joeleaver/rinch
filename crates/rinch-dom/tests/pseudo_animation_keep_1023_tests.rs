//! A `::before` / `::after` keeps its box — and its animation and
//! transitions — across a cascade of its originator (#1023).
//!
//! `resolve_styles_recursive` used to free every generated box of an element
//! and mint it again on each cascade of that element, whatever changed. Since
//! #1004 a generated box carries its pseudo cascade, so its `@keyframes`
//! animation starts — and every class toggle, hover or inherited change on the
//! originator restarted it from t=0 on a new node, and a `transition` declared
//! on the pseudo-element never ran (the box was always new, so there was no
//! before-change style).
//!
//! Chrome 155: `.w::before { content: "x"; display: inline-block; animation:
//! k 10s linear infinite }` with `.w.on { outline: 1px solid red }` — toggling
//! `.on` leaves the `::before` animation's `currentTime` where it was. The
//! animation restarts when `animation-name` changes, when the pseudo stops
//! being rendered (`content: none`, `display: none`) and comes back, or when
//! the originator leaves the document and is inserted again; a `transition` on the
//! `::before` runs.
//!
//! Every clock is backdated by 400ms before the change, so "kept" and
//! "restarted" are 400ms apart rather than a few microseconds.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::transition::TransitionProperty;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

const CSS: &str = "\
    @keyframes k { from { background-color: rgb(255, 0, 0); } to { background-color: rgb(0, 0, 255); } } \
    @keyframes k2 { from { background-color: rgb(0, 255, 0); } to { background-color: rgb(0, 0, 255); } } \
    .w { font-size: 16px; line-height: 20px; color: rgb(10, 20, 30); } \
    .w::before { content: \"x\"; display: inline-block; width: 13px; \
                 animation: k 10s linear infinite; } \
    .w::after { content: \"y\"; display: inline-block; width: 17px; } \
    .w.on { outline: 1px solid red; } \
    .p.dark .w { color: rgb(200, 100, 50); } \
    .w.renamed::before { animation-name: k2; } \
    .w.gone::before { content: none; } \
    .w.attr::after { content: attr(data-label); } \
    .w.empty::after { content: ''; } \
    .w.inl::after { content: attr(data-label); display: inline; } \
    .t::before { content: \"t\"; display: inline-block; color: rgb(255, 0, 0); \
                 transition: color 1s linear; } \
    .t.on::before { color: rgb(0, 0, 255); }";

/// `body > div.p > div.w`, laid out once.
fn doc_with(class: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let p = doc.create_element("div");
    doc.set_attribute(p, "class", "p");
    doc.append_child(body, p);
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", class);
    let mid = doc.create_text("mid");
    doc.append_child(w, mid);
    doc.append_child(p, w);
    doc.resolve_layout(VW, VH);
    (doc, p, w)
}

/// The generated box of the given kind (`after == false` for `::before`).
fn generated(doc: &RinchDocument, owner: NodeId, after: bool) -> Option<u64> {
    let children = &doc.tree.get(owner.0).unwrap().children;
    let pick = if after {
        children.last()
    } else {
        children.first()
    };
    pick.copied()
        .filter(|&c| doc.tree.get(c).unwrap().is_pseudo_element)
}

fn before(doc: &RinchDocument, owner: NodeId) -> u64 {
    generated(doc, owner, false).expect("the element generates a ::before")
}

fn start_time(doc: &RinchDocument, node: u64) -> f64 {
    doc.tree
        .active_animations
        .get(&node)
        .expect("the ::before runs an animation")[0]
        .start_time_ms
}

fn backdate(doc: &mut RinchDocument, node: u64, ms: f64) -> f64 {
    let anim = &mut doc
        .tree
        .active_animations
        .get_mut(&node)
        .expect("the ::before runs an animation")[0];
    anim.start_time_ms -= ms;
    anim.start_time_ms
}

fn anim_name(doc: &RinchDocument, node: u64) -> String {
    doc.tree.active_animations.get(&node).expect("animation")[0]
        .name
        .clone()
}

fn text_of(doc: &RinchDocument, node: u64) -> String {
    doc.tree
        .get(node)
        .unwrap()
        .children
        .iter()
        .filter_map(|&c| doc.tree.get(c).unwrap().text_content())
        .collect()
}

#[test]
fn a_class_change_on_the_originator_keeps_the_before_and_its_clock() {
    let (mut doc, _p, w) = doc_with("w");
    let b = before(&doc, w);
    let started = backdate(&mut doc, b, 400.0);

    doc.set_attribute(w, "class", "w on");
    doc.resolve_layout(VW, VH);

    assert_eq!(before(&doc, w), b, "the ::before is the same node");
    assert_eq!(
        start_time(&doc, b),
        started,
        "the animation keeps its clock across an unrelated class change"
    );
    assert_eq!(doc.tree.active_animations.len(), 1, "no stray animation");
}

#[test]
fn an_inherited_change_above_keeps_the_clock_and_reaches_the_before() {
    let (mut doc, p, w) = doc_with("w");
    let b = before(&doc, w);
    let started = backdate(&mut doc, b, 400.0);

    doc.set_attribute(p, "class", "p dark");
    doc.resolve_layout(VW, VH);

    assert_eq!(before(&doc, w), b, "the ::before is the same node");
    assert_eq!(start_time(&doc, b), started, "the clock is kept");
    let c = doc.tree.get(b).unwrap().computed_style.color.unwrap();
    let [r, g, bl, _] = c.to_rgba8().to_u8_array();
    assert_eq!(
        (r, g, bl),
        (200, 100, 50),
        "the kept ::before inherits the new colour"
    );
}

#[test]
fn a_new_animation_name_restarts_the_animation() {
    let (mut doc, _p, w) = doc_with("w");
    let b = before(&doc, w);
    let started = backdate(&mut doc, b, 400.0);

    doc.set_attribute(w, "class", "w renamed");
    doc.resolve_layout(VW, VH);

    let b2 = before(&doc, w);
    assert_eq!(anim_name(&doc, b2), "k2");
    assert!(
        start_time(&doc, b2) > started + 300.0,
        "a renamed animation starts afresh"
    );
}

#[test]
fn a_pseudo_that_stops_and_starts_being_generated_restarts() {
    let (mut doc, _p, w) = doc_with("w");
    let b = before(&doc, w);
    let started = backdate(&mut doc, b, 400.0);

    doc.set_attribute(w, "class", "w gone");
    doc.resolve_layout(VW, VH);
    let pseudos = doc
        .tree
        .get(w.0)
        .unwrap()
        .children
        .iter()
        .filter(|&&c| doc.tree.get(c).unwrap().is_pseudo_element)
        .count();
    assert_eq!(
        pseudos, 1,
        "content: none generates no ::before, the ::after stays"
    );
    assert!(
        !doc.tree.active_animations.contains_key(&b) || doc.tree.get(b).is_none(),
        "the old box's animation is gone"
    );

    doc.set_attribute(w, "class", "w");
    doc.resolve_layout(VW, VH);
    let b2 = before(&doc, w);
    assert!(
        start_time(&doc, b2) > started + 300.0,
        "a ::before generated again runs a new animation from t=0"
    );
}

#[test]
fn a_transition_declared_on_a_before_runs() {
    let (mut doc, _p, w) = doc_with("t");
    let b = before(&doc, w);
    assert!(doc.tree.active_transitions.is_empty());

    doc.set_attribute(w, "class", "t on");
    doc.resolve_layout(VW, VH);

    assert_eq!(before(&doc, w), b, "the ::before is the same node");
    let running = doc
        .tree
        .active_transitions
        .get(&b)
        .is_some_and(|m| m.contains_key(&TransitionProperty::Color));
    assert!(running, "the ::before's colour transitions");
}

#[test]
fn a_kept_after_takes_new_content_and_stays_last() {
    let (mut doc, _p, w) = doc_with("w attr");
    doc.set_attribute(w, "data-label", "one");
    doc.resolve_layout(VW, VH);
    let a = generated(&doc, w, true).expect("an ::after");
    assert_eq!(text_of(&doc, a), "one");

    doc.set_attribute(w, "data-label", "two");
    doc.resolve_layout(VW, VH);
    let a2 = generated(&doc, w, true).expect("an ::after");
    assert_eq!(a2, a, "the ::after is the same node");
    assert_eq!(text_of(&doc, a), "two", "it shows the new content");
    // ...and lays it out: the inline-block's own inline layout holds the new
    // text node, not a freed one.
    let t = doc.tree.get(a).unwrap().children[0];
    assert_eq!(
        doc.tree.get(t).unwrap().ifc_root,
        Some(a),
        "the new text is in the box's IFC"
    );
    let layout = doc
        .tree
        .get(a)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("the box lays its text out");
    assert!(
        layout.text_ranges.iter().any(|r| r.node_id == t),
        "its layout names the new text node"
    );
    let children = &doc.tree.get(w.0).unwrap().children;
    assert_eq!(
        children.len(),
        3,
        "::before, the text, ::after: {children:?}"
    );
    assert_eq!(*children.last().unwrap(), a);
}

#[test]
fn a_kept_after_gains_and_loses_its_text_child() {
    let (mut doc, _p, w) = doc_with("w empty");
    let a = generated(&doc, w, true).expect("an ::after");
    assert!(doc.tree.get(a).unwrap().children.is_empty(), "content: ''");

    doc.set_attribute(w, "class", "w");
    doc.resolve_layout(VW, VH);
    assert_eq!(generated(&doc, w, true), Some(a));
    assert_eq!(text_of(&doc, a), "y");

    doc.set_attribute(w, "class", "w empty");
    doc.resolve_layout(VW, VH);
    assert_eq!(generated(&doc, w, true), Some(a));
    assert!(doc.tree.get(a).unwrap().children.is_empty());
}

/// An inline `::after` is laid out in its originator's inline layout, which
/// must name the new text node once the content changes.
#[test]
fn a_kept_inline_after_takes_new_content_into_the_originators_layout() {
    let (mut doc, _p, w) = doc_with("w inl");
    doc.set_attribute(w, "data-label", "one");
    doc.resolve_layout(VW, VH);
    let a = generated(&doc, w, true).expect("an ::after");

    doc.set_attribute(w, "data-label", "two");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        generated(&doc, w, true),
        Some(a),
        "the ::after is the same node"
    );
    let t = doc.tree.get(a).unwrap().children[0];
    assert_eq!(text_of(&doc, a), "two");
    let root = doc
        .tree
        .get(t)
        .unwrap()
        .ifc_root
        .expect("the text is laid out");
    let layout = doc
        .tree
        .get(root)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("inline layout");
    assert!(
        layout.text_content.ends_with("two"),
        "got {:?}",
        layout.text_content
    );
    assert!(
        layout.text_ranges.iter().any(|r| r.node_id == t),
        "its layout names the new text node"
    );
}

/// A list marker is still generated afresh, and one that appears on a
/// restyle (an `<li>` back to `display: list-item`) is a structural change
/// the item's inline layout has to see.
#[test]
fn a_list_marker_that_appears_on_a_restyle_is_laid_out() {
    let mut doc = RinchDocument::new();
    doc.load_css("li { font-size: 16px; line-height: 20px } .nomark { display: block }");
    let body = doc.body();
    let ul = doc.create_element("ul");
    doc.append_child(body, ul);
    let li = doc.create_element("li");
    doc.set_attribute(li, "class", "nomark");
    let t = doc.create_text("item");
    doc.append_child(li, t);
    doc.append_child(ul, li);
    doc.resolve_layout(VW, VH);
    let text = |doc: &RinchDocument| {
        doc.tree
            .get(li.0)
            .unwrap()
            .text_layout
            .as_ref()
            .map(|l| l.text_content.clone())
            .unwrap_or_default()
    };
    assert_eq!(text(&doc), "item", "no marker while display: block");

    doc.set_attribute(li, "class", "");
    doc.resolve_layout(VW, VH);
    assert!(
        text(&doc).starts_with('\u{2022}') && text(&doc).ends_with("item"),
        "the marker is laid out: {:?}",
        text(&doc)
    );
}

/// The same, where nothing about the `<li>`'s box changes: the marker comes
/// back with an inherited `list-style-type` from the list.
#[test]
fn a_list_marker_that_comes_back_with_its_list_style_is_laid_out() {
    let mut doc = RinchDocument::new();
    doc.load_css("li { font-size: 16px; line-height: 20px } .none { list-style-type: none }");
    let body = doc.body();
    let ul = doc.create_element("ul");
    doc.set_attribute(ul, "class", "none");
    doc.append_child(body, ul);
    let li = doc.create_element("li");
    let t = doc.create_text("item");
    doc.append_child(li, t);
    doc.append_child(ul, li);
    doc.resolve_layout(VW, VH);
    let text = |doc: &RinchDocument| {
        doc.tree
            .get(li.0)
            .unwrap()
            .text_layout
            .as_ref()
            .map(|l| l.text_content.clone())
            .unwrap_or_default()
    };
    assert_eq!(text(&doc), "item", "no marker under list-style-type: none");

    doc.set_attribute(ul, "class", "");
    doc.resolve_layout(VW, VH);
    assert!(
        text(&doc).starts_with('\u{2022}') && text(&doc).ends_with("item"),
        "the marker is laid out: {:?}",
        text(&doc)
    );
}

/// A theme change re-cascades the whole document (`recompute_all_styles_full`)
/// and keeps every running animation's clock (#762); a `::before`'s is kept
/// with its box.
#[test]
fn a_theme_restyle_keeps_the_before_and_its_clock() {
    let (mut doc, _p, w) = doc_with("w");
    let b = before(&doc, w);
    let started = backdate(&mut doc, b, 400.0);

    doc.recompute_all_styles_full();
    doc.resolve_layout(VW + 10.0, VH);

    assert_eq!(before(&doc, w), b, "the ::before is the same node");
    assert_eq!(start_time(&doc, b), started, "the clock is kept");
}

/// So does a change of the root font size, which drops every cached style.
#[test]
fn a_root_font_size_change_keeps_the_before_and_its_clock() {
    let (mut doc, _p, w) = doc_with("w");
    let b = before(&doc, w);
    let started = backdate(&mut doc, b, 400.0);

    doc.tree.perf.reset();
    let html = NodeId(doc.tree.html_id);
    doc.set_style(html, "font-size", "20px");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        doc.tree
            .perf
            .get(rinch_dom::perf::Counter::FullRestyleRootFontSize),
        1,
        "positive control: the root font size changed"
    );

    assert_eq!(before(&doc, w), b, "the ::before is the same node");
    assert_eq!(start_time(&doc, b), started, "the clock is kept");
}

fn pseudo_children(doc: &RinchDocument, n: NodeId) -> Vec<u64> {
    doc.tree
        .get(n.0)
        .unwrap()
        .children
        .iter()
        .copied()
        .filter(|&c| doc.tree.get(c).unwrap().is_pseudo_element)
        .collect()
}

/// A kept box whose `::before` rule leaves with the theme sheet is removed.
/// The element has no `::before` rule any more, so only the box it already
/// has makes the cascade ask about the pseudo (review of #1643, e1).
#[test]
fn a_before_whose_rule_leaves_with_the_theme_sheet_is_removed() {
    let mut doc = RinchDocument::new();
    doc.set_theme_css(".w::before { content: 'T'; }");
    let body = doc.body();
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", "w");
    let t = doc.create_text("mid");
    doc.append_child(w, t);
    doc.append_child(body, w);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        pseudo_children(&doc, w).len(),
        1,
        "positive control: the theme ::before"
    );
    doc.set_theme_css(".other { color: red; }");
    doc.recompute_all_styles_full();
    doc.resolve_layout(VW + 10.0, VH);
    assert_eq!(
        pseudo_children(&doc, w).len(),
        0,
        "the ::before left with its rule"
    );
}

/// A kept `z-index` box becomes a stacking context when its originator
/// becomes a flex container (it is a flex item then), and stops being one
/// when the originator stops (review of #1643, e7).
#[test]
fn a_kept_before_becomes_and_stops_being_a_flex_item() {
    let mut doc = RinchDocument::new();
    doc.load_css(".w::before { content: 'z'; z-index: 3; } .w.fx { display: flex; }");
    let body = doc.body();
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", "w");
    let t = doc.create_text("mid");
    doc.append_child(w, t);
    doc.append_child(body, w);
    doc.resolve_layout(VW, VH);
    let b = pseudo_children(&doc, w)[0];
    assert!(!doc.tree.get(b).unwrap().creates_stacking_context());
    doc.set_attribute(w, "class", "w fx");
    doc.resolve_layout(VW, VH);
    assert_eq!(pseudo_children(&doc, w), vec![b], "kept");
    assert!(
        doc.tree.get(b).unwrap().creates_stacking_context(),
        "a z-indexed flex item"
    );
    doc.set_attribute(w, "class", "w");
    doc.resolve_layout(VW, VH);
    assert!(
        !doc.tree.get(b).unwrap().creates_stacking_context(),
        "no longer a flex item"
    );
}
