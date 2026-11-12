//! `set_style` against an inline style that a browser's CSSOM would hold as a
//! declaration block of longhands, collapsed when the attribute is parsed
//! (#1298, #1353), and a reactive `style:` (`StyleProp`) composing onto a
//! duplicate it does not own (#1355).
//!
//! Every expected value is Chrome 157's, measured on the same markup with
//! `el.setAttribute('style', …)` then `el.style.setProperty(…)` and read back
//! with `getAttribute('style')` / `getComputedStyle`. Chrome's string orders
//! the important declarations last (Blink's parser groups them); rinch keeps
//! the author's order, so a string here names the same declarations in the
//! author's order, and the cascade result is pinned separately.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, StyleProp};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn div(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

fn style_of(doc: &RinchDocument, n: NodeId) -> String {
    doc.get_attribute(n, "style").unwrap_or_default()
}

fn rgb(doc: &RinchDocument, n: NodeId) -> (u8, u8, u8) {
    let c = doc.tree.get(n.0).unwrap().computed_style.color;
    let c = c.expect("color resolves").to_rgba8();
    (c.r, c.g, c.b)
}

/// A positioned parent with no border, and a child in it.
fn positioned(child_style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = div(
        &mut doc,
        body,
        "position: relative; width: 300px; height: 200px",
    );
    let child = div(&mut doc, parent, child_style);
    doc.resolve_layout(VW, VH);
    (doc, child)
}

fn xy(doc: &RinchDocument, node: NodeId) -> (f32, f32) {
    let l = doc.tree.get(node.0).unwrap().layout;
    (l.x, l.y)
}

/// #1298, the issue's shape, from both sides of the important shorthand.
/// Chrome: `left` computes 25px either way, and the attribute reads
/// `position: absolute; width: 10px; height: 10px; top: 0px !important;
/// right: 0px !important; bottom: 0px !important; left: 25px;` for both.
#[test]
fn a_normal_write_replaces_the_longhand_of_an_important_shorthand() {
    for style in [
        "position: absolute; left: 5px; inset: 0 !important; width: 10px; height: 10px",
        "position: absolute; inset: 0 !important; left: 5px; width: 10px; height: 10px",
    ] {
        let (mut doc, child) = positioned(style);
        assert_eq!(
            xy(&doc, child),
            (0.0, 0.0),
            "{style}: the important inset wins the parse"
        );
        doc.set_style(child, "left", "25px");
        doc.resolve_layout(VW + 1.0, VH);
        assert_eq!(xy(&doc, child), (25.0, 0.0), "{style}");
        assert_eq!(
            style_of(&doc, child),
            "position: absolute; top: 0px !important; right: 0px !important; \
             bottom: 0px !important; left: 25px; width: 10px; height: 10px",
            "{style}"
        );
    }
}

/// Two writes into one important shorthand, as a batch. Chrome: `top: 3px;
/// right: 0px !important; bottom: 0px !important; left: 25px;`.
#[test]
fn a_batch_takes_each_written_longhand_out_of_the_important_shorthand() {
    let (mut doc, child) =
        positioned("position: absolute; inset: 0 !important; width: 10px; height: 10px");
    doc.set_styles(child, &[("left", "25px"), ("top", "3px")]);
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(xy(&doc, child), (25.0, 3.0));
    assert_eq!(
        style_of(&doc, child),
        "position: absolute; top: 3px; right: 0px !important; \
         bottom: 0px !important; left: 25px; width: 10px; height: 10px",
    );
}

/// The other two shapes of the rule. A shorthand written over an important
/// longhand drops it (Chrome: `margin: 5px; width: 1px;`, margin-top 5px);
/// a longhand written over an important shorthand keeps the rest important
/// (Chrome: margin-top 5px, the others `0px !important`).
#[test]
fn shorthand_over_important_longhand_and_longhand_over_important_shorthand() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = div(
        &mut doc,
        body,
        "margin-top: 3px !important; width: 1px; height: 1px",
    );
    let b = div(
        &mut doc,
        body,
        "margin: 0 !important; width: 1px; height: 1px",
    );
    doc.resolve_layout(VW, VH);
    doc.set_style(a, "margin", "5px");
    doc.set_style(b, "margin-top", "5px");
    doc.resolve_layout(VW + 1.0, VH);

    let cs = |n: NodeId| {
        let s = &doc.tree.get(n.0).unwrap().computed_style;
        (s.margin_top.to_px(), s.margin_left.to_px())
    };
    assert_eq!(cs(a), (5.0, 5.0));
    assert_eq!(style_of(&doc, a), "width: 1px; height: 1px; margin: 5px");
    assert_eq!(cs(b), (5.0, 0.0));
    assert_eq!(
        style_of(&doc, b),
        "margin-top: 5px; margin-right: 0px !important; margin-bottom: 0px !important; \
         margin-left: 0px !important; width: 1px; height: 1px",
    );
}

/// The boundary: only a *shared longhand* is replaced. `left` and
/// `inset-inline-start` are different longhands of one logical group, and
/// Chrome's `setProperty("left", "5px")` leaves the important `inset-inline`
/// winning: `left` computes 0px, the attribute is `width: 1px;
/// inset-inline: 0px !important; left: 5px;`.
#[test]
fn an_important_logical_twin_is_not_replaced() {
    let (mut doc, child) =
        positioned("position: absolute; inset-inline: 0 !important; width: 10px; height: 10px");
    doc.set_style(child, "left", "5px");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(xy(&doc, child).0, 0.0);
    assert_eq!(
        style_of(&doc, child),
        "position: absolute; inset-inline: 0 !important; width: 10px; height: 10px; left: 5px",
    );
}

/// #1353, the issue's shape: Chrome reads back `margin: 1px; gap: 2px;
/// color: blue; padding: 9px;` — `color` was collapsed when the attribute
/// was parsed, though nothing wrote it.
#[test]
fn an_untouched_duplicate_collapses_as_the_cssom_parse_collapses_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let n = div(
        &mut doc,
        body,
        "margin: 1px; color: red; gap: 2px; color: blue",
    );
    doc.set_style(n, "padding", "9px");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        style_of(&doc, n),
        "margin: 1px; gap: 2px; color: blue; padding: 9px"
    );
    assert_eq!(rgb(&doc, n), (0, 0, 255));
}

/// The collapse is post-validity and priority-aware, as Chrome's parse is,
/// and the winner keeps its own slot. Chrome 157, each after
/// `setProperty('padding', '9px')`:
/// - `color: blue; gap: 2px; color: notacolor` → `color: blue; gap: 2px;
///   padding: 9px;` (blue: the invalid one never competed)
/// - `color: notacolor !important; gap: 2px; color: blue` → `gap: 2px;
///   color: blue; padding: 9px;` (blue)
/// - `color: red !important; gap: 2px; color: blue` → `gap: 2px; color: red
///   !important; padding: 9px;` (red)
#[test]
fn the_collapse_drops_invalid_and_outranked_declarations() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let older = div(&mut doc, body, "color: blue; gap: 2px; color: notacolor");
    let invalid = div(
        &mut doc,
        body,
        "color: notacolor !important; gap: 2px; color: blue",
    );
    let important = div(
        &mut doc,
        body,
        "color: red !important; gap: 2px; color: blue",
    );
    for n in [older, invalid, important] {
        doc.set_style(n, "padding", "9px");
    }
    doc.resolve_layout(VW, VH);

    assert_eq!(style_of(&doc, older), "color: blue; gap: 2px; padding: 9px");
    assert_eq!(rgb(&doc, older), (0, 0, 255));
    assert_eq!(
        style_of(&doc, invalid),
        "gap: 2px; color: blue; padding: 9px"
    );
    assert_eq!(rgb(&doc, invalid), (0, 0, 255));
    assert_eq!(
        style_of(&doc, important),
        "color: red !important; gap: 2px; padding: 9px"
    );
    assert_eq!(rgb(&doc, important), (255, 0, 0));
}

/// #1355: a reactive `style:` binding (`StyleProp`) composing onto a node
/// whose own style carries `color: notacolor !important; color: blue`.
/// Chrome computes blue for that attribute; before the fix `StyleProp`
/// collapsed it to the invalid important declaration while writing `gap`,
/// and Stylo then dropped it (black).
#[test]
fn a_style_prop_leaves_another_authors_duplicate_to_stylo() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let n = {
        let mut d = doc.borrow_mut();
        let body = d.body();
        div(&mut d, body, "color: notacolor !important; color: blue")
    };
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let handle = NodeHandle::new(n, Rc::downgrade(&dyn_doc));
    let mut prop = StyleProp::default();
    prop.apply(&handle, "gap: 4px");
    prop.apply(&handle, "gap: 6px");
    let mut d = doc.borrow_mut();
    d.resolve_layout(VW, VH);
    assert_eq!(rgb(&d, n), (0, 0, 255), "attribute = {:?}", style_of(&d, n));
}
