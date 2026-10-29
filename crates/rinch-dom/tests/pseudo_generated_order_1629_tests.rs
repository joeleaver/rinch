//! A generated `::before` stays the first box of its element and a generated
//! `::after` the last, whatever is inserted later (#1629).
//!
//! `resolve_pseudo_element` puts the `::after` box at the end of the element's
//! child list, and the `::before` box (or a list marker) at its start, when the
//! element is cascaded — usually when it is attached. A child inserted after
//! that went through `append_child` / `insert_before` / `insert_child` /
//! `replace_node`, which knew nothing about generated boxes: an append landed
//! after the `::after` box, and an `insert_child(p, c, 0)` before the
//! `::before` box. Inserting a child does not re-cascade its parent, so nothing
//! put the generated box back. CSS 2.1 §12.1: `::before` is the element's
//! first child box and `::after` its last, always.
//!
//! Every fixture checks two things: the element's child list (generated box
//! first / last) and the inline formatting context's flat text, which is what
//! is drawn. The text is all ASCII words; the order of words, not a measured
//! width, is the assertion, so no literal pins the host's fonts. Line height
//! and font size are declared anyway.

#![cfg(feature = "software-renderer")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::reactive::Signal;
use rinch_dom::RinchDocument;

const CSS: &str = "body { font-size: 16px; line-height: 20px; margin: 0 } \
                   p { margin: 0; width: 600px } \
                   .a::after { content: \" SUFFIX\"; color: rgb(200, 100, 50) } \
                   .b::before { content: \"PREFIX \"; color: rgb(50, 100, 200) }";

/// `body > p.<class>`, attached and laid out **before** it has any children,
/// so its generated boxes exist before anything is inserted.
fn doc(class: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let p = doc.create_element("p");
    doc.set_attribute(p, "class", class);
    doc.append_child(body, p);
    doc.resolve_layout(800.0, 400.0);
    (doc, p)
}

fn text(doc: &mut RinchDocument, s: &str) -> NodeId {
    doc.create_text(s)
}

fn span(doc: &mut RinchDocument, s: &str) -> NodeId {
    let e = doc.create_element("span");
    let t = doc.create_text(s);
    doc.append_child(e, t);
    e
}

/// A layout that cannot early-return: the viewport changes every call.
fn relayout(doc: &mut RinchDocument, n: &mut f32) {
    *n += 1.0;
    doc.resolve_layout(800.0 + *n, 400.0);
}

/// The IFC's flat text, with collapsible runs of spaces shown as one.
fn flat(doc: &RinchDocument, p: NodeId) -> String {
    let tl = doc
        .tree
        .get(p.0)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("the element is an IFC root");
    tl.text_content.clone()
}

/// `B` for a generated box that is not an `::after` (a `::before`), `A` for
/// an `::after`, `.` for anything else, in child-list order.
fn shape(doc: &RinchDocument, p: NodeId) -> String {
    let children = &doc.tree.get(p.0).unwrap().children;
    children
        .iter()
        .map(|&c| {
            let n = doc.tree.get(c).unwrap();
            if !n.is_pseudo_element {
                '.'
            } else if flat_of(doc, c).contains("SUFFIX") {
                'A'
            } else {
                'B'
            }
        })
        .collect()
}

fn flat_of(doc: &RinchDocument, id: u64) -> String {
    let n = doc.tree.get(id).unwrap();
    match &n.kind {
        rinch_dom::node::NodeKind::Text(t) => t.content.clone(),
        _ => n.children.iter().map(|&c| flat_of(doc, c)).collect(),
    }
}

fn generated(doc: &RinchDocument, p: NodeId) -> NodeId {
    NodeId(
        doc.tree
            .get(p.0)
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|&c| doc.tree.get(c).unwrap().is_pseudo_element)
            .expect("the element has a generated box"),
    )
}

// ── append_child ────────────────────────────────────────────────────────────

/// The issue's shape: children appended to an element whose `::after` was
/// generated when it was attached. They used to land after it: the IFC read
/// `"SUFFIXalpha beta …"`, the suffix first and its space trimmed as the
/// IFC's leading space.
#[test]
fn children_appended_after_attach_go_before_the_after_box() {
    let (mut d, p) = doc("a");
    let t = text(&mut d, "alpha beta ");
    d.append_child(p, t);
    let s = span(&mut d, "gamma");
    d.append_child(p, s);
    let t = text(&mut d, " delta");
    d.append_child(p, t);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "...A");
    assert_eq!(flat(&d, p), "alpha beta gamma delta SUFFIX");
}

/// The first child of an element whose only child was its `::after`.
#[test]
fn the_first_child_goes_before_a_lone_after_box() {
    let (mut d, p) = doc("a");
    assert_eq!(shape(&d, p), "A", "a lone ::after box before any child");
    let t = text(&mut d, "alpha");
    d.append_child(p, t);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), ".A");
    assert_eq!(flat(&d, p), "alpha SUFFIX");
}

/// A child of the element appended again (a move within one parent) goes to
/// the end of the element's own children, still before the `::after` box.
#[test]
fn a_child_moved_to_the_end_stays_before_the_after_box() {
    let (mut d, p) = doc("a");
    let a = span(&mut d, "alpha");
    d.append_child(p, a);
    let t = text(&mut d, " beta ");
    d.append_child(p, t);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    d.append_child(p, a);
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "..A");
    assert_eq!(flat(&d, p), "beta alpha SUFFIX");
}

/// A `::before` does not move an append (an append is at the end), and both
/// generated boxes keep their places around it.
#[test]
fn an_append_lands_between_a_before_and_an_after_box() {
    let (mut d, p) = doc("a b");
    assert_eq!(shape(&d, p), "BA");
    let t = text(&mut d, "alpha");
    d.append_child(p, t);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "B.A");
    assert_eq!(flat(&d, p), "PREFIX alpha SUFFIX");
}

// ── insert_child ────────────────────────────────────────────────────────────

/// Index 0 is before the element's first *own* child: after the `::before`.
#[test]
fn insert_child_at_zero_goes_after_the_before_box() {
    let (mut d, p) = doc("b");
    let t = text(&mut d, "beta");
    d.append_child(p, t);
    let a = text(&mut d, "alpha ");
    d.insert_child(p, a, 0);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "B..");
    assert_eq!(flat(&d, p), "PREFIX alpha beta");
}

/// An index past the end is an append: before the `::after`.
#[test]
fn insert_child_past_the_end_goes_before_the_after_box() {
    let (mut d, p) = doc("a");
    let t = text(&mut d, "alpha ");
    d.insert_child(p, t, 0);
    let t = text(&mut d, "beta");
    d.insert_child(p, t, 99);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "..A");
    assert_eq!(flat(&d, p), "alpha beta SUFFIX");
}

/// An index that counts the generated boxes (one read off `get_children`,
/// which lists them) at the very end — `len` — is still before the `::after`.
#[test]
fn insert_child_at_the_listed_length_goes_before_the_after_box() {
    let (mut d, p) = doc("a b");
    let t = text(&mut d, "alpha ");
    d.append_child(p, t);
    let len = d.get_children(p).len();
    let t = text(&mut d, "beta");
    d.insert_child(p, t, len);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "B..A");
    assert_eq!(flat(&d, p), "PREFIX alpha beta SUFFIX");
}

// ── insert_before ───────────────────────────────────────────────────────────

/// A reference that is the `::before` box itself (`get_children` lists it,
/// so a caller can name it): the child goes after it, not before.
#[test]
fn insert_before_the_before_box_goes_after_it() {
    let (mut d, p) = doc("b");
    let t = text(&mut d, "beta");
    d.append_child(p, t);
    let before = generated(&d, p);
    let a = text(&mut d, "alpha ");
    d.insert_before(p, a, before);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "B..");
    assert_eq!(flat(&d, p), "PREFIX alpha beta");
}

/// A reference that is not a child of the element is an append: before the
/// `::after`.
#[test]
fn insert_before_a_stranger_goes_before_the_after_box() {
    let (mut d, p) = doc("a");
    let t = text(&mut d, "alpha");
    d.append_child(p, t);
    let stranger = d.create_element("i");
    let b = text(&mut d, " beta");
    d.insert_before(p, b, stranger);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "..A");
    assert_eq!(flat(&d, p), "alpha beta SUFFIX");
}

/// A reference that is the `::after` box: before it, as it always was (the
/// shape a `for` reconcile's `insert_after` takes, since `next_sibling`
/// answers the generated box).
#[test]
fn insert_before_the_after_box_goes_before_it() {
    let (mut d, p) = doc("a");
    let t = text(&mut d, "alpha");
    d.append_child(p, t);
    let after = generated(&d, p);
    let b = text(&mut d, " beta");
    d.insert_before(p, b, after);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "..A");
    assert_eq!(flat(&d, p), "alpha beta SUFFIX");
}

// ── replace_node ────────────────────────────────────────────────────────────

/// A replacement takes the place of the node it replaces — between the
/// generated boxes — including one moved in from the element's own end.
#[test]
fn replace_node_keeps_the_generated_boxes_at_the_ends() {
    let (mut d, p) = doc("a b");
    let x = span(&mut d, "xx");
    d.append_child(p, x);
    let t = text(&mut d, " beta");
    d.append_child(p, t);
    let mut n = 0.0;
    relayout(&mut d, &mut n);
    let a = span(&mut d, "alpha");
    d.replace_node(x, a);
    relayout(&mut d, &mut n);
    assert_eq!(shape(&d, p), "B..A");
    assert_eq!(flat(&d, p), "PREFIX alpha beta SUFFIX");
}

// ── rsx's reactive helpers, through NodeHandle ──────────────────────────────

/// A `for` reconcile and a `show_dom` branch whose markers are appended to an
/// element already holding its generated boxes, then grown: every row and the
/// branch land between them.
#[test]
fn a_for_and_an_if_reconcile_stay_between_the_generated_boxes() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    doc.borrow_mut().load_css(CSS);
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc, body);
    let p = scope.create_element("p");
    p.set_attribute("class", "a b");
    scope.parent().append_child(&p);
    doc.borrow_mut().resolve_layout(800.0, 400.0);

    let rows = Signal::new(vec!["one".to_string()]);
    let _marker = rinch_core::for_loop::for_each_dom_typed(
        &mut scope,
        &p,
        move || rows.get(),
        |s: &String| s.clone(),
        |s: String, sc: &mut RenderScope| {
            let e = sc.create_element("span");
            let t = sc.create_text(&format!("{s} "));
            e.append_child(&t);
            e
        },
    );
    let showing = Signal::new(false);
    let _branch = rinch_core::show::show_dom(
        &mut scope,
        &p,
        move || showing.get(),
        |sc: &mut RenderScope| {
            let e: NodeHandle = sc.create_element("b");
            let t = sc.create_text("shown");
            e.append_child(&t);
            e
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    rows.set(vec!["one".into(), "two".into(), "three".into()]);
    showing.set(true);
    doc.borrow_mut().resolve_layout(801.0, 400.0);

    let pid = NodeId(p.node_id().0);
    let d = doc.borrow();
    let s = shape(&d, pid);
    assert!(
        s.starts_with('B') && s.ends_with('A') && s.matches(['A', 'B']).count() == 2,
        "the generated boxes stay at the ends: {s}"
    );
    assert_eq!(flat(&d, pid), "PREFIX one two three shown SUFFIX");
}

// ── block, flex and grid containers ─────────────────────────────────────────
//
// In those the generated boxes are real Taffy children, not inline IFC
// members, so where an insertion attaches the child in the parent's Taffy
// list decides the layout (the inline fixtures above cannot see it). Each
// fixture is a differential (from the review of #1633): the same tree filled
// AFTER its parent was attached against filled BEFORE (generated boxes made
// last, in their right place), element box by element box.

const BOX_CSS: &str = "body { font-size: 16px; line-height: 20px; margin: 0 } \
  .c { width: 600px; margin: 0 } \
  .flex { display: flex } .grid { display: grid; grid-template-columns: repeat(6, 50px) } \
  .item { width: 40px; height: 10px; display: block } \
  .a::after { content: \"Z\"; display: block; width: 30px; height: 10px } \
  .b::before { content: \"Y\"; display: block; width: 20px; height: 10px }";

type Boxes = Vec<(bool, (f32, f32, f32, f32))>;

/// Every element box under `p` in DOM order (generated ones included).
fn boxes(d: &RinchDocument, p: u64, out: &mut Boxes) {
    for &c in &d.tree.get(p).unwrap().children {
        let n = d.tree.get(c).unwrap();
        if n.is_element() {
            let l = &n.layout;
            out.push((n.is_pseudo_element, (l.x, l.y, l.width, l.height)));
        }
        boxes(d, c, out);
    }
}

fn build_boxes(classes: &str, fill_first: bool) -> Boxes {
    let mut d = RinchDocument::new();
    d.load_css(BOX_CSS);
    let body = d.body();
    let p = d.create_element("div");
    d.set_attribute(p, "class", classes);
    let kids: Vec<NodeId> = (0..4)
        .map(|i| {
            let e = d.create_element("div");
            d.set_attribute(e, "class", "item");
            let t = d.create_text(&format!("w{i} "));
            d.append_child(e, t);
            e
        })
        .collect();
    if fill_first {
        for &k in &kids {
            d.append_child(p, k);
        }
        d.append_child(body, p);
        d.resolve_layout(800.0, 400.0);
    } else {
        d.append_child(body, p);
        d.resolve_layout(800.0, 400.0);
        for &k in &kids {
            d.append_child(p, k);
        }
        d.resolve_layout(801.0, 400.0);
        d.resolve_layout(800.0, 400.0);
    }
    let mut out = Vec::new();
    boxes(&d, p.0, &mut out);
    out
}

fn same_either_way(classes: &str) {
    assert_eq!(
        build_boxes(classes, true),
        build_boxes(classes, false),
        "{classes}: filled before attach vs after"
    );
}

#[test]
fn flex_after() {
    same_either_way("c flex a")
}

#[test]
fn flex_both() {
    same_either_way("c flex a b")
}

#[test]
fn grid_both() {
    same_either_way("c grid a b")
}

#[test]
fn block_both() {
    same_either_way("c a b")
}
