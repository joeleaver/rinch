//! #1372 — a `position: fixed` box whose ancestor has a `transform` (or
//! `contain: paint`/`layout`, a `filter`, `backdrop-filter`, `perspective`, or
//! a `will-change` naming one of those) is contained by that ancestor, not by
//! the viewport (CSS Transforms 1 §2, Filter Effects 1 §5, CSS Containment 2
//! §3): its insets, percentages, margins and static position are of the
//! ancestor's padding box, the ancestor's transform moves it, and it scrolls
//! with the ancestor's content and with the page.
//!
//! Every number in `CHROME` is **Chrome 151**'s (Playwright 1.62's
//! `chrome-headless-shell`, standards mode, `* { box-sizing: border-box }`,
//! `body { margin: 0 }`, an 800x600 viewport, device scale factor 1),
//! `getBoundingClientRect` in the viewport: `mark=x,y,width,height`. No row
//! holds text. Sizes are off round numbers so that an answer measured against
//! the wrong box (the viewport, the layout parent, the border box) cannot
//! coincide with the right one.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The transformed ancestor most rows use: offset from the viewport, with a
/// padding and a border so the padding box is neither the border box nor the
/// content box.
const T: &str = "transform:translateX(0);margin:13px 0 0 41px;width:333px;height:211px;padding:7px;border:3px solid black";

/// (name, html, Chrome 151). `{T}` is replaced by [`T`].
const CHROME: &[(&str, &str, &str)] = &[
    (
        "l01_offsets",
        r#"<div style="{T}"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=55,25,57,23",
    ),
    (
        "l02_percent_size_insets",
        r#"<div style="{T}"><div data-m="a" style="position:fixed;left:5%;top:3%;width:37%;height:21%"></div></div>"#,
        "a=60.34,22.14,120.98,43.05",
    ),
    (
        "l03_inset_zero",
        r#"<div style="{T}"><div data-m="a" style="position:fixed;inset:0"></div></div>"#,
        "a=44,16,327,205",
    ),
    (
        "l04_right_bottom",
        r#"<div style="{T}"><div data-m="a" style="position:fixed;right:17px;bottom:19px;width:57px;height:23px"></div></div>"#,
        "a=297,179,57,23",
    ),
    (
        "l05_margins",
        r#"<div style="{T}"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px;margin:5px 0 0 7.5%"></div></div>"#,
        "a=79.52,30,57,23",
    ),
    (
        "l06_static_position",
        r#"<div style="{T}"><div style="height:17px"></div><div data-m="a" style="position:fixed;width:57px;height:23px;margin-left:5px"></div></div>"#,
        "a=56,40,57,23",
    ),
    (
        "l07_static_y_inset_x",
        r#"<div style="{T}"><div style="height:17px"></div><div data-m="a" style="position:fixed;left:13px;width:57px;height:23px"></div></div>"#,
        "a=57,40,57,23",
    ),
    // The transformed div is the containing block, not the `position:
    // relative` parent between them.
    (
        "l08_non_parent_cb",
        r#"<div style="{T}"><div style="position:relative;left:9px;top:4px;margin:6px 0 0 19px;padding:5px;height:100px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div></div>"#,
        "a=55,25,57,23",
    ),
    (
        "l09_non_parent_static",
        r#"<div style="{T}"><div style="position:relative;left:9px;top:4px;margin:6px 0 0 19px;padding:5px;height:100px"><div data-m="a" style="position:fixed;width:57px;height:23px"></div></div></div>"#,
        "a=84,38,57,23",
    ),
    (
        "l10_translate",
        r#"<div style="transform:translate(23px,11px);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=75,33,57,23",
    ),
    (
        "l11_nested_transforms",
        r#"<div style="transform:translate(5px,7px);margin:13px 0 0 41px;width:333px;height:211px;padding:3px"><div style="margin-left:17px;padding-top:6px"><div style="transform:translate(0);margin-left:21px;width:201px;height:101px;border:2px solid"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div></div></div>"#,
        "a=100,40,57,23",
    ),
    // A fixed box is not itself a containing block of fixed boxes.
    (
        "l12_fixed_in_fixed_in_transform",
        r#"<div style="{T}"><div data-m="b" style="position:fixed;left:11px;top:9px;width:157px;height:123px"><div data-m="a" style="position:fixed;left:3px;top:5px;width:57px;height:23px"></div></div></div>"#,
        "a=47,21,57,23 b=55,25,157,123",
    ),
    (
        "l13_scale",
        r#"<div style="transform:scale(1.5);transform-origin:0 0;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=57.5,26.5,85.5,34.5",
    ),
    (
        "l15_shrink_to_fit",
        r#"<div style="{T}"><div data-m="a" style="position:fixed;left:11px;top:9px"><div style="width:77px;height:13px"></div></div></div>"#,
        "a=55,25,77,13",
    ),
    (
        "l16_abs_in_contained_fixed",
        r#"<div style="{T}"><div style="position:fixed;left:11px;top:9px;width:157px;height:123px"><div data-m="a" style="position:absolute;right:3px;bottom:5px;width:57px;height:23px"></div></div></div>"#,
        "a=152,120,57,23",
    ),
    (
        "l17_flex_parent_transform",
        r#"<div style="{T};display:flex;justify-content:center;align-items:flex-end"><div data-m="a" style="position:fixed;width:57px;height:23px"></div></div>"#,
        "a=179,191,57,23",
    ),
    (
        "l18_percent_margin_padding",
        r#"<div style="{T}"><div data-m="a" style="position:fixed;left:3px;right:7px;top:9px;height:23px;margin:0 4.5% 0 2.5%;padding-left:10%"></div></div>"#,
        "a=55.17,25,294.13,23",
    ),
    (
        "l19_auto_height_grandparent",
        r#"<div style="transform:translate(7px,3px);padding:11px 0 0 19px"><div style="position:relative;margin-left:23px;width:111px"><div data-m="a" style="position:fixed;right:5px;bottom:7px;width:57px;height:23px"></div></div></div>"#,
        "a=745,-16,57,23",
    ),
    // `auto` margins share what the insets leave in the ancestor.
    (
        "l20_auto_margins_centre",
        r#"<div style="transform:translateX(0);margin:40px 0 0 50px;width:300px;height:250px"><div data-m="a" style="position:fixed;inset:0;margin:auto;width:203px;height:101px"></div></div>"#,
        "a=98.5,114.5,203,101",
    ),
    (
        "l21_auto_margins_between_insets",
        r#"<div style="transform:translateX(0);margin:40px 0 0 50px;width:300px;height:250px"><div data-m="a" style="position:fixed;left:13px;right:17px;top:0;margin:0 auto;width:111px;height:10px"></div></div>"#,
        "a=142.5,40,111,10",
    ),
    // The other properties that contain a fixed box.
    (
        "p00_contain_paint",
        r#"<div style="contain:paint;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=52,22,57,23",
    ),
    (
        "p01_filter",
        r#"<div style="filter:brightness(1);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=52,22,57,23",
    ),
    (
        "p02_perspective",
        r#"<div style="perspective:500px;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=52,22,57,23",
    ),
    (
        "p03_will_change_transform",
        r#"<div style="will-change:transform;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=52,22,57,23",
    ),
    (
        "p04_will_change_filter",
        r#"<div style="will-change:filter;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=52,22,57,23",
    ),
    (
        "p05_backdrop_filter",
        r#"<div style="backdrop-filter:blur(2px);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=52,22,57,23",
    ),
    (
        "p06_contain_layout",
        r#"<div style="contain:layout;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=52,22,57,23",
    ),
    // …and some that do not.
    (
        "n01_opacity_does_not",
        r#"<div style="opacity:0.5;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=11,9,57,23",
    ),
    (
        "n02_will_change_opacity_does_not",
        r#"<div style="will-change:opacity;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=11,9,57,23",
    ),
    (
        "n03_inline_transform_does_not",
        r#"<span style="transform:translateX(5px);margin-left:41px"><b data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></b></span>"#,
        "a=11,9,57,23",
    ),
    (
        "n04_contain_size_does_not",
        r#"<div style="contain:size;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "a=11,9,57,23",
    ),
];

/// (name, html, the element scrolled — `page` for the page — by how much,
/// Chrome 151 after the scroll).
const SCROLLED: &[(&str, &str, &str, f64, &str)] = &[
    // The transformed scroller is the containing block: the box is its
    // content and scrolls with it.
    (
        "s01_transformed_scroller",
        r#"<div id="s" style="{T};overflow:auto"><div style="height:900px"></div><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div>"#,
        "s",
        47.0,
        "a=55,-22,57,23",
    ),
    // A scroller between the box and its containing block does not carry it.
    (
        "s02_scroller_between",
        r#"<div style="{T}"><div id="s" style="overflow:auto;height:150px;margin-left:13px"><div style="height:900px"></div><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div></div>"#,
        "s",
        47.0,
        "a=55,25,57,23",
    ),
    // The page's scroll carries it, where a viewport-fixed box stays put.
    (
        "s03_page_scroll",
        r#"<div style="{T}"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div><div style="height:2000px"></div>"#,
        "page",
        37.0,
        "a=55,-12,57,23",
    ),
    (
        "s04_static_in_transformed_scroller",
        r#"<div id="s" style="{T};overflow:auto"><div style="height:31px"></div><div data-m="a" style="position:fixed;width:57px;height:23px"></div><div style="height:900px"></div></div>"#,
        "s",
        47.0,
        "a=51,7,57,23",
    ),
];

fn lay_out(html: &str, viewport: (f32, f32)) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("* { box-sizing: border-box; }");
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, &html.replace("{T}", T));
    doc.append_child(body, wrap);
    doc.resolve_layout(viewport.0, viewport.1);
    doc
}

fn one(doc: &RinchDocument, selector: &str) -> u64 {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

/// `mark=x,y,width,height` for every mark `want` names: the box as painted
/// (`painted_border_box`, the transforms applied), in the viewport.
fn dump(doc: &RinchDocument, want: &str) -> String {
    want.split(' ')
        .map(|part| {
            let m = part.split_once('=').unwrap().0;
            let id = one(doc, &format!("[data-m={m}]"));
            let r = rinch_dom::paint::painted_border_box(&doc.tree, id, 1.0);
            format!(
                "{m}={},{},{},{}",
                num(r.x0),
                num(r.y0),
                num(r.width()),
                num(r.height())
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Within the pixel grid: rinch puts every box edge on a whole pixel, Chrome
/// reports the unrounded box, so a width can be a whole pixel off.
fn close(want: &str, got: &str) -> bool {
    let nums = |s: &str| -> Vec<f64> {
        s.split(' ')
            .flat_map(|p| p.split_once('=').unwrap().1.split(','))
            .map(|v| v.parse().unwrap())
            .collect()
    };
    let (w, g) = (nums(want), nums(got));
    w.len() == g.len() && w.iter().zip(&g).all(|(a, b)| (a - b).abs() <= 1.0)
}

fn scroll(doc: &mut RinchDocument, target: &str, by: f64) {
    let id = if target == "page" {
        doc.tree.body_id
    } else {
        one(doc, &format!("#{target}"))
    };
    doc.set_scroll_top(NodeId(id), by);
    assert_eq!(doc.scroll_top(NodeId(id)), by, "{target} scrolls by {by}");
}

#[test]
fn fixed_boxes_in_a_transformed_ancestor_match_chrome_151() {
    let bad: Vec<_> = CHROME
        .iter()
        .filter_map(|(n, h, want)| {
            let got = dump(&lay_out(h, VIEWPORT), want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(CHROME.len() >= 25, "the table is being read");
    assert!(
        bad.is_empty(),
        "{} of {}:\n{bad:#?}",
        bad.len(),
        CHROME.len()
    );
}

/// Each row laid out at another viewport first: an incremental layout lands
/// where a fresh one does.
#[test]
fn a_relayout_lands_where_a_fresh_layout_does() {
    let bad: Vec<_> = CHROME
        .iter()
        .filter_map(|(n, h, want)| {
            let mut doc = lay_out(h, (VIEWPORT.0 + 41.0, VIEWPORT.1 + 37.0));
            doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
            let got = dump(&doc, want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn a_contained_fixed_box_scrolls_as_chrome_151_scrolls_it() {
    let bad: Vec<_> = SCROLLED
        .iter()
        .filter_map(|(n, h, target, by, want)| {
            let mut doc = lay_out(h, VIEWPORT);
            scroll(&mut doc, target, *by);
            let got = dump(&doc, want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A transformed scroller that holds a fixed box counts it in its scrollable
/// overflow, as its containing block (Chrome 151: `scrollHeight` 371 for a
/// box at `top: 300px`, 71px tall, in a 205px-tall padding box).
#[test]
fn a_contained_fixed_box_is_its_containing_blocks_overflow() {
    let html = r#"<div id="s" style="transform:translate(0);overflow:auto;width:333px;height:211px;border:3px solid"><div data-m="a" style="position:fixed;left:11px;top:300px;width:57px;height:71px"></div></div>"#;
    let doc = lay_out(html, VIEWPORT);
    let s = one(&doc, "#s");
    assert_eq!(doc.scroll_height(NodeId(s)), 371.0);
    // Without the transform it is the viewport's, and adds nothing (Chrome:
    // `scrollHeight` 205, the client height; rinch's `scroll_height` is the
    // content's extent alone, here none).
    let doc = lay_out(&html.replace("transform:translate(0);", ""), VIEWPORT);
    let s = one(&doc, "#s");
    assert!(doc.scroll_height(NodeId(s)) <= doc.client_height(NodeId(s)));
    assert_eq!(doc.client_height(NodeId(s)), 205.0);
}

/// The containing block follows the ancestor's style: a transform added after
/// the first layout contains the box, and taken away again releases it to the
/// viewport — with no restyle of the box itself, and through a wrapper that is
/// not restyled either.
#[test]
fn adding_and_removing_the_transform_moves_the_box() {
    let mut doc = lay_out(
        r#"<div id="t" style="margin:13px 0 0 41px;width:333px;height:211px;padding:7px;border:3px solid"><div><div data-m="a" style="position:fixed;left:11px;top:9px;width:37%;height:23px"></div></div></div>"#,
        VIEWPORT,
    );
    assert_eq!(dump(&doc, "a=0"), "a=11,9,296,23", "against the viewport");
    let t = one(&doc, "#t");
    let style = "margin:13px 0 0 41px;width:333px;height:211px;padding:7px;border:3px solid";
    doc.set_attribute(
        NodeId(t),
        "style",
        &format!("{style};transform:translate(5px,3px)"),
    );
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    // 41+3+11+5, 13+3+9+3; 37% of the 327px padding box.
    assert_eq!(dump(&doc, "a=0"), "a=60,28,121,23", "against the div");
    doc.set_attribute(NodeId(t), "style", style);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        dump(&doc, "a=0"),
        "a=11,9,296,23",
        "against the viewport again"
    );
}

/// A fixed subtree moved under a transformed ancestor after the first layout
/// is contained by it, and moved out again is not.
#[test]
fn moving_the_box_under_a_transform_contains_it() {
    let mut doc = lay_out(
        &format!(
            r#"<div id="t" style="{T}"></div><div id="u"><div id="w"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px"></div></div></div>"#
        ),
        VIEWPORT,
    );
    assert_eq!(dump(&doc, "a=0"), "a=11,9,57,23");
    let (t, u, w) = (one(&doc, "#t"), one(&doc, "#u"), one(&doc, "#w"));
    doc.append_child(NodeId(t), NodeId(w));
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(dump(&doc, "a=0"), "a=55,25,57,23");
    doc.append_child(NodeId(u), NodeId(w));
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(dump(&doc, "a=0"), "a=11,9,57,23");
}

/// `position: fixed` given to a box already under a transformed ancestor,
/// and taken away.
#[test]
fn becoming_fixed_under_a_transform_is_contained() {
    let mut doc = lay_out(
        r#"<div style="transform:translate(0);margin:13px 0 0 41px;width:333px;height:211px"><div id="a" data-m="a" style="width:57px;height:23px"></div></div>"#,
        VIEWPORT,
    );
    let a = one(&doc, "#a");
    assert_eq!(dump(&doc, "a=0"), "a=41,13,57,23", "in flow");
    doc.set_attribute(
        NodeId(a),
        "style",
        "position:fixed;left:11px;top:9px;width:57px;height:23px",
    );
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(dump(&doc, "a=0"), "a=52,22,57,23", "fixed, in the div");
}

/// A `transform` transition that finishes at `none` stops containing the
/// fixed box on the frame the tick finishes it, with no cascade of the box's
/// own and the dirty set drained as the shell drains it: the tick re-answers
/// for the fixed boxes under it, as it re-syncs the absolute ones.
#[test]
fn a_transition_that_finishes_at_none_releases_the_box() {
    let mut doc = RinchDocument::new();
    doc.set_animation_clock_ms(Some(1_000_000.0));
    doc.load_css(
        "body { margin: 0 } .t { margin: 13px 0 0 41px; width: 203px; height: 80px; \
         transition: transform 1000ms linear; transform: translateX(20px); } \
         .t.on { transform: none; }",
    );
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "t");
    doc.append_child(body, div);
    let mid = doc.create_element("div");
    doc.append_child(div, mid);
    let f = doc.create_element("div");
    doc.set_attribute(f, "data-m", "a");
    doc.set_attribute(
        f,
        "style",
        "position:fixed;left:11px;top:9px;width:37%;height:23px",
    );
    doc.append_child(mid, f);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(dump(&doc, "a=0"), "a=72,22,75,23", "in the div, translated");

    doc.set_attribute(div, "class", "t on");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let w = dump(&doc, "a=0");
    assert!(w.ends_with(",22,75,23"), "mid-run: still in the div: {w}");
    let _ = doc.take_dirty_nodes();
    for t in doc
        .tree
        .active_transitions
        .get_mut(&div.0)
        .expect("the class change started a transform transition")
        .values_mut()
    {
        t.start_time_ms -= 10_000.0;
    }
    doc.tick_transitions();
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(dump(&doc, "a=0"), "a=11,9,296,23", "finished at none");
}

/// `a > b > c > fixed`, `a` transformed and `b` paint-contained: `c` starting
/// or stopping being a containing block moves the box between `c` and `b`
/// although it stays contained throughout — every contained box under `c` is
/// re-synced, not only those whose containment flips (review of #2723).
/// Chrome 151: `64,36,62.3,23` in `c`, `47,25,93.3,23` in `b` (every box a
/// `flow-root`, so no margin collapses through one).
#[test]
fn a_box_that_stays_contained_follows_its_containing_block_to_another_ancestor() {
    let html = |c: &str| {
        format!(
            r#"<div style="display:flow-root;transform:translate(0);margin:7px 0 0 13px;width:401px;height:301px"><div style="display:flow-root;contain:paint;margin:9px 0 0 23px;width:301px;height:201px"><div id="c" style="display:flow-root;{c}margin:11px 0 0 17px;width:201px;height:101px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:31%;height:23px"></div></div></div></div>"#
        )
    };
    let rest = "display:flow-root;margin:11px 0 0 17px;width:201px;height:101px";
    let in_c = "a=64,36,62.3,23";
    let in_b = "a=47,25,93.3,23";

    let mut doc = lay_out(&html("contain:paint;"), VIEWPORT);
    let got = dump(&doc, in_c);
    assert!(close(in_c, &got), "in c: {got}");
    let c = one(&doc, "#c");
    doc.set_attribute(NodeId(c), "style", rest);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let got = dump(&doc, in_b);
    assert!(close(in_b, &got), "c no longer contains it: {got}");

    let mut doc = lay_out(&html(""), VIEWPORT);
    let got = dump(&doc, in_b);
    assert!(close(in_b, &got), "in b: {got}");
    let c = one(&doc, "#c");
    doc.set_attribute(NodeId(c), "style", &format!("contain:paint;{rest}"));
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let got = dump(&doc, in_c);
    assert!(close(in_c, &got), "c contains it now: {got}");
}

/// A `filter` (or `backdrop-filter`, or `will-change: filter`) applies to a
/// non-atomic inline element too, and makes a `<span>` the containing block
/// of a fixed box inside it: its padding box, from its first fragment.
/// `contain: paint`, `perspective` and `transform` do not apply to one.
/// Chrome 151, the bundled Inter as `ProbeFace`, `font: 16px/20px`: the span's
/// box is `45,7,23,20`.
#[test]
fn a_filtered_inline_span_contains_a_fixed_box() {
    use parley::fontique::{Blob, FontInfoOverride};
    const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
    let mut bad = Vec::new();
    for (f, want) in [
        ("filter:brightness(1)", "a=56,16,31,23"),
        ("backdrop-filter:blur(1px)", "a=56,16,31,23"),
        ("will-change:filter", "a=56,16,31,23"),
        ("contain:paint", "a=11,9,31,23"),
        ("perspective:100px", "a=11,9,31,23"),
        ("transform:translateX(3px)", "a=11,9,31,23"),
    ] {
        let mut doc = RinchDocument::new();
        doc.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(FACE)),
            Some(FontInfoOverride {
                family_name: Some("ProbeFace"),
                ..Default::default()
            }),
        );
        doc.load_css("* { box-sizing: border-box; }");
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0; font: 16px/20px ProbeFace");
        let wrap = doc.create_element("div");
        doc.set_inner_html(
            wrap,
            &format!(
                r#"<div style="margin:7px 0 0 13px;width:401px"><i style="display:inline-block;width:29px;height:10px"></i><span style="{f};padding-left:5px;margin-left:3px">yy<b data-m="a" style="position:fixed;left:11px;top:9px;width:31px;height:23px"></b></span></div>"#
            ),
        );
        doc.append_child(body, wrap);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let got = dump(&doc, want);
        if !close(want, &got) {
            bad.push(format!("{f}: chrome[{want}] rinch[{got}]"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
