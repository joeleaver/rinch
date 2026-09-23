//! A viewport resize restyles only what the new size reaches.
//!
//! `restyle_for_viewport_change` used to re-cascade every element on every
//! resize. It now restyles everything only when a media query's answer flips,
//! and otherwise only the elements whose last cascade resolved a viewport unit
//! (`Node::uses_viewport_units`), with their subtrees. These fixtures are the
//! correctness half: after each resize the document must compute exactly what
//! a fresh one built at the final size computes (`style_twin::assert_twin`).
//! `perf_counter_baselines::resize_by_1px*` is the cost half.

mod style_twin;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use style_twin::assert_twin;

const CSS: &str = "
    body { font-size: 16px; line-height: 20px; }
    .vw { width: 10vw; height: 3vh; }
    .vmin { width: 10vmin; }
    .vmax { height: 5vmax; }
    .fs { font-size: 2vw; }
    .fs .em { width: 3em; }
    .gen::before { content: \"x\"; width: 7vw; display: inline-block; }
    .calc { width: calc(20vw - 10px); }
    @media (min-width: 900px) { .mq-w { color: rgb(255, 0, 0); } }
    @media (max-height: 650px) { .mq-h { margin-top: 5px; } }
    @media (orientation: portrait) { .mq-o { padding-left: 7px; } }
    @media (min-aspect-ratio: 3/2) { .mq-ar { border-top: 2px solid black; } }
    .fixed { position: fixed; inset: 0; }
    .abs { position: absolute; top: 0; left: 0; right: 0; height: 10px; }
";

fn el(doc: &mut RinchDocument, parent: NodeId, class: &str) -> NodeId {
    let e = doc.create_element("div");
    doc.set_attribute(e, "class", class);
    doc.append_child(parent, e);
    e
}

fn build(w: f32, h: f32) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    for class in [
        "vw", "vmin", "vmax", "gen", "calc", "mq-w", "mq-h", "mq-o", "mq-ar", "fixed", "abs",
    ] {
        let e = el(&mut doc, body, class);
        let t = doc.create_text(class);
        doc.append_child(e, t);
    }
    let fs = el(&mut doc, body, "fs");
    let em = el(&mut doc, fs, "em");
    let t = doc.create_text("em");
    doc.append_child(em, t);
    // A viewport-unit user nested under an unrelated element.
    let wrap = el(&mut doc, body, "wrap");
    let inner = el(&mut doc, wrap, "vw");
    let t = doc.create_text("inner");
    doc.append_child(inner, t);
    doc.resolve_layout(w, h);
    doc.resolve_layout(w, h);
    doc
}

/// Every size a live resize might visit, each compared against a fresh
/// document built at that size. The sequence crosses every media query in the
/// sheet in both directions, and includes steps that cross none (where only
/// the viewport-unit users may restyle).
#[test]
fn every_resize_matches_a_fresh_document_at_the_final_size() {
    let sizes = [
        (800.0, 600.0),
        (801.0, 600.0),
        (1000.0, 600.0),
        (1001.0, 601.0),
        (1000.0, 700.0),
        (500.0, 900.0),
        (501.0, 900.0),
        (899.0, 600.0),
        (900.0, 600.0),
        (900.0, 599.0),
    ];
    let mut doc = build(800.0, 600.0);
    for (w, h) in sizes {
        doc.resolve_layout(w, h);
        doc.resolve_layout(w, h);
        let fresh = build(w, h);
        assert_twin(&doc, &fresh, &format!("after resizing to {w}x{h}"));
    }
}

/// With no media query and no viewport unit in any sheet, a resize cascades
/// nothing at all — and still lays out at the new width.
#[test]
fn a_resize_with_nothing_viewport_dependent_cascades_nothing() {
    let mut doc = RinchDocument::new();
    doc.load_css("body { font-size: 16px; } .a { width: 50%; }");
    let body = doc.body();
    let a = el(&mut doc, body, "a");
    doc.resolve_layout(800.0, 600.0);
    doc.resolve_layout(800.0, 600.0);
    doc.tree.perf.reset();
    doc.resolve_layout(1000.0, 600.0);
    let s = doc.tree.perf.end_frame();
    assert_eq!(s.get(rinch_dom::perf::Counter::ElementsCascaded), 0);
    assert_eq!(s.get(rinch_dom::perf::Counter::FullRestyles), 0);
    assert_eq!(doc.tree.nodes[a.0].layout.width, 500.0);
}

/// A resize that crosses no media query restyles exactly the viewport-unit
/// users; one that crosses a query restyles the whole document.
#[test]
fn only_a_flipped_media_query_restyles_the_whole_document() {
    use rinch_dom::perf::Counter;
    let mut doc = build(800.0, 600.0);
    doc.tree.perf.reset();
    doc.resolve_layout(820.0, 600.0);
    let s = doc.tree.perf.end_frame();
    assert_eq!(s.get(Counter::FullRestyles), 0, "no query flips at 820");
    // .vw, .vmin, .vmax, .gen (its ::before), .calc, .fs, and the nested .vw.
    assert_eq!(s.get(Counter::ViewportUnitRestyles), 7);
    doc.resolve_layout(950.0, 600.0);
    let s = doc.tree.perf.end_frame();
    assert_eq!(s.get(Counter::FullRestyles), 1, "min-width: 900px flips");
    assert_eq!(s.get(Counter::FullRestyleViewport), 1);
}

/// A `::before` sized in `vw` makes its element a viewport-unit user even when
/// the element's own style uses none.
#[test]
fn a_viewport_unit_in_a_pseudo_element_marks_its_element() {
    let mut doc = RinchDocument::new();
    doc.load_css(".gen::before { content: \"x\"; width: 7vw; display: inline-block; }");
    let body = doc.body();
    let g = el(&mut doc, body, "gen");
    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.nodes[g.0].uses_viewport_units.get());
    let plain = el(&mut doc, body, "plain");
    doc.resolve_layout(800.0, 600.0);
    assert!(!doc.tree.nodes[plain.0].uses_viewport_units.get());
}

/// An element's inline `style` applies to the element, never to its
/// `::before` / `::after` (CSS Style Attributes §3: the declarations "apply to
/// the element"; a pseudo-element's style comes from rules that select it).
/// Chrome: `getComputedStyle(el, "::before").width` is `auto` under
/// `<div style="width: 100px">`. The inline block is no longer handed to the
/// pseudo match at all (it cost two wasted cascades per inline-styled element
/// per restyle); this pins that the generated box still computes as Chrome's
/// — it did before the change too, so it is a guard, not a fail-first
/// fixture.
#[test]
fn an_inline_style_does_not_reach_the_pseudo_elements() {
    let mut doc = RinchDocument::new();
    doc.load_css(".gen::before { content: \"x\"; display: inline-block; }");
    let body = doc.body();
    let g = doc.create_element("div");
    doc.set_attribute(g, "class", "gen");
    doc.set_attribute(g, "style", "width: 100px; padding-left: 9px");
    doc.append_child(body, g);
    doc.resolve_layout(800.0, 600.0);
    let pseudo = doc.tree.nodes[g.0]
        .children
        .iter()
        .copied()
        .find(|&c| doc.tree.nodes[c].is_pseudo_element)
        .expect("the ::before is generated");
    let ps = &doc.tree.nodes[pseudo].computed_style;
    assert!(
        ps.width.lays_out_as_auto(),
        "the ::before took the element's inline width: {:?}",
        ps.width
    );
    assert_eq!(ps.padding_left.to_px(), 0.0);
}

/// A resize installs a new Stylo `Device`, and restyles only what the size
/// reaches — so `<html>` is usually not re-cascaded after one. The root-relative
/// units other than `rem` (`rch`, `rex`, `rcap`, `ric`) resolve against the
/// Device's root *style*, which therefore has to survive the rebuild the way
/// the root font-size does (`DeviceParams::root_style`). Without it, every
/// cascade after a resize read the fresh Device's default root style: 16px and
/// the initial font (review of #887, p20).
#[test]
fn root_relative_units_survive_resizes_that_restyle_nothing() {
    let css = "html { font-size: 30px; } .a { width: 5rem; } .a.on { width: 6rem; height: 2rch; }";
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    let body = doc.body();
    let a = el(&mut doc, body, "a");
    doc.resolve_layout(800.0, 600.0);
    doc.resolve_layout(800.0, 600.0);
    for w in [810.0, 820.0, 830.0] {
        doc.resolve_layout(w, 600.0);
        doc.resolve_layout(w, 600.0);
    }
    doc.set_attribute(a, "class", "a on");
    doc.resolve_layout(830.0, 600.0);

    let mut fresh = RinchDocument::new();
    fresh.load_css(css);
    let body = fresh.body();
    el(&mut fresh, body, "a on");
    fresh.resolve_layout(830.0, 600.0);
    fresh.resolve_layout(830.0, 600.0);
    assert_twin(&doc, &fresh, "a class toggle using rch after three resizes");
}

/// The same for a node inserted after a resize: it cascades under the new
/// Device (review of #887, p2c).
#[test]
fn root_relative_units_in_a_node_inserted_after_a_resize() {
    let css = "html { font-size: 30px; font-family: monospace; } \
               .x { width: 10rch; height: 10rex; }";
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    doc.resolve_layout(800.0, 600.0);
    doc.resolve_layout(1000.0, 700.0);
    let body = doc.body();
    el(&mut doc, body, "x");
    doc.resolve_layout(1000.0, 700.0);

    let mut fresh = RinchDocument::new();
    fresh.load_css(css);
    let body = fresh.body();
    el(&mut fresh, body, "x");
    fresh.resolve_layout(1000.0, 700.0);
    fresh.resolve_layout(1000.0, 700.0);
    assert_twin(&doc, &fresh, "an rch/rex node inserted after a resize");
}
