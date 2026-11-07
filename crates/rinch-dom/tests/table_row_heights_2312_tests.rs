//! How a table shares height between its rows (#2312), as Blink does.
//!
//! - A `rowspan` cell taller than its rows grows the rows past its first
//!   where another spanning cell starts; else its rows of no declared height
//!   that hold something, in proportion to their heights; else, when they are
//!   all empty, the last; else every row that holds something, in proportion
//!   to its height. rinch shared it equally (Taffy's grid).
//! - A table taller than its rows gives the extra height to its body
//!   sections (not a `thead` or `tfoot`), in proportion to their heights, and
//!   each section to its rows as above, except that empty rows grow when the
//!   rows that hold something all declare a height. rinch shared it equally
//!   between the rows of no declared height.
//! - A caption takes none of it: the table's `height` is its rows'.
//! - A table whose `height` is less than its rows is as tall as its rows.
//! - A `display: block` cell stands in an anonymous cell, at its top: it is
//!   as tall as its content, not stretched to its row.
//!
//! Every expected number is Chrome 156's for the same markup
//! (`getBoundingClientRect`, `body { margin: 0; font: 16px/20px sans-serif }`
//! at 800x600). The cells hold fixed-size blocks and no text, so no number
//! depends on the host's fonts. rinch rounds each box's edges to whole
//! pixels, so Chrome's numbers are compared rounded the same way.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

fn doc(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// A `width`x`h` block.
fn d(h: u32) -> String {
    format!("<div style='width:10px;height:{h}px'></div>")
}

/// `(y, height)` of `#id`'s border box in the document.
fn yh(doc: &RinchDocument, id: &str) -> (f32, f32) {
    let n = doc.query_selector(&format!("#{id}")).expect(id);
    let (_, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n.0, 1.0);
    (y as f32, doc.tree.get(n.0).unwrap().layout.height)
}

/// Chrome's `(y, height)` on rinch's whole-pixel grid: Taffy rounds each
/// edge, so a box is as tall as the distance between its rounded edges.
fn snapped((y, h): (f32, f32)) -> (f32, f32) {
    (y.round(), (y + h).round() - y.round())
}

/// Each `(id, (y, height))` of `expected` (Chrome's), snapped.
fn check(what: &str, html: &str, expected: &[(&str, (f32, f32))]) {
    let doc = doc(html);
    let got: Vec<_> = expected.iter().map(|(id, _)| (*id, yh(&doc, id))).collect();
    for ((id, want), (_, have)) in expected.iter().zip(&got) {
        assert_eq!(
            *have,
            snapped(*want),
            "{what}: #{id}, Chrome {want:?} (all: {got:?})"
        );
    }
}

#[test]
fn a_rowspan_grows_its_rows_in_proportion_to_their_heights() {
    // The issue's case: 70px past 30 + 20 goes 42 and 28.
    check(
        "two declared rows",
        "<table id=t style='border-spacing:0'><tr style='height:30px'><td rowspan=2 id=s style='padding:0'>\
         <div style='width:10px;height:120px'></div></td><td id=b style='padding:0'></td></tr>\
         <tr style='height:20px'><td id=c style='padding:0'></td></tr></table>",
        &[("t", (0.0, 120.0)), ("b", (0.0, 72.0)), ("c", (72.0, 48.0))],
    );
    // Spacing and padding, three rows: the one row of no declared height
    // that holds something takes all of it.
    check(
        "a free row takes it",
        &format!(
            "<table id=t style='border-spacing:3px'><tr style='height:30px'><td id=s rowspan=3 style='padding:2px'>{}</td>\
             <td id=a style='padding:2px'></td></tr><tr style='height:20px'><td id=b style='padding:2px'></td></tr>\
             <tr><td id=c style='padding:2px'>{}</td></tr></table>",
            d(200),
            d(15)
        ),
        &[
            ("t", (0.0, 210.0)),
            ("s", (3.0, 204.0)),
            ("a", (3.0, 30.0)),
            ("b", (36.0, 20.0)),
            ("c", (59.0, 148.0)),
        ],
    );
    // Two free rows share it by their heights; the declared one keeps 40.
    check(
        "free rows by their heights",
        &format!(
            "<table id=t style='border-spacing:2px'><tr><td id=s rowspan=3 style='padding:1px'>{}</td>\
             <td id=a style='padding:1px'>{}</td></tr><tr style='height:40px'><td id=b style='padding:1px'>{}</td></tr>\
             <tr><td id=c style='padding:1px'>{}</td></tr></table>",
            d(150),
            d(10),
            d(5),
            d(30)
        ),
        &[
            ("a", (2.0, 29.453)),
            ("b", (33.453, 40.0)),
            ("c", (75.453, 78.547)),
        ],
    );
}

#[test]
fn a_rowspan_grows_the_row_where_another_starts_first() {
    check(
        "nested rowspan",
        &format!(
            "<table id=t style='border-spacing:2px'><tr><td id=s rowspan=3 style='padding:1px'>{}</td>\
             <td id=a style='padding:1px'>{}</td></tr><tr><td id=b style='padding:1px'>{}</td>\
             <td rowspan=2 id=s2 style='padding:1px'>{}</td></tr><tr><td id=c style='padding:1px'>{}</td></tr></table>",
            d(150),
            d(10),
            d(30),
            d(20),
            d(5)
        ),
        &[
            ("t", (0.0, 156.0)),
            ("a", (2.0, 12.0)),
            ("b", (16.0, 129.0)),
            ("s2", (16.0, 138.0)),
            ("c", (147.0, 7.0)),
        ],
    );
}

#[test]
fn a_rowspan_over_empty_rows_grows_the_last() {
    check(
        "empty rows",
        &format!(
            "<table id=t style='border-spacing:4px'><tr><td id=s rowspan=3 style='padding:0'>{}</td>\
             <td id=a style='padding:0'></td></tr><tr><td id=b style='padding:0'></td></tr>\
             <tr><td id=c style='padding:0'></td></tr></table>",
            d(100)
        ),
        &[
            ("t", (0.0, 108.0)),
            ("a", (4.0, 0.0)),
            ("b", (8.0, 0.0)),
            ("c", (12.0, 92.0)),
        ],
    );
}

#[test]
fn a_taller_tables_rows_share_by_their_content() {
    // The issue's case.
    check(
        "the issue's",
        &format!(
            "<table id=t style='border-spacing:0;height:200px'><tr style='height:30px'><td id=a style='padding:0'></td></tr>\
             <tr><td id=b style='padding:0'>{}</td></tr><tr><td id=c style='padding:0'>{}</td></tr></table>",
            d(20),
            d(40)
        ),
        &[
            ("t", (0.0, 200.0)),
            ("a", (0.0, 30.0)),
            ("b", (30.0, 56.656)),
            ("c", (86.656, 113.344)),
        ],
    );
    // Padding on the table, spacing, cell padding, and an empty row that
    // stays empty.
    check(
        "spacing, padding, an empty row",
        &format!(
            "<table id=t style='border-spacing:4px;padding:3px;height:300px'><tr style='height:30px'>\
             <td id=a style='padding:2px'></td></tr><tr><td id=b style='padding:2px'>{}</td></tr>\
             <tr><td id=c style='padding:2px'>{}</td></tr><tr><td id=e style='padding:0'></td></tr></table>",
            d(20),
            d(40)
        ),
        &[
            ("t", (0.0, 300.0)),
            ("a", (7.0, 30.0)),
            ("b", (41.0, 86.109)),
            ("c", (131.109, 157.891)),
            ("e", (293.0, 0.0)),
        ],
    );
    // A cell's `height` declares its row as a row's does.
    check(
        "a cell's height",
        &format!(
            "<table id=t style='border-spacing:2px;height:150px'><tr><td id=a style='padding:0;height:30px'></td></tr>\
             <tr><td id=b style='padding:0'>{}</td></tr><tr><td id=c style='padding:0'>{}</td></tr></table>",
            d(10),
            d(20)
        ),
        &[
            ("a", (2.0, 30.0)),
            ("b", (34.0, 37.328)),
            ("c", (73.328, 74.672)),
        ],
    );
}

#[test]
fn declared_rows_share_a_taller_table_by_their_heights() {
    check(
        "every row declared",
        &format!(
            "<table id=t style='border-spacing:2px;height:150px'><tr style='height:30px'><td id=a style='padding:0'></td></tr>\
             <tr style='height:20px'><td id=b style='padding:0'></td></tr>\
             <tr style='height:45px'><td id=c style='padding:0'>{}</td></tr></table>",
            d(10)
        ),
        &[
            ("a", (2.0, 44.828)),
            ("b", (48.828, 29.891)),
            ("c", (80.719, 67.281)),
        ],
    );
    // The rows that hold nothing grow, and not the one declared `0`.
    check(
        "empty rows",
        "<table id=t style='border-spacing:2px;height:120px'><tr style='height:30px'><td id=a style='padding:0'></td></tr>\
         <tr><td id=b style='padding:0'></td></tr><tr><td id=c style='padding:0'></td></tr>\
         <tr style='height:0'><td id=e style='padding:0'></td></tr></table>",
        &[
            ("a", (2.0, 30.0)),
            ("b", (34.0, 40.0)),
            ("c", (76.0, 40.0)),
            ("e", (118.0, 0.0)),
        ],
    );
}

#[test]
fn a_taller_table_grows_its_body_not_its_header_or_footer() {
    check(
        "thead, tbody, tfoot",
        &format!(
            "<table id=t style='border-spacing:2px;height:200px'><thead><tr><td id=a style='padding:0'>{}</td></tr></thead>\
             <tbody><tr><td id=b style='padding:0'>{}</td></tr><tr><td id=c style='padding:0'>{}</td></tr></tbody>\
             <tfoot><tr><td id=e style='padding:0'>{}</td></tr></tfoot></table>",
            d(20),
            d(10),
            d(30),
            d(15)
        ),
        &[
            ("a", (2.0, 20.0)),
            ("b", (24.0, 38.75)),
            ("c", (64.75, 116.25)),
            ("e", (183.0, 15.0)),
        ],
    );
    // Two bodies share it by their heights, and each its rows.
    check(
        "two tbodies",
        &format!(
            "<table id=t style='border-spacing:2px;height:200px'><tbody><tr><td id=a style='padding:0'>{}</td></tr></tbody>\
             <tbody><tr style='height:10px'><td id=b style='padding:0'></td></tr>\
             <tr><td id=c style='padding:0'>{}</td></tr></tbody></table>",
            d(20),
            d(30)
        ),
        &[
            ("a", (2.0, 62.578)),
            ("b", (66.578, 10.0)),
            ("c", (78.578, 119.422)),
        ],
    );
}

#[test]
fn a_caption_takes_none_of_a_tables_height() {
    // The issue's case: the table is its height and the caption.
    check(
        "the issue's",
        &format!(
            "<table id=t style='border-spacing:0;height:200px'><caption id=cap>{}</caption>\
             <tr style='height:30px'><td id=a style='padding:0'></td></tr><tr><td id=b style='padding:0'></td></tr></table>",
            d(20)
        ),
        &[
            ("t", (0.0, 220.0)),
            ("cap", (0.0, 20.0)),
            ("a", (20.0, 30.0)),
            ("b", (50.0, 170.0)),
        ],
    );
}

/// With spacing, rinch's caption sits inside the table's outer spacing (at
/// `y = 3`, and one more spacing above the first row), where Chrome's is
/// outside it (#2350): the rows' heights are compared, and the table is that
/// one spacing taller than Chrome's.
#[test]
fn a_caption_takes_none_of_a_spaced_tables_height() {
    let doc = doc(&format!(
        "<table id=t style='border-spacing:3px;height:200px'><caption id=cap>{}</caption>\
         <tr style='height:30px'><td id=a style='padding:0'></td></tr><tr><td id=b style='padding:0'>{}</td></tr>\
         <tr><td id=c style='padding:0'>{}</td></tr></table>",
        d(20),
        d(10),
        d(25)
    ));
    // Chrome 156: the rows at 23, 56 and 104.141, 30, 45.141 and 112.859
    // tall, in a table 220 tall; rinch's are 3px lower.
    let rows = [yh(&doc, "a"), yh(&doc, "b"), yh(&doc, "c")];
    let chrome = [(26.0, 30.0), (59.0, 45.141), (107.141, 112.859)].map(snapped);
    assert_eq!(rows, chrome, "the rows");
    // Chrome: 220. rinch: 220 and the caption's extra gap (#2350).
    assert_eq!(yh(&doc, "t"), (0.0, 223.0), "the table");
}

#[test]
fn a_table_shorter_than_its_rows_grows_to_them() {
    check(
        "the issue's",
        "<table id=t style='border-spacing:0;height:50px'><tr style='height:30px'><td id=a style='padding:0'></td></tr>\
         <tr style='height:30px'><td id=b style='padding:0'></td></tr></table>",
        &[("t", (0.0, 60.0)), ("a", (0.0, 30.0)), ("b", (30.0, 30.0))],
    );
    check(
        "spacing, three rows",
        "<table id=t style='border-spacing:2px;height:50px'><tr style='height:30px'><td id=a style='padding:0'></td></tr>\
         <tr style='height:20px'><td id=b style='padding:0'></td></tr>\
         <tr style='height:25px'><td id=c style='padding:0'></td></tr></table>",
        &[
            ("t", (0.0, 83.0)),
            ("a", (2.0, 30.0)),
            ("b", (34.0, 20.0)),
            ("c", (56.0, 25.0)),
        ],
    );
}

#[test]
fn a_block_cell_is_as_tall_as_its_content() {
    // The issue's case.
    check(
        "the issue's",
        &format!(
            "<table id=t style='border-spacing:0'><tr style='height:40px'><td style='display:block;padding:0' id=a>{}</td></tr>\
             <tr><td id=b style='padding:0'>{}</td></tr></table>",
            d(10),
            d(18)
        ),
        &[("t", (0.0, 58.0)), ("a", (0.0, 10.0)), ("b", (40.0, 18.0))],
    );
    check(
        "spacing and padding",
        &format!(
            "<table id=t style='border-spacing:2px'><tr style='height:40px'><td style='display:block;padding:1px' id=a>{}</td></tr>\
             <tr style='height:23px'><td id=b style='padding:0'>{}</td></tr><tr><td id=c style='padding:0'>{}</td></tr></table>",
            d(10),
            d(5),
            d(7)
        ),
        &[
            ("t", (0.0, 78.0)),
            ("a", (2.0, 12.0)),
            ("b", (44.0, 23.0)),
            ("c", (69.0, 7.0)),
        ],
    );
}
