//! #2357: `Node::under_isolated`, the per-element "below an isolated node"
//! flag `TElement::matches_user_and_content_rules` reads, cannot go stale.
//! From the review of PR #2366 (round 3), where a subtree re-inserted while
//! the document had no isolated node kept a stale `true`, and a theme class
//! rule then failed to match it once another view existed.
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// Theme-like author rules; `letter-spacing` marks which rules matched.
const CSS: &str = "*, *::before, *::after { box-sizing: border-box }
.hl { letter-spacing: 4px }";

struct T {
    doc: RinchDocument,
    w: f32,
}

impl T {
    fn new() -> Self {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        T { doc, w: 800.0 }
    }
    fn el(&mut self, parent: NodeId, tag: &str) -> NodeId {
        let e = self.doc.create_element(tag);
        self.doc.append_child(parent, e);
        e
    }
    fn resolve(&mut self) {
        // A new viewport each time, so the resolve never early-returns.
        self.w += 1.0;
        let w = self.w;
        self.doc.resolve_layout(w, 600.0);
    }
    fn border_box(&self, n: NodeId) -> bool {
        self.doc
            .tree
            .get(n.0)
            .unwrap()
            .computed_style
            .box_sizing_border_box
    }
    fn letter_spacing(&self, n: NodeId) -> f32 {
        self.doc
            .tree
            .get(n.0)
            .unwrap()
            .computed_style
            .letter_spacing
    }
}

/// Detach a subtree from inside a view without freeing it, free the view
/// (the isolated count goes to 0), re-insert the subtree outside, create
/// another view (count 1), then add a class: the theme's class rule must
/// match, since the subtree is under no view.
#[test]
fn a_subtree_reinserted_while_no_node_is_isolated_is_not_left_isolated() {
    let mut t = T::new();
    let body = t.doc.body();
    let host1 = t.el(body, "div");
    let view1 = t.el(host1, "div");
    t.doc.isolate_styles(view1);
    let d = t.el(view1, "div");
    let p = t.el(d, "p");
    let s = t.el(p, "span");
    t.resolve();
    assert!(!t.border_box(s), "inside the view");

    t.doc.remove_child(view1, d);
    t.doc.set_inner_html(host1, "");
    assert_eq!(t.doc.tree.isolated_nodes, 0, "the view was freed");
    t.doc.append_child(body, d);
    t.resolve();

    let view2 = t.el(body, "div");
    t.doc.isolate_styles(view2);
    t.el(view2, "i");
    t.resolve();
    t.doc.set_attribute(s, "class", "hl");
    t.resolve();
    assert!(
        t.border_box(s),
        "the theme's `*` rule matches s outside any view"
    );
    assert_eq!(
        t.letter_spacing(s),
        4.0,
        "the theme's class rule matches s outside any view"
    );
}

/// A subtree (div > p > span) moved into a view and out again, with no
/// other change: the moved root's own style changes only in a reset
/// property (`box-sizing`), and its descendants must follow both ways.
#[test]
fn a_subtree_moved_into_a_view_and_out_again_is_matched_anew() {
    let mut t = T::new();
    let body = t.doc.body();
    let view = t.el(body, "div");
    t.doc.isolate_styles(view);
    let inner = t.el(view, "div");
    let outside = t.el(body, "section");
    let d = t.el(outside, "div");
    let p = t.el(d, "p");
    let s = t.el(p, "span");
    t.doc.set_attribute(s, "class", "hl");
    t.resolve();
    assert!(t.border_box(d) && t.border_box(p) && t.border_box(s));
    assert_eq!(t.letter_spacing(s), 4.0);

    t.doc.insert_before(view, d, inner);
    t.resolve();
    assert!(
        !t.border_box(d) && !t.border_box(p) && !t.border_box(s),
        "moved in: the theme's `*` rule no longer matches"
    );
    assert_eq!(t.letter_spacing(s), 0.0, "moved in: nor its class rule");

    t.doc.append_child(outside, d);
    t.resolve();
    assert!(
        t.border_box(d) && t.border_box(p) && t.border_box(s),
        "moved out: the theme's `*` rule matches again"
    );
    assert_eq!(t.letter_spacing(s), 4.0, "moved out: and its class rule");
}
