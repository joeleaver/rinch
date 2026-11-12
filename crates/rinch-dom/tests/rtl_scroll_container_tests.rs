//! #2546: a `direction: rtl` scroll container scrolls from its right edge,
//! as Chrome does. Its scrollable overflow reaches to the left of its content
//! box (`scrollbar::content_extents` measures from the right edge leftward),
//! its horizontal offset (`scrollLeft`) is 0 at the start, where the content's
//! right end shows, and runs negative to `-range` (`scrollbar::scroll_bounds`),
//! and everything it holds is laid out right to left (#1995) — the
//! left-to-right fallback #2543 put in until this model existed is gone.
//!
//! **Chrome 157** (`--headless=new`, `* { box-sizing: border-box }`,
//! 800x600, scale 1). The scroller `c` is `width:200px; height:60px;
//! overflow:auto; direction:rtl; scrollbar-width:none`. Per row: the scroll
//! range (`-min scrollLeft`), and each mark's x relative to `c` at the
//! scroll start and at the far end (`scrollLeft = -1e6`).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::{ScrollbarAxis, scroll_bounds};
use rinch_dom::testing::query_selector;

const S: &str =
    "width:200px;height:60px;overflow:auto;direction:rtl;scrollbar-width:none;margin:5px 0 0 9px";

/// (name, inner html of `c`, extra style of `c`, Chrome range, Chrome start, Chrome far end).
const CHROME: &[(&str, &str, &str, f64, &str, &str)] = &[
    (
        "s_block",
        r#"<div data-m="a" style="width:300px;height:20px"></div>"#,
        "",
        100.0,
        "a=-100",
        "a=0",
    ),
    (
        "s_flex",
        r#"<div data-m="a" style="width:150px;height:20px;flex-shrink:0"></div><div data-m="b" style="width:150px;height:20px;flex-shrink:0"></div>"#,
        "display:flex",
        100.0,
        "a=50 b=-100",
        "a=150 b=0",
    ),
    (
        "s_nested",
        r#"<div><div data-m="a" style="width:300px;height:20px"></div></div>"#,
        "",
        100.0,
        "a=-100",
        "a=0",
    ),
    (
        "s_grid",
        r#"<div data-m="a" style="height:20px"></div><div data-m="b" style="height:20px"></div>"#,
        "display:grid;grid-template-columns:150px 150px",
        100.0,
        "a=50 b=-100",
        "a=150 b=0",
    ),
    (
        "s_narrow",
        r#"<div data-m="a" style="width:100px;height:20px"></div>"#,
        "",
        0.0,
        "a=100",
        "a=100",
    ),
    // `padding-left` is an rtl scroller's inline-end padding: it extends the
    // in-flow overflow, as `padding-right` does an ltr one's.
    (
        "s_padding",
        r#"<div data-m="a" style="width:300px;height:20px"></div>"#,
        "padding:0 20px 0 10px",
        130.0,
        "a=-120",
        "a=10",
    ),
    (
        "s_padding_border",
        r#"<div data-m="a" style="width:300px;height:20px"></div>"#,
        "padding:0 20px 0 10px;border:3px solid black",
        136.0,
        "a=-123",
        "a=13",
    ),
    // The margin box counts: `margin-left` reaches further left without
    // moving the box, `margin-right` moves it left.
    (
        "s_margin_left",
        r#"<div data-m="a" style="width:300px;height:20px;margin-left:30px"></div>"#,
        "",
        130.0,
        "a=-100",
        "a=30",
    ),
    (
        "s_margin_right",
        r#"<div data-m="a" style="width:300px;height:20px;margin-right:30px"></div>"#,
        "",
        130.0,
        "a=-130",
        "a=0",
    ),
    // Where it is drawn: an absolute box, by `left` or by `right`.
    (
        "s_abs_left",
        r#"<div data-m="a" style="position:absolute;top:0;left:-150px;width:100px;height:20px"></div>"#,
        "position:relative",
        150.0,
        "a=-150",
        "a=0",
    ),
    (
        "s_abs_right",
        r#"<div data-m="a" style="position:absolute;top:0;right:250px;width:100px;height:20px"></div>"#,
        "position:relative",
        150.0,
        "a=-150",
        "a=0",
    ),
    // A `relative` box drawn back inside the box overflows nothing.
    (
        "s_relative",
        r#"<div data-m="a" style="position:relative;left:-50px;width:150px;height:20px"></div>"#,
        "",
        0.0,
        "a=0",
        "a=0",
    ),
    // The scroller's own lines: an rtl line wider than the box is aligned to
    // its right edge and overflows to the left.
    (
        "s_ifc_line",
        r#"<span data-m="a" style="display:inline-block;width:300px;height:20px"></span>"#,
        "line-height:20px",
        100.0,
        "a=-100",
        "a=0",
    ),
    (
        "s_ifc_two_lines",
        r#"<span data-m="a" style="display:inline-block;width:250px;height:20px"></span><br><span data-m="b" style="display:inline-block;width:320px;height:20px"></span>"#,
        "line-height:20px",
        120.0,
        "a=-50 b=-120",
        "a=70 b=0",
    ),
    // A `nowrap` line is aligned within the box too: at its right edge, and
    // overflowing to the left (it sat at the left before).
    (
        "s_nowrap_short",
        r#"<span data-m="a" style="display:inline-block;width:10px;height:10px"></span>"#,
        "white-space:nowrap",
        0.0,
        "a=190",
        "a=190",
    ),
    (
        "s_nowrap_line",
        r#"<span data-m="a" style="display:inline-block;width:300px;height:10px"></span>"#,
        "white-space:nowrap",
        100.0,
        "a=-100",
        "a=0",
    ),
    // An anonymous block box's line, beside a block child.
    (
        "s_anon_line",
        r#"<div data-m="a" style="width:100px;height:20px"></div><span data-m="b" style="display:inline-block;width:250px;height:20px"></span>"#,
        "line-height:20px",
        50.0,
        "a=100 b=-50",
        "a=150 b=0",
    ),
    // An ltr box in an rtl scroller is still placed by the scroller.
    (
        "s_ltr_child",
        r#"<div data-m="a" style="direction:ltr;width:300px;height:20px"></div>"#,
        "",
        100.0,
        "a=-100",
        "a=0",
    ),
];

/// (name, rinch's range, start, far end, why) — rows whose views are not
/// Chrome's yet, for reasons outside the scroll model.
const KNOWN: &[(&str, f64, &str, &str, &str)] = &[
    (
        "s_nested",
        0.0,
        "a=-100",
        "a=-100",
        "a box overflowing its own parent is in no scroll range, ltr too (#2054)",
    ),
    (
        "s_anon_line",
        49.0,
        "a=100 b=-49",
        "a=149 b=0",
        "an anonymous box's lines are aligned across 201px of a 200px box, \
         so its rtl line starts a pixel right of Chrome's (#2546 review notes)",
    ),
];

fn lay_out(inner: &str, extra: &str) -> (RinchDocument, u64) {
    let mut doc = RinchDocument::new();
    doc.load_css("* { box-sizing: border-box; }");
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(
        wrap,
        &format!(r#"<div data-m="c" style="{S};{extra}">{inner}</div>"#),
    );
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    let c = query_selector(&doc.tree, "[data-m=c]")[0];
    (doc, c)
}

/// Each mark's x relative to `c` as it is shown at `c`'s current scroll
/// (`compute_absolute_position` takes the scroll offsets off).
fn shown(doc: &RinchDocument, c: u64, want: &str) -> String {
    let (cx, _) = rinch_dom::paint::compute_absolute_position(&doc.tree, c, 1.0);
    want.split(' ')
        .map(|part| {
            let m = part.split_once('=').unwrap().0;
            let id = query_selector(&doc.tree, &format!("[data-m={m}]"))[0];
            let (x, _) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
            format!("{m}={}", (x - cx).round())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The range, and the views at the scroll start and at the far end — the
/// far end is the bounds' `min`, which must be `-range` with a `max` of 0.
fn measure(inner: &str, extra: &str, want: &str) -> (f64, String, String) {
    let (mut doc, c) = lay_out(inner, extra);
    let (min, max) = scroll_bounds(&doc.tree, c, ScrollbarAxis::Horizontal);
    assert_eq!(max, 0.0, "an rtl scroller's range ends at 0");
    assert_eq!(doc.scroll_left(NodeId(c)), 0.0, "it starts at 0");
    let start = shown(&doc, c, want);
    doc.set_scroll_left(NodeId(c), min);
    let far = shown(&doc, c, want);
    (-min, start, far)
}

/// The range and both ends' views are Chrome's.
#[test]
fn an_rtl_scroller_scrolls_from_its_right_edge_as_chrome() {
    let mut bad = Vec::new();
    for (n, inner, extra, range, start, far) in CHROME {
        if KNOWN.iter().any(|k| k.0 == *n) {
            continue;
        }
        let (got_range, got_start, got_far) = measure(inner, extra, start);
        if (got_range - range).abs() > 0.5
            || (got_start.as_str(), got_far.as_str()) != (*start, *far)
        {
            bad.push(format!(
                "{n}: chrome[{range}: {start} | {far}] rinch[{got_range}: {got_start} | {got_far}]"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn known_gaps_pin_rinchs_current_views() {
    let mut bad = Vec::new();
    for (n, now_range, now_start, now_far, why) in KNOWN {
        let (_, inner, extra, range, start, far) = CHROME.iter().find(|r| r.0 == *n).unwrap();
        assert!(
            (range, start, far) != (now_range, now_start, now_far),
            "{n} is a gap: {why}"
        );
        let (got_range, got_start, got_far) = measure(inner, extra, start);
        if (got_range, got_start.as_str(), got_far.as_str()) != (*now_range, *now_start, *now_far) {
            bad.push(format!(
                "{n} ({why}): pinned[{now_range}: {now_start} | {now_far}] \
                 rinch[{got_range}: {got_start} | {got_far}]"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// An ltr scroller keeps its `0..=range` bounds.
#[test]
fn an_ltr_scroller_still_scrolls_from_its_left_edge() {
    let (doc, c) = lay_out(
        r#"<div style="width:300px;height:20px"></div>"#,
        "direction:ltr",
    );
    assert_eq!(
        scroll_bounds(&doc.tree, c, ScrollbarAxis::Horizontal),
        (0.0, 100.0)
    );
}

/// Turning a laid-out rtl box into a scroll container (and back) re-lays its
/// content out as a fresh layout does. (Under #2543's left-to-right fallback
/// this needed a re-sync of the content's Taffy styles; kept as a guard.)
#[test]
fn toggling_overflow_on_an_rtl_box_agrees_with_a_fresh_layout() {
    let inner = r#"<div><div data-m="a" style="width:300px;height:20px"></div></div>"#;
    let style = |overflow: &str| format!("{S};overflow:{overflow}");
    for (from, to) in [("visible", "auto"), ("auto", "visible")] {
        let (mut doc, c) = lay_out(inner, &format!("overflow:{from}"));
        doc.set_attribute(NodeId(c), "style", &style(to));
        doc.resolve_layout(800.0, 600.0);
        let (fresh, fc) = lay_out(inner, &format!("overflow:{to}"));
        assert_eq!(
            shown(&doc, c, "a=0"),
            shown(&fresh, fc, "a=0"),
            "overflow {from} -> {to}"
        );
    }
}

/// An rtl page (`direction: rtl` on `<html>`) at 800x600 with `body {
/// margin: 0 }`: top-level rows start at the right. `body` is the body's
/// extra style. (name, body html, body style, Chrome 157 `mark=x`).
const ROOT: &[(&str, &str, &str, &str)] = &[
    (
        "root_flex",
        r#"<div style="display:flex;height:30px"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"#,
        "",
        "a=760 b=710",
    ),
    (
        "root_grid",
        r#"<div style="display:grid;grid-template-columns:100px 150px;height:30px"><div data-m="a" style="height:20px"></div><div data-m="b" style="height:20px"></div></div>"#,
        "",
        "a=700 b=550",
    ),
    // `body`'s `overflow` is the viewport's: still the root scroller.
    (
        "root_body_overflow",
        r#"<div style="height:30px"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"#,
        "overflow:auto",
        "a=760 b=750",
    ),
];

fn lay_out_rtl_page(body_html: &str, body_style: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("* { box-sizing: border-box; }");
    let html = doc.tree.html_id;
    doc.set_attribute(NodeId(html), "style", "direction: rtl");
    let body = doc.body();
    doc.set_attribute(body, "style", &format!("margin: 0; {body_style}"));
    doc.set_inner_html(body, body_html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn page_x(doc: &RinchDocument, want: &str) -> String {
    want.split(' ')
        .map(|part| {
            let m = part.split_once('=').unwrap().0;
            let id = query_selector(&doc.tree, &format!("[data-m={m}]"))[0];
            let (x, _) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
            format!("{m}={}", x.round())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn an_rtl_pages_top_level_rows_start_at_the_right() {
    let bad: Vec<_> = ROOT
        .iter()
        .filter_map(|(n, html, style, want)| {
            let got = page_x(&lay_out_rtl_page(html, style), want);
            (got != *want).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// An rtl page's own horizontal overflow is laid out to the left of the
/// viewport and scrolls from the right (#2546). Chrome 157, a 1200px block on
/// an 800px rtl page with `body { overflow: auto }`: `scrollWidth` 1200,
/// `scrollLeft` 0 → -400, the block at x = -400 at the start and 0 at the far
/// end.
#[test]
fn an_rtl_pages_left_overflow_scrolls_from_the_right() {
    let mut doc = lay_out_rtl_page(
        r#"<div data-m="a" style="width:1200px;height:20px"></div>"#,
        "overflow:auto",
    );
    let body = doc.tree.body_id;
    assert_eq!(page_x(&doc, "a=0"), "a=-400", "the start");
    assert_eq!(
        scroll_bounds(&doc.tree, body, ScrollbarAxis::Horizontal),
        (-400.0, 0.0)
    );
    doc.set_scroll_left(NodeId(body), -400.0);
    assert_eq!(page_x(&doc, "a=0"), "a=0", "the far end");
}

/// An offset goes on the device grid toward 0 on both sides (#1543), so an
/// rtl container's negative `scrollLeft` clamped to `-range..=0` stays in it:
/// a floor would push `-100.3` at scale 1.5 out to `-100.67`.
#[test]
fn a_negative_offset_snaps_toward_zero() {
    use rinch_dom::node::snap_to_device;
    assert_eq!(snap_to_device(100.3, 1.5), 100.0);
    assert_eq!(snap_to_device(-100.3, 1.5), -100.0);
    assert_eq!(snap_to_device(-100.0, 1.5), -100.0, "on the grid already");
    assert_eq!(snap_to_device(-0.4, 2.0), 0.0);
}

/// The scroller's own text, in a line wider than it: rtl, so the line is
/// aligned to the right edge and overflows to the left (parley), and that
/// overflow is the range — whatever the host's font makes of its width, so
/// it is measured against rinch's own line.
#[test]
fn an_rtl_scrollers_own_text_overflow_is_its_range() {
    let (doc, c) = lay_out(
        "abcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz",
        "white-space:nowrap;font-size:16px;line-height:20px",
    );
    let line = doc.tree.nodes[c]
        .text_layout
        .as_ref()
        .expect("an IFC root")
        .layout
        .width() as f64;
    assert!(
        line > 300.0,
        "the premise: a line well past the box ({line})"
    );
    let (min, max) = scroll_bounds(&doc.tree, c, ScrollbarAxis::Horizontal);
    assert_eq!(max, 0.0);
    assert!(
        (-min - (line - 200.0)).abs() <= 1.0,
        "range {} for a {line}px line in a 200px box",
        -min
    );
}
