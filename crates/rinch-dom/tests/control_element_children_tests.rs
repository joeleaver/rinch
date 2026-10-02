//! An **element** child of a `<textarea>`, `<input>`, `<canvas>`, `<video>` or
//! `<iframe>` is not rendered (#1178, #1288).
//!
//! A browser renders none of these elements' children. A `<textarea>` is
//! RCDATA to the parser and an `<input>` is void, so only the DOM can put an
//! element inside one (rsx `textarea { div {} }`, `append_child`); a canvas's
//! children are fallback content, a video's `<source>`/`<track>`, an iframe's
//! ignored. Measured in Chrome 153 with `line-height: 20px; padding: 0;
//! border: 0` (and `* { margin: 0; box-sizing: border-box }` for the replaced
//! elements):
//!
//! | markup | Chrome 153 | rinch before |
//! |---|---|---|
//! | `textarea { div(h 100) }` | 40 tall (2 rows) | 100 |
//! | `textarea { "abc" div(h 100) }` | 40, "abc" drawn once as the value | 120 |
//! | `canvas{block} > div(50x40)` | 300x150 | 50x40 |
//! | `canvas{block} > ["words", div(h 40)]` | 300x150, nothing drawn | 70x60, text drawn |
//! | `canvas{block} > button > "press"` | button has no box, is not drawn or hit | 25x20 box, drawn, hit |
//!
//! #1159 and #1173 left these elements **hollow** when their children are text
//! and non-atomic inlines; a block-level, atomic-inline or out-of-flow element
//! child was still laid out (and sized the element: a Taffy node with children
//! never calls its measure) and painted. The UA sheet now gives every element
//! child of the five `display: none !important`, which is what a browser's
//! layout tree amounts to: no box, nothing painted, nothing hit.
//!
//! Every text fixture registers the bundled Inter and declares `line-height`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VW: f32 = 400.0;
const VH: f32 = 300.0;
const BARE: &str = "margin: 0; box-sizing: border-box; font-family: ProbeFace; \
                    font-size: 10px; line-height: 20px; padding: 0; border: 0";

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

/// `body > div(width: 300px, white) > <tag style="display: block; {BARE}; {style}">`.
fn host(doc: &mut RinchDocument, tag: &str, style: &str) -> (NodeId, NodeId) {
    let body = doc.body();
    let c = el(
        doc,
        body,
        "div",
        &format!("{BARE}; width: 300px; color: black; background: white"),
    );
    let h = el(doc, c, tag, &format!("display: block; {BARE}; {style}"));
    (c, h)
}

fn wh(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let l = doc.tree.get(id.0).unwrap().layout;
    (l.width, l.height)
}

fn pixels(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

/// Pixels darker than the white ground (glyphs, or a red/black box).
fn ink(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| p[3] > 0 && (p[0] as u16 + p[1] as u16 + p[2] as u16) < 600)
        .count()
}

/// Chrome: a textarea holding a 100px block is the size of an empty one.
#[test]
fn a_block_child_does_not_size_a_textarea() {
    let mut doc = doc();
    let (_, t) = host(&mut doc, "textarea", "width: 200px");
    el(&mut doc, t, "div", "height: 100px; background: red");
    doc.resolve_layout(VW, VH);
    assert_eq!(wh(&doc, t), (200.0, 40.0), "Chrome 153: two rows, 40 tall");
}

/// Chrome: the text beside the block is the default value, drawn once; the
/// block is neither laid out nor drawn. The anonymous block box around the
/// text used to be an IFC root of its own, drawn as a line above the block.
#[test]
fn text_beside_a_block_child_is_the_value_and_the_block_is_not_drawn() {
    let mut mixed = doc();
    let (_, t) = host(&mut mixed, "textarea", "width: 200px");
    text(&mut mixed, t, "abc");
    let b = el(&mut mixed, t, "div", "height: 100px; background: red");
    let mixed_px = pixels(&mut mixed);
    assert_eq!(wh(&mixed, t), (200.0, 40.0), "Chrome 153: 40, not 120");
    assert_eq!(wh(&mixed, b), (0.0, 0.0), "the block has no box");

    let mut value = doc();
    let (_, tv) = host(&mut value, "textarea", "width: 200px");
    value.set_attribute(tv, "value", "abc");
    let value_px = pixels(&mut value);
    assert!(ink(&value_px) > 20, "positive control: the value is drawn");
    assert!(
        mixed_px == value_px,
        "pixel for pixel the textarea with value \"abc\" ({} vs {} dark px)",
        ink(&mixed_px),
        ink(&value_px)
    );
}

/// An author `display` on the child cannot bring it back (the UA rule is
/// `!important`), nor can positioning it out of flow.
#[test]
fn an_author_display_or_position_does_not_render_a_control_child() {
    for child_style in [
        "display: block !important; height: 100px; background: red",
        "display: flex; height: 100px; background: red",
        "display: inline-block; width: 50px; height: 100px; background: red",
        "position: absolute; top: 0; left: 0; width: 50px; height: 50px; background: red",
    ] {
        for tag in ["textarea", "input"] {
            let mut d = doc();
            let (_, t) = host(&mut d, tag, "width: 200px");
            let b = el(&mut d, t, "div", child_style);
            let px = pixels(&mut d);
            let want_h = if tag == "textarea" { 40.0 } else { 20.0 };
            assert_eq!(
                wh(&d, t).1,
                want_h,
                "<{tag}> holding a div [{child_style}]: an empty control's height"
            );
            assert_eq!(wh(&d, b), (0.0, 0.0), "<{tag}> > div [{child_style}]: no box");
            assert_eq!(ink(&px), 0, "<{tag}> > div [{child_style}]: nothing drawn");
        }
    }
}

/// Positive control for the rule's reach: the same child of a plain `div` is
/// laid out and drawn.
#[test]
fn a_div_child_of_a_div_is_still_rendered() {
    let mut d = doc();
    let (_, h) = host(&mut d, "div", "width: 200px");
    let b = el(&mut d, h, "div", "height: 100px; background: red");
    let px = pixels(&mut d);
    assert_eq!(wh(&d, b), (200.0, 100.0));
    assert_eq!(wh(&d, h), (200.0, 100.0));
    assert_eq!(ink(&px), 200 * 100);
}

/// A child moved out of a control is rendered again where it lands.
#[test]
fn a_child_moved_out_of_a_textarea_is_rendered_again() {
    let mut d = doc();
    let (c, t) = host(&mut d, "textarea", "width: 200px");
    let b = el(&mut d, t, "div", "height: 50px; background: red");
    d.resolve_layout(VW, VH);
    assert_eq!(wh(&d, b), (0.0, 0.0));
    d.append_child(c, b);
    d.resolve_layout(VW + 1.0, VH);
    assert_eq!(wh(&d, b), (300.0, 50.0), "out of the textarea, it is a block");
    assert_eq!(wh(&d, t), (200.0, 40.0));
}

/// #1288: a canvas, video or iframe keeps its 300x150 default object size
/// whatever element children it holds, and draws none of them.
#[test]
fn a_replaced_elements_element_children_do_not_size_or_draw() {
    for tag in ["canvas", "video", "iframe"] {
        for display in ["block", "flex"] {
            let mut d = doc();
            let (_, h) = host(&mut d, tag, &format!("display: {display}; border: 0"));
            let b = el(&mut d, h, "div", "width: 50px; height: 40px; background: red");
            let px = pixels(&mut d);
            assert_eq!(
                wh(&d, h),
                (300.0, 150.0),
                "<{tag} display:{display}> > div(50x40): Chrome 153 300x150"
            );
            assert_eq!(wh(&d, b), (0.0, 0.0), "<{tag}> > div: no box");
            assert_eq!(ink(&px), 0, "<{tag}> > div: nothing drawn");
        }
    }
}

/// #1288: fallback words beside a block in a canvas — Chrome 300x150, nothing
/// drawn. rinch laid the words out in an anonymous block box and drew them.
#[test]
fn canvas_fallback_text_beside_a_block_is_not_drawn() {
    let mut d = doc();
    let (_, h) = host(&mut d, "canvas", "");
    text(&mut d, h, "fallback words");
    el(&mut d, h, "div", "height: 40px");
    let px = pixels(&mut d);
    assert_eq!(wh(&d, h), (300.0, 150.0), "Chrome 153: 300x150");
    assert_eq!(ink(&px), 0, "the fallback words are not drawn");
}

/// #1288: a button as canvas fallback has no box and draws nothing (Chrome
/// draws and hits neither; rinch gave it a 25x20 box at the canvas origin).
#[test]
fn a_canvas_fallback_button_has_no_box() {
    let mut d = doc();
    let (_, h) = host(&mut d, "canvas", "");
    let btn = el(&mut d, h, "button", "background: black; color: black");
    text(&mut d, btn, "press");
    let px = pixels(&mut d);
    assert_eq!(wh(&d, h), (300.0, 150.0));
    assert_eq!(wh(&d, btn), (0.0, 0.0), "the fallback button has no box");
    assert_eq!(ink(&px), 0, "nothing drawn");
}
