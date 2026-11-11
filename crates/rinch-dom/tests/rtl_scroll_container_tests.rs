//! Review of #2543 (#1995): a `direction: rtl` scroll container must keep its
//! overflow reachable. Chrome scrolls one from its right edge (`scrollLeft`
//! runs 0 → negative) and counts the overflow to its left as scrollable;
//! rinch's scroll offsets and scrollable overflow start at the left edge, so
//! #2543 at first laid a 300px child out at x = -100 in a 200px rtl
//! scroller and left that 100px unreachable (main: range 100). Until rinch
//! has Chrome's model (#2546), everything an rtl scroller holds is laid out
//! left to right (`RinchDocument::lays_out_rtl`): the range is Chrome's, and
//! the two ends show what Chrome's two ends show, the other way round.
//!
//! **Chrome 157** (`--headless=new`, `* { box-sizing: border-box }`,
//! 800x600, scale 1). The scroller `c` is `width:200px; height:60px;
//! overflow:auto; direction:rtl; scrollbar-width:none`. Per row: the scroll
//! range (`|min scrollLeft|`), and each mark's x relative to `c` at the
//! scroll start and at the far end (`scrollLeft = -1e6`).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
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
];

/// (name, rinch's start, rinch's far end, why) — rows whose views are not
/// Chrome's yet.
const KNOWN: &[(&str, &str, &str, &str)] = &[
    (
        "s_block",
        "a=0",
        "a=-100",
        "rinch scrolls an rtl scroller from its left edge (#2546)",
    ),
    (
        "s_flex",
        "a=0 b=150",
        "a=-100 b=50",
        "left to right, so mirrored (#2546)",
    ),
    (
        "s_nested",
        "a=0",
        "a=0",
        "a box overflowing its own parent is in no scroll range, ltr too (#2054)",
    ),
    ("s_grid", "a=0 b=150", "a=-100 b=50", "as s_flex (#2546)"),
    (
        "s_narrow",
        "a=0",
        "a=0",
        "an rtl scroller's content is laid out left to right (#2546)",
    ),
];

/// Rows whose range rinch does not see at all, ltr or rtl, for another
/// reason (#2054); `KNOWN` pins their views.
const NO_RANGE: &[&str] = &["s_nested"];

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

/// The range, and the views at the scroll start and at the far end.
fn measure(inner: &str, extra: &str, want: &str) -> (f64, String, String) {
    let (mut doc, c) = lay_out(inner, extra);
    let range = (doc.scroll_width(NodeId(c)) - 200.0).max(0.0);
    let start = shown(&doc, c, want);
    doc.set_scroll_left(NodeId(c), range);
    let far = shown(&doc, c, want);
    (range, start, far)
}

fn sorted(views: [&str; 2]) -> [String; 2] {
    let mut v = [views[0].to_string(), views[1].to_string()];
    v.sort();
    v
}

/// The whole overflow is reachable: the range is Chrome's, and for one box
/// the two ends show what Chrome's two ends show (in either order, until
/// #2546).
#[test]
fn an_rtl_scroller_reaches_all_its_overflow() {
    let mut bad = Vec::new();
    for (n, inner, extra, range, start, far) in CHROME {
        if NO_RANGE.contains(n) {
            continue;
        }
        let (got_range, got_start, got_far) = measure(inner, extra, start);
        if (got_range - range).abs() > 0.5 {
            bad.push(format!("{n}: range chrome[{range}] rinch[{got_range}]"));
        }
        // One mark: its two ends are what the two ends of the scroll show.
        // (Several marks are mirrored, so only the range compares; their
        // views are pinned in `KNOWN`.)
        let views = sorted([&got_start, &got_far]);
        if !start.contains(' ') && *range > 0.0 && views != sorted([start, far]) {
            bad.push(format!(
                "{n}: ends chrome[{start} | {far}] rinch[{got_start} | {got_far}]"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn known_gaps_pin_rinchs_current_views() {
    let mut bad = Vec::new();
    for (n, now_start, now_far, why) in KNOWN {
        let (_, inner, extra, _, start, far) = CHROME.iter().find(|r| r.0 == *n).unwrap();
        assert!((start, far) != (now_start, now_far), "{n} is a gap: {why}");
        let (_, got_start, got_far) = measure(inner, extra, start);
        if (got_start.as_str(), got_far.as_str()) != (*now_start, *now_far) {
            bad.push(format!(
                "{n} ({why}): pinned[{now_start} | {now_far}] rinch[{got_start} | {got_far}]"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Turning a laid-out rtl box into a scroll container (and back) re-lays its
/// content out as a fresh layout does: `overflow` is not inherited, so the
/// content's own Taffy styles must be re-synced.
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
/// margin: 0 }`: its root scroller is exempt from the left-to-right fallback
/// (review of #2543), so top-level rows start at the right. `body` is the
/// body's extra style. (name, body html, body style, Chrome 157 `mark=x`).
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

/// Known gap (#2546): an rtl page's own horizontal overflow is laid out to
/// the left of the viewport, and rinch's root scroll range counts only
/// overflow to the right. Chrome 157, a 1200px block on an 800px rtl page:
/// `scrollWidth` 1200, `scrollLeft` 0 → -400, the block at x = -400 at the
/// start and 0 at the far end. rinch puts it at -400 with no range to reach
/// its left 400px.
#[test]
fn known_gap_an_rtl_pages_left_overflow_is_unreachable() {
    let doc = lay_out_rtl_page(
        r#"<div style="height:30px"><div data-m="a" style="width:1200px;height:20px"></div></div>"#,
        "",
    );
    assert_eq!(
        page_x(&doc, "a=0"),
        "a=-400",
        "laid out as Chrome lays it out"
    );
    let range = [doc.tree.html_id, doc.tree.body_id, doc.tree.root_id]
        .into_iter()
        .map(|id| doc.scroll_width(NodeId(id)) - 800.0)
        .fold(0.0_f64, f64::max);
    assert!(
        range < 1.0,
        "rinch scrolls an rtl page's left overflow ({range}): #2546 is fixed, move this to Chrome's 400"
    );
}
