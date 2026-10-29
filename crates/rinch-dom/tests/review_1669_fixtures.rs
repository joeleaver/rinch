//! Review fixtures for PR #1669 (#1042).
#![allow(dead_code)]
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

/// `<div style="font:16px/25px ProbeFace;{style}">{inner}</div>` in the body,
/// or, with `column`, inside a `flex-start` flex column 300px wide (where a
/// `fit-content` box is as wide as its line).
fn lay_out(style: &str, inner: &str, column: bool) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    let parent = if column {
        let w = d.create_element("div");
        d.set_attribute(
            w,
            "style",
            "display:flex;flex-direction:column;align-items:flex-start;width:300px",
        );
        d.append_child(body, w);
        w
    } else {
        body
    };
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("font:16px/25px ProbeFace;{style}"));
    d.append_child(parent, c);
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

/// Each line's text and alignment offset.
fn lines(style: &str, inner: &str) -> Vec<(String, f32)> {
    let (d, c) = lay_out(style, inner, false);
    read(&d, c)
}

fn read(d: &RinchDocument, c: NodeId) -> Vec<(String, f32)> {
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("the div is an IFC root");
    il.layout
        .lines()
        .map(|l| {
            (
                il.text_content[l.text_range()].to_string(),
                l.metrics().offset,
            )
        })
        .collect()
}

/// The x of the zero-width `inline-block` marker the text starts with.
fn marker_x(style: &str, text: &str) -> f32 {
    let (d, c) = lay_out(
        style,
        &format!("<b style=\"display:inline-block;width:0;height:10px\"></b>{text}"),
        false,
    );
    let n = d.tree.get(c.0).unwrap();
    let m = n
        .children
        .iter()
        .copied()
        .find(|&k| d.tree.get(k).unwrap().is_element())
        .expect("the marker");
    d.tree.get(m).unwrap().layout.x
}

#[track_caller]
fn near(got: f32, want: f32) {
    assert!((got - want).abs() < 0.01, "got {got}, Chrome 155: {want}");
}

const W: &str = "width:200px;font:16px/25px ProbeFace;";

/// Review of #1669, finding 1: parley's `trailing_whitespace` is the trailing
/// white space of the line's LAST shaped run only, but the narrowing sums every
/// trailing white-space unit. Trailing white space split across runs (a tab is
/// its own span; a font-size or letter-spacing change) is over-narrowed at the
/// end of a paragraph. Chrome 155, bundled Inter.
#[test]
fn trailing_white_space_across_shaped_runs_is_counted_once() {
    for (style, html, want) in [
        ("white-space:pre-wrap;text-align:right", "abc\t\t", 128.0),
        ("white-space:pre-wrap;text-align:right", "abc \t", 164.0),
        (
            "white-space:pre-wrap;text-align:right",
            "abc <span style='letter-spacing:10px'>  </span>",
            138.58,
        ),
        (
            "white-space:pre-wrap;text-align:right",
            "abc <span style='font-size:32px'> </span>",
            158.58,
        ),
        // Controls that already match: one run, or before a forced break.
        ("white-space:pre-wrap;text-align:right", "abc\t", 164.0),
        (
            "white-space:pre-wrap;text-align:right",
            "abc\t\t<br>x",
            128.0,
        ),
        (
            "white-space:pre-wrap;text-align:right",
            "abc <span style='color:red'>  </span>",
            158.58,
        ),
    ] {
        let got = marker_x(&format!("{W}{style}"), html);
        assert!(
            (got - want).abs() < 0.05,
            "{html:?}: got {got}, Chrome 155: {want}"
        );
    }
}

/// Review of #1669, finding 2: rinch hands parley no CSS `direction`, so a
/// `direction: rtl` paragraph of Latin text is laid out left-to-right and the
/// `is_rtl()` guard does not see it. Chrome 155: 172.08 (the spaces sit at
/// the line's visual left, inside the box). Base 0b8fc2ce matched; the PR
/// moves it to 158.58.
#[test]
fn a_direction_rtl_paragraph_of_latin_text_is_not_narrowed() {
    use rinch_dom::text_query::glyph_bounds_for_offset_layout;
    let (d, c) = lay_out(
        &format!("{W}direction:rtl;white-space:pre-wrap;text-align:right"),
        "abc   ",
        false,
    );
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let x = glyph_bounds_for_offset_layout(&il.layout, 0).unwrap().x;
    assert!((x - 172.08).abs() < 0.05, "got {x}, Chrome 155: 172.08");
}

/// Review of #1669, finding 3, since parley 0.12: no alignment re-breaks
/// the paragraph for its trailing spaces any more — parley hangs them only
/// as far as they overflow itself, so the `right` and `center` lines that
/// used to pay one pass for it (#1042) pay none. (The positions are
/// `trailing_white_space_across_shaped_runs_is_counted_once`'s.)
#[test]
fn no_alignment_pays_a_pass_for_its_trailing_spaces() {
    use rinch_dom::perf::Counter;
    for (align, want) in [("left", 0), ("justify", 0), ("right", 0), ("center", 0)] {
        let mut d = doc();
        let body = d.body();
        let c = d.create_element("div");
        d.set_attribute(
            c,
            "style",
            &format!(
                "width:200px;font:16px/25px ProbeFace;white-space:pre-wrap;text-align:{align}"
            ),
        );
        d.append_child(body, c);
        let t = d.create_text("abc   ");
        d.append_child(c, t);
        d.resolve_layout(800.0, 600.0);
        d.tree.perf.reset();
        d.set_attribute(
            c,
            "style",
            &format!(
                "width:190px;font:16px/25px ProbeFace;white-space:pre-wrap;text-align:{align}"
            ),
        );
        d.resolve_layout(800.0, 600.0);
        let passes = d.tree.perf.get(Counter::IfcHangPasses);
        assert!(
            (passes > 0) == (want > 0),
            "{align}: {passes} hang passes for one relayout"
        );
    }
}

/// Review of #1669, finding 4: a flex item's own text (a text leaf,
/// `shape_text_leaf`) counts its trailing spaces too. Chrome 155: 158.58.
/// Ignored until parley 0.12, which hangs them only as far as they overflow
/// itself (#1669 covered IFCs only).
#[test]
fn a_flex_items_own_text_counts_its_trailing_spaces() {
    let (d, c) = lay_out(
        &format!("{W}display:flex;flex-direction:column;white-space:pre-wrap;text-align:right"),
        "abc   ",
        false,
    );
    let n = d.tree.get(c.0).unwrap();
    let leaf = n
        .children
        .iter()
        .find_map(|&k| d.tree.get(k).unwrap().cached_text_parley.as_ref())
        .expect("a text leaf");
    let off = leaf.lines().next().unwrap().metrics().offset;
    assert!((off - 158.58).abs() < 0.05, "got {off}, Chrome 155: 158.58");
}
