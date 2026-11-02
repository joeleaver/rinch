//! `float` and `clear` (`floats`). Expected numbers are Chrome 153's for the
//! same markup (16px/20px sans-serif, body margin 0).
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const HTML: &str = "<div style='width:400px' id=w1>\
<div id=f1 style='float:left;width:100px;height:50px'></div>\
<div id=f2 style='float:left;width:60px;height:30px'></div>\
<div id=f3 style='float:right;width:80px;height:40px'></div>\
<p id=p1 style='margin:0'>x</p>\
<div id=c style='clear:both;height:10px'></div></div>\
<div style='width:400px' id=w2>\
<div style='float:left;width:180px;height:40px' id=col1></div>\
<div style='float:left;width:180px;height:60px' id=col2></div>\
<div id=after style='clear:both;height:5px'></div></div>\
<div style='width:400px' id=w3>\
<img id=im style='float:left;width:100px;height:60px;margin-right:10px'>hello</div>\
<div style='width:400px' id=w4>
  <img id=im2 style='float:left;width:50px;height:50px'>
  wrapped</div>";

fn doc() -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    d.set_inner_html(body, HTML);
    d.resolve_layout(800.0, 600.0);
    d
}
fn rect_of(d: &RinchDocument, n: NodeId) -> (f32, f32, f32, f32) {
    let l = d.tree.get(n.0).unwrap().layout;
    let (x, y) = rinch_dom::paint::compute_absolute_position(&d.tree, n.0, 1.0);
    (x as f32, y as f32, l.width, l.height)
}
fn rect(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    rect_of(d, d.query_selector(&format!("#{id}")).expect(id))
}

#[test]
fn floats_stack_from_their_sides() {
    let d = doc();
    assert_eq!(rect(&d, "f1"), (0.0, 0.0, 100.0, 50.0));
    assert_eq!(rect(&d, "f2"), (100.0, 0.0, 60.0, 30.0));
    assert_eq!(rect(&d, "f3"), (320.0, 0.0, 80.0, 40.0));
}

#[test]
fn clear_moves_below_the_floats_and_floats_do_not_grow_the_parent() {
    let d = doc();
    assert_eq!(rect(&d, "c"), (0.0, 50.0, 400.0, 10.0));
    assert_eq!(rect(&d, "w1"), (0.0, 0.0, 400.0, 60.0));
    assert_eq!(rect(&d, "w2"), (0.0, 60.0, 400.0, 65.0));
    assert_eq!(rect(&d, "col2"), (180.0, 60.0, 180.0, 60.0));
    assert_eq!(rect(&d, "after").1, 120.0);
    assert_eq!(rect(&d, "w3"), (0.0, 125.0, 400.0, 20.0));
}

/// Chrome starts the paragraph's line at x 160, after both left floats, and
/// ends it at 320, before the right one. rinch narrows the paragraph's box to
/// that room (`floats` module docs), so its line starts at the same x.
#[test]
fn text_beside_floats_starts_after_them() {
    let d = doc();
    let (x, y, w, _) = rect(&d, "p1");
    assert_eq!((x, y, w), (160.0, 0.0, 160.0));
}

/// An image floated inside a line of text: the text's anonymous box sits
/// beside it, past its margin (Chrome: "hello" at x 110).
#[test]
fn a_floated_image_pushes_its_line_aside() {
    let d = doc();
    assert_eq!(rect(&d, "im"), (0.0, 125.0, 100.0, 60.0));
    let w3 = d.query_selector("#w3").unwrap();
    let anon = d.tree.get(w3.0).unwrap().run_boxes.clone();
    assert_eq!(anon.len(), 1, "the text has one anonymous box");
    assert_eq!(rect_of(&d, NodeId(anon[0])).0, 110.0);
}

/// The white space before a float collapses to nothing, so the text starts
/// on the float's top line. `#im` above still overhangs here, so `#im2` sits
/// beside it (Chrome: `#im2` at 110,145, "wrapped" from x 160).
#[test]
fn white_space_around_a_float_takes_no_line() {
    let d = doc();
    let im = rect(&d, "im2");
    assert_eq!(im, (110.0, 145.0, 50.0, 50.0));
    let w4 = d.query_selector("#w4").unwrap();
    let boxes: Vec<_> = d
        .tree
        .get(w4.0)
        .unwrap()
        .run_boxes
        .iter()
        .map(|&a| rect_of(&d, NodeId(a)))
        .filter(|r| r.3 > 0.0)
        .collect();
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert_eq!((boxes[0].0, boxes[0].1), (160.0, im.1));
}

/// A float that stops floating gives its neighbour the full width back.
#[test]
fn unfloating_restores_the_line() {
    let mut d = doc();
    let f = d.query_selector("#im").unwrap();
    d.set_attribute(f, "style", "width:100px;height:60px");
    d.resolve_layout(800.0, 600.0);
    let w3 = d.query_selector("#w3").unwrap();
    let anon = d.tree.get(w3.0).unwrap().run_boxes.clone();
    for a in anon {
        let t = d.tree.get(a).unwrap().taffy_id.unwrap();
        assert_eq!(d.tree.taffy.style(t).unwrap().contain, taffy::Contain::NONE);
    }
}
