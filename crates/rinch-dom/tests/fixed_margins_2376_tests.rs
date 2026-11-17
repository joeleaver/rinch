//! #2376 — a `position: fixed` box's horizontal margins offset it and come
//! out of the room it is sized in, as an absolute box's do (CSS 2.1 §10.3.7):
//! `left: 10px; margin-left: 23px` puts its border box at 33, and between
//! `left: 10px; right: 10px` a `margin: 0 23px` narrows it by 46.
//!
//! Every number in `CHROME` is **Chrome 153**'s (Playwright's
//! `chrome-headless-shell`, standards mode, `* { box-sizing: border-box }`,
//! an 800x600 viewport, device scale factor 1, the bundled Inter registered
//! as `ProbeFace` through `@font-face`, `body { margin: 0; font: 16px/20px
//! ProbeFace }`), `getBoundingClientRect` in the viewport:
//! `mark=x,y,width,height`.
//!
//! The viewport rows (`m*`) were fixed by #1980/#2002 (commit 66241e0a8),
//! which landed after #2376 was filed: this file pins them. Rows `t*` sit
//! inside an ancestor with a `transform`, which is the fixed box's containing
//! block (CSS Transforms 1 §2), so its insets and margins are of that
//! ancestor's padding box (#1372; until then they were pinned here as known
//! gaps, placed against the viewport with no margins).

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);

const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho";

/// (name, html, Chrome 153). `{T}` is replaced by `TEXT`.
const CHROME: &[(&str, &str, &str)] = &[
    (
        "m01_issue_left_width",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:10px;top:0;width:100px;margin-left:23px">x</div></div>"##,
        "a=33,0,100,20",
    ),
    (
        "m02_issue_left_right",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:10px;right:10px;top:50px;margin:0 23px">x</div></div>"##,
        "a=33,50,734,20",
    ),
    (
        "m03_issue_auto_wrapping",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:10px;top:0;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=33,0,750,60",
    ),
    (
        "m04_right_width",
        r##"<div data-m="a" style="position:fixed;right:13px;top:7px;width:101px;margin-right:19px">x</div>"##,
        "a=667,7,101,20",
    ),
    (
        "m05_right_with_left_margin",
        r##"<div data-m="a" style="position:fixed;right:13px;top:7px;width:101px;margin-left:29px">x</div>"##,
        "a=686,7,101,20",
    ),
    (
        "m06_right_auto_wrapping",
        r##"<div data-m="a" style="position:fixed;right:13px;top:7px;margin:0 19px 0 31px">{T}</div>"##,
        "a=31,7,737,60",
    ),
    (
        "m07_static_x",
        r##"<div style="margin-left:37px"><div data-m="a" style="position:fixed;top:9px;width:101px;margin-left:23px">x</div></div>"##,
        "a=60,9,101,20",
    ),
    (
        "m08_static_x_auto_wrapping",
        r##"<div style="margin-left:37px"><div data-m="a" style="position:fixed;top:9px;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=60,9,723,60",
    ),
    (
        "m09_percent_margin",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:10px;top:3px;width:31%;margin-left:5.5%">x</div></div>"##,
        "a=54,3,248,20",
    ),
    (
        "m10_percent_margins_between_insets",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:10px;right:14px;top:3px;margin:0 3.5% 0 5.5%">x</div></div>"##,
        "a=54,3,704,20",
    ),
    (
        "m11_negative_margin",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:50px;top:3px;width:121px;margin-left:-17px">x</div></div>"##,
        "a=33,3,121,20",
    ),
    (
        "m12_negative_margins_between_insets",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:50px;right:60px;top:3px;margin:0 -13px 0 -17px">x</div></div>"##,
        "a=33,3,720,20",
    ),
    (
        "m13_auto_margins_centre",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:10px;right:14px;top:3px;width:203px;margin:0 auto">x</div></div>"##,
        "a=296.5,3,203,20",
    ),
    (
        "m14_auto_left_margin",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:10px;right:14px;top:3px;width:203px;margin-left:auto;margin-right:21px">x</div></div>"##,
        "a=562,3,203,20",
    ),
    (
        "m15_vertical_margin",
        r##"<div style="width:300px"><div data-m="a" style="position:fixed;left:10px;top:7px;width:101px;margin:11px 0 0 23px">x</div></div>"##,
        "a=33,18,101,20",
    ),
    (
        "t01_transform_left_width",
        r##"<div style="transform:translateX(0);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:10px;top:5px;width:100px;margin-left:23px">x</div></div>"##,
        "a=74,18,100,20",
    ),
    (
        "t02_transform_left_right",
        r##"<div style="transform:translateX(0);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:10px;right:12px;top:5px;margin:0 23px 0 19px">x</div></div>"##,
        "a=70,18,269,20",
    ),
    (
        "t03_transform_auto_wrapping",
        r##"<div style="transform:translateX(0);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:10px;top:5px;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=74,18,283,140",
    ),
    (
        "t04_transform_percent_margin",
        r##"<div style="transform:translateX(0);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:10px;top:5px;width:101px;margin-left:10.5%">x</div></div>"##,
        "a=85.95,18,101,20",
    ),
    (
        "t05_transform_negative_margin",
        r##"<div style="transform:translateX(0);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;left:40px;right:30px;top:5px;margin:0 -9px 0 -17px">x</div></div>"##,
        "a=64,18,289,20",
    ),
    (
        "t06_transform_right_width",
        r##"<div style="transform:translateX(0);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="a" style="position:fixed;right:13px;top:5px;width:101px;margin-right:19px">x</div></div>"##,
        "a=241,18,101,20",
    ),
];

fn rows() -> Vec<(&'static str, String, &'static str)> {
    CHROME
        .iter()
        .map(|(n, h, w)| (*n, h.replace("{T}", TEXT), *w))
        .collect()
}

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

fn lay_out(html: &str, viewport: (f32, f32)) -> RinchDocument {
    let mut doc = document();
    doc.load_css("* { box-sizing: border-box; }");
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; font: 16px/20px ProbeFace");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(viewport.0, viewport.1);
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

#[test]
fn fixed_margins_match_chrome_153() {
    let rows = rows();
    let bad: Vec<_> = rows
        .iter()
        .filter_map(|(n, h, want)| {
            let got = dump(&lay_out(h, VIEWPORT), want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(rows.len() >= 21, "the table is being read: {}", rows.len());
    assert!(bad.is_empty(), "{} of {}:\n{bad:#?}", bad.len(), rows.len());
}

/// Every row laid out at another viewport height first, then at the measured
/// one: an incremental layout lands where a fresh one does.
#[test]
fn a_relayout_lands_where_a_fresh_layout_does() {
    let bad: Vec<_> = rows()
        .iter()
        .filter_map(|(n, h, want)| {
            let mut doc = lay_out(h, (VIEWPORT.0, VIEWPORT.1 + 37.0));
            doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
            let got = dump(&doc, want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}
