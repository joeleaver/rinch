//! #2038: a `position: fixed` box against a later in-flow `overflow: auto`
//! sibling. CSS 2 Appendix E puts positioned `z-index: auto`/`0` boxes at step
//! 8, after every in-flow block whatever the tree order, and among themselves
//! in tree order; a scroller with neither `position` nor `z-index` is no
//! stacking context. Every expected answer below is Chrome 155's
//! `document.elementFromPoint(86, 118)` on the same markup, scroller scrolled
//! by 40px (measured headless).

use super::hit_test;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FIXED: &str = "position: fixed; top: 90px; left: 0; width: 800px; height: 70px; \
                     overflow: hidden; background: rgb(255, 0, 0)";
const SCROLLER: &str = "width: 500px; height: 226px; padding: 13px; overflow-y: auto; \
                        background: rgb(0, 0, 255)";
const CONTENT: &str = "height: 2000px; background: rgb(0, 255, 0)";

struct Case {
    doc: RinchDocument,
    fixed: NodeId,
    scroller: NodeId,
    content: NodeId,
}

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

/// `wrap > [fixed, scroller > content]` (or the scroller first), the
/// scroller scrolled 40px.
fn case(fixed_first: bool, fixed_extra: &str, scroller_extra: &str, content_extra: &str) -> Case {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
    let body = doc.body();
    let wrap = el(&mut doc, body, "width: 800px; height: 600px");
    let fixed_style = format!("{FIXED}; {fixed_extra}");
    let scroller_style = format!("{SCROLLER}; {scroller_extra}");
    let content_style = format!("{CONTENT}; {content_extra}");
    let (fixed, scroller) = if fixed_first {
        let f = el(&mut doc, wrap, &fixed_style);
        (f, el(&mut doc, wrap, &scroller_style))
    } else {
        let s = el(&mut doc, wrap, &scroller_style);
        (el(&mut doc, wrap, &fixed_style), s)
    };
    let content = el(&mut doc, scroller, &content_style);
    doc.resolve_layout(800.0, 600.0);
    doc.set_scroll_top(scroller, 40.0);
    doc.resolve_layout(800.0, 600.0);
    Case {
        doc,
        fixed,
        scroller,
        content,
    }
}

/// The software painter's pixel at (86, 118).
fn painted(c: &mut Case) -> [u8; 3] {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    let mut painter = TinySkiaPainter::new(800, 600);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &c.doc.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut c.doc.font_cx,
        &mut layout_cx,
    );
    let i = ((118 * painter.width() + 86) * 4) as usize;
    let d = painter.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];

fn check(mut c: Case, want: NodeId, name: &str) {
    let hit = hit_test(&c.doc.tree, 86.0, 118.0);
    let px = painted(&mut c);
    let want_px = if want == c.fixed { RED } else { GREEN };
    eprintln!(
        "{name}: hit {hit:?} (fixed {}, scroller {}, content {}), pixel {px:?}",
        c.fixed.0, c.scroller.0, c.content.0
    );
    assert_eq!(hit, Some(want.0), "{name}: hit");
    assert_eq!(px, want_px, "{name}: paint");
}

#[test]
fn issue_case_fixed_before_scroller() {
    let c = case(true, "", "", "");
    let f = c.fixed;
    check(c, f, "issue");
}

#[test]
fn scroller_before_fixed() {
    let c = case(false, "", "", "");
    let f = c.fixed;
    check(c, f, "scroller_first");
}

#[test]
fn later_z0_positioned_content_is_on_top_of_earlier_fixed() {
    let c = case(true, "", "", "position: relative; z-index: 0");
    let w = c.content;
    check(c, w, "z0_content");
}

#[test]
fn later_zauto_positioned_content_is_on_top_of_earlier_fixed() {
    let c = case(true, "", "", "position: relative");
    let w = c.content;
    check(c, w, "zauto_content");
}

#[test]
fn earlier_z0_positioned_content_is_under_later_fixed() {
    let c = case(false, "", "", "position: relative; z-index: 0");
    let f = c.fixed;
    check(c, f, "z0_content_scroller_first");
}

#[test]
fn positioned_scroller_later_is_on_top() {
    let c = case(true, "", "position: relative", "");
    let w = c.content;
    check(c, w, "positioned_scroller");
}

#[test]
fn negative_z_fixed_is_under_in_flow_content() {
    let c = case(true, "z-index: -1", "", "");
    let w = c.content;
    check(c, w, "zneg_fixed");
}
