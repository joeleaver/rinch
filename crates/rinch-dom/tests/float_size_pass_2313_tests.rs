//! A floated table laid out by a size-only pass kept that pass's cell
//! layout (Postrinch's held-back mail, review of #2313/#2319): its one cell
//! came out at its min-content width inside a table laid out at its full
//! width, and the table's height left the wrapped cell out, so the next block
//! overlapped it. Reduced from the mail's structure (a logo in an `align=left`
//! table, in a `<th>`, next to a table whose `display: inline-block` cell
//! holds a row with a `width=32` column, which moves its columns and sends
//! the layout through a second root compute) to a synthetic page.
//!
//! The cause was Taffy's block algorithm (rinch's fork): a container asked
//! only its size laid its floats out in full, which wrote their descendants'
//! layouts for the room of that question, and the later final layout of the
//! float, answered from its cache, never wrote them back. The fork now lays a
//! float out in its container's run mode (`docs/internals/dependencies.md`,
//! *Taffy Fork*, item 11).
//!
//! Chrome 156 (headless, the bundled Inter as `@font-face`, the same markup
//! in a 560px block): with text in the cell, the table 89.28x24.39 and its
//! cell the same; with a loaded 3x2 picture (`width: 100%`), 5x4 and 3x2;
//! with an image's alt text, Chrome's cell is 16px wider than rinch's (its
//! broken-image icon), and both cells fill their table.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const RED_3X2: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP4z8DwH4YZkDkAm34L9XKwuTwAAAAASUVORK5CYII=";

/// The page, with `cell` in the floated table's one cell.
fn page(cell: &str) -> String {
    format!(
        r#"<table><tr><td><table><tr><td><table id="t1" style="border-collapse: collapse; width: 100% !important"><tr><th style="width: 100% !important; font-weight: normal"><table id="t2" cellspacing="0" align="left"><tr><td id="td">{cell}</td></tr></table></th></tr></table><table id="n"><tr><td><table><tr><th><table><tr><td><table><tr><td><table><tr><td><table><tr><td style="display: inline-block"><table><tr><td><table><tr><th></th><th width="32"></th></tr></table></td></tr></table></td></tr></table></td></tr></table></td></tr></table></td></tr></table></th></tr></table></td></tr></table></td></tr></table></td></tr></table>"#
    )
}

fn layout(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("I"),
            ..Default::default()
        }),
    );
    doc.load_css("html, body { margin: 0; font-family: I; font-size: 16px; line-height: 1.4 }");
    let root = doc.body();
    let host = doc.create_element("div");
    doc.set_attribute(host, "style", "width: 560px");
    doc.append_child(root, host);
    doc.set_inner_html(host, html);
    doc.resolve_layout(800.0, 800.0);
    doc
}

/// `(y, width, height)` of `#id`.
fn at(doc: &RinchDocument, id: &str) -> (f64, f32, f32) {
    let n = rinch_dom::query::query_all(&doc.tree, doc.tree.root_id, &format!("#{id}"))[0];
    let (_, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n, 1.0);
    let l = &doc.tree.nodes[n].layout;
    (y, l.width, l.height)
}

/// The floated table holds its cell: as wide and as tall as it, and the
/// next block starts below both.
fn holds(doc: &RinchDocument, what: &str) {
    let (_, tw, th) = at(doc, "t2");
    let (cy, cw, ch) = at(doc, "td");
    let (ny, _, _) = at(doc, "n");
    assert_eq!((cw, ch), (tw, th), "{what}: the cell fills its table");
    assert!(
        ny >= cy + f64::from(ch),
        "{what}: the next block is below the cell"
    );
}

#[test]
fn a_floated_tables_cell_keeps_its_final_width() {
    let doc = layout(&page("Brand Logo"));
    holds(&doc, "text");
    // Chrome 156: 89.28x24.39 (rinch lays out in whole pixels).
    let (_, w, h) = at(&doc, "t2");
    assert_eq!((w, h), (89.0, 24.0), "Chrome's table");
}

#[test]
fn an_alt_placeholder_in_a_floated_table_fills_its_cell() {
    let doc = layout(&page(
        r#"<a><img id="a" alt="Brand Logo" style="display: block"></a>"#,
    ));
    holds(&doc, "alt");
    let (_, aw, ah) = at(&doc, "a");
    let (_, cw, _) = at(&doc, "td");
    // The alt text on one line, as in Chrome (whose icon makes its cell
    // 105.28 wide); before the fix it wrapped at its widest word.
    assert_eq!(aw, cw - 2.0, "the alt box fills the cell's content box");
    assert_eq!(ah, 22.0, "one line");
}

#[test]
fn a_loaded_picture_in_a_floated_table_is_chromes() {
    let doc = layout(&page(&format!(
        r#"<img id="a" src="{RED_3X2}" style="display: block; width: 100%">"#
    )));
    holds(&doc, "loaded");
    // Chrome 156: the table and cell 5x4, the picture 3x2.
    assert_eq!(at(&doc, "td").1..=at(&doc, "td").2, 5.0..=4.0);
    assert_eq!(at(&doc, "a").1..=at(&doc, "a").2, 3.0..=2.0);
}
