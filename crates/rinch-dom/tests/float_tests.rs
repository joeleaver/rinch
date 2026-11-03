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

/// An image floated inside a line of text: the float does not split the
/// run, so the container holds the text's line itself, at the full width,
/// starting past the image's margin (Chrome: "hello" at x 110).
#[test]
fn a_floated_image_pushes_its_line_aside() {
    let d = doc();
    assert_eq!(rect(&d, "im"), (0.0, 125.0, 100.0, 60.0));
    assert!(
        d.tree
            .get(d.query_selector("#w3").unwrap().0)
            .unwrap()
            .run_boxes
            .is_empty()
    );
    assert_eq!(lines(&d, "w3")[0].1, 110.0);
}

/// The white space before a float collapses to nothing, so the text starts
/// on the float's top line. `#im` above still overhangs here, so `#im2` sits
/// beside it (Chrome: `#im2` at 110,145, "wrapped" from x 160).
#[test]
fn white_space_around_a_float_takes_no_line() {
    let d = doc();
    let im = rect(&d, "im2");
    assert_eq!(im, (110.0, 145.0, 50.0, 50.0));
    let l = lines(&d, "w4");
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!((l[0].0, l[0].1), (0.0, 160.0));
    assert_eq!(rect(&d, "w4").1, im.1);
}

/// A float that stops floating gives the line the full width back.
#[test]
fn unfloating_restores_the_line() {
    let mut d = doc();
    let f = d.query_selector("#im").unwrap();
    d.set_attribute(f, "style", "width:100px;height:60px");
    d.resolve_layout(800.0, 600.0);
    assert!(lines(&d, "w3").iter().all(|l| l.1 == 0.0));
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

/// A float in the middle of a line splits neither the line nor the
/// paragraph: "aaa " and "bbb" share one line before the right float, and the
/// paragraph is one line tall, the float overhanging it (Chrome 156: `#p8`
/// 0,0 400x20, `#f8` 300,0 100x40, the next box at y 20).
#[test]
fn a_float_in_the_middle_of_a_line_keeps_the_line_whole() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    d.set_inner_html(
        body,
        "<div style='width:400px'><p id=p8 style='margin:0'>aaa \
         <span id=f8 style='float:right;width:100px;height:40px'></span>bbb</p>\
         <div id=after8 style='height:5px'></div></div>",
    );
    d.resolve_layout(800.0, 600.0);
    assert_eq!(rect(&d, "p8"), (0.0, 0.0, 400.0, 20.0));
    assert_eq!(rect(&d, "f8"), (300.0, 0.0, 100.0, 40.0));
    assert_eq!(rect(&d, "after8").1, 20.0);
    let l = lines(&d, "p8");
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!((l[0].1, l[0].2), (0.0, 300.0));
}

fn later_line_doc() -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    d.set_inner_html(
        body,
        "<div style='width:400px'><p id=p9 style='margin:0'>aaa<br>bbb \
         <span id=f9 style='float:right;width:100px;height:30px'></span>ccc</p></div>\
         <div style='width:400px'><p id=p10 style='margin:0'>\
         <span style='display:inline-block;width:350px;height:10px'></span>\
         <span id=f10 style='float:right;width:100px;height:30px'></span> after</p></div>",
    );
    d.resolve_layout(800.0, 600.0);
    d
}

/// A float standing on a later line of its paragraph goes to that line's
/// top, and only lines from there on make room for it (Chrome 156: `#f9` at
/// 300,20; the first line full width, the second ending at 300).
#[test]
fn a_float_on_a_later_line_goes_to_that_lines_top() {
    let d = later_line_doc();
    assert_eq!(rect(&d, "p9"), (0.0, 0.0, 400.0, 40.0));
    assert_eq!(rect(&d, "f9"), (300.0, 20.0, 100.0, 30.0));
    let l = lines(&d, "p9");
    assert_eq!(l.len(), 2, "{l:?}");
    assert!((400.0..=401.0).contains(&l[0].2), "{l:?}");
    assert_eq!((l[1].0, l[1].1, l[1].2), (20.0, 0.0, 300.0));
}

/// A float that does not fit beside what its line already holds goes below
/// that line (Chrome 156: the 350px box leaves 50px, the float needs 100, so
/// it starts at the line's bottom, which is the paragraph's: `#f10` at
/// 300,70 under a 30px line).
#[test]
fn a_float_that_does_not_fit_beside_its_line_goes_below_it() {
    let d = later_line_doc();
    let (_, py, _, ph) = rect(&d, "p10");
    let (fx, fy, fw, fh) = rect(&d, "f10");
    assert_eq!((fx, fw, fh), (300.0, 100.0, 30.0));
    assert_eq!(fy, py + ph, "the float starts where its line ends");
    let l = lines(&d, "p10");
    assert_eq!(l.len(), 1, "{l:?}");
}

/// A box that establishes a formatting context and holds only floats is as
/// tall as they are plus its padding: the floats' bottom is the content
/// box's, and the bottom padding goes below it (Chrome 156: 8 + 22 + 8 = 38,
/// the heading's margin below that).
#[test]
fn a_padded_box_holding_only_floats_keeps_its_bottom_padding() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    d.set_inner_html(
        body,
        "<div style='width:400px'><div id=bar style='overflow:hidden;padding:8px'>\
         <div style='float:left;width:50px;height:22px'></div>\
         <div style='float:right;width:60px;height:22px'></div></div>\
         <div id=next style='height:5px'></div></div>",
    );
    d.resolve_layout(800.0, 600.0);
    assert_eq!(rect(&d, "bar").3, 38.0);
    assert_eq!(rect(&d, "next").1, 38.0);
}

fn nowrap_doc(align: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    d.set_inner_html(
        body,
        &format!(
            "<div style='width:400px'><div style='float:left;width:100px;height:50px'></div>\
             <p id=n style='margin:0;white-space:nowrap;text-align:{align}'>x</p></div>"
        ),
    );
    d.resolve_layout(800.0, 600.0);
    d
}

/// A paragraph that does not wrap starts its line beside a float too, and
/// one aligned is aligned within the room the float leaves (Chrome 156: the
/// line at x 100; centred, within 100..400).
#[test]
fn a_line_that_does_not_wrap_starts_beside_a_float() {
    let l = lines(&nowrap_doc("left"), "n");
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].1, 100.0, "{l:?}");
    let l = lines(&nowrap_doc("center"), "n");
    assert_eq!((l[0].1, l[0].2), (100.0, 400.0), "{l:?}");
}

/// A float that comes and goes: the paragraph is laid out with no float (its
/// measure cache keeps that size), beside one, and with it gone again, when
/// it takes the whole width back and is broken that way for paint too (a
/// review of #2080: the measure answered from its cache before it dropped the
/// bands it had recorded beside the float, so paint kept breaking the lines
/// around the float's old room).
#[test]
fn lines_take_the_whole_width_back_when_the_float_goes() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    let words = "x ".repeat(120);
    d.set_inner_html(
        body,
        &format!("<div id=w style='width:400px'><p id=p style='margin:0'>{words}</p></div>"),
    );
    d.resolve_layout(800.0, 600.0);
    let before = lines(&d, "p");
    let w = d.query_selector("#w").unwrap();
    let p = d.query_selector("#p").unwrap();
    let f = d.create_element("div");
    d.set_attribute(f, "style", "float:left;width:100px;height:50px");
    d.insert_before(w, f, p);
    d.resolve_layout(800.0, 600.0);
    assert_eq!(lines(&d, "p")[0].1, 100.0);
    d.remove_child(w, f);
    d.resolve_layout(800.0, 600.0);
    let after = lines(&d, "p");
    assert_eq!(after, before);
    assert_eq!(rect(&d, "p").3, 20.0 * before.len() as f32);
}

/// Text edited in a paragraph beside a float lays the page out again: the
/// paragraph grows from one line to three and the paragraph after it moves
/// down (a review of #2080: a box beside a float kept no cache entry, so
/// marking it dirty stopped there and its parent's cached layout was served).
#[test]
fn text_edited_beside_a_float_moves_what_follows() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif; }");
    let body = d.body();
    d.set_inner_html(
        body,
        "<div style='width:400px'><div style='float:left;width:100px;height:50px'></div>\
         <p id=p style='margin:0'>short</p><p id=q style='margin:0'>after</p></div>",
    );
    d.resolve_layout(800.0, 600.0);
    assert_eq!(rect(&d, "q").1, 20.0);
    let p = d.query_selector("#p").unwrap();
    let text = d.tree.get(p.0).unwrap().children[0];
    d.set_text_content(NodeId(text), &"x ".repeat(110));
    d.resolve_layout(800.0, 600.0);
    let n = lines(&d, "p").len();
    assert!(n >= 3, "{n} lines");
    assert_eq!(rect(&d, "p").3, 20.0 * n as f32);
    assert_eq!(rect(&d, "q").1, 20.0 * n as f32);
}
