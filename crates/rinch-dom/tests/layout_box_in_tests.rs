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
