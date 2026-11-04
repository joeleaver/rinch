//! #2144 — a layout reached through a **history** of sizes must be the layout
//! a fresh document in the same end state gets.
//!
//! Two shapes went stale: a table cell's content kept its old position after
//! another cell of the row grew, and a float's (shrink-to-fit) content kept
//! its old line count after the float's containing block narrowed and widened
//! again. Each fixture runs the history, then an unrelated edit (so the last
//! layout is an incremental one that has to reuse what the history left), and
//! compares every element's box with a fresh layout of the end state.
//!
//! No number here is a glyph measurement: each assertion compares two layouts
//! of the same document on the same host.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::node::NodeKind;

const VP: (f32, f32) = (800.0, 600.0);

const BASE_CSS: &str = "body { margin: 0; font-size: 16px; line-height: 20px }";

fn doc_with(css: &str, html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(BASE_CSS);
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    settle(&mut doc);
    doc
}

fn settle(doc: &mut RinchDocument) {
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
}

fn id(doc: &RinchDocument, sel: &str) -> NodeId {
    doc.query_selector(sel)
        .unwrap_or_else(|| panic!("no element matches {sel}"))
}

/// Every element under `<body>` in document order, with its tag, `id`
/// attribute and box (parent-relative, the layout itself).
fn boxes(doc: &RinchDocument) -> Vec<(String, (f32, f32, f32, f32))> {
    let mut out = Vec::new();
    let mut stack = vec![doc.body().0];
    while let Some(n) = stack.pop() {
        let node = &doc.tree.nodes[n];
        if let NodeKind::Element(el) = &node.kind {
            let l = &node.layout;
            let name = format!(
                "{}#{}",
                el.tag,
                node.attributes.get("id").cloned().unwrap_or_default()
            );
            out.push((name, (l.x, l.y, l.width, l.height)));
        }
        stack.extend(node.children.iter().rev().copied());
    }
    out
}

#[track_caller]
fn assert_same_as_fresh(got: &RinchDocument, fresh: &RinchDocument, what: &str) {
    let (a, b) = (boxes(got), boxes(fresh));
    assert_eq!(a.len(), b.len(), "{what}: element count");
    let diffs: Vec<String> = a
        .iter()
        .zip(&b)
        .filter(|(x, y)| x != y)
        .map(|(x, y)| format!("{}: history {:?} fresh {:?}", x.0, x.1, y.1))
        .collect();
    assert!(
        diffs.is_empty(),
        "{what}: the history differs from a fresh layout:\n{}",
        diffs.join("\n")
    );
}

const TABLE_CSS: &str = "td { padding: 0 } table { border-spacing: 0 }
    .pill { display: inline-flex; max-width: 100% }
    .half { display: inline-block; width: 50%; height: 10px }";

const GROWN: &str = "a much longer text in the other cell that takes room";

fn table_html(table_style: &str, b: &str) -> String {
    format!(
        "<table style=\"{table_style}\"><tr><td id=a><span id=pill class=pill>{}</span>\
         <span id=half class=half></span></td><td id=b>{b}</td></tr></table>\
         <p id=other>other</p>",
        "word ".repeat(40)
    )
}

/// Where `#pill` is drawn, relative to `#a`'s border box: its own layout
/// plus the cell's content-box origin, which is how paint and hit testing
/// place an atomic inline (`paint::ifc_content_box_offset`).
fn pill_in_cell(doc: &RinchDocument) -> f32 {
    let pill = id(doc, "#pill");
    let node = &doc.tree.nodes[pill.0];
    let (_, dy) = rinch_dom::paint::ifc_content_box_offset(&doc.tree, node);
    let mut y = node.layout.y + dy;
    let a = id(doc, "#a").0;
    let mut cur = node.parent;
    while let Some(p) = cur {
        if p == a {
            break;
        }
        y += doc.tree.nodes[p].layout.y;
        cur = doc.tree.nodes[p].parent;
    }
    y
}

/// `#a`'s content is centred: `vertical-align: middle` (the UA's, through
/// the row) splits the room its lines leave between above and below, and the
/// pill is on the first line, at the top of it.
#[track_caller]
fn assert_centred(doc: &RinchDocument, what: &str) {
    let a = &doc.tree.nodes[id(doc, "#a").0];
    let lines = a
        .text_layout
        .as_ref()
        .expect("#a lays out its lines")
        .layout
        .height();
    let room = a.layout.height - lines;
    assert!(
        room > 0.0,
        "{what}: positive control, #a is taller than its lines"
    );
    assert_eq!(
        pill_in_cell(doc),
        room / 2.0,
        "{what}: the pill is drawn in the middle of #a"
    );
}

fn edit_other(doc: &mut RinchDocument, text: &str) {
    let other = doc.tree.nodes[id(doc, "#other").0].children[0];
    doc.set_text_content(NodeId(other), text);
    settle(doc);
}

fn table_history(table_style: &str) {
    let mut doc = doc_with(TABLE_CSS, &table_html(table_style, "x"));
    let fresh = doc_with(TABLE_CSS, &table_html(table_style, GROWN));
    assert_centred(&fresh, "fresh");
    assert_ne!(
        boxes(&doc),
        boxes(&fresh),
        "positive control: growing #b moves something"
    );

    let b = id(&doc, "#b");
    doc.set_inner_html(b, GROWN);
    settle(&mut doc);
    assert_centred(&doc, "#b grown");
    assert_same_as_fresh(&doc, &fresh, "#b grown");
    // Unrelated edits re-read the table without rebuilding #a's lines.
    for (i, text) in ["edited", "edited again", "and again"].iter().enumerate() {
        edit_other(&mut doc, text);
        assert_centred(&doc, &format!("unrelated edit {i}"));
        let mut fresh = doc_with(TABLE_CSS, &table_html(table_style, GROWN));
        edit_other(&mut fresh, text);
        assert_same_as_fresh(&doc, &fresh, &format!("unrelated edit {i}"));
    }
}

/// Case 1 of #2144: each layout after `#b` grew added the cell's
/// `vertical-align` offset to the pill once more, on top of the padding that
/// already moves it: drawn at 2x, 3x, ... the offset.
#[test]
fn a_cell_after_another_cell_grows_matches_a_fresh_layout() {
    table_history("");
}

#[test]
fn a_cell_in_a_fixed_width_table_after_another_cell_grows_matches_a_fresh_layout() {
    table_history("width: 400px");
}

/// A paint-only restyle of the cell (its `color`, inherited from `<body>`)
/// runs no compute: the cascade's `padding-top` replaced the one alignment
/// wrote, and nothing aligned the cell again, so its lines went to the top.
#[test]
fn a_colour_restyle_keeps_a_cell_centred() {
    let mut doc = doc_with(TABLE_CSS, &table_html("", GROWN));
    assert_centred(&doc, "before");
    let before = boxes(&doc);
    let body = doc.body();
    doc.set_style(body, "color", "rgb(200, 0, 0)");
    settle(&mut doc);
    assert_centred(&doc, "after a colour change");
    assert_eq!(boxes(&doc), before, "a colour change moves no box");
    edit_other(&mut doc, "edited");
    assert_centred(&doc, "after a colour change and an edit");
}

/// The same restyle on a cell whose content is a block box: Taffy placed it,
/// so it carries the offset itself, and aligning the cell again must not add
/// it twice.
#[test]
fn a_colour_restyle_keeps_a_block_in_a_cell_where_it_was() {
    let mut doc = doc_with(
        TABLE_CSS,
        "<table><tr><td id=a><div id=k style='height:10px'></div></td>\
         <td><div style='height:50px'></div></td></tr></table><p id=other>other</p>",
    );
    let k = id(&doc, "#k");
    assert_eq!(
        doc.tree.nodes[k.0].layout.y, 20.0,
        "positive control: (50 - 10) / 2"
    );
    let body = doc.body();
    doc.set_style(body, "color", "rgb(200, 0, 0)");
    settle(&mut doc);
    assert_eq!(doc.tree.nodes[k.0].layout.y, 20.0, "after a colour change");
    edit_other(&mut doc, "edited");
    assert_eq!(doc.tree.nodes[k.0].layout.y, 20.0, "after an edit");
}

/// `vertical-align` itself is no Taffy property: changing it runs no
/// compute, and the cell is aligned again from the offset its block child
/// carries.
#[test]
fn a_vertical_align_change_moves_a_block_in_a_cell() {
    let mut doc = doc_with(
        TABLE_CSS,
        "<table><tr><td id=a><div id=k style='height:10px'></div></td>\
         <td><div style='height:50px'></div></td></tr></table>",
    );
    let (a, k) = (id(&doc, "#a"), id(&doc, "#k"));
    assert_eq!(doc.tree.nodes[k.0].layout.y, 20.0, "middle: (50 - 10) / 2");
    doc.set_style(a, "vertical-align", "bottom");
    settle(&mut doc);
    assert_eq!(doc.tree.nodes[k.0].layout.y, 40.0, "bottom");
    doc.set_style(a, "vertical-align", "top");
    settle(&mut doc);
    assert_eq!(doc.tree.nodes[k.0].layout.y, 0.0, "top");
}
