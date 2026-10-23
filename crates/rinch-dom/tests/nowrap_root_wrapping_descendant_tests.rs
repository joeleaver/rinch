//! A `white-space: nowrap` / `pre` IFC root still wraps the text of a
//! descendant whose own `white-space` allows wrapping (#1212).
//!
//! CSS Text 3 §5 decides soft wrap opportunities per character. rinch broke a
//! `nowrap`/`pre` root's whole paragraph unconstrained, so a `normal`,
//! `pre-wrap` or `pre-line` span inside one never wrapped. The root's own wrap
//! mode is now the root text style's `TextWrapMode`, and the paragraph is
//! broken at the container's width whenever some text in it may wrap. Parley
//! takes each break opportunity's wrap mode from the character before it,
//! which is Chrome's answer at every seam pinned here (it is not Chrome's
//! after a `normal` span followed by the root's preserved spaces, where
//! Chrome breaks before the spaces).
//!
//! Every expectation is Chrome 153's on the same markup (bundled Inter as
//! `ProbeFace`, 16px/25px, a `width: 50px` block), read line by line. Note the
//! first line, `aaa bbb`: there is no break between the root's space and
//! `bbb`, because the space before the opportunity is the non-wrapping
//! root's — so that line overflows the 50px box (it is 60.84px), exactly as
//! in Chrome.

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
/// a wrapping paragraph, not one line to cut whole: it keeps its three lines
/// rather than collapsing them to one cut prefix (the cut is made per line at
/// paint, #1100).
#[test]
fn an_unfaithful_ellipsis_root_with_a_wrapping_span_keeps_its_lines() {
    let (lines, h) = lay_out(
        "white-space:nowrap;overflow:hidden;text-overflow:ellipsis",
        "aaa <span style=\"white-space:normal;color:red\">bbb ccc ddd</span>",
    );
    assert_eq!(lines, ["aaa bbb", "ccc", "ddd"]);
    assert_eq!(h, 75.0);
}

/// `(line text, offset)` of each line of `#c`'s layout.
fn offsets(style: &str, inner: &str) -> Vec<(String, f32)> {
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
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("the div is an IFC root");
    il.layout
        .lines()
        .map(|l| {
            (
                il.text_content[l.text_range()].trim().to_string(),
                l.metrics().offset,
            )
        })
        .collect()
}

const SPACES: &str = "                    ";

/// A `pre` root's preserved spaces do not hang (CSS Text 3 §4.1.3 hangs
/// only `pre-wrap`'s): `aaa` and twenty spaces overflow a 50px right-aligned
/// box, and an overflowing line starts at the start edge. Chrome 153: offset
/// 0. With no wrapping text the root is broken unconstrained; broken at the
/// box's width instead, the line keeps offset 0 only through the alignment
/// width `ifc::align_unhung_trailing` narrows.
#[test]
fn a_pre_roots_trailing_spaces_do_not_hang() {
    let lines = offsets("white-space:pre;text-align:right", &format!("aaa{SPACES}"));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].1, 0.0, "{lines:?}");
}

/// The same spaces after a wrapping span: the paragraph is now broken at the
/// box's width, and the root's own spaces still do not hang — Chrome 153
/// puts `ccaaa` and its spaces at offset 0, overflowing, after `bb`.
#[test]
fn a_pre_roots_trailing_spaces_do_not_hang_beside_a_wrapping_span() {
    let lines = offsets(
        "white-space:pre;text-align:right",
        &format!("<span style=\"white-space:normal\">bb cc</span>aaa{SPACES}"),
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0].0, "bb");
    assert_eq!(lines[1].0, "ccaaa");
    assert_eq!(lines[1].1, 0.0, "{lines:?}");
}

/// Why a paragraph with no wrapping text is still broken unconstrained:
/// parley marks a break opportunity after every inline box whatever the wrap
/// mode, so a `nowrap` root broken at 50px would wrap `bbb ccc` after the
/// chip. Chrome 153 keeps `aaabbb ccc` on one line. (Kills the mutant that
/// always breaks at the box's width.)
#[test]
fn a_nowrap_root_does_not_wrap_after_an_inline_box() {
    let (lines, h) = lay_out(
        "white-space:nowrap",
        "aaa<b style=\"display:inline-block;width:20px;height:10px\"></b>bbb ccc",
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(h, 25.0);
}

/// A `pre-wrap` span's spaces hang at the wrap, and the `pre` root's spaces
/// after them do not join the hang: they start the next line, as content.
/// Chrome 153: `aaaaa␣␣` at offset 5.08 (the hanging spaces past the edge),
/// `␣␣␣bb` at 16.91.
#[test]
fn a_pre_roots_spaces_after_hanging_pre_wrap_spaces_start_the_next_line() {
    let lines = offsets(
        "white-space:pre;text-align:right",
        "<span style=\"white-space:pre-wrap\">aaaaa  </span>   bb",
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0].0, "aaaaa");
    assert_eq!(lines[1].0, "bb");
    assert!((lines[0].1 - 5.08).abs() < 0.01, "{lines:?}");
    assert!((lines[1].1 - 16.91).abs() < 0.01, "{lines:?}");
}

/// A line ended by a forced break after the `pre` root's spaces is aligned
/// with those spaces as content too. Chrome 153 (round-2 review of #1228,
/// c13/c21): `ccaaa␣␣␣` overflows and starts at 0, `cca␣␣␣` fits and sits at
/// 9.08, and the last line `d` at 40.20.
#[test]
fn a_pre_roots_spaces_before_a_newline_are_content_in_alignment() {
    let lines = offsets(
        "white-space:pre;text-align:right",
        "<span style=\"white-space:normal\">bb cc</span>aaa   \nddd",
    );
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[1].0, "ccaaa");
    assert_eq!(lines[1].1, 0.0, "{lines:?}");
    let lines = offsets(
        "white-space:pre;text-align:right",
        "<span style=\"white-space:normal\">bb cc</span>a   \nd",
    );
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[1].0, "cca");
    assert!((lines[1].1 - 9.08).abs() < 0.01, "{lines:?}");
    assert!((lines[2].1 - 40.20).abs() < 0.01, "{lines:?}");
}

/// The trailing `pre` spaces may come from two adjacent text nodes (here the
/// root's and a `<b>`'s): every one counts. Chrome 153 (c22): 9.08.
#[test]
fn trailing_pre_spaces_across_adjacent_text_nodes_all_count() {
    let lines = offsets(
        "white-space:pre;text-align:right",
        "<span style=\"white-space:normal\">bb cc</span>a <b style=\"font-weight:normal\">  </b>",
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[1].0, "cca");
    assert!((lines[1].1 - 9.08).abs() < 0.01, "{lines:?}");
}

// ---- #1237: a paragraph that does not wrap aligns within the box ----
//
// A `nowrap`/`pre` root with no wrapping text is broken unconstrained, and
// parley then aligns each line within the paragraph's own widest line. CSS
// aligns it within the line box, the container's content width: an
// overflowing line starts at the start edge (CSS Text 3 §7.1), and every
// other line is right-aligned or centred against the box. Every expectation
// is Chrome 155's on the same markup (bundled Inter as `ProbeFace`,
// 16px/25px), the x of the line's first character in the div.

fn near(got: f32, want: f32, what: &str) {
    assert!(
        (got - want).abs() < 0.02,
        "{what}: got {got}, Chrome {want}"
    );
}

/// `(line text, offset)` of each line, and the x of each inline-block child,
/// for `inner` in a div styled `style`; `wrapper` wraps the div in a block of
/// that style (an auto-width div then takes the wrapper's width).
fn aligned(style: &str, inner: &str, wrapper: Option<&str>) -> (Vec<(String, f32)>, Vec<f32>) {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("font:16px/25px ProbeFace;{style}"));
    match wrapper {
        Some(w) => {
            let outer = d.create_element("div");
            d.set_attribute(outer, "style", w);
            d.append_child(body, outer);
            d.append_child(outer, c);
        }
        None => d.append_child(body, c),
    }
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    aligned_in(&d, c)
}

fn aligned_in(d: &RinchDocument, c: NodeId) -> (Vec<(String, f32)>, Vec<f32>) {
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("the div is an IFC root");
    let lines = il
        .layout
        .lines()
        .map(|l| {
            (
                il.text_content[l.text_range()].trim().to_string(),
                l.metrics().offset,
            )
        })
        .collect();
    let boxes = n
        .children
        .iter()
        .filter_map(|&k| d.tree.get(k))
        .filter(|k| {
            k.computed_style.display == rinch_dom::computed_style::DisplayValue::InlineBlock
        })
        .map(|k| k.layout.x)
        .collect();
    (lines, boxes)
}

/// The issue's first row, and its `center` and `end` twins: one short line
/// in a 50px box. rinch put each at 0, the start of its own width.
#[test]
fn a_nowrap_line_is_aligned_within_the_box() {
    for (style, text, want) in [
        ("white-space:nowrap;text-align:right", "a", 41.02),
        ("white-space:nowrap;text-align:end", "a", 41.02),
        ("white-space:nowrap;text-align:center", "ab", 15.61),
    ] {
        let (lines, _) = aligned(&format!("width:50px;{style}"), text, None);
        assert_eq!(lines.len(), 1, "{style}: {lines:?}");
        near(lines[0].1, want, style);
    }
}

/// The issue's second row: the long `pre` line overflows the box and starts
/// at its start edge; the short one is right-aligned against the box, not
/// against the long line (rinch: 99.59).
#[test]
fn a_pre_roots_short_line_is_aligned_within_the_box_not_its_longest_line() {
    let (lines, _) = aligned(
        "width:50px;white-space:pre;text-align:right",
        "aa\nbbbbbbbbbbbb",
        None,
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    near(lines[0].1, 32.03, "aa");
    assert_eq!(lines[1].1, 0.0, "the overflowing line: {lines:?}");
}

/// Lines ended by forced breaks — a `<br>` in `nowrap` text, a newline in
/// `pre` — each aligned on its own, centred too. Chrome 155: `a` 41.02 and
/// `bb` 30.41; `a` 20.50 and `bb` 15.20.
#[test]
fn every_line_of_a_paragraph_that_does_not_wrap_is_aligned_within_the_box() {
    let (lines, _) = aligned(
        "width:50px;white-space:nowrap;text-align:right",
        "a<br>bb",
        None,
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    near(lines[0].1, 41.02, "a");
    near(lines[1].1, 30.41, "bb");
    let (lines, _) = aligned(
        "width:50px;white-space:pre;text-align:center",
        "a\nbb",
        None,
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    near(lines[0].1, 20.50, "a");
    near(lines[1].1, 15.20, "bb");
}

/// A `pre` root's spaces do not hang, so the spaces before a newline are
/// aligned as content: `a` and three spaces end 50px in, `a` at 27.52, and
/// centred with two spaces at 16.00 (rinch: 0.81 and 5.30, within `b`'s or
/// `bb`'s width). The issue's third row is
/// `a_pre_roots_trailing_spaces_do_not_hang` above.
#[test]
fn a_pre_roots_spaces_before_a_newline_are_content_within_the_box() {
    let (lines, _) = aligned(
        "width:50px;white-space:pre;text-align:right",
        "a   \nb",
        None,
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    near(lines[0].1, 27.52, "a + 3 spaces");
    near(lines[1].1, 40.20, "b");
    let (lines, _) = aligned(
        "width:50px;white-space:pre;text-align:center",
        "a  \nbb",
        None,
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    near(lines[0].1, 16.00, "a + 2 spaces");
    near(lines[1].1, 15.20, "bb");
}

/// A no-break space never hangs: `a&nbsp;` ends at the box's edge (36.52).
#[test]
fn a_nowrap_roots_trailing_nbsp_is_content_within_the_box() {
    let (lines, _) = aligned(
        "width:50px;white-space:nowrap;text-align:right",
        "a&nbsp;",
        None,
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    near(lines[0].1, 36.52, "a&nbsp;");
}

/// A final newline is a forced break with no line after it (#1172): the
/// paragraph keeps its one line, aligned within the box (31.22).
#[test]
fn a_pre_roots_final_newline_keeps_one_line_aligned_within_the_box() {
    let (lines, _) = aligned("width:50px;white-space:pre;text-align:right", "ab\n", None);
    assert_eq!(lines.len(), 1, "{lines:?}");
    near(lines[0].1, 31.22, "ab");
}

/// Inline boxes move with their line: a 10px chip and `a` right-aligned put
/// the chip at 31.02. A 30px chip and `aaaa` overflow: one line, still — the
/// paragraph is not broken at the box's width (parley would wrap after the
/// chip, #1238) — starting at the start edge.
#[test]
fn a_nowrap_roots_inline_boxes_are_aligned_within_the_box() {
    let chip = |w: u32| format!("<b style=\"display:inline-block;width:{w}px;height:10px\"></b>");
    let (lines, boxes) = aligned(
        "width:50px;white-space:nowrap;text-align:right",
        &format!("{}a", chip(10)),
        None,
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    near(lines[0].1, 31.02, "line");
    near(boxes[0], 31.02, "chip");
    let (lines, boxes) = aligned(
        "width:50px;white-space:nowrap;text-align:right",
        &format!("{}aaaa", chip(30)),
        None,
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1, 0.0, "{lines:?}");
    assert_eq!(boxes[0], 0.0);
}

/// An auto-width root is aligned within its own content width, not the
/// pixel of slack its paint layout is broken with
/// (`ifc_paint_max_width`): `a` at 41.02 in a 50px block, as in Chrome.
#[test]
fn an_auto_width_nowrap_root_is_aligned_within_its_content_width() {
    let (lines, _) = aligned(
        "white-space:nowrap;text-align:right",
        "a",
        Some("width:50px"),
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    near(lines[0].1, 41.02, "a");
}

/// When the box's width changes the line follows it: `a` right-aligned in a
/// 25%-wide box, at two viewport widths.
#[test]
fn a_nowrap_line_follows_the_box_when_it_resizes() {
    let mut d = doc();
    let body = d.body();
    d.set_attribute(body, "style", "margin:0");
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        "width:25%;font:16px/25px ProbeFace;white-space:nowrap;text-align:right",
    );
    d.append_child(body, c);
    d.set_inner_html(c, "a");
    d.resolve_layout(200.0, 600.0);
    near(aligned_in(&d, c).0[0].1, 41.02, "50px box");
    d.resolve_layout(400.0, 600.0);
    near(aligned_in(&d, c).0[0].1, 91.02, "100px box");
}

/// parley takes the paragraph's direction from its text (rinch hands it no
/// CSS `direction`), so Hebrew text is a right-to-left paragraph to it. Such
/// a paragraph is aligned within the box too: centred, `שלום` sits half the
/// box's free space in (Chrome 155: 10.38; Inter has no Hebrew, so the width
/// is the host's fallback font's and the offset is checked against it; rinch
/// put it at 0). `start` is not handed the box's width (it does not move a
/// line in a left-to-right block), so it stays at 0, Chrome's answer, where
/// parley's right-to-left `start` would be the right edge. (`text-align:
/// right` is parley's `End`, the left edge of a right-to-left paragraph: 0
/// before and after, Chrome 20.75 — the inferred direction, not this issue.)
#[test]
fn a_nowrap_right_to_left_paragraph_is_aligned_within_the_box() {
    let (lines, _) = aligned("width:50px;white-space:nowrap", "שלום", None);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1, 0.0, "{lines:?}");

    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        "width:50px;font:16px/25px ProbeFace;white-space:nowrap;text-align:center",
    );
    d.append_child(body, c);
    d.set_inner_html(c, "שלום");
    d.resolve_layout(800.0, 600.0);
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("the div is an IFC root");
    assert!(il.layout.is_rtl());
    let line = il.layout.get(0).unwrap();
    let advance = line.metrics().advance;
    assert!(advance > 10.0 && advance < 48.0, "fits the box: {advance}");
    near(line.metrics().offset, (50.0 - advance) / 2.0, "centred");
}
