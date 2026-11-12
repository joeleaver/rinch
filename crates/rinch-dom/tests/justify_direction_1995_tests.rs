//! #1995, #1985, #1987 — `direction`, block-layout `justify-self` /
//! `justify-items`, physical `left` / `right`, `justify-content`'s
//! `start` / `end` / `left` / `right`, and `safe` on the block axis: values
//! the style→Taffy conversion used to drop or flatten.
//!
//! `direction` never reached Taffy, so a `direction: rtl` grid, flex row or
//! block container was laid out left to right (#1995); Taffy's block layout
//! read neither `justify-self` nor `justify-items` (#1985); and `align-*`
//! lost `safe` (#1987). An absolutely positioned box's static position in a
//! block container ignored its `justify-self` (#2541, the `abs_*` rows), and
//! an rtl line ordered its inline-blocks left to right (#2542, the
//! `r_inline_block*` rows).
//!
//! Every number is **Chrome 157**'s (`--headless=new`, standards mode,
//! `* { box-sizing: border-box }`, 800x600, device scale factor 1),
//! `getBoundingClientRect` relative to the container `c`'s border box:
//! `mark=x,y,width,height`. The container has `padding: 7px 0 0 11px`, so
//! no item's start, centre and end coincide. No text is laid out.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

/// (name, html, Chrome 157).
const CHROME: &[(&str, &str, &str)] = &[
    (
        "r_g_normal",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=250,7,50,20",
    ),
    (
        "r_g_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-items:start"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=250,7,50,20",
    ),
    (
        "r_g_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-items:end"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=300,7,40,20 b=150,7,50,20",
    ),
    (
        "r_g_left",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-items:left"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=300,7,40,20 b=150,7,50,20",
    ),
    (
        "r_g_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-items:right"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=250,7,50,20",
    ),
    (
        "r_g_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-items:center"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=330,7,40,20 b=200,7,50,20",
    ),
    (
        "r_g_js_left",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl"><div data-m="a" style="width:40px;height:20px;justify-self:left"></div><div data-m="b" style="width:50px;height:20px;justify-self:right"></div></div>"##,
        "a=300,7,40,20 b=250,7,50,20",
    ),
    (
        "r_g_jc_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-content:start"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=250,7,50,20",
    ),
    (
        "r_g_jc_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-content:end"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=221,7,40,20 b=111,7,50,20",
    ),
    (
        "r_g_jc_left",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-content:left"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=221,7,40,20 b=111,7,50,20",
    ),
    (
        "r_g_jc_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-content:right"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=250,7,50,20",
    ),
    (
        "r_g_jc_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;direction:rtl;justify-content:center"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=290.5,7,40,20 b=180.5,7,50,20",
    ),
    (
        "r_f_row",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;direction:rtl"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=310,7,50,20",
    ),
    (
        "r_f_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;direction:rtl;justify-content:flex-end"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=61,7,40,20 b=11,7,50,20",
    ),
    (
        "r_f_jc_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;direction:rtl;justify-content:end"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=61,7,40,20 b=11,7,50,20",
    ),
    (
        "r_f_jc_left",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;direction:rtl;justify-content:left"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=61,7,40,20 b=11,7,50,20",
    ),
    (
        "r_f_jc_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;direction:rtl;justify-content:right"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=310,7,50,20",
    ),
    (
        "r_f_col_ai_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column;direction:rtl;align-items:flex-start"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=350,27,50,20",
    ),
    (
        "r_f_col",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column;direction:rtl"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=350,27,50,20",
    ),
    (
        "r_b",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=350,27,50,20",
    ),
    (
        "r_b_auto",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="height:20px;margin-left:30px"></div></div>"##,
        "a=41,7,359,20",
    ),
    (
        "r_b_mr",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="width:40px;height:20px;margin-right:13px"></div></div>"##,
        "a=347,7,40,20",
    ),
    (
        "r_b_inner_ltr",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="width:40px;height:20px;direction:ltr"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "f_jc_left",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:left"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=51,7,50,20",
    ),
    (
        "f_jc_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:right"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=310,7,40,20 b=350,7,50,20",
    ),
    (
        "f_jc_rr_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:row-reverse;justify-content:start"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=61,7,40,20 b=11,7,50,20",
    ),
    (
        "f_jc_rr_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:row-reverse;justify-content:end"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=310,7,50,20",
    ),
    (
        "f_jc_rr_left",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:row-reverse;justify-content:left"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=61,7,40,20 b=11,7,50,20",
    ),
    (
        "f_jc_cr_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column-reverse;justify-content:start"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,27,40,20 b=11,7,50,20",
    ),
    (
        "g_jc_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 150px;justify-content:right"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=150,7,40,20 b=250,7,50,20",
    ),
    (
        "g_jc_stretch",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:auto 150px;justify-content:stretch"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=250,7,50,20",
    ),
    (
        "g_jc_normal_auto",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:auto 150px"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=250,7,50,20",
    ),
    (
        "b_ji_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;justify-items:center"><div data-m="a" style="width:40px;height:20px"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=185.5,7,40,20 b=180.5,27,50,20",
    ),
    (
        "b_js_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:end"></div><div data-m="b" style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,20 b=11,27,50,20",
    ),
    (
        "b_js_center_auto",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="height:20px;justify-self:center"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=190.5,7,30,20",
    ),
    (
        "b_js_end_auto_text",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="height:20px;justify-self:end"><span style="display:inline-block;width:30px;height:10px"></span></div></div>"##,
        "a=370,7,30,20",
    ),
    (
        "b_js_stretch_auto",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="height:20px;justify-self:stretch"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=11,7,389,20",
    ),
    (
        "b_js_left",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="width:40px;height:20px;justify-self:left"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "b_js_start_rtl",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="width:40px;height:20px;justify-self:start"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "b_js_end_rtl",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "b_js_end_margin",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:end;margin-right:13px;margin-left:5px"></div></div>"##,
        "a=347,7,40,20",
    ),
    (
        "b_js_center_margin",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:center;margin-left:30px"></div></div>"##,
        "a=200.5,7,40,20",
    ),
    (
        "b_js_end_automargin",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:end;margin-left:auto;margin-right:auto"></div></div>"##,
        "a=185.5,7,40,20",
    ),
    (
        "b_js_end_over",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:440px;height:20px;justify-self:end"></div></div>"##,
        "a=-40,7,440,20",
    ),
    (
        "b_js_safe_end_over",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:440px;height:20px;justify-self:safe end"></div></div>"##,
        "a=11,7,440,20",
    ),
    (
        "b_js_center_over",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:440px;height:20px;justify-self:center"></div></div>"##,
        "a=-14.5,7,440,20",
    ),
    (
        "b_ji_center_js_normal",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;justify-items:center"><div data-m="a" style="width:40px;height:20px;justify-self:normal"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "b_js_flex_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:flex-end"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "b_js_self_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:self-end;direction:rtl"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "b_js_baseline",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:last baseline"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "b_js_end_img",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><img data-m="a" style="display:block;width:40px;height:20px;justify-self:end"></div>"##,
        "a=360,7,40,20",
    ),
    (
        "b_js_end_table",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><table data-m="a" style="justify-self:end;border-spacing:0"><tr><td style="padding:0"><div style="width:30px;height:10px"></div></td></tr></table></div>"##,
        "a=370,7,30,10",
    ),
    (
        "b_js_end_bfc",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div style="float:left;width:100px;height:50px"></div><div data-m="a" style="overflow:hidden;width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "b_js_end_float",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="float:left;width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "b_js_end_abs",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "abs_js_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:center"></div></div>"##,
        "a=185.5,7,40,20",
    ),
    (
        "abs_items_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;justify-items:end"><div data-m="a" style="position:absolute;width:40px;height:20px"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "abs_items_stretch",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;justify-items:stretch"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:auto"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "abs_rtl_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;direction:rtl"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "abs_rtl_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;direction:rtl"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:start"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "abs_rtl_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;direction:rtl"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:center"></div></div>"##,
        "a=185.5,7,40,20",
    ),
    (
        "abs_self_end_rtl_box",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:self-end;direction:rtl"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "abs_right",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:right"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "abs_end_margins",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end;margin:0 5px 0 3px"></div></div>"##,
        "a=355,7,40,20",
    ),
    (
        "abs_end_auto_margin",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end;margin:0 auto"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "abs_end_border",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;border-left:4px solid;border-right:6px solid;padding-right:13px"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=341,7,40,20",
    ),
    (
        "abs_end_wide",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:450px;height:20px;justify-self:end"></div></div>"##,
        "a=-50,7,450,20",
    ),
    (
        "abs_center_wide",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:450px;height:20px;justify-self:center"></div></div>"##,
        "a=-19.5,7,450,20",
    ),
    (
        "abs_safe_end_wide",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:450px;height:20px;justify-self:safe end"></div></div>"##,
        "a=0,7,450,20",
    ),
    (
        "abs_safe_center_wide",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:450px;height:20px;justify-self:safe center"></div></div>"##,
        "a=11,7,450,20",
    ),
    (
        "abs_rtl_end_wide",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;direction:rtl;padding:7px 13px 0 11px"><div data-m="a" style="position:absolute;width:450px;height:20px;justify-self:end"></div></div>"##,
        "a=11,7,450,20",
    ),
    (
        "abs_rtl_safe_end_wide",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;direction:rtl;padding:7px 13px 0 11px"><div data-m="a" style="position:absolute;width:450px;height:20px;justify-self:safe end"></div></div>"##,
        "a=-50,7,450,20",
    ),
    (
        "abs_end_left_inset",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end;left:10px"></div></div>"##,
        "a=10,7,40,20",
    ),
    (
        "abs_end_after_block",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;padding-right:13px"><div style="height:15px"></div><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=347,22,40,20",
    ),
    (
        "abs_end_cb_above",
        r##"<div style="position:relative;padding:3px 0 0 5px"><div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;padding-right:13px"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end"></div></div></div>"##,
        "a=347,7,40,20",
    ),
    (
        "abs_end_fixed",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;"><div data-m="a" style="position:fixed;width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "abs_end_flex",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;display:flex"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "abs_end_span",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative"><span data-m="a" style="position:absolute;width:40px;height:20px;justify-self:end"></span></div>"##,
        "a=11,7,40,20",
    ),
    (
        "abs_items_end_span",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;justify-items:end"><span data-m="a" style="position:absolute;width:40px;height:20px"></span></div>"##,
        "a=11,7,40,20",
    ),
    (
        "abs_fit_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;"><div data-m="a" style="position:absolute;justify-self:end"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=0,7,400,20",
    ),
    (
        "abs_fit_end_room",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-right:111px"><div data-m="a" style="position:absolute;justify-self:end"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=0,7,289,30",
    ),
    (
        "abs_fit_normal_room",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-right:111px"><div data-m="a" style="position:absolute"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=11,7,389,20",
    ),
    (
        "abs_fit_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-left:111px"><div data-m="a" style="position:absolute;justify-self:center"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=111,7,289,30",
    ),
    (
        "abs_fit_center_near_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding:7px 151px 0 11px"><div data-m="a" style="position:absolute;justify-self:center"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=0,7,260,30",
    ),
    (
        "abs_fit_start",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-left:111px"><div data-m="a" style="position:absolute;justify-self:start"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=111,7,289,30",
    ),
    (
        "abs_fit_normal",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-left:111px"><div data-m="a" style="position:absolute"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=111,7,289,30",
    ),
    (
        "abs_fit_rtl_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-left:111px;direction:rtl"><div data-m="a" style="position:absolute;justify-self:end"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=111,7,289,30",
    ),
    (
        "abs_fit_rtl_normal",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-left:111px;direction:rtl"><div data-m="a" style="position:absolute"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=0,7,400,20",
    ),
    (
        "abs_fit_rtl_normal_room",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-right:111px;direction:rtl"><div data-m="a" style="position:absolute"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=0,7,289,30",
    ),
    (
        "abs_fit_rtl_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;font-size:0;line-height:0;padding-right:111px;direction:rtl"><div data-m="a" style="position:absolute;justify-self:center"><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span><span style="display:inline-block;width:150px;height:10px"></span></div></div>"##,
        "a=0,7,300,20",
    ),
    (
        "abs_left_rtl",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;position:relative;direction:rtl"><div data-m="a" style="position:absolute;width:40px;height:20px;justify-self:left"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "b_js_end_inline",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><span data-m="a" style="display:inline-block;width:40px;height:20px;justify-self:end"></span></div>"##,
        "a=11,7,40,20",
    ),
    (
        "g_ai_safe_end",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-rows:30px"><div data-m="a" style="width:40px;height:50px;align-self:safe end"></div></div>"##,
        "a=11,7,40,50",
    ),
    (
        "g_ai_end_over",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-rows:30px"><div data-m="a" style="width:40px;height:50px;align-self:end"></div></div>"##,
        "a=11,-13,40,50",
    ),
    (
        "f_ai_safe_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;height:40px;align-items:safe center"><div data-m="a" style="width:40px;height:70px"></div></div>"##,
        "a=11,7,40,70",
    ),
    (
        "f_ai_center_over",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;height:40px;align-items:center"><div data-m="a" style="width:40px;height:70px"></div></div>"##,
        "a=11,-11.5,40,70",
    ),
    (
        "b_js_start_auto",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="height:20px;justify-self:start"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=11,7,30,20",
    ),
    (
        "b_js_baseline_auto",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="height:20px;justify-self:baseline"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=11,7,30,20",
    ),
    (
        "b_js_left_auto",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="height:20px;justify-self:left"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=11,7,30,20",
    ),
    (
        "b_js_self_start_rtl_item",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:self-start;direction:rtl"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "b_ji_stretch",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;justify-items:stretch"><div data-m="a" style="height:20px"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=11,7,389,20",
    ),
    (
        "b_ji_end_js_auto_max",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;justify-items:end"><div data-m="a" style="height:20px;max-width:200px"><div style="width:300px;height:10px"></div></div></div>"##,
        "a=200,7,200,20",
    ),
    (
        "b_ji_end_minw",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;justify-items:end"><div data-m="a" style="height:20px;min-width:100px"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=300,7,100,20",
    ),
    (
        "b_js_end_float_inside",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:100px;height:30px;justify-self:end"><div data-m="f" style="float:left;width:20px;height:10px"></div></div></div>"##,
        "a=300,7,100,30 f=300,7,20,10",
    ),
    // An outer float reaches into the moved box: its own float avoids it
    // where the box is, not where it would have started.
    (
        "b_js_end_outer_float",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div style="float:right;width:60px;height:50px"></div><div data-m="a" style="width:100px;height:30px;justify-self:end"><div data-m="f" style="float:right;width:20px;height:10px"></div></div></div>"##,
        "a=300,7,100,30 f=320,7,20,10",
    ),
    (
        "b_js_end_rtl_margin",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="width:40px;height:20px;justify-self:end;margin-left:13px;margin-right:5px"></div></div>"##,
        "a=24,7,40,20",
    ),
    (
        "b_js_center_rtl_auto",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="height:20px;justify-self:center"><div style="width:30px;height:10px"></div></div></div>"##,
        "a=190.5,7,30,20",
    ),
    (
        "b_ji_center_nested",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;justify-items:center"><div data-m="a" style="width:200px;height:40px"><div data-m="n" style="width:20px;height:10px"></div></div></div>"##,
        "a=105.5,7,200,40 n=105.5,7,20,10",
    ),
    (
        "b_js_end_pct",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:25%;height:20px;justify-self:end"></div></div>"##,
        "a=302.75,7,97.25,20",
    ),
    (
        "b_js_end_ml_auto_one",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px"><div data-m="a" style="width:40px;height:20px;justify-self:end;margin-right:auto"></div></div>"##,
        "a=11,7,40,20",
    ),
    (
        "r_abs_static",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;position:relative"><div data-m="a" style="position:absolute;width:40px;height:20px"></div></div>"##,
        "a=360,7,40,20",
    ),
    (
        "r_g_abs",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;position:relative;display:grid;grid-template-columns:100px 150px"><div data-m="a" style="position:absolute;width:40px;height:20px"></div></div>"##,
        "a=360,0,40,20",
    ),
    (
        "r_f_wrap",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;display:flex;flex-wrap:wrap"><div data-m="a" style="width:300px;height:20px"></div><div data-m="b" style="width:150px;height:20px"></div></div>"##,
        "a=100,7,300,20 b=250,43.5,150,20",
    ),
    (
        "r_float",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><div data-m="a" style="float:left;width:40px;height:20px"></div><div data-m="b" style="float:right;width:50px;height:20px"></div></div>"##,
        "a=11,7,40,20 b=350,7,50,20",
    ),
    (
        "r_inline_block",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl"><span data-m="a" style="display:inline-block;width:40px;height:20px"></span><span data-m="b" style="display:inline-block;width:50px;height:20px"></span></div>"##,
        "a=360,7,40,20 b=310,7,50,20",
    ),
    (
        "r_inline_block_three",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;font-size:0;line-height:0"><span data-m="a" style="display:inline-block;width:40px;height:20px"></span><span data-m="b" style="display:inline-block;width:50px;height:20px"></span><span data-m="e" style="display:inline-block;width:60px;height:20px"></span></div>"##,
        "a=360,7,40,20 b=310,7,50,20 e=250,7,60,20",
    ),
    (
        "r_inline_block_wrap",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;font-size:0;line-height:0"><span data-m="a" style="display:inline-block;width:200px;height:20px"></span><span data-m="b" style="display:inline-block;width:150px;height:20px"></span><span data-m="e" style="display:inline-block;width:100px;height:20px"></span></div>"##,
        "a=200,7,200,20 b=50,7,150,20 e=300,27,100,20",
    ),
    (
        "r_inline_block_br",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;font-size:0;line-height:0"><span data-m="a" style="display:inline-block;width:40px;height:20px"></span><br><span data-m="b" style="display:inline-block;width:50px;height:20px"></span><span data-m="e" style="display:inline-block;width:60px;height:20px"></span></div>"##,
        "a=360,7,40,20 b=350,27,50,20 e=290,27,60,20",
    ),
    (
        "r_inline_block_center",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;font-size:0;line-height:0;text-align:center"><span data-m="a" style="display:inline-block;width:40px;height:20px"></span><span data-m="b" style="display:inline-block;width:50px;height:20px"></span></div>"##,
        "a=210.5,7,40,20 b=160.5,7,50,20",
    ),
    (
        "r_inline_block_ltr_span",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;font-size:0;line-height:0"><span data-m="a" style="display:inline-block;width:40px;height:20px"></span><span style="direction:ltr"><span data-m="b" style="display:inline-block;width:50px;height:20px"></span><span data-m="e" style="display:inline-block;width:60px;height:20px"></span></span></div>"##,
        "a=360,7,40,20 b=310,7,50,20 e=250,7,60,20",
    ),
    (
        "r_inline_block_embed",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;direction:rtl;font-size:0;line-height:0"><span data-m="a" style="display:inline-block;width:40px;height:20px"></span><span style="direction:ltr;unicode-bidi:embed"><span data-m="b" style="display:inline-block;width:50px;height:20px"></span><span data-m="e" style="display:inline-block;width:60px;height:20px"></span></span></div>"##,
        "a=360,7,40,20 b=250,7,50,20 e=300,7,60,20",
    ),
    (
        "l_inline_block_isolate_rtl",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;font-size:0;line-height:0"><span data-m="a" style="display:inline-block;width:40px;height:20px"></span><span style="direction:rtl;unicode-bidi:isolate"><span data-m="b" style="display:inline-block;width:50px;height:20px"></span><span data-m="e" style="display:inline-block;width:60px;height:20px"></span></span></div>"##,
        "a=11,7,40,20 b=111,7,50,20 e=51,7,60,20",
    ),
];

/// (name, rinch's answer, why) — rows of `CHROME` that are not Chrome's yet,
/// each for a reason that is not this change's.
const KNOWN: &[(&str, &str, &str)] = &[
    (
        "r_inline_block_embed",
        "a=360,7,40,20 b=310,7,50,20 e=250,7,60,20",
        "`unicode-bidi` is not read (#2588)",
    ),
    (
        "l_inline_block_isolate_rtl",
        "a=11,7,40,20 b=51,7,50,20 e=101,7,60,20",
        "`unicode-bidi` is not read (#2588)",
    ),
    (
        "abs_left_rtl",
        "a=11,7,40,20",
        "Chrome 157 aligns an absolute box's static position by `justify-self: left` to the start \
         of an rtl block container, the right, where an in-flow box (`b_js_left`'s 11) and \
         css-align-3 take the left: rinch keeps `left` physical",
    ),
];

fn lay_out(html: &str, height: f32) -> RinchDocument {
    let mut doc = RinchDocument::new();
    // The Chrome numbers were measured under this rule (file docs).
    doc.load_css("* { box-sizing: border-box; }");
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
fn direction_and_justify_match_chrome_157() {
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
    assert!(rows > 40, "the table is being read: {rows} rows");
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

/// `direction` and then `justify-self` written onto a laid-out container
/// move its items as a fresh layout places them: both reach Taffy's style as
/// a layout change (a container's `direction` reaches its children by
/// inheritance, and each child's Taffy style changes with it).
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
        ("r_g_normal", "r_g_left", "c"),
        ("b_js_end", "b_js_end_rtl", "c"),
        ("r_b", "r_f_row", "c"),
        ("b_js_start_rtl", "b_js_end_rtl", "a"),
        ("b_js_end_margin", "b_js_center_margin", "a"),
        ("r_abs_static", "abs_rtl_end", "a"),
        ("abs_fit_rtl_normal", "abs_fit_rtl_end", "a"),
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
