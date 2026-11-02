//! Reviewer fixtures for PR #1670: the five text queries answer relative to
//! the node's BORDER box, for an IFC root and for a flex container whose
//! text is a measured leaf. Asymmetric padding/border, off the fixed point.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc_with(style: &str, text: &str) -> (RinchDocument, u64, u64) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    let body = doc.body();
    let p = doc.create_element("div");
    doc.set_attribute(p, "style", style);
    doc.append_child(body, p);
    let t = doc.create_text(text);
    doc.append_child(p, t);
    doc.resolve_layout(400.0, 600.0);
    (doc, p.0, t.0)
}

const BOX: &str = "margin: 0; width: 200px; font-family: ProbeFace; font-size: 16px; \
     line-height: 24px; padding: 11px 6px 4px 17px; border-left: 4px solid red; \
     border-top: 2px solid red;";

#[test]
fn an_ifc_root_answers_from_its_border_box() {
    let (doc, p, _) = doc_with(BOX, "abc");
    let (x, y) = doc.query_caret_position(p, 0).expect("laid out");
    assert_eq!((x, y), (21.0, 13.0), "caret at the content origin");
    let g = doc.query_glyph_bounds(p, 0).expect("a glyph");
    assert!(
        (g.x - 21.0).abs() < 0.5 && g.y >= 13.0 - 0.01,
        "glyph {g:?}"
    );
    let r = doc.query_selection_rects(p, 0, 1);
    assert_eq!(r.len(), 1);
    assert!(
        (r[0].0 - 21.0).abs() < 0.01 && (r[0].1 - 13.0).abs() < 0.01,
        "{r:?}"
    );
}

#[test]
fn a_flex_containers_measured_text_answers_from_its_border_box() {
    let style = format!("display: flex; {BOX}");
    let (doc, p, t) = doc_with(&style, "abc");
    let parent_answer = doc.query_caret_position(p, 0);
    let text_answer = doc.query_caret_position(t, 0);
    eprintln!("PROBE flex: parent {parent_answer:?} text {text_answer:?}");
    let (x, y) = parent_answer.expect("the measured text child answers");
    assert_eq!((x, y), (21.0, 13.0), "caret at the content origin");
    let g = doc.query_glyph_bounds(p, 0).expect("a glyph");
    assert!((g.x - 21.0).abs() < 0.5, "glyph {g:?}");
}
