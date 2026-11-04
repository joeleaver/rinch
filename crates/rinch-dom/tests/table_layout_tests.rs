//! `<table>` layout on Taffy's grid (`table_layout`). Every expected number is
//! Chrome's, for the same markup with declared line boxes.
use rinch_core::dom::{DomDocument, NodeId};
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
fn abs(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    abs_of(d, n)
}
fn abs_of(d: &RinchDocument, n: NodeId) -> (f32, f32, f32, f32) {
    let mut cur = Some(n.0);
    let l = d.tree.get(n.0).unwrap().layout;
    let (mut x, mut y) = (0.0, 0.0);
    while let Some(i) = cur {
        let node = d.tree.get(i).unwrap();
        x += node.layout.x;
        y += node.layout.y;
        cur = node.parent;
    }
    (x, y, l.width, l.height)
}

#[test]
fn cells_form_rows_and_columns_with_spacing() {
    let d = doc("<table id=t style='border-spacing:2px'><tr>
        <td id=a style='padding:0'><div style='width:50px;height:20px'></div></td>
        <td id=b style='padding:0'><div style='width:30px;height:40px'></div></td></tr>
        <tr><td id=c style='padding:0'><div style='width:10px;height:10px'></div></td>
        <td id=e style='padding:0'><div style='width:70px;height:10px'></div></td></tr></table>");
    assert_eq!(abs(&d, "t"), (0.0, 0.0, 126.0, 56.0));
    assert_eq!(abs(&d, "a"), (2.0, 2.0, 50.0, 40.0));
    assert_eq!(abs(&d, "b"), (54.0, 2.0, 70.0, 40.0));
    assert_eq!(abs(&d, "c"), (2.0, 44.0, 50.0, 10.0));
    assert_eq!(abs(&d, "e"), (54.0, 44.0, 70.0, 10.0));
}

#[test]
fn rowspan_and_colspan_take_their_slots() {
    let d = doc("<table style='border-spacing:0'>
        <tr><td id=a rowspan=2 style='padding:0'><div style='width:20px;height:50px'></div></td>
            <td id=b style='padding:0'><div style='width:20px;height:10px'></div></td></tr>
        <tr><td id=c style='padding:0'><div style='width:20px;height:10px'></div></td></tr>
        <tr><td id=e colspan=2 style='padding:0'><div style='height:5px'></div></td></tr></table>");
    let (.., h) = abs(&d, "a");
    assert_eq!(h, 50.0);
    assert_eq!(abs(&d, "b").0, 20.0);
    assert_eq!(abs(&d, "c").0, 20.0);
    assert_eq!(abs(&d, "e"), (0.0, 50.0, 40.0, 5.0));
}

#[test]
fn head_and_foot_order_whatever_the_source() {
    let d = doc("<table style='border-spacing:0'>
        <tfoot><tr><td id=f style='padding:0;height:10px'></td></tr></tfoot>
        <tbody><tr><td id=b style='padding:0;height:10px'></td></tr></tbody>
        <thead><tr><td id=h style='padding:0;height:10px'></td></tr></thead></table>");
    assert_eq!(abs(&d, "h").1, 0.0);
    assert_eq!(abs(&d, "b").1, 10.0);
    assert_eq!(abs(&d, "f").1, 20.0);
}

#[test]
fn a_wide_table_shares_room_by_max_content() {
    // Chrome 153: 300px between 100 and 300 of max-content gives 150 and 450.
    let d = doc("<table style='width:600px;border-spacing:0'><tr>
        <td id=a style='padding:0'><div style='width:100px;height:10px;display:inline-block'></div></td>
        <td id=b style='padding:0'><div style='width:300px;height:10px;display:inline-block'></div></td>
        </tr></table>");
    assert_eq!(abs(&d, "a").2, 150.0);
    assert_eq!(abs(&d, "b").2, 450.0);
}

#[test]
fn vertical_align_middle_moves_content_not_the_box() {
    let d = doc("<table style='border-spacing:0'><tr>
        <td id=a style='padding:0'><div id=k style='width:20px;height:10px'></div></td>
        <td style='padding:0'><div style='width:20px;height:50px'></div></td></tr></table>");
    assert_eq!(abs(&d, "a").3, 50.0);
    assert_eq!(abs(&d, "k").1, 20.0);
}

#[test]
fn collapsed_borders_draw_one_edge() {
    let d = doc("<table id=t style='border-collapse:collapse'><tr>
        <td id=a style='padding:0;border:2px solid red'><div style='width:20px;height:10px'></div></td>
        <td id=b style='padding:0;border:2px solid red'><div style='width:20px;height:10px'></div></td>
        </tr></table>");
    assert_eq!(abs(&d, "t").2, 46.0);
    assert_eq!(abs(&d, "b").0, abs(&d, "a").0 + abs(&d, "a").2);
}
#[test]
fn a_line_of_text_is_centred_in_a_tall_row() {
    let d = doc(
        "<table style='border-spacing:0'><tbody><tr>
        <td id=a style='padding:0'>x</td>
        <td style='padding:0'><div style='width:20px;height:60px'></div></td></tr></tbody></table>",
    );
    assert_eq!(abs(&d, "a").3, 60.0);
    let n = d.query_selector("#a").unwrap();
    assert_eq!(
        d.tree.get(n.0).unwrap().computed_style.padding_top.to_px(),
        20.0
    );
}

#[test]
fn cellpadding_cellspacing_and_bgcolor_map_to_css() {
    let d = doc(
        "<table cellspacing=6 cellpadding=4><tbody><tr bgcolor=#ff0000>
        <td id=a><div style='width:20px;height:10px'></div></td></tr></tbody></table>",
    );
    assert_eq!(abs(&d, "a"), (6.0, 6.0, 28.0, 18.0));
    let n = d.query_selector("#a").unwrap();
    let under = d.tree.get(n.0).unwrap().table_cell.as_ref().unwrap().under;
    assert!(under[1].is_some_and(|c| c.components[0] == 1.0));
}

#[test]
fn a_changed_row_moves_its_cells_and_stays_stable_across_layouts() {
    let mut d = doc("<table style='border-spacing:0'><tbody>
        <tr><td id=a style='padding:0'>x</td><td style='padding:0'><div id=k style='height:60px'></div></td></tr>
        </tbody></table>");
    d.resolve_layout(600.0, 800.0);
    let n = d.query_selector("#a").unwrap();
    assert_eq!(
        d.tree.get(n.0).unwrap().computed_style.padding_top.to_px(),
        20.0
    );
    let k = d.query_selector("#k").unwrap();
    d.set_attribute(k, "style", "height:100px");
    d.resolve_layout(600.0, 800.0);
    assert_eq!(abs(&d, "a").3, 100.0);
    assert_eq!(
        d.tree.get(n.0).unwrap().computed_style.padding_top.to_px(),
        40.0
    );
}

/// A cell's `colspan` or `rowspan` set or removed after the table was laid
/// out takes effect: the table is placed again, as one built with the span
/// from the start is (a review of #2079 found the span read only on a
/// rebuild some other change owed).
#[test]
fn a_span_changed_after_layout_places_the_table_again() {
    const ROWS: &str = "<table id=t style='border-spacing:0'>\
        <tr><td id=a>aaaa</td><td id=b>b</td></tr>\
        <tr><td id=c>c</td><td id=d>dddddddd</td></tr></table>";
    let fresh = doc(&ROWS.replace("<td id=a>", "<td id=a colspan=2>"));
    let mut edited = doc(ROWS);
    let a = edited.query_selector("#a").unwrap();
    edited.set_attribute(a, "colspan", "2");
    edited.resolve_layout(600.0, 800.0);
    assert_eq!(abs(&edited, "a"), abs(&fresh, "a"));
    assert_eq!(abs(&edited, "t"), abs(&fresh, "t"));

    let c = edited.query_selector("#c").unwrap();
    edited.set_attribute(c, "rowspan", "2");
    edited.resolve_layout(600.0, 800.0);
    let fresh = doc(&ROWS
        .replace("<td id=a>", "<td id=a colspan=2>")
        .replace("<td id=c>", "<td id=c rowspan=2>"));
    assert_eq!(abs(&edited, "c"), abs(&fresh, "c"));
    assert_eq!(abs(&edited, "d"), abs(&fresh, "d"));

    edited.remove_attribute(a, "colspan");
    edited.remove_attribute(c, "rowspan");
    edited.resolve_layout(600.0, 800.0);
    let fresh = doc(ROWS);
    for id in ["a", "b", "c", "d", "t"] {
        assert_eq!(abs(&edited, id), abs(&fresh, id), "#{id}");
    }
}

/// An 800x200 PNG.
const WIDE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAyAAAADICAIAAACf7RJNAAADBElEQVR42u3WMQ0AAAgEsZeDJhQjCxNsNKmCmy7VAwDAoUgAAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECAMBgAQAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAACDpQIAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAfLMLtdks6n5QTwAAAABJRU5ErkJggg==";

#[test]
fn a_percentage_width_picture_does_not_widen_its_column() {
    // css-sizing-3 §5.2.2: a replaced element whose `width` or `max-width`
    // is a percentage contributes no min-content width, so the declared
    // columns hold and the picture scales into its cell. Chrome 153: the
    // 300px and 200px columns share the 600px table as 360 and 240, and the
    // picture is 360x90, for `width="100%"` and for `max-width: 100%` alike.
    // rinch used to let the picture's natural 800px widen its column.
    for picture in [
        "width='100%' style='display:block'",
        "style='display:block; max-width:100%'",
    ] {
        let d = doc(&format!(
            "<table style='width:600px;border-spacing:0'><tr>
            <td id=a width=300 style='padding:0'><img id=i {picture} src='{WIDE}'></td>
            <td id=b width=200 style='padding:0'>text</td></tr></table>"
        ));
        assert_eq!(abs(&d, "a").2, 360.0, "{picture}");
        assert_eq!(abs(&d, "i"), (0.0, 0.0, 360.0, 90.0), "{picture}");
        assert_eq!(abs(&d, "b").0, 360.0, "{picture}");
    }
}
