//! Preserved spaces at the end of a line **hang**; they never make the text a
//! line taller (CSS Text 3 §4.1.3, `white-space: pre-wrap`).
//!
//! Found in the rich-text editor: typing in a bulleted list item, the bullet
//! jumped a line down whenever the item's text ended in a space ("bullet text "),
//! and back up at the next character. The editor's `li` is a flex row
//! (`align-items: baseline`) holding the marker and the paragraph, and its text
//! is `pre-wrap`. Two things combined:
//!
//! 1. **The measure left the space out.** The paragraph's IFC reported parley's
//!    `Layout::width`, which drops every line's trailing white space, so the flex
//!    item was sized to `"bullet text"` and then laid out at that width with the
//!    space in it. Chrome's max-content width of a `pre-wrap` `"bullet text "` is
//!    the text *and* the space.
//! 2. **Parley 0.11.1 does not hang the space.** Laid out at a width the space
//!    overflows, it hangs the first overflowing space and then commits the line,
//!    so what is left — nothing, or the rest of the spaces — makes one more line.
//!    Any `pre-wrap` block does this when a space is typed at the right edge.
//!
//! The paragraph came out two lines tall, and the marker, which Taffy 0.12
//! baseline-aligns by its bottom edge (see
//! [`a_multi_line_flex_items_baseline_is_its_bottom_edge_not_its_first_line`]),
//! went down with it (#1013).
//!
//! Every expected number below is Chrome 153's, measured with the same markup and
//! the same face (the bundled Inter, registered here under an override name so
//! the host's fonts cannot answer instead) at 16px with a 25px line box. Taffy
//! rounds box edges to whole pixels, so Chrome's paragraph at `19.5, 78.91`
//! wide is `20, 78` here (its right edge 98.41 rounds to 98).

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::caret_position_for_offset;

const VW: f32 = 400.0;
const VH: f32 = 300.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
/// One line box.
const LINE: f32 = 25.0;
/// `"bullet text"` on this face at 16px, in Chrome (and in parley): 74.41.
const TEXT_W: f32 = 74.4;
/// One space: 4.5.
const SPACE_W: f32 = 4.5;

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn boxof(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.x, n.layout.y, n.layout.width, n.layout.height)
}

fn caret(doc: &RinchDocument, id: NodeId, offset: usize) -> (f32, f32) {
    caret_position_for_offset(doc, id.0 as u64, offset).expect("an IFC root has a text layout")
}

/// The editor's list item, reduced: `li { display: flex; flex-wrap: wrap;
/// align-items: baseline; gap }` holding the marker `<span>` and the paragraph.
/// Returns `(doc, li, marker, paragraph)`.
fn list_item(white_space: &str, paragraph: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut d = doc();
    let body = d.body();
    let li = el(
        &mut d,
        body,
        "div",
        &format!(
            "display: flex; flex-wrap: wrap; align-items: baseline; gap: 6px; width: 200px; \
             font-family: ProbeFace; font-size: 16px; line-height: 25px; white-space: {white_space}"
        ),
    );
    let marker = el(&mut d, li, "span", "");
    text(&mut d, marker, "\u{2022} ");
    let p = el(&mut d, li, "p", "margin: 0 0 12px");
    text(&mut d, p, paragraph);
    d.resolve_layout(VW, VH);
    (d, li, marker, p)
}

/// The reported bug. Chrome: `li 0,0,200x37`, marker `0,0,13.5x25`, paragraph
/// `19.5,0,78.91x25` — one line, the marker's space and the paragraph's space
/// both counted in their widths. On `main` the paragraph was `74x50` (two lines)
/// and so was the marker (`9x50`: its text ends in a space too), with the
/// paragraph pushed down to the marker's second line.
#[test]
fn a_pre_wrap_list_item_ending_in_a_space_is_one_line() {
    let (d, li, marker, p) = list_item("pre-wrap", "bullet text ");
    assert_eq!(boxof(&d, marker), (0.0, 0.0, 14.0, LINE), "marker");
    assert_eq!(boxof(&d, p), (20.0, 0.0, 78.0, LINE), "paragraph");
    assert_eq!(boxof(&d, li).3, LINE + 12.0, "list item");
    // The caret after the space sits after it, on the first line.
    let (x, y) = caret(&d, p, "bullet text ".len());
    assert_eq!(
        y, 0.0,
        "caret after the trailing space is on the first line"
    );
    assert!(
        (x - (TEXT_W + SPACE_W)).abs() < 0.1,
        "caret after the trailing space: x {x}"
    );
}

/// Chrome: paragraph `19.5,0,87.91x25` — three spaces, one line.
#[test]
fn a_pre_wrap_list_item_ending_in_several_spaces_is_one_line() {
    let (d, _, marker, p) = list_item("pre-wrap", "bullet text   ");
    assert_eq!(boxof(&d, marker), (0.0, 0.0, 14.0, LINE));
    assert_eq!(boxof(&d, p), (20.0, 0.0, 87.0, LINE));
}

/// The control, identical before and after: collapsible spaces at the end of a
/// line are removed, so neither box counts them. Chrome: marker `0,0,9x25`,
/// paragraph `15,0,74.41x25`.
#[test]
fn a_normal_white_space_list_item_ending_in_a_space_is_one_line() {
    let (d, _, marker, p) = list_item("normal", "bullet text ");
    assert_eq!(boxof(&d, marker), (0.0, 0.0, 9.0, LINE));
    assert_eq!(boxof(&d, p), (15.0, 0.0, 74.0, LINE));
}

/// A long item, as Chrome lays this markup out: the paragraph's flex base size
/// is its max-content width, wider than the row, so `flex-wrap: wrap` puts it on
/// a flex line of its own under the marker (Chrome: marker `0,0,13.5x25`,
/// paragraph `0,31,200x75`, li 118). On `main` the marker was two lines tall, so
/// the paragraph started at 56.
#[test]
fn a_long_pre_wrap_list_item_wraps_below_a_one_line_marker() {
    let (d, li, marker, p) = list_item(
        "pre-wrap",
        "a long list item that wraps onto more than one line here",
    );
    assert_eq!(boxof(&d, marker), (0.0, 0.0, 14.0, LINE));
    assert_eq!(boxof(&d, p), (0.0, 31.0, 200.0, 3.0 * LINE));
    assert_eq!(boxof(&d, li).3, 118.0);
}

/// A `pre-wrap` block `width` wide holding `s`.
fn block(width: &str, s: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    let b = el(
        &mut d,
        body,
        "div",
        &format!(
            "width: {width}; font-family: ProbeFace; font-size: 16px; line-height: 25px; \
             white-space: pre-wrap"
        ),
    );
    text(&mut d, b, s);
    d.resolve_layout(VW, VH);
    (d, b)
}

/// No flex at all: a space typed at the right edge of a `pre-wrap` block. The
/// text fits in 75px, its space does not. Chrome: 25 tall, the caret after the
/// space at `78.91` on the first line. On `main`: 50 tall, the caret at the start
/// of an empty second line.
#[test]
fn a_space_typed_at_the_right_edge_hangs() {
    let (d, b) = block("75px", "bullet text ");
    assert_eq!(boxof(&d, b).3, LINE);
    let (x, y) = caret(&d, b, "bullet text ".len());
    assert_eq!(y, 0.0);
    assert!((x - (TEXT_W + SPACE_W)).abs() < 0.1, "caret x {x}");
}

/// Every space of the sequence hangs, not only the first. Chrome: 25 tall, the
/// caret after the last space at 87.91. On `main`: 50 tall.
#[test]
fn a_run_of_spaces_at_the_right_edge_all_hang() {
    let (d, b) = block("75px", "bullet text   ");
    assert_eq!(boxof(&d, b).3, LINE);
    let (_, y) = caret(&d, b, "bullet text   ".len());
    assert_eq!(y, 0.0);
}

/// Spaces at a soft wrap hang too, so the next line starts with the next word,
/// not a space. Chrome: 50 tall, `text` at `0,27` (the second line, x 0). On
/// `main` the second space started the second line and pushed `text` right.
#[test]
fn spaces_at_a_soft_wrap_hang_and_the_next_line_starts_with_the_word() {
    let s = "bullet   text";
    let (d, b) = block("50px", s);
    assert_eq!(boxof(&d, b).3, 2.0 * LINE);
    let word = s.find("text").unwrap();
    assert_eq!(caret(&d, b, word), (0.0, LINE), "`text` starts line two");
}

/// A space the width cannot hold before a forced break stays on its line; the
/// newline ends that line and `more` is the second. Chrome: 50 tall, `more` at
/// `0,27`. On `main`: 75 tall, `more` on the third line.
#[test]
fn a_hanging_space_before_a_newline_does_not_add_a_line() {
    let s = "bullet text \nmore";
    let (d, b) = block("75px", s);
    assert_eq!(boxof(&d, b).3, 2.0 * LINE);
    assert_eq!(caret(&d, b, s.find("more").unwrap()), (0.0, LINE));
}

/// `text-align: right` lines the text, not its hanging space, up with the
/// edge: `text` lands exactly where it does without the space (46.67: the 0.6
/// of slack between 74.41 and 75, right of its left-aligned 46.07), on one line.
/// That is the spec's rule: at the end of a `pre-wrap` paragraph the spaces hang
/// unconditionally, and hanging glyphs take no part in alignment (CSS Text 3
/// §4.1.3). Chrome 153 treats the line as overflowing instead and leaves `text`
/// at its start-aligned 46.06. On `main` the space went to a line of its own.
#[test]
fn right_aligned_text_hangs_its_spaces_past_the_edge() {
    let (with_space, b) = block("75px; text-align: right", "bullet text ");
    assert_eq!(boxof(&with_space, b).3, LINE);
    let (without, b2) = block("75px; text-align: right", "bullet text");
    let at = caret(&with_space, b, "bullet ".len());
    assert_eq!(at.1, 0.0);
    assert_eq!(at, caret(&without, b2, "bullet ".len()));
    assert!((at.0 - 46.67).abs() < 0.1, "`text` x {}", at.0);
}

/// **A stated divergence, not a fix.** Baseline alignment in a flex row should
/// line the items' *first* baselines up (css-flexbox-1 §8.3). Taffy 0.12 cannot
/// be told a leaf's baseline: its measure function returns a size, and
/// `compute_leaf_layout` reports `first_baselines: Point::NONE`, so every item's
/// baseline is synthesized from its bottom border edge (block containers report
/// none either). A marker beside a three-line paragraph is aligned with the
/// paragraph's **bottom**, at `y = 50`; Chrome puts it on the first line, at
/// `y = 0`. Taffy 0.14 lets the measure return a `LayoutOutput` with baselines
/// (#1013).
///
/// `flex: 1 1 0` keeps the paragraph beside the marker (Chrome: paragraph
/// `19.5,0,180.5x75`, marker `0,0,13.5x25`).
#[test]
fn a_multi_line_flex_items_baseline_is_its_bottom_edge_not_its_first_line() {
    let mut d = doc();
    let body = d.body();
    let li = el(
        &mut d,
        body,
        "div",
        "display: flex; align-items: baseline; gap: 6px; width: 200px; \
         font-family: ProbeFace; font-size: 16px; line-height: 25px; white-space: pre-wrap",
    );
    let marker = el(&mut d, li, "span", "");
    text(&mut d, marker, "\u{2022} ");
    let p = el(&mut d, li, "p", "margin: 0; flex: 1 1 0");
    text(
        &mut d,
        p,
        "a long list item that wraps onto more than one line here",
    );
    d.resolve_layout(VW, VH);
    assert_eq!(boxof(&d, p), (20.0, 0.0, 180.0, 3.0 * LINE));
    // Chrome: (0, 0, 13.5, 25).
    assert_eq!(boxof(&d, marker), (0.0, 2.0 * LINE, 14.0, LINE));
}

/// The space counts only as far as the available width reaches; past it, it
/// hangs. A `flex-start` item of a 76px column is fit-content: the text is
/// 74.41, 78.91 with its space, so Chrome makes it 76x25 (and 74.41x25 without
/// the space). On `main` it was 74 wide and two lines tall.
#[test]
fn a_trailing_space_counts_only_up_to_the_available_width() {
    for (s, want) in [("bullet text ", 76.0), ("bullet text", 74.0)] {
        let mut d = doc();
        let body = d.body();
        let c = el(
            &mut d,
            body,
            "div",
            "display: flex; flex-direction: column; align-items: flex-start; width: 76px; \
             font-family: ProbeFace; font-size: 16px; line-height: 25px; white-space: pre-wrap",
        );
        let p = el(&mut d, c, "p", "margin: 0");
        text(&mut d, p, s);
        d.resolve_layout(VW, VH);
        let (_, _, w, h) = boxof(&d, p);
        assert_eq!((w, h), (want, LINE), "{s:?}");
    }
}

/// Spaces at a soft wrap are no part of the min-content width: a flex item in a
/// 10px row shrinks to its min-content width, the widest word *without* the
/// spaces after it. Chrome: 41.58x50 for each of these (`bullet` is 41.58;
/// the right edge rounds to 42 here).
#[test]
fn spaces_at_a_soft_wrap_are_not_in_the_min_content_width() {
    for s in ["bullet text", "bullet   text", "bullet text "] {
        let mut d = doc();
        let body = d.body();
        let c = el(
            &mut d,
            body,
            "div",
            "display: flex; width: 10px; font-family: ProbeFace; font-size: 16px; \
             line-height: 25px; white-space: pre-wrap",
        );
        let p = el(&mut d, c, "p", "margin: 0");
        text(&mut d, p, s);
        d.resolve_layout(VW, VH);
        let (_, _, w, h) = boxof(&d, p);
        assert_eq!((w, h), (42.0, 2.0 * LINE), "{s:?}");
    }
}
/// **A guard**: a line that ends at a soft wrap keeps leaving its spaces out of
/// the measured width, as parley's `Layout::width` always did. A `flex-start`
/// item of a 60px column wrapping `bullet text more` as `bullet ` / `text ` /
/// `more` is as wide as its widest line *without* its hanging space, 41.58 (42
/// here), `pre-wrap` exactly like `normal`; counting the space would make it 46.
///
/// Chrome makes this item **60** wide in both modes: a fit-content width is the
/// available width once the text wraps (CSS Sizing 3 §5.2,
/// `min(max-content, max(min-content, available))`), where rinch reports the
/// widest line. That divergence is older than this fix, the same in `normal`
/// white space, and not what this file is about.
#[test]
fn a_soft_wrapped_lines_hanging_space_is_not_in_the_measured_width() {
    for ws in ["pre-wrap", "normal"] {
        let mut d = doc();
        let body = d.body();
        let c = el(
            &mut d,
            body,
            "div",
            &format!(
                "display: flex; flex-direction: column; align-items: flex-start; width: 60px; \
                 font-family: ProbeFace; font-size: 16px; line-height: 25px; white-space: {ws}"
            ),
        );
        let p = el(&mut d, c, "p", "margin: 0");
        text(&mut d, p, "bullet text more");
        d.resolve_layout(VW, VH);
        assert_eq!(boxof(&d, p), (0.0, 0.0, 42.0, 3.0 * LINE), "{ws}");
    }
}

// ---- From #1018's review (Chrome 153, the same face and markup) ----

/// A shrink-to-fit box of `white-space: {ws}` holding `s`: a `flex-start` item
/// of a 300px column. Returns its box and the caret x at char `at`.
fn fit_content(ws: &str, s: &str, at: usize) -> ((f32, f32), f32) {
    let mut d = doc();
    let body = d.body();
    let c = el(
        &mut d,
        body,
        "div",
        "display: flex; flex-direction: column; align-items: flex-start; width: 300px; \
         font-family: ProbeFace; font-size: 16px; line-height: 25px",
    );
    let p = el(&mut d, c, "div", &format!("white-space: {ws}"));
    text(&mut d, p, s);
    d.resolve_layout(VW, VH);
    let (_, _, w, h) = boxof(&d, p);
    ((w, h), caret(&d, p, at).0)
}

/// NBSP is not white space that hangs (CSS Text 3 §4.1.3 hangs spaces and
/// tabs): `text` + three NBSPs is one unbreakable word that does not fit 80px,
/// so Chrome wraps it: 50 tall, `text` at the start of line two.
#[test]
fn nbsp_does_not_hang() {
    let s = "bullet text\u{a0}\u{a0}\u{a0}";
    let (d, b) = block("80px", s);
    assert_eq!(boxof(&d, b).3, 2.0 * LINE);
}

/// `pre-line` collapses spaces and removes them at the end of a line, so they
/// are no part of its max-content width. Chrome: 27.92.
#[test]
fn pre_line_trailing_spaces_are_not_measured() {
    let ((w, _), _) = fit_content("pre-line", "abc   ", 0);
    assert_eq!(w, 28.0);
}

/// The atomic-inline measure site: an `inline-block` counts its trailing
/// preserved spaces. Chrome: 41.42.
#[test]
fn inline_block_counts_its_trailing_spaces() {
    let mut d = doc();
    let body = d.body();
    let ib = el(
        &mut d,
        body,
        "span",
        "display: inline-block; font-family: ProbeFace; font-size: 16px; line-height: 25px; \
         white-space: pre-wrap",
    );
    text(&mut d, ib, "abc   ");
    d.resolve_layout(VW, VH);
    assert_eq!((boxof(&d, ib).2, boxof(&d, ib).3), (41.0, LINE));
}

/// The rebroken line leaves no room for a following word, however narrow:
/// Chrome puts `i` (3.88px) on line two.
#[test]
fn a_narrow_word_after_hanging_spaces_still_wraps() {
    let s = "bullet text   i";
    let (d, b) = block("75px", s);
    assert_eq!(boxof(&d, b).3, 2.0 * LINE);
    assert_eq!(caret(&d, b, s.len()).1, LINE);
}

/// Shrink-to-fit and alignment agree: the box counts its trailing spaces, so
/// the text starts at the box's start. Chrome: 41.42 wide, `abc` at 0 for
/// right and center alignment alike.
#[test]
#[ignore = "#1042: parley's `align` hangs every trailing space, so the text starts past its own box"]
fn right_aligned_fit_content_text_starts_at_its_start() {
    for align in ["right", "center"] {
        let ((w, _), x) = fit_content(&format!("pre-wrap; text-align: {align}"), "abc   ", 0);
        assert_eq!(w, 41.0);
        assert!(x.abs() < 1.0, "{align}: `abc` at {x}");
    }
}
