//! A table row's `height`, and a collapsing table's padding.
//!
//! - A `<tr>`'s `height` (its `style`, or the `height` attribute, which maps
//!   to it) is the least the row is: a row of empty cells is that tall, and
//!   one whose cells are taller grows past it. rinch's rows are
//!   `display: contents` and generate no box, and their height reached no
//!   track: a `<tr height="32"><td></td></tr>` spacer row was 0px tall (an
//!   account-alert mail was 77px shorter than in Chrome).
//! - A table with `border-collapse: collapse` has no padding (CSS 2.1
//!   §17.6.2); rinch laid its cells out inside the padding it declared (a
//!   newsletter's footer tables were 40px taller each).
//!
//! Every expected number is Chrome 156's for the same markup (Playwright,
//! `getBoundingClientRect`, `body { margin: 0; font: 16px/20px sans-serif }`
//! at 800x600). The cells hold fixed-size blocks and no text, so no number
//! depends on the host's fonts.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

/// A 10x20 block, the content of a cell that has some.
const D: &str = "<div style='width:10px;height:20px'></div>";

fn doc(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `#id`'s border box in the document: `(x, y, width, height)`.
fn rect(doc: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = doc.query_selector(&format!("#{id}")).expect(id);
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n.0, 1.0);
    let l = &doc.tree.get(n.0).unwrap().layout;
    (x as f32, y as f32, l.width, l.height)
}

/// `(y, height)` of `#id`.
fn yh(doc: &RinchDocument, id: &str) -> (f32, f32) {
    let (_, y, _, h) = rect(doc, id);
    (y, h)
}

/// What, the markup, `(y, height)` of the first row's cell and of the next
/// row's, and the table's height.
type Case<'a> = (&'a str, String, (f32, f32), (f32, f32), f32);

#[test]
fn a_rows_height_is_the_least_it_is() {
    // (markup, Chrome's (y, height) of the first row's cell, of the next
    // row's cell, and the table's height)
    let cases: &[Case] = &[
        (
            "an empty row with a style height",
            format!(
                "<table id=t style='border-spacing:0'><tr style='height:32px'><td id=a style='padding:0'></td></tr>\
                 <tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (0.0, 32.0),
            (32.0, 20.0),
            52.0,
        ),
        (
            "the height attribute",
            format!(
                "<table id=t style='border-spacing:0'><tr height=37><td id=a style='padding:0'>{D}</td></tr>\
                 <tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (0.0, 37.0),
            (37.0, 20.0),
            57.0,
        ),
        (
            "a row shorter than its content grows",
            format!(
                "<table id=t style='border-spacing:0'><tr style='height:10px'><td id=a style='padding:0'>{D}</td></tr>\
                 <tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (0.0, 20.0),
            (20.0, 20.0),
            40.0,
        ),
        (
            "a cell taller than its row",
            format!(
                "<table id=t style='border-spacing:0'><tr style='height:30px'><td id=a style='padding:0;height:45px'></td></tr>\
                 <tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (0.0, 45.0),
            (45.0, 20.0),
            65.0,
        ),
        (
            "border-spacing and cell padding: the row is the cell's border box",
            format!(
                "<table id=t style='border-spacing:4px'><tr style='height:33px'><td id=a style='padding:3px'></td></tr>\
                 <tr><td id=b style='padding:3px'>{D}</td></tr></table>"
            ),
            (4.0, 33.0),
            (41.0, 26.0),
            71.0,
        ),
        (
            "a percentage in an auto-height table sizes nothing",
            format!(
                "<table id=t style='border-spacing:0'><tr style='height:50%'><td id=a style='padding:0'></td></tr>\
                 <tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (0.0, 0.0),
            (0.0, 20.0),
            20.0,
        ),
        (
            "a rowspan over two declared rows",
            format!(
                "<table id=t style='border-spacing:0'><tr style='height:30px'><td id=a rowspan=2 style='padding:0'></td>\
                 <td style='padding:0'></td></tr><tr style='height:41px'><td style='padding:0'></td></tr>\
                 <tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (0.0, 71.0),
            (71.0, 20.0),
            91.0,
        ),
        (
            "below a caption",
            format!(
                "<table id=t style='border-spacing:0'><caption>{D}</caption><tr style='height:29px'>\
                 <td id=a style='padding:0'></td></tr><tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (20.0, 29.0),
            (49.0, 20.0),
            69.0,
        ),
        (
            "a taller table's extra height goes to the row without one",
            format!(
                "<table id=t style='border-spacing:0;height:100px'><tr style='height:30px'><td id=a style='padding:0'>{D}</td></tr>\
                 <tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (0.0, 30.0),
            (30.0, 70.0),
            100.0,
        ),
        (
            "an em height",
            format!(
                "<table id=t style='border-spacing:0;font-size:16px'><tr style='height:3em'><td id=a style='padding:0'></td></tr>\
                 <tr><td id=b style='padding:0'>{D}</td></tr></table>"
            ),
            (0.0, 48.0),
            (48.0, 20.0),
            68.0,
        ),
    ];
    for (what, html, a, b, table) in cases {
        let doc = doc(html);
        assert_eq!(yh(&doc, "a"), *a, "{what}: the first row's cell");
        assert_eq!(yh(&doc, "b"), *b, "{what}: the next row's cell");
        assert_eq!(rect(&doc, "t").3, *table, "{what}: the table");
    }
}

/// The cell's content keeps its `vertical-align: middle` in the taller row:
/// Chrome puts the 20px block of a 37px row at 8.5.
#[test]
fn a_cells_content_is_centred_in_the_rows_height() {
    let doc = doc(&format!(
        "<table style='border-spacing:0'><tr height=37><td style='padding:0'><div id=c style='width:10px;height:20px'></div></td></tr>\
         <tr><td style='padding:0'>{D}</td></tr></table>"
    ));
    assert_eq!(yh(&doc, "c"), (8.5, 20.0));
}

/// When every row declares a height, a taller table shares its height out
/// between them in proportion to their heights: Chrome gives the 50px past
/// 30 + 20 as 60 and 40 (#2312; rinch's grid stretched them by equal shares,
/// 55 and 45).
#[test]
fn rows_that_all_declare_a_height_fill_a_taller_table() {
    let doc = doc(&format!(
        "<table id=t style='border-spacing:0;height:100px'><tr style='height:30px'><td id=a style='padding:0'>{D}</td></tr>\
         <tr style='height:20px'><td id=b style='padding:0'>{D}</td></tr></table>"
    ));
    assert_eq!(yh(&doc, "a"), (0.0, 60.0));
    assert_eq!(yh(&doc, "b"), (60.0, 40.0));
}

/// A row restyled to another height lays its table out again.
#[test]
fn restyling_a_rows_height_lays_the_table_out_again() {
    let mut doc = doc(&format!(
        "<table id=t style='border-spacing:0'><tr id=r style='height:32px'><td id=a style='padding:0'></td></tr>\
         <tr><td id=b style='padding:0'>{D}</td></tr></table>"
    ));
    assert_eq!(yh(&doc, "a"), (0.0, 32.0));
    let row = doc.query_selector("#r").unwrap();
    doc.set_attribute(row, "style", "height:47px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(yh(&doc, "a"), (0.0, 47.0), "the new height");
    assert_eq!(yh(&doc, "b"), (47.0, 20.0), "the next row moved down");
    doc.set_attribute(row, "style", "height:50%");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(yh(&doc, "a"), (0.0, 0.0), "a percentage sizes nothing");
}

#[test]
fn a_collapsing_table_has_no_padding() {
    // Chrome 156: the cell at the table's corner, the table as big as it.
    let doc = doc(&format!(
        "<table id=t style='border-collapse:collapse;padding:20px 13px'><tr><td id=a style='padding:0'>{D}</td></tr></table>"
    ));
    assert_eq!(rect(&doc, "t"), (0.0, 0.0, 10.0, 20.0));
    assert_eq!(rect(&doc, "a"), (0.0, 0.0, 10.0, 20.0));

    // Separate borders: the padding is there, around the cell.
    let doc = self::doc(&format!(
        "<table id=t style='border-collapse:separate;border-spacing:0;padding:20px 13px'><tr><td id=a style='padding:0'>{D}</td></tr></table>"
    ));
    assert_eq!(rect(&doc, "t"), (0.0, 0.0, 36.0, 60.0));
    assert_eq!(rect(&doc, "a"), (13.0, 20.0, 10.0, 20.0));

    // A `display: block` table is a block holding the anonymous table its
    // rows make: the block's padding is its own, collapsing or not.
    let doc = self::doc(&format!(
        "<table id=t style='display:block;border-collapse:collapse;padding:7px 5px'><tr><td id=a style='padding:0'>{D}</td></tr></table>"
    ));
    assert_eq!(rect(&doc, "t").3, 34.0);
    assert_eq!(rect(&doc, "a"), (5.0, 7.0, 10.0, 20.0));
}
