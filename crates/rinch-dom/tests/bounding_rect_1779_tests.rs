//! `NodeHandle::bounding_rect()` on rinch-dom (issue #1779): where an element
//! is painted, synchronously, in the viewport frame `ClickContext` and the
//! bounds signals use — not the parent-relative `get_layout_bounds`.
//!
//! The fixtures sit off the fixed points a wrong answer would share with the
//! right one: the element is offset inside its parent (so a parent-relative
//! answer differs), under a `scale(2)` ancestor (so an untransformed sum
//! differs), and inside a scrolled container (so a sum that ignores scroll
//! differs). Those boxes are sized by declaration. The inline-element
//! fixtures below measure text, against Chrome 153 with the bundled Inter.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId};
use rinch_core::reactive::ElementBounds;
use rinch_dom::RinchDocument;

const CSS: &str = "body { margin: 0 } div { margin: 0; padding: 0 } \
    .box { position: absolute; left: 600px; top: 300px; width: 100px; height: 100px } \
    .outer { position: absolute; left: 30px; top: 40px; width: 200px; height: 200px; \
             transform: scale(2); transform-origin: 0 0 } \
    .inner { margin-left: 10px; margin-top: 5px; width: 50px; height: 20px } \
    .scroller { position: absolute; left: 400px; top: 0; width: 100px; height: 100px; \
                overflow: auto } \
    .tall { height: 1000px } \
    .row { margin-top: 300px; width: 60px; height: 10px } \
    .gone { display: none } \
    .contents { display: contents }";

struct Fixture {
    doc: Rc<RefCell<RinchDocument>>,
    inner: NodeId,
    row: NodeId,
    scroller: NodeId,
    hidden_child: NodeId,
    contents: NodeId,
    text: NodeId,
}

fn el(doc: &mut RinchDocument, parent: NodeId, class: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "class", class);
    doc.append_child(parent, n);
    n
}

fn fixture() -> Fixture {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let outer = el(&mut doc, body, "outer");
    let inner = el(&mut doc, outer, "inner");
    let scroller = el(&mut doc, body, "scroller");
    let tall = el(&mut doc, scroller, "tall");
    let row = el(&mut doc, tall, "row");
    // Inside a box away from the origin: a box-less element's zeroed layout,
    // summed up the chain, would otherwise answer zero by sitting at (0, 0).
    let boxed = el(&mut doc, body, "box");
    let gone = el(&mut doc, boxed, "gone");
    let hidden_child = el(&mut doc, gone, "inner");
    let contents = el(&mut doc, boxed, "contents");
    el(&mut doc, contents, "inner");
    let text = doc.create_text("t");
    doc.append_child(inner, text);
    doc.resolve_layout(800.0, 600.0);
    Fixture {
        doc: Rc::new(RefCell::new(doc)),
        inner,
        row,
        scroller,
        hidden_child,
        contents,
        text,
    }
}

fn handle(f: &Fixture, id: NodeId) -> NodeHandle {
    let doc: Rc<RefCell<dyn DomDocument>> = f.doc.clone();
    NodeHandle::new(id, Rc::downgrade(&doc))
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> ElementBounds {
    ElementBounds {
        x,
        y,
        width,
        height,
    }
}

/// The painted box under a `scale(2)` ancestor: 30 + 2·10 across, 40 + 2·5
/// down, twice the size. `get_layout_bounds` answers (10, 5, 50, 20) for the
/// same node.
#[test]
fn the_rect_is_the_painted_box_under_a_transformed_ancestor() {
    let f = fixture();
    let h = handle(&f, f.inner);
    assert_eq!(h.get_layout_bounds(), Some((10.0, 5.0, 50.0, 20.0)));
    assert_eq!(h.bounding_rect(), Some(rect(50.0, 50.0, 100.0, 40.0)));
}

/// It is the same box the bounds signals and `ClickContext` report.
#[test]
fn the_rect_is_the_painted_border_box() {
    let f = fixture();
    let r = rinch_dom::paint::painted_border_box(&f.doc.borrow().tree, f.inner.0, 1.0);
    let b = handle(&f, f.inner).bounding_rect().unwrap();
    assert_eq!(
        (b.x, b.y, b.width, b.height),
        (
            r.x0 as f32,
            r.y0 as f32,
            r.width() as f32,
            r.height() as f32
        )
    );
}

/// A scroll moves the answer with no layout in between: it is where the
/// element is now, not where it was laid out.
#[test]
fn a_scrolled_container_moves_the_rect() {
    let f = fixture();
    let row = handle(&f, f.row);
    assert_eq!(row.bounding_rect(), Some(rect(400.0, 300.0, 60.0, 10.0)));
    handle(&f, f.scroller).set_scroll_top(120.0);
    assert_eq!(row.bounding_rect(), Some(rect(400.0, 180.0, 60.0, 10.0)));
}

/// An element that generates no box answers a zero rect, as Chrome's
/// `getBoundingClientRect` does: under a `display: none` ancestor, or
/// `display: contents` itself.
#[test]
fn an_element_with_no_box_answers_a_zero_rect() {
    let f = fixture();
    assert_eq!(
        handle(&f, f.hidden_child).bounding_rect(),
        Some(ElementBounds::default())
    );
    assert_eq!(
        handle(&f, f.contents).bounding_rect(),
        Some(ElementBounds::default())
    );
}

/// A text node, a detached element and an unknown id answer `None`; the
/// element answers again once it is put back.
#[test]
fn a_text_node_or_a_node_out_of_the_document_answers_none() {
    let f = fixture();
    assert_eq!(handle(&f, f.text).bounding_rect(), None);
    assert_eq!(handle(&f, NodeId(999_999)).bounding_rect(), None);
    let inner = handle(&f, f.inner);
    let parent = inner.parent_node().unwrap();
    inner.remove();
    assert_eq!(inner.bounding_rect(), None);
    parent.append_child(&inner);
    assert!(inner.bounding_rect().is_some());
}

// ── a flowed inline element: the union of its line fragments ──────────────
//
// A `<span>`/`<a>` owns no box (its layout is a 0x0 ghost); `getBoundingClientRect`
// answers the union of its fragments. Every expectation is Chrome 153's
// (`--headless=new`, standards mode, `* { box-sizing: border-box }`, 800x600),
// with this crate's bundled Inter loaded through `@font-face` as `ProbeFace`.
// Inter at 16px has a rounded ascent + descent of 20px, the declared
// line-height, so each fragment is its line box.

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const C: &str = "position: absolute; left: 100px; top: 50px; width: 400px; \
                 font: 16px/20px ProbeFace; padding: 7px 0 0 11px; margin: 0";

fn inline_case(extra: &str, inner: &str) -> (Rc<RefCell<RinchDocument>>, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, &format!(r#"<div style="{C}; {extra}">{inner}</div>"#));
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    let s = *rinch_dom::testing::query_selector(&doc.tree, "[id=s]")
        .first()
        .expect("[id=s]");
    (Rc::new(RefCell::new(doc)), NodeId(s))
}

fn inline_rect(extra: &str, inner: &str) -> ElementBounds {
    let (doc, s) = inline_case(extra, inner);
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    NodeHandle::new(s, Rc::downgrade(&dyn_doc))
        .bounding_rect()
        .expect("a laid-out inline has a rect")
}

fn assert_close(got: ElementBounds, want: (f32, f32, f32, f32)) {
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    assert!(
        near(got.x, want.0)
            && near(got.y, want.1)
            && near(got.width, want.2)
            && near(got.height, want.3),
        "got {got:?}, Chrome 153 {want:?}"
    );
}

/// `aaaa <span>bbbb</span> cc` — the span's one fragment.
#[test]
fn an_inline_span_answers_its_fragment() {
    let r = inline_rect("", r#"aaaa <span id="s">bbbb</span> cc"#);
    assert_close(r, (151.4375, 57.0, 39.1875, 20.0));
}

/// A span wrapped over two lines: the box around both fragments, from the
/// second line's start to the widest right edge.
#[test]
fn a_wrapped_inline_span_answers_the_union_of_its_fragments() {
    let r = inline_rect(
        "width: 120px",
        r#"aaaa aaaa <span id="s">bbbb cccc dddd eeee</span> ff"#,
    );
    assert_close(r, (111.0, 77.0, 81.0, 40.0));
}

/// Under a `scale(2)` on the block holding the line, the painted union.
#[test]
fn an_inline_span_under_a_transform_answers_its_painted_union() {
    let r = inline_rect(
        "transform: scale(2); transform-origin: 0 0",
        r#"aaaa <span id="s">bbbb</span> cc"#,
    );
    assert_close(r, (202.875, 64.0, 78.375, 40.0));
}

/// An `<a>` holding a nested `<strong>`: its whole content.
#[test]
fn an_inline_link_answers_its_whole_content() {
    let r = inline_rect(
        "",
        r##"aaaa <a id="s" href="#">bb<strong>cc</strong>dd</a> ee"##,
    );
    assert_close(r, (151.4375, 57.0, 57.46875, 20.0));
}

/// A span on the third line of a block that scrolls its own lines moves up by
/// the scroll, with no layout in between.
#[test]
fn an_inline_span_in_a_scrolled_block_moves_with_the_scroll() {
    let (doc, s) = inline_case(
        "width: 60px; height: 27px; overflow: auto",
        r#"aaaa aaaa <span id="s">bbbb</span>"#,
    );
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let span = NodeHandle::new(s, Rc::downgrade(&dyn_doc));
    let before = span.bounding_rect().unwrap();
    span.parent_node().unwrap().set_scroll_top(13.0);
    let after = span.bounding_rect().unwrap();
    assert_eq!(before.y, 97.0, "third line: 50 + 7 + 2 x 20");
    assert_eq!((after.x, after.y), (before.x, before.y - 13.0));
}

/// A span that starts mid-line and ends on a shorter next line: the union
/// runs from that line's start to the first fragment's right edge — neither
/// the first fragment's left nor the last one's right.
#[test]
fn a_span_wrapped_from_mid_line_answers_the_union_not_its_ends() {
    let r = inline_rect(
        "width: 120px",
        r#"aaaa <span id="s">bbbbbb cc</span> ff"#,
    );
    assert_close(r, (111.0, 57.0, 99.21875, 40.0));
}
