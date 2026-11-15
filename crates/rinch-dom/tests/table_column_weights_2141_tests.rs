//! #2141 — a table with a width of its own shares that width between its
//! `auto` columns by their max-content widths. Those were measured apart
//! from the layout and remembered per cell (#2141), then read by nothing
//! after #2306, and the measure was deleted (#2327): Taffy's own track
//! sizing gives the columns, from the cells' cached contributions.
//!
//! So every fixture is a **history** compared with a **fresh layout** of the
//! state it ends in: what an incremental layout remembers must never give an
//! answer a fresh layout would not. Each first checks that the step moved a column (the
//! positive control). No number here is a glyph measurement: each assertion
//! compares two layouts of the same document on the same host.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

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
        h.push_str(&format!(
            "<tr id=r{i}><td id=a{i}>{a}</td><td id=b{i}>{b}</td></tr>"
        ));
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
    assert_ne!(
        cells(&fresh),
        before,
        "positive control: the long text widens column a"
    );
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

/// A spanning cell shares its width between columns: an edit there matches a
/// fresh layout.
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

/// A cell changes while its table has no width of its own (or a fixed
/// layout), when no column is measured, and the width comes back: the
/// column must be measured from the cell as it is now. The remembered width
/// used to be dropped only when Taffy still called the cell dirty, which the
/// computes of the layouts in between had cleared (review of #2153: 407/193
/// where a fresh layout gives 585/15).
#[test]
fn a_cell_changed_while_its_table_had_no_width_is_measured_again() {
    for (prop, off, on) in [
        ("width", "auto", "600px"),
        ("table-layout", "fixed", "auto"),
    ] {
        let mut d = doc(&rows(&[(SHORT, "x"), ("cd", "y")]));
        let t = d.query_selector("table").unwrap();
        d.set_style(t, prop, off);
        settle(&mut d);
        set_text(&mut d, "#a0", LONG);
        d.set_style(t, prop, on);
        settle(&mut d);
        let fresh = doc(&rows(&[(LONG, "x"), ("cd", "y")]));
        assert_ne!(
            cells(&fresh),
            cells(&doc(&rows(&[(SHORT, "x"), ("cd", "y")]))),
            "positive control"
        );
        assert_eq!(cells(&d), cells(&fresh), "{prop}: {off} -> edit -> {on}");
    }
}

/// #2156: a table with a width inside an `inline-block`. The box's own
/// compute lays the table out before the root compute; its columns used to be
/// measured between the two, and kept their old shares.
#[test]
fn a_table_in_an_inline_block_moves_its_columns_after_a_cell_edit() {
    let html = |t: &str| {
        format!(
            "<div style='width:700px'>x <span style='display:inline-block'><table>\
             <tr><td id=a>{t}</td><td>yy</td></tr></table></span> y</div>"
        )
    };
    let mut d = doc(&html(SHORT));
    set_text(&mut d, "#a", LONG);
    let fresh = doc(&html(LONG));
    assert_ne!(cells(&fresh), cells(&doc(&html(SHORT))), "positive control");
    assert_eq!(cells(&d), cells(&fresh));
}
