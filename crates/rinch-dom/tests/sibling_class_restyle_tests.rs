//! A selector that reads one element's class and styles another
//! (`p.some-class + p`) restyles that other element when the class is added to
//! or removed from a live document, in its box as well as its colour. The
//! editor's node decorations put a class on a block's host element, and an app
//! styles the blocks beside it this way.

use rinch_core::dom::{DomContent, DomTree, NodeId};
use rinch_dom::RinchDocument;

type Rgb = (u8, u8, u8);
const BLACK: Rgb = (0, 0, 0);
const RED: Rgb = (255, 0, 0);

struct Page {
    doc: RinchDocument,
    blocks: Vec<NodeId>,
}

/// `<div><p/><p/><p><span/></p></div>` under `css`, laid out once.
fn page(css: &str) -> Page {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "p {{ color: black; height: 20px; margin: 0; }} {css}"
    ));
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.append_child(body, wrap);
    let blocks = (0..3)
        .map(|i| {
            let p = doc.create_element("p");
            doc.append_child(wrap, p);
            let t = doc.create_text(&format!("block {i}"));
            doc.append_child(p, t);
            p
        })
        .collect();
    let mut page = Page { doc, blocks };
    page.settle();
    page
}

impl Page {
    fn settle(&mut self) {
        self.doc.resolve_layout(800.0, 600.0);
    }

    fn color(&mut self, id: NodeId) -> Rgb {
        self.settle();
        let style = &self.doc.tree.get(id.0).unwrap().computed_style;
        let c = style.color.unwrap().to_rgba8();
        (c.r, c.g, c.b)
    }

    fn colors(&mut self) -> Vec<Rgb> {
        self.blocks
            .clone()
            .iter()
            .map(|id| self.color(*id))
            .collect()
    }

    /// Each block's `(y, height)`.
    fn boxes(&mut self) -> Vec<(f32, f32)> {
        self.settle();
        self.blocks
            .iter()
            .map(|id| {
                let l = &self.doc.tree.get(id.0).unwrap().layout;
                (l.y, l.height)
            })
            .collect()
    }

    fn class(&mut self, block: usize, value: Option<&str>) {
        match value {
            Some(v) => self.doc.set_attribute(self.blocks[block], "class", v),
            None => self.doc.remove_attribute(self.blocks[block], "class"),
        }
    }
}

#[test]
fn the_next_sibling_follows_a_class_added_and_removed() {
    let mut p = page("p.some-class + p { color: red; }");
    for _ in 0..2 {
        p.class(0, Some("some-class"));
        assert_eq!(p.colors(), [BLACK, RED, BLACK]);
        p.class(0, Some("other"));
        assert_eq!(p.colors(), [BLACK; 3]);
        p.class(0, Some("other some-class"));
        assert_eq!(p.colors(), [BLACK, RED, BLACK]);
        p.class(0, None);
        assert_eq!(p.colors(), [BLACK; 3]);
    }
    // The class moving down one takes the rule with it.
    p.class(0, Some("some-class"));
    p.settle();
    p.class(0, None);
    p.class(1, Some("some-class"));
    assert_eq!(p.colors(), [BLACK, BLACK, RED]);
}

#[test]
fn a_hidden_next_sibling_leaves_and_takes_its_room_again() {
    let mut p = page("p.some-class + p { display: none; }");
    let open = p.boxes();
    assert_eq!(open, [(0.0, 20.0), (20.0, 20.0), (40.0, 20.0)]);
    p.class(0, Some("some-class"));
    let closed = p.boxes();
    assert_eq!(closed[1].1, 0.0);
    assert_eq!(closed[2], (20.0, 20.0));
    p.class(0, None);
    assert_eq!(p.boxes(), open);
}

#[test]
fn the_next_siblings_box_follows_a_size_rule() {
    let mut p = page("p.some-class + p { height: 50px; margin-top: 5px; }");
    p.class(0, Some("some-class"));
    assert_eq!(p.boxes(), [(0.0, 20.0), (25.0, 50.0), (75.0, 20.0)]);
    p.class(0, None);
    assert_eq!(p.boxes(), [(0.0, 20.0), (20.0, 20.0), (40.0, 20.0)]);
}

#[test]
fn later_siblings_and_their_descendants_follow() {
    let mut p = page("p.some-class ~ p span { color: red; }");
    let span = p.doc.create_element("span");
    p.doc.append_child(p.blocks[2], span);
    assert_eq!(p.color(span), BLACK);
    p.class(0, Some("some-class"));
    assert_eq!(p.color(span), RED);
    p.class(0, None);
    assert_eq!(p.color(span), BLACK);
}
