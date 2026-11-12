//! #2596 — a table with a caption is as tall as a flex item, a grid item or
//! an absolutely positioned box as it is in flow: its rows, its spacing and
//! its captions, each once. rinch counted the captions twice there (the
//! table's grid already holds them as rows, and its parent added them on top
//! when it laid the table out at the height it had measured).
//!
//! Every number is Chrome 156's (`--headless=new`, 800x600,
//! `getBoundingClientRect` of `[data-m]` relative to its `[data-case]`
//! container). The cells hold fixed-size blocks and no text, so nothing here
//! depends on the host's fonts. A table's box is its wrapper's: the captions
//! are inside it.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const CSS: &str = "body { margin: 0; font: 16px/20px sans-serif } td { padding: 0 } \
                   table { border-spacing: 0 } .t { border-spacing: 3px; border: 2px solid }";

/// Two rows, 27 and 19 tall, 3px spacing and a 2px border: 72 tall with a
/// 13px caption, as in flow.
const ROWS: &str = "<tr><td data-m=r1><div style='width:20px;height:27px'></div></td></tr>\
                    <tr><td data-m=r2><div style='width:20px;height:19px'></div></td></tr>";

type Rect = (f32, f32, f32, f32);

fn lay_out(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `[data-m=m]`'s border box, relative to `[data-case]`'s.
fn rect(doc: &RinchDocument, m: &str) -> Rect {
    let one = |sel: &str| {
        let found = query_selector(&doc.tree, sel);
        assert_eq!(found.len(), 1, "{sel}");
        found[0]
    };
    let pos = |id| rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let id = one(&format!("[data-m={m}]"));
    let (cx, cy) = pos(one("[data-case]"));
    let (x, y) = pos(id);
    let l = doc.tree.get(id).unwrap().layout;
    ((x - cx) as f32, (y - cy) as f32, l.width, l.height)
}

fn table(table: &str, captions: &str) -> String {
    format!("<table data-m=t class=t {table}>{captions}{ROWS}</table>")
}

#[track_caller]
fn check(case: &str, container: &str, table_attrs: &str, captions: &str, chrome: &[(&str, Rect)]) {
    let inner = table(table_attrs, captions);
    check_html(
        case,
        &format!("<div data-case {container}>{inner}</div>"),
        chrome,
    );
}

#[track_caller]
fn check_html(case: &str, html: &str, chrome: &[(&str, Rect)]) {
    let doc = lay_out(html);
    let got: Vec<(&str, Rect)> = chrome.iter().map(|&(m, _)| (m, rect(&doc, m))).collect();
    // Chrome lays out in 1/64 px; rinch rounds a box to whole pixels.
    let near = |a: Rect, b: Rect| {
        (a.0 - b.0).abs() <= 0.5
            && (a.1 - b.1).abs() <= 0.5
            && (a.2 - b.2).abs() <= 0.5
            && (a.3 - b.3).abs() <= 0.5
    };
    let same = got.iter().zip(chrome).all(|(g, c)| near(g.1, c.1));
    assert!(same, "{case}: rinch {got:?}, Chrome {chrome:?}");
}

const CAPTION: &str = "<caption data-m=c style='height:13px'></caption>";

#[test]
fn in_flow_the_control_issue_2596() {
    check(
        "flow",
        "style='height:200px'",
        "",
        CAPTION,
        &[("t", (0.0, 0.0, 30.0, 72.0)), ("c", (0.0, 0.0, 30.0, 13.0))],
    );
}

#[test]
fn absolutely_positioned_issue_2596() {
    let pos = "style='position:relative;height:200px'";
    check(
        "top only",
        pos,
        "style='position:absolute;top:7px;left:0'",
        CAPTION,
        &[("t", (0.0, 7.0, 30.0, 72.0)), ("c", (0.0, 7.0, 30.0, 13.0))],
    );
    check(
        "top and bottom",
        pos,
        "style='position:absolute;top:7px;bottom:11px;left:0'",
        CAPTION,
        &[("t", (0.0, 7.0, 30.0, 72.0)), ("c", (0.0, 7.0, 30.0, 13.0))],
    );
    // Centred in the room the insets leave: 7 + (200 - 7 - 11 - 72) / 2.
    check(
        "top, bottom and auto margins",
        pos,
        "style='position:absolute;top:7px;bottom:11px;left:0;margin:auto 0'",
        CAPTION,
        &[
            ("t", (0.0, 62.0, 30.0, 72.0)),
            ("c", (0.0, 62.0, 30.0, 13.0)),
        ],
    );
    // A caption of content and margins: 5 + 11 + 3 above the table box.
    check(
        "a caption with content and margins",
        pos,
        "style='position:absolute;top:7px;left:0'",
        "<caption data-m=c style='margin:5px 0 3px'><div style='height:11px'></div></caption>",
        &[
            ("t", (0.0, 7.0, 30.0, 78.0)),
            ("c", (0.0, 12.0, 30.0, 11.0)),
        ],
    );
    // A table `height` is the table box's; the caption is outside it.
    check(
        "a table height",
        pos,
        "style='position:absolute;top:7px;left:0;height:90px'",
        CAPTION,
        &[
            ("t", (0.0, 7.0, 30.0, 103.0)),
            ("c", (0.0, 7.0, 30.0, 13.0)),
        ],
    );
}

#[test]
fn a_flex_item_issue_2596() {
    let row = "style='display:flex;height:200px;align-items:start'";
    check(
        "row, align-items: start",
        row,
        "",
        CAPTION,
        &[("t", (0.0, 0.0, 30.0, 72.0)), ("c", (0.0, 0.0, 30.0, 13.0))],
    );
    check(
        "row, a bottom caption",
        row,
        "",
        "<caption data-m=c style='height:13px;caption-side:bottom'></caption>",
        &[
            ("t", (0.0, 0.0, 30.0, 72.0)),
            ("c", (0.0, 59.0, 30.0, 13.0)),
        ],
    );
    check(
        "row, a caption on each side",
        row,
        "",
        "<caption data-m=c style='height:13px'></caption>\
         <caption data-m=d style='height:9px;caption-side:bottom'></caption>",
        &[
            ("t", (0.0, 0.0, 30.0, 81.0)),
            ("c", (0.0, 0.0, 30.0, 13.0)),
            ("d", (0.0, 72.0, 30.0, 9.0)),
        ],
    );
    check(
        "row, a table height",
        row,
        "style='height:90px'",
        CAPTION,
        &[
            ("t", (0.0, 0.0, 30.0, 103.0)),
            ("c", (0.0, 0.0, 30.0, 13.0)),
        ],
    );
    // Stretched: the table (captions included) fills the line.
    check(
        "row, stretched",
        "style='display:flex;height:200px'",
        "",
        CAPTION,
        &[
            ("t", (0.0, 0.0, 30.0, 200.0)),
            ("c", (0.0, 0.0, 30.0, 13.0)),
        ],
    );
    check(
        "column",
        "style='display:flex;flex-direction:column;height:200px'",
        "",
        CAPTION,
        &[
            ("t", (0.0, 0.0, 800.0, 72.0)),
            ("c", (0.0, 0.0, 800.0, 13.0)),
        ],
    );
}

#[test]
fn a_grid_item_issue_2596() {
    check(
        "grid, align-items: start",
        "style='display:grid;height:200px;align-items:start'",
        "",
        CAPTION,
        &[
            ("t", (0.0, 0.0, 800.0, 72.0)),
            ("c", (0.0, 0.0, 800.0, 13.0)),
        ],
    );
}

#[test]
fn in_a_shrink_to_fit_box_issue_2596() {
    let inner = table("", CAPTION);
    check_html(
        "inline-block",
        &format!(
            "<div data-case style='height:200px'><div style='display:inline-block'>{inner}</div></div>"
        ),
        &[("t", (0.0, 0.0, 30.0, 72.0)), ("c", (0.0, 0.0, 30.0, 13.0))],
    );
}

/// A wrapper's height, handed by the parent, holds the captions; the rows
/// share what is left of it by their heights (27 : 19 of 200 - 13 - 13).
const STRETCHED: [(&str, Rect); 4] = [
    ("t", (0.0, 0.0, 800.0, 200.0)),
    ("c", (0.0, 0.0, 800.0, 13.0)),
    ("r1", (5.0, 18.0, 790.0, 102.125)),
    ("r2", (5.0, 123.125, 790.0, 71.875)),
];

#[test]
fn a_stretched_or_flexed_table_shares_its_wrappers_height_issue_2596() {
    let column = "style='display:flex;flex-direction:column;height:200px'";
    check(
        "column, flex-grow",
        column,
        "style='flex-grow:1'",
        CAPTION,
        &STRETCHED,
    );
    // Its own `height` is a minimum the flexed size passes.
    check(
        "column, flex-grow and a height",
        column,
        "style='flex-grow:1;height:90px'",
        CAPTION,
        &STRETCHED,
    );
    check(
        "grid, stretched",
        "style='display:grid;height:200px'",
        "",
        CAPTION,
        &STRETCHED,
    );
    let narrow = STRETCHED
        .map(|(m, (x, y, _, h))| (m, (x, y, if m == "t" || m == "c" { 30.0 } else { 20.0 }, h)));
    check(
        "row, stretched",
        "style='display:flex;height:200px'",
        "",
        CAPTION,
        &narrow,
    );
}

#[test]
fn a_tables_own_height_leaves_the_captions_out_issue_2596() {
    // 90 - 13 of border, padding and spacing for the rows, 27 : 19.
    let own = |w: f32, wt: f32, y: f32| {
        [
            ("t", (0.0, y, wt, 103.0)),
            ("c", (0.0, y, wt, 13.0)),
            ("r1", (5.0, y + 18.0, w, 45.188)),
            ("r2", (5.0, y + 66.188, w, 31.813)),
        ]
    };
    check(
        "column, a height",
        "style='display:flex;flex-direction:column;height:200px'",
        "style='height:90px'",
        CAPTION,
        &own(790.0, 800.0, 0.0),
    );
    check(
        "absolute, a border-box height",
        "style='position:relative;height:200px'",
        "style='position:absolute;top:7px;height:90px;box-sizing:border-box'",
        CAPTION,
        &own(20.0, 30.0, 7.0),
    );
}
