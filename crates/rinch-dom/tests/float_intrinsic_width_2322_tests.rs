//! Intrinsic widths around floats and fixed percentage tables (#2322,
//! #2326), each number Chrome 156's for the same markup in a 560px column.
//!
//! - A `table-layout: fixed` table whose width is a percentage asks for all
//!   the room it is offered (Blink's `kTableMaxInlineSize` quirk): the auto
//!   table cell, block-hosted table, float or inline-block around it fills
//!   its room, and the percentage resolves against that. Email's
//!   `<table style="table-layout:fixed;width:75%">` inside a
//!   `<th style="display:block;width:100%">` column was drawn at a quarter of
//!   Chrome's width (#2322).
//! - A float sized from its content resolves its children's percentages
//!   against the width it took.
//! - The floats in an inline formatting context share its lines, so at
//!   max-content a line is as wide as its content and the floats in it: a
//!   cell holding a float and then text is as wide as both (#2326).
//! - A cell's `vertical-align` counts its floats as content: a float beside
//!   or above the cell's text stays at the cell's top (#2326).
//!
//! The content is sized boxes (inline-blocks for text), so no width here
//! depends on the host's fonts; a `y` is read only for a box at the top of
//! its line.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CSS: &str =
    "body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, &format!("<div style='width:560px'>{html}</div>"));
    d.resolve_layout(800.0, 800.0);
    d
}

/// The border box of `#id`, in page coordinates.
fn abs(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    let l = d.tree.get(n.0).unwrap().layout;
    let (mut x, mut y) = (0.0, 0.0);
    let mut cur = Some(n.0);
    while let Some(i) = cur {
        let node = d.tree.get(i).unwrap();
        x += node.layout.x;
        y += node.layout.y;
        cur = node.parent;
    }
    (x, y, l.width, l.height)
}

fn width(d: &RinchDocument, id: &str) -> f32 {
    abs(d, id).2
}

/// Within half a pixel: Chrome lays out in fractions, rinch rounds.
#[track_caller]
fn near(got: f32, chrome: f32) {
    assert!((got - chrome).abs() <= 0.5, "rinch {got}, Chrome {chrome}");
}

/// An inline-block standing for `w` px of text.
fn text(w: u32) -> String {
    format!("<span style='display:inline-block;width:{w}px;height:20px'></span>")
}

const BOX37: &str = "<div style='width:37px;height:10px'></div>";

// --- #2322: a fixed table with a percentage width fills its room ---------

#[test]
fn the_issue_shape_a_block_hosted_th_column_fills_the_host() {
    let d = doc(
        "<table id=o style='display:block;table-layout:fixed;width:100%'><tr>\
         <th id=th style='display:block;width:100%'>\
         <table id=f align=left style='table-layout:fixed;width:75%'><tr>\
         <td id=fc style='padding-right:100px'>\
         <img id=im alt='Brand Logo' width=250 style='display:block;width:100%'>\
         </td></tr></table></th></tr></table>",
    );
    // rinch was 196, 146 and 45: the content's width.
    near(width(&d, "th"), 558.0);
    near(width(&d, "f"), 417.0);
    near(width(&d, "im"), 312.0);
}

#[test]
fn a_fixed_percentage_table_fills_an_auto_table_cell() {
    let d = doc(&format!(
        "<table id=o><tr><td id=c><table id=f style='table-layout:fixed;width:70%'>\
         <tr><td>{BOX37}</td></tr></table></td></tr></table>"
    ));
    near(width(&d, "o"), 560.0);
    near(width(&d, "f"), 387.8);
}

#[test]
fn a_fixed_percentage_table_fills_an_inline_block() {
    let d = doc(&format!(
        "<div id=o style='display:inline-block'><table id=f style='table-layout:fixed;width:70%'>\
         <tr><td>{BOX37}</td></tr></table></div>"
    ));
    near(width(&d, "o"), 560.0);
    near(width(&d, "f"), 392.0);
}

#[test]
fn a_fixed_percentage_table_fills_no_more_than_a_max_width() {
    let d = doc(&format!(
        "<div id=o style='display:inline-block;max-width:310px'>\
         <table id=f style='table-layout:fixed;width:70%'><tr><td>{BOX37}</td></tr></table></div>"
    ));
    near(width(&d, "o"), 310.0);
    near(width(&d, "f"), 217.0);
}

#[test]
fn an_auto_layout_or_widthless_fixed_table_does_not_fill() {
    // The quirk is a fixed table's with a percentage width only.
    for table in [
        "<table id=f style='width:70%'>",
        "<table id=f style='table-layout:fixed'>",
    ] {
        let d = doc(&format!(
            "<table id=o><tr><td id=c>{table}<tr><td>{BOX37}</td></tr></table></td></tr></table>"
        ));
        near(width(&d, "o"), 49.0);
    }
}

// --- a float's percentage children ---------------------------------------

#[test]
fn a_float_resolves_its_childrens_percentages_against_its_width() {
    // rinch stretched both to the float's whole 333.
    for child in [
        "<div id=f style='width:70%;height:10px'></div>",
        &format!("<table id=f style='width:70%'><tr><td>{BOX37}</td></tr></table>"),
    ] {
        let d = doc(&format!(
            "<div id=o style='float:left'>{child}<div style='width:333px;height:10px'></div></div>"
        ));
        near(width(&d, "o"), 333.0);
        near(width(&d, "f"), 233.09);
    }
}

// --- #2326: floats share the lines of their inline formatting context ----

#[test]
fn an_auto_cell_holding_a_float_and_text_is_as_wide_as_both() {
    let d = doc(&format!(
        "<table id=o><tr><td id=c><table id=f align=left><tr><td>\
         <div style='width:201px;height:30px'></div></td></tr></table>{}</td></tr></table>",
        text(157)
    ));
    // 207 + 157 + the cell's padding and the table's spacing. rinch was 213.
    near(width(&d, "o"), 370.0);
    near(abs(&d, "f").1, 3.0);
}

#[test]
fn an_auto_cell_holding_a_float_and_more_text_than_fits_fills_its_room() {
    // The #2326 shape: a floated table and text, rinch 447 where Chrome
    // fills the 560.
    let d = doc(&format!(
        "<table id=o><tr><td id=c><table id=f align=left><tr><td>\
         <div style='width:301px;height:30px'></div></td></tr></table>{}</td></tr></table>",
        text(311)
    ));
    near(width(&d, "o"), 560.0);
}

#[test]
fn a_float_with_margins_shares_the_line() {
    let d = doc(&format!(
        "<table id=o><tr><td id=c><div id=f style='float:left;margin:7px 3px;width:131px;height:20px'>\
         </div>{}</td></tr></table>",
        text(97)
    ));
    near(width(&d, "o"), 240.0);
    let (x, y, _, _) = abs(&d, "f");
    near(x, 6.0);
    near(y, 10.0);
}

#[test]
fn nested_floats_in_a_cell_stay_at_its_top() {
    // The #2326 shape: rinch put the outer float at y 94 (here 104),
    // overflowing a table too narrow for it and its text.
    let d = doc(&format!(
        "<table id=o><tr><td id=c><div id=f1 style='float:left;border:1px solid'>\
         <div id=f2 style='float:left;width:60px;height:200px'></div><div id=t2>{}</div></div>{}\
         </td><td id=c2>{}</td></tr></table>",
        text(171),
        text(70),
        text(9)
    ));
    // A block after a float does not share its line: the outer float is
    // 2 + 171, and the cell 173 + 70.
    near(width(&d, "f1"), 173.0);
    near(width(&d, "o"), 262.0);
    near(abs(&d, "f1").1, 3.0);
    near(abs(&d, "c2").0, 249.0);
}

#[test]
fn a_float_after_a_forced_break_shares_only_its_own_line() {
    // The first line is the widest: 264, not 264 + 50.
    let d = doc(&format!(
        "<table id=o><tr><td id=c>{}<br><div id=f style='float:left;width:50px;height:20px'></div>{}\
         </td></tr></table>",
        text(264),
        text(38)
    ));
    near(width(&d, "o"), 270.0);
    // The second line, with its float, is the widest: 50 + 38.
    let d = doc(&format!(
        "<table id=o><tr><td id=c>{}<br><div id=f style='float:left;width:50px;height:20px'></div>{}\
         </td></tr></table>",
        text(32),
        text(38)
    ));
    near(width(&d, "o"), 94.0);
    // A float on the first line is not carried to a wider second one: 264,
    // not 50 + 264.
    let d = doc(&format!(
        "<table id=o><tr><td id=c><div id=f style='float:left;width:50px;height:20px'></div>{}<br>{}\
         </td></tr></table>",
        text(32),
        text(264)
    ));
    near(width(&d, "o"), 270.0);
}

#[test]
fn left_and_right_floats_both_share_the_line() {
    let d = doc(&format!(
        "<table id=o><tr><td id=c><div id=f style='float:left;width:100px;height:20px'></div>\
         <div id=g style='float:right;width:80px;height:20px'></div>{}</td></tr></table>",
        text(69)
    ));
    near(width(&d, "o"), 255.0);
    near(abs(&d, "g").0, 172.0);
}

#[test]
fn a_float_that_clears_the_side_starts_its_sum_again() {
    // The cleared float goes below the first: 100 + 69, not 100 + 80 + 69.
    let d = doc(&format!(
        "<table id=o><tr><td id=c><div id=f style='float:left;width:100px;height:20px'></div>\
         <div id=g style='float:left;clear:left;width:80px;height:20px'></div>{}</td></tr></table>",
        text(69)
    ));
    near(width(&d, "o"), 175.0);
    near(abs(&d, "f").1, 3.0);
}

#[test]
fn a_shrink_to_fit_box_holding_a_float_and_text_is_as_wide_as_both() {
    // An inline-block (laid out by its own compute) and a float. rinch was
    // 100 for both: the text wrapped below the float.
    for outer in [
        "<div id=o style='display:inline-block'>",
        "<div id=o style='float:left'>",
    ] {
        let d = doc(&format!(
            "{outer}<div id=f style='float:left;width:100px;height:20px'></div>{}</div>",
            text(83)
        ));
        near(width(&d, "o"), 183.0);
        near(abs(&d, "f").1, 0.0);
    }
}

// --- #2326: a cell's vertical alignment counts its floats ----------------

#[test]
fn a_middle_aligned_cell_keeps_a_float_taller_than_its_text_at_the_top() {
    // The text does not fit beside the float and goes below it; the cell is
    // as tall as both, so `vertical-align: middle` leaves nothing to share.
    // rinch aligned the cell by its line alone and moved the float to 51.
    let d = doc(&format!(
        "<table id=o><tr><td id=c style='width:300px'>\
         <div id=f style='float:left;width:290px;height:120px'></div>{}</td><td id=c2>{}</td>\
         </tr></table>",
        text(70),
        text(9)
    ));
    near(abs(&d, "f").1, 3.0);
}
