//! A preserved tab advances to the next tab stop (#1122), and is preserved by
//! `break-spaces` and collapsed by `pre-line` (#1131).
//!
//! css-text-3 §4.2: a preserved tab is rendered as a horizontal shift that
//! lines its end up with the next tab stop, the stops at multiples of
//! `tab-size` (8, the initial value) times the advance of a space of the
//! block container, `letter-spacing` and `word-spacing` included. rinch-dom
//! used to lay every tab out as four spaces, whatever its column.
//!
//! Every number is Chrome 155's on the same markup: the bundled Inter
//! (`Inter-Regular.ttf`, loaded with `@font-face` there and registered as
//! `ProbeFace` here), `font: 16px/24px`, `body { margin: 0 }`, each case a
//! `width: 600px` block. A caret's x is a collapsed `Range`'s left edge less
//! the block's, for each offset of the text node. In Inter at 16px a space
//! is 4.5 wide, so a stop falls every 36px; `a` is 8.98, `b` 9.80, `c` 9.14.
//!
//! What Chrome 155 does, as these fixtures pin it:
//! - the stop is the first multiple of the interval *strictly* past the
//!   tab's start, measured in logical order from the line's start edge,
//!   before `text-align` moves the line;
//! - when that stop is less than half a space away, the tab goes on to the
//!   next one (an inline-block 33.8 wide puts the tab's end at 72, one 33.7
//!   wide at 36);
//! - the interval is the block container's: a 16px span inside a 32px block
//!   has 72px stops, and a span's own `letter-spacing` does not change it
//!   while the block's does;
//! - the tab takes no `letter-spacing` of its own;
//! - under `pre-wrap` and `break-spaces` a tab that overflows hangs at the
//!   end of its line;
//! - `normal`, `nowrap` and `pre-line` collapse it to a space.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::dom_cursor_to_ifc_offset;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const TOL: f32 = 0.05;

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    d
}

fn text_nodes(d: &RinchDocument, id: u64, out: &mut Vec<u64>) {
    let n = d.tree.get(id).unwrap();
    if matches!(n.kind, rinch_dom::node::NodeKind::Text(_)) {
        out.push(id);
    }
    for &c in &n.children {
        text_nodes(d, c, out);
    }
}

/// A `width: 600px` block styled `style` holding `html`, laid out; returns
/// the document and the block.
fn block(style: &str, html: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    d.set_attribute(body, "style", "margin: 0");
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        &format!("font: 16px/24px ProbeFace; width: 600px; {style}"),
    );
    d.append_child(body, c);
    d.set_inner_html(c, html);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

/// The caret `(x, y)` for every DOM offset of every text node of the block,
/// in document order — what the Chrome probe reads.
fn carets(style: &str, html: &str) -> Vec<(f32, f32)> {
    let (d, c) = block(style, html);
    let mut texts = Vec::new();
    text_nodes(&d, c.0, &mut texts);
    let il = d
        .tree
        .get(c.0)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("the block is an IFC root");
    let mut out = Vec::new();
    for t in texts {
        let len = match &d.tree.get(t).unwrap().kind {
            rinch_dom::node::NodeKind::Text(td) => td.content.len(),
            _ => unreachable!(),
        };
        for o in 0..=len {
            let flat = dom_cursor_to_ifc_offset(&il.text_ranges, t, o).unwrap();
            out.push(d.query_caret_position(c.0, flat).unwrap());
        }
    }
    out
}

fn xs(style: &str, html: &str) -> Vec<f32> {
    carets(style, html).into_iter().map(|c| c.0).collect()
}

#[track_caller]
fn assert_xs(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() <= TOL, "got {got:?}, Chrome 155 {want:?}");
    }
}

/// The width of a shrink-to-fit `inline-block` around `html`.
fn shrink_width(style: &str, html: &str) -> f32 {
    let (d, c) = block(
        "",
        &format!("<span id=\"m\" style=\"display: inline-block; {style}\">{html}</span>"),
    );
    let m = d.tree.get(c.0).unwrap().children[0];
    d.tree.get(m).unwrap().layout.width
}

const ABC: &str = "a\tb\tc";

#[test]
fn a_pre_tab_advances_to_the_next_stop() {
    assert_xs(
        &xs("white-space: pre", ABC),
        &[0.0, 8.98, 36.0, 45.80, 72.0, 81.14],
    );
}

#[test]
fn a_pre_wrap_tab_advances_to_the_next_stop() {
    assert_xs(
        &xs("white-space: pre-wrap", ABC),
        &[0.0, 8.98, 36.0, 45.80, 72.0, 81.14],
    );
}

#[test]
fn a_break_spaces_tab_is_preserved_issue_1131() {
    assert_xs(
        &xs("white-space: break-spaces", "a\tb"),
        &[0.0, 8.98, 36.0, 45.80],
    );
}

#[test]
fn a_collapsing_white_space_makes_a_tab_one_space() {
    for ws in ["normal", "nowrap", "pre-line"] {
        assert_xs(
            &xs(&format!("white-space: {ws}"), "a\tb"),
            &[0.0, 8.98, 13.48, 23.28],
        );
    }
}

#[test]
fn tabs_at_the_line_start_take_a_whole_interval_each() {
    assert_xs(&xs("white-space: pre", "\t\tb"), &[0.0, 36.0, 72.0, 81.80]);
}

#[test]
fn a_tab_close_to_its_stop_still_ends_there() {
    // 10 `m`s end at 140.16, 3.84 short of 144: more than half a space.
    let got = xs("white-space: pre", "mmmmmmmmmm\tb");
    assert_xs(&got[10..], &[140.16, 144.0, 153.80]);
    let got = xs("white-space: pre", "xxxxxxxxxxxxxxx\tb");
    assert_xs(&got[15..], &[131.02, 144.0, 153.80]);
}

#[test]
fn a_stop_less_than_half_a_space_away_is_skipped() {
    let ib = |w: &str| {
        format!("<span style=\"display: inline-block; height: 10px; width: {w}px\"></span>\tb")
    };
    // Each case's text node is the tab and `b`: offsets 0..=2. Half a space
    // is 2.25: Chrome puts the tab's end at 72 after a box 33.8 wide and at
    // 36 after one 33.7 wide. rinch lays a box out at whole pixels, so the
    // pins are the two sides of that threshold it can reach: 2px (34) and
    // 3px (33) short of 36.
    assert_xs(&xs("white-space: pre", &ib("34")), &[34.0, 72.0, 81.80]);
    assert_xs(&xs("white-space: pre", &ib("33")), &[33.0, 36.0, 45.80]);
}

#[test]
fn letter_and_word_spacing_widen_the_interval_and_not_the_tab() {
    assert_xs(
        &xs("white-space: pre; letter-spacing: 3px", "a\tb"),
        &[0.0, 11.98, 60.0, 72.80],
    );
    assert_xs(
        &xs("white-space: pre; word-spacing: 5px", "a\tb"),
        &[0.0, 8.98, 76.0, 85.80],
    );
}

#[test]
fn the_interval_is_the_block_containers() {
    // A 16px span in a 32px block: 72px stops, from the block's space.
    assert_xs(
        &xs(
            "white-space: pre; font-size: 32px",
            "<span style=\"font-size: 16px\">a\tb</span>",
        ),
        &[0.0, 8.98, 72.0, 81.80],
    );
    // A span's own letter-spacing leaves the stops alone...
    assert_xs(
        &xs(
            "white-space: pre",
            "<span style=\"letter-spacing: 10px\">a\tb</span>",
        ),
        &[0.0, 18.98, 36.0, 55.80],
    );
    // ...and the block's moves them, though the span resets its own.
    assert_xs(
        &xs(
            "white-space: pre; letter-spacing: 10px",
            "<span style=\"letter-spacing: 0\">a\tb</span>",
        ),
        &[0.0, 8.98, 116.0, 125.80],
    );
}

#[test]
fn text_align_moves_the_line_after_its_tabs_are_set() {
    // `pre-wrap`: a `pre` root aligns within its own widest line, not the
    // block (#1237), tab or no tab.
    assert_xs(
        &xs("white-space: pre-wrap; text-align: right", "a\tb"),
        &[554.20, 563.19, 590.20, 600.0],
    );
    assert_xs(
        &xs("white-space: pre-wrap; text-align: center", "a\tb"),
        &[277.09, 286.08, 313.09, 322.89],
    );
}

/// rinch-dom has no `direction` property; a paragraph is right-to-left when
/// its text starts with a right-to-left letter. Chrome 155 puts the stops of
/// such a line on its right (`a\tb\tc` under `direction: rtl`: `b`'s left
/// edge at 600 - 36 - 9.80 = 554.20, `c`'s at 600 - 72 - 9.14 = 518.86), the
/// tab's start counted in logical order. The relation pinned here holds for
/// whichever face draws the Hebrew: each tab's left edge — its end, on a
/// right-to-left line — is a whole number of intervals from the line's
/// right end.
#[test]
fn a_right_to_left_line_sets_its_stops_from_the_right() {
    let text = "\u{5d0}\t\u{5d1}\u{5d1}\u{5d1}\t\u{5d2}";
    let (d, c) = block("white-space: pre", text);
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let line = il.layout.lines().next().unwrap();
    let right = line.metrics().offset + line.metrics().advance;
    // Each cluster's visual left edge, by its first byte.
    let mut left = std::collections::HashMap::new();
    for item in line.items() {
        if let parley::PositionedLayoutItem::GlyphRun(g) = item {
            let mut x = g.offset();
            for cl in g.run().visual_clusters() {
                left.insert(cl.text_range().start, x);
                x += cl.advance();
            }
        }
    }
    let (first, second) = (left[&2], left[&9]);
    for (x, stop) in [(first, 36.0), (second, 72.0)] {
        assert!(
            (right - x - stop).abs() <= TOL,
            "{first} {second}, right {right}"
        );
    }
}

#[test]
fn an_overflowing_tab_hangs_at_the_soft_wrap() {
    for ws in ["pre-wrap", "break-spaces"] {
        let got = carets(&format!("white-space: {ws}; width: 60px"), "aaaa\tbb");
        let want = [
            (0.0, 0.0),
            (8.98, 0.0),
            (17.97, 0.0),
            (26.95, 0.0),
            (35.94, 0.0),
            (72.0, 0.0),
            (9.80, 24.0),
            (19.59, 24.0),
        ];
        // The caret after the tab is the start of the next line in Chrome
        // too; compare that one by line only.
        assert_eq!(got.len(), want.len(), "{ws}: {got:?}");
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            if i == 5 {
                continue;
            }
            assert!(
                (g.0 - w.0).abs() <= TOL && (g.1 - w.1).abs() <= 1.0,
                "{ws}: got {got:?}, Chrome 155 {want:?}"
            );
        }
        let (d, c) = block(&format!("white-space: {ws}; width: 60px"), "aaaa\tbb");
        assert_eq!(d.tree.get(c.0).unwrap().layout.height, 48.0, "{ws}");
    }
}

#[test]
fn a_trailing_tab_is_counted_in_a_shrink_to_fit_width() {
    let w = shrink_width("white-space: pre", "a\t");
    assert!((w - 36.0).abs() <= TOL, "{w}");
    // Chrome: 54.94. rinch-dom gives an atomic inline whole pixels.
    let w = shrink_width("white-space: pre", "ab\tcd");
    assert!((w - 54.94).abs() <= 0.5, "{w}");
}

#[test]
fn a_tab_is_one_flat_byte() {
    let (d, c) = block("white-space: pre", ABC);
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.text_content.len(), ABC.len());
    assert_eq!(d.tab_flat_bytes(), 1);
}

#[test]
fn a_press_on_a_tab_lands_on_its_nearer_side() {
    let (d, c) = block("white-space: pre", "a\tb");
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let at = |x: f32| rinch_dom::text_query::byte_offset_from_position(&il.layout, x, 12.0);
    // The tab spans 8.98..36: a press left of its middle is before it.
    assert_eq!(at(12.0), 1);
    assert_eq!(at(33.0), 2);
    assert_eq!(at(40.0), 2);
}

#[test]
fn a_selected_tab_is_highlighted_to_its_stop() {
    let (d, c) = block("white-space: pre", "a\tb");
    let rects = d.query_selection_rects(c.0, 1, 2);
    assert_eq!(rects.len(), 1, "{rects:?}");
    let (x, _, w, _) = rects[0];
    assert!(
        (x - 8.98).abs() <= TOL && (x + w - 36.0).abs() <= TOL,
        "{rects:?}"
    );
}

/// A line cut by `text-overflow: ellipsis` keeps the lines as laid out, its
/// tabs at their stops (#1100: the cut is made at paint; it used to be drawn
/// from a flat rebuild, where a tab was one space wide, so a paragraph with
/// a tab was clipped with no "…").
#[test]
fn an_ellipsis_cut_keeps_its_tabs_at_their_stops() {
    let style = "white-space: pre-wrap; width: 100px; overflow: hidden; text-overflow: ellipsis";
    let html = "a\tb bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let got = xs(style, html);
    // The first line, `a`, the tab and `b`, fits; the second is cut.
    assert_xs(&got[..4], &[0.0, 8.98, 36.0, 45.80]);
    assert_cut(style, html);
}

/// The whole-line cut of a `pre` root (#1586): Chrome 155 keeps the tab at
/// its stop, `b` at 36, and draws the "…". The flat rebuild before #1100 drew
/// the tab one space wide, `b` at 13.48.
#[test]
fn a_pre_lines_ellipsis_keeps_its_tab_at_its_stop() {
    let style = "white-space: pre; width: 100px; overflow: hidden; text-overflow: ellipsis";
    let html = "a\tbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let got = xs(style, html);
    assert_xs(&got[..3], &[0.0, 8.98, 36.0]);
    assert_cut(style, html);
}

/// A line of the block is cut and ends in "…".
#[track_caller]
fn assert_cut(style: &str, html: &str) {
    let (d, c) = block(style, html);
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let painted = il.painted_lines().expect("a line is cut");
    assert!(
        painted.iter().any(|l| l.ends_with('\u{2026}')),
        "{painted:?}"
    );
}
/// Sixteen `xxx\t` words in a 400px `pre-wrap` block: each ends at a stop
/// 36px on, so the first line holds eleven and ends at 396, and the second
/// starts with `lll` — Chrome 155's carets, every one. Laid out with each
/// tab one space wide, the whole text fits one line; only once the tabs are
/// at their stops does it wrap, and the second line's tabs are set again
/// from where the wrap leaves them.
#[test]
fn tabs_are_set_again_on_the_lines_their_widths_make() {
    let words = "aaa\tbbb\tccc\tddd\teee\tfff\tggg\thhh\tiii\tjjj\tkkk\tlll\tmmm\tnnn\tooo\tppp";
    let got = carets("white-space: pre-wrap; width: 400px", words);
    #[rustfmt::skip]
    let chrome: [f32; 64] = [
        0.0, 8.98, 17.97, 26.95, 36.0, 45.80, 55.59, 65.39, 72.0, 81.14, 90.28, 99.42,
        108.0, 117.80, 127.59, 137.39, 144.0, 153.33, 162.66, 171.98, 180.0, 185.30,
        190.59, 196.52, 216.0, 225.81, 235.63, 245.44, 252.0, 261.45, 270.92, 280.38,
        288.0, 291.88, 295.75, 299.63, 324.0, 327.88, 331.75, 335.63, 360.0, 368.78,
        377.56, 386.34, 396.0, 3.88, 7.75, 11.63, 36.0, 50.02, 64.03, 78.05, 108.0,
        117.45, 126.91, 136.36, 144.0, 153.59, 163.19, 172.78, 180.0, 189.80, 199.59,
        209.39,
    ];
    assert_eq!(got.len(), chrome.len());
    for (i, (g, w)) in got.iter().zip(chrome).enumerate() {
        // Char 44 starts the second line; Chrome draws its caret at the end
        // of the first, rinch at the start of the second.
        if i == 44 {
            assert_eq!(g, &(0.0, 24.0), "char 44: {got:?}");
            continue;
        }
        let line = if i < 44 { 0.0 } else { 24.0 };
        assert!(
            (g.0 - w).abs() <= TOL && g.1 == line,
            "char {i} at {g:?}, Chrome 155 ({w}, {line}): {got:?}"
        );
    }
}
