//! `box-sizing` (#1278): rinch read it nowhere, so every box was sized as a
//! border box whatever its CSS said. The initial value is `content-box`:
//! `width` and `height` size the content, padding and border go outside.
//! Numbers are Chrome 156's for the same markup (body margin 0, 800x600).
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(800.0, 600.0);
    d
}

fn rect(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    let l = d.tree.get(n.0).unwrap().layout;
    let (x, y) = rinch_dom::paint::compute_absolute_position(&d.tree, n.0, 1.0);
    (x as f32, y as f32, l.width, l.height)
}

/// The issue's own table: `content-box` (said or not) adds the padding to
/// the declared height, `border-box` keeps it inside.
#[test]
fn padding_goes_outside_a_content_box_and_inside_a_border_box() {
    let d = doc(
        "<div id=a style='box-sizing:content-box;height:20px;padding-top:15px'>x</div>\
                 <div id=b style='height:20px;padding-top:15px'>x</div>\
                 <div id=c style='box-sizing:border-box;height:20px;padding-top:15px'>x</div>",
    );
    assert_eq!(rect(&d, "a").3, 35.0);
    assert_eq!(rect(&d, "b").3, 35.0);
    assert_eq!(rect(&d, "c").3, 20.0);
}

/// A percentage child of a padded `width:600px` box resolves against 600,
/// the content box (the mail fixture that found this: 48% columns, 288px).
#[test]
fn percentages_resolve_against_a_content_box_width() {
    let d = doc("<div id=o style='width:600px;padding:16px'>\
                 <div id=c style='width:48%;padding:6px;box-sizing:border-box;height:10px'></div></div>");
    assert_eq!(rect(&d, "o").2, 632.0);
    assert_eq!(rect(&d, "c").2, 288.0);
}

/// An auto-width inline-block with padding is as wide as its content plus
/// the padding, once, however often it is laid out: its measured width was
/// written back as a `content-box` width, adding the padding again each time.
#[test]
fn a_padded_inline_block_counts_its_padding_once() {
    let mut d = doc(
        "<div style='width:400px'><span id=o style='display:inline-block;padding:0 10px'>\
                     <span style='display:inline-block;width:100px;height:10px'></span></span></div>",
    );
    assert_eq!(rect(&d, "o").2, 120.0);
    d.resolve_layout(800.0, 600.0);
    assert_eq!(rect(&d, "o").2, 120.0);
}

/// An absolute box between two insets, or `width: stretch`, fills the room
/// with its border box (Chrome 156: 380 between 10px insets with 20px +
/// 20px padding and a 3px border; 400 for `stretch` with 5% padding).
#[test]
fn an_absolute_box_between_insets_fills_the_room_with_its_border_box() {
    let d = doc("<div style='position:relative;width:400px;height:100px'>\
                 <div id=a style='position:absolute;left:10px;right:10px;padding:0 20px;border-left:3px solid;height:10px'></div>\
                 <div id=b style='position:absolute;left:0;width:stretch;padding:0 5%;height:10px;top:20px'></div></div>");
    let (ax, _, aw, _) = rect(&d, "a");
    assert_eq!((ax, aw), (10.0, 380.0));
    let (bx, _, bw, _) = rect(&d, "b");
    assert_eq!((bx, bw), (0.0, 400.0));
}

/// A padded `<body>` fills the viewport with its border box, as an auto-width
/// body does in a browser (Chrome 156: 800).
#[test]
fn a_padded_body_is_as_wide_as_the_viewport() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; padding: 10px; }");
    let body = d.body();
    d.resolve_layout(800.0, 600.0);
    assert_eq!(d.tree.get(body.0).unwrap().layout.width, 800.0);
}

/// An absolute box held by all four insets, or `width`/`height: stretch`,
/// fills its containing block with its border box on both axes, the block
/// being an ancestor further up than its parent (Chrome 156: 300x200 for
/// both, padding 10 and a 5px border; a review of #2090 found 330x230).
#[test]
fn an_absolute_box_filling_a_grandparent_block_fills_it_with_its_border_box() {
    let d = doc(
        "<div style='position:relative;width:300px;height:200px'><div>\
                 <div id=a style='position:absolute;top:0;right:0;bottom:0;left:0;padding:10px;border:5px solid'></div>\
                 <div id=b style='position:absolute;left:0;top:0;width:stretch;height:stretch;padding:10px;border:5px solid'></div>\
                 </div></div>",
    );
    assert_eq!(rect(&d, "a"), (0.0, 0.0, 300.0, 200.0));
    assert_eq!(rect(&d, "b"), (0.0, 0.0, 300.0, 200.0));
}

/// An auto-width `<body>` with padding and a `max-width` is sized as the
/// content box it is: `max-width: 600px` caps its content at 600, its border
/// box at 640 (Chrome 156; a review of #2090 found it forced to border-box,
/// 560 of content).
#[test]
fn a_padded_bodys_max_width_caps_its_content_box() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; padding: 20px; max-width: 600px; }");
    let body = d.body();
    d.resolve_layout(800.0, 600.0);
    assert_eq!(d.tree.get(body.0).unwrap().layout.width, 640.0);
}
