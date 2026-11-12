//! `display: flow-root` establishes a block formatting context (#2351), each
//! number Chrome 157's for the same markup in a 560px column.
//!
//! A flow-root box contains its floats (its height reaches their bottom),
//! its children's margins do not collapse through its edges, and it is laid
//! out beside the floats outside it rather than under them, as an
//! `overflow: hidden` box is. rinch mapped it to a plain block, so Taffy saw
//! nothing that started a new context: the floats inside spilled into the
//! boxes after it.
//!
//! Every box here is sized, so no number depends on the host's fonts.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CSS: &str = "body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; } \
                   .f { float: left }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, &format!("<div style='width:560px'>{html}</div>"));
    d.resolve_layout(800.0, 800.0);
    d
}

/// The border box of `#id`, in page coordinates: `(x, y, width, height)`.
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

#[track_caller]
fn chrome(d: &RinchDocument, id: &str, want: (f32, f32, f32, f32)) {
    let got = abs(d, id);
    let close = |a: f32, b: f32| (a - b).abs() <= 0.5;
    assert!(
        close(got.0, want.0)
            && close(got.1, want.1)
            && close(got.2, want.2)
            && close(got.3, want.3),
        "#{id}: rinch {got:?}, Chrome {want:?}"
    );
}

#[test]
fn a_flow_root_is_as_tall_as_a_float_taller_than_its_content() {
    let d = doc(
        "<div id=a style='display:flow-root'><div class=f style='width:97px;height:63px'></div>\
         <div id=a_in style='height:17px'></div></div><div id=after style='height:11px'></div>",
    );
    chrome(&d, "a", (0.0, 0.0, 560.0, 63.0));
    chrome(&d, "a_in", (0.0, 0.0, 560.0, 17.0));
    chrome(&d, "after", (0.0, 63.0, 560.0, 11.0));
}

#[test]
fn the_issue_shape_a_float_beside_an_inline_block_in_a_nested_block() {
    let d = doc(
        "<div id=r style='display:flow-root'><div style='width:300px'>\
         <div class=f style='width:100px;height:50px'></div>\
         <span style='display:inline-block;width:50px;height:13px'></span></div></div>\
         <div id=after style='height:9px'></div>",
    );
    chrome(&d, "r", (0.0, 0.0, 560.0, 50.0));
    chrome(&d, "after", (0.0, 50.0, 560.0, 9.0));
}

#[test]
fn a_flow_root_is_laid_out_beside_an_outer_float() {
    let d = doc(
        "<div id=w><div class=f style='width:137px;height:41px'></div>\
         <div id=b style='display:flow-root;height:23px'></div></div>",
    );
    chrome(&d, "w", (0.0, 0.0, 560.0, 23.0));
    chrome(&d, "b", (137.0, 0.0, 423.0, 23.0));
}

#[test]
fn a_flow_root_beside_an_outer_float_contains_its_own() {
    let d = doc(
        "<div id=w><div class=f style='width:111px;height:29px'></div>\
         <div id=b style='display:flow-root'><div class=f style='width:43px;height:37px'></div>\
         </div></div>",
    );
    chrome(&d, "w", (0.0, 0.0, 560.0, 37.0));
    chrome(&d, "b", (111.0, 0.0, 449.0, 37.0));
}

#[test]
fn a_childs_margins_do_not_collapse_through_a_flow_roots_edges() {
    // The flow-root's own margins still collapse with its siblings' (29 and
    // 17 make 29; 7 and 11 make 11); its child's 19 and 13 stay inside it.
    let d = doc("<div id=prev style='height:5px;margin-bottom:29px'></div>\
         <div id=c style='display:flow-root;margin-top:17px;margin-bottom:7px'>\
         <div id=c_in style='margin-top:19px;margin-bottom:13px;height:7px'></div></div>\
         <div id=next style='height:3px;margin-top:11px'></div>");
    chrome(&d, "prev", (0.0, 0.0, 560.0, 5.0));
    chrome(&d, "c", (0.0, 34.0, 560.0, 39.0));
    chrome(&d, "c_in", (0.0, 53.0, 560.0, 7.0));
    chrome(&d, "next", (0.0, 84.0, 560.0, 3.0));
}

#[test]
fn a_nested_flow_root_contains_its_float_and_its_margin() {
    let d = doc(
        "<div id=d style='display:flow-root'><div id=d_in style='display:flow-root;margin-top:6px'>\
         <div class=f style='width:53px;height:71px'></div><div style='height:9px'></div></div></div>\
         <div id=after style='height:4px'></div>",
    );
    chrome(&d, "d", (0.0, 0.0, 560.0, 77.0));
    chrome(&d, "d_in", (0.0, 6.0, 560.0, 71.0));
    chrome(&d, "after", (0.0, 77.0, 560.0, 4.0));
}

#[test]
fn an_inline_flow_root_is_an_inline_block_that_contains_its_float() {
    let d = doc("<div id=w><span id=e style='display:inline flow-root'>\
         <span class=f style='width:31px;height:47px'></span>\
         <span style='display:inline-block;width:20px;height:13px'></span></span></div>");
    chrome(&d, "w", (0.0, 0.0, 560.0, 47.0));
    chrome(&d, "e", (0.0, 0.0, 51.0, 47.0));
}

#[test]
fn a_flow_root_flex_item_contains_its_float() {
    let d = doc("<div id=w style='display:flex;align-items:flex-start'>\
         <div id=fi style='display:flow-root;width:200px'>\
         <div class=f style='width:61px;height:59px'></div><div style='height:8px'></div></div></div>");
    chrome(&d, "w", (0.0, 0.0, 560.0, 59.0));
    chrome(&d, "fi", (0.0, 0.0, 200.0, 59.0));
}

/// Switching a block to `flow-root` and back after the first layout lays the
/// float out again: the Taffy style is rebuilt from the new display.
#[test]
fn toggling_flow_root_after_layout_contains_and_releases_the_float() {
    let mut d = doc(
        "<div id=a><div class=f style='width:97px;height:63px'></div>\
         <div style='height:17px'></div></div>",
    );
    chrome(&d, "a", (0.0, 0.0, 560.0, 17.0));
    let a = d.query_selector("#a").unwrap();
    d.set_attribute(a, "style", "display:flow-root");
    d.resolve_layout(800.0, 800.0);
    chrome(&d, "a", (0.0, 0.0, 560.0, 63.0));
    d.set_attribute(a, "style", "display:block");
    d.resolve_layout(800.0, 800.0);
    chrome(&d, "a", (0.0, 0.0, 560.0, 17.0));
}

/// A flow-root keeps its first line's baseline (it is a formatting-context
/// root, not layout-contained): a sibling flex item lines up with the
/// inline-block's bottom, 7px above the flow-root's bottom edge. (The line
/// is in a child block, so the flow-root is a Taffy block container whose
/// baseline Taffy reports, not a measured leaf.)
#[test]
fn a_flow_root_flex_item_is_aligned_by_its_first_lines_baseline() {
    let d = doc(
        "<div id=w style='display:flex;align-items:baseline;font-size:0;line-height:0'>\
         <div id=i1 style='display:flow-root;padding-bottom:7px'><div>\
         <span style='display:inline-block;width:20px;height:30px'></span></div></div>\
         <div id=i2 style='width:9px;height:10px'></div></div>",
    );
    chrome(&d, "w", (0.0, 0.0, 560.0, 37.0));
    chrome(&d, "i1", (0.0, 0.0, 20.0, 37.0));
    chrome(&d, "i2", (20.0, 20.0, 9.0, 10.0));
}
