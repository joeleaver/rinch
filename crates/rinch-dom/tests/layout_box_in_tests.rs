//! `DomLayout::layout_box_in` on rinch-dom (#3065): a node's border box as
//! laid out, before any transform, in a container's padding-box frame, and
//! `None` wherever the web's `offsetParent` chain would not reach the
//! container. The twin of `rinch-web/tests/layout_box_in_3065.rs`, on the
//! same fixture.
//!
//! The container is bordered (wider on the left) and padded, and offset in
//! the page, so an answer from its border box, its content box or the
//! viewport differs from the right one; the nested box is positioned and
//! translated, and the item is scaled, so a painted answer differs too.

use rinch_core::dom::{DomContent, DomLayout, DomTree, NodeId};
use rinch_core::reactive::ElementBounds;
use rinch_dom::RinchDocument;

const CSS: &str = "body { margin: 0 } div { margin: 0; padding: 0; border: 0 } \
    .lb-c { position: relative; left: 7px; top: 3px; width: 300px; height: 200px; \
            border: 5px solid black; border-left-width: 9px; padding: 4px 6px } \
    .lb-direct { margin-left: 13px; width: 40px; height: 10px } \
    .lb-p { position: relative; box-sizing: content-box; margin-left: 11px; width: 100px; height: 60px; \
            border: 2px solid black; padding: 3px; transform: translateX(40px) } \
    .lb-n { margin-top: 8px; width: 50px; height: 20px; transform: scale(2) } \
    .lb-fixed { position: fixed; left: 5px; top: 5px; width: 10px; height: 10px } \
    .lb-static { width: 100px; height: 40px; border: 3px solid black; padding: 2px } \
    .lb-in { width: 10px; height: 10px; margin-left: 4px } \
    .lb-t { width: 100px; height: 40px; border: 3px solid black; transform: translateX(1px) } \
    .lb-tin { width: 10px; height: 10px; margin-left: 4px } \
    .lb-frac { position: relative; margin-left: 2.25px; width: 20.5px; height: 10px } \
    .lb-gone { display: none }";

struct Fixture {
    doc: RinchDocument,
    ids: Vec<(&'static str, NodeId)>,
}

impl Fixture {
    fn new() -> Self {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let mut ids = Vec::new();
        let mut el = |doc: &mut RinchDocument, parent: NodeId, class: &'static str| {
            let n = doc.create_element("div");
            doc.set_attribute(n, "class", class);
            doc.append_child(parent, n);
            ids.push((class, n));
            n
        };
        let body = doc.body();
        let c = el(&mut doc, body, "lb-c");
        el(&mut doc, c, "lb-direct");
        let p = el(&mut doc, c, "lb-p");
        el(&mut doc, p, "lb-n");
        el(&mut doc, c, "lb-fixed");
        let frac = el(&mut doc, c, "lb-frac");
        let gone = el(&mut doc, frac, "lb-gone");
        el(&mut doc, gone, "lb-in-gone");
        let s = el(&mut doc, body, "lb-static");
        el(&mut doc, s, "lb-in");
        let t = el(&mut doc, body, "lb-t");
        el(&mut doc, t, "lb-tin");
        doc.resolve_layout(800.0, 600.0);
        Self { doc, ids }
    }

    fn id(&self, class: &str) -> NodeId {
        self.ids.iter().find(|(c, _)| *c == class).unwrap().1
    }

    fn box_in(&self, node: &str, container: &str) -> Option<ElementBounds> {
        self.doc.layout_box_in(self.id(node), self.id(container))
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Option<ElementBounds> {
    Some(ElementBounds {
        x,
        y,
        width,
        height,
    })
}

/// A child of a bordered, padded container: from its padding edge.
#[test]
fn a_child_is_measured_from_the_containers_padding_edge() {
    let f = Fixture::new();
    assert_eq!(f.box_in("lb-direct", "lb-c"), rect(19.0, 4.0, 40.0, 10.0));
}

/// Inside a positioned, bordered, translated box: its border and padding
/// count, its translate does not, nor the item's own scale.
#[test]
fn a_nested_positioned_box_and_transforms() {
    let mut f = Fixture::new();
    assert_eq!(f.box_in("lb-n", "lb-c"), rect(22.0, 27.0, 50.0, 20.0));
    assert_eq!(f.box_in("lb-n", "lb-p"), rect(3.0, 11.0, 50.0, 20.0));
    // A positive control: `lb-n` is painted scaled and translated.
    let n = f.id("lb-n");
    let painted = f.doc.bounding_rect(n).expect("a painted box");
    assert_eq!((painted.width, painted.height), (100.0, 40.0));
}

/// At a fractional offset (8.25px) with a fractional width (20.5px), whole
/// pixels: the layout is rounded to the pixel grid, and the web's `offset*`
/// answer the same (`rinch-web/tests/layout_box_in_3065.rs`).
#[test]
fn a_fractional_box_answers_whole_pixels_as_on_the_web() {
    let f = Fixture::new();
    assert_eq!(f.box_in("lb-frac", "lb-c"), rect(8.0, 84.0, 21.0, 10.0));
}

/// No `offsetParent` chain reaches the container, as on the web: it is not
/// positioned, the node is `position: fixed`, or the node is not rendered.
/// Each is a real ancestor laid out around the node, which is what the walk
/// alone used to accept (#3065 review).
#[test]
fn none_without_an_offset_parent_chain_to_the_container() {
    let f = Fixture::new();
    assert_eq!(f.box_in("lb-in", "lb-static"), None);
    assert_eq!(f.box_in("lb-fixed", "lb-c"), None);
    assert_eq!(f.box_in("lb-in-gone", "lb-c"), None);
    // The control: laid out under its container, and measured against the
    // body (its offset parent's chain ends there).
    let in_body = f.doc.layout_box_in(f.id("lb-in"), f.doc.body());
    assert!(in_body.is_some_and(|b| b.width == 10.0));
}

/// A transformed box that is not positioned is an offset parent all the
/// same: it is a containing block for absolutes (measured: Chrome 157,
/// Firefox 160).
#[test]
fn a_transformed_container_is_an_offset_parent() {
    let f = Fixture::new();
    assert_eq!(f.box_in("lb-tin", "lb-t"), rect(4.0, 0.0, 10.0, 10.0));
}

/// What makes a box an `offsetParent` (a containing block for absolutes),
/// measured in Chrome 158 and Firefox 160, which agree on every row: a static
/// child's `offsetParent` and an absolute child's are both the box exactly
/// for these declarations. The engines also count `translate`, `rotate`,
/// `scale`, `offset-path`, `transform-style: preserve-3d` and
/// `content-visibility: auto`/`hidden`, which rinch does not implement. The
/// twin of the web test's list (`rinch-web/tests/layout_box_in_3065.rs`).
const OFFSET_PARENTS: &[&str] = &[
    "position: relative",
    "transform: translateX(1px)",
    "filter: blur(0px)",
    "filter: opacity(1)",
    "backdrop-filter: blur(1px)",
    "perspective: 100px",
    "contain: paint",
    "contain: layout",
    "contain: strict",
    "contain: content",
    "will-change: transform",
    "will-change: filter",
    "will-change: backdrop-filter",
    "will-change: perspective",
    "will-change: contain",
    "will-change: position",
];

/// What makes neither, in both engines.
const NOT_OFFSET_PARENTS: &[&str] = &[
    "",
    "transform: none",
    "filter: none",
    "backdrop-filter: none",
    "perspective: none",
    "contain: size",
    "contain: style",
    "contain: inline-size",
    "container-type: size",
    "clip-path: inset(0)",
    "isolation: isolate",
    "mix-blend-mode: multiply",
    "opacity: 0.5",
    "overflow: hidden",
    "z-index: 1",
    "will-change: opacity",
    "will-change: left",
];

/// For each declaration, a box 30px into the page holding a static child 4px
/// into it and an absolute child at `left: 5px`: the static child's box in
/// it, and how far the absolute child is painted from the box's left edge
/// (5 where the box contains it, -25 where the page does).
fn offset_parent_answers(decls: &[&str]) -> Vec<(String, Option<ElementBounds>, f32)> {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "body { margin: 0 } div { margin: 0; padding: 0; border: 0 } \
         .op-c { margin-left: 30px; width: 200px; height: 20px } \
         .op-k { margin-left: 4px; width: 10px; height: 10px } \
         .op-a { position: absolute; left: 5px; top: 0; width: 1px; height: 1px }",
    );
    let body = doc.body();
    let mut made = Vec::new();
    for decl in decls {
        let c = doc.create_element("div");
        doc.set_attribute(c, "class", "op-c");
        doc.set_attribute(c, "style", decl);
        doc.append_child(body, c);
        let k = doc.create_element("div");
        doc.set_attribute(k, "class", "op-k");
        doc.append_child(c, k);
        let a = doc.create_element("div");
        doc.set_attribute(a, "class", "op-a");
        doc.append_child(c, a);
        made.push((decl.to_string(), c, k, a));
    }
    doc.resolve_layout(800.0, 2000.0);
    made.into_iter()
        .map(|(decl, c, k, a)| {
            let at = |doc: &mut RinchDocument, n| doc.bounding_rect(n).map_or(f32::NAN, |r| r.x);
            let placed = at(&mut doc, a) - at(&mut doc, c);
            (decl, doc.layout_box_in(k, c), placed)
        })
        .collect()
}

/// Each declaration the engines make an `offsetParent` for makes one here,
/// and the box contains its absolute child as it does there (#3065 review:
/// only `position`, `transform` and `filter` counted).
#[test]
fn every_declaration_the_engines_count_makes_an_offset_parent() {
    let wrong: Vec<_> = offset_parent_answers(OFFSET_PARENTS)
        .into_iter()
        .filter(|(_, b, placed)| *b != rect(4.0, 0.0, 10.0, 10.0) || *placed != 5.0)
        .collect();
    assert!(wrong.is_empty(), "not an offset parent here: {wrong:?}");
}

/// And none of the others does: the child is not measured against the box,
/// and its absolute sibling is placed against the page.
#[test]
fn no_other_declaration_makes_an_offset_parent() {
    let wrong: Vec<_> = offset_parent_answers(NOT_OFFSET_PARENTS)
        .into_iter()
        .filter(|(_, b, placed)| b.is_some() || *placed != -25.0)
        .collect();
    assert!(wrong.is_empty(), "an offset parent here: {wrong:?}");
}

/// A box at `margin-left: inner`, `width` wide, inside a positioned box at
/// `left: outer` (all px): its box in the positioned one.
fn rounded_in(outer: f32, inner: f32, width: f32) -> Option<ElementBounds> {
    let mut doc = RinchDocument::new();
    doc.load_css("body { margin: 0 } div { margin: 0; padding: 0; border: 0 }");
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        &format!("position: relative; left: {outer}px; width: 100px; height: 10px"),
    );
    let body = doc.body();
    doc.append_child(body, c);
    let k = doc.create_element("div");
    doc.set_attribute(
        k,
        "style",
        &format!("margin-left: {inner}px; width: {width}px; height: 5px"),
    );
    doc.append_child(c, k);
    doc.resolve_layout(800.0, 600.0);
    doc.layout_box_in(k, c)
}

/// Each offset is rounded on its own, from the unrounded layout, half up
/// (`floor(x + 0.5)`), as Chrome 158 rounds `offsetLeft` and `offsetWidth`:
/// not the difference of two edges each rounded where it falls in the page.
/// Firefox 160 rounds a negative tie away from zero instead (-0.5 to -1,
/// -8.5 to -9); every other row is both engines' answer.
#[test]
fn offsets_round_half_up_each_on_its_own() {
    let x = |outer, inner| rounded_in(outer, inner, 10.0).map(|b| b.x);
    // Ties, positive and negative, in a box at a whole pixel.
    assert_eq!(x(0.0, 0.5), Some(1.0));
    assert_eq!(x(0.0, 2.5), Some(3.0));
    assert_eq!(x(0.0, -0.5), Some(0.0));
    assert_eq!(x(0.0, -1.5), Some(-1.0));
    assert_eq!(x(0.0, -8.5), Some(-8.0));
    assert_eq!(x(0.0, -0.75), Some(-1.0));
    assert_eq!(x(0.0, 0.25), Some(0.0));
    // In a box at a half pixel, whose edge rounds to 1: the offset is still
    // the tie, rounded.
    assert_eq!(x(0.5, -0.5), Some(0.0));
    assert_eq!(x(0.5, 0.5), Some(1.0));
    assert_eq!(x(0.5, -8.5), Some(-8.0));
    assert_eq!(x(0.25, 0.25), Some(0.0));
    // The width is its own length rounded, wherever it starts.
    let w = |inner, width| rounded_in(0.0, inner, width).map(|b| b.width);
    assert_eq!(w(0.5, 20.5), Some(21.0));
    assert_eq!(w(0.5, 20.25), Some(20.0));
    assert_eq!(w(-0.5, 20.5), Some(21.0));
    assert_eq!(w(0.25, 20.5), Some(21.0));
}
