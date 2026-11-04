//! #2141 — a table with a width of its own shares that width between its
//! `auto` columns in proportion to each column's max-content width
//! (`RinchDocument::measure_table_columns`). That width is now the widest of
//! the column's cells' own, each asked size-only and only again once the cell
//! changed, where a detached layout of the whole table used to ask every row
//! on every edit.
//!
//! So every fixture is a **history** compared with a **fresh layout** of the
//! state it ends in: the remembered widths must never give an answer a fresh
//! layout would not. Each first checks that the step moved a column (the
//! positive control). No number here is a glyph measurement: each assertion
//! compares two layouts of the same document on the same host.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;

const VP: (f32, f32) = (800.0, 600.0);
const CSS: &str = "body { margin: 0; font-size: 16px; line-height: 20px }
    table { width: 600px; border-spacing: 0 } td { padding: 0 }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    settle(&mut d);
    d
}

fn settle(d: &mut RinchDocument) {
    d.resolve_layout(VP.0, VP.1);
    d.resolve_layout(VP.0, VP.1);
}

/// Every cell's box, in document order.
fn cells(d: &RinchDocument) -> Vec<(f32, f32, f32, f32)> {
    d.query_selector_all("td")
        .into_iter()
        .map(|c| {
            let l = d.tree.nodes[c.0].layout;
            (l.x, l.y, l.width, l.height)
        })
        .collect()
}

fn rows(texts: &[(&str, &str)]) -> String {
    let mut h = String::from("<table><tbody>");
    for (i, (a, b)) in texts.iter().enumerate() {
        h.push_str(&format!("<tr id=r{i}><td id=a{i}>{a}</td><td id=b{i}>{b}</td></tr>"));
    }
    h.push_str("</tbody></table>");
    h
}

fn set_text(d: &mut RinchDocument, sel: &str, text: &str) {
    let n = d.query_selector(sel).unwrap();
    let t = d.tree.nodes[n.0].children[0];
    d.set_text_content(NodeId(t), text);
    settle(d);
}

const SHORT: &str = "ab";
const LONG: &str = "a much longer text that wants a wider column";

/// A cell's text grows and shrinks back: its column's share follows both
/// ways. A mutant that never asks a cell again keeps the first share.
#[test]
fn a_cell_that_grows_and_shrinks_moves_its_column() {
    let start = [(SHORT, "x"), ("cd", "y"), ("ef", "z")];
    let mut d = doc(&rows(&start));
    let before = cells(&d);

    set_text(&mut d, "#a1", LONG);
    let fresh = doc(&rows(&[(SHORT, "x"), (LONG, "y"), ("ef", "z")]));
    assert_ne!(cells(&fresh), before, "positive control: the long text widens column a");
    assert_eq!(cells(&d), cells(&fresh), "grown");

    set_text(&mut d, "#a1", "cd");
    assert_eq!(cells(&d), before, "shrunk back");
}

/// The row holding a column's widest cell goes: the column narrows to its
/// next widest. A mutant that takes the widest of only the cells it asked
/// this layout keeps the removed row's width.
#[test]
fn removing_the_widest_cell_narrows_its_column() {
    let mut d = doc(&rows(&[(SHORT, "x"), (LONG, "y"), ("ef", "z")]));
    let r1 = d.query_selector("#r1").unwrap();
    d.remove_child(d.tree.nodes[r1.0].parent.map(NodeId).unwrap(), r1);
    settle(&mut d);
    let fresh = doc(&rows(&[(SHORT, "x"), ("ef", "z")]));
    let with = doc(&rows(&[(SHORT, "x"), (LONG, "y"), ("ef", "z")]));
    assert_ne!(cells(&fresh)[0], cells(&with)[0], "positive control");
    assert_eq!(cells(&d), cells(&fresh));
}

/// An edit to a cell that is not its column's widest leaves the shares as
/// they were, and an edit to another column's cell is seen in that column.
#[test]
fn an_edit_in_the_other_column_is_seen_there() {
    let mut d = doc(&rows(&[(LONG, "x"), (SHORT, "y")]));
    set_text(&mut d, "#b1", LONG);
    let fresh = doc(&rows(&[(LONG, "x"), (SHORT, LONG)]));
    assert_ne!(
        cells(&fresh),
        cells(&doc(&rows(&[(LONG, "x"), (SHORT, "y")]))),
        "positive control"
    );
    assert_eq!(cells(&d), cells(&fresh));
}

/// A spanning cell shares its width between columns: such a table is still
/// measured by the detached layout, and an edit there matches a fresh one.
#[test]
fn a_table_with_a_spanning_cell_matches_a_fresh_layout() {
    let html = |t: &str| {
        format!(
            "<table><tr><td id=s colspan=2>{t}</td></tr>\
             <tr><td>{SHORT}</td><td>cd</td></tr></table>"
        )
    };
    let mut d = doc(&html(SHORT));
    set_text(&mut d, "#s", LONG);
    assert_eq!(cells(&d), cells(&doc(&html(LONG))));
}

/// The widths taken from the cells' own are the detached layout's, on tables
/// of every shape the cell path takes (and two it leaves to the detached
/// layout, which must agree with themselves): each built fresh both ways.
#[test]
fn cell_widths_give_the_detached_layouts_columns() {
    let shapes = [
        rows(&[(SHORT, "x"), (LONG, "y"), ("ef", LONG)]),
        // A block, an inline-block and a percentage-padded cell.
        format!(
            "<table><tr><td><div style='width:120px;height:5px'></div>{LONG}</td>\
             <td><span style='display:inline-block;width:40px;height:5px'></span> {SHORT}</td>\
             <td style='padding-left:5%'>{LONG} {LONG}</td></tr></table>"
        ),
        // A rowspan, which shares no width.
        format!(
            "<table><tr><td rowspan=2>{LONG}</td><td>{SHORT}</td></tr>\
             <tr><td>{LONG}</td></tr></table>"
        ),
        // A cell with a declared width, and a nested table.
        format!(
            "<table><tr><td style='width:100px'>{SHORT}</td>\
             <td><table style='width:auto'><tr><td>{LONG}</td></tr></table></td></tr></table>"
        ),
        // Left to the detached layout: a spanning cell, a caption.
        format!(
            "<table><tr><td colspan=2>{LONG} {LONG}</td></tr>\
             <tr><td>{SHORT}</td><td>cd</td></tr></table>"
        ),
        format!("<table><caption>{LONG} {LONG} {LONG}</caption><tr><td>{SHORT}</td><td>cd</td></tr></table>"),
        // Text straight in the table: a grid item that is no cell.
        format!("<table><tr><td>{SHORT}</td><td>cd</td></tr>{LONG} {LONG}</table>"),
    ];
    for (i, html) in shapes.into_iter().enumerate() {
        let mut fast = RinchDocument::new();
        fast.load_css(CSS);
        let body = fast.body();
        fast.set_inner_html(body, &html);
        fast.resolve_layout(VP.0, VP.1);
        let asked = fast.tree.perf.end_frame().get(Counter::TableColumnMeasures);
        // Positive control: the first four take the cell path, the rest
        // the detached layout.
        assert_eq!(asked > 0, i < 4, "shape {i}: cells asked {asked}");
        settle(&mut fast);
        let mut slow = RinchDocument::new();
        slow.tree.table_columns_by_detached_layout = true;
        slow.load_css(CSS);
        let body = slow.body();
        slow.set_inner_html(body, &html);
        settle(&mut slow);
        assert_eq!(cells(&fast), cells(&slow), "{html}");
    }
}
