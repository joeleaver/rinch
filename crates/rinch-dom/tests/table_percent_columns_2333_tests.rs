//! #2333, #2334 — a table's percentage columns, measured against Chrome 156
//! (headless, `getBoundingClientRect`) on the same markup.
//!
//! - #2333: a percentage column resolves against the table's width less
//!   **every** `border-spacing` (n + 1 of them for n columns). rinch's grid
//!   took the percentage of its content box, the gaps between the columns
//!   included, so only the two outer spacings (its padding) were left out.
//! - #2334: percentages adding up past 100% are capped in an automatic-layout
//!   table, each in column order to what the earlier ones left of 100%.
//!   (A `table-layout: fixed` table is not covered: Blink shares its width
//!   in a way rinch's fixed layout does not model.)
//!
//! No text: a cell holds 40px inline-blocks at `font-size: 0`. Every fixture
//! has a `border-spacing` that is not zero, so the two bases differ.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (1000.0, 800.0);
const CSS: &str = "body { margin: 0 } table { line-height: 0 } \
    i { display: inline-block; width: 40px; height: 10px; vertical-align: top }";

/// `n` blocks.
fn b(n: usize) -> String {
    vec!["<i></i>"; n].join("")
}

fn layout(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(VP.0, VP.1);
    d
}

/// The widths of every `td` of `#t`, in order.
fn widths(d: &RinchDocument) -> Vec<f32> {
    d.query_selector_all("#t td")
        .iter()
        .map(|n| d.tree.nodes[n.0].layout.width)
        .collect()
}

/// Each width within 1px of Chrome's (Taffy rounds each edge to the pixel).
fn assert_near(what: &str, got: &[f32], chrome: &[f32]) {
    assert_eq!(got.len(), chrome.len(), "{what}: rinch {got:?}");
    for (g, c) in got.iter().zip(chrome) {
        assert!(
            (g - c).abs() <= 1.0,
            "{what}: rinch {got:?}, Chrome {chrome:?}"
        );
    }
}

#[test]
fn a_percentage_column_resolves_against_the_width_less_every_spacing_issue_2333() {
    // 300 - 5 x 4 = 280 for the columns: 30% is 84. Less the outer spacing
    // only (292, as a grid's content box) it was 88.
    let d = layout(&format!(
        "<table id=t style=\"width:300px;border-spacing:4px;font-size:0\"><tr>\
         <td style=\"width:30%;padding:0 6px\">{}</td>\
         <td width=150 style=\"padding:0 9px\">{}</td>\
         <td style=\"padding:0 3px\">{}</td>\
         <td style=\"padding:0 2px\">{}</td></tr></table>",
        b(3),
        b(2),
        b(6),
        b(1)
    ));
    assert_near("issue's table", &widths(&d), &[84.0, 106.0, 46.0, 44.0]);
}

#[test]
fn an_email_table_of_percentage_columns_with_cellspacing_issue_2333() {
    // A `width="100%"` table in a 640px column, `cellspacing=8`: 640 - 32 =
    // 608, so 25% is 152 and 50% is 304 (the grid's basis, 624, gave 156
    // and 312).
    let d = layout(&format!(
        "<div style=\"width:640px\"><table id=t width=\"100%\" cellspacing=8 cellpadding=0 \
         style=\"font-size:0\"><tr>\
         <td width=\"25%\">{}</td><td width=\"50%\">{}</td><td>{}</td></tr></table></div>",
        b(1),
        b(2),
        b(1)
    ));
    assert_near("email table", &widths(&d), &[152.0, 304.0, 152.0]);
}

#[test]
fn percentage_columns_adding_up_to_100_share_every_spacing_issue_2333() {
    // 500 - 4 x 5 = 480: 40 / 35 / 25% are 192 / 168 / 120. A positive
    // control: percentages adding up to 100% share the whole width
    // whatever their basis, so this was right before #2333 too.
    let d = layout(&format!(
        "<table id=t style=\"width:500px;border-spacing:5px;font-size:0\"><tr>\
         <td style=\"width:40%;padding:0 3px\">{0}</td>\
         <td style=\"width:35%;padding:0 3px\">{0}</td>\
         <td style=\"width:25%;padding:0 3px\">{0}</td></tr></table>",
        b(1)
    ));
    assert_near("40/35/25", &widths(&d), &[192.0, 168.0, 120.0]);
}

#[test]
fn a_percentage_column_beside_a_colspan_and_unequal_spacings_issue_2333() {
    // Horizontal spacing 10, three columns: 400 - 40 = 360, so 30% is 108
    // and 20% is 72.
    let d = layout(&format!(
        "<table id=t style=\"width:400px;border-spacing:10px 2px;font-size:0\">\
         <tr><td colspan=2 style=\"padding:0\">{0}</td>\
         <td style=\"width:20%;padding:0\">{0}</td></tr>\
         <tr><td style=\"width:30%;padding:0\">{0}</td><td style=\"padding:0\">{0}</td>\
         <td style=\"padding:0\">{0}</td></tr></table>",
        b(1)
    ));
    assert_near("colspan", &widths(&d), &[298.0, 72.0, 108.0, 180.0, 72.0]);
}

#[test]
fn a_percentage_column_in_a_table_with_padding_and_border_issue_2333() {
    // 300 - 6 border - 14 padding - 12 spacing = 268: 50% is 134.
    let d = layout(&format!(
        "<table id=t style=\"width:300px;border-spacing:4px;font-size:0;\
         border:3px solid;padding:7px\"><tr>\
         <td style=\"width:50%;padding:0 3px\">{0}</td>\
         <td style=\"padding:0 3px\">{0}</td></tr></table>",
        b(1)
    ));
    assert_near("padded", &widths(&d), &[134.0, 134.0]);
}

#[test]
fn percentages_past_100_are_capped_in_column_order_issue_2334() {
    // 70% and 60% resolve as 70 and 30% of 584: the issue's table. From the
    // min guess (46 each) to the percentage guess (408.8, 175.2, 46) for
    // 584: 374.88, 163.13, 46. Uncapped it was 288 / 250 / 46.
    let d = layout(&format!(
        "<table id=t style=\"width:600px;border-spacing:4px;font-size:0\"><tr>\
         <td style=\"width:70%;padding:0 3px\">{0}</td>\
         <td style=\"width:60%;padding:0 3px\">{0}</td>\
         <td style=\"padding:0 3px\">{1}</td></tr></table>",
        b(1),
        b(2)
    ));
    assert_near("70/60", &widths(&d), &[374.88, 163.13, 46.0]);
}

#[test]
fn a_third_percentage_past_100_is_left_with_nothing_issue_2334() {
    // 60 / 50 / 30% of 685 are 60 / 40 / 0%: the third column is its
    // min-content, as the auto one.
    let d = layout(&format!(
        "<table id=t style=\"width:700px;border-spacing:3px;font-size:0\"><tr>\
         <td style=\"width:60%;padding:0 3px\">{0}</td>\
         <td style=\"width:50%;padding:0 3px\">{0}</td>\
         <td style=\"width:30%;padding:0 3px\">{0}</td>\
         <td style=\"padding:0 3px\">{0}</td></tr></table>",
        b(1)
    ));
    assert_near("60/50/30", &widths(&d), &[354.36, 238.63, 46.02, 46.0]);
}
