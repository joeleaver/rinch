//! Trailing white space that does not hang takes part in `text-align` (#1042).
//!
//! parley 0.11.1's `align` treats every line's trailing white space as
//! hanging: its free space is `width - advance + trailing_whitespace`. CSS
//! Text 3 §4.1.3 hangs a preserved space unconditionally only at a soft wrap;
//! at the end of the paragraph or before a forced break it hangs only the part
//! that does not fit ("conditionally hang"), and a no-break space never hangs.
//! Chrome 155 counts the rest as content when it aligns the line, and so does
//! rinch now (`ifc::align_unhung_trailing`, which narrows the line's alignment
//! width by what counts).
//!
//! Every expectation is Chrome 155's on the same markup: the bundled Inter as
//! `ProbeFace`, `16px/25px`, read as the x of the line's first glyph (or of a
//! zero-width `inline-block` marker before the text) from the container's
//! left edge. Not covered, each a separate gap: right-to-left lines (parley
//! offsets an RTL line by its trailing white space, so narrowing the width
//! would move it the wrong way; left as it was), a `nowrap`/`pre` root with no
//! wrapping text (#1237, aligned within its widest line), `break-spaces`
//! spaces that overflow (Chrome wraps them; rinch hangs them), and the
//! distribution of a justified line that ends at a soft wrap.

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

const W200: &str = "width:200px;font:16px/25px ProbeFace";

/// The issue's first rows: `abc` and three `pre-wrap` spaces end the
/// paragraph and fit, so they are content. Chrome 155: 158.58 right and end,
/// 79.28 centred (rinch hung them: 172.08 and 86.04).
#[test]
fn pre_wrap_spaces_that_fit_at_the_paragraphs_end_are_aligned_as_content() {
    for (align, want) in [("right", 158.58), ("end", 158.58), ("center", 79.28)] {
        let l = lines(
            &format!("{W200};white-space:pre-wrap;text-align:{align}"),
            "abc   ",
        );
        assert_eq!(l.len(), 1, "{l:?}");
        near(l[0].1, want);
    }
}

/// `break-spaces` spaces never hang: the same numbers.
#[test]
fn break_spaces_that_fit_are_aligned_as_content() {
    for (align, want) in [("right", 158.58), ("center", 79.28)] {
        let l = lines(
            &format!("{W200};white-space:break-spaces;text-align:{align}"),
            "abc   ",
        );
        near(l[0].1, want);
    }
}

/// Collapsible spaces at the paragraph's end are removed before alignment,
/// as before: Chrome 155 172.08 right, 86.03 centred.
#[test]
fn collapsible_trailing_spaces_are_still_removed() {
    for (align, want) in [("right", 172.08), ("center", 86.03)] {
        let l = lines(
            &format!("{W200};white-space:normal;text-align:{align}"),
            "abc   ",
        );
        assert_eq!(l[0].0, "abc", "{l:?}");
        assert!((l[0].1 - want).abs() < 0.02, "{l:?}, Chrome 155: {want}");
    }
}

/// Before a forced break — a preserved newline or a `<br>` — the spaces are
/// content too (CSS Text 3 conditionally hangs them, and they fit). Chrome
/// 155: `abc` at 158.58 and `longer line` at 120.88 right; 79.28 and 60.44
/// centred.
#[test]
fn spaces_that_fit_before_a_forced_break_are_aligned_as_content() {
    for inner in ["abc   \nlonger line", "abc   <br>longer line"] {
        let l = lines(
            &format!("{W200};white-space:pre-wrap;text-align:right"),
            inner,
        );
        assert_eq!(l.len(), 2, "{l:?}");
        near(l[0].1, 158.58);
        near(l[1].1, 120.88);
    }
    let l = lines(
        &format!("{W200};white-space:pre-wrap;text-align:center"),
        "abc   \nlonger line",
    );
    near(l[0].1, 79.28);
    near(l[1].1, 60.44);
}

/// Only the part that fits is content: `abc` and ten spaces in a 40px box
/// fill the line, the rest hangs, and the line starts at 0, right-aligned or
/// centred (Chrome 155; rinch hung all ten: 12.08 and 6.04). Off the fixed
/// point where all the spaces fit.
#[test]
fn spaces_that_overflow_at_the_paragraphs_end_hang_only_past_the_edge() {
    for align in ["right", "center"] {
        let l = lines(
            &format!("width:40px;font:16px/25px ProbeFace;white-space:pre-wrap;text-align:{align}"),
            "abc          ",
        );
        assert_eq!(l.len(), 1, "{l:?}");
        near(l[0].1, 0.0);
    }
}

/// At a soft wrap the spaces still hang whether they fit or not: Chrome 155
/// `abc` at 32.08 and `defgh` at 16.14 in a 60px box (unchanged).
#[test]
fn spaces_at_a_soft_wrap_still_hang() {
    let l = lines(
        "width:60px;font:16px/25px ProbeFace;white-space:pre-wrap;text-align:right",
        "abc   defgh",
    );
    assert_eq!(l.len(), 2, "{l:?}");
    near(l[0].1, 32.08);
    assert!((l[1].1 - 16.14).abs() < 0.02, "{l:?}");
}

/// The issue's shrink-to-fit rows: since #1018 the box counts the spaces, and
/// now the line does too, so `abc` starts at the box's start (Chrome 155: 0,
/// in a 41.42px box; rinch drew it 14.08 / 7.04 in, past its own box).
#[test]
fn a_shrink_to_fit_box_draws_its_spaces_inside_itself() {
    for align in ["right", "center"] {
        let (d, c) = lay_out(
            &format!("width:fit-content;white-space:pre-wrap;text-align:{align}"),
            "abc   ",
            true,
        );
        let l = read(&d, c);
        // The box is rounded down to 41 (Chrome: 41.42), and an auto-width
        // box's paint layout is broken at its width plus up to a pixel of
        // slack (`ifc_paint_max_width`), here 42: so the line ends at 42 and
        // starts 0.58 (right) or 0.29 (centred) in. It was a whole space and
        // more off.
        assert!(l[0].1 < 0.6, "{l:?}");
    }
}

/// The issue's inline-block row: the box is as wide as `longer line`, and
/// `abc` and its spaces before the forced break are right-aligned in it as one
/// piece. Chrome 155: `longer line` at 0 and `abc` at 37.70. Read as the
/// distance between the two lines, since rinch breaks an auto-width box's
/// lines at its rounded width plus up to a pixel of slack
/// (`ifc_paint_max_width`) and so puts `longer line` 0.88 in.
#[test]
fn an_inline_blocks_line_before_a_forced_break_counts_its_spaces() {
    let (d, c) = lay_out(
        "display:inline-block;white-space:pre-wrap;text-align:right",
        "abc   \nlonger line",
        false,
    );
    let l = read(&d, c);
    assert_eq!(l.len(), 2, "{l:?}");
    near(l[0].1 - l[1].1, 37.70);
}

/// A no-break space never hangs (the issue's comment): an `x` and a trailing
/// NBSP, and ` &nbsp; ` (whose collapsible spaces around the NBSP go), in a
/// 100px box. Chrome 155, x of a marker before `x`: right 86.77 and 82.27,
/// centred 43.38 and 41.13 (rinch: 91.27 / 45.63 for both).
#[test]
fn a_trailing_nbsp_is_aligned_as_content() {
    let s = "width:100px;font:16px/25px ProbeFace";
    near(marker_x(&format!("{s};text-align:right"), "x&nbsp;"), 86.77);
    near(
        marker_x(&format!("{s};text-align:right"), "x &nbsp; "),
        82.27,
    );
    near(
        marker_x(&format!("{s};text-align:center"), "x&nbsp;"),
        43.38,
    );
    near(
        marker_x(&format!("{s};text-align:center"), "x &nbsp; "),
        41.13,
    );
}

/// An NBSP before a soft wrap stays content while the collapsible space after
/// it hangs: Chrome 155 puts `aa&nbsp;` flush right in a 50px box, `aa` at
/// 27.53 (rinch: 32.03).
#[test]
fn an_nbsp_at_a_soft_wrap_is_content_and_the_space_after_it_hangs() {
    let l = lines(
        "width:50px;font:16px/25px ProbeFace;text-align:right",
        "aa&nbsp; bbbbbbb",
    );
    assert_eq!(l.len(), 2, "{l:?}");
    near(l[0].1, 27.53);
}

/// `pre-wrap`: an NBSP and two spaces that fit at the paragraph's end are all
/// content. Chrome 155: `x` at 77.77.
#[test]
fn a_pre_wrap_nbsp_and_spaces_at_the_end_are_content() {
    let l = lines(
        "width:100px;font:16px/25px ProbeFace;white-space:pre-wrap;text-align:right",
        "x&nbsp;  ",
    );
    near(l[0].1, 77.77);
}

/// Start alignment is untouched: the line starts at 0 with its spaces.
#[test]
fn start_aligned_lines_are_unchanged() {
    let l = lines(&format!("{W200};white-space:pre-wrap"), "abc   \nd");
    near(l[0].1, 0.0);
    near(l[1].1, 0.0);
}

/// A right-to-left paragraph is left as it was, not moved further from
/// Chrome. Chrome 155 centres the whole line, its trailing spaces (at its left
/// end) included: offset `(200 - advance) / 2`. rinch still hangs them, half a
/// run short of that (a separate gap: parley offsets an RTL line by its
/// trailing white space, so the fix here, a narrowed alignment width, would
/// move it a whole run short instead, and is not applied to RTL).
#[test]
fn a_right_to_left_line_is_not_narrowed() {
    let (d, c) = lay_out(
        &format!("{W200};white-space:pre-wrap;text-align:center;direction:rtl"),
        "\u{5d0}\u{5d1}\u{5d2}   ",
        false,
    );
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().unwrap();
    let m = *il.layout.lines().next().unwrap().metrics();
    assert!(m.hanging_advance > 10.0, "the spaces are on the line");
    let chrome = (200.0 - m.advance) / 2.0;
    near(m.offset, chrome - m.hanging_advance / 2.0);
}

/// Only the spaces at the very end of a line hang; spaces of another
/// `white-space` before them are content (#1243's second item, Chrome 153,
/// cases c7 and c19 of the #1228 review). c7: in a `pre-wrap` root, `pre`
/// spaces then `pre-wrap` spaces end the first line at a soft wrap; the
/// `pre-wrap` ones hang and the `pre` ones count, so the line overflows and
/// starts at 0 (rinch: 5.08). c19: in a `pre` root, `pre-wrap` spaces then
/// the root's own end the paragraph; all of them count, `cca` at 0.08 (rinch:
/// 9.08, narrowed by the `pre` spaces only).
#[test]
fn spaces_before_the_hanging_run_are_content() {
    let w = "width:50px;font:16px/25px ProbeFace";
    let l = lines(
        &format!("{w};white-space:pre-wrap;text-align:right"),
        "aaaaa<span style=\"white-space:pre\">  </span>   bbbb",
    );
    assert_eq!(l.len(), 2, "{l:?}");
    assert_eq!(l[0].0.trim(), "aaaaa");
    near(l[0].1, 0.0);
    let l = lines(
        &format!("{w};white-space:pre;text-align:right"),
        "<span style=\"white-space:normal\">bb cc</span><span style=\"white-space:pre-wrap\">a  </span>   ",
    );
    assert_eq!(l.len(), 2, "{l:?}");
    assert_eq!(l[1].0.trim(), "cca");
    near(l[1].1, 0.08);
}
