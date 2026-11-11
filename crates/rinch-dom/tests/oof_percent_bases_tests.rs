//! Out-of-flow boxes resolved against the wrong base (#1980, #2002, #1986).
//!
//! - **#1980, #2002** — a `position: fixed` box whose containing block is the
//!   viewport took none of the margin rules an absolute box against the
//!   initial containing block takes: its margins (plain, percentage or
//!   `calc()`) were not added to its position, took nothing out of the room
//!   between two insets, and `margin: auto` did not centre it; its `calc()`
//!   sizes, margins and padding resolved against its Taffy parent; and the
//!   read-back stretched an `auto` width between two insets to the whole
//!   room after Taffy had applied `max-width`. The viewport is the same box
//!   the initial containing block is, so a fixed box now takes the same rules.
//! - **#1986** — the static position of an absolute (or fixed) child of a
//!   grid container that is **not** its containing block is aligned in the
//!   grid's content box (css-grid-1 §9.4: "as if it were the sole grid item
//!   in a grid area whose edges coincide with the content edges of the grid
//!   container"), whatever its grid placement. Taffy aligns it in the padding
//!   box (or its grid area), which is right only when the grid *is* the
//!   containing block.
//!
//! Every number in `CHROME` is **Chrome 157**'s (`--headless=new`, standards
//! mode, `* { box-sizing: border-box }`, an 800x600 viewport, `body { margin:
//! 0 }`), `getBoundingClientRect` in the viewport: `mark=x,y,width,height`. No
//! row holds text.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// (name, html, Chrome 157). The `fx` rows put the fixed box in an
/// unpositioned 406px block with an auto height (`padding: 5px 7px; border:
/// 3px`), so a basis taken from the Taffy parent is told from the viewport.
const CHROME: &[(&str, &str, &str)] = &[
    (
        "fx01_calc_size",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:10px;top:10px;width:calc(25% + 0px);height:calc(50% + 2px)"></div></div>"##,
        "a=10,10,200,302",
    ),
    (
        "fx02_calc_margin",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:10px;top:10px;width:50px;height:20px;margin-left:calc(10% + 1px);margin-top:calc(5% + 1px)"></div></div>"##,
        "a=91,51,50,20",
    ),
    (
        "fx03_pct_margin",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:10px;top:10px;width:50px;height:20px;margin-left:10%;margin-top:5%"></div></div>"##,
        "a=90,50,50,20",
    ),
    (
        "fx04_px_margin",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:10px;top:10px;width:50px;height:20px;margin:7px 0 0 5px"></div></div>"##,
        "a=15,17,50,20",
    ),
    (
        "fx05_end_margin",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;right:10px;bottom:10px;width:50px;height:20px;margin:0 5px 7px 0"></div></div>"##,
        "a=735,563,50,20",
    ),
    (
        "fx06_pct_padding",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:0;top:0;width:200px;height:100px;padding-left:10%;padding-top:calc(5% + 1px)"><div data-m="c" style="height:10px"></div></div></div>"##,
        "a=0,0,200,100 c=80,41,120,10",
    ),
    (
        "fx07_calc_max_width",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:0;right:0;top:0;height:10px;max-width:calc(50% + 0px)"></div></div>"##,
        "a=0,0,400,10",
    ),
    (
        "fx08_calc_min_height",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:0;top:0;width:10px;min-height:calc(50% + 0px)"></div></div>"##,
        "a=0,0,10,300",
    ),
    (
        "fx09_paired_x_margins",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:10px;right:10px;top:0;height:10px;margin:0 20px"></div></div>"##,
        "a=30,0,740,10",
    ),
    (
        "fx10_paired_y_margins",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;top:10px;bottom:10px;left:0;width:10px;margin:20px 0"></div></div>"##,
        "a=0,30,10,540",
    ),
    (
        "fx11_issue_bar",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:0;right:0;top:20px;margin:0 auto;max-width:300px;height:20px"></div></div>"##,
        "a=250,20,300,20",
    ),
    (
        "fx12_issue_dialog",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;inset:0;margin:auto;width:200px;height:100px"></div></div>"##,
        "a=300,250,200,100",
    ),
    (
        "fx13_one_auto_margin",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:0;right:0;top:0;width:200px;height:10px;margin-left:auto"></div></div>"##,
        "a=600,0,200,10",
    ),
    (
        "fx14_inline_overflow",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:0;right:0;top:0;width:1000px;height:10px;margin:0 auto"></div></div>"##,
        "a=0,0,1000,10",
    ),
    (
        "fx15_block_overflow",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;top:0;bottom:0;left:0;width:10px;height:700px;margin:auto 0"></div></div>"##,
        "a=0,-50,10,700",
    ),
    (
        "fx16_static_px_margin",
        r##"<div style="margin:30px 0 0 100px;width:300px"><div data-m="a" style="position:fixed;width:50px;height:20px;margin:5px 0 0 10px"></div></div>"##,
        "a=110,35,50,20",
    ),
    (
        "fx17_static_pct_margin",
        r##"<div style="margin:30px 0 0 100px;width:300px"><div data-m="a" style="position:fixed;width:50px;height:20px;margin:5% 0 0 10%"></div></div>"##,
        "a=180,70,50,20",
    ),
    (
        "fx18_calc_inset",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;top:calc(50% + 4px);left:calc(10% + 1px);width:10px;height:10px"></div></div>"##,
        "a=81,304,10,10",
    ),
    (
        "fx19_pct_max_width_paired",
        r##"<div style="width:406px;padding:5px 7px;border:3px solid;margin:13px 0 0 17px"><div data-m="a" style="position:fixed;left:0;right:0;top:0;height:10px;max-width:25%;margin:0 auto"></div></div>"##,
        "a=300,0,200,10",
    ),
    (
        "g01_issue_calc_insets",
        r##"<div data-m="g" style="position:relative;display:grid;grid-template-columns:100px 200px;grid-template-rows:30px 40px;width:300px"><div data-m="a" style="position:absolute;grid-column:2;grid-row:2;top:calc(50% + 1px);left:calc(10% + 2px);height:5px;width:5px"></div></div>"##,
        "g=0,0,300,70 a=122,51,5,5",
    ),
    (
        "g02_calc_sizes",
        r##"<div data-m="g" style="position:relative;display:grid;grid-template-columns:100px 200px;grid-template-rows:30px 40px;width:300px"><div data-m="a" style="position:absolute;grid-column:2;grid-row:2;left:0;top:0;width:calc(50% + 1px);height:calc(50% + 1px)"></div></div>"##,
        "g=0,0,300,70 a=100,30,101,21",
    ),
    (
        "g03_calc_end_insets",
        r##"<div data-m="g" style="position:relative;display:grid;grid-template-columns:100px 200px;grid-template-rows:30px 40px;width:308px;padding:4px"><div data-m="a" style="position:absolute;grid-column:1;grid-row:1;right:calc(10% + 1px);bottom:calc(20% + 2px);width:5px;height:5px"></div></div>"##,
        "g=0,0,308,78 a=271.61,56.2,5,5",
    ),
    (
        "s01_issue_center",
        r##"<div data-m="c" style="position:relative;width:420px;height:120px;padding:3px 0 0 5px"><div style="width:400px;height:80px;padding:7px 0 0 11px;margin:6px 0 0 4px;display:grid;grid-template-columns:100px 150px;justify-items:center"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px"></div><div style="width:50px;height:20px"></div></div></div>"##,
        "c=0,0,420,120 a=194.5,16,40,30",
    ),
    (
        "s02_start",
        r##"<div data-m="c" style="position:relative;width:420px;height:120px;padding:3px 0 0 5px"><div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;grid-template-columns:100px 150px"><div style="width:50px;height:20px"></div><div data-m="a" style="position:absolute;width:40px;height:30px"></div></div></div>"##,
        "c=0,0,420,120 a=20,16,40,30",
    ),
    (
        "s03_end_end",
        r##"<div data-m="c" style="position:relative;width:420px;height:120px;padding:3px 0 0 5px"><div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;justify-items:end;align-items:end"><div data-m="a" style="position:absolute;width:40px;height:30px"></div></div></div>"##,
        "c=0,0,420,120 a=360,54,40,30",
    ),
    (
        "s04_self_center",
        r##"<div data-m="c" style="position:relative;width:420px;height:120px;padding:3px 0 0 5px"><div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;border:2px solid"><div data-m="a" style="position:absolute;width:40px;height:30px;justify-self:center;align-self:center;margin-left:6px"></div></div></div>"##,
        "c=0,0,420,120 a=193,35,40,30",
    ),
    (
        "s05_placed",
        r##"<div data-m="c" style="position:relative;width:420px;height:120px;padding:3px 0 0 5px"><div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;grid-template-columns:100px 150px;grid-template-rows:20px 30px"><div data-m="a" style="position:absolute;width:40px;height:30px;grid-column:2;grid-row:2"></div></div></div>"##,
        "c=0,0,420,120 a=20,16,40,30",
    ),
    (
        "s06_fixed_in_grid",
        r##"<div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;justify-items:center;align-items:end"><div data-m="a" style="position:fixed;width:40px;height:30px"></div></div>"##,
        "a=185,51,40,30",
    ),
    (
        "s07_icb_in_grid",
        r##"<div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;justify-items:center;align-items:center"><div data-m="a" style="position:absolute;width:40px;height:30px"></div></div>"##,
        "a=185,32,40,30",
    ),
    (
        "s08_fit_start",
        r##"<div data-m="c" style="position:relative;width:450px;height:120px;padding:3px 0 0 5px"><div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;"><div data-m="a" style="position:absolute;font-size:0;line-height:0"><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div></div></div></div>"##,
        "c=0,0,450,120 a=20,16,430,20",
    ),
    (
        "s09_fit_end",
        r##"<div data-m="c" style="position:relative;width:450px;height:120px;padding:3px 0 0 5px"><div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;justify-items:end"><div data-m="a" style="position:absolute;font-size:0;line-height:0"><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div></div></div></div>"##,
        "c=0,0,450,120 a=0,16,400,20",
    ),
    (
        "s10_fit_center",
        r##"<div data-m="c" style="position:relative;width:450px;height:120px;padding:3px 0 0 5px"><div style="width:400px;height:80px;padding:7px 9px 5px 11px;margin:6px 0 0 4px;display:grid;justify-items:center"><div data-m="a" style="position:absolute;font-size:0;line-height:0"><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:150px;height:10px"></div></div></div></div>"##,
        "c=0,0,450,120 a=0,16,420,20",
    ),
];

/// (name, rinch's answer, why) — rows of `CHROME` that are not Chrome's yet,
/// each for a reason that is not one of these issues'. Pinned so that a fix
/// has to come here and take the row out.
const KNOWN: &[(&str, &str, &str)] = &[
    (
        "g01_issue_calc_insets",
        "g=0,0,300,70 a=132,66,5,5",
        "#1981: a mixed calc() on a grid-placed absolute child resolves against the grid container's padding box, not its grid area, which only Taffy computes",
    ),
    (
        "g02_calc_sizes",
        "g=0,0,300,70 a=100,30,151,36",
        "#1981: a mixed calc() on a grid-placed absolute child resolves against the grid container's padding box, not its grid area, which only Taffy computes",
    ),
    (
        "g03_calc_end_insets",
        "g=0,0,308,78 a=271,55,5,5",
        "#1981: a mixed calc() on a grid-placed absolute child resolves against the grid container's padding box, not its grid area, which only Taffy computes",
    ),
];

fn lay_out(html: &str, viewport: (f32, f32)) -> RinchDocument {
    let mut doc = RinchDocument::new();
    // The Chrome numbers were measured under this rule (file docs).
    doc.load_css("* { box-sizing: border-box; }");
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
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

/// Whether two dumps agree to within the pixel grid (rinch puts box edges on
/// whole pixels; Chrome reports the unrounded box).
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
fn out_of_flow_bases_match_chrome_157() {
    let mut rows = 0;
    let bad: Vec<_> = CHROME
        .iter()
        .filter(|(n, ..)| known(n).is_none())
        .filter_map(|(n, h, want)| {
            rows += 1;
            let got = dump(&lay_out(h, VIEWPORT), want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(rows >= 20, "the table is being read: {rows} rows");
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
            let got = dump(&lay_out(h, VIEWPORT), chrome);
            assert!(!close(chrome, now), "{n} is a gap: {why}");
            (!close(now, &got)).then(|| format!("{n} ({why}): pinned[{now}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A fixed box's `auto` margins and its percentage margins follow the
/// viewport across a resize: laid out at 800x600, then at 1000x700, each
/// answer is the one a fresh layout at that size gives (by CSS 2.1 §10.3.7
/// and §10.6.4 against the new viewport).
#[test]
fn fixed_margins_follow_a_viewport_resize() {
    let cases = [
        ("fx12_issue_dialog", "a=400,300,200,100"),
        ("fx11_issue_bar", "a=350,20,300,20"),
        ("fx03_pct_margin", "a=110,60,50,20"),
        ("fx09_paired_x_margins", "a=30,0,940,10"),
    ];
    for (name, at_1000) in cases {
        let (_, html, at_800) = CHROME.iter().find(|c| c.0 == name).unwrap();
        let mut doc = lay_out(html, VIEWPORT);
        assert!(close(at_800, &dump(&doc, at_800)), "{name} at 800x600");
        doc.resolve_layout(1000.0, 700.0);
        let got = dump(&doc, at_1000);
        assert!(
            close(at_1000, &got),
            "{name} at 1000x700: want[{at_1000}] got[{got}]"
        );
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let back = dump(&doc, at_800);
        assert!(close(at_800, &back), "{name} back at 800x600: got[{back}]");
    }
}
