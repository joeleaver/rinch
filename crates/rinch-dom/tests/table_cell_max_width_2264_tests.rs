//! A table cell's `max-width` (#2264). It caps what the cell asks of its
//! column — its min-content, max-content and CSS `width` contributions — and
//! nothing else: the cell is laid out at its column's width, past its
//! `max-width` when the column is wider. Blink: `CreateCellInlineConstraint`
//! caps the contributions, `SetupTableCellConstraintSpaceBuilder` fixes the
//! cell's inline size to its columns'. No condition on the table.
//!
//! Every expected number is Chrome 156's for the same markup, laid out at
//! 600x800 with the CSS below. The boxes are sized divs, so no number here
//! depends on the host's fonts.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CSS: &str =
    "body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(600.0, 800.0);
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

fn bx(w: f32, h: f32) -> String {
    format!("<div style='width:{w}px;height:{h}px'></div>")
}

#[test]
fn an_auto_tables_column_is_capped_at_the_cells_max_width() {
    // 2a. The 200px box overflows a 90px column.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td></tr></table>",
        bx(200.0, 20.0)
    ));
    assert_eq!(abs(&d, "t"), (0.0, 0.0, 90.0, 20.0));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 90.0, 20.0));
}

#[test]
fn a_cell_fills_its_tables_width_past_its_max_width() {
    // 2b: the one column takes the table's 400.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:400px'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td></tr></table>",
        bx(50.0, 20.0)
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 400.0, 20.0));
}

#[test]
fn the_capped_contribution_weighs_the_columns_share() {
    // 2c: max-contents 90 (200 capped) and 100 share 400 by weight.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:400px'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td>\
         <td id=b style='padding:0'>{}</td></tr></table>",
        bx(200.0, 20.0),
        bx(100.0, 20.0)
    ));
    // Chrome: a 189.469, b 210.531; rinch rounds the edges to whole pixels.
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 189.0, 20.0));
    assert_eq!(abs(&d, "b"), (189.0, 0.0, 211.0, 20.0));
    // 2c3: an uncapped 40 next to two 100s; the cell ends at 83 past its 60.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:500px'><tr>\
         <td id=a style='padding:0;max-width:60px'>{}</td>\
         <td id=b style='padding:0'>{}</td><td id=c style='padding:0'>{}</td></tr></table>",
        bx(40.0, 20.0),
        bx(100.0, 20.0),
        bx(100.0, 20.0)
    ));
    // Chrome: edges at 0, 83.328, 291.656 and 500.
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 83.0, 20.0));
    assert_eq!(abs(&d, "b"), (83.0, 0.0, 209.0, 20.0));
    assert_eq!(abs(&d, "c"), (292.0, 0.0, 208.0, 20.0));
}

#[test]
fn a_cell_fills_a_column_another_row_widened() {
    // 2d: no table width, and the column is the other row's 300.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0'>\
         <tr><td id=a style='padding:0;max-width:90px'>{}</td></tr>\
         <tr><td id=b style='padding:0'>{}</td></tr></table>",
        bx(50.0, 20.0),
        bx(300.0, 20.0)
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 300.0, 20.0));
    assert_eq!(abs(&d, "b"), (0.0, 20.0, 300.0, 20.0));
}

#[test]
fn a_cell_fills_a_column_a_colspan_widened() {
    // 2d2: the 400px spanning cell splits evenly between two 50s.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0'>\
         <tr><td id=a style='padding:0;max-width:90px'>{}</td><td id=b style='padding:0'>{}</td></tr>\
         <tr><td id=c colspan=2 style='padding:0'>{}</td></tr></table>",
        bx(50.0, 20.0),
        bx(50.0, 20.0),
        bx(400.0, 20.0)
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 200.0, 20.0));
    assert_eq!(abs(&d, "b"), (200.0, 0.0, 200.0, 20.0));
    assert_eq!(abs(&d, "c"), (0.0, 20.0, 400.0, 20.0));
}

#[test]
fn a_spanning_cells_max_width_caps_what_it_spreads() {
    // The 200px box in a `max-width: 90px` colspan asks 90 of two 30s.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0'>\
         <tr><td id=a style='padding:0'>{}</td><td id=b style='padding:0'>{}</td></tr>\
         <tr><td id=c colspan=2 style='padding:0;max-width:90px'>{}</td></tr></table>",
        bx(30.0, 20.0),
        bx(30.0, 20.0),
        bx(200.0, 20.0)
    ));
    assert_eq!(abs(&d, "t"), (0.0, 0.0, 90.0, 40.0));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 45.0, 20.0));
    assert_eq!(abs(&d, "b"), (45.0, 0.0, 45.0, 20.0));
    assert_eq!(abs(&d, "c"), (0.0, 20.0, 90.0, 20.0));
}

#[test]
fn a_cell_fills_a_table_min_width() {
    // 2e.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;min-width:400px'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td></tr></table>",
        bx(50.0, 20.0)
    ));
    assert_eq!(abs(&d, "t").2, 400.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 400.0, 20.0));
}

#[test]
fn a_cells_width_and_max_width_together() {
    // 2j2: `width: 300px; max-width: 300px` in a 500px table is 500.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:500px'><tr>\
         <td id=a style='padding:0;width:300px;max-width:300px'>{}</td></tr></table>",
        bx(50.0, 20.0)
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 500.0, 20.0));
    // A `width` over the `max-width` asks the max-width: 120 and 100 share
    // 500 by weight.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:500px'><tr>\
         <td id=a style='padding:0;width:300px;max-width:120px'>{}</td>\
         <td id=b style='padding:0;width:100px'>{}</td></tr></table>",
        bx(50.0, 20.0),
        bx(50.0, 20.0)
    ));
    // Chrome: a 272.719, b 227.281.
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 273.0, 20.0));
    assert_eq!(abs(&d, "b"), (273.0, 0.0, 227.0, 20.0));
    // And in an auto table the column is the max-width.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0'><tr>\
         <td id=a style='padding:0;width:300px;max-width:120px'>{}</td>\
         <td id=b style='padding:0'>{}</td></tr></table>",
        bx(50.0, 20.0),
        bx(50.0, 20.0)
    ));
    assert_eq!(abs(&d, "t").2, 170.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 120.0, 20.0));
    assert_eq!(abs(&d, "b"), (120.0, 0.0, 50.0, 20.0));
}

#[test]
fn the_cells_minimum_is_capped_too() {
    // 2l2: the 300px box asks min(300, 150) of a 100px table. Chrome widens
    // the table to 150; rinch keeps it at 100 and the column overflows it
    // (#2266, any cell wider than its table's `width`). The cell is 150.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:100px'><tr>\
         <td id=a style='padding:0;max-width:150px'>{}</td></tr></table>",
        bx(300.0, 20.0)
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 150.0, 20.0));
}

#[test]
fn a_content_box_max_width_caps_the_border_box_at_it_plus_padding() {
    let d = doc(&format!(
        "<table id=t style='border-spacing:0'><tr>\
         <td id=a style='padding:0 7px;max-width:90px'>{}</td>\
         <td id=b style='padding:0'>{}</td></tr></table>",
        bx(200.0, 20.0),
        bx(30.0, 20.0)
    ));
    assert_eq!(abs(&d, "t").2, 134.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 104.0, 20.0));
    assert_eq!(abs(&d, "b"), (104.0, 0.0, 30.0, 20.0));
}

#[test]
fn a_mails_capped_cell_holds_two_cards_to_a_row() {
    // A newsletter writes `width:540px; max-width:540px` on the one cell of a
    // 665px table, around two 260px inline-block cards and a 20px spacer. In
    // Chrome the cell is 665 and the cards share a line; held to 540, the
    // second card wrapped.
    let card = "display:inline-block;width:260px;height:30px;vertical-align:top";
    let d = doc(&format!(
        "<table id=t style='width:665px;border-spacing:0'><tr>\
         <td id=c width=540 style='width:540px;max-width:540px;padding:0;font-size:0'>\
         <div id=a style='{card}'></div>\
         <div style='display:inline-block;width:20px;height:0'></div>\
         <div id=b style='{card}'></div></td></tr></table>"
    ));
    assert_eq!(abs(&d, "c"), (0.0, 0.0, 665.0, 30.0));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 260.0, 30.0));
    assert_eq!(abs(&d, "b"), (280.0, 0.0, 260.0, 30.0));
}

#[test]
fn the_row_is_as_tall_as_the_cell_at_its_columns_width() {
    // Two 70px boxes share a line in the 300px cell; at its 100px max-width
    // they would take two. The row is measured at the stretched width.
    let d = doc("<table id=t style='border-spacing:0;width:300px'><tr>\
         <td id=a style='padding:0;max-width:100px;font-size:0;line-height:0'>\
         <div id=x style='display:inline-block;width:70px;height:20px'></div>\
         <div id=y style='display:inline-block;width:70px;height:20px'></div></td></tr></table>");
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 300.0, 20.0));
    assert_eq!(abs(&d, "y"), (70.0, 0.0, 70.0, 20.0));
}

#[test]
fn nested_tables_each_answer_for_their_own_cells() {
    // An auto table in a sized table's cell: its capped cell is 90.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:400px'><tr><td id=o style='padding:0'>\
         <table id=i style='border-spacing:0'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td></tr></table></td></tr></table>",
        bx(200.0, 20.0)
    ));
    assert_eq!(abs(&d, "o"), (0.0, 0.0, 400.0, 20.0));
    assert_eq!(abs(&d, "i"), (0.0, 0.0, 90.0, 20.0));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 90.0, 20.0));
    // A sized table in an auto table's capped cell: the inner cell fills the
    // inner 300px table past its own 90, and the inner table overflows the
    // outer cell, which keeps its 150.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0'><tr><td id=o style='padding:0;max-width:150px'>\
         <table id=i style='border-spacing:0;width:300px'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td></tr></table></td></tr></table>",
        bx(50.0, 20.0)
    ));
    assert_eq!(abs(&d, "t").2, 150.0);
    assert_eq!(abs(&d, "o"), (0.0, 0.0, 150.0, 20.0));
    assert_eq!(abs(&d, "i"), (0.0, 0.0, 300.0, 20.0));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 300.0, 20.0));
}

#[test]
fn a_restyle_between_sized_and_auto_tables_keeps_the_rule() {
    // Sized (2b's shape, a 200px box): the cell is the table's 400. Auto (2a):
    // the cap holds, 90. Sized again: 400. Each viewport differs, so each
    // resolve lays out.
    let html = format!(
        "<table id=t style='border-spacing:0;width:400px'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td></tr></table>",
        bx(200.0, 20.0)
    );
    let mut d = doc(&html);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 400.0, 20.0));
    let t = d.query_selector("#t").unwrap();
    d.set_attribute(t, "style", "border-spacing:0");
    d.resolve_layout(601.0, 800.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 90.0, 20.0), "sized -> auto");
    d.set_attribute(t, "style", "border-spacing:0;width:400px");
    d.resolve_layout(602.0, 800.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 400.0, 20.0), "auto -> sized");
}

#[test]
fn a_restyle_of_the_cells_max_width_drops_and_restores_the_cap() {
    let mut d = doc(&format!(
        "<table id=t style='border-spacing:0'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td>\
         <td id=b style='padding:0'>{}</td></tr></table>",
        bx(200.0, 20.0),
        bx(30.0, 20.0)
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 90.0, 20.0));
    let a = d.query_selector("#a").unwrap();
    d.set_attribute(a, "style", "padding:0");
    d.resolve_layout(601.0, 800.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 200.0, 20.0), "cap dropped");
    assert_eq!(abs(&d, "b").0, 200.0);
    d.set_attribute(a, "style", "padding:0;max-width:110px");
    d.resolve_layout(602.0, 800.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 110.0, 20.0), "cap restored");
    assert_eq!(abs(&d, "b").0, 110.0);
}

#[test]
fn a_fixed_table_lays_a_capped_cell_out_at_its_column() {
    // An auto column takes what the 100px one leaves, 300, past the cap.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:400px;table-layout:fixed'><tr>\
         <td id=a style='padding:0;max-width:90px'>{}</td>\
         <td id=b style='padding:0;width:100px'>{}</td></tr></table>",
        bx(50.0, 20.0),
        bx(50.0, 20.0)
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 300.0, 20.0));
    assert_eq!(abs(&d, "b"), (300.0, 0.0, 100.0, 20.0));
    // And a first-row `width` over the `max-width` sets the column at the
    // max-width.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:400px;table-layout:fixed'><tr>\
         <td id=a style='padding:0;width:300px;max-width:120px'>{}</td>\
         <td id=b style='padding:0'>{}</td></tr></table>",
        bx(50.0, 20.0),
        bx(50.0, 20.0)
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 120.0, 20.0));
    assert_eq!(abs(&d, "b"), (120.0, 0.0, 280.0, 20.0));
}

/// Six 40x20 inline-blocks, laid two to a line at 90px, six at 400.
fn six() -> String {
    "<div style='display:inline-block;width:40px;height:20px'></div>".repeat(6)
}

#[test]
fn a_td_displayed_as_a_block_keeps_its_max_width() {
    // Responsive email stacks its columns with `display: block` on a `td`:
    // it is then a block in an anonymous cell, and its `max-width` clamps
    // it as any block's does. Chrome 156: 90x60, its boxes two to a line.
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:400px'><tr>\
         <td id=a style='padding:0;max-width:90px;display:block;font-size:0;line-height:0'>{}</td>\
         </tr></table>",
        six()
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 90.0, 60.0));
}

#[test]
fn a_td_displayed_as_a_flex_box_keeps_its_max_width() {
    let item = "<div style='width:40px;height:20px;flex:none'></div>".repeat(6);
    let d = doc(&format!(
        "<table id=t style='border-spacing:0;width:400px'><tr>\
         <td id=a style='padding:0;max-width:90px;display:flex;flex-wrap:wrap'>{item}</td>\
         </tr></table>"
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 90.0, 60.0));
}

#[test]
fn a_restyle_between_a_cell_and_a_block_td_keeps_each_rule() {
    // A cell stretches past its max-width (400x20); the block it becomes is
    // clamped (90x60); the cell again stretches.
    let cell = "padding:0;max-width:90px;font-size:0;line-height:0";
    let mut d = doc(&format!(
        "<table id=t style='border-spacing:0;width:400px'><tr>\
         <td id=a style='{cell}'>{}</td></tr></table>",
        six()
    ));
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 400.0, 20.0));
    let a = d.query_selector("#a").unwrap();
    d.set_attribute(a, "style", &format!("{cell};display:block"));
    d.resolve_layout(601.0, 800.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 90.0, 60.0), "cell -> block");
    d.set_attribute(a, "style", cell);
    d.resolve_layout(602.0, 800.0);
    assert_eq!(abs(&d, "a"), (0.0, 0.0, 400.0, 20.0), "block -> cell");
}
