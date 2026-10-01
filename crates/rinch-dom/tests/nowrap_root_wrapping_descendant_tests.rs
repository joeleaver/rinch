//! A `white-space: nowrap` / `pre` IFC root still wraps the text of a
//! descendant whose own `white-space` allows wrapping (#1212).
//!
//! CSS Text 3 §5 decides soft wrap opportunities per character: at a boundary
//! the nearest common ancestor's `text-wrap-mode` applies. rinch broke a
//! `nowrap`/`pre` root's whole paragraph unconstrained, so a `normal`,
//! `pre-wrap` or `pre-line` span inside one never wrapped. The root's own wrap
//! mode is now the root text style's `TextWrapMode`, and the paragraph is
//! broken at the container's width whenever some text in it may wrap.
//!
//! Every expectation is Chrome 153's on the same markup (bundled Inter as
//! `ProbeFace`, 16px/25px, a `width: 50px` block), read line by line. Note the
//! first line, `aaa bbb`: there is no break between the root's space and
//! `bbb`, because their common ancestor is the non-wrapping root — so that line
//! overflows the 50px box (it is 60.84px), exactly as in Chrome.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d
}

/// `<div style="width:50px;{style}">{inner}</div>`, laid out; returns the
/// div's lines (each trimmed at both ends, since a space at a soft wrap is
/// not ink and Chrome's per-character rects put the one in
/// `the_roots_own_text_stays_unwrapped_beside_a_wrapping_span` at the start
/// of the second line) and its height.
fn lay_out(style: &str, inner: &str) -> (Vec<String>, f32) {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        &format!("width:50px;font:16px/25px ProbeFace;{style}"),
    );
    d.append_child(body, c);
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    read(&d, c)
}

fn read(d: &RinchDocument, c: NodeId) -> (Vec<String>, f32) {
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("the div is an IFC root");
    let lines = il
        .layout
        .lines()
        .map(|l| il.text_content[l.text_range()].trim().to_string())
        .collect();
    (lines, n.layout.height)
}

const SPANS: &str = "aaa <span style=\"white-space:normal\">bbb ccc ddd</span>";

/// The issue's table: a `normal` span in a `pre` root, and in a `nowrap` one.
#[test]
fn a_wrapping_span_wraps_inside_a_nowrap_or_pre_root() {
    for root in ["white-space:pre", "white-space:nowrap"] {
        let (lines, h) = lay_out(root, SPANS);
        assert_eq!(
            lines,
            ["aaa bbb", "ccc", "ddd"],
            "{root}: Chrome 153 wraps the span's text, and not between the root's space and it"
        );
        assert_eq!(h, 75.0, "{root}: three 25px lines");
    }
}

/// `pre-wrap` and `pre-line` spans wrap too: wrapping is `text-wrap-mode`,
/// not the collapse half of `white-space`.
#[test]
fn pre_wrap_and_pre_line_spans_wrap_inside_a_pre_root() {
    for span in ["white-space:pre-wrap", "white-space:pre-line"] {
        let (lines, h) = lay_out(
            "white-space:pre",
            &format!("aaa <span style=\"{span}\">bbb ccc ddd</span>"),
        );
        assert_eq!(lines, ["aaa bbb", "ccc", "ddd"], "{span}");
        assert_eq!(h, 75.0, "{span}");
    }
}

/// The root's own text does not wrap, even when a span in the same paragraph
/// does: `aaa bbb ccc` belongs to the `pre` root and stays one line, then the
/// span wraps after it.
#[test]
fn the_roots_own_text_stays_unwrapped_beside_a_wrapping_span() {
    let (lines, h) = lay_out(
        "white-space:pre",
        "aaa bbb ccc<span style=\"white-space:normal\"> ddd eee</span>",
    );
    assert_eq!(lines, ["aaa bbb ccc", "ddd", "eee"]);
    assert_eq!(h, 75.0);
}

/// Control: with no wrapping descendant a `nowrap` root is still one line.
#[test]
fn an_all_nowrap_root_is_still_one_line() {
    let (lines, h) = lay_out(
        "white-space:nowrap",
        "aaa <span style=\"color:red\">bbb ccc ddd</span>",
    );
    assert_eq!(lines, ["aaa bbb ccc ddd"]);
    assert_eq!(h, 25.0);
}

/// A `nowrap` root clipped with `text-overflow: ellipsis` whose span wraps is
/// a wrapping paragraph, not one line to cut whole: when the flat rebuild is
/// not faithful (a coloured span) it keeps its three lines (#1091's "clipped,
/// no …" route) rather than collapsing them to one cut prefix.
#[test]
fn an_unfaithful_ellipsis_root_with_a_wrapping_span_keeps_its_lines() {
    let (lines, h) = lay_out(
        "white-space:nowrap;overflow:hidden;text-overflow:ellipsis",
        "aaa <span style=\"white-space:normal;color:red\">bbb ccc ddd</span>",
    );
    assert_eq!(lines, ["aaa bbb", "ccc", "ddd"]);
    assert_eq!(h, 75.0);
}
