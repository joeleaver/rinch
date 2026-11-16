//! #2716 — `grid-auto-columns` and `grid-auto-rows` size a grid's implicit
//! tracks, and the explicit tracks no track list sizes (those only
//! `grid-template-areas` creates): lengths, `fr`, percentages, `minmax()`,
//! `fit-content()`, `auto` and the content keywords, several values cycling
//! (forwards after the explicit grid, backwards before it), with
//! `grid-auto-flow: column` and `dense`, and an implicit track a named line
//! creates (#2717). A table ignores both.
//!
//! Every number in `CHROME` is **Chrome 153**'s (Playwright's
//! `chrome-headless-shell` 153.0.8010.12, standards mode,
//! `* { box-sizing: border-box }`, an 800x600 viewport, device scale factor
//! 1, the bundled Inter registered as `ProbeFace` through `@font-face`,
//! `body { margin: 0; font: 16px/20px ProbeFace }`), `getBoundingClientRect`
//! in the viewport: `mark=x,y,width,height`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The grid every row shares, as in `grid_named_lines_2704_tests.rs`: unequal
/// gaps, padding and a border, so a track misplaced by one gap or the padding
/// edge moves a box by a distinct amount.
const G: &str = "display:grid;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid";

/// (name, grid style, items as `mark:style` joined by `|`, an item's text
/// after `#`, Chrome 153).
const CHROME: &[(&str, &str, &str, &str)] = &[
    (
        "a01_auto_lengths",
        "grid-auto-columns:53px;grid-auto-rows:37px",
        "a:grid-column:3;grid-row:2",
        "a=136,56,53,37",
    ),
    (
        "a02_auto_sizes_area_tracks",
        "grid-auto-columns:53px;grid-auto-rows:37px;grid-template-areas:'a b c' 'd e f'",
        "a:grid-area:f|b:",
        "a=136,56,53,37 b=16,14,53,37",
    ),
    (
        "a03_implicit_after_explicit",
        "grid-template-columns:97px 71px;grid-template-rows:61px;grid-auto-columns:41px;grid-auto-rows:29px",
        "a:grid-column:4;grid-row:1|b:grid-column:5;grid-row:3|c:grid-column:1;grid-row:2",
        "a=246,14,41,61 b=294,114,41,29 c=16,80,97,29",
    ),
    (
        "a04_fr_cycle",
        "grid-template-columns:97px;grid-auto-columns:1fr 2fr;grid-template-rows:61px",
        "a:grid-column:2|b:grid-column:3|c:grid-column:4|d:grid-column:5;grid-row:2",
        "a=120,14,45.66,61 b=172.66,14,91.34,61 c=271,14,45.66,61 d=323.66,80,91.34,165",
    ),
    (
        "a05_minmax",
        "grid-template-columns:97px 213px;grid-auto-rows:minmax(23px, auto) 31px",
        "a:grid-row:1|b:grid-row:2;height:47px|c:grid-row:3|d:grid-row:4",
        "a=16,14,97,77 b=16,96,97,47 c=16,132,97,77 d=16,214,97,31",
    ),
    (
        "a06_auto_and_length_cycle",
        "grid-template-columns:97px 213px;grid-auto-rows:auto 29px",
        "a:grid-row:1|b:grid-row:2|c:grid-row:3|d:grid-row:4;height:13px",
        "a=16,14,97,79 b=16,98,97,29 c=16,132,97,79 d=16,216,97,13",
    ),
    (
        "a07_cycle_before_grid",
        "grid-template-columns:97px 71px;grid-auto-columns:13px 19px 23px;grid-template-rows:61px",
        "a:grid-column:-4|b:grid-column:-6 / -5|c:grid-column:1|d:grid-column:3|e:grid-column:5",
        "a=62,14,23,61 b=16,80,13,165 c=92,80,97,165 d=274,80,13,165 e=320,80,23,165",
    ),
    (
        "a08_column_flow",
        "grid-template-rows:61px 149px;grid-auto-flow:column;grid-auto-columns:43px 31px",
        "a:|b:|c:|d:|e:",
        "a=16,14,43,61 b=16,80,43,149 c=66,14,31,61 d=66,80,31,149 e=104,14,43,61",
    ),
    (
        "a09_row_dense",
        "grid-template-columns:97px 213px 71px;grid-auto-flow:row dense;grid-auto-rows:29px 17px",
        "a:grid-column:span 2|b:grid-column:span 2|c:|d:|e:grid-column:span 3",
        "a=16,14,317,29 b=16,48,317,17 c=340,14,71,29 d=340,48,71,17 e=16,70,395,29",
    ),
    (
        "a10_column_dense",
        "grid-template-rows:61px 89px 37px;grid-auto-flow:column dense;grid-auto-columns:43px 31px",
        "a:grid-row:span 2|b:grid-row:span 2|c:|d:|e:grid-row:span 3",
        "a=16,14,43,155 b=66,14,31,155 c=16,174,43,37 d=66,174,31,37 e=104,14,43,197",
    ),
    (
        "a11_percent",
        "grid-template-columns:97px;grid-auto-columns:13%;grid-auto-rows:8%",
        "a:grid-column:2;grid-row:2|b:grid-column:3;grid-row:3",
        "a=120,37.47,51.86,18.47 b=178.86,60.94,51.86,18.47",
    ),
    (
        "a12_fit_content",
        "grid-template-columns:97px;grid-auto-columns:fit-content(37px);grid-template-rows:61px",
        "a:grid-column:2;width:23px|b:grid-column:3;width:59px|c:grid-column:4",
        "a=120,14,23,61 b=150,14,59,61 c=216,14,0,61",
    ),
    (
        "a13_content_keywords",
        "grid-template-columns:97px;grid-auto-columns:min-content max-content;grid-template-rows:61px",
        "a:grid-column:2;font:16px/20px ProbeFace#alpha beta gamma|b:grid-column:3;font:16px/20px ProbeFace#alpha beta",
        "a=120,14,56,61 b=183,14,79,61",
    ),
    (
        "a14_areas_and_implicit",
        "grid-template-areas:'a b' 'c d';grid-template-columns:97px;grid-auto-columns:59px;grid-auto-rows:41px",
        "a:grid-area:d|b:grid-column:3;grid-row:3",
        "a=120,60,59,41 b=186,106,59,41",
    ),
    (
        "a15_name_creates_implicit_sized",
        "grid-template-columns:[a] 97px [b] 213px [a] 71px [b];grid-template-rows:61px 149px;grid-auto-columns:19px",
        "c:grid-column:a -3;grid-row:2|d:grid-column:1;grid-row:1",
        "c=16,80,19,149 d=42,14,97,61",
    ),
];

/// The row's markup: the grid and its items, as the Chrome page built it.
fn html(grid: &str, items: &str) -> String {
    let kids: String = items
        .split('|')
        .map(|item| {
            let (m, rest) = item.split_once(':').unwrap();
            let (style, text) = rest.split_once('#').unwrap_or((rest, ""));
            format!(r#"<div data-m="{m}" style="{style}">{text}</div>"#)
        })
        .collect();
    let grid = grid.replace('\'', "&quot;");
    format!(r#"<div style="{G};{grid}">{kids}</div>"#)
}

fn rows() -> Vec<(&'static str, String, &'static str)> {
    CHROME
        .iter()
        .map(|(n, g, i, w)| (*n, html(g, i), *w))
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

#[test]
fn implicit_track_sizes_match_chrome_153() {
    let rows = rows();
    let bad: Vec<_> = rows
        .iter()
        .filter_map(|(n, h, want)| {
            let got = dump(&lay_out(h, VIEWPORT), want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert_eq!(rows.len(), 15, "the table is being read");
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

/// Changing only `grid-auto-columns` re-sizes the implicit track on the next
/// layout: the value reaches Taffy's style, and a style that differs in
/// nothing else is still a layout change.
#[test]
fn changing_only_the_auto_track_size_resizes_the_track() {
    let (_, h, want) = rows()
        .into_iter()
        .find(|r| r.0 == "a01_auto_lengths")
        .unwrap();
    let mut doc = lay_out(&h, VIEWPORT);
    assert!(close(want, &dump(&doc, want)), "{}", dump(&doc, want));
    let a = one(&doc, "[data-m=a]");
    let grid = NodeId(doc.tree.get(a).unwrap().parent.unwrap());
    let style = doc.get_attribute(grid, "style").unwrap();
    let moved = style.replace("grid-auto-columns:53px", "grid-auto-columns:29px");
    assert_ne!(style, moved, "the style is rewritten: {style}");
    doc.set_attribute(grid, "style", &moved);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    // Chrome 153: two 29px columns and their gaps before it.
    let want = "a=88,56,29,37";
    assert!(close(want, &dump(&doc, want)), "{}", dump(&doc, want));
}

/// A table is laid out as a grid whose rows past the declared heights are
/// implicit, but Chrome ignores `grid-auto-rows`/`-columns` on a table: the
/// table and its cells are where they are without them (Chrome 153: the table
/// 130x68, the second row's cell at 10,39, 53x19).
#[test]
fn a_table_ignores_grid_auto_tracks() {
    let row = r#"<tr><td data-m="{m}" style="padding:0;width:53px;height:19px"></td><td style="padding:0;width:47px;height:19px"></td></tr>"#;
    for table in ["", "grid-auto-rows:83px;grid-auto-columns:211px"] {
        let html = format!(
            r#"<table data-m="t" style="border-spacing:10px;{table}">{}{}</table>"#,
            row.replace("{m}", "c"),
            row.replace("{m}", "d"),
        );
        let doc = lay_out(&html, VIEWPORT);
        let want = "t=0,0,130,68 c=10,10,53,19 d=10,39,53,19";
        assert!(
            close(want, &dump(&doc, want)),
            "{table}: {}",
            dump(&doc, want)
        );
    }
}
