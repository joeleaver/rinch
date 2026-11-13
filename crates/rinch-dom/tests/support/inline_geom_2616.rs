//! The scene and Chrome 157's answers of #2616 and #2618, shared by
//! `rinch-dom`'s `inline_geom_2616_tests` (the bounding rect and the
//! containing block) and `rinch`'s `inline_box_hit_2618_tests` (where a press
//! lands, through `#[path]`):
//!
//! - #2616: a wrapped right-to-left inline element's bounding rect took its
//!   padding and border on the left of its first fragment and the right of
//!   its last, which are a left-to-right element's sides; and so did the
//!   containing block a positioned one forms.
//! - #2618: a press on an inline element's padding or border reached the
//!   block, not the element.
//!
//! Inline elements wrapping over two and three lines, left-to-right and
//! right-to-left, nested, with uneven padding, border and margin, set in this
//! crate's Inter (registered as `ProbeFace`, loaded by Chrome through
//! `@font-face` from the same file).

#![allow(dead_code)]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

pub const W: u32 = 300;
pub const H: u32 = 800;

/// Chrome 157's page: the same body under
/// `@font-face { font-family: ProbeFace; src: url(Inter-Regular.ttf) }`.
pub const BODY: &str = r#"
<div dir=rtl style="left:10px;top:0;width:100px"><span id=p style="border:2px solid #000;border-right-width:3px;padding:4px 6px 5px 9px;margin:0 4px 0 7px;background:#ff0">MMMMM MMMMM</span></div>
<div dir=rtl style="left:150px;top:0;width:100px"><span id=q style="border:2px solid #000;border-left-width:3px;padding:3px 8px 6px 5px;background:#ff0">MMMMM MMMMM MMMMM</span></div>
<div style="left:10px;top:200px;width:120px"><span id=r style="border:1px solid #000;padding:0 5px;margin:0 3px"><span id=s style="border:2px solid #000;padding:3px 4px 2px 6px">MMMMM MMMMM MMMMM</span></span></div>
<div dir=rtl style="left:150px;top:200px;width:120px"><span id=t style="border:1px solid #000;padding:0 5px;margin:0 3px"><span id=u style="border:2px solid #000;padding:3px 4px 2px 6px">MMMMM MMMMM MMMMM</span></span></div>
<div style="left:10px;top:400px;width:100px"><span id=e style="border:2px solid #000;padding:0 6px">MMMMM MMMMM</span></div>
<div dir=rtl style="left:150px;top:400px;width:100px"><span id=g style="border:2px solid #000;background:#ff0">MMMMM MMMMM MMMMM</span></div>
<div dir=rtl style="left:10px;top:540px;width:100px"><span id=v style="position:relative;border:2px solid #000;border-right-width:3px;padding:4px 6px 5px 9px">MMMMM MMMMM MM<b id=v1 style="left:0;top:0"></b><b id=v2 style="right:0;bottom:0"></b></span></div>
<div style="left:150px;top:540px;width:100px"><span id=w style="position:relative;border:2px solid #000;border-right-width:3px;padding:4px 6px 5px 9px">MMMMM MMMMM MM<b id=w1 style="left:0;top:0"></b><b id=w2 style="right:0;bottom:0"></b></span></div>
<div style="left:10px;top:680px;width:100px"><span id=x style="padding:6px 7px">MMMMM MMMMM</span></div>
"#;

pub const CSS: &str = "html, body { margin: 0; background: #fff; }
     div { position: absolute; font: 16px/40px ProbeFace; }
     span { color: transparent; }
     b { position: absolute; width: 1px; height: 1px; }";

/// `(x, y, width, height)`, CSS px.
pub type Rect = (f64, f64, f64, f64);

/// Each element's `getBoundingClientRect`, CSS px, in Chrome 157 at 1x.
pub const BOUNDS: &[(&str, Rect)] = &[
    ("p", (24.7344, 4.0, 85.2656, 73.0)),
    ("q", (167.7344, 5.0, 82.2656, 113.0)),
    ("r", (10.0, 209.0, 89.2656, 102.0)),
    ("s", (10.0, 205.0, 89.2656, 109.0)),
    ("t", (182.7344, 209.0, 87.2656, 102.0)),
    ("u", (182.7344, 205.0, 87.2656, 109.0)),
    ("e", (10.0, 408.0, 80.2656, 64.0)),
    ("g", (175.7344, 408.0, 74.2656, 104.0)),
    ("v", (28.7344, 544.0, 81.2656, 113.0)),
    ("w", (150.0, 544.0, 83.2656, 113.0)),
    ("x", (10.0, 684.0, 79.2656, 72.0)),
];

/// The top-left of each 1px absolute box in `v` and `w` (`left: 0; top:
/// 0` and `right: 0; bottom: 0`), CSS px: the containing block the span
/// forms runs from the first fragment's top and start side to the last
/// one's bottom and end side, inside its padding edges.
pub const ABS: &[(&str, (f64, f64))] = &[
    ("v1", (72.0938, 546.0)),
    ("v2", (106.0, 654.0)),
    ("w1", (152.0, 546.0)),
    ("w2", (183.9063, 654.0)),
];

/// Points in each fragment's left and right border and padding (halfway
/// down), its top and bottom border and padding (halfway along), and just
/// past its left and right side, and what Chrome 157's `elementFromPoint`
/// finds there: `Some(id)` an inline element, `None` the block. Points past
/// the block are left out.
pub const HITS: &[(&str, f32, f32, Option<&str>)] = &[
    ("p 0 l-border", 25.3344, 20.5, Some("p")),
    ("p 0 l-pad", 31.2344, 20.5, Some("p")),
    ("p 0 r-border", 105.4, 20.5, Some("p")),
    ("p 0 r-pad", 100.0, 20.5, Some("p")),
    ("p 0 t-border", 65.3672, 4.6, Some("p")),
    ("p 0 t-pad", 65.3672, 8.0, Some("p")),
    ("p 0 b-border", 65.3672, 36.4, Some("p")),
    ("p 0 b-pad", 65.3672, 32.5, Some("p")),
    ("p 0 l-out", 23.2344, 20.5, None),
    ("p 0 r-out", 107.5, 20.5, None),
    ("p 1 l-border", 27.3344, 60.5, Some("p")),
    ("p 1 l-pad", 33.2344, 60.5, Some("p")),
    ("p 1 r-border", 109.4, 60.5, Some("p")),
    ("p 1 r-pad", 104.0, 60.5, Some("p")),
    ("p 1 t-border", 68.3672, 44.6, Some("p")),
    ("p 1 t-pad", 68.3672, 48.0, Some("p")),
    ("p 1 b-border", 68.3672, 76.4, Some("p")),
    ("p 1 b-pad", 68.3672, 72.5, Some("p")),
    ("p 1 l-out", 25.2344, 60.5, None),
    ("q 0 l-border", 168.3344, 21.5, Some("q")),
    ("q 0 l-pad", 173.2344, 21.5, Some("q")),
    ("q 0 r-border", 249.4, 21.5, Some("q")),
    ("q 0 r-pad", 244.0, 21.5, Some("q")),
    ("q 0 t-border", 208.8672, 5.6, Some("q")),
    ("q 0 t-pad", 208.8672, 8.5, Some("q")),
    ("q 0 b-border", 208.8672, 37.4, Some("q")),
    ("q 0 b-pad", 208.8672, 33.0, Some("q")),
    ("q 0 l-out", 166.2344, 21.5, None),
    ("q 1 l-border", 178.3344, 61.5, Some("q")),
    ("q 1 l-pad", 183.2344, 61.5, Some("q")),
    ("q 1 r-border", 249.4, 61.5, Some("q")),
    ("q 1 r-pad", 244.0, 61.5, Some("q")),
    ("q 1 t-border", 213.8672, 45.6, Some("q")),
    ("q 1 t-pad", 213.8672, 48.5, Some("q")),
    ("q 1 b-border", 213.8672, 77.4, Some("q")),
    ("q 1 b-pad", 213.8672, 73.0, Some("q")),
    ("q 1 l-out", 176.2344, 61.5, None),
    ("q 2 l-border", 170.3344, 101.5, Some("q")),
    ("q 2 l-pad", 175.2344, 101.5, Some("q")),
    ("q 2 r-border", 249.4, 101.5, Some("q")),
    ("q 2 r-pad", 244.0, 101.5, Some("q")),
    ("q 2 t-border", 209.8672, 85.6, Some("q")),
    ("q 2 t-pad", 209.8672, 88.5, Some("q")),
    ("q 2 b-border", 209.8672, 117.4, Some("q")),
    ("q 2 b-pad", 209.8672, 113.0, Some("q")),
    ("q 2 l-out", 168.2344, 101.5, None),
    ("r 0 l-border", 13.6, 220.0, Some("r")),
    ("r 0 l-pad", 16.5, 220.0, Some("r")),
    ("r 0 r-border", 98.6656, 220.0, Some("s")),
    ("r 0 r-pad", 95.7656, 220.0, Some("s")),
    ("r 0 t-border", 56.1328, 209.6, Some("s")),
    ("r 0 t-pad", 56.1328, 210.25, Some("s")),
    ("r 0 b-border", 56.1328, 230.4, Some("s")),
    ("r 0 b-pad", 56.1328, 229.75, Some("s")),
    ("r 0 l-out", 11.5, 220.0, None),
    ("r 0 r-out", 100.7656, 220.0, None),
    ("r 1 l-border", 10.6, 260.0, Some("s")),
    ("r 1 l-pad", 13.5, 260.0, Some("s")),
    ("r 1 r-border", 81.6656, 260.0, Some("s")),
    ("r 1 r-pad", 78.7656, 260.0, Some("s")),
    ("r 1 t-border", 46.1328, 249.6, Some("s")),
    ("r 1 t-pad", 46.1328, 250.25, Some("s")),
    ("r 1 b-border", 46.1328, 270.4, Some("s")),
    ("r 1 b-pad", 46.1328, 269.75, Some("s")),
    ("r 1 r-out", 83.7656, 260.0, None),
    ("r 2 l-border", 10.6, 300.0, Some("s")),
    ("r 2 l-pad", 13.5, 300.0, Some("s")),
    ("r 2 r-border", 93.6656, 300.0, Some("r")),
    ("r 2 r-pad", 90.7656, 300.0, Some("r")),
    ("r 2 t-border", 52.1328, 289.6, Some("s")),
    ("r 2 t-pad", 52.1328, 290.25, Some("s")),
    ("r 2 b-border", 52.1328, 310.4, Some("s")),
    ("r 2 b-pad", 52.1328, 309.75, Some("s")),
    ("r 2 r-out", 95.7656, 300.0, None),
    ("s 0 l-border", 19.6, 219.5, Some("s")),
    ("s 0 l-pad", 24.0, 219.5, Some("s")),
    ("s 0 r-border", 98.6656, 219.5, Some("s")),
    ("s 0 r-pad", 95.2656, 219.5, Some("s")),
    ("s 0 t-border", 59.1328, 205.6, Some("s")),
    ("s 0 t-pad", 59.1328, 208.5, Some("s")),
    ("s 0 b-border", 59.1328, 233.4, Some("s")),
    ("s 0 b-pad", 59.1328, 231.0, Some("s")),
    ("s 0 l-out", 17.5, 219.5, Some("r")),
    ("s 0 r-out", 100.7656, 219.5, None),
    ("s 1 l-border", 10.6, 259.5, Some("s")),
    ("s 1 l-pad", 15.0, 259.5, Some("s")),
    ("s 1 r-border", 81.6656, 259.5, Some("s")),
    ("s 1 r-pad", 78.2656, 259.5, Some("s")),
    ("s 1 t-border", 46.1328, 245.6, Some("s")),
    ("s 1 t-pad", 46.1328, 248.5, Some("s")),
    ("s 1 b-border", 46.1328, 273.4, Some("s")),
    ("s 1 b-pad", 46.1328, 271.0, Some("s")),
    ("s 1 r-out", 83.7656, 259.5, None),
    ("s 2 l-border", 10.6, 299.5, Some("s")),
    ("s 2 l-pad", 15.0, 299.5, Some("s")),
    ("s 2 r-border", 87.6656, 299.5, Some("s")),
    ("s 2 r-pad", 84.2656, 299.5, Some("s")),
    ("s 2 t-border", 49.1328, 285.6, Some("s")),
    ("s 2 t-pad", 49.1328, 288.5, Some("s")),
    ("s 2 b-border", 49.1328, 313.4, Some("s")),
    ("s 2 b-pad", 49.1328, 311.0, Some("s")),
    ("s 2 r-out", 89.7656, 299.5, Some("r")),
    ("t 0 l-border", 183.3344, 220.0, Some("u")),
    ("t 0 l-pad", 186.2344, 220.0, Some("u")),
    ("t 0 r-border", 266.4, 220.0, Some("t")),
    ("t 0 r-pad", 263.5, 220.0, Some("t")),
    ("t 0 t-border", 224.8672, 209.6, Some("u")),
    ("t 0 t-pad", 224.8672, 210.25, Some("u")),
    ("t 0 b-border", 224.8672, 230.4, Some("u")),
    ("t 0 b-pad", 224.8672, 229.75, Some("u")),
    ("t 0 l-out", 181.2344, 220.0, None),
    ("t 0 r-out", 268.5, 220.0, None),
    ("t 1 l-border", 198.3344, 260.0, Some("u")),
    ("t 1 l-pad", 201.2344, 260.0, Some("u")),
    ("t 1 r-border", 269.4, 260.0, Some("u")),
    ("t 1 r-pad", 266.5, 260.0, Some("u")),
    ("t 1 t-border", 233.8672, 249.6, Some("u")),
    ("t 1 t-pad", 233.8672, 250.25, Some("u")),
    ("t 1 b-border", 233.8672, 270.4, Some("u")),
    ("t 1 b-pad", 233.8672, 269.75, Some("u")),
    ("t 1 l-out", 196.2344, 260.0, None),
    ("t 2 l-border", 184.3344, 300.0, Some("t")),
    ("t 2 l-pad", 187.2344, 300.0, Some("t")),
    ("t 2 r-border", 269.4, 300.0, Some("u")),
    ("t 2 r-pad", 266.5, 300.0, Some("u")),
    ("t 2 t-border", 226.8672, 289.6, Some("u")),
    ("t 2 t-pad", 226.8672, 290.25, Some("u")),
    ("t 2 b-border", 226.8672, 310.4, Some("u")),
    ("t 2 b-pad", 226.8672, 309.75, Some("u")),
    ("t 2 l-out", 182.2344, 300.0, None),
    ("u 0 l-border", 183.3344, 219.5, Some("u")),
    ("u 0 l-pad", 187.7344, 219.5, Some("u")),
    ("u 0 r-border", 260.4, 219.5, Some("u")),
    ("u 0 r-pad", 257.0, 219.5, Some("u")),
    ("u 0 t-border", 221.8672, 205.6, Some("u")),
    ("u 0 t-pad", 221.8672, 208.5, Some("u")),
    ("u 0 b-border", 221.8672, 233.4, Some("u")),
    ("u 0 b-pad", 221.8672, 231.0, Some("u")),
    ("u 0 l-out", 181.2344, 219.5, None),
    ("u 0 r-out", 262.5, 219.5, Some("t")),
    ("u 1 l-border", 198.3344, 259.5, Some("u")),
    ("u 1 l-pad", 202.7344, 259.5, Some("u")),
    ("u 1 r-border", 269.4, 259.5, Some("u")),
    ("u 1 r-pad", 266.0, 259.5, Some("u")),
    ("u 1 t-border", 233.8672, 245.6, Some("u")),
    ("u 1 t-pad", 233.8672, 248.5, Some("u")),
    ("u 1 b-border", 233.8672, 273.4, Some("u")),
    ("u 1 b-pad", 233.8672, 271.0, Some("u")),
    ("u 1 l-out", 196.2344, 259.5, None),
    ("u 2 l-border", 190.3344, 299.5, Some("u")),
    ("u 2 l-pad", 194.7344, 299.5, Some("u")),
    ("u 2 r-border", 269.4, 299.5, Some("u")),
    ("u 2 r-pad", 266.0, 299.5, Some("u")),
    ("u 2 t-border", 229.8672, 285.6, Some("u")),
    ("u 2 t-pad", 229.8672, 288.5, Some("u")),
    ("u 2 b-border", 229.8672, 313.4, Some("u")),
    ("u 2 b-pad", 229.8672, 311.0, Some("u")),
    ("u 2 l-out", 188.2344, 299.5, Some("t")),
    ("e 0 l-border", 10.6, 420.0, Some("e")),
    ("e 0 l-pad", 15.0, 420.0, Some("e")),
    ("e 0 r-border", 89.6656, 420.0, Some("e")),
    ("e 0 r-pad", 85.2656, 420.0, Some("e")),
    ("e 0 t-border", 50.1328, 408.6, Some("e")),
    ("e 0 t-pad", 50.1328, 410.25, Some("e")),
    ("e 0 b-border", 50.1328, 431.4, Some("e")),
    ("e 0 b-pad", 50.1328, 429.75, Some("e")),
    ("e 0 r-out", 91.7656, 420.0, None),
    ("e 1 l-border", 10.6, 460.0, Some("e")),
    ("e 1 l-pad", 15.0, 460.0, Some("e")),
    ("e 1 r-border", 89.6656, 460.0, Some("e")),
    ("e 1 r-pad", 85.2656, 460.0, Some("e")),
    ("e 1 t-border", 50.1328, 448.6, Some("e")),
    ("e 1 t-pad", 50.1328, 450.25, Some("e")),
    ("e 1 b-border", 50.1328, 471.4, Some("e")),
    ("e 1 b-pad", 50.1328, 469.75, Some("e")),
    ("e 1 r-out", 91.7656, 460.0, None),
    ("g 0 l-border", 176.3344, 420.0, Some("g")),
    ("g 0 l-pad", 178.2344, 420.0, Some("g")),
    ("g 0 r-border", 249.4, 420.0, Some("g")),
    ("g 0 r-pad", 247.5, 420.0, Some("g")),
    ("g 0 t-border", 212.8672, 408.6, Some("g")),
    ("g 0 t-pad", 212.8672, 410.25, Some("g")),
    ("g 0 b-border", 212.8672, 431.4, Some("g")),
    ("g 0 b-pad", 212.8672, 429.75, Some("g")),
    ("g 0 l-out", 174.2344, 420.0, None),
    ("g 1 l-border", 178.3344, 460.0, Some("g")),
    ("g 1 l-pad", 180.2344, 460.0, Some("g")),
    ("g 1 r-border", 249.4, 460.0, Some("g")),
    ("g 1 r-pad", 247.5, 460.0, Some("g")),
    ("g 1 t-border", 213.8672, 448.6, Some("g")),
    ("g 1 t-pad", 213.8672, 450.25, Some("g")),
    ("g 1 b-border", 213.8672, 471.4, Some("g")),
    ("g 1 b-pad", 213.8672, 469.75, Some("g")),
    ("g 1 l-out", 176.2344, 460.0, None),
    ("g 2 l-border", 176.3344, 500.0, Some("g")),
    ("g 2 l-pad", 178.2344, 500.0, Some("g")),
    ("g 2 r-border", 249.4, 500.0, Some("g")),
    ("g 2 r-pad", 247.5, 500.0, Some("g")),
    ("g 2 t-border", 212.8672, 488.6, Some("g")),
    ("g 2 t-pad", 212.8672, 490.25, Some("g")),
    ("g 2 b-border", 212.8672, 511.4, Some("g")),
    ("g 2 b-pad", 212.8672, 509.75, Some("g")),
    ("g 2 l-out", 174.2344, 500.0, None),
    ("v 0 l-border", 29.3344, 560.5, Some("v")),
    ("v 0 l-pad", 35.2344, 560.5, Some("v")),
    ("v 0 r-border", 109.4, 560.5, Some("v")),
    ("v 0 r-pad", 104.0, 560.5, Some("v")),
    ("v 0 t-border", 69.3672, 544.6, Some("v")),
    ("v 0 t-pad", 69.3672, 548.0, Some("v")),
    ("v 0 b-border", 69.3672, 576.4, Some("v")),
    ("v 0 b-pad", 69.3672, 572.5, Some("v")),
    ("v 0 l-out", 27.2344, 560.5, None),
    ("v 1 l-border", 38.3344, 600.5, Some("v")),
    ("v 1 l-pad", 44.2344, 600.5, Some("v")),
    ("v 1 r-border", 109.4, 600.5, Some("v")),
    ("v 1 r-pad", 104.0, 600.5, Some("v")),
    ("v 1 t-border", 73.8672, 584.6, Some("v")),
    ("v 1 t-pad", 73.8672, 588.0, Some("v")),
    ("v 1 b-border", 73.8672, 616.4, Some("v")),
    ("v 1 b-pad", 73.8672, 612.5, Some("v")),
    ("v 1 l-out", 36.2344, 600.5, None),
    ("v 2 l-border", 70.6938, 640.5, Some("v")),
    ("v 2 l-pad", 76.5938, 640.5, Some("v")),
    ("v 2 r-border", 109.4001, 640.5, Some("v")),
    ("v 2 r-pad", 104.0001, 640.5, Some("v")),
    ("v 2 t-border", 90.047, 624.6, Some("v")),
    ("v 2 t-pad", 90.047, 628.0, Some("v")),
    ("v 2 b-border", 90.047, 656.4, Some("v")),
    ("v 2 b-pad", 90.047, 652.5, Some("v")),
    ("v 2 l-out", 68.5938, 640.5, None),
    ("w 0 l-border", 150.6, 560.5, Some("w")),
    ("w 0 l-pad", 156.5, 560.5, Some("w")),
    ("w 0 r-border", 232.6656, 560.5, Some("w")),
    ("w 0 r-pad", 227.2656, 560.5, Some("w")),
    ("w 0 t-border", 191.6328, 544.6, Some("w")),
    ("w 0 t-pad", 191.6328, 548.0, Some("w")),
    ("w 0 b-border", 191.6328, 576.4, Some("w")),
    ("w 0 b-pad", 191.6328, 572.5, Some("w")),
    ("w 0 r-out", 234.7656, 560.5, None),
    ("w 1 l-border", 150.6, 600.5, Some("w")),
    ("w 1 l-pad", 156.5, 600.5, Some("w")),
    ("w 1 r-border", 221.6656, 600.5, Some("w")),
    ("w 1 r-pad", 216.2656, 600.5, Some("w")),
    ("w 1 t-border", 186.1328, 584.6, Some("w")),
    ("w 1 t-pad", 186.1328, 588.0, Some("w")),
    ("w 1 b-border", 186.1328, 616.4, Some("w")),
    ("w 1 b-pad", 186.1328, 612.5, Some("w")),
    ("w 1 r-out", 223.7656, 600.5, None),
    ("w 2 l-border", 150.6, 640.5, Some("w")),
    ("w 2 l-pad", 156.5, 640.5, Some("w")),
    ("w 2 r-border", 187.3063, 640.5, Some("w")),
    ("w 2 r-pad", 181.9063, 640.5, Some("w")),
    ("w 2 t-border", 168.9532, 624.6, Some("w")),
    ("w 2 t-pad", 168.9532, 628.0, Some("w")),
    ("w 2 b-border", 168.9532, 656.4, Some("w")),
    ("w 2 b-pad", 168.9532, 652.5, Some("w")),
    ("w 2 r-out", 189.4063, 640.5, None),
    ("x 0 l-border", 10.6, 700.0, Some("x")),
    ("x 0 l-pad", 13.5, 700.0, Some("x")),
    ("x 0 r-border", 88.6656, 700.0, Some("x")),
    ("x 0 r-pad", 85.7656, 700.0, Some("x")),
    ("x 0 t-border", 49.6328, 684.6, Some("x")),
    ("x 0 t-pad", 49.6328, 687.0, Some("x")),
    ("x 0 b-border", 49.6328, 715.4, Some("x")),
    ("x 0 b-pad", 49.6328, 713.0, Some("x")),
    ("x 0 r-out", 90.7656, 700.0, None),
    ("x 1 l-border", 10.6, 740.0, Some("x")),
    ("x 1 l-pad", 13.5, 740.0, Some("x")),
    ("x 1 r-border", 88.6656, 740.0, Some("x")),
    ("x 1 r-pad", 85.7656, 740.0, Some("x")),
    ("x 1 t-border", 49.6328, 724.6, Some("x")),
    ("x 1 t-pad", 49.6328, 727.0, Some("x")),
    ("x 1 b-border", 49.6328, 755.4, Some("x")),
    ("x 1 b-pad", 49.6328, 753.0, Some("x")),
    ("x 1 r-out", 90.7656, 740.0, None),
];

/// The page, laid out at `W` x `H`.
pub fn document() -> RinchDocument {
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
    doc.load_css(CSS);
    let b = doc.body();
    doc.set_inner_html(b, BODY);
    doc.resolve_layout(W as f32, H as f32);
    doc
}
