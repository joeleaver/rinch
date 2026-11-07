//! #2306, #2281, #2266 — how an automatic-layout table shares its width
//! between its columns, measured against Chrome 156 (Playwright,
//! `getBoundingClientRect`) on the same markup.
//!
//! - #2306: a table with a width below its cells' declared column widths
//!   gives each column its min-content width plus the same fraction of its
//!   declared width (padding and border included) minus its min-content.
//!   rinch shared the width in proportion to the declared widths.
//! - #2281 (1): a table with a width below its columns' max-content does the
//!   same between min-content and max-content. rinch shared it in proportion
//!   to max-content and froze a column at its minimum.
//! - #2281 (2): a `colspan` cell's min-content past its columns' max-content
//!   goes to them in proportion to their max-content. rinch gave equal
//!   shares past their growth limits.
//! - #2266: a table's declared width widens to its columns' minimum.
//!
//! Above the guesses (the table wider than every column's max-content),
//! the excess goes to the `auto` columns in proportion to their max-content,
//! or with none, to the declared ones in proportion to their widths: pinned
//! here as positive controls.
//!
//! No text: a cell holds `n` 40px inline-blocks at `font-size: 0` (min-content
//! 40, max-content `40 n`). Every fixture has more than one column, padding
//! and `border-spacing` that are not zero, so no expected width sits on a
//! fixed point of the old algorithm.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (1000.0, 800.0);
const CSS: &str = "body { margin: 0 } table { line-height: 0 } \
    i { display: inline-block; width: 40px; height: 10px; vertical-align: top }";

/// `n` blocks.
fn b(n: usize) -> String {
    vec!["<i></i>"; n].join(" ")
}

fn layout(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(VP.0, VP.1);
    d
}

/// The widths of every `td` of `#t`, in order, the table's width and height.
fn measure(d: &RinchDocument) -> (Vec<f32>, f32, f32) {
    let t = d.query_selector("#t").unwrap();
    let w = d
        .query_selector_all("#t td")
        .iter()
        .map(|n| d.tree.nodes[n.0].layout.width)
        .collect();
    let l = &d.tree.nodes[t.0].layout;
    (w, l.width, l.height)
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
fn a_table_narrower_than_its_declared_columns_interpolates_from_min_content_issue_2306() {
    // Spacing 6: 576 for three columns of min 14 / 100 / 50 (padding, and a
    // block in two) and declared 404 / 365 / 130 (width plus padding):
    // r = 412 / 735. In proportion to the declared widths gave 259/234/83.
    let d = layout(&format!(
        "<table id=t style=\"width:600px;border-spacing:6px;font-size:0\"><tr>\
         <td width=390 style=\"padding:0 7px\"><div style=\"height:10px\"></div></td>\
         <td width=305 style=\"padding:0 30px\">{}</td>\
         <td width=120 style=\"padding:0 5px\">{}</td></tr></table>",
        b(4),
        b(1)
    ));
    let (w, tw, _) = measure(&d);
    assert_near("columns", &w, &[232.61, 248.53, 94.86]);
    assert_eq!(tw, 600.0);
}

#[test]
fn the_mail_033_shape_a_picture_column_beside_a_padded_text_column_issue_2306() {
    // A 696px table: a `width=390` column holding a box whose height is 60%
    // of its width (a scaled picture), beside a `width=305` text column with
    // 30px side padding. Chrome makes them 354.86 / 341.14, so the text
    // column fits seven blocks a line (3 lines) and the picture is 212.9
    // tall; shares by declared width gave 390.6 / 305.4, six a line, and a
    // 234px picture.
    let d = layout(&format!(
        "<table id=t style=\"width:696px;border-spacing:0\"><tr>\
         <td width=390 style=\"padding:0\"><div style=\"width:100%;padding-bottom:60%\"></div></td>\
         <td width=305 style=\"padding:20px 30px;font-size:0\">{}</td></tr></table>",
        b(17)
    ));
    let (w, _, h) = measure(&d);
    assert_near(
        &format!("columns (table height {h})"),
        &w,
        &[354.86, 341.14],
    );
    assert!((h - 212.9).abs() <= 1.0, "table height {h}, Chrome 212.9");
}

#[test]
fn a_table_with_a_width_below_its_max_content_interpolates_from_min_content_issue_2281() {
    // Spacing 4: 580 for four columns of min 46 and max 806 / 206 / 86 / 406:
    // r = 396 / 1320. Shares by max-content froze the third at its minimum.
    let d = layout(&format!(
        "<table id=t style=\"width:600px;border-spacing:4px;font-size:0\"><tr>\
         <td style=\"padding:0 3px\">{}</td><td style=\"padding:0 3px\">{}</td>\
         <td style=\"padding:0 3px\">{}</td><td style=\"padding:0 3px\">{}</td>\
         </tr></table>",
        b(20),
        b(5),
        b(2),
        b(10)
    ));
    let (w, _, h) = measure(&d);
    assert_near(
        &format!("columns (table height {h})"),
        &w,
        &[274.0, 94.0, 58.0, 154.0],
    );
    assert_eq!(h, 48.0, "four lines of the first column, as in Chrome");
}

#[test]
fn a_declared_column_interpolates_before_the_auto_columns_grow_issue_2281() {
    // Below the specified guess (min 138, guess 248, room 234): the declared
    // column grows from its min-content towards 156, the auto ones stay at
    // their minimum.
    let row = format!(
        "<tr><td width=150 style=\"padding:0 3px\">{}</td>\
         <td style=\"padding:0 3px\">{}</td><td style=\"padding:0 3px\">{}</td></tr>",
        b(1),
        b(6),
        b(3)
    );
    let d = layout(&format!(
        "<table id=t style=\"width:250px;border-spacing:4px;font-size:0\">{row}</table>"
    ));
    assert_near(
        "below the specified guess",
        &measure(&d).0,
        &[142.0, 46.0, 46.0],
    );
    // Between the specified and the max guess (room 384): the declared one
    // at 156, the auto ones from min-content towards max-content.
    let d = layout(&format!(
        "<table id=t style=\"width:400px;border-spacing:4px;font-size:0\">{row}</table>"
    ));
    assert_near("past it", &measure(&d).0, &[156.0, 143.14, 84.86]);
}

#[test]
fn past_max_content_the_excess_goes_to_the_auto_columns_by_their_max_content() {
    // Positive control: 684 for a declared 106 and auto max 126 / 86.
    let d = layout(&format!(
        "<table id=t style=\"width:700px;border-spacing:4px;font-size:0\"><tr>\
         <td width=100 style=\"padding:0 3px\">{}</td>\
         <td style=\"padding:0 3px\">{}</td><td style=\"padding:0 3px\">{}</td></tr></table>",
        b(1),
        b(3),
        b(2)
    ));
    assert_near("columns", &measure(&d).0, &[106.0, 343.52, 234.48]);
}

#[test]
fn past_the_declared_widths_of_a_table_of_declared_columns_the_excess_goes_by_width() {
    // Positive control: 684 for declared 106 / 206 / 56.
    let d = layout(&format!(
        "<table id=t style=\"width:700px;border-spacing:4px;font-size:0\"><tr>\
         <td width=100 style=\"padding:0 3px\">{0}</td>\
         <td width=200 style=\"padding:0 3px\">{0}</td>\
         <td width=50 style=\"padding:0 3px\">{0}</td></tr></table>",
        b(1)
    ));
    assert_near("columns", &measure(&d).0, &[197.02, 382.89, 104.09]);
}

#[test]
fn a_colspan_cells_min_content_goes_to_its_columns_by_their_max_content_issue_2281() {
    // Columns of min 44 / 44 and max 124 / 44 (2px side padding), under a
    // colspan of one unbreakable 300px box (295 past the 5px spacing): the
    // 127 past their max-content goes 124:44. Equal shares gave 186 / 106.
    let d = layout(&format!(
        "<div style=\"width:600px\"><table id=t style=\"border-spacing:5px;font-size:0\">\
         <tr><td style=\"padding:0 2px\">{}</td><td style=\"padding:0 2px\">{}</td></tr>\
         <tr><td colspan=2 style=\"padding:0\"><i style=\"width:300px\"></i></td></tr>\
         </table></div>",
        b(3),
        b(1)
    ));
    let (w, tw, _) = measure(&d);
    assert_near("columns", &w, &[217.73, 77.27, 300.0]);
    assert_eq!(tw, 310.0);
}

#[test]
fn a_tables_declared_width_widens_to_its_columns_minimum_issue_2266() {
    // Columns of min 308 and 58 with 3px spacing: 375, past the 100 declared.
    let d = layout(
        "<table id=t style=\"border-spacing:3px;width:100px\"><tr>\
         <td style=\"padding:0 4px\"><div style=\"width:300px;height:20px\"></div></td>\
         <td style=\"padding:0 4px\"><div style=\"width:50px;height:20px\"></div></td>\
         </tr></table>",
    );
    let (w, tw, _) = measure(&d);
    assert_near("columns", &w, &[308.0, 58.0]);
    assert_eq!(tw, 375.0, "the table widens to its columns, as in Chrome");
}

#[test]
fn a_fixed_table_keeps_its_declared_width_issue_2266() {
    // The rule is automatic layout's: `table-layout: fixed` keeps 100.
    let d = layout(
        "<table id=t style=\"border-spacing:3px;width:100px;table-layout:fixed\"><tr>\
         <td style=\"padding:0 4px\"><div style=\"width:300px;height:20px\"></div></td>\
         </tr></table>",
    );
    assert_eq!(measure(&d).1, 100.0);
}

#[test]
fn a_widened_table_is_centred_by_its_widened_width_issue_2266() {
    // `margin: 0 auto` in a 600px block: Chrome 156 puts the 300px table at
    // 150, centred by the width it widened to, not the 100 it declares.
    let d = layout(
        "<div style=\"width:600px\"><table id=t style=\"border-spacing:0;width:100px;\
         margin:0 auto\"><tr><td style=\"padding:0\"><div style=\"width:300px;height:20px\">\
         </div></td></tr></table></div>",
    );
    let t = d.query_selector("#t").unwrap();
    let l = &d.tree.nodes[t.0].layout;
    assert_eq!((l.x, l.width), (150.0, 300.0));
}

#[test]
fn a_declared_columns_excess_share_is_by_its_width_not_another_cells_max_content() {
    // The first column declares 50 (56 with padding) on an empty cell, and a
    // cell below it holds five blocks (max-content 206). Chrome 156 shares
    // the 388 past the spacing 56:66 — by the declared widths — not by the
    // 206 a max-content ask would give it.
    let d = layout(&format!(
        "<table id=t style=\"width:400px;border-spacing:4px;font-size:0\">\
         <tr><td width=50 style=\"padding:0 3px\"></td><td width=60 style=\"padding:0 3px\">{0}</td></tr>\
         <tr><td style=\"padding:0 3px\">{1}</td><td style=\"padding:0 3px\">{0}</td></tr></table>",
        b(1),
        b(5)
    ));
    let w = measure(&d).0;
    assert_near("columns", &w[..2], &[178.09, 209.91]);
}

#[test]
fn a_scroll_container_cell_keeps_its_min_content_in_a_table_with_a_width() {
    // An `overflow: hidden` cell's min-content (156: a block, then a 150px
    // one) still floors its column, as in Chrome 156: min 156 / 46, max
    // 196 / 246, into 292. A grid item that is a scroll container has an
    // automatic minimum of 0, which an `auto` track would give it.
    let d = layout(&format!(
        "<table id=t style=\"width:300px;border-spacing:4px;font-size:0\"><tr>\
         <td style=\"padding:0 3px;overflow:hidden\">{}<i style=\"width:150px\"></i></td>\
         <td style=\"padding:0 3px\">{}</td></tr></table>",
        b(1),
        b(6)
    ));
    assert_near("columns", &measure(&d).0, &[170.33, 117.67]);
}

#[test]
fn a_colspan_min_content_between_its_columns_min_and_max_grows_them_by_their_room() {
    // Columns of min 44 / 44 and max 124 / 84 under a 150px unbreakable
    // colspan (145 past the spacing), in a 160px room that holds the table at
    // its minimum: the 57 the span needs past their min-content goes 80:40,
    // their rooms to grow. Equal shares gave 72.5 / 72.5.
    let d = layout(&format!(
        "<div style=\"width:160px\"><table id=t style=\"border-spacing:5px;font-size:0\">\
         <tr><td style=\"padding:0 2px\">{}</td><td style=\"padding:0 2px\">{}</td></tr>\
         <tr><td colspan=2 style=\"padding:0\"><i style=\"width:150px\"></i></td></tr>\
         </table></div>",
        b(3),
        b(2)
    ));
    let (w, tw, _) = measure(&d);
    assert_near("columns", &w[..2], &[82.0, 63.0]);
    assert_eq!(tw, 160.0);
}

/// One row of `tds` (each `(attributes, blocks)`) in a table `width` px wide
/// with 4px spacing, laid out, and the row's widths.
fn row_widths(width: u32, tds: &[(&str, usize)]) -> Vec<f32> {
    let cells: String = tds
        .iter()
        .map(|(a, n)| format!("<td {a}>{}</td>", b(*n)))
        .collect();
    let d = layout(&format!(
        "<table id=t style=\"width:{width}px;border-spacing:4px;font-size:0\"><tr>{cells}</tr></table>"
    ));
    measure(&d).0
}

#[test]
fn empty_auto_columns_share_the_excess_equally_before_a_declared_one() {
    // Email's logo-and-filler row: a `width=100` cell beside an empty one.
    // Chrome 156 gives the empty auto column the excess (108 / 280); the
    // first cut of #2306 gave it to the declared column (388 / 0).
    let decl = "width=100 style=\"padding:0 4px\"";
    let empty = "style=\"padding:0\"";
    assert_near(
        "one filler",
        &row_widths(400, &[(decl, 1), (empty, 0)]),
        &[108.0, 280.0],
    );
    assert_near(
        "two fillers",
        &row_widths(400, &[(decl, 1), (empty, 0), (empty, 0)]),
        &[108.0, 138.0, 138.0],
    );
    // Before a percentage column too.
    assert_near(
        "beside a percentage",
        &row_widths(400, &[("style=\"width:20%;padding:0 4px\"", 1), (empty, 0)]),
        &[77.59, 310.41],
    );
    // An empty auto column beside one with content gets none of it.
    assert_near(
        "beside content",
        &row_widths(400, &[("style=\"padding:0 4px\"", 1), (empty, 0)]),
        &[388.0, 0.0],
    );
    // Over two rows, the second empty too: 126 / 368 in Chrome.
    let d = layout(&format!(
        "<table id=t style=\"width:500px;border-spacing:2px;font-size:0\">\
         <tr><td width=120 style=\"padding:0 3px\">{0}</td><td style=\"padding:0\"></td></tr>\
         <tr><td style=\"padding:0 3px\">{0}</td><td style=\"padding:0\"></td></tr></table>",
        b(1)
    ));
    assert_near("two rows", &measure(&d).0[..2], &[126.0, 368.0]);
}

#[test]
fn the_excess_goes_to_a_declared_column_before_a_percentage_one() {
    // A `width:20%` cell stops at 20% of the table's room; the `width=100`
    // one takes the rest. Chrome 156: 157.59 / 630.41 (the tiers swapped
    // give 678 / 110).
    assert_near(
        "columns",
        &row_widths(
            800,
            &[
                ("style=\"width:20%;padding:0 5px\"", 1),
                ("width=100 style=\"padding:0 5px\"", 1),
            ],
        ),
        &[157.59, 630.41],
    );
}

#[test]
fn percentage_columns_share_the_excess_by_their_percentages() {
    // 20% and 30% in an 800px table: Chrome 156 gives 315.19 / 472.81, the
    // excess past the percentages going 2:3. As `auto` tracks they would
    // share it by max-content, equally (394 / 394).
    assert_near(
        "columns",
        &row_widths(
            800,
            &[
                ("style=\"width:20%;padding:0 5px\"", 1),
                ("style=\"width:30%;padding:0 5px\"", 1),
            ],
        ),
        &[315.19, 472.81],
    );
}
