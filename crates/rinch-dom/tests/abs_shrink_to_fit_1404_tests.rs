//! #1404 — the width an **auto-width absolutely positioned box** is shrunk
//! to fit in is its containing block's, less its insets and margins (CSS 2.1
//! §10.3.7: `min(max(preferred minimum, available), preferred)`).
//!
//! Taffy measures such a box in the padding-box width of the box it lays it
//! out in — its direct parent — whatever its insets and margins are and
//! wherever its containing block is. So `left: 20px` in a 394px containing
//! block gave 394 (and overflowed it by 20), and a box under a narrow
//! unpositioned wrapper wrapped at the wrapper's width.
//!
//! Every number in `CHROME` is **Chrome 153**'s (`--headless=new`, standards
//! mode, `* { box-sizing: border-box }`, 800x600, device scale factor 1, the
//! bundled Inter registered as `ProbeFace` through `@font-face`,
//! `16px/20px`), `getBoundingClientRect` relative to the container `c`'s
//! border box: `mark=x,y,width,height`.
//!
//! The container is `400px` wide with a `3px` border and `padding-left:
//! 12px`, so its padding box is 394 wide and starts at 3, its content box at
//! 15, and no expectation sits where those coincide. The content of most
//! boxes is one `inline-block` holding a line whose max-content width is
//! 544.2 and whose longest word is 66.1: an `auto` atomic inline is capped at
//! the width its line gives it (#1476), so the box is exactly as wide as the
//! space it was given, between 66 and 544. (Plain wrapping content answers
//! its widest line instead — issue #1276, the `g` rows.)

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// (name, html, Chrome 153).
const CHROME: &[(&str, &str, &str)] = &[
    (
        "d01_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,8,374,40 t=23,8,374,40",
    ),
    (
        "d02_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;right:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=3,8,374,40 t=3,8,374,40",
    ),
    (
        "d03_left_pct",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:50%"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=200,8,197,80 t=200,8,197,80",
    ),
    (
        "d04_left_margin_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:0;margin-right:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=3,8,374,40 t=3,8,374,40",
    ),
    (
        "d05_left_margin_pct",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:0;margin-left:10%"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=42.39,8,354.61,40 t=42.39,8,354.61,40",
    ),
    (
        "d06_right_margin_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;right:20px;margin-right:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=3,8,354,40 t=3,8,354,40",
    ),
    (
        "d07_left_past_min_content",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:340px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=343,8,66.14,200 t=343,8,66.14,200",
    ),
    (
        "d08_left_max_width",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:0;max-width:300px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=3,8,300,40 t=3,8,300,40",
    ),
    (
        "d09_left_padding",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:20px;padding:0 10px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,8,374,40 t=33,8,354,40",
    ),
    (
        "d10_negative_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:-200px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=-197,8,544.23,20 t=-197,8,544.23,20",
    ),
    (
        "d11_both_insets",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:0;right:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=3,8,394,40 t=3,8,394,40",
    ),
    (
        "d12_static_margin_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;margin-left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=35,8,362,40 t=35,8,362,40",
    ),
    (
        "d13_static",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=15,8,382,40 t=15,8,382,40",
    ),
    (
        "d14_flex_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;display:flex;"><div data-m="a" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,8,374,40 t=23,8,374,40",
    ),
    (
        "d15_grid_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;display:grid;"><div data-m="a" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,3,374,40 t=23,3,374,40",
    ),
    (
        "d16_flex_column_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;display:flex;flex-direction:column;"><div data-m="a" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,8,374,40 t=23,8,374,40",
    ),
    (
        "d17_left_min_width",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:20px;min-width:390px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,8,390,40 t=23,8,390,40",
    ),
    (
        "d18_left_top",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:20px;top:30px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,33,374,40 t=23,33,374,40",
    ),
    (
        "d19_static_margin_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;margin-right:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=15,8,362,40 t=15,8,362,40",
    ),
    (
        "d20_fit_content_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:20px;width:fit-content"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,8,374,40 t=23,8,374,40",
    ),
    (
        "d21_right_margin_left_auto",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;right:20px;margin-left:auto"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=3,8,374,40 t=3,8,374,40",
    ),
    (
        "d22_left_border",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:20px;border:5px solid"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=23,8,374,50 t=28,13,364,40",
    ),
    (
        "a01_left0",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;left:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=3,8,394,40 t=3,8,394,40",
    ),
    (
        "a02_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=23,8,374,40 t=23,8,374,40",
    ),
    (
        "a03_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;right:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=3,8,374,40 t=3,8,374,40",
    ),
    (
        "a04_left_pct",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;left:50%"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=200,8,197,80 t=200,8,197,80",
    ),
    (
        "a05_static",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=45,8,352,40 t=45,8,352,40",
    ),
    (
        "a06_static_margin_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;margin-left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=65,8,332,40 t=65,8,332,40",
    ),
    (
        "a07_static_top",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;top:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=45,3,352,40 t=45,3,352,40",
    ),
    (
        "a08_static_margin_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;margin-right:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=45,8,332,40 t=45,8,332,40",
    ),
    (
        "a09_static_far_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;margin-left:300px;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=315,8,200,200 a=315,8,82,200 t=315,8,82,200",
    ),
    (
        "a10_two_levels_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div style="margin-left:40px;width:100px"><div data-m="a" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div></div>"##,
        "p=45,8,200,200 a=23,8,374,40 t=23,8,374,40",
    ),
    (
        "a11_static_in_flex_center",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;display:flex;justify-content:center;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=3,8,284,40 t=3,8,284,40",
    ),
    (
        "a11b_static_in_flex_end",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;display:flex;justify-content:flex-end;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=3,8,242,60 t=3,8,242,60",
    ),
    (
        "a11c_static_in_flex_start",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;display:flex;padding-left:6px;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=51,8,346,40 t=51,8,346,40",
    ),
    (
        "a12_static_in_grid",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;display:grid;padding-left:6px;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=51,8,346,40 t=51,8,346,40",
    ),
    (
        "a13_static_inline_after_text",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;">ab cd <span data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></div></div>"##,
        "p=45,8,200,200 a=87.22,8,309.78,40 t=87.22,8,309.78,40",
    ),
    (
        "a14_static_block_after_text",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;">ab cd <div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=45,28,352,40 t=45,28,352,40",
    ),
    (
        "a15_in_inline_block_static",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div>ab cd <span style="display:inline-block;width:100px;height:20px"><div data-m="a" style="position:absolute;top:40px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></span></div></div>"##,
        "a=61.72,43,335.28,40 t=61.72,43,335.28,40",
    ),
    (
        "a16_in_inline_block_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div>ab cd <span style="display:inline-block;width:100px;height:20px"><div data-m="a" style="position:absolute;left:20px;top:40px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></span></div></div>"##,
        "a=23,43,374,40 t=23,43,374,40",
    ),
    (
        "a17_span_cb_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div>ab <span style="position:relative">cd ef gh<div data-m="a" style="position:absolute;left:0;top:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></span></div></div>"##,
        "a=38.28,28,66.14,200 t=38.28,28,66.14,200",
    ),
    (
        "a24_container_left_margin",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;left:20px;margin-right:30px"><div><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div></div>"##,
        "p=45,8,200,200 a=23,8,344,40 t=23,8,344,40",
    ),
    (
        "a25_fit_content_static",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;width:fit-content"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=45,8,352,40 t=45,8,352,40",
    ),
    (
        "a26_contents_wrapper_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div style="display:contents"><div data-m="a" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div></div>"##,
        "p=45,8,200,200 a=23,8,374,40 t=23,8,374,40",
    ),
    (
        "a27_container_static_margin",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;margin-left:20px"><div><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div></div>"##,
        "p=45,8,200,200 a=65,8,332,40 t=65,8,332,40",
    ),
    (
        "d23_container_right_margin",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;right:20px;margin-right:20px"><div><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "a=3,8,354,40 t=3,8,354,40",
    ),
    (
        "d24_grid_margin_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;display:grid;"><div data-m="a" style="position:absolute;left:0;margin-right:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=3,3,374,40 t=3,3,374,40",
    ),
    (
        "d25_container_left0_margin",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:0;margin-right:20px"><div><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "a=3,8,374,40 t=3,8,374,40",
    ),
    (
        "i09_container_left_margin",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:400px;top:0;margin-right:50px"><div><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "a=386,-9,350,40 t=386,-9,350,40",
    ),
    (
        "a18_in_auto_abs",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="position:absolute;left:100px;top:10px;height:100px"><div style="width:150px;height:10px"></div><div data-m="a" style="position:absolute;left:20px;top:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=103,13,150,100 a=123,33,130,100 t=123,33,130,100",
    ),
    (
        "a19_fit_content_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;left:20px;width:fit-content"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=23,8,374,40 t=23,8,374,40",
    ),
    (
        "a20_static_padded_parent",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;padding-left:9px;border-left:2px solid;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=56,8,341,40 t=56,8,341,40",
    ),
    (
        "a21_left_margin_right",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;left:20px;margin-right:30px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=23,8,344,40 t=23,8,344,40",
    ),
    (
        "a22_right_only_padding",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;right:20px;padding:0 10px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=3,8,374,40 t=13,8,354,40",
    ),
    (
        "a23_static_max_width",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;max-width:250px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=45,8,200,200 a=45,8,250,60 t=45,8,250,60",
    ),
    (
        "i01_left0",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:0;top:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=-14,-9,544.23,20 t=-14,-9,544.23,20",
    ),
    (
        "i02_left400",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:400px;top:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=386,-9,400,40 t=386,-9,400,40",
    ),
    (
        "i03_right500",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;right:500px;top:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=-14,-9,300,40 t=-14,-9,300,40",
    ),
    (
        "i04_static",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=15,8,544.23,20 t=15,8,544.23,20",
    ),
    (
        "i05_static_far_right",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;margin-left:300px;"><div data-m="a" style="position:absolute;"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=315,8,200,200 a=315,8,471,40 t=315,8,471,40",
    ),
    (
        "i06_left_past_min",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:760px;top:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=746,-9,66.14,200 t=746,-9,66.14,200",
    ),
    (
        "i07_left_pct",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:50%;top:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=386,-9,400,40 t=386,-9,400,40",
    ),
    (
        "i08_static_margin_left",
        r##"<div data-m="c" style="width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;margin-left:300px;"><div data-m="a" style="position:absolute;margin-left:100px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "p=315,8,200,200 a=415,8,371,40 t=415,8,371,40",
    ),
    (
        "g01_boxes_direct_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:20px;font-size:0;line-height:0"><span style="display:inline-block;width:130px;height:20px"></span><span style="display:inline-block;width:130px;height:20px"></span><span style="display:inline-block;width:130px;height:20px"></span><span style="display:inline-block;width:130px;height:20px"></span></div></div>"##,
        "a=23,8,374,40",
    ),
    (
        "g02_boxes_issue_example",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;left:0;top:0;font-size:0;line-height:0"><span style="display:inline-block;width:130px;height:20px"></span><span style="display:inline-block;width:130px;height:20px"></span><span style="display:inline-block;width:130px;height:20px"></span><span style="display:inline-block;width:130px;height:20px"></span></div></div></div>"##,
        "p=45,8,200,200 a=3,3,394,40",
    ),
    (
        "g03_text_direct_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="a" style="position:absolute;left:20px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</div></div>"##,
        "a=23,8,374,40",
    ),
    (
        "g04_text_anc_left",
        r##"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"><div data-m="p" style="width:200px;height:200px;margin-left:30px;"><div data-m="a" style="position:absolute;left:20px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</div></div></div>"##,
        "p=45,8,200,200 a=23,8,374,40",
    ),
];

/// (name, rinch's answer, why) — rows of `CHROME` that are not Chrome's yet,
/// each for a reason that is not this issue's. Pinned so that a fix has to
/// come here and take the row out.
const KNOWN: &[(&str, &str, &str)] = &[
    (
        "a11_static_in_flex_center",
        "p=45,8,200,200 a=-31,8,352,40 t=-31,8,352,40",
        "a box centred by its flex container from its static position: rinch gives it the room after the container's content edge and centres it there (Chrome: twice the shorter distance from the centre to a containing-block edge, and placed at that edge)",
    ),
    (
        "a11b_static_in_flex_end",
        "p=45,8,200,200 a=-107,8,352,40 t=-107,8,352,40",
        "as a11, for `justify-content: flex-end`",
    ),
    (
        "a12_static_in_grid",
        "p=45,8,200,200 a=45,8,352,40 t=45,8,352,40",
        "the static position in a grid container that is not the containing block is its padding edge in rinch and its content edge in Chrome",
    ),
    (
        "g01_boxes_direct_left",
        "a=23,8,260,40",
        "#1276: content that wraps answers its widest line, not the available width",
    ),
    (
        "g02_boxes_issue_example",
        "p=45,8,200,200 a=3,3,390,40",
        "#1276 (the issue's own example: 130x80 before this fix)",
    ),
    ("g03_text_direct_left", "a=23,8,328,40", "#1276"),
    ("g04_text_anc_left", "p=45,8,200,200 a=23,8,328,40", "#1276"),
];

fn document() -> RinchDocument {
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

/// A document whose `<body>` (no margin) holds `html`, not laid out yet.
fn mount(html: &str) -> RinchDocument {
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc
}

fn lay_out(html: &str) -> RinchDocument {
    let mut doc = mount(html);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
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

fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Whether two dumps agree to within the pixel grid: rinch puts every box
/// edge on a whole pixel, Chrome reports the unrounded box.
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

fn known(name: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    KNOWN.iter().find(|k| k.0 == name)
}

#[test]
fn shrink_to_fit_widths_match_chrome_153() {
    let mut rows = 0;
    let bad: Vec<_> = CHROME
        .iter()
        .filter(|(n, ..)| known(n).is_none())
        .filter_map(|(n, h, want)| {
            rows += 1;
            let got = dump(&lay_out(h), want);
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
            let (_, h, chrome) = CHROME
                .iter()
                .find(|c| c.0 == *n)
                .unwrap_or_else(|| panic!("{n} is a row of CHROME"));
            let got = dump(&lay_out(h), chrome);
            assert!(!close(chrome, now), "{n} is a gap: {why}");
            (!close(now, &got)).then(|| format!("{n} ({why}): pinned[{now}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Every row, laid out a second time with nothing changed (a new viewport
/// height, so the pass is not skipped): nothing moves.
#[test]
fn a_second_layout_moves_nothing() {
    let bad: Vec<_> = CHROME
        .iter()
        .filter_map(|(n, h, want)| {
            let mut doc = lay_out(h);
            let first = dump(&doc, want);
            doc.resolve_layout(VIEWPORT.0, VIEWPORT.1 + 40.0);
            let second = dump(&doc, want);
            (first != second).then(|| format!("{n}: [{first}] then [{second}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// `(name, html before, html after, a change to make)`: each history ends in
/// the same layout as a fresh one of the final markup.
fn histories() -> Vec<(&'static str, String, String)> {
    let t = r##"<span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span>"##;
    let r = |inner: &str, cw: u32| {
        format!(
            r##"<div data-m="c" style="position:relative;width:{cw}px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;">{inner}</div>"##
        )
    };
    let wrapped = |ml: u32, abs: &str| {
        format!(
            r##"<div data-m="p" style="width:200px;height:200px;margin-left:{ml}px"><div data-m="a" style="position:absolute;{abs}">{t}</div></div>"##
        )
    };
    let direct =
        |abs: &str| format!(r##"<div data-m="a" style="position:absolute;{abs}">{t}</div>"##);
    vec![
        // The static position moves: the wrapper's margin.
        (
            "static_moves",
            r(&wrapped(30, ""), 400),
            r(&wrapped(130, ""), 400),
        ),
        // The containing block is resized under an ancestor-resolved box.
        (
            "cb_resized",
            r(&wrapped(30, "left:20px"), 400),
            r(&wrapped(30, "left:20px"), 300),
        ),
        // ... under a box Taffy resolves itself, from its static position.
        ("parent_resized", r(&direct(""), 400), r(&direct(""), 300)),
        // An inset is written.
        (
            "inset_written",
            r(&direct("left:20px"), 400),
            r(&direct("left:60px"), 400),
        ),
        // The box stops being static.
        (
            "goes_static",
            r(&wrapped(30, "left:20px"), 400),
            r(&wrapped(30, ""), 400),
        ),
        // The wrapper starts generating no box.
        (
            "contents",
            r(&wrapped(30, ""), 400),
            r(
                &wrapped(30, "").replacen("width:200px", "display:contents;width:200px", 1),
                400,
            ),
        ),
    ]
}

#[test]
fn incremental_layout_equals_a_fresh_one() {
    let bad: Vec<_> = histories()
        .into_iter()
        .filter_map(|(n, before, after)| {
            let mut doc = lay_out(&before);
            // Replace the container's markup in place: the wrapper div the
            // fixture mounted holds it.
            let c = one(&doc, "[data-m=c]");
            let wrap = doc.tree.get(c).unwrap().parent.unwrap();
            doc.set_inner_html(rinch_core::dom::NodeId(wrap), &after);
            doc.resolve_layout(VIEWPORT.0, VIEWPORT.1 + 40.0);
            let fresh = lay_out(&after);
            let want = "a=0,0,0,0 t=0,0,0,0";
            let (got, exp) = (dump(&doc, want), dump(&fresh, want));
            (got != exp).then(|| format!("{n}: incremental[{got}] fresh[{exp}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// What it costs, in root computes: a box shrunk to fit from a static
/// position away from its containing block's edge pays one more on its first
/// layout (the position is known only after it) and none on a relayout that
/// moves nothing. A box whose static position is at the edge, and one with
/// an inset, pay nothing extra.
#[test]
fn a_static_offset_costs_one_compute_on_first_layout_only() {
    use rinch_dom::perf::Counter;
    let computes = |h: &str| {
        let mut doc = mount(h);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let first = doc.tree.perf.get(Counter::TaffyRootComputes);
        doc.tree.perf.end_frame();
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1 + 40.0);
        (first, doc.tree.perf.get(Counter::TaffyRootComputes))
    };
    // Short content, so no #1476 min-content round blurs the count.
    let row = |n: &str| {
        CHROME.iter().find(|r| r.0 == n).unwrap().1.replace(
            "Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters",
            "Wavy",
        )
    };
    // At the edge: the plain cost.
    let (base, again) = computes(&row("d11_both_insets"));
    assert_eq!(again, 1, "a relayout is one compute");
    assert_eq!(
        computes(&row("d01_left")),
        (base, 1),
        "an inset: the keyword"
    );
    assert_eq!(
        computes(&row("d13_static")),
        (base + 1, 1),
        "a static offset: one more, once"
    );
    assert_eq!(
        computes(&row("a02_left")),
        (base + 1, 1),
        "an ancestor-resolved box, as #386 has it"
    );
    assert_eq!(
        computes(&row("a05_static")),
        (base + 2, 1),
        "both: one each"
    );
}
