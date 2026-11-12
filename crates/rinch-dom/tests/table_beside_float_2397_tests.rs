//! #2397: a table, or a block-level replaced element, after a float sits
//! beside the float only when its border box fits the room there, and moves
//! down below the float when it does not (CSS 2.2 §9.5: its border box must not
//! overlap the float's margin box).
//!
//! rinch placed an auto-width table beside the float whatever its width, so
//! it overflowed the room past the containing block's edge. The Taffy fork's
//! block layout tried each slot beside the floats with the width an auto width
//! *stretches* to, which always fits; an auto table's width is rinch's
//! `fit-content` (`table_layout::apply_table_style`), at least its min-content
//! width. The fork now measures, in the fit test, the width the box will have
//! in that slot: a sizing keyword's, and a table's or replaced element's own
//! (fork branch `table-beside-float-2397`).
//!
//! # Chrome 156, headless, standards mode
//!
//! Each case is one `<td style="width:300px; padding:0">` of a
//! `cellspacing=0 cellpadding=0` table: a float, then the subject. Columns are
//! the subject's border box relative to the cell (x, y, width) and the cell's
//! height.
//!
//! | case | float | subject | Chrome | rinch before |
//! |---|---|---|---|---|
//! | issue | left 280 | auto table, 40px box | (0, 50, 40), 60 | **(280, 0, 40), 50** |
//! | fits | left 237 | same | (237, 0, 40), 50 | same |
//! | exact | left 260 | same (40px of room) | (260, 0, 40), 50 | same |
//! | justover | left 263 | same | (0, 50, 40), 60 | **(263, 0, 40), 50** |
//! | right | right 280 | same | (0, 50, 40), 60 | **(0, 0, 40), 50** |
//! | rightfits | right 237 | same | (0, 0, 40), 50 | same |
//! | stacked | left 280x30, then left 150x30 cleared | same | (150, 30, 40), 60 | **(280, 0, 40), 60** |
//! | textshrink | left 200 | auto table of seven words | (200, 0, 100), 50 | same |
//! | textmin | left 280 | auto table of one 10-letter word | (0, 50, 66.75), 70 | **(280, 0, 53), 50** |
//! | email | `<table align=left width=100%>` 50 tall | auto table, `&nbsp;` | (0, 50, 3.34), 70 | **(300, 0, 3), 50** |
//! | divfull | `<div float:left width:100%>` 50 tall | same | (0, 50, 3.34), 70 | **(300, 0, 3), 50** |
//! | borderedfits | left 245 | `border=1 cellpadding=5`, 54 wide | (245, 0, 54), 50 | same |
//! | borderedover | left 247 | same | (0, 50, 54), 74 | **(247, 0, 54), 50** |
//! | margin | left 237 | `margin-left: 30px` (overlaps the float) | (237, 0, 40), 50 | same |
//! | minw | left 237 | `min-width: 80px` | (0, 50, 80), 60 | same |
//! | declared | left 280 | `width: 40px` | (0, 50, 40), 60 | same |
//! | pct | left 200 | `width=50%` | (0, 50, 150), 60 | same |
//! | canvas | left 280 | `<canvas 40x10 display:block>` | (0, 50, 40), 60 | **(280, 0, 40), 50** |
//! | maxcontent | left 280 | `overflow:hidden; width:max-content` div | (0, 50, 40), 60 | **(280, 0, 40), 50** |
//! | ofh | left 280 | `overflow:hidden` div, auto width | (280, 0, 20), 50 | same |
//! | flex | left 280 | `display:flex` div, auto width | (280, 0, 20), 50 | same |
//! | declared | left 250 | `width: 50px` (or `width="50"`), 100px box | (0, 50, 100), 60 | **(250, 0, 100), 50** |
//! | pct30 | left 200 | `width: 30%`, 150px box | (0, 50, 150), 60 | **(200, 0, 150), 50** |
//! | fixed50 | left 250 | `width: 50px; table-layout: fixed`, 100px box | (250, 0, 50), 50 | same |
//! | flowroot | left 280 | `display:flow-root` div | (280, 0, 20), 50 | same (#2351) |
//!
//! The `textmin`, `email` and `divfull` widths are the text's, so only their
//! position and the cell's height are asserted (those hold for any font in
//! which the word is wider than 20px and the seven short words fit 100px in
//! some lines).

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const T40: &str = r#"<table class="t" cellspacing="0" cellpadding="0"><tr><td style="padding:0"><div style="width:40px;height:10px"></div></td></tr></table>"#;

fn float(side: &str, width: &str, height: u32) -> String {
    format!(r#"<div style="float:{side};width:{width};height:{height}px"></div>"#)
}

/// Lay one case out and return the subject's `(x, y, width)` relative to the
/// cell, and the cell's height.
fn case(content: &str) -> (f64, f64, f32, f32) {
    let mut doc = RinchDocument::new();
    doc.load_css("body { margin: 0; font-size: 12px; line-height: 20px }");
    let body = doc.body();
    doc.set_inner_html(
        body,
        &format!(
            r#"<table cellspacing="0" cellpadding="0"><tr><td id="cell" style="width:300px;padding:0">{content}</td></tr></table>"#
        ),
    );
    doc.resolve_layout(720.0, 600.0);
    let q = |sel: &str| rinch_dom::query::query_all(&doc.tree, doc.tree.root_id, sel)[0];
    let (cell, t) = (q("#cell"), q(".t"));
    let (cx, cy) = rinch_dom::paint::compute_absolute_position(&doc.tree, cell, 1.0);
    let (tx, ty) = rinch_dom::paint::compute_absolute_position(&doc.tree, t, 1.0);
    (
        tx - cx,
        ty - cy,
        doc.tree.nodes[t].layout.width,
        doc.tree.nodes[cell].layout.height,
    )
}

/// Chrome's `(x, y, width, cell height)`; a width of `None` is the text's,
/// not asserted.
type Expected = (f64, f64, Option<f32>, f32);

fn check(cases: &[(&str, String, Expected)]) {
    let mut wrong = Vec::new();
    for (name, content, chrome) in cases {
        let (x, y, w, h) = case(content);
        if (x, y, chrome.2.is_none_or(|cw| cw == w), h) != (chrome.0, chrome.1, true, chrome.3) {
            let got = (x, y, w, h);
            wrong.push(format!("{name}: rinch {got:?}, Chrome {chrome:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

const TEXT_TABLE: &str = r#"<table class="t" cellspacing="0" cellpadding="0"><tr><td style="padding:0">aaa bbb ccc ddd eee fff ggg</td></tr></table>"#;
const WORD_TABLE: &str = r#"<table class="t" cellspacing="0" cellpadding="0"><tr><td style="padding:0">aaaaaaaaaa</td></tr></table>"#;
const NBSP_TABLE: &str = r#"<table class="t" cellspacing="0" cellpadding="0"><tr><td style="padding:0">&nbsp;</td></tr></table>"#;
const BORDERED: &str = r#"<table class="t" border="1" cellspacing="0" cellpadding="5"><tr><td><div style="width:40px;height:10px"></div></td></tr></table>"#;

fn with_box(table_attrs: &str) -> String {
    format!(
        r#"<table class="t" {table_attrs} cellspacing="0" cellpadding="0"><tr><td style="padding:0"><div style="width:40px;height:10px"></div></td></tr></table>"#
    )
}

/// The issue's shape, and the auto table on both sides of the fit: it moves
/// below a float it does not fit beside, and stays beside one it fits beside,
/// at exactly its width of room too.
#[test]
fn an_auto_table_moves_below_a_float_only_when_it_does_not_fit_beside_it() {
    check(&[
        (
            "issue",
            float("left", "280px", 50) + T40,
            (0.0, 50.0, Some(40.0), 60.0),
        ),
        (
            "fits",
            float("left", "237px", 50) + T40,
            (237.0, 0.0, Some(40.0), 50.0),
        ),
        (
            "exact",
            float("left", "260px", 50) + T40,
            (260.0, 0.0, Some(40.0), 50.0),
        ),
        (
            "justover",
            float("left", "263px", 50) + T40,
            (0.0, 50.0, Some(40.0), 60.0),
        ),
        (
            "right",
            float("right", "280px", 50) + T40,
            (0.0, 50.0, Some(40.0), 60.0),
        ),
        (
            "rightfits",
            float("right", "237px", 50) + T40,
            (0.0, 0.0, Some(40.0), 50.0),
        ),
        (
            "stacked",
            float("left", "280px", 30)
                + r#"<div style="float:left;clear:left;width:150px;height:30px"></div>"#
                + T40,
            (150.0, 30.0, Some(40.0), 60.0),
        ),
    ]);
}

/// The fit is the table's min-content width: one that can shrink to the room
/// stays beside the float at the room's width, one that cannot moves down.
#[test]
fn an_auto_table_of_text_fits_by_its_min_content_width() {
    check(&[
        (
            "textshrink",
            float("left", "200px", 50) + TEXT_TABLE,
            (200.0, 0.0, Some(100.0), 50.0),
        ),
        (
            "textmin",
            float("left", "280px", 50) + WORD_TABLE,
            (0.0, 50.0, None, 70.0),
        ),
    ]);
}

/// The email pattern: a full-width floated logo table (or div), then an auto
/// spacer table. Chrome puts the spacer below the float.
#[test]
fn an_auto_table_after_a_full_width_float_moves_below_it() {
    check(&[
        (
            "email",
            r#"<table align="left" width="100%" cellspacing="0" cellpadding="0"><tr><td style="padding:0;height:50px">logo</td></tr></table>"#.to_string()
                + NBSP_TABLE,
            (0.0, 50.0, None, 70.0),
        ),
        ("divfull", float("left", "100%", 50) + NBSP_TABLE, (0.0, 50.0, None, 70.0)),
    ]);
}

/// The table's border box is what must fit: borders and cell padding count
/// (54 wide), its margins do not (they may overlap the float), and a
/// `min-width` counts.
#[test]
fn a_tables_border_box_is_what_must_fit() {
    check(&[
        (
            "borderedfits",
            float("left", "245px", 50) + BORDERED,
            (245.0, 0.0, Some(54.0), 50.0),
        ),
        (
            "borderedover",
            float("left", "247px", 50) + BORDERED,
            (0.0, 50.0, Some(54.0), 74.0),
        ),
        (
            "margin",
            float("left", "237px", 50) + &with_box(r#"style="margin-left:30px""#),
            (237.0, 0.0, Some(40.0), 50.0),
        ),
        (
            "minw",
            float("left", "237px", 50) + &with_box(r#"style="min-width:80px""#),
            (0.0, 50.0, Some(80.0), 60.0),
        ),
        (
            "declared",
            float("left", "280px", 50) + &with_box(r#"style="width:40px""#),
            (0.0, 50.0, Some(40.0), 60.0),
        ),
        (
            "pct",
            float("left", "200px", 50) + &with_box(r#"width="50%""#),
            (0.0, 50.0, Some(150.0), 60.0),
        ),
    ]);
}

/// The same rule for the other boxes that size themselves: a block-level
/// replaced element and a box with a sizing keyword width move down; a
/// formatting-context root with an auto width is stretched to the room
/// beside the float, however narrow, and stays there (its content overflows).
#[test]
fn self_sized_boxes_move_down_and_stretched_ones_stay_beside() {
    let box40 = r#"<div style="width:40px;height:10px"></div>"#;
    check(&[
        (
            "canvas",
            float("left", "280px", 50)
                + r#"<canvas class="t" width="40" height="10" style="display:block"></canvas>"#,
            (0.0, 50.0, Some(40.0), 60.0),
        ),
        (
            "maxcontent",
            float("left", "280px", 50)
                + &format!(
                    r#"<div class="t" style="overflow:hidden;width:max-content">{box40}</div>"#
                ),
            (0.0, 50.0, Some(40.0), 60.0),
        ),
        (
            "ofh",
            float("left", "280px", 50)
                + &format!(r#"<div class="t" style="overflow:hidden">{box40}</div>"#),
            (280.0, 0.0, Some(20.0), 50.0),
        ),
        (
            "flex",
            float("left", "280px", 50)
                + r#"<div class="t" style="display:flex"><div style="width:40px;height:10px;flex-shrink:0"></div></div>"#,
            (280.0, 0.0, Some(20.0), 50.0),
        ),
        (
            "flowroot",
            float("left", "280px", 50)
                + &format!(r#"<div class="t" style="display:flow-root">{box40}</div>"#),
            (280.0, 0.0, Some(20.0), 50.0),
        ),
    ]);
}

/// A table with a declared width narrower than its columns' minimum (an
/// auto-layout table widens to it, #2266): what must fit is the width it
/// widens to, not its declaration. Chrome 156 moves a `width: 50px` (or
/// `width="50"`) table holding a 100px box below a 250px float, at width 100;
/// a `width: 30%` table holding a 150px box below a 200px float, at 150. A
/// `table-layout: fixed` table keeps its 50 and stays beside. (Review of
/// #2415; rinch put the first three beside the float, overflowing.)
#[test]
fn a_table_narrower_than_its_minimum_fits_by_the_width_it_widens_to() {
    let box_table = |style: &str, attrs: &str, w: u32| {
        format!(
            r#"<table class="t" {attrs} cellspacing="0" cellpadding="0" style="{style}"><tr><td style="padding:0"><div style="width:{w}px;height:10px"></div></td></tr></table>"#
        )
    };
    check(&[
        (
            "declared 50px, min 100",
            float("left", "250px", 50) + &box_table("width:50px", "", 100),
            (0.0, 50.0, Some(100.0), 60.0),
        ),
        (
            "width=50 attribute, min 100",
            float("left", "250px", 50) + &box_table("", r#"width="50""#, 100),
            (0.0, 50.0, Some(100.0), 60.0),
        ),
        (
            "30%, min 150",
            float("left", "200px", 50) + &box_table("width:30%", "", 150),
            (0.0, 50.0, Some(150.0), 60.0),
        ),
        (
            "fixed layout keeps its 50",
            float("left", "250px", 50) + &box_table("width:50px;table-layout:fixed", "", 100),
            (250.0, 0.0, Some(50.0), 50.0),
        ),
    ]);
}
