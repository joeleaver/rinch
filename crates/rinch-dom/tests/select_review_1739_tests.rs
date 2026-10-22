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

fn build(doc: &mut RinchDocument, container: &str, css: &str, opts: &[&str]) -> (NodeId, NodeId) {
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!("font: 16px/20px ProbeFace; width: 600px; {container}"),
    );
    doc.append_child(body, div);
    let sel = doc.create_element("select");
    doc.set_attribute(sel, "style", &format!("font: 16px/20px ProbeFace; {css}"));
    doc.append_child(div, sel);
    for label in opts {
        let o = doc.create_element("option");
        doc.append_child(sel, o);
        let t = doc.create_text(label);
        doc.append_child(o, t);
    }
    (div, sel)
}

fn sz(container: &str, css: &str, opts: &[&str]) -> (f32, f32) {
    let mut doc = document();
    let (_, sel) = build(&mut doc, container, css, opts);
    doc.resolve_layout(800.0, 600.0);
    let l = doc.tree.get(sel.0).unwrap().layout;
    (l.width, l.height)
}

const O: &[&str] = &["a", "Hello world option"];

/// Chrome 155: a flex item's automatic minimum is min(specified width, label
/// content width), so a `px`-width select in a 50px row keeps 160 / 100 / 25.
#[test]
fn a_px_width_select_in_a_narrow_flex_row_keeps_its_automatic_minimum() {
    for (css, chrome) in [
        ("width: 200px", 160.0),
        ("width: 100px", 100.0),
        ("width: 50%", 25.0),
        ("width: 200px; min-width: 0", 50.0),
    ] {
        assert_eq!(
            sz("display:flex;width:50px", css, O).0,
            chrome,
            "Chrome 155: [{css}] {chrome}"
        );
    }
}

/// The measure must not read a label cached before a `px` width stopped
/// re-shaping it: incremental and fresh agree, and both are Chrome's 302.
#[test]
fn a_label_added_under_a_px_width_is_measured_incrementally_as_fresh() {
    let mut doc = document();
    let (_, sel) = build(&mut doc, "display:flex;width:50px", "", O);
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(sel, "style", "font: 16px/20px ProbeFace; width: 400px");
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let t = doc.create_text("Hello world option Hello world option");
    doc.append_child(o, t);
    doc.resolve_layout(801.0, 600.0);
    let inc = doc.tree.get(sel.0).unwrap().layout.width;
    let mut fresh = document();
    let (_, s2) = build(
        &mut fresh,
        "display:flex;width:50px",
        "width:400px",
        &[
            "a",
            "Hello world option",
            "Hello world option Hello world option",
        ],
    );
    fresh.resolve_layout(801.0, 600.0);
    let fr = fresh.tree.get(s2.0).unwrap().layout.width;
    assert_eq!((inc, fr), (302.0, 302.0), "Chrome 155: 302");
}

/// Kills dropping `!important` from the UA `overflow: clip` (Chrome 155 computes
/// `clip` for every author value, so none of these shrinks).
#[test]
fn author_overflow_does_not_make_it_shrink() {
    for css in ["overflow: hidden", "overflow: auto", "overflow: visible"] {
        assert_eq!(
            sz("display:flex;width:50px", css, O).0,
            160.0,
            "Chrome 155: [{css}] 160"
        );
    }
}

/// `display: flex|grid` on a select is ignored (Chrome 155: 160; main: 600).
#[test]
fn an_inner_display_on_a_select_is_ignored() {
    for css in ["display: flex", "display: grid"] {
        assert_eq!(sz("", css, O).0, 160.0, "Chrome 155: [{css}] 160");
    }
}
