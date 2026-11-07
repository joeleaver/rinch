//! Two table cases from the #2312 row-heights work.
//!
//! - **#2350.** A caption is outside the table box (CSS 2.2 §17.4), so it is
//!   outside the table's padding and outer `border-spacing`, and no spacing
//!   separates it from the rows or from another caption. rinch put it inside
//!   the padding that carries the outer spacing, and the grid's row gap
//!   followed it: one spacing (and the padding) too low, and the table one
//!   spacing taller than Chrome's.
//! - **#2358.** A table is never shorter than its rows, also when its parent
//!   hands it its height: a column flex item, a grid item in a shorter track,
//!   an absolutely positioned table (in a block, grid or flex container). rinch
//!   kept the height it was handed.
//!
//! Every expected number is Chrome 156's for the same markup
//! (`getBoundingClientRect`, `body { margin: 0; font: 16px/20px sans-serif }
//! td { padding: 0 } table { border-spacing: 0 }` at 800x600). The cells hold
//! fixed-size blocks and no text, so no number depends on the host's fonts.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CSS: &str =
    "body { margin: 0; font: 16px/20px sans-serif } td { padding: 0 } table { border-spacing: 0 }";

fn doc(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `(x, y, width, height)` of `#id`'s border box in the document.
fn rect(doc: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = doc.query_selector(&format!("#{id}")).expect(id);
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n.0, 1.0);
    let l = doc.tree.get(n.0).unwrap().layout;
    (x as f32, y as f32, l.width, l.height)
}

/// `(y, height)` only: a table's width as a flex or grid item is not what
/// these cases are about (Chrome stretches an auto-width table there).
fn yh(doc: &RinchDocument, id: &str) -> (f32, f32) {
    let (_, y, _, h) = rect(doc, id);
    (y, h)
}

/// `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

fn check(what: &str, doc: &RinchDocument, expected: &[(&str, Rect)]) {
    let got: Vec<_> = expected
        .iter()
        .map(|(id, _)| (*id, rect(doc, id)))
        .collect();
    for ((id, want), (_, have)) in expected.iter().zip(&got) {
        assert_eq!(have, want, "{what}: #{id}, Chrome {want:?} (all: {got:?})");
    }
}

fn check_yh(what: &str, html: &str, expected: &[(&str, (f32, f32))]) {
    let doc = doc(html);
    let got: Vec<_> = expected.iter().map(|(id, _)| (*id, yh(&doc, id))).collect();
    for ((id, want), (_, have)) in expected.iter().zip(&got) {
        assert_eq!(have, want, "{what}: #{id}, Chrome {want:?} (all: {got:?})");
    }
}

/// A `w`x`h` block.
fn d(w: u32, h: u32) -> String {
    format!("<div style='width:{w}px;height:{h}px'></div>")
}

// ---------------------------------------------------------------- #2350

#[test]
fn a_caption_is_outside_the_outer_spacing() {
    // The issue's case. rinch was: the caption at y = 3, the first row at 26,
    // the table 223 tall.
    let doc = doc(&format!(
        "<table id=t style='border-spacing:3px;height:200px'><caption id=cap>{}</caption>\
         <tr style='height:30px'><td id=a></td></tr><tr><td id=b>{}</td></tr></table>",
        d(10, 20),
        d(10, 10)
    ));
    check(
        "the issue's",
        &doc,
        &[
            ("t", (0.0, 0.0, 16.0, 220.0)),
            ("cap", (0.0, 0.0, 16.0, 20.0)),
            ("a", (3.0, 23.0, 10.0, 30.0)),
            ("b", (3.0, 56.0, 10.0, 161.0)),
        ],
    );
}

#[test]
fn captions_on_both_sides_are_outside_the_padding() {
    // Uneven spacing (4px across, 6px down) and padding on every side, so
    // no side's offset can stand in for another's.
    let doc = doc(&format!(
        "<table id=t style='border-spacing:4px 6px;padding:5px 3px 7px 2px'>\
         <caption id=c1>{}</caption><caption id=c2 style='caption-side:bottom'>{}</caption>\
         <tr style='height:30px'><td id=a>{}</td></tr><tr><td id=b>{}</td></tr></table>",
        d(0, 13),
        d(0, 11),
        d(40, 10),
        d(40, 17)
    ));
    check(
        "top and bottom",
        &doc,
        &[
            ("t", (0.0, 0.0, 53.0, 101.0)),
            ("c1", (0.0, 0.0, 53.0, 13.0)),
            ("c2", (0.0, 90.0, 53.0, 11.0)),
            ("a", (6.0, 24.0, 40.0, 30.0)),
            ("b", (6.0, 60.0, 40.0, 17.0)),
        ],
    );
}

#[test]
fn captions_stack_with_no_spacing_between_them() {
    let doc = doc(&format!(
        "<table id=t style='border-spacing:7px'><caption id=c1>{}</caption><caption id=c2>{}</caption>\
         <tr><td id=a>{}</td></tr></table>",
        d(0, 13),
        d(0, 11),
        d(40, 10)
    ));
    check(
        "two top captions",
        &doc,
        &[
            ("t", (0.0, 0.0, 54.0, 48.0)),
            ("c1", (0.0, 0.0, 54.0, 13.0)),
            ("c2", (0.0, 13.0, 54.0, 11.0)),
            ("a", (7.0, 31.0, 40.0, 10.0)),
        ],
    );
    // Below a table with a height: the rows share it, the captions take none.
    let doc = self::doc(&format!(
        "<table id=t style='border-spacing:3px;padding:4px;height:120px'>\
         <tr style='height:30px'><td id=a>{}</td></tr><tr><td id=b>{}</td></tr>\
         <caption id=c1 style='caption-side:bottom'>{}</caption><caption id=c2 style='caption-side:bottom'>{}</caption></table>",
        d(40, 10),
        d(40, 10),
        d(0, 13),
        d(0, 7)
    ));
    check(
        "two bottom captions",
        &doc,
        &[
            ("t", (0.0, 0.0, 54.0, 140.0)),
            ("a", (7.0, 7.0, 40.0, 30.0)),
            ("b", (7.0, 40.0, 40.0, 73.0)),
            ("c1", (0.0, 120.0, 54.0, 13.0)),
            ("c2", (0.0, 133.0, 54.0, 7.0)),
        ],
    );
}

#[test]
fn a_caption_spans_the_table_and_keeps_its_margins() {
    // As wide as the table's border box, across its columns' spacing.
    let doc = doc(&format!(
        "<table id=t style='border-spacing:5px 2px'><caption id=cap>{}</caption>\
         <tr><td id=a>{}</td><td id=b>{}</td></tr></table>",
        d(0, 9),
        d(40, 10),
        d(23, 10)
    ));
    check(
        "two columns",
        &doc,
        &[
            ("t", (0.0, 0.0, 78.0, 23.0)),
            ("cap", (0.0, 0.0, 78.0, 9.0)),
            ("a", (5.0, 11.0, 40.0, 10.0)),
            ("b", (50.0, 11.0, 23.0, 10.0)),
        ],
    );
    // A caption wider than the rows widens the table to it.
    let doc = self::doc(&format!(
        "<table id=t style='border-spacing:7px'><caption id=cap>{}</caption><tr><td id=a>{}</td></tr></table>",
        d(100, 13),
        d(40, 10)
    ));
    check(
        "a wide caption",
        &doc,
        &[
            ("t", (0.0, 0.0, 100.0, 37.0)),
            ("cap", (0.0, 0.0, 100.0, 13.0)),
            ("a", (7.0, 20.0, 86.0, 10.0)),
        ],
    );
    // Its own margins, on top.
    let doc = self::doc(&format!(
        "<table id=t style='border-spacing:7px'><caption id=cap style='margin:3px 2px 5px 4px'>{}</caption>\
         <tr><td id=a>{}</td></tr></table>",
        d(0, 13),
        d(40, 10)
    ));
    check(
        "caption margins",
        &doc,
        &[
            ("t", (0.0, 0.0, 54.0, 45.0)),
            ("cap", (4.0, 3.0, 48.0, 13.0)),
            ("a", (7.0, 28.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn a_caption_shorter_than_the_spacing() {
    // Shorter than the spacing, and empty: a margin trick that leaves the
    // row gap in place would clamp the caption's row at zero here.
    let doc = doc(&format!(
        "<table id=t style='border-spacing:9px'><caption id=cap>{}</caption><tr><td id=a>{}</td></tr></table>",
        d(0, 4),
        d(40, 10)
    ));
    check(
        "4px caption",
        &doc,
        &[
            ("t", (0.0, 0.0, 58.0, 32.0)),
            ("cap", (0.0, 0.0, 58.0, 4.0)),
            ("a", (9.0, 13.0, 40.0, 10.0)),
        ],
    );
    let doc = self::doc(&format!(
        "<table id=t style='border-spacing:9px'><caption id=cap></caption><tr><td id=a>{}</td></tr></table>",
        d(40, 10)
    ));
    check(
        "empty caption",
        &doc,
        &[
            ("t", (0.0, 0.0, 58.0, 28.0)),
            ("cap", (0.0, 0.0, 58.0, 0.0)),
            ("a", (9.0, 9.0, 40.0, 10.0)),
        ],
    );
}

/// The table's border is not taken out: rinch paints a table's border and
/// background around its captions, so the caption stays inside the border
/// (Chrome: outside it, at `(0, 0, 62, 13)`; #2394). The rows and the table
/// match.
#[test]
fn a_caption_stays_inside_the_tables_border() {
    let doc = doc(&format!(
        "<table id=t style='border-spacing:4px 6px;padding:5px;border:2px solid'><caption id=cap>{}</caption>\
         <tr style='height:30px'><td id=a>{}</td></tr></table>",
        d(0, 13),
        d(40, 10)
    ));
    check(
        "border",
        &doc,
        &[
            ("t", (0.0, 0.0, 62.0, 69.0)),
            ("cap", (2.0, 2.0, 58.0, 13.0)),
            ("a", (11.0, 26.0, 40.0, 30.0)),
        ],
    );
}

#[test]
fn a_caption_follows_a_change_of_spacing() {
    let mut doc = doc(&format!(
        "<table id=t style='border-spacing:3px'><caption id=cap>{}</caption><tr><td id=a>{}</td></tr></table>",
        d(0, 13),
        d(40, 10)
    ));
    assert_eq!(rect(&doc, "a"), (3.0, 16.0, 40.0, 10.0));
    let t = doc.query_selector("#t").unwrap();
    doc.set_attribute(t, "style", "border-spacing:7px");
    doc.resolve_layout(800.0, 600.0);
    // As Chrome 156 lays out the same table built with 7px spacing (the
    // 13px caption, then 7px of spacing above the row).
    check(
        "spacing 3 -> 7",
        &doc,
        &[
            ("t", (0.0, 0.0, 54.0, 37.0)),
            ("cap", (0.0, 0.0, 54.0, 13.0)),
            ("a", (7.0, 20.0, 40.0, 10.0)),
        ],
    );
}

// ---------------------------------------------------------------- #2358

const ROWS: &str =
    "<tr style='height:30px'><td id=a></td></tr><tr style='height:35px'><td id=b></td></tr>";

#[test]
fn a_column_flex_item_grows_to_its_rows() {
    // The issue's case: Chrome 60, rinch was 40.
    check_yh(
        "the issue's",
        "<div style='display:flex;flex-direction:column;height:300px'><table id=t style='height:40px'>\
         <tr style='height:30px'><td></td></tr><tr style='height:30px'><td></td></tr></table>\
         <div id=sib style='height:10px;flex:none'></div></div>",
        &[("t", (0.0, 60.0)), ("sib", (60.0, 10.0))],
    );
    check_yh(
        "spacing",
        "<div style='display:flex;flex-direction:column;height:300px'><table id=t style='height:47px;border-spacing:2px'>\
         <tr style='height:25px'><td id=a></td></tr><tr style='height:35px'><td id=b></td></tr></table>\
         <div id=sib style='height:10px;flex:none'></div></div>",
        &[
            ("t", (0.0, 66.0)),
            ("a", (2.0, 25.0)),
            ("b", (29.0, 35.0)),
            ("sib", (66.0, 10.0)),
        ],
    );
    // Shrunk below its rows by a sibling that does not shrink.
    check_yh(
        "shrunk",
        "<div style='display:flex;flex-direction:column;height:100px'><table id=t style='height:80px'>\
         <tr style='height:30px'><td></td></tr><tr style='height:30px'><td></td></tr></table>\
         <div id=sib style='height:60px;flex:none'></div></div>",
        &[("t", (0.0, 60.0)), ("sib", (60.0, 60.0))],
    );
}

#[test]
fn a_stretched_row_flex_item_grows_to_its_rows() {
    check_yh(
        "40px line",
        "<div style='display:flex;height:40px'><table id=t><tr style='height:30px'><td id=a></td></tr>\
         <tr style='height:30px'><td id=b></td></tr></table><div id=sib style='width:10px'></div></div>",
        &[
            ("t", (0.0, 60.0)),
            ("a", (0.0, 30.0)),
            ("b", (30.0, 30.0)),
            ("sib", (0.0, 40.0)),
        ],
    );
}

#[test]
fn an_absolutely_positioned_table_grows_to_its_rows() {
    // The issue's case: Chrome 60, rinch was 50.
    check_yh(
        "the issue's",
        "<table id=t style='position:absolute;top:0;left:0;height:50px'><tr style='height:30px'><td></td></tr>\
         <tr style='height:30px'><td></td></tr></table>",
        &[("t", (0.0, 60.0))],
    );
    check_yh(
        "a height from its insets",
        &format!(
            "<div style='position:relative;height:300px'><table id=t style='position:absolute;top:7px;bottom:250px;left:0'>{ROWS}</table></div>"
        ),
        &[("t", (7.0, 65.0)), ("a", (7.0, 30.0)), ("b", (37.0, 35.0))],
    );
    // Anchored by its bottom: it grows upwards.
    check_yh(
        "bottom anchored",
        &format!(
            "<div style='position:relative;height:300px'><table id=t style='position:absolute;bottom:10px;left:0;height:47px'>{ROWS}</table></div>"
        ),
        &[
            ("t", (225.0, 65.0)),
            ("a", (225.0, 30.0)),
            ("b", (255.0, 35.0)),
        ],
    );
    check_yh(
        "in a grid",
        &format!(
            "<div style='display:grid;position:relative;height:300px'><table id=t style='position:absolute;top:0;height:47px'>{ROWS}</table></div>"
        ),
        &[("t", (0.0, 65.0)), ("a", (0.0, 30.0)), ("b", (30.0, 35.0))],
    );
    check_yh(
        "in a flex container",
        &format!(
            "<div style='display:flex;position:relative;height:300px'><table id=t style='position:absolute;top:0;height:47px'>{ROWS}</table></div>"
        ),
        &[("t", (0.0, 65.0)), ("a", (0.0, 30.0)), ("b", (30.0, 35.0))],
    );
}

#[test]
fn a_grid_item_grows_past_its_track() {
    // The issue's case: Chrome 60, rinch was 40.
    check_yh(
        "the issue's",
        "<div style='display:grid;grid-template-rows:40px;height:300px'><table id=t>\
         <tr style='height:30px'><td></td></tr><tr style='height:30px'><td></td></tr></table></div>",
        &[("t", (0.0, 60.0))],
    );
    // It overflows its track; the next track stays where it was.
    check_yh(
        "the next track",
        &format!(
            "<div style='display:grid;grid-template-rows:43px 10px;height:300px'><table id=t>{ROWS}</table><div id=sib></div></div>"
        ),
        &[
            ("t", (0.0, 65.0)),
            ("a", (0.0, 30.0)),
            ("b", (30.0, 35.0)),
            ("sib", (43.0, 10.0)),
        ],
    );
    // A taller track still shares its height between the rows.
    let doc = doc(&format!(
        "<div style='display:grid;grid-template-rows:100px;height:300px'><table id=t>{ROWS}</table></div>"
    ));
    // Chrome 156: 46.141 and 53.859.
    assert_eq!(yh(&doc, "t"), (0.0, 100.0));
    assert_eq!(yh(&doc, "a"), (0.0, 46.0));
    assert_eq!(yh(&doc, "b"), (46.0, 54.0));
}
