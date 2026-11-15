//! #2377 — an absolutely positioned child of a grid container that is its
//! containing block, placed in a grid area, has **that area** as its
//! containing block (css-grid-1 §9.1): an `auto` width shrinks to fit in the
//! area less its insets and margins, not in the whole grid. A `grid-column`
//! of `auto` (or an `auto` line) takes the padding edge on that side, so an
//! unplaced box still has the whole padding box.
//!
//! Every number in `CHROME` is **Chrome 153**'s (Playwright's
//! `chrome-headless-shell`, standards mode, `* { box-sizing: border-box }`,
//! an 800x600 viewport, device scale factor 1, the bundled Inter registered
//! as `ProbeFace` through `@font-face`, `body { margin: 0; font: 16px/20px
//! ProbeFace }`), `getBoundingClientRect` in the viewport:
//! `mark=x,y,width,height`.
//!
//! The rows that place a box by a name (`g04`, `g16`, `g17`, `g18`) each
//! have a twin placed by line numbers; they were known gaps until rinch read
//! `grid-template-areas` and the `[names]` of a track list (#2704).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);

const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";

/// (name, html, Chrome 153). `{T}` is replaced by `TEXT`.
const CHROME: &[(&str, &str, &str)] = &[
    (
        "g01_issue",
        r##"<div style="position:relative;display:grid;grid-template-columns:100px 200px;width:300px;height:300px"><div data-m="a" style="position:absolute;grid-column:2;margin:0 17px 0 23px">alpha beta gamma delta epsilon zeta eta theta iota kappa</div></div>"##,
        "a=123,0,160,60",
    ),
    (
        "g02_column_margins",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=143,3,268,60",
    ),
    (
        "g03_span_two",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:1 / span 2;grid-row:2;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=39,80,277,60",
    ),
    (
        "g04_named_area",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid;grid-template-areas:&quot;a b c&quot; &quot;d e f&quot;"><div data-m="a" style="position:absolute;grid-area:e;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=143,80,173,120",
    ),
    (
        "g05_auto_whole_grid",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=26,3,385,40",
    ),
    (
        "g06_column_auto_explicit",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:auto;grid-row:2;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=26,80,385,40",
    ),
    (
        "g07_line_to_padding_edge",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2 / auto;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=143,3,268,60",
    ),
    (
        "g08_left_inset",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;left:5px;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=148,3,263,60",
    ),
    (
        "g09_both_insets",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;left:5px;right:9px;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=148,3,254,80",
    ),
    (
        "g10_right_inset_row_two",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;grid-row:2;right:6px;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=143,80,262,60",
    ),
    (
        "g11_percent_width",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;width:50.5%;margin-left:7px">x</div></div>"##,
        "a=127,3,155.53,20",
    ),
    (
        "g12_short_content",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:3;margin:0 3px 0 5px">ab</div></div>"##,
        "a=345,3,19,20",
    ),
    (
        "g13_auto_left_inset",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;left:7px;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=33,3,378,60",
    ),
    (
        "g14_justify_end",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;justify-self:end;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=143,3,268,60",
    ),
    (
        "g15_fit_content_left_inset",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;width:fit-content;left:5px;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=148,3,263,60",
    ),
    (
        "g16_named_area_span",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid;grid-template-areas:&quot;a b c&quot; &quot;d e f&quot;"><div data-m="a" style="position:absolute;grid-column:b / c;grid-row:e;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=143,80,251,80",
    ),
    (
        "g17_named_lines",
        r##"<div style="position:relative;display:grid;grid-template-columns:[s] 97px [m] 213px [e] 71px [f];grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:m / f;grid-row:2;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=143,80,251,80",
    ),
    (
        "g18_named_line_to_padding_edge",
        r##"<div style="position:relative;display:grid;grid-template-columns:[s] 97px [m] 213px [e] 71px [f];grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column-start:e;right:4px;margin:0 3px 0 5px">{T}</div></div>"##,
        "a=345,3,76,260",
    ),
    (
        "g19_line_to_padding_edge_right_inset",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column-start:3;right:4px;margin:0 3px 0 5px">{T}</div></div>"##,
        "a=345,3,76,260",
    ),
    (
        "g20_contents_wrapper",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div style="display:contents"><div data-m="a" style="position:absolute;grid-column:2;margin:0 17px 0 23px">{T}</div></div></div>"##,
        "a=143,3,268,60",
    ),
    (
        "g21_line_beyond_explicit_grid",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:6;grid-row:2;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=26,80,385,40",
    ),
    (
        "g22_negative_lines",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:-3 / -2;grid-row:-2;margin:0 17px 0 23px">{T}</div></div>"##,
        "a=143,80,173,120",
    ),
    (
        "g23_span_into_implicit_tracks",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:3 / span 3;margin:0 3px 0 5px">{T}</div></div>"##,
        "a=345,3,80,240",
    ),
    (
        "g24_height_stretch_one_inset",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;grid-row:2;top:6px;height:stretch;margin:4px 17px 9px 23px">x</div></div>"##,
        "a=143,90,9,157",
    ),
    (
        "g25_height_fit_content_one_inset",
        r##"<div style="position:relative;display:grid;grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid"><div data-m="a" style="position:absolute;grid-column:2;grid-row:2;bottom:6px;height:fit-content;margin:4px 17px 9px 23px">{T}</div></div>"##,
        "a=143,181,268,60",
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
fn grid_area_containing_blocks_match_chrome_153() {
    let rows = rows();
    let bad: Vec<_> = rows
        .iter()
        .filter_map(|(n, h, want)| {
            let got = dump(&lay_out(h, VIEWPORT), want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert_eq!(rows.len(), 25, "the table is being read");
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

/// A box sized in its area follows a change of the area's tracks on the next
/// layout: `grid-column: 1 / span 2` covers 97 + 7 + 213, then 97 + 7 + 150
/// once the second column narrows — 277 wide (Chrome 153's `g03`), then that
/// less its margins, 214.
#[test]
fn a_box_follows_its_area_when_a_track_changes() {
    let (_, html, want) = rows().into_iter().find(|r| r.0 == "g03_span_two").unwrap();
    let mut doc = lay_out(&html, VIEWPORT);
    assert!(close(want, &dump(&doc, want)), "{}", dump(&doc, want));
    let grid = NodeId(
        doc.tree
            .get(one(&doc, "[data-m=a]"))
            .unwrap()
            .parent
            .unwrap(),
    );
    let style = doc.get_attribute(grid, "style").unwrap();
    let narrower = style.replace("213px", "150px");
    assert_ne!(style, narrower);
    doc.set_attribute(grid, "style", &narrower);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    let id = one(&doc, "[data-m=a]");
    let (x, _) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let width = doc.tree.get(id).unwrap().layout.width;
    assert_eq!((x, width), (39.0, 214.0));
}
