//! A `<textarea>` or `<input>` holding a **text child** (#1159).
//!
//! To a browser a control's children are not content. A `<textarea>`'s text
//! children are its *default value*: it shows them until something sets its
//! value, and they size it no more than a value does. An `<input>` shows its
//! children not at all. Measured in Chrome 153 (`font-size: 10px;
//! line-height: 20px; padding: 0; border: 0`): a textarea holding "hello world
//! foo" as a child is exactly the size of an empty one, with the text drawn
//! once as its value; after a script sets `.value`, only the value is drawn.
//!
//! rinch used to lay the children out as an inline formatting context: the
//! control was sized by them, and `paint_input_value` drew the `value`
//! attribute over them, so a textarea that had been typed into (typing writes
//! `value`) showed its text twice. Now the children form no content: the
//! control is sized as a childless one is, and a textarea's direct text
//! children are what `paint_input_value` draws when no `value` attribute is
//! set.
//!
//! Every text fixture registers the bundled Inter and declares `line-height`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VW: f32 = 400.0;
const VH: f32 = 200.0;
const BARE: &str =
    "font-family: ProbeFace; font-size: 10px; line-height: 20px; padding: 0; border: 0";

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

fn container(doc: &mut RinchDocument, style: &str) -> NodeId {
    let body = doc.body();
    el(
        doc,
        body,
        "div",
        &format!(
            "width: 300px; font-family: ProbeFace; font-size: 10px; line-height: 20px; \
             color: black; background: white; {style}"
        ),
    )
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
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

/// Glyph coverage: a pixel darker than the white ground.
fn ink(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| p[3] > 0 && (p[0] as u16 + p[1] as u16 + p[2] as u16) < 600)
        .count()
}

/// One block textarea in a fresh document, built by `fill`.
fn one_textarea(fill: impl FnOnce(&mut RinchDocument, NodeId)) -> Vec<[u8; 4]> {
    let mut doc = doc();
    let c = container(&mut doc, "");
    let t = el(
        &mut doc,
        c,
        "textarea",
        &format!("display: block; width: 200px; {BARE}"),
    );
    fill(&mut doc, t);
    pixels(&mut doc)
}

/// A textarea whose value arrived as a child draws exactly what the same
/// textarea draws with that text as its `value`: once, where a value goes.
#[test]
fn a_textarea_child_paints_exactly_as_the_same_value() {
    let child = one_textarea(|d, t| text(d, t, "Hello there"));
    let value = one_textarea(|d, t| d.set_attribute(t, "value", "Hello there"));
    assert!(ink(&value) > 50, "positive control: the value is drawn");
    assert_eq!(ink(&child), ink(&value), "the child is drawn as the value");
    assert!(child == value, "pixel for pixel");
}

/// A `value` wins over the children: typing writes `value`, and the child text
/// must not be drawn beside it (Chrome: only `.value` shows once it is set).
#[test]
fn a_value_attribute_replaces_the_child_text() {
    let both = one_textarea(|d, t| {
        text(d, t, "CHILD CHILD");
        d.set_attribute(t, "value", "VALUE");
    });
    let value = one_textarea(|d, t| d.set_attribute(t, "value", "VALUE"));
    assert!(ink(&value) > 20, "positive control: the value is drawn");
    assert!(both == value, "the child text is not drawn under a value");
}

/// Even an empty `value`: the field was cleared (a submit handler writing "").
#[test]
fn an_empty_value_attribute_still_replaces_the_child_text() {
    let both = one_textarea(|d, t| {
        text(d, t, "CHILD CHILD");
        d.set_attribute(t, "value", "");
    });
    let empty = one_textarea(|_, _| {});
    assert!(both == empty, "a cleared textarea shows nothing");
}

/// An `<input>` has no content model: a child is never drawn.
#[test]
fn an_input_draws_none_of_its_children() {
    let build = |fill: &dyn Fn(&mut RinchDocument, NodeId)| {
        let mut doc = doc();
        let c = container(&mut doc, "");
        let i = el(
            &mut doc,
            c,
            "input",
            &format!("display: block; width: 200px; {BARE}"),
        );
        fill(&mut doc, i);
        pixels(&mut doc)
    };
    let with_child = build(&|d, i| text(d, i, "hello world"));
    let empty = build(&|_, _| {});
    let valued = build(&|d, i| d.set_attribute(i, "value", "hello world"));
    assert!(ink(&valued) > 50, "positive control: a value is drawn");
    assert!(with_child == empty, "the child is not drawn");
}

/// The children do not size the control, wherever its size is intrinsic.
#[test]
fn a_text_child_does_not_size_the_control() {
    for (name, host, tag) in [
        ("inline textarea", "", "textarea"),
        ("flex-row textarea", "display: flex", "textarea"),
        ("inline input", "", "input"),
        ("flex-row input", "display: flex", "input"),
    ] {
        let size = |child: bool| {
            let mut doc = doc();
            let c = container(&mut doc, host);
            let t = el(&mut doc, c, tag, BARE);
            if child {
                text(&mut doc, t, "hello world foo");
            }
            doc.resolve_layout(VW, VH);
            wh(&doc, t)
        };
        assert_eq!(size(true), size(false), "{name}");
    }
}

/// A block textarea narrower than its child text is still `rows` lines tall,
/// and the text is not laid out as content below it.
#[test]
fn a_long_child_does_not_grow_a_block_textarea() {
    let mut doc = doc();
    let c = container(&mut doc, "");
    let t = el(
        &mut doc,
        c,
        "textarea",
        &format!("display: block; width: 60px; {BARE}"),
    );
    text(&mut doc, t, "aaa bbb ccc ddd eee fff ggg");
    let after = el(&mut doc, c, "div", "height: 10px");
    doc.resolve_layout(VW, VH);
    assert_eq!(wh(&doc, t), (60.0, 40.0));
    assert_eq!(doc.tree.get(after.0).unwrap().layout.y, 40.0);
    assert!(
        doc.tree.get(t.0).unwrap().text_layout.is_none(),
        "no inline layout is built for a control's children"
    );
}

/// `rows`, `line-height` and `type` still move a text-child control, at an
/// unchanged viewport.
#[test]
fn rows_line_height_and_type_still_reach_a_text_child_control() {
    let mut doc = doc();
    let c = container(&mut doc, "");
    let t = el(
        &mut doc,
        c,
        "textarea",
        &format!("display: block; width: 100px; {BARE}"),
    );
    text(&mut doc, t, "abc");
    doc.resolve_layout(VW, VH);
    assert_eq!(wh(&doc, t).1, 40.0);
    doc.set_attribute(t, "rows", "3");
    doc.resolve_layout(VW, VH);
    assert_eq!(wh(&doc, t).1, 60.0, "rows");
    doc.set_style(t, "line-height", "30px");
    doc.resolve_layout(VW, VH);
    assert_eq!(wh(&doc, t).1, 90.0, "line-height");

    let i = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; width: 30px; {BARE}"),
    );
    text(&mut doc, i, "aaa bbb ccc");
    doc.resolve_layout(VW, VH);
    assert_eq!(wh(&doc, i).1, 20.0, "a text input is one line");
    let bare_checkbox = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; width: 30px; {BARE}"),
    );
    doc.set_attribute(bare_checkbox, "type", "checkbox");
    doc.set_attribute(i, "type", "checkbox");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        wh(&doc, i),
        wh(&doc, bare_checkbox),
        "a checkbox is not sized by its child"
    );
    doc.set_attribute(i, "type", "text");
    doc.resolve_layout(VW, VH);
    assert_eq!(wh(&doc, i).1, 20.0, "and back");
}

/// A colour-only restyle of a text-child textarea costs what it costs for an
/// empty one: no Taffy compute, no shaping.
#[test]
fn a_colour_restyle_of_a_text_child_textarea_runs_no_layout() {
    let frame = |child: bool| {
        let mut doc = doc();
        let c = container(&mut doc, "");
        let mut ts = vec![];
        for _ in 0..5 {
            let t = el(
                &mut doc,
                c,
                "textarea",
                &format!("display: block; width: 100px; {BARE}"),
            );
            if child {
                text(&mut doc, t, "abc");
            }
            ts.push(t);
        }
        doc.resolve_layout(VW, VH);
        doc.resolve_layout(VW, VH);
        doc.tree.perf.end_frame();
        doc.set_style(ts[3], "color", "red");
        doc.resolve_layout(VW, VH);
        let f = doc.tree.perf.frame();
        [
            Counter::TaffyRootComputes,
            Counter::TaffyMeasureCalls,
            Counter::ShapeMeasureIfc,
            Counter::ShapeIfcBuild,
            Counter::LayoutSkippedPaintOnly,
        ]
        .map(|k| f.get(k))
    };
    let empty = frame(false);
    assert_eq!(empty[0], 0, "positive control: an empty one computes nothing");
    assert_eq!(frame(true), empty);
}

/// What a desktop field starts editing from, and what `live_value` answers:
/// the `value` attribute, else a textarea's text children, else nothing.
#[test]
fn live_value_is_the_value_else_a_textareas_text_children() {
    let mut doc = doc();
    let c = container(&mut doc, "");
    let t = el(&mut doc, c, "textarea", BARE);
    text(&mut doc, t, "one ");
    text(&mut doc, t, "two");
    assert_eq!(doc.live_value(t).as_deref(), Some("one two"));
    doc.set_attribute(t, "value", "typed");
    assert_eq!(doc.live_value(t).as_deref(), Some("typed"));
    doc.set_attribute(t, "value", "");
    assert_eq!(doc.live_value(t).as_deref(), Some(""));

    let i = el(&mut doc, c, "input", BARE);
    text(&mut doc, i, "child");
    assert_eq!(doc.live_value(i), None, "an input's children are no value");
}
