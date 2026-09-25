//! Layout's clamp of a scrolled container (`clamp_scroll_offsets`) measures
//! the range **this** pass laid out (review of PR #1045, round 2).
//!
//! It asks `content_extents`, which reads an IFC root's (and an anonymous
//! box's) `text_layout` — built by `build_ifc_layouts` and
//! `copy_cached_text_layouts` later in `resolve_layout`. Run before them, the
//! clamp measured last pass's lines (or none, after an invalidation): a
//! bottom-pinned text scroller snapped from its bottom to 0 when text was
//! appended, and widening it left an offset past the new end.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "* { box-sizing: border-box; border-width: 0; } body { margin: 0; font-size: 16px; line-height: 20px; }";

fn words(n: usize) -> String {
    vec!["ab"; n].join(" ")
}

/// A 200x100 scroller (width given) holding `text`, optionally inside a
/// `display: contents` span. Returns (doc, scroller, text node).
fn text_scroller(
    width: &str,
    wrap_in_contents: bool,
    text: &str,
) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let s = doc.create_element("div");
    doc.set_attribute(
        s,
        "style",
        &format!("width: {width}; height: 100px; overflow: auto"),
    );
    doc.append_child(body, s);
    let t = doc.create_text(text);
    if wrap_in_contents {
        let w = doc.create_element("span");
        doc.set_attribute(w, "style", "display: contents");
        doc.append_child(s, w);
        doc.append_child(w, t);
    } else {
        doc.append_child(s, t);
    }
    doc.resolve_layout(800.0, 600.0);
    (doc, s, t)
}

fn report(label: &str, doc: &RinchDocument, s: NodeId, before: f64) -> (f64, f64) {
    let top = doc.scroll_top(s);
    let max = doc.scroll_height(s) - 100.0;
    eprintln!("{label}: offset before {before}, after layout {top}, valid max now {max}");
    (top, max)
}

#[test]
fn clamp_after_text_append_direct() {
    let (mut doc, s, t) = text_scroller("200px", false, &words(80));
    let max0 = doc.scroll_height(s) - 100.0;
    assert!(max0 > 50.0, "positive control: text overflows, max {max0}");
    doc.set_scroll_top(s, max0);
    assert_eq!(doc.scroll_top(s), max0);
    // Append more text: the range grows, the offset is still legal.
    doc.set_text_content(t, &words(120));
    doc.resolve_layout(800.0, 600.0);
    let (top, max) = report("direct text, append", &doc, s, max0);
    assert!(max >= max0);
    assert_eq!(top, max0, "a legal offset was clamped");
}

#[test]
fn clamp_after_text_append_contents_wrapped() {
    let (mut doc, s, t) = text_scroller("200px", true, &words(80));
    let max0 = doc.scroll_height(s) - 100.0;
    assert!(max0 > 50.0, "positive control: text overflows, max {max0}");
    doc.set_scroll_top(s, max0);
    doc.set_text_content(t, &words(120));
    doc.resolve_layout(800.0, 600.0);
    let (top, max) = report("contents-wrapped text, append", &doc, s, max0);
    assert!(max >= max0);
    assert_eq!(top, max0, "a legal offset was clamped");
}

#[test]
fn clamp_after_resize_narrower() {
    // width 25% of the viewport: 200px at 800, 150px at 600.
    let (mut doc, s, _t) = text_scroller("25%", false, &words(80));
    let max0 = doc.scroll_height(s) - 100.0;
    assert!(max0 > 50.0);
    doc.set_scroll_top(s, max0);
    doc.resolve_layout(600.0, 600.0);
    let (top, max) = report("direct text, narrower", &doc, s, max0);
    assert!(
        max > max0,
        "narrower re-wraps to more lines: {max} vs {max0}"
    );
    assert_eq!(top, max0, "a legal offset was clamped by last pass's lines");
}

#[test]
fn clamp_after_resize_wider_leaves_no_illegal_offset() {
    let (mut doc, s, _t) = text_scroller("25%", false, &words(80));
    let max0 = doc.scroll_height(s) - 100.0;
    doc.set_scroll_top(s, max0);
    doc.resolve_layout(1200.0, 600.0);
    let (top, max) = report("direct text, wider", &doc, s, max0);
    assert!(max < max0);
    assert!(
        top <= max + 1e-6,
        "offset {top} left beyond the range {max}"
    );
}

#[test]
fn clamp_mixed_after_text_append_in_run() {
    // #995's shape: text, block, text; the trailing run grows.
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let s = doc.create_element("div");
    doc.set_attribute(s, "style", "width: 200px; height: 100px; overflow: auto");
    doc.append_child(body, s);
    let a = doc.create_text(&words(10));
    doc.append_child(s, a);
    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "height: 50px");
    doc.append_child(s, b);
    let c = doc.create_text(&words(40));
    doc.append_child(s, c);
    doc.resolve_layout(800.0, 600.0);
    let max0 = doc.scroll_height(s) - 100.0;
    assert!(max0 > 50.0);
    doc.set_scroll_top(s, max0);
    doc.set_text_content(c, &words(80));
    doc.resolve_layout(800.0, 600.0);
    let (top, max) = report("mixed, trailing run grows", &doc, s, max0);
    assert!(max >= max0);
    assert_eq!(top, max0);
}

#[test]
fn clamp_sticky_header_unrelated_layout() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let s = doc.create_element("div");
    doc.set_attribute(s, "style", "width: 200px; height: 100px; overflow: auto");
    doc.append_child(body, s);
    let h = doc.create_element("div");
    doc.set_attribute(h, "style", "position: sticky; top: 0; height: 30px");
    doc.append_child(s, h);
    for _ in 0..10 {
        let r = doc.create_element("div");
        doc.set_attribute(r, "style", "height: 37px");
        doc.append_child(s, r);
    }
    let sib = doc.create_element("div");
    doc.set_attribute(sib, "style", "width: 10px; height: 10px");
    doc.append_child(body, sib);
    doc.resolve_layout(800.0, 600.0);
    let max0 = doc.scroll_height(s) - 100.0;
    assert_eq!(max0, 30.0 + 370.0 - 100.0);
    doc.set_scroll_top(s, max0);
    doc.set_style(sib, "width", "20px");
    doc.resolve_layout(800.0, 600.0);
    let (top, _) = report("sticky header, unrelated layout", &doc, s, max0);
    assert_eq!(top, max0);
}
