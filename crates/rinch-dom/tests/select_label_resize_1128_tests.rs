//! A closed `<select>` follows its widest option label when the label is
//! edited through an element inside the option, or through the option's
//! `label` attribute (issue #1128).
//!
//! Every expected width is Chrome 155's for the same markup, measured with the
//! bundled Inter registered as `ProbeFace` at `font: 16px/20px` (Chrome sizes a
//! select from a descendant edit after the task that made it; the widths are
//! the settled ones).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc
}

fn select(doc: &mut RinchDocument) -> NodeId {
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.set_attribute(sel, "style", "font: 16px/20px ProbeFace");
    doc.append_child(body, sel);
    sel
}

fn option_with_text(doc: &mut RinchDocument, sel: NodeId, text: &str) -> (NodeId, NodeId) {
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let t = doc.create_text(text);
    doc.append_child(o, t);
    (o, t)
}

fn width(doc: &mut RinchDocument, sel: NodeId, viewport_w: f32) -> f32 {
    doc.resolve_layout(viewport_w, 600.0);
    doc.tree.get(sel.0).unwrap().layout.width
}

/// `option { {|| name.get()} }`: rsx puts the text in a `display: contents`
/// span. Editing that text grows the select (31 -> 160 in Chrome 155), and
/// shrinking it back gives 31 again.
#[test]
fn editing_text_inside_a_wrapper_in_an_option_resizes_the_select() {
    let mut doc = document();
    let sel = select(&mut doc);
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "display: contents");
    doc.append_child(o, span);
    let t = doc.create_text("a");
    doc.append_child(span, t);
    assert_eq!(width(&mut doc, sel, 800.0), 31.0);

    doc.set_text_content(t, "Hello world option");
    assert_eq!(width(&mut doc, sel, 801.0), 160.0);

    doc.set_text_content(t, "a");
    assert_eq!(width(&mut doc, sel, 802.0), 31.0);
}

/// Text appended into a span inside an option (Chrome 155: 31 -> 121), and a
/// text node edited two elements below the option (`option > span > b`,
/// 31 -> 160): the walk up to the select passes any number of elements
/// inside the option.
#[test]
fn content_added_deep_inside_an_option_resizes_the_select() {
    // `option > span > "a"`, then " Hello world" appended into the span.
    let mut doc = document();
    let sel = select(&mut doc);
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let span = doc.create_element("span");
    doc.append_child(o, span);
    let t = doc.create_text("a");
    doc.append_child(span, t);
    assert_eq!(width(&mut doc, sel, 800.0), 31.0);
    let more = doc.create_text(" Hello world");
    doc.append_child(span, more);
    assert_eq!(width(&mut doc, sel, 801.0), 121.0);

    // `option > span > b > "a"`, the text node edited two levels below the
    // option.
    let mut doc = document();
    let sel = select(&mut doc);
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let span = doc.create_element("span");
    doc.append_child(o, span);
    let b = doc.create_element("b");
    doc.append_child(span, b);
    let t = doc.create_text("a");
    doc.append_child(b, t);
    assert_eq!(width(&mut doc, sel, 800.0), 31.0);
    doc.set_text_content(t, "Hello world option");
    assert_eq!(width(&mut doc, sel, 801.0), 160.0);
}

/// The `label` attribute is the option's label: setting it, changing it and
/// removing it each resize the select (Chrome 155: 31 -> 160 -> 91, and an
/// option with `label` removed falls back to its text).
#[test]
fn an_option_label_attribute_resizes_the_select() {
    let mut doc = document();
    let sel = select(&mut doc);
    let (o, _) = option_with_text(&mut doc, sel, "a");
    assert_eq!(width(&mut doc, sel, 800.0), 31.0);

    doc.set_attribute(o, "label", "Hello world option");
    assert_eq!(width(&mut doc, sel, 801.0), 160.0);

    doc.set_attribute(o, "label", "Mid label");
    assert_eq!(width(&mut doc, sel, 802.0), 91.0);

    doc.remove_attribute(o, "label");
    assert_eq!(width(&mut doc, sel, 803.0), 31.0);
}

/// A `label` on the widest option narrows the select to the next widest
/// (Chrome 155: 160 -> 91). `disabled` and `hidden` do not change the width
/// (91 stays 91).
#[test]
fn a_label_that_shrinks_the_widest_option_narrows_the_select() {
    let mut doc = document();
    let sel = select(&mut doc);
    let (wide, _) = option_with_text(&mut doc, sel, "Hello world option");
    option_with_text(&mut doc, sel, "Mid label");
    assert_eq!(width(&mut doc, sel, 800.0), 160.0);

    doc.set_attribute(wide, "label", "x");
    assert_eq!(width(&mut doc, sel, 801.0), 91.0);

    doc.set_attribute(wide, "disabled", "");
    doc.set_attribute(wide, "hidden", "");
    assert_eq!(width(&mut doc, sel, 802.0), 91.0);
}

/// A `label` attribute on an element that is not an option, and text edits
/// outside any option, leave the select alone (positive control: the same
/// document's select still resizes for an option edit afterwards).
#[test]
fn a_label_on_another_element_does_not_touch_the_select() {
    let mut doc = document();
    let sel = select(&mut doc);
    let (_, t) = option_with_text(&mut doc, sel, "a");
    let body = doc.body();
    let p = doc.create_element("p");
    doc.append_child(body, p);
    let pt = doc.create_text("x");
    doc.append_child(p, pt);
    assert_eq!(width(&mut doc, sel, 800.0), 31.0);
    doc.set_attribute(p, "label", "Hello world option");
    doc.set_text_content(pt, "Hello world option");
    assert_eq!(width(&mut doc, sel, 801.0), 31.0);
    doc.set_text_content(t, "Hello world option");
    assert_eq!(width(&mut doc, sel, 802.0), 160.0);
}

/// The width of a select built fresh with `labels`, laid out once: the oracle
/// for an edit history ending in the same options (a browser sizes a select
/// from the options it has, so it agrees by construction).
fn fresh(labels: &[&str]) -> f32 {
    let mut doc = document();
    let sel = select(&mut doc);
    for label in labels {
        option_with_text(&mut doc, sel, label);
    }
    width(&mut doc, sel, 800.0)
}

/// An option moved to another select re-sizes both: the one it left and the
/// one it joined, through `append_child` and `insert_before` alike (review of
/// #1737, F1 — the old select kept the moved option's 160).
#[test]
fn an_option_moved_to_another_select_resizes_both() {
    for use_insert_before in [false, true] {
        let mut doc = document();
        let a = select(&mut doc);
        let (wide, _) = option_with_text(&mut doc, a, "Hello world option");
        option_with_text(&mut doc, a, "a");
        let b = select(&mut doc);
        let (b_first, _) = option_with_text(&mut doc, b, "a");
        assert_eq!(width(&mut doc, a, 800.0), 160.0);
        if use_insert_before {
            doc.insert_before(b, wide, b_first);
        } else {
            doc.append_child(b, wide);
        }
        assert_eq!(
            width(&mut doc, a, 801.0),
            fresh(&["a"]),
            "the select it left"
        );
        assert_eq!(width(&mut doc, b, 802.0), 160.0, "the select it joined");
    }
}

/// `set_inner_html` on an option replaces its label, an empty string included
/// (review of #1737, F2 — clearing it left the select at 160).
#[test]
fn set_inner_html_on_an_option_resizes_the_select() {
    let mut doc = document();
    let sel = select(&mut doc);
    let (o, _) = option_with_text(&mut doc, sel, "a");
    assert_eq!(width(&mut doc, sel, 800.0), 31.0);
    doc.set_inner_html(o, "<b>Hello <i>world option</i></b>");
    assert_eq!(width(&mut doc, sel, 801.0), 160.0);
    doc.set_inner_html(o, "");
    assert_eq!(width(&mut doc, sel, 802.0), fresh(&[""]));
}

/// An edit inside an option in an `<optgroup>`: the walk passes the group on
/// its way to the select (review of #1737, F3). 160 is rinch's unindented
/// width; Chrome 155 indents an option in a group (178, #1736).
#[test]
fn an_edit_inside_an_option_in_an_optgroup_resizes_the_select() {
    let mut doc = document();
    let sel = select(&mut doc);
    let group = doc.create_element("optgroup");
    doc.append_child(sel, group);
    let o = doc.create_element("option");
    doc.append_child(group, o);
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "display: contents");
    doc.append_child(o, span);
    let t = doc.create_text("a");
    doc.append_child(span, t);
    assert_eq!(width(&mut doc, sel, 800.0), 31.0);
    doc.set_text_content(t, "Hello world option");
    assert_eq!(width(&mut doc, sel, 801.0), 160.0);
}
