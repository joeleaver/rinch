//! #2514 — a cell standing in an anonymous inline-table's place (a
//! `display: table-cell` box whose parent box is an inline, #2510) aligns its
//! content in a box taller than it by its own `vertical-align`, as a table's
//! cell does. rinch kept the content at the top.
//!
//! Every number is Chrome 156's (`--headless=new`, standards mode,
//! `16px/20px`), `getBoundingClientRect` of the content relative to its
//! cell. The fixtures are laid out in the bundled Inter; a line of text is
//! 20px whatever the face, and nothing here measures a width.

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
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", "font: 16px/20px ProbeFace; width: 400px");
    doc.set_inner_html(c, inner);
    doc.append_child(body, c);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn one(doc: &RinchDocument, m: &str) -> rinch_dom::node::RawNodeId {
    let found = query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1, "{m}");
    found[0]
}

/// How far `[data-m=k]`'s box is below the top of `[data-m=c]`'s, and the
/// cell's height.
fn offset(doc: &RinchDocument) -> (f32, f32) {
    let pos = |id| rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let (k, c) = (one(doc, "k"), one(doc, "c"));
    (
        (pos(k).1 - pos(c).1) as f32,
        doc.tree.get(c).unwrap().layout.height,
    )
}

/// The issue's cell: `height: 50px` around a 20px inline-block.
fn cell(align: &str) -> String {
    format!(
        "ab <span>x<span data-m=c style=\"display:table-cell;height:50px;\
         vertical-align:{align}\"><span data-m=k style=\"display:inline-block\">12</span>\
         </span>y</span> z"
    )
}

#[test]
fn a_middle_cell_centres_its_content_issue_2514() {
    // Chrome: 15 (rinch: 0).
    assert_eq!(offset(&lay_out(&cell("middle"))), (15.0, 50.0));
}

#[test]
fn a_bottom_cell_puts_its_content_at_the_bottom_issue_2514() {
    // Chrome: 30 (rinch: 0).
    assert_eq!(offset(&lay_out(&cell("bottom"))), (30.0, 50.0));
}

#[test]
fn a_top_cell_keeps_its_content_at_the_top_issue_2514() {
    // The positive control: Chrome 0, as rinch was.
    assert_eq!(offset(&lay_out(&cell("top"))), (0.0, 50.0));
}

#[test]
fn a_middle_cell_with_padding_moves_a_block_child_issue_2514() {
    // A block child, which Taffy places: `padding: 3px 0 5px` round a 50px
    // content box; the 10px block at 3 + 20. Chrome: 23, the cell 58.
    let d = lay_out(
        "ab <span>x<span data-m=c style=\"display:table-cell;height:50px;\
         padding:3px 0 5px;vertical-align:middle\"><div data-m=k \
         style=\"height:10px;width:10px\"></div></span>y</span> z",
    );
    assert_eq!(offset(&d), (23.0, 58.0));
}

#[test]
fn a_restyle_that_lays_nothing_out_keeps_the_content_aligned_issue_2514() {
    // A colour change runs no compute; the content stays where `middle`
    // put it (the restyle replaced the padding that carried it).
    let mut d = lay_out(&cell("middle"));
    let c = one(&d, "c");
    d.set_attribute(
        rinch_core::dom::NodeId(c),
        "style",
        "display:table-cell;height:50px;vertical-align:middle;color:red",
    );
    d.resolve_layout(800.0, 600.0);
    assert_eq!(offset(&d), (15.0, 50.0));
    // And laid out again at another width, still once.
    d.resolve_layout(700.0, 600.0);
    assert_eq!(offset(&d), (15.0, 50.0));
}
