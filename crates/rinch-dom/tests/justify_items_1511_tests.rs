//! #1511 — `justify-items` and `justify-self`: where a grid places its items
//! (and an auto-inset absolute child's static position) on the inline axis.
//! rinch's `ComputedStyle` carried neither, so Taffy's grid saw `normal`
//! everywhere: `place-items: center` centred an item vertically only.
//!
//! Every number is **Chrome 156**'s (`--headless=new`, standards mode,
//! `* { box-sizing: border-box }`, 800x600, device scale factor 1),
//! `getBoundingClientRect` relative to the container `c`'s border box:
//! `mark=x,y,width,height`. The grid has `padding: 7px 0 0 11px` and two
//! columns of different widths (100px, 150px) holding a 40px and a 50px item,
//! so no item's start, centre and end coincide. No text is laid out.
//!
//! Keyword mapping, as Chrome answers it: `left` / `right` are `start` /
//! `end` (rinch lays out left to right), `legacy <kw>` is `<kw>` (and is
//! inherited by a descendant's `legacy`, which rinch does not do yet:
//! `KNOWN`), `self-*` / `flex-*` are `start` /
//! `end`, `safe` falls back to `start` when the item overflows its area,
//! `justify-self: normal` / `stretch` is `start` for a sized item. A flex
//! container ignores both properties.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

/// (name, html, Chrome 156).
const CHROME: &[(&str, &str, &str)] = &[
    (
        "g_ji_normal",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=111,7,50,20",
    ),
    (
        "g_ji_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:start"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=111,7,50,20",
    ),
    (
        "g_ji_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=41,7,40,20 b=161,7,50,20",
    ),
    (
        "g_ji_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:end"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=211,7,50,20",
    ),
    (
        "g_ji_flex-end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:flex-end"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=211,7,50,20",
    ),
    (
        "g_ji_self-end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:self-end"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=211,7,50,20",
    ),
    (
        "g_ji_left",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:left"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=111,7,50,20",
    ),
    (
        "g_ji_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:right"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=211,7,50,20",
    ),
    (
        "g_ji_legacy_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:legacy center"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=41,7,40,20 b=161,7,50,20",
    ),
    (
        "g_ji_legacy_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:legacy right"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=211,7,50,20",
    ),
    (
        "g_ji_safe_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:safe end"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=211,7,50,20",
    ),
    // From the #1989 review: an item wider than its track, so `safe` matters.
    (
        "g_safe_end_overflow",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:safe end"><div data-m="a" style="width:140px;height:20px"></div></div>"##,
        "a=11,7,140,20",
    ),
    (
        "g_js_last_baseline",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:end"><div data-m="a" style="width:40px;height:20px;justify-self:last baseline"></div></div>"##,
        "a=71,7,40,20",
    ),
    // An auto-width item, so a dropped `flex-start` (stretch) shows.
    (
        "g_auto_width_flex_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:flex-start"><div data-m="a" style="height:20px"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=11,7,30,20",
    ),
    (
        "g_ji_unsafe_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:unsafe end"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=211,7,50,20",
    ),
    (
        "g_ji_anchor_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:anchor-center"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=111,7,50,20",
    ),
    (
        "g_js_end_over_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div data-m="a" style="width:40px;height:20px;justify-self:end"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=161,7,50,20",
    ),
    (
        "g_js_start_over_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:end"><div data-m="a" style="width:40px;height:20px;justify-self:start"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=211,7,50,20",
    ),
    (
        "g_js_center_alone",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;"><div data-m="a" style="width:40px;height:20px;justify-self:center"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=41,7,40,20 b=111,7,50,20",
    ),
    (
        "g_js_normal_over_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div data-m="a" style="width:40px;height:20px;justify-self:normal"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=161,7,50,20",
    ),
    (
        "g_js_stretch_over_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div data-m="a" style="width:40px;height:20px;justify-self:stretch"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=161,7,50,20",
    ),
    (
        "g_js_auto_over_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:end"><div data-m="a" style="width:40px;height:20px;justify-self:auto"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=211,7,50,20",
    ),
    (
        "g_auto_width_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div data-m="a" style="height:20px"><div style="width:30px;height:10px"></div></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=46,7,30,20 b=161,7,50,20",
    ),
    (
        "g_auto_width_stretch",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:stretch"><div data-m="a" style="height:20px"><div style="width:30px;height:10px"></div></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,100,20 b=111,7,50,20",
    ),
    (
        "g_auto_width_normal",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;"><div data-m="a" style="height:20px"><div style="width:30px;height:10px"></div></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,100,20 b=111,7,50,20",
    ),
    (
        "g_place_items",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:minmax(0,1fr);place-items:center"><div data-m="a" style="width:130px;height:30px"></div></div>"##,
        "a=140.5,28.5,130,30",
    ),
    (
        "g_inline_grid",
        r##"<div data-m="c" style="margin:9px 0 0 14px"><span data-m="q" style="display:inline-grid;width:400px;height:80px;padding:7px 0 0 11px;grid-template-columns:100px 150px;justify-items:end"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></span></div>"##,
        "q=0,0,400,80 a=71,7,40,20 b=211,7,50,20",
    ),
    (
        "g_nested_inherit",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div data-m="a" style="width:90px;height:40px;display:grid;justify-items:end;grid-template-columns:70px"><div data-m="n" style="width:20px;height:10px"></div></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=16,7,90,40 n=66,7,20,10 b=161,7,50,20",
    ),
    (
        "g_legacy_inherit",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;justify-items:legacy center"><div data-m="a" style="width:90px;height:40px;display:grid;grid-template-columns:70px"><div data-m="n" style="width:20px;height:10px"></div></div></div>"##,
        "a=160.5,7,90,40 n=185.5,7,20,10",
    ),
    (
        "f_ji_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-items:center"><div data-m="a" style="width:40px;height:20px;justify-self:end"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=51,7,50,20",
    ),
    (
        "f_ji_end_column",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column;justify-items:end"><div data-m="a" style="width:40px;height:20px;justify-self:center"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=11,27,50,20",
    ),
    (
        "b_ji_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;justify-items:center"><div data-m="a" style="width:40px;height:20px;"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=185.5,7,40,20 b=180.5,27,50,20",
    ),
    (
        "b_js_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:end"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=11,27,50,20",
    ),
    (
        "abs_ji_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=180,0,40,30",
    ),
    (
        "abs_js_end",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;justify-self:end;align-self:end"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=360,50,40,30",
    ),
    (
        "abs_anc_ji_center",
        r##"<div data-m="c" style="position:relative;width:420px;height:120px;padding:3px 0 0 5px;margin:9px 0 0 14px"><div data-m="p" style="width:400px;height:80px;padding:7px 0 0 11px;margin:6px 0 0 4px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px"></div><div style="width:50px;height:20px"></div></div></div>"##,
        "p=9,9,400,80 a=194.5,16,40,30",
    ),
    (
        "g_legacy_grid_inherit",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:legacy center"><div data-m="a" style="width:90px;height:40px;display:grid;grid-template-columns:70px"><div data-m="n" style="width:20px;height:10px"></div></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=16,7,90,40 n=41,7,20,10 b=161,7,50,20",
    ),
    (
        "g_safe_center_overflow",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:safe center"><div data-m="a" style="width:40px;height:20px;width:140px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,140,20 b=161,7,50,20",
    ),
    (
        "g_unsafe_center_overflow",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div data-m="a" style="width:40px;height:20px;width:140px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=-9,7,140,20 b=161,7,50,20",
    ),
    (
        "g_js_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;"><div data-m="a" style="width:40px;height:20px;justify-self:right"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=71,7,40,20 b=111,7,50,20",
    ),
    (
        "g_js_left_over_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-items:end"><div data-m="a" style="width:40px;height:20px;justify-self:left"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=211,7,50,20",
    ),
    (
        "g_js_anchor_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;"><div data-m="a" style="width:40px;height:20px;justify-self:anchor-center"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=41,7,40,20 b=111,7,50,20",
    ),
    (
        "g_js_baseline",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;"><div data-m="a" style="width:40px;height:20px;justify-self:baseline"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=111,7,50,20",
    ),
];

/// (name, rinch's answer, why) — rows of `CHROME` that are not Chrome's yet,
/// each for a reason that is not this issue's.
const KNOWN: &[(&str, &str, &str)] = &[
    (
        "b_ji_center",
        "a=11,7,40,20 b=11,27,50,20",
        "Taffy's block layout reads neither property; Chrome aligns block-level boxes (#1985)",
    ),
    (
        "b_js_end",
        "a=11,7,40,20 b=11,27,50,20",
        "as b_ji_center (#1985)",
    ),
    (
        "g_legacy_inherit",
        "a=11,7,90,40 n=11,7,20,10",
        "`a` is a block-level box (#1985), and `legacy` is not inherited (#1987)",
    ),
    (
        "g_legacy_grid_inherit",
        "a=16,7,90,40 n=16,7,20,10 b=161,7,50,20",
        "Stylo's servo build does not inherit `legacy` into a descendant's `legacy` (#1987)",
    ),
    (
        "g_js_anchor_center",
        "a=11,7,40,20 b=111,7,50,20",
        "Stylo drops `anchor-center` at parse: its anchor-positioning pref is off (#1987)",
    ),
    (
        "abs_anc_ji_center",
        "p=9,9,400,80 a=189,9,40,30",
        "an unpositioned grid's static position is its padding box, Chrome's its content box (#1986)",
    ),
];

fn lay_out(html: &str, height: f32) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, height);
    doc
}

fn one(doc: &RinchDocument, selector: &str) -> u64 {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

fn screen(doc: &RinchDocument, id: u64) -> (f32, f32) {
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    (x as f32, y as f32)
}

fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `mark=x,y,width,height` for every mark `want` names, relative to `c`.
fn dump(doc: &RinchDocument, want: &str) -> String {
    let (cx, cy) = screen(doc, one(doc, "[data-m=c]"));
    want.split(' ')
        .map(|part| {
            let m = part.split_once('=').unwrap().0;
            let id = one(doc, &format!("[data-m={m}]"));
            let (x, y) = screen(doc, id);
            let l = doc.tree.get(id).unwrap().layout;
            format!(
                "{m}={},{},{},{}",
                num(x - cx),
                num(y - cy),
                num(l.width),
                num(l.height)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Within the pixel grid: rinch rounds every edge, Chrome does not.
fn close(want: &str, got: &str) -> bool {
    let nums = |s: &str| -> Vec<f32> {
        s.split(' ')
            .flat_map(|p| p.split_once('=').unwrap().1.split(','))
            .map(|v| v.parse().unwrap())
            .collect()
    };
    let (w, g) = (nums(want), nums(got));
    w.len() == g.len() && w.iter().zip(&g).all(|(a, b)| (a - b).abs() <= 0.75)
}

#[test]
fn justify_items_and_self_match_chrome_156() {
    let mut rows = 0;
    let bad: Vec<_> = CHROME
        .iter()
        .filter(|(n, ..)| !KNOWN.iter().any(|k| k.0 == *n))
        .filter_map(|(n, h, want)| {
            rows += 1;
            let got = dump(&lay_out(h, 600.0), want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(rows > 25, "the table is being read: {rows} rows");
    assert!(bad.is_empty(), "{} of {rows}:\n{bad:#?}", bad.len());
}

#[test]
fn known_gaps_pin_rinchs_current_answer() {
    let bad: Vec<_> = KNOWN
        .iter()
        .filter_map(|(n, now, why)| {
            let (_, h, chrome) = CHROME.iter().find(|c| c.0 == *n).unwrap();
            let got = dump(&lay_out(h, 600.0), chrome);
            assert!(!close(chrome, now), "{n} is a gap: {why}");
            (!close(now, &got)).then(|| format!("{n} ({why}): pinned[{now}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// `justify-items` and then `justify-self` written onto a laid-out grid move
/// its items as a fresh layout places them: the change reaches Taffy's style
/// and is a layout change.
#[test]
fn a_restyle_agrees_with_a_fresh_layout() {
    let row = |n: &str| CHROME.iter().find(|r| r.0 == n).unwrap();
    let style_of = |html: &str, mark: &str| {
        html.split(&format!(r#"data-m="{mark}" style=""#))
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    for (before, after, mark) in [
        ("g_ji_normal", "g_ji_end", "c"),
        ("g_ji_end", "g_ji_center", "c"),
        ("g_ji_center", "g_js_end_over_center", "a"),
    ] {
        let (_, before_html, _) = row(before);
        let (_, after_html, want) = row(after);
        let mut doc = lay_out(before_html, 600.0);
        let id = one(&doc, &format!("[data-m={mark}]"));
        doc.set_attribute(
            rinch_core::dom::NodeId(id),
            "style",
            &style_of(after_html, mark),
        );
        doc.resolve_layout(800.0, 600.0);
        let got = dump(&doc, want);
        assert!(
            close(want, &got),
            "{before} -> {after}: chrome[{want}] rinch[{got}]"
        );
    }
}
