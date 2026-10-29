//! #893 — an auto-width `position: fixed` box is **shrunk to fit** (CSS 2.1
//! §10.3.7, §10.6.4), as an absolute one is: its width is the smaller of its
//! preferred width and what the viewport — its containing block — leaves it
//! after the insets, capped by `max-width`; with one vertical inset (or none)
//! its height is its content's.
//!
//! rinch filled the viewport instead: an unpaired inset gave the width (and
//! height) of the whole viewport, so a tooltip with `max-width: 420px` was
//! drawn 420 wide however short its text, and a fixed box with no insets at
//! all covered the window.
//!
//! Every number in `CHROME` is **Chrome 156**'s (`--headless=new`, standards
//! mode, `* { box-sizing: border-box }`, an 800x600 viewport, device scale
//! factor 1, the bundled Inter registered as `ProbeFace` through
//! `@font-face`, `body { margin: 0; font: 16px/20px ProbeFace }`),
//! `getBoundingClientRect` in the viewport: `mark=x,y,width,height`. The `t`
//! mark is an `inline-block` holding a line whose max-content width is 544.2
//! and whose longest word is 66.1: an `auto` atomic inline is capped at the
//! width its line gives it (#1476), so the box is exactly as wide as the room
//! it was given whenever that is under 544.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// (name, html, Chrome 156).
const CHROME: &[(&str, &str, &str)] = &[
    (
        "f01_issue_tooltip",
        r##"<div data-m="a" style="position:fixed;left:280px;top:200px;max-width:420px;padding:4px 8px;border:1px solid;font-size:12px;line-height:16px"><div>VENDORS - Family Management</div><div>Ctrl+click to open</div></div>"##,
        "a=280,200,202.39,42",
    ),
    (
        "f02_short_left_top",
        r##"<div data-m="a" style="position:fixed;left:20px;top:30px;max-width:420px">Ctrl+click to open</div>"##,
        "a=20,30,134.31,20",
    ),
    (
        "f03_long_max_width",
        r##"<div data-m="a" style="position:fixed;left:20px;top:30px;max-width:300px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "a=20,30,300,40 t=20,30,300,40",
    ),
    (
        "f04_long_text_max_width",
        r##"<div data-m="a" style="position:fixed;left:20px;top:30px;max-width:300px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</div>"##,
        "a=20,30,300,40",
    ),
    (
        "f05_padding_border",
        r##"<div data-m="a" style="position:fixed;left:20px;top:30px;padding:4px 8px;border:3px solid">Ctrl+click to open</div>"##,
        "a=20,30,156.31,34",
    ),
    (
        "f06_left_and_right",
        r##"<div data-m="a" style="position:fixed;left:20px;right:30px;top:30px">Ctrl+click to open</div>"##,
        "a=20,30,750,20",
    ),
    (
        "f07_right_only",
        r##"<div data-m="a" style="position:fixed;right:20px;top:10px">Ctrl+click to open</div>"##,
        "a=645.69,10,134.31,20",
    ),
    (
        "f08_bottom_only",
        r##"<div data-m="a" style="position:fixed;left:10px;bottom:20px">Ctrl+click to open</div>"##,
        "a=10,560,134.31,20",
    ),
    (
        "f09_static",
        r##"<div style="margin:12px 0 0 100px">text<div data-m="a" style="position:fixed">box</div>tail</div>"##,
        "a=100,32,27.77,20",
    ),
    (
        "f10_static_long",
        r##"<div style="margin-left:300px"><div data-m="a" style="position:fixed"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=300,0,500,40 t=300,0,500,40",
    ),
    (
        "f11_left_in_narrow_parent",
        r##"<div style="width:200px"><div data-m="a" style="position:fixed;left:400px;top:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=400,0,400,40 t=400,0,400,40",
    ),
    (
        "f12_fit_content",
        r##"<div style="width:200px"><div data-m="a" style="position:fixed;left:400px;top:0;width:fit-content"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=400,0,400,40 t=400,0,400,40",
    ),
    (
        "f13_min_width",
        r##"<div data-m="a" style="position:fixed;left:20px;top:30px;min-width:300px">Ctrl+click to open</div>"##,
        "a=20,30,300,20",
    ),
    (
        "f14_transform_short",
        r##"<div style="transform:translateX(0);margin:40px 0 0 50px;width:300px;height:100px"><div data-m="a" style="position:fixed;left:10px;top:10px">Ctrl+click to open</div></div>"##,
        "a=60,50,134.31,20",
    ),
    (
        "f15_transform_long",
        r##"<div style="transform:translateX(0);margin:40px 0 0 50px;width:300px;height:100px"><div data-m="a" style="position:fixed;left:10px;top:10px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=60,50,290,40",
    ),
    (
        "f16_max_width_pct",
        r##"<div style="width:200px"><div data-m="a" style="position:fixed;left:0;top:0;max-width:50%"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "a=0,0,400,40",
    ),
    (
        "f17_right_long",
        r##"<div data-m="a" style="position:fixed;right:300px;top:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "a=0,0,500,40 t=0,0,500,40",
    ),
    // The static position of a fixed box inside another fixed one is
    // measured from that one's viewport position, not from the boxes above
    // it: the room is 800 - 300, not 800 - 400.
    (
        "f18_static_in_fixed",
        r##"<div style="margin-left:100px"><div data-m="o" style="position:fixed;left:300px;top:0;width:400px"><div data-m="a" style="position:fixed"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div></div>"##,
        "o=300,0,400,0 a=300,0,500,40 t=300,0,500,40",
    ),
    // From the #1996 review: percentage limits resolve against the viewport,
    // each against its own axis (10% of 600 tall; 50% of 800 wide).
    (
        "f19_max_height_pct",
        r##"<div data-m="a" style="position:fixed;left:0;top:0;width:100px;max-height:10%;overflow:hidden"><div style="height:200px"></div></div>"##,
        "a=0,0,100,60",
    ),
    (
        "f20_min_width_pct",
        r##"<div data-m="a" style="position:fixed;left:20px;top:30px;min-width:50%;height:10px"></div>"##,
        "a=20,30,400,10",
    ),
];

/// (name, rinch's answer, why) — rows of `CHROME` that are not Chrome's yet,
/// each for a reason that is not this issue's. Pinned so that a fix has to
/// come here and take the row out.
const KNOWN: &[(&str, &str, &str)] = &[
    (
        "f14_transform_short",
        "a=10,10,134,20",
        "#1372: a transformed ancestor is not a fixed box's containing block (the width is Chrome's: the content is narrower than either block)",
    ),
    (
        "f15_transform_long",
        "a=10,10,544,20",
        "#1372: shrunk to fit in the viewport, not in the transformed ancestor",
    ),
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

fn lay_out(html: &str) -> RinchDocument {
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; font: 16px/20px ProbeFace");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    doc
}

fn one(doc: &RinchDocument, selector: &str) -> u64 {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

/// `mark=x,y,width,height` for every mark `want` names, in the viewport.
fn dump(doc: &RinchDocument, want: &str) -> String {
    want.split(' ')
        .map(|part| {
            let m = part.split_once('=').unwrap().0;
            let id = one(doc, &format!("[data-m={m}]"));
            let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
            let l = doc.tree.get(id).unwrap().layout;
            format!(
                "{m}={},{},{},{}",
                num(x as f32),
                num(y as f32),
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
/// edge on a whole pixel, Chrome reports the unrounded box — so a width,
/// whose two edges are rounded apart, can be a whole pixel off.
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
fn fixed_shrink_to_fit_matches_chrome_156() {
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
    assert!(rows >= 10, "the table is being read: {rows} rows");
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

/// Every row, laid out a second time with nothing changed but the viewport
/// height (so the pass is not skipped), then a third time back at the first
/// viewport. Nothing moves the second time but what depends on the
/// viewport's height — a box pinned to its bottom, a percentage height
/// limit (`f19`: 10% of 640 is 64) — and the third layout is the first.
#[test]
fn a_second_layout_moves_nothing() {
    let mut rows = 0;
    let bad: Vec<_> = CHROME
        .iter()
        .flat_map(|(n, h, want)| {
            rows += 1;
            let mut doc = lay_out(h);
            let first = dump(&doc, want);
            doc.resolve_layout(VIEWPORT.0, VIEWPORT.1 + 40.0);
            let second = dump(&doc, want);
            doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
            let third = dump(&doc, want);
            let tall = n.contains("bottom") || n.contains("height_pct");
            [
                (!tall && first != second).then(|| format!("{n}: [{first}] then [{second}]")),
                (first != third).then(|| format!("{n}: [{first}] then back [{third}]")),
            ]
        })
        .flatten()
        .collect();
    assert!(rows >= 10, "the table is being read: {rows} rows");
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The viewports `RESIZE` lays each row out at, in turn, on one document.
const VIEWPORTS: [(f32, f32); 5] = [
    (800.0, 600.0),
    (800.0, 640.0),
    (1000.0, 600.0),
    (600.0, 300.0),
    (800.0, 600.0),
];

/// (name, html, the box at each of `VIEWPORTS`) — not measured in a browser
/// but read off the spec: a percentage `min-`/`max-` size of a fixed box is
/// of the viewport's matching axis (CSS 2.1 §10.4, §10.7), whatever its
/// parent's size. Each sits inside a parent the percentage is **not** of —
/// a 200px-wide one, or an auto-height one — so resolving against the
/// layout parent gives another number. Laid out again at each viewport, the
/// limit follows it.
const RESIZE: &[(&str, &str, [&str; 5])] = &[
    (
        "max_height_pct",
        r##"<div data-m="a" style="position:fixed;left:0;top:0;width:100px;max-height:10%;overflow:hidden"><div style="height:200px"></div></div>"##,
        [
            "a=0,0,100,60",
            "a=0,0,100,64",
            "a=0,0,100,60",
            "a=0,0,100,30",
            "a=0,0,100,60",
        ],
    ),
    (
        "min_height_pct",
        r##"<div data-m="a" style="position:fixed;left:0;top:0;width:100px;min-height:20%"><div style="height:10px"></div></div>"##,
        [
            "a=0,0,100,120",
            "a=0,0,100,128",
            "a=0,0,100,120",
            "a=0,0,100,60",
            "a=0,0,100,120",
        ],
    ),
    (
        "max_width_pct",
        r##"<div style="width:200px"><div data-m="a" style="position:fixed;left:0;top:0;max-width:50%;height:10px"><div style="width:2000px;height:10px"></div></div></div>"##,
        [
            "a=0,0,400,10",
            "a=0,0,400,10",
            "a=0,0,500,10",
            "a=0,0,300,10",
            "a=0,0,400,10",
        ],
    ),
    (
        "min_width_pct",
        r##"<div style="width:200px"><div data-m="a" style="position:fixed;left:20px;top:30px;min-width:50%;height:10px"></div></div>"##,
        [
            "a=20,30,400,10",
            "a=20,30,400,10",
            "a=20,30,500,10",
            "a=20,30,300,10",
            "a=20,30,400,10",
        ],
    ),
];

#[test]
fn percentage_limits_follow_the_viewport() {
    let mut checked = 0;
    let bad: Vec<_> = RESIZE
        .iter()
        .flat_map(|(n, h, wants)| {
            let mut doc = lay_out(h);
            VIEWPORTS
                .iter()
                .zip(wants)
                .enumerate()
                .filter_map(|(i, (&(w, vh), want))| {
                    if i > 0 {
                        doc.resolve_layout(w, vh);
                    }
                    checked += 1;
                    let got = dump(&doc, want);
                    (got != *want).then(|| format!("{n} at {w}x{vh}: want[{want}] rinch[{got}]"))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        checked,
        RESIZE.len() * VIEWPORTS.len(),
        "every layout is checked"
    );
    assert!(bad.is_empty(), "{bad:#?}");
}
