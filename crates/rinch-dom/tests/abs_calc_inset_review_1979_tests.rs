//! Review fixtures for PR #1979 (#1789/#1974): calc() percentages on an
//! absolute box whose containing block is content-sized, in the shapes the
//! PR's own file does not cover: a containing block further up than the
//! parent, a hoisted box, a `display: contents` parent, a transformed parent,
//! a nested absolute, min-size, and the Stepper separator's declarations.
//! Expected rects (viewport, `getBoundingClientRect`) are Chromium
//! 153.0.8010.12's with `*{box-sizing:border-box} body{margin:0}`.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

fn rect(html: &str, m: &str) -> [f32; 4] {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0;");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    let found = rinch_dom::testing::query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1);
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, found[0], 1.0);
    let l = doc.tree.get(found[0]).unwrap().layout;
    [x as f32, y as f32, l.width, l.height]
}

fn check(html: &str, m: &str, want: [f32; 4]) {
    let got = rect(html, m);
    for i in 0..4 {
        assert!(
            (got[i] - want[i]).abs() < 0.51,
            "{m}: got {got:?}, Chrome {want:?}"
        );
    }
}

/// static parent, auto-height positioned grandparent: the basis is the grandparent (#386 path, unchanged).
#[test]
fn a_static_parent_rel_grand_auto() {
    check(
        "<div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black;width:406px\"><div data-m=\"p\" style=\"padding:4px 9px\"><div style=\"height:28px\"></div><div data-m=\"a\" style=\"position:absolute;width:20px;height:10px;top:calc(100% + 4px);left:calc(50% - 10px)\"></div></div><div style=\"height:13px\"></div></div>",
        "a",
        [193.0, 66.0, 20.0, 10.0],
    );
}

/// static parent, fixed-height positioned grandparent (unchanged).
#[test]
fn b_static_parent_rel_grand_fixed() {
    check(
        "<div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black;width:406px;height:100px\"><div data-m=\"p\" style=\"padding:4px 9px\"><div style=\"height:28px\"></div><div data-m=\"a\" style=\"position:absolute;width:20px;height:10px;top:calc(100% + 4px);left:calc(50% - 10px)\"></div></div></div>",
        "a",
        [193.0, 101.0, 20.0, 10.0],
    );
}

/// static flex parent, auto-height positioned grandparent, bottom/right (unchanged).
#[test]
fn b2_static_flexparent_rel_grand_auto() {
    check(
        "<div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black;width:406px\"><div data-m=\"p\" style=\"display:flex;padding:4px 9px\"><div style=\"height:28px;width:30px\"></div><div data-m=\"a\" style=\"position:absolute;width:20px;height:10px;bottom:calc(100% - 4px);right:calc(25% + 3px)\"></div></div><div style=\"height:13px\"></div></div>",
        "a",
        [280.0, -3.0, 20.0, 10.0],
    );
}

/// hoisted out of a static inline span into its positioned block (red at base: y 7).
#[test]
fn c_hoisted_from_static_span() {
    check(
        "<div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black;width:406px;font-size:10px;line-height:10px\"><div style=\"height:28px\"></div><span><div data-m=\"a\" style=\"position:absolute;width:20px;height:10px;top:calc(100% + 4px);left:calc(50% - 10px)\"></div></span></div>",
        "a",
        [193.0, 45.0, 20.0, 10.0],
    );
}

/// display: contents parent, positioned grandparent is the Taffy parent (red at base: y 7).
#[test]
fn d_display_contents_parent() {
    check(
        "<div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black;width:406px\"><div style=\"height:28px\"></div><div style=\"display:contents\"><div data-m=\"a\" style=\"position:absolute;width:20px;height:10px;top:calc(100% + 4px);left:calc(50% - 10px)\"></div></div></div>",
        "a",
        [193.0, 45.0, 20.0, 10.0],
    );
}

/// a position: relative display: contents wrapper establishes nothing (unchanged).
#[test]
fn e_contents_rel_in_static_in_rel() {
    check(
        "<div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black;width:406px\"><div style=\"padding:6px\"><div style=\"height:28px\"></div><div style=\"display:contents;position:relative\"><div data-m=\"a\" style=\"position:absolute;width:20px;height:10px;top:calc(100% + 4px);left:calc(50% - 10px)\"></div></div></div></div>",
        "a",
        [193.0, 57.0, 20.0, 10.0],
    );
}

/// min-height calc against an auto-height block (red at base: height 5).
#[test]
fn f2_minmax() {
    check(
        "<div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black;width:406px\"><div style=\"height:28px\"></div><div data-m=\"a\" style=\"position:absolute;top:0;left:0;width:10px;height:5px;min-height:calc(50% + 1px);min-width:calc(25% + 1px)\"></div></div>",
        "a",
        [3.0, 3.0, 101.0, 20.0],
    );
}

/// abs inside an auto-height abs box (red at base: y 6).
#[test]
fn k_nested_abs() {
    check(
        "<div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black;width:406px\"><div style=\"height:28px\"></div><div data-m=\"o\" style=\"position:absolute;top:2px;left:3px;width:calc(50% + 0px);padding:3px\"><div style=\"height:20px\"></div><div data-m=\"a\" style=\"position:absolute;top:calc(100% + 1px);left:calc(50% + 1px);width:calc(25% + 1px);height:4px\"></div></div></div>",
        "a",
        [107.0, 32.0, 51.0, 4.0],
    );
}

/// a transformed static parent is the containing block (red at base: y 7).
#[test]
fn l_transform_parent() {
    check(
        "<div style=\"position:relative;width:406px\"><div data-m=\"cb\" style=\"transform:translateX(0px);padding:5px 7px;border:3px solid black\"><div style=\"height:28px\"></div><div data-m=\"a\" style=\"position:absolute;width:20px;height:10px;top:calc(100% + 4px);left:calc(50% - 10px)\"></div></div></div>",
        "a",
        [193.0, 45.0, 20.0, 10.0],
    );
}

/// auto-height block that is a flex column item; own height calc (red at base: y 7, h 0).
#[test]
fn o_flex_item_rel_auto_in_column() {
    check(
        "<div style=\"display:flex;flex-direction:column;width:406px\"><div data-m=\"cb\" style=\"position:relative;padding:5px 7px;border:3px solid black\"><div style=\"height:28px\"></div><div data-m=\"a\" style=\"position:absolute;width:20px;height:calc(50% + 2px);top:calc(100% + 4px);left:calc(50% - 10px)\"></div></div></div>",
        "a",
        [193.0, 45.0, 20.0, 21.0],
    );
}

/// Stepper's horizontal separator (red at base: x 26).
#[test]
fn h_stepper_horizontal() {
    check(
        "<div style=\"width:600px;display:flex;flex-direction:column\"><div style=\"display:flex;gap:16px;flex-direction:row;align-items:flex-start\"><div data-m=\"s1\" style=\"display:flex;flex:1;position:relative;flex-direction:column;align-items:center\"><div style=\"width:36px;height:36px;flex-shrink:0;z-index:1\"></div><div style=\"width:80px;height:20px;margin-top:10px\"></div><div data-m=\"sep1\" style=\"position:absolute;top:calc(36px / 2);left:calc(50% + 36px / 2 + 8px);right:calc(-50% + 36px / 2 + 8px);height:2px\"></div></div><div data-m=\"s2\" style=\"display:flex;flex:1;position:relative;flex-direction:column;align-items:center\"><div style=\"width:36px;height:36px;flex-shrink:0;z-index:1\"></div><div style=\"width:80px;height:20px;margin-top:10px\"></div><div data-m=\"sep2\" style=\"position:absolute;top:calc(36px / 2);left:calc(50% + 36px / 2 + 8px);right:calc(-50% + 36px / 2 + 8px);height:2px\"></div></div><div data-m=\"s3\" style=\"display:flex;flex:0;position:relative;flex-direction:column;align-items:center\"><div style=\"width:36px;height:36px;flex-shrink:0;z-index:1\"></div><div style=\"width:80px;height:20px;margin-top:10px\"></div><div data-m=\"sep3\" style=\"position:absolute;display:none;top:calc(36px / 2);left:calc(50% + 36px / 2 + 8px);right:calc(-50% + 36px / 2 + 8px);height:2px\"></div></div></div></div>",
        "sep1",
        [148.0, 18.0, 192.0, 2.0],
    );
}
