//! #2359 — `vertical-align: baseline` in a table: a row's baseline-aligned
//! cells (and the anonymous cell of a `<td style="display: block">`, whose
//! `vertical-align` is the initial `baseline`) put their first lines on one
//! baseline, the lowest of theirs from the row's top (CSS 2.1 §17.5.3).
//! rinch aligned them as `top`.
//!
//! Every number is Chrome 156's (`--headless=new`, standards mode, the
//! bundled Inter registered as `ProbeFace` through `@font-face`, `16px/20px`
//! unless a cell says otherwise), `getBoundingClientRect().y` relative to the
//! table. A line's baseline is marked by a 4x10 inline-block standing on it,
//! so a mark 10px above it; a 16/20 line puts its baseline 16 below its top.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn lay_out(inner: &str) -> RinchDocument {
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
    doc.load_css(
        "body { margin: 0; font: 16px/20px ProbeFace } td { padding: 0 } \
         table { border-spacing: 0 } i { display: inline-block; width: 4px; height: 10px }",
    );
    let body = doc.body();
    doc.set_inner_html(body, inner);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn one(doc: &RinchDocument, m: &str) -> rinch_dom::node::RawNodeId {
    let found = query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1, "{m}");
    found[0]
}

/// `[data-m=m]`'s `y`, from the top of `[data-m=t]`.
fn y(doc: &RinchDocument, m: &str) -> f32 {
    let pos = |id| rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    (pos(one(doc, m)).1 - pos(one(doc, "t")).1) as f32
}

#[track_caller]
fn at(doc: &RinchDocument, chrome: &[(&str, f32)]) {
    let got: Vec<(&str, f32)> = chrome.iter().map(|&(m, _)| (m, y(doc, m))).collect();
    let close = got
        .iter()
        .zip(chrome)
        .all(|(g, c)| (g.1 - c.1).abs() <= 0.5);
    assert!(close, "rinch {got:?}, Chrome {chrome:?}");
}

#[test]
fn a_display_block_cell_lowers_to_a_baseline_cell_issue_2359() {
    // The issue's row: the block's text on the baseline of the other cell's,
    // below its 20px padding. Chrome: the block at 20 (rinch: 0).
    let d = lay_out(
        "<table data-m=t><tr style=\"height:60px\">\
         <td style=\"display:block\" data-m=a>x</td>\
         <td style=\"padding-top:20px;vertical-align:baseline\">x<i data-m=s></i></td>\
         </tr></table>",
    );
    at(&d, &[("a", 20.0), ("s", 26.0)]);
}

#[test]
fn two_baseline_cells_share_the_lower_baseline_issue_2359() {
    // Chrome: both marks at 26 (rinch: the first at 6).
    let d = lay_out(
        "<table data-m=t><tr style=\"height:60px\">\
         <td style=\"vertical-align:baseline\">x<i data-m=a></i></td>\
         <td style=\"padding-top:20px;vertical-align:baseline\">x<i data-m=b></i></td>\
         </tr></table>",
    );
    at(&d, &[("a", 26.0), ("b", 26.0)]);
}

#[test]
fn a_big_line_a_box_with_no_line_and_a_nested_line_issue_2359() {
    // A 32/40 line's baseline (31) is the row's: the 16/20 lines move down to
    // it, a block with no line puts its bottom (30) on it, and a line inside
    // a block counts the block's margin and padding (10 + 16, so 5 more).
    // The tall `middle` cell is not aligned by it. Chrome: marks at 21, the
    // block at 1.
    let d = lay_out(
        "<table data-m=t><tr>\
         <td style=\"vertical-align:baseline;font:32px/40px ProbeFace\">X<i data-m=a></i></td>\
         <td style=\"vertical-align:baseline\">x<i data-m=b></i></td>\
         <td style=\"vertical-align:baseline\"><div data-m=c style=\"height:30px;width:10px\"></div></td>\
         <td style=\"vertical-align:baseline\"><div style=\"margin-top:7px;padding-top:3px\">y<i data-m=d></i></div></td>\
         <td><div data-m=e style=\"height:70px;width:5px\"></div></td>\
         </tr></table>",
    );
    at(
        &d,
        &[
            ("a", 21.0),
            ("b", 21.0),
            ("c", 1.0),
            ("d", 21.0),
            ("e", 0.0),
        ],
    );
}

#[test]
fn a_rowspan_cell_aligns_in_its_first_row_issue_2359() {
    // `cellspacing=4`: the spanning cell's line on the 12px-padded cell's.
    // Chrome: both marks at 18.
    let d = lay_out(
        "<table data-m=t cellspacing=4><tr>\
         <td style=\"vertical-align:baseline\" rowspan=2>x<i data-m=a></i></td>\
         <td style=\"vertical-align:baseline;padding-top:12px\">x<i data-m=b></i></td></tr>\
         <tr><td>z</td></tr></table>",
    );
    at(&d, &[("a", 18.0), ("b", 18.0)]);
}

#[test]
fn a_block_cell_beside_middle_cells_stays_at_the_top_issue_2359() {
    // The positive control: no cell of the row is baseline-aligned (the UA's
    // `middle`), so the block's baseline is the row's. Chrome: 0, as before.
    let d = lay_out(
        "<table data-m=t><tr style=\"height:60px\">\
         <td style=\"display:block\" data-m=a>x</td>\
         <td style=\"padding-top:20px\">x</td></tr></table>",
    );
    at(&d, &[("a", 0.0)]);
}

#[test]
fn baselines_hold_across_layouts_and_a_restyle_issue_2359() {
    // Laid out again at another width, and after a colour change that runs
    // no compute, nothing moves twice.
    let mut d = lay_out(
        "<table data-m=t><tr style=\"height:60px\">\
         <td style=\"display:block\" data-m=a>x</td>\
         <td style=\"vertical-align:baseline\" data-m=c>x<i data-m=b></i></td>\
         <td style=\"padding-top:20px;vertical-align:baseline\">x<i data-m=s></i></td>\
         </tr></table>",
    );
    at(&d, &[("a", 20.0), ("b", 26.0), ("s", 26.0)]);
    d.resolve_layout(700.0, 600.0);
    at(&d, &[("a", 20.0), ("b", 26.0), ("s", 26.0)]);
    let c = rinch_core::dom::NodeId(one(&d, "c"));
    d.set_attribute(c, "style", "vertical-align:baseline;color:red");
    d.resolve_layout(700.0, 600.0);
    at(&d, &[("a", 20.0), ("b", 26.0), ("s", 26.0)]);
    let a = rinch_core::dom::NodeId(one(&d, "a"));
    d.set_attribute(a, "style", "display:block;color:red");
    d.resolve_layout(600.0, 600.0);
    at(&d, &[("a", 20.0), ("b", 26.0), ("s", 26.0)]);
}
