//! `float` and `clear` (`floats`, `float_wrap`). Expected numbers are
//! Chrome's for the same markup (16px/20px sans-serif, body margin 0): 153
//! for the boxes, 156 for the lines. Line positions are asserted only where
//! they do not depend on the font (where a line starts, and where it may
//! end), so the host's fonts do not move them.
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
/// Each line of IFC root `n`'s paint layout: its top, and where its room
/// starts and ends, from the root's content box.
fn lines_of(d: &RinchDocument, n: NodeId) -> Vec<(f32, f32, f32)> {
    let node = d.tree.get(n.0).unwrap();
    let layout = &node.text_layout.as_ref().expect("an IFC root").layout;
    layout
        .lines()
        .map(|l| {
            let m = l.metrics();
            // The line box's top: the baseline less the ascent and half
            // the leading, which is negative for a 16px font on a 20px line
            // in some fonts (`block_min_coord` clamps it to 0).
            (
                (m.baseline - m.ascent - m.leading * 0.5).round(),
                m.inline_min_coord.round(),
                m.inline_max_coord.round(),
            )
        })
        .collect()
}
fn lines(d: &RinchDocument, id: &str) -> Vec<(f32, f32, f32)> {
    lines_of(d, d.query_selector(&format!("#{id}")).expect(id))
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
/// ends it at 320, before the right one; the paragraph's box itself is not
/// narrowed (its background is the full 400px).
#[test]
fn text_beside_floats_starts_after_them() {
    let d = doc();
    assert_eq!(rect(&d, "p1"), (0.0, 0.0, 400.0, 20.0));
    assert_eq!(lines(&d, "p1"), vec![(0.0, 160.0, 320.0)]);
}

/// An image floated inside a line of text: the text's anonymous box keeps
/// the full width and its line starts past the image's margin (Chrome:
/// "hello" at x 110).
#[test]
fn a_floated_image_pushes_its_line_aside() {
    let d = doc();
    assert_eq!(rect(&d, "im"), (0.0, 125.0, 100.0, 60.0));
    let w3 = d.query_selector("#w3").unwrap();
    let anon = d.tree.get(w3.0).unwrap().run_boxes.clone();
    assert_eq!(anon.len(), 1, "the text has one anonymous box");
    assert_eq!(rect_of(&d, NodeId(anon[0])).0, 0.0);
    assert_eq!(lines_of(&d, NodeId(anon[0]))[0].1, 110.0);
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
    assert_eq!((boxes[0].0, boxes[0].1), (0.0, im.1));
    let run = d.tree.get(w4.0).unwrap().run_boxes.clone();
    let text = run
        .into_iter()
        .find(|&a| rect_of(&d, NodeId(a)).3 > 0.0)
        .unwrap();
    assert_eq!(lines_of(&d, NodeId(text))[0].1, 160.0);
}

/// A float that stops floating gives its neighbour's line the full width
/// back.
#[test]
fn unfloating_restores_the_line() {
    let mut d = doc();
    let f = d.query_selector("#im").unwrap();
    d.set_attribute(f, "style", "width:100px;height:60px");
    d.resolve_layout(800.0, 600.0);
    let w3 = d.query_selector("#w3").unwrap();
    let anon = d.tree.get(w3.0).unwrap().run_boxes.clone();
    for a in anon {
        if let Some(first) = d.tree.get(a).unwrap().text_layout.as_ref() {
            let x = first
                .layout
                .lines()
                .next()
                .unwrap()
                .metrics()
                .inline_min_coord;
            assert_eq!(x, 0.0);
        }
    }
}

const WRAP: &str = "<div style='width:400px'>\
<div id=f5 style='float:left;width:100px;height:50px'></div>\
<p id=p5 style='margin:0'>x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x \
x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x \
x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x x \
x x x x x x x x x x x x x x x x x x</p></div>\
<div style='width:400px'><div style='float:left;width:380px;height:50px'></div>\
<p id=p6 style='margin:0'>abcdefghijklmnopqrstuvwxyz</p></div>\
<div style='width:400px'><div style='float:right;width:150px;height:30px'></div>\
<p id=p7 style='margin:0;padding-left:20px'>x x x x x x x x x x x x x x x x x x x x x x x x \
x x x x x x x x x x x x x x x x x x x x x x x</p></div>";

fn wrap_doc() -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    d.set_inner_html(body, WRAP);
    d.resolve_layout(800.0, 600.0);
    d
}

/// Text longer than the float beside it: the lines its 50px reaches (the
/// first three, the third from 40 to 60) start after it, and the rest take
/// the full width under it (Chrome 156: x 100, 100, 100, then 0).
#[test]
fn text_flows_back_under_a_float_it_outlasts() {
    let d = wrap_doc();
    let l = lines(&d, "p5");
    assert!(l.len() >= 4, "{l:?}");
    for (i, &(top, start, end)) in l.iter().enumerate() {
        assert_eq!(top, 20.0 * i as f32, "{l:?}");
        let want = if i < 3 { 100.0 } else { 0.0 };
        // An auto-width paragraph's paint lines are broken a pixel wider
        // than its box (`ifc_paint_max_width`).
        assert_eq!(start, want, "line {i}: {l:?}");
        assert!((400.0..=401.0).contains(&end), "line {i}: {l:?}");
    }
    let (_, _, w, h) = rect(&d, "p5");
    assert_eq!((w, h), (400.0, 20.0 * l.len() as f32));
}

/// A line whose first word does not fit beside the float moves down below
/// it, and the paragraph's box grows by the gap (Chrome 156: the box is 70
/// tall, its line 50 down at x 0).
#[test]
fn a_line_that_does_not_fit_beside_a_float_moves_below_it() {
    let d = wrap_doc();
    let l = lines(&d, "p6");
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!((l[0].0, l[0].1), (50.0, 0.0));
    assert_eq!(rect(&d, "p6").3, 70.0);
}

/// A right float ends the lines beside it, the paragraph's padding still
/// starts them (Chrome 156: the first two lines end before x 250, the third
/// runs on).
#[test]
fn a_right_float_ends_the_lines_beside_it() {
    let d = wrap_doc();
    let l = lines(&d, "p7");
    assert!(l.len() >= 3, "{l:?}");
    assert_eq!(&l[..2], &[(0.0, 0.0, 230.0), (20.0, 0.0, 230.0)]);
    assert_eq!(l[2].1, 0.0);
    assert!((380.0..=381.0).contains(&l[2].2), "{l:?}");
}

/// The lines follow the float when it changes: wider, they start further in;
/// gone, they start at 0.
#[test]
fn lines_follow_a_float_that_changes() {
    let mut d = wrap_doc();
    let f = d.query_selector("#f5").unwrap();
    d.set_attribute(f, "style", "float:left;width:150px;height:50px");
    d.resolve_layout(800.0, 600.0);
    assert_eq!(lines(&d, "p5")[0].1, 150.0);
    d.set_attribute(f, "style", "width:150px;height:50px");
    d.resolve_layout(800.0, 600.0);
    assert!(lines(&d, "p5").iter().all(|l| l.1 == 0.0));
}

/// A caret, a glyph's bounds, a hit test and a selection beside a float
/// are where the line is drawn: from x 160 in `#p1`, as Chrome 156 has its
/// text.
#[test]
fn carets_and_hit_tests_beside_a_float_find_the_moved_line() {
    use rinch_dom::text_query as q;
    let d = doc();
    let p1 = d.query_selector("#p1").unwrap();
    let layout = &d
        .tree
        .get(p1.0)
        .unwrap()
        .text_layout
        .as_ref()
        .unwrap()
        .layout;
    assert_eq!(q::caret_position_for_offset_layout(layout, 0).0, 160.0);
    let g = q::glyph_bounds_for_offset_layout(layout, 0).unwrap();
    assert_eq!(g.x, 160.0);
    // Left of the line's start is before its first character, past its end
    // after its last.
    assert_eq!(q::byte_offset_from_position(layout, 150.0, 10.0), 0);
    assert_eq!(
        q::byte_offset_from_position(layout, 170.0 + g.width, 10.0),
        1
    );
    let rects = q::selection_rects_for_layout(layout, 0, 1);
    assert_eq!(rects.len(), 1, "{rects:?}");
    assert_eq!(rects[0].0, 160.0);
}
