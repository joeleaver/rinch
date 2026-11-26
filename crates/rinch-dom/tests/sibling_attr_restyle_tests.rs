//! A selector that reads one element's attribute or class and styles another
//! (`p[data-x="y"] + p`, `p.some-class ~ p`) restyles that other element when
//! the attribute or class is added, changed or removed on a live document.
//! The editor writes an app's `data-*` attributes on a block's host element,
//! and an app styles the blocks beside it this way.

use rinch_core::dom::{DomContent, DomTree, NodeId};
use rinch_dom::RinchDocument;

type Rgb = (u8, u8, u8);
const BLACK: Rgb = (0, 0, 0);
const RED: Rgb = (255, 0, 0);

struct Page {
    doc: RinchDocument,
    wrap: NodeId,
    blocks: Vec<NodeId>,
}

/// `<div><p/><p/><p/></div>` under `css`, laid out once.
fn page(css: &str) -> Page {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!("p {{ color: black; }} {css}"));
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
    let mut page = Page { doc, wrap, blocks };
    page.settle();
    page
}

impl Page {
    fn settle(&mut self) {
        self.doc.resolve_layout(800.0, 600.0);
    }

    fn colors(&mut self) -> Vec<Rgb> {
        self.settle();
        self.blocks
            .iter()
            .map(|id| {
                let c = self
                    .doc
                    .tree
                    .get(id.0)
                    .unwrap()
                    .computed_style
                    .color
                    .unwrap()
                    .to_rgba8();
                (c.r, c.g, c.b)
            })
            .collect()
    }

    fn set(&mut self, block: usize, name: &str, value: &str) {
        self.doc.set_attribute(self.blocks[block], name, value);
    }

    fn remove(&mut self, block: usize, name: &str) {
        self.doc.remove_attribute(self.blocks[block], name);
    }
}

#[test]
fn an_attribute_selector_matches_and_stops_matching_the_element_itself() {
    for selector in [
        "p[data-x]",
        r#"p[data-x="y"]"#,
        r#"p[data-x^="y"]"#,
        r#"p[data-x$="y"]"#,
        r#"p[data-x*="y"]"#,
        r#"p[data-x~="y"]"#,
        r#"p[data-x|="y"]"#,
        r#"p[data-x="Y" i]"#,
    ] {
        let mut page = page(&format!("{selector} {{ color: red; }}"));
        assert_eq!(page.colors(), [BLACK; 3], "{selector}");
        page.set(1, "data-x", "y");
        assert_eq!(page.colors(), [BLACK, RED, BLACK], "{selector}");
        page.remove(1, "data-x");
        assert_eq!(page.colors(), [BLACK; 3], "{selector}");
    }
}

#[test]
fn the_next_sibling_follows_an_attribute_added_changed_and_removed() {
    let mut page = page(r#"p[data-x="y"] + p { color: red; }"#);
    assert_eq!(page.colors(), [BLACK; 3]);
    page.set(0, "data-x", "y");
    assert_eq!(page.colors(), [BLACK, RED, BLACK]);
    page.set(0, "data-x", "z");
    assert_eq!(page.colors(), [BLACK; 3]);
    page.set(0, "data-x", "y");
    assert_eq!(page.colors(), [BLACK, RED, BLACK]);
    page.remove(0, "data-x");
    assert_eq!(page.colors(), [BLACK; 3]);
    page.set(1, "data-x", "y");
    assert_eq!(page.colors(), [BLACK, BLACK, RED]);
}

#[test]
fn the_next_sibling_follows_a_class_added_and_removed() {
    let mut page = page("p.some-class + p { color: red; }");
    page.set(0, "class", "some-class");
    assert_eq!(page.colors(), [BLACK, RED, BLACK]);
    page.set(0, "class", "other");
    assert_eq!(page.colors(), [BLACK; 3]);
    page.set(0, "class", "other some-class");
    assert_eq!(page.colors(), [BLACK, RED, BLACK]);
    page.remove(0, "class");
    assert_eq!(page.colors(), [BLACK; 3]);
}

#[test]
fn later_siblings_follow_through_the_general_sibling_combinator() {
    for (css, name, value) in [
        ("p[data-x] ~ p { color: red; }", "data-x", ""),
        ("p.c ~ p { color: red; }", "class", "c"),
    ] {
        let mut page = page(css);
        page.set(0, name, value);
        assert_eq!(page.colors(), [BLACK, RED, RED], "{css}");
        page.remove(0, name);
        assert_eq!(page.colors(), [BLACK; 3], "{css}");
    }
}

/// Further along: a chain of two `+`, and a descendant of the next sibling.
#[test]
fn a_chain_and_a_siblings_descendant_follow_too() {
    let mut chain = page(r#"p[data-x="y"] + p + p { color: red; }"#);
    chain.set(0, "data-x", "y");
    assert_eq!(chain.colors(), [BLACK, BLACK, RED]);
    chain.remove(0, "data-x");
    assert_eq!(chain.colors(), [BLACK; 3]);

    let mut nested = page(r#"p[data-x="y"] + p span { color: red; }"#);
    let span = nested.doc.create_element("span");
    nested.doc.append_child(nested.blocks[1], span);
    let color = |page: &mut Page| {
        page.settle();
        let c = page
            .doc
            .tree
            .get(span.0)
            .unwrap()
            .computed_style
            .color
            .unwrap()
            .to_rgba8();
        (c.r, c.g, c.b)
    };
    assert_eq!(color(&mut nested), BLACK);
    nested.set(0, "data-x", "y");
    assert_eq!(color(&mut nested), RED);
    nested.remove(0, "data-x");
    assert_eq!(color(&mut nested), BLACK);
}

/// A sibling inserted after the styled block matches at once, and the one it
/// displaced stops matching.
#[test]
fn a_block_inserted_after_the_styled_one_takes_the_rule() {
    let mut page = page(r#"p[data-x="y"] + p { color: red; }"#);
    page.set(0, "data-x", "y");
    assert_eq!(page.colors(), [BLACK, RED, BLACK]);
    let new = page.doc.create_element("p");
    page.doc.insert_before(page.wrap, new, page.blocks[1]);
    page.blocks.insert(1, new);
    assert_eq!(page.colors(), [BLACK, RED, BLACK, BLACK]);
}
