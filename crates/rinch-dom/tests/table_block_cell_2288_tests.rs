//! A `<td style="display: block">` in a row of real cells (#2288). CSS wraps
//! the block in an anonymous table cell (CSS 2.1 §17.2.1), and the column it
//! sits in is that anonymous cell's: its width is `auto`, whatever the
//! block's own `width` or the `<td>`'s `width` attribute says. So in a table
//! wider than its columns, the free width goes to that auto column, not to
//! the columns of real cells with a declared width. Email headers rely on it:
//! a logo cell, a `display: block` spacer, then icon cells of `width=32`,
//! which Chrome draws at the right edge.
//!
//! Every expected number is Chrome 156's for the same markup, laid out at
//! 720x800 with the CSS below. The boxes are sized divs, so no number here
//! depends on the host's fonts.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CSS: &str = "body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; } \
                   td { padding: 0; }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(720.0, 800.0);
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

fn bx(id: &str, w: f32, h: f32) -> String {
    format!("<div id={id} style='width:{w}px;height:{h}px'></div>")
}

#[test]
fn the_free_width_goes_to_the_block_cells_auto_column() {
    let d = doc(&format!(
        "<table id=t width=388 cellspacing=0 cellpadding=0 style='border-collapse:collapse'><tr>\
         <td id=a width=32>{}</td>\
         <td id=b width=15 style='display:block;width:15px'></td>\
         <td id=c style='width:4px'></td>\
         <td id=d width=32 align=right>{}</td>\
         <td id=e width=0 style='display:block;width:0px'></td>\
         <td id=f style='width:4px'></td>\
         <td id=g width=32 align=right>{}</td>\
         <td id=h width=0 style='display:block;width:0px'></td>\
         </tr></table>",
        bx("ai", 32.0, 32.0),
        bx("di", 37.0, 37.0),
        bx("gi", 37.0, 37.0),
    ));
    // Chrome: a 0, c 306, d 310 (37 wide: its box widens the column), f 347,
    // g 351; the table is 388.
    assert_eq!(abs(&d, "t").2, 388.0);
    assert_eq!(abs(&d, "ai").0, 0.0);
    assert_eq!(abs(&d, "c").0, 306.0);
    assert_eq!(abs(&d, "di").0, 310.0);
    assert_eq!(abs(&d, "f").0, 347.0);
    assert_eq!(abs(&d, "gi").0, 351.0);
}

#[test]
fn two_block_columns_share_the_free_width_by_their_widths() {
    // Free 400 - 150 - 60 = 190, shared 15 : 45 between the two anonymous
    // cells' columns: 62.5 and 187.5.
    let d = doc(&format!(
        "<table id=t width=400 cellspacing=0 cellpadding=0><tr>\
         <td width=50>{}</td>\
         <td id=b width=15 style='display:block;width:15px'></td>\
         <td width=50>{}</td>\
         <td id=d style='display:block;width:45px'></td>\
         <td width=50>{}</td>\
         </tr></table>",
        bx("a", 50.0, 20.0),
        bx("c", 50.0, 20.0),
        bx("e", 50.0, 20.0),
    ));
    // Chrome: b 50 (15 wide), c 112.5, d 162.5 (45 wide), e 350; rinch
    // rounds the edges to whole pixels.
    let (bx_, _, bw, _) = abs(&d, "b");
    assert_eq!((bx_, bw), (50.0, 15.0));
    assert!((abs(&d, "c").0 - 112.5).abs() <= 0.5, "{:?}", abs(&d, "c"));
    let (dx, _, dw, _) = abs(&d, "d");
    assert!((dx - 162.5).abs() <= 0.5, "{dx}");
    assert_eq!(dw, 45.0);
    assert_eq!(abs(&d, "e").0, 350.0);
}

#[test]
fn a_spanning_row_adds_no_column_that_takes_the_free_width() {
    // The mail's rule row under the icons spans `colspan=100`; the columns
    // only it reaches take nothing. The block holds text wider than its own
    // 15px, which overflows it and does not widen the column.
    let d = doc(&format!(
        "<table id=t width=400 cellspacing=0 cellpadding=0><tr>\
         <td width=50>{}</td>\
         <td id=b width=15 style='display:block;width:15px'>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;</td>\
         <td width=50>{}</td>\
         </tr><tr><td id=z colspan=100><div style='height:20px'></div></td></tr></table>",
        bx("a", 50.0, 20.0),
        bx("c", 50.0, 20.0),
    ));
    // Chrome: b 50 (15 wide), c 350, z 400 wide.
    let (bx_, _, bw, _) = abs(&d, "b");
    assert_eq!((bx_, bw), (50.0, 15.0));
    assert_eq!(abs(&d, "c").0, 350.0);
    assert_eq!(abs(&d, "z").2, 400.0);
}

#[test]
fn an_auto_table_packs_the_block_at_its_own_width() {
    let d = doc(&format!(
        "<table id=t cellspacing=0 cellpadding=0><tr>\
         <td width=50>{}</td>\
         <td id=b style='display:block;width:15px'></td>\
         <td width=50>{}</td>\
         </tr></table>",
        bx("a", 50.0, 20.0),
        bx("c", 50.0, 20.0),
    ));
    // Chrome: the table is 115, b at 50 (15 wide), c at 65.
    assert_eq!(abs(&d, "t").2, 115.0);
    let (bx_, _, bw, _) = abs(&d, "b");
    assert_eq!((bx_, bw), (50.0, 15.0));
    assert_eq!(abs(&d, "c").0, 65.0);
}

#[test]
fn a_declared_column_is_weighed_by_its_block_when_wider() {
    // Every column declares a width (50, 60), so the 400px table shares its
    // room by weight; the first column's weight is the larger of its width
    // and its max-content, here the 100px block's. Chrome: 250 / 150.
    let d = doc("<table id=t width=400 cellspacing=0 cellpadding=0>\
         <tr><td id=b width=100 style='display:block'></td></tr>\
         <tr><td id=x width=50></td><td id=y width=60></td></tr></table>");
    assert_eq!(abs(&d, "x").2, 250.0);
    assert_eq!(abs(&d, "y").0, 250.0);
    assert_eq!(abs(&d, "y").2, 150.0);
}

#[test]
fn a_declared_column_is_weighed_by_its_content_when_wider() {
    // The same weighing with a real cell: `width=50` holding 100px.
    // Chrome: 250 / 150.
    let d = doc(&format!(
        "<table id=t width=400 cellspacing=0 cellpadding=0><tr>\
         <td id=x width=50>{}</td><td id=y width=60></td></tr></table>",
        bx("xi", 100.0, 10.0)
    ));
    assert_eq!(abs(&d, "x").2, 250.0);
    assert_eq!(abs(&d, "y").0, 250.0);
}

#[test]
fn a_declared_column_is_weighed_by_its_min_content_not_its_max_content() {
    // A `width=50` cell holding three 100px inline-blocks with spaces
    // between them: min-content 100, max-content over 300. Blink weighs a
    // cell with a width by max(width, min-content), here 100 against 60.
    // Chrome: 250 / 150 (by max-content it would be about 334 / 66).
    let ib = "<span style='display:inline-block;width:100px;height:10px'></span>";
    let d = doc(&format!(
        "<table id=t width=400 cellspacing=0 cellpadding=0><tr>\
         <td id=x width=50>{ib} {ib} {ib}</td><td id=y width=60></td></tr></table>"
    ));
    assert_eq!(abs(&d, "x").2, 250.0);
    assert_eq!(abs(&d, "y").0, 250.0);
}
