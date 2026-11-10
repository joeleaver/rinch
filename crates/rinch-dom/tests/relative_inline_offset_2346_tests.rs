//! A `position: relative` **inline** element moves everything it holds by its
//! offset (CSS 2.1 §9.4.3): its text, its background, the atomic inlines in
//! its lines (an `inline-block`, an `inline-flex`, an image), the relative
//! spans nested in it, and an absolute box it is the containing block of
//! (issues #2346 and #1425).
//!
//! Every absolute number is Chrome 156's (`--headless`, standards mode,
//! `* { box-sizing: border-box }`, 800x600), from `getBoundingClientRect`
//! **relative to the block container's border box**, with this crate's
//! bundled Inter loaded through `@font-face` under the name the fixtures
//! register it with. The scaffold is `abs_inline_containing_block_tests.rs`'s:
//!
//! ```html
//! <div C="position: relative; width: 400px; font: 16px/20px ProbeFace;
//!         margin: 13px 0 0 17px; padding: 7px 0 0 11px">…</div>
//! ```
//!
//! so the content box starts at `(11, 7)`, and every offset is off the
//! fixed point (none is 0, and `left` differs from `top`). The vertical
//! numbers are declarations (Inter at 16px is 20px from ascent to descent,
//! the declared line height); a horizontal number after `lead ` is the
//! advance of `lead ` in Inter (36.641). Each case is also checked against
//! a twin without the offset, so the move is exactly the offset whatever the
//! line is.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VW: f32 = 800.0;
const VH: f32 = 600.0;
const C: &str = "position: relative; width: 400px; font: 16px/20px ProbeFace; \
                 margin: 13px 0 0 17px; padding: 7px 0 0 11px;";
const R: &str = "position: relative";
const IB: &str = "display: inline-block; width: 30px; height: 20px; background: rgb(255, 0, 255)";

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.load_css("* { box-sizing: border-box; }");
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

struct Case {
    doc: RinchDocument,
    c: u64,
}

fn one(doc: &RinchDocument, sel: &str) -> u64 {
    let ids = query_selector(&doc.tree, sel);
    assert_eq!(ids.len(), 1, "exactly one {sel}");
    ids[0]
}

impl Case {
    fn with(c_extra: &str, inner: &str) -> Self {
        let mut doc = document();
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_inner_html(
            wrap,
            &format!(r#"<div data-m="c" style="{C}{c_extra}">{inner}</div>"#),
        );
        doc.append_child(body, wrap);
        doc.resolve_layout(VW, VH);
        let c = one(&doc, "[data-m=c]");
        Case { doc, c }
    }

    fn new(inner: &str) -> Self {
        Self::with("", inner)
    }

    fn id(&self, m: &str) -> u64 {
        one(&self.doc, &format!("[data-m={m}]"))
    }

    /// `m`'s box relative to the container's border box, as laid out.
    fn rect(&self, m: &str) -> [f32; 4] {
        let id = self.id(m);
        let (x, y) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, id, 1.0);
        let (cx, cy) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, self.c, 1.0);
        let l = self.doc.tree.get(id).unwrap().layout;
        [(x - cx) as f32, (y - cy) as f32, l.width, l.height]
    }

    /// `m`'s **painted** border box (what `getBoundingClientRect` answers on
    /// desktop) relative to the container's.
    fn painted(&self, m: &str) -> [f32; 4] {
        let r = rinch_dom::paint::painted_border_box(&self.doc.tree, self.id(m), 1.0);
        let c = rinch_dom::paint::painted_border_box(&self.doc.tree, self.c, 1.0);
        [
            (r.x0 - c.x0) as f32,
            (r.y0 - c.y0) as f32,
            r.width() as f32,
            r.height() as f32,
        ]
    }

    /// The glyph at flat byte `byte` of the container's lines, relative to
    /// its content origin.
    fn glyph(&self, byte: usize) -> [f32; 2] {
        let b = rinch_dom::text_query::glyph_bounds_for_offset(&self.doc, self.c, byte)
            .expect("a glyph there");
        [b.x, b.y]
    }

    fn caret(&self, byte: usize) -> [f32; 2] {
        let (x, y) = rinch_dom::text_query::caret_position_for_offset(&self.doc, self.c, byte)
            .expect("a caret there");
        [x, y]
    }
}

fn ib(extra: &str) -> String {
    format!(r#"<span data-m="ib" style="{IB}; {extra}"></span>"#)
}

#[track_caller]
fn assert_rect(got: [f32; 4], want: [f32; 4], what: &str) {
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g - w).abs() < 0.01,
            "{what}: got {got:?}, Chrome 156 {want:?}"
        );
    }
}

/// `inner` with `{OFF}` replaced by `offset`, against the same markup with
/// no offset: the move of `m`'s box, laid out and painted.
#[track_caller]
fn moved_by(c_extra: &str, inner: &str, offset: &str, m: &str) -> [f32; 2] {
    let moved = Case::with(c_extra, &inner.replace("{OFF}", offset));
    let twin = Case::with(c_extra, &inner.replace("{OFF}", ""));
    let (a, b) = (moved.rect(m), twin.rect(m));
    assert_eq!(&a[2..], &b[2..], "the offset moves the box, never sizes it");
    let (pa, pb) = (moved.painted(m), twin.painted(m));
    let laid = [a[0] - b[0], a[1] - b[1]];
    assert_eq!(
        laid,
        [pa[0] - pb[0], pa[1] - pb[1]],
        "the painted box moves as the laid-out one does"
    );
    laid
}

// ── The issue ───────────────────────────────────────────────────────────────

/// #2346's markup: the box is 5px down, at `(11, 12)` (Chrome 156; it was
/// at `(11, 7)`, where it would be without the offset).
#[test]
fn the_issues_inline_block_is_moved_down_by_the_spans_top() {
    let c = Case::new(&format!(r#"<span style="{R}; top: 5px">{}</span>"#, ib("")));
    assert_rect(c.rect("ib"), [11.0, 12.0, 30.0, 20.0], "laid out");
    assert_rect(c.painted("ib"), [11.0, 12.0, 30.0, 20.0], "painted");
}

/// #2346's second case, in a middle-aligned 100px cell: the cell's
/// alignment plus the span's 5. With the issue's default font Chrome 156
/// puts the box 46 down (41 + 5); with Inter at `16px/20px` its line is 24
/// tall (the box on the baseline, the strut's descent below it), so 43:
/// `(100 - 24) / 2 + 5`. Before the fix rinch gave 38, where the box would
/// be without the offset.
#[test]
fn in_a_middle_aligned_cell_the_offset_adds_to_the_alignment() {
    let mut doc = document();
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_inner_html(
        wrap,
        &format!(
            r#"<table style="border-collapse: collapse; font: 16px/20px ProbeFace"><tr>
               <td data-m="td" style="height: 100px; vertical-align: middle; padding: 0">
               <span style="{R}; top: 5px">{}</span></td></tr></table>"#,
            ib("")
        ),
    );
    doc.append_child(body, wrap);
    doc.resolve_layout(VW, VH);
    let td = one(&doc, "[data-m=td]");
    let b = one(&doc, "[data-m=ib]");
    let (_, ty) = rinch_dom::paint::compute_absolute_position(&doc.tree, td, 1.0);
    let (_, by) = rinch_dom::paint::compute_absolute_position(&doc.tree, b, 1.0);
    assert_eq!((by - ty) as f32, 43.0, "Chrome 156: 38 + 5");
}

// ── Each inset ──────────────────────────────────────────────────────────────

const LEAD: &str = r#"lead <span style="position: relative; {OFF}">IB</span>"#;

fn lead() -> String {
    LEAD.replace("IB", &ib(""))
}

#[test]
fn top_moves_the_box_down() {
    assert_eq!(moved_by("", &lead(), "top: 5px", "ib"), [0.0, 5.0]);
    let c = Case::new(&lead().replace("{OFF}", "top: 5px"));
    assert_rect(c.rect("ib"), [47.641, 12.0, 30.0, 20.0], "Chrome 156");
}

#[test]
fn left_moves_the_box_right() {
    assert_eq!(moved_by("", &lead(), "left: 7px", "ib"), [7.0, 0.0]);
    let c = Case::new(&lead().replace("{OFF}", "left: 7px"));
    assert_rect(c.rect("ib"), [54.641, 7.0, 30.0, 20.0], "Chrome 156");
}

#[test]
fn bottom_moves_the_box_up() {
    assert_eq!(moved_by("", &lead(), "bottom: 5px", "ib"), [0.0, -5.0]);
}

#[test]
fn right_moves_the_box_left() {
    assert_eq!(moved_by("", &lead(), "right: 7px", "ib"), [-7.0, 0.0]);
}

/// Over-constrained: `top` wins over `bottom`, and `left` over `right` in a
/// left-to-right block (CSS 2.1 §9.4.3).
#[test]
fn top_and_left_win_over_bottom_and_right() {
    assert_eq!(
        moved_by(
            "",
            &lead(),
            "top: 5px; bottom: 9px; left: 3px; right: 11px",
            "ib"
        ),
        [3.0, 5.0]
    );
}

/// A percentage `left` is of the containing block's content width (389, the
/// 400px border box less its 11px padding → 38.9, Chrome 156's
/// `86.531 - 47.641`); a
/// percentage `top` of its height only when that is a length — here it is
/// `auto`, so `top: 10%` computes to `auto` and moves nothing (Chrome 156).
#[test]
fn a_percentage_left_is_of_the_blocks_width_and_top_needs_a_definite_height() {
    let [dx, dy] = moved_by("", &lead(), "left: 10%; top: 10%", "ib");
    assert!((dx - 38.9).abs() < 0.001, "got {dx}");
    assert_eq!(dy, 0.0);
}

/// With `height: 100px` (content 93 tall under the 7px top padding) a
/// percentage `top` is of it: `10%` is 9.3 (Chrome 156: 16.297 and, for
/// `bottom: 10%`, -2.297, against 7).
#[test]
fn a_percentage_top_is_of_a_definite_height() {
    let [dx, dy] = moved_by("height: 100px;", &lead(), "left: 10%; top: 10%", "ib");
    assert!((dx - 38.9).abs() < 0.001, "got {dx}");
    assert!((dy - 9.3).abs() < 0.001, "got {dy}");
    let [dx, dy] = moved_by("height: 100px;", &lead(), "right: 10%; bottom: 10%", "ib");
    assert!((dx + 38.9).abs() < 0.001, "got {dx}");
    assert!((dy + 9.3).abs() < 0.001, "got {dy}");
}

/// `inner` in the container `C` (`c_extra` appended) inside a wrapper
/// styled `outer`, against the same with no offset: the move of `m`'s box.
#[track_caller]
fn moved_in(outer: &str, c_extra: &str, inner: &str, offset: &str, m: &str) -> [f32; 2] {
    let case = |off: &str| {
        let mut doc = document();
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_inner_html(
            wrap,
            &format!(
                r#"<div style="{outer}"><div data-m="c" style="{C}{c_extra}">{}</div></div>"#,
                inner.replace("{OFF}", off)
            ),
        );
        doc.append_child(body, wrap);
        doc.resolve_layout(VW, VH);
        let c = one(&doc, "[data-m=c]");
        Case { doc, c }.rect(m)
    };
    let (a, b) = (case(offset), case(""));
    [a[0] - b[0], a[1] - b[1]]
}

/// A height that is definite without being a length is one a percentage
/// `top` resolves against, as Chrome 156 does: a flex item stretched across
/// a 100px row, an absolute box sized by `top: 0; bottom: 100px` in a 200px
/// block, and `height: 50%` of a 200px parent. The first two are 87 tall
/// (100 less the container's 13px top margin), 80 under its 7px padding, so
/// `top: 10%` is 8; the third is 100 (`box-sizing: border-box`), 93 inside,
/// so 9.3 (Chrome 156: 15, 15 and 16.297, against 7). An auto-height
/// block's is not definite (above).
#[test]
fn a_percentage_top_is_of_a_height_that_layout_makes_definite() {
    for (outer, c_extra, want) in [
        ("display: flex; height: 100px", "", 8.0),
        (
            "position: relative; height: 200px",
            "position: absolute; top: 0; bottom: 100px;",
            8.0,
        ),
        ("height: 200px", "height: 50%;", 9.3),
    ] {
        let [dx, dy] = moved_in(outer, c_extra, &lead(), "top: 10%", "ib");
        assert_eq!(dx, 0.0, "{outer}");
        assert!((dy - want).abs() < 0.001, "{outer}: got {dy}");
    }
    // A flex item with an `auto` block-axis margin is not stretched (CSS
    // Flexbox §8.1), so its height is its content's: Chrome 156 moves the
    // span by nothing, for `margin-top: auto` and `margin: auto 0` alike.
    for margin in ["margin-top: auto;", "margin: auto 0;"] {
        let [_, dy] = moved_in(
            "display: flex; height: 100px",
            margin,
            &lead(),
            "top: 10%",
            "ib",
        );
        assert_eq!(dy, 0.0, "{margin}");
    }
    // A column flex item's height is its content's here: not definite.
    let [_, dy] = moved_in(
        "display: flex; flex-direction: column",
        "",
        &lead(),
        "top: 10%",
        "ib",
    );
    assert_eq!(dy, 0.0, "an auto-height column item");
}

/// A sticky span is not moved by its insets where nothing scrolls it past
/// them: this one is below its `top: 5px` line, so it stays where it flows.
#[test]
fn a_sticky_span_out_of_reach_of_its_insets_moves_nothing() {
    let sticky = r#"lead <span style="position: sticky; {OFF}">IB</span>"#.replace("IB", &ib(""));
    assert_eq!(
        moved_by("", &sticky, "top: 5px; left: 7px", "ib"),
        [0.0, 0.0]
    );
}

// ── Nesting and what moves ──────────────────────────────────────────────────

/// Offsets compose: a relative span in a relative span moves by both.
#[test]
fn nested_relative_spans_add_up() {
    let inner = format!(
        r#"lead <span style="{R}; {{OFF}}">a<span style="{R}; top: 4px; left: 5px">b{}</span></span>"#,
        ib("")
    );
    assert_eq!(
        moved_by("", &inner, "top: 3px; left: 2px", "ib"),
        [2.0, 3.0]
    );
    let c = Case::new(&inner.replace("{OFF}", "top: 3px; left: 2px"));
    let twin = Case::new(
        &inner
            .replace("{OFF}", "")
            .replace("top: 4px; left: 5px", ""),
    );
    let (a, b) = (c.rect("ib"), twin.rect("ib"));
    assert_eq!(
        [a[0] - b[0], a[1] - b[1]],
        [7.0, 7.0],
        "3 + 4 down, 2 + 5 right"
    );
    // And the text: `a` (byte 5) by the outer span's alone, `b` (byte 6) by
    // both.
    let (ga, gb) = (c.glyph(5), twin.glyph(5));
    assert_eq!([ga[0] - gb[0], ga[1] - gb[1]], [2.0, 3.0], "`a`");
    let (ga, gb) = (c.glyph(6), twin.glyph(6));
    assert_eq!([ga[0] - gb[0], ga[1] - gb[1]], [7.0, 7.0], "`b`");
}

/// The box's own offset adds to the span's.
#[test]
fn an_inline_blocks_own_offset_adds_to_its_spans() {
    let inner = format!(
        r#"lead <span style="{R}; {{OFF}}">{}</span>"#,
        ib("position: relative; top: 2px; left: 3px")
    );
    assert_eq!(
        moved_by("", &inner, "top: 5px; left: 7px", "ib"),
        [7.0, 5.0]
    );
}

#[test]
fn an_image_and_an_inline_flex_move_with_the_span() {
    let img = r#"lead <span style="position: relative; {OFF}"><img data-m="ib" style="width: 30px; height: 20px"></span>"#;
    assert_eq!(moved_by("", img, "top: 5px; left: 7px", "ib"), [7.0, 5.0]);
    let flex = r#"lead <span style="position: relative; {OFF}"><span data-m="ib" style="display: inline-flex; width: 30px; height: 20px"></span></span>"#;
    assert_eq!(moved_by("", flex, "top: 5px; left: 7px", "ib"), [7.0, 5.0]);
}

/// A span that wraps moves the box on its second line too.
#[test]
fn a_wrapped_span_moves_the_box_on_its_second_line() {
    let inner = format!(
        r#"lead <span style="{R}; {{OFF}}">alpha beta gamma {} delta</span>"#,
        ib("")
    );
    let c = Case::with("width: 131px;", &inner.replace("{OFF}", ""));
    assert!(c.rect("ib")[1] >= 7.0 + 20.0, "the box is on a later line");
    assert_eq!(
        moved_by("width: 131px;", &inner, "top: 5px; left: 7px", "ib"),
        [7.0, 5.0]
    );
}

/// The text moves too (#1425): its glyphs and the caret, where `get_glyph_bounds`
/// and the editor read them. The text after the span does not.
#[test]
fn the_spans_text_and_caret_move_and_the_text_after_it_does_not() {
    let inner = r#"lead <span style="position: relative; {OFF}">text</span> tail"#;
    let moved = Case::new(&inner.replace("{OFF}", "top: 5px; left: 7px"));
    let twin = Case::new(&inner.replace("{OFF}", ""));
    // "lead " is bytes 0..5, "text" 5..9, " tail" after.
    for byte in [5, 8] {
        let (a, b) = (moved.glyph(byte), twin.glyph(byte));
        assert_eq!([a[0] - b[0], a[1] - b[1]], [7.0, 5.0], "glyph at {byte}");
        let (a, b) = (moved.caret(byte), twin.caret(byte));
        assert_eq!([a[0] - b[0], a[1] - b[1]], [7.0, 5.0], "caret at {byte}");
    }
    for byte in [0, 11] {
        assert_eq!(moved.glyph(byte), twin.glyph(byte), "glyph at {byte} stays");
    }
}

/// The span's own `getBoundingClientRect()` (the union of its fragments,
/// #1779) is where it is drawn: Chrome 156 answers `(54.641, 12)`,
/// 28.328 x 20, the unmoved `(47.641, 7)` plus the offset.
#[test]
fn the_spans_bounding_rect_is_moved() {
    let inner = r#"lead <span data-m="span" style="position: relative; {OFF}">text</span>"#;
    let rect = |offset: &str| {
        let mut c = Case::new(&inner.replace("{OFF}", offset));
        let span = rinch_core::dom::NodeId(c.id("span"));
        let cid = rinch_core::dom::NodeId(c.c);
        let s = c.doc.bounding_rect(span).unwrap();
        let o = c.doc.bounding_rect(cid).unwrap();
        [s.x - o.x, s.y - o.y, s.width, s.height]
    };
    let (a, b) = (rect("top: 5px; left: 7px"), rect(""));
    assert_eq!([a[0] - b[0], a[1] - b[1]], [7.0, 5.0]);
    assert_eq!(&a[2..], &b[2..]);
    assert_rect(a, [54.641, 12.0, 28.328, 20.0], "Chrome 156");
}

/// The editor's queries (`rinch-editor-view` paints its highlight and
/// carets through them) answer where the span's text and boxes are drawn:
/// the selection with its boxes, the caret beside an image, the line box
/// under the caret, and the caret at an element's edge box.
#[test]
fn the_editors_queries_follow_the_moved_text_and_boxes() {
    use rinch_core::dom::CaretAffinity;
    let inner = r#"lead <span data-m="span" style="position: relative; padding-left: 2px; {OFF}">te<img data-m="img" style="width: 10px; height: 10px">xt</span> tail"#;
    let moved = Case::new(&inner.replace("{OFF}", "top: 5px; left: 7px"));
    let twin = Case::new(&inner.replace("{OFF}", ""));
    let d = |a: (f32, f32), b: (f32, f32)| [a.0 - b.0, a.1 - b.1];
    // The text alone, then with the image selected.
    for boxes in [vec![], vec![moved.id("img")]] {
        let twin_boxes: Vec<u64> = if boxes.is_empty() {
            vec![]
        } else {
            vec![twin.id("img")]
        };
        let a = moved
            .doc
            .query_selection_rects_with_boxes(moved.c, 5, 9, &boxes);
        let b = twin
            .doc
            .query_selection_rects_with_boxes(twin.c, 5, 9, &twin_boxes);
        assert_eq!(a.len(), b.len(), "{boxes:?}");
        for (a, b) in a.iter().zip(&b) {
            assert_eq!(d((a.0, a.1), (b.0, b.1)), [7.0, 5.0], "{boxes:?}");
            assert_eq!((a.2, a.3), (b.2, b.3));
        }
    }
    // The pieces outside the span stay.
    let a = moved
        .doc
        .query_selection_rects_with_boxes(moved.c, 1, 12, &[]);
    let b = twin
        .doc
        .query_selection_rects_with_boxes(twin.c, 1, 12, &[]);
    assert_eq!(a.first().map(|r| r.1), b.first().map(|r| r.1));
    assert_eq!(a.last().map(|r| r.1), b.last().map(|r| r.1));
    for after in [false, true] {
        let a = moved
            .doc
            .query_inline_box_caret(moved.c, moved.id("img"), after)
            .unwrap();
        let b = twin
            .doc
            .query_inline_box_caret(twin.c, twin.id("img"), after)
            .unwrap();
        assert_eq!(d((a.0, a.1), (b.0, b.1)), [7.0, 5.0], "beside the image");
        let a = moved
            .doc
            .query_inline_edge_caret(moved.c, moved.id("span"), false, after)
            .unwrap();
        let b = twin
            .doc
            .query_inline_edge_caret(twin.c, twin.id("span"), false, after)
            .unwrap();
        assert_eq!(
            d((a.0, a.1), (b.0, b.1)),
            [7.0, 5.0],
            "at the span's start edge"
        );
    }
    let a = moved
        .doc
        .query_line_box(moved.c, 6, CaretAffinity::Downstream)
        .unwrap();
    let b = twin
        .doc
        .query_line_box(twin.c, 6, CaretAffinity::Downstream)
        .unwrap();
    assert_eq!(a.0 - b.0, 5.0, "the line box under a caret in the span");
    let a = moved
        .doc
        .query_line_box(moved.c, 1, CaretAffinity::Downstream)
        .unwrap();
    let b = twin
        .doc
        .query_line_box(twin.c, 1, CaretAffinity::Downstream)
        .unwrap();
    assert_eq!(a, b, "and not outside it");
}

/// Selected boxes drawn at two offsets, both at the byte where the span
/// starts: the one before the span is highlighted where it stands, the one
/// in the span where the span moved it.
#[test]
fn selected_boxes_are_highlighted_at_their_own_offsets() {
    let c = Case::new(
        r#"lead<img data-m="a" style="width: 10px; height: 10px"><span style="position: relative; top: 5px; left: 7px"><img data-m="b" style="width: 10px; height: 10px">text</span> tail"#,
    );
    let rects = c
        .doc
        .query_selection_rects_with_boxes(c.c, 1, 8, &[c.id("a"), c.id("b")]);
    let covers = |m: &str| {
        let [x, y, w, h] = c.painted(m);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        rects
            .iter()
            .any(|r| r.0 <= cx && cx <= r.0 + r.2 && r.1 <= cy && cy <= r.1 + r.3)
    };
    assert!(covers("a"), "the box before the span: {rects:?}");
    assert!(
        covers("b"),
        "the moved box in the span: {rects:?} vs {:?}",
        c.painted("b")
    );
}

/// A change of `bottom` alone is a new offset too.
#[test]
fn a_new_bottom_alone_is_applied_on_the_next_layout() {
    let mut c = Case::new(&format!(
        r#"lead <span data-m="span" style="{R}; bottom: 3px">text{}</span>"#,
        ib("")
    ));
    let (before_box, before_glyph) = (c.rect("ib"), c.glyph(5));
    let span = c.id("span");
    c.doc.set_attribute(
        rinch_core::dom::NodeId(span),
        "style",
        "position: relative; bottom: 9px",
    );
    c.doc.resolve_layout(VW, VH);
    assert_eq!(c.rect("ib")[1] - before_box[1], -6.0);
    assert_eq!(c.glyph(5)[1] - before_glyph[1], -6.0);
}

/// Over-constrained in a right-to-left block, `right` wins: `left: 3px;
/// right: 9px` moves the box 9px left (Chrome 156).
#[test]
fn right_wins_over_left_in_a_right_to_left_block() {
    assert_eq!(
        moved_by("direction: rtl;", &lead(), "left: 3px; right: 9px", "ib"),
        [-9.0, 0.0]
    );
}

/// The selection highlight over the span's text is where the text is drawn.
#[test]
fn the_selection_highlight_moves_with_the_text() {
    let inner = r#"lead <span style="position: relative; {OFF}">text</span> tail"#;
    let moved = Case::new(&inner.replace("{OFF}", "top: 5px; left: 7px"));
    let twin = Case::new(&inner.replace("{OFF}", ""));
    let rects = |c: &Case| {
        c.doc
            .query_selection_rects(c.c, 5, 9)
            .into_iter()
            .map(|(x, y, w, h)| [x, y, w, h])
            .collect::<Vec<_>>()
    };
    let (a, b) = (rects(&moved), rects(&twin));
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(
        [a[0][0] - b[0][0], a[0][1] - b[0][1], a[0][2], a[0][3]],
        [7.0, 5.0, b[0][2], b[0][3]]
    );
    // Across the span's edges: the pieces outside it stay.
    let (a, b) = (
        moved.doc.query_selection_rects(moved.c, 2, 12),
        twin.doc.query_selection_rects(twin.c, 2, 12),
    );
    assert_eq!(b.len(), 1, "one line, one piece");
    assert_eq!(a.len(), 3, "before, in and after the span");
    assert_eq!(
        a[0].1, b[0].1,
        "the piece before the span stays on its line"
    );
    assert_eq!(a[1].1, b[0].1 + 5.0, "the span's piece is 5px down");
    assert_eq!(a[2].1, b[0].1, "the piece after it stays");
}

/// An absolute box whose containing block is the span hangs from its moved
/// fragment: Chrome 153 and 156 put `left: 0; top: 0` at `(52.641, 10)`, the
/// unmoved fragment's `(47.641, 7)` plus `(5, 3)` (#1425). rinch snaps such
/// a box to the pixel grid (`abs_inline_containing_block_tests.rs`: the
/// unmoved one is at 48), so 53. With `auto` insets it is at its static
/// position, which moves with the span too: Chrome's `80.969`, after `text`.
#[test]
fn an_absolute_box_in_the_span_hangs_from_its_moved_fragment() {
    let abs = |insets: &str| {
        Case::new(&format!(
            r#"lead <span style="{R}; left: 5px; top: 3px">text<span data-m="abs" style="position: absolute; {insets} width: 40px; height: 30px"></span>tail</span>"#
        ))
        .rect("abs")
    };
    assert_eq!(abs("left: 0; top: 0;"), [53.0, 10.0, 40.0, 30.0]);
    assert_eq!(abs(""), [81.0, 10.0, 40.0, 30.0]);
    // A block-level one's static position is below the line, at the
    // content box's left edge: moved 7px right and 5px down by a
    // `left: 7px; top: 5px` span, Chrome 156's `(18, 32)` (`(11, 27)`
    // without the offset).
    let block = |offset: &str| {
        Case::new(&format!(
            r#"lead <span style="{R}; {offset}">text<div data-m="abs" style="position: absolute; width: 40px; height: 30px"></div>tail</span>"#
        ))
        .rect("abs")
    };
    assert_eq!(block(""), [11.0, 27.0, 40.0, 30.0]);
    assert_eq!(block("left: 7px; top: 5px"), [18.0, 32.0, 40.0, 30.0]);
}

/// A change of the offset moves the box and the text on the next layout.
#[test]
fn a_new_offset_is_applied_on_the_next_layout() {
    let mut c = Case::new(&format!(
        r#"lead <span data-m="span" style="{R}; top: 5px">text{}</span>"#,
        ib("")
    ));
    let (before_box, before_glyph) = (c.rect("ib"), c.glyph(5));
    let span = c.id("span");
    c.doc.set_attribute(
        rinch_core::dom::NodeId(span),
        "style",
        "position: relative; top: 9px; left: 4px",
    );
    c.doc.resolve_layout(VW, VH);
    let (after_box, after_glyph) = (c.rect("ib"), c.glyph(5));
    assert_eq!(
        [after_box[0] - before_box[0], after_box[1] - before_box[1]],
        [4.0, 4.0]
    );
    assert_eq!(
        [
            after_glyph[0] - before_glyph[0],
            after_glyph[1] - before_glyph[1]
        ],
        [4.0, 4.0]
    );
}

// ── Pixels ──────────────────────────────────────────────────────────────────

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    fn pixels(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut cx,
        );
        painter.pixels().as_chunks::<4>().0.to_vec()
    }

    /// The bounding box of the pixels `hit` picks, in device px.
    fn bbox(px: &[[u8; 4]], hit: impl Fn(&[u8; 4]) -> bool) -> Option<[usize; 4]> {
        let w = VW as usize;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (i, p) in px.iter().enumerate() {
            if hit(p) {
                let (x, y) = (i % w, i / w);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
        (x0 != usize::MAX).then_some([x0, y0, x1, y1])
    }

    fn magenta(p: &[u8; 4]) -> bool {
        p[0] > 200 && p[1] < 60 && p[2] > 200
    }

    fn green(p: &[u8; 4]) -> bool {
        p[0] < 60 && p[1] > 200 && p[2] < 60
    }

    /// Dark ink: black text (the canvas is transparent, its pixels
    /// premultiplied).
    fn ink(p: &[u8; 4]) -> bool {
        p[3] > 100 && p[0] < 60 && p[1] < 60 && p[2] < 60
    }

    fn painted_case(inner: &str, offset: &str) -> Vec<[u8; 4]> {
        let mut c = Case::new(&inner.replace("{OFF}", offset));
        pixels(&mut c.doc)
    }

    /// The inline-block is painted at its moved box: 7px right, 5px down.
    #[test]
    fn the_inline_block_is_painted_where_it_moved() {
        let inner = lead();
        let a = bbox(&painted_case(&inner, "top: 5px; left: 7px"), magenta).unwrap();
        let b = bbox(&painted_case(&inner, ""), magenta).unwrap();
        assert_eq!([a[0] - b[0], a[1] - b[1]], [7, 5]);
        assert_eq!([a[2] - a[0], a[3] - a[1]], [b[2] - b[0], b[3] - b[1]]);
    }

    /// The span's text and background are painted where they moved; the
    /// text before it is not.
    #[test]
    fn the_spans_text_and_background_are_painted_where_they_moved() {
        let inner =
            r#"<span style="position: relative; {OFF}; background: rgb(0, 255, 0)">text</span>"#;
        let (moved, twin) = (
            painted_case(inner, "top: 5px; left: 7px"),
            painted_case(inner, "top: 0px"),
        );
        let (a, b) = (bbox(&moved, green).unwrap(), bbox(&twin, green).unwrap());
        assert_eq!(
            [a[0] as i64 - b[0] as i64, a[1] as i64 - b[1] as i64],
            [7, 5],
            "background"
        );
        let (a, b) = (bbox(&moved, ink).unwrap(), bbox(&twin, ink).unwrap());
        assert_eq!(
            [a[0] as i64 - b[0] as i64, a[1] as i64 - b[1] as i64],
            [7, 5],
            "text"
        );
        assert_eq!(
            [a[2] - a[0], a[3] - a[1]],
            [b[2] - b[0], b[3] - b[1]],
            "same ink"
        );
    }

    /// The span's underline moves with its glyphs: green, under black text,
    /// 7px right and 5px down of where the twin draws it.
    #[test]
    fn the_spans_underline_moves_with_its_text() {
        let inner = r#"lead <span style="position: relative; {OFF}; text-decoration: underline; text-decoration-color: rgb(0, 255, 0)">text</span>"#;
        let (moved, twin) = (
            painted_case(inner, "top: 5px; left: 7px"),
            painted_case(inner, "top: 0px"),
        );
        // A 1px line between pixel rows: any green ink.
        let greenish = |p: &[u8; 4]| p[1] > 40 && p[0] < 20 && p[2] < 20;
        let (a, b) = (
            bbox(&moved, greenish).unwrap(),
            bbox(&twin, greenish).unwrap(),
        );
        assert_eq!([a[0] - b[0], a[1] - b[1]], [7, 5]);
        assert_eq!([a[2] - a[0], a[3] - a[1]], [b[2] - b[0], b[3] - b[1]]);
    }

    /// A wavy underline (`text-decoration-style: wavy`, painted as a path)
    /// moves with its text too.
    #[test]
    fn the_spans_wavy_underline_moves_with_its_text() {
        let inner = r#"lead <span style="position: relative; {OFF}; text-decoration: underline wavy rgb(0, 255, 0)">text</span>"#;
        let (moved, twin) = (
            painted_case(inner, "top: 5px; left: 7px"),
            painted_case(inner, "top: 0px"),
        );
        let greenish = |p: &[u8; 4]| p[1] > 40 && p[0] < 20 && p[2] < 20;
        let (a, b) = (
            bbox(&moved, greenish).unwrap(),
            bbox(&twin, greenish).unwrap(),
        );
        assert_eq!([a[0] - b[0], a[1] - b[1]], [7, 5]);
        assert_eq!([a[2] - a[0], a[3] - a[1]], [b[2] - b[0], b[3] - b[1]]);
    }

    /// A container that clips its overflow clips the moved text at its
    /// padding box: the text moved 12px down in a 20px-tall `overflow:
    /// hidden` box draws nothing below it, as in Chrome. (The clip is left
    /// out only where nothing reaches past the box, so the moved lines must
    /// count as reaching past it.) The twin without the clip draws below.
    #[test]
    fn a_clipping_container_clips_the_moved_text() {
        let inner = r#"<span style="position: relative; top: 12px">text</span>"#;
        let below = |overflow: &str| {
            let mut c = Case::with(&format!("height: 27px; {overflow}"), inner);
            let px = pixels(&mut c.doc);
            let (_, cy) = rinch_dom::paint::compute_absolute_position(&c.doc.tree, c.c, 1.0);
            let bottom = cy as usize + 27;
            bbox(&px, ink).map(|b| b[3] > bottom)
        };
        assert_eq!(
            below(""),
            Some(true),
            "positive control: unclipped, it draws below"
        );
        assert_eq!(
            below("overflow: hidden;"),
            Some(false),
            "clipped at the box"
        );
    }

    /// Text before the span is not moved: the ink of `lead ` alone is the
    /// same with and without the offset.
    #[test]
    fn the_text_before_the_span_is_painted_where_it_was() {
        let inner =
            r#"lead <span style="position: relative; {OFF}; color: rgb(0, 255, 0)">text</span>"#;
        let (moved, twin) = (
            painted_case(inner, "top: 5px; left: 7px"),
            painted_case(inner, "top: 0px"),
        );
        assert_eq!(bbox(&moved, ink), bbox(&twin, ink), "the black `lead `");
        let (a, b) = (bbox(&moved, green).unwrap(), bbox(&twin, green).unwrap());
        assert_eq!([a[0] - b[0], a[1] - b[1]], [7, 5], "the green `text`");
    }
}
