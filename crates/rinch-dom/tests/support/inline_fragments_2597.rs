//! The scene and Chrome's boxes of #2597 and #2607, shared by the software
//! painter's `inline_fragments_2597_tests` and the GPU painter's
//! `gpu_inline_fragments_2597` (`rinch/tests`, through `#[path]`):
//!
//! - #2597: an inline element holding only an atomic inline drew no box.
//! - #2607: a wrapped inline element's fragment took in the white space its
//!   line ends with, and its open ends were a pixel too long.
//!
//! The text is set in this crate's Inter (registered as `ProbeFace`, loaded by
//! Chrome through `@font-face` from the same file) and is transparent, so the
//! boxes are the only ink and their horizontal geometry is the font's, not the
//! host's.

#![allow(dead_code)]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

pub const W: u32 = 300;
pub const H: u32 = 600;

/// Chrome 157's page: the same body under
/// `@font-face { font-family: ProbeFace; src: url(Inter-Regular.ttf) }`.
pub const BODY: &str = r#"
<div style="left:10px;top:0"><span id=a style="border:2px solid #000;background:#ff0;padding:0 3px"><i style="width:101px"></i></span></div>
<div style="left:150px;top:0"><span id=b style="background:#ff0"><i style="width:37px"></i></span></div>
<div dir=rtl style="left:10px;top:50px;width:200px"><span id=c style="border:2px solid #000;background:#ff0;padding:0 5px 0 1px"><i style="width:53px"></i></span></div>
<div style="left:10px;top:100px;width:70px"><span id=d style="border:2px solid #000;background:#ff0"><i style="width:41px"></i> <i style="width:43px"></i> <i style="width:39px"></i></span></div>
<div style="left:10px;top:230px;width:100px"><span id=e style="border:2px solid #000">MMMMM MMMMM</span></div>
<div style="left:10px;top:320px;width:90px"><span id=f style="border:3px solid #000;background:#ff0;padding:0 4px">lorem ipsum dolor sit</span></div>
<div dir=rtl style="left:10px;top:460px;width:100px"><span id=g style="border:2px solid #000;background:#ff0">MMMMM MMMMM MMMMM</span></div>
<div style="left:150px;top:100px;width:110px">ab <span id=h style="border:2px solid #000;background:#ff0">MMMM </span>MMMMM</div>
<div style="left:150px;top:230px;width:100px;white-space:pre-wrap"><span id=k style="border:2px solid #000">MMMMM MMMMM</span></div>
<div style="left:150px;top:320px;width:100px"><span id=l style="border:2px solid #000">MMMM&nbsp; MMMMM</span></div>
<div style="left:150px;top:400px;width:140px">ab <span id=m style="border:2px solid #000;background:#ff0">MMM </span><i style="width:20px"></i></div>
<div style="left:150px;top:450px;width:100px">MMMMMMM<span id=n style="border:2px solid #000;background:#ff0"> </span>MMMM</div>
<div style="left:150px;top:540px;width:100px">ab <span id=o style="border:2px solid #000;background:#ff0;padding:0 3px"><i style="width:20px"></i> </span>MMMMMMM</div>
"#;

pub const CSS: &str = "html, body { margin: 0; background: #fff; }
     div { position: absolute; font: 16px/40px ProbeFace; }
     i { display: inline-block; height: 10px; }
     span, div { color: transparent; }";

/// One fragment as Chrome 157 reports it (`getClientRects`, CSS px) and
/// draws it: `(x, y, width, height)`, its border width, and whether it draws
/// its left and its right side.
pub struct Fragment {
    pub rect: (f64, f64, f64, f64),
    pub border: f64,
    pub left: bool,
    pub right: bool,
}

const fn frag(rect: (f64, f64, f64, f64), border: f64, left: bool, right: bool) -> Fragment {
    Fragment {
        rect,
        border,
        left,
        right,
    }
}

/// Each element's fragments, measured in Chrome 157 at 1x. Its
/// `getBoundingClientRect` is their union.
pub const CHROME: &[(&str, &[Fragment])] = &[
    // #2597: only an atomic inline, with a border and padding; with a
    // background only; right-to-left, its padding uneven.
    ("a", &[frag((10.0, 8.0, 111.0, 24.0), 2.0, true, true)]),
    ("b", &[frag((150.0, 10.0, 37.0, 20.0), 0.0, false, false)]),
    ("c", &[frag((147.0, 58.0, 63.0, 24.0), 2.0, true, true)]),
    // Three atomic inlines on three lines, a space after each of the first
    // two, which their lines hang.
    (
        "d",
        &[
            frag((10.0, 108.0, 43.0, 24.0), 2.0, true, false),
            frag((10.0, 148.0, 43.0, 24.0), 2.0, false, false),
            frag((10.0, 188.0, 41.0, 24.0), 2.0, false, true),
        ],
    ),
    // #2607: text on two lines, on three, on three right-to-left; and an
    // element ending in the white space its line ends with.
    (
        "e",
        &[
            frag((10.0, 238.0, 74.2656, 24.0), 2.0, true, false),
            frag((10.0, 278.0, 74.2656, 24.0), 2.0, false, true),
        ],
    ),
    (
        "f",
        &[
            frag((10.0, 327.0, 49.5781, 26.0), 3.0, true, false),
            frag((10.0, 367.0, 88.9844, 26.0), 3.0, false, false),
            frag((10.0, 407.0, 24.5625, 26.0), 3.0, false, true),
        ],
    ),
    (
        "g",
        &[
            frag((35.7344, 468.0, 74.2656, 24.0), 2.0, false, true),
            frag((37.7344, 508.0, 72.2656, 24.0), 2.0, false, false),
            frag((35.7344, 548.0, 74.2656, 24.0), 2.0, true, false),
        ],
    ),
    (
        "h",
        &[frag((173.2813, 108.0, 61.8125, 24.0), 2.0, true, true)],
    ),
    // What is not removed: a preserved space (`pre-wrap`) the line ends
    // with, and an NBSP before the space it ends with.
    (
        "k",
        &[
            frag((150.0, 238.0, 78.7656, 24.0), 2.0, true, false),
            frag((150.0, 278.0, 74.2656, 24.0), 2.0, false, true),
        ],
    ),
    (
        "l",
        &[
            frag((150.0, 328.0, 64.3125, 24.0), 2.0, true, false),
            frag((150.0, 368.0, 74.2656, 24.0), 2.0, false, true),
        ],
    ),
    // From the review of #2623: a space with an inline-block after it on
    // its line is not at the line's end; an element holding only the space
    // its line ends with is its border alone; and the space after an
    // inline-block at a wrap is removed though a space comes before it.
    (
        "m",
        &[frag((173.2813, 408.0, 51.8594, 24.0), 2.0, true, true)],
    ),
    ("n", &[frag((251.1719, 458.0, 4.0, 24.0), 2.0, true, true)]),
    ("o", &[frag((173.2813, 548.0, 30.0, 24.0), 2.0, true, true)]),
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

fn lum(p: [u8; 4]) -> u8 {
    ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8
}

/// Inked: anything not white (the yellow background included).
fn inked(p: [u8; 4]) -> bool {
    lum(p) < 250 || p[2] < 200
}

/// Where `px` (the page painted at `scale` over white, `pw` device px wide)
/// is not Chrome's: each fragment's inked box is its rectangle with each edge
/// on the nearest device pixel, and halfway down it the dark runs are its
/// left and right sides where it has them, the border width wide.
///
/// The fragment's height is checked at 1x only: at 2x Chrome takes the font's
/// rounded ascent and descent at the device size (39 device px for 16px
/// Inter's 20, not 40), which rinch does not (and did not before #2607).
pub fn misses(px: &[[u8; 4]], pw: u32, scale: f64) -> Vec<String> {
    let ph = px.len() as u32 / pw;
    let at = |x: u32, y: u32| px[(y * pw + x) as usize];
    let snap = |v: f64| (v * scale).round() as u32;
    let mut out = Vec::new();
    for (id, frags) in CHROME {
        for (n, f) in frags.iter().enumerate() {
            let (x, y, w, h) = f.rect;
            let (x0, y0, x1, y1) = (snap(x), snap(y), snap(x + w), snap(y + h));
            // The ink in a window a few px around it (the scene keeps other
            // ink further away than that).
            let m = (3.0 * scale) as u32;
            let (mut ix0, mut iy0, mut ix1, mut iy1) = (u32::MAX, u32::MAX, 0, 0);
            for yy in y0.saturating_sub(m)..(y1 + m).min(ph) {
                for xx in x0.saturating_sub(m)..(x1 + m).min(pw) {
                    if inked(at(xx, yy)) {
                        (ix0, iy0) = (ix0.min(xx), iy0.min(yy));
                        (ix1, iy1) = (ix1.max(xx + 1), iy1.max(yy + 1));
                    }
                }
            }
            let what = format!("#{id} fragment {n} at {scale}x");
            if (ix0, ix1) != (x0, x1) {
                out.push(format!(
                    "{what}: ink from x = {ix0} to {ix1}, Chrome {x0} to {x1}"
                ));
            }
            if scale == 1.0 && (iy0, iy1) != (y0, y1) {
                out.push(format!(
                    "{what}: ink from y = {iy0} to {iy1}, Chrome {y0} to {y1}"
                ));
            }
            // Halfway down: the sides it draws.
            let mid = (y0 + y1) / 2;
            let mut dark: Vec<(u32, u32)> = Vec::new();
            for xx in x0.saturating_sub(m)..(x1 + m).min(pw) {
                if lum(at(xx, mid)) < 100 {
                    match dark.last_mut() {
                        Some(r) if r.1 == xx => r.1 = xx + 1,
                        _ => dark.push((xx, xx + 1)),
                    }
                }
            }
            let bw = (f.border * scale).round() as u32;
            let mut want = Vec::new();
            if f.left {
                want.push((x0, x0 + bw));
            }
            if f.right {
                match want.last_mut() {
                    Some(l) if l.1 >= x1 - bw => l.1 = x1,
                    _ => want.push((x1 - bw, x1)),
                }
            }
            if dark != want {
                out.push(format!(
                    "{what}: sides at {dark:?} halfway down, Chrome {want:?}"
                ));
            }
        }
    }
    out
}

/// The union of an element's fragments as Chrome reports it
/// (`getBoundingClientRect`), CSS px.
pub fn chrome_union(id: &str) -> (f64, f64, f64, f64) {
    let frags = CHROME.iter().find(|(i, _)| *i == id).unwrap().1;
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for f in frags {
        let (x, y, w, h) = f.rect;
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + w), y1.max(y + h));
    }
    (x0, y0, x1 - x0, y1 - y0)
}

/// Points over the lines and what Chrome 157's `elementFromPoint` finds
/// there: `Some(id)` an element's text, `None` the block — the white space a
/// line hangs is past the element's fragment.
pub const HITS: &[(f32, f32, Option<&str>)] = &[
    (50.0, 258.0, Some("e")),
    (86.0, 258.0, None),
    (80.0, 298.0, Some("e")),
    (58.0, 340.0, Some("f")),
    (40.0, 480.0, Some("g")),
    (37.0, 520.0, None),
    (232.0, 120.0, Some("h")),
    (237.0, 120.0, None),
    (226.0, 258.0, Some("k")),
    (212.0, 348.0, Some("l")),
];

/// A solid side's ends (review of #2623): butt caps (#2607) on whole device
/// pixels, so a side beside an absent one stops at the box, and the sides of
/// boxes meeting at a fractional device pixel meet without a seam, as Chrome
/// 157 draws them.
pub mod seams {
    use super::*;

    pub const W: u32 = 300;
    pub const H: u32 = 200;

    pub const BODY: &str = r#"
<div style="left:10px;top:10px;width:50px;height:30px;border-top:6px solid #000"></div>
<div style="left:80px;top:10px;width:30px;height:40px;border-left:6px solid #000"></div>
<div class=f style="left:130px;top:10px;width:100.4px"><div></div><div></div><div></div></div>
<table><tr><td>Name</td><td>Qty</td><td>Price</td><td>Total</td></tr><tr><td>Apple</td><td>3</td><td>1.25</td><td>3.75</td></tr><tr><td>Banana pie</td><td>12</td><td>0.5</td><td>6</td></tr></table>
"#;

    pub const CSS: &str = "html, body { margin: 0; background: #fff; }
         div { position: absolute; box-sizing: border-box; }
         .f { display: flex; }
         .f > div { position: static; flex: 1; height: 10px; border-top: 4px solid #000; }
         td { border-bottom: 1px solid #888; padding: 2px 7px; font: 13px/17px ProbeFace;
              color: transparent; }
         table { border-collapse: collapse; position: absolute; left: 10px; top: 60px; }";

    pub fn document() -> RinchDocument {
        use parley::fontique::{Blob, FontInfoOverride};
        let mut doc = RinchDocument::new();
        doc.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(FACE)),
            Some(FontInfoOverride {
                family_name: Some("ProbeFace"),
                ..Default::default()
            }),
        );
        doc.load_css(CSS);
        let b = doc.body();
        doc.set_inner_html(b, BODY);
        doc.resolve_layout(W as f32, H as f32);
        doc
    }

    /// Where `px` (painted at `scale`, 1 or 1.5) is not Chrome's.
    pub fn misses(px: &[[u8; 4]], pw: u32, scale: f64) -> Vec<String> {
        let at = |x: u32, y: u32| lum(px[(y * pw + x) as usize]);
        let d = |v: f64| (v * scale).round() as u32;
        let mut out = Vec::new();
        // The lone top side and the lone left side end at the box, as
        // Chrome draws them (1x: x 10..60 and y 10..50; 1.5x: 15..90 and
        // 15..75): no cap past either end.
        let row = d(12.0);
        let ink: Vec<u32> = (0..d(70.0)).filter(|&x| at(x, row) < 250).collect();
        if ink.first() != Some(&d(10.0)) || ink.last() != Some(&(d(60.0) - 1)) {
            out.push(format!(
                "top side's row {row}: ink {:?}..{:?}, Chrome {}..{}",
                ink.first(),
                ink.last(),
                d(10.0),
                d(60.0) - 1
            ));
        }
        let col = d(82.0);
        let ink: Vec<u32> = (0..d(60.0)).filter(|&y| at(col, y) < 250).collect();
        if ink.first() != Some(&d(10.0)) || ink.last() != Some(&(d(50.0) - 1)) {
            out.push(format!(
                "left side's column {col}: ink {:?}..{:?}, Chrome {}..{}",
                ink.first(),
                ink.last(),
                d(10.0),
                d(50.0) - 1
            ));
        }
        // Three `flex: 1` boxes in 100.4px: their top sides one unbroken
        // black band, 130 to 230.4 (Chrome: every pixel 0 at 1x and 1.5x).
        for y in d(10.0) + 1..d(14.0) - 1 {
            let lighter: Vec<(u32, u8)> = (d(130.0) + 1..d(230.4) - 1)
                .map(|x| (x, at(x, y)))
                .filter(|&(_, l)| l > 8)
                .collect();
            if !lighter.is_empty() {
                out.push(format!("flex row {y}: lighter pixels {lighter:?}"));
            }
        }
        // A collapsed table's row lines: each row of device pixels is one
        // level all across the table (10 to 217.3), no lighter seam where
        // two cells meet.
        for y in d(60.0)..d(130.0) {
            let line: Vec<u8> = (d(10.0) + 2..d(217.3) - 2).map(|x| at(x, y)).collect();
            let (lo, hi) = (line.iter().min().unwrap(), line.iter().max().unwrap());
            if *lo < 250 && hi - lo > 3 {
                let x = d(10.0) + 2 + line.iter().position(|l| l == hi).unwrap() as u32;
                out.push(format!(
                    "table row {y}: from {lo} to {hi} (lightest at x = {x})"
                ));
            }
        }
        out
    }
}
