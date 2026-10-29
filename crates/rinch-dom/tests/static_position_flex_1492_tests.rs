//! #1492 — the static position of an auto-inset out-of-flow child of a flex
//! container: as if it were the container's sole flex item (css-flexbox-1
//! §4.1), so `justify-content` places it on the main axis and `align-self`
//! (else the container's `align-items`) on the cross axis — `stretch`,
//! `normal` and `baseline` as the start. rinch followed the main axis only:
//! it handed Taffy `align-self: flex-start` for every out-of-flow box whose
//! own `align-self` was unset.
//!
//! Every number is **Chrome 153**'s (`--headless=new`, standards mode,
//! `* { box-sizing: border-box }`, 800x600, device scale factor 1),
//! `getBoundingClientRect` relative to the container `c`'s border box:
//! `mark=x,y,width,height`. The container has `padding: 7px 0 0 11px`, so
//! its content box starts at (11, 7) and no row sits where its corners
//! coincide; the box is 40x30 between two 50x20 items. No text is laid out.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

/// (name, html, Chrome 153).
const CHROME: &[(&str, &str, &str)] = &[
    (
        "f_jc_flex-start",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:flex-start;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_jc_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,7,40,30",
    ),
    (
        "f_jc_flex-end",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,30",
    ),
    (
        "f_jc_space-between",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:space-between;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_jc_space-around",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:space-around;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,7,40,30",
    ),
    (
        "f_jc_space-evenly",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:space-evenly;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,7,40,30",
    ),
    (
        "f_jc_start",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:start;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_jc_end",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,30",
    ),
    (
        "f_ai_stretch",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:stretch;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_ai_flex-start",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:flex-start;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_ai_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,28.5,40,30",
    ),
    (
        "f_ai_flex-end",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,50,40,30",
    ),
    (
        "f_ai_baseline",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:baseline;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_ai_start",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:start;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_ai_end",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,50,40,30",
    ),
    (
        "f_ai_normal",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:normal;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_self_auto",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;align-self:auto;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,28.5,40,30",
    ),
    (
        "f_self_stretch",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;align-self:stretch;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_self_flex-start",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;align-self:flex-start;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_self_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;align-self:center;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,28.5,40,30",
    ),
    (
        "f_self_flex-end",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;align-self:flex-end;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,50,40,30",
    ),
    (
        "f_self_baseline",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;align-self:baseline;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_cc",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,28.5,40,30",
    ),
    (
        "f_ee",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:flex-end;align-items:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=360,50,40,30",
    ),
    (
        "f_cc_fixed",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:fixed;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,28.5,40,30",
    ),
    (
        "f_ee_fixed",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:flex-end;align-items:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:fixed;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=360,50,40,30",
    ),
    (
        "f_cc_icb",
        r##"<div data-m="c" style="width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,28.5,40,30",
    ),
    (
        "f_cc_margin",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;margin:4px 0 0 6px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=188.5,30.5,40,30",
    ),
    (
        "f_ee_margin",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:flex-end;align-items:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;margin:4px 10px 6px 6px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=350,44,40,30",
    ),
    (
        "f_cc_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;left:5px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=5,28.5,40,30",
    ),
    (
        "f_cc_top",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;top:5px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,5,40,30",
    ),
    (
        "f_cc_autosize",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=205.5,43.5,0,0",
    ),
    (
        "f_stretch_autosize",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,0",
    ),
    (
        "f_selfstretch_autosize",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;align-self:stretch;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,0",
    ),
    (
        "f_column_cc",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,28.5,40,30",
    ),
    (
        "f_column_ee",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column;justify-content:flex-end;align-items:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=360,50,40,30",
    ),
    (
        "f_column_ss",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_row-reverse_cc",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:row-reverse;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,28.5,40,30",
    ),
    (
        "f_row-reverse_ee",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:row-reverse;justify-content:flex-end;align-items:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,50,40,30",
    ),
    (
        "f_row-reverse_ss",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:row-reverse;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,30",
    ),
    (
        "f_column-reverse_cc",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column-reverse;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=185.5,28.5,40,30",
    ),
    (
        "f_column-reverse_ee",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column-reverse;justify-content:flex-end;align-items:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=360,7,40,30",
    ),
    (
        "f_column-reverse_ss",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-direction:column-reverse;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,50,40,30",
    ),
    (
        "f_wrap_reverse_end",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-wrap:wrap-reverse;align-items:flex-end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,7,40,30",
    ),
    (
        "f_wrap_reverse_start",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-wrap:wrap-reverse;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,50,40,30",
    ),
    (
        "f_wrap_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;flex-wrap:wrap;align-items:center;align-content:flex-start;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=11,28.5,40,30",
    ),
    (
        "f_overflow_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:flex;align-items:center;justify-content:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:500px;height:100px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=-44.5,-6.5,500,100",
    ),
    (
        "f_anc_cc",
        r##"<div data-m="c" style="position:relative;width:420px;height:120px;padding:3px 0 0 5px;margin:9px 0 0 14px"><div data-m="p" style="width:400px;height:80px;padding:7px 0 0 11px;margin:6px 0 0 4px;display:flex;justify-content:center;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div></div>"##,
        "p=9,9,400,80 a=194.5,37.5,40,30",
    ),
    (
        "f_inlineflex_cc",
        r##"<div data-m="c" style="margin:9px 0 0 14px"><span style="display:inline-flex;position:relative;width:400px;height:80px;padding:7px 0 0 11px;justify-content:center;align-items:center" data-m="q"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px"></div><div style="width:50px;height:20px"></div></span></div>"##,
        "q=0,0,400,80 a=185.5,28.5,40,30",
    ),
    (
        "g_ai_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 100px;align-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=0,25,40,30",
    ),
    (
        "g_ai_end",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 100px;align-items:end;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=0,50,40,30",
    ),
    (
        "g_plain",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 100px;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=0,0,40,30",
    ),
    (
        "g_ji_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 100px;justify-items:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=180,0,40,30",
    ),
    (
        "g_ac_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 100px;align-content:center;justify-content:center;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=0,0,40,30",
    ),
    (
        "g_self_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:80px;padding:7px 0 0 11px;margin:9px 0 0 14px;display:grid;grid-template-columns:100px 100px;"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px;align-self:center;justify-self:center;"></div><div style="width:50px;height:20px"></div></div>"##,
        "a=180,25,40,30",
    ),
];

/// (name, rinch's answer, why) — rows of `CHROME` that are not Chrome's yet,
/// each for a reason that is not this issue's.
const KNOWN: &[(&str, &str, &str)] = &[];

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

fn one(doc: &RinchDocument, selector: &str) -> usize {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

fn screen(doc: &RinchDocument, id: usize) -> (f32, f32) {
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
fn flex_static_positions_match_chrome_153() {
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

/// Laid out again with nothing changed (another viewport height), and after
/// `align-items` is written onto a laid-out container: as a fresh layout.
#[test]
fn a_relayout_and_a_restyle_agree_with_a_fresh_layout() {
    for (n, h, want) in CHROME {
        let mut doc = lay_out(h, 600.0);
        let first = dump(&doc, want);
        doc.resolve_layout(800.0, 640.0);
        assert_eq!(first, dump(&doc, want), "{n}: a second layout moved it");
    }
    let row = |n: &str| CHROME.iter().find(|r| r.0 == n).unwrap();
    let (_, before, _) = row("f_jc_center");
    let (_, after, want) = row("f_cc");
    let mut doc = lay_out(before, 600.0);
    let c = one(&doc, "[data-m=c]");
    let after_style = after
        .split(r#"data-m="c" style=""#)
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    doc.set_attribute(rinch_core::dom::NodeId(c), "style", after_style);
    doc.resolve_layout(800.0, 640.0);
    assert_eq!(
        dump(&doc, want),
        dump(&lay_out(after, 600.0), want),
        "restyled"
    );
}
