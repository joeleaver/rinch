//! Offsets into the collapsed text of an IFC (#1180): pins from the review of
//! #1187 for a range starting at a removed space, the two offset-map clamps,
//! and the root's wavy-underline end. Bundled Inter, declared line-height; no
//! geometry asserted.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::{dom_cursor_to_ifc_offset, ifc_offset_to_dom_cursor};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn root(style: &str) -> (RinchDocument, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    let body = d.body();
    let p = d.create_element("div");
    d.set_attribute(
        p,
        "style",
        &format!("width:300px;font:16px/25px ProbeFace;{style}"),
    );
    d.append_child(body, p);
    (d, p)
}

/// `a<span style="background:red"> </span><br>b`: the span's only content is
/// the held space the `<br>` removes, so its background range is empty *at*
/// 1 — not `0..1`, which paints red behind `a` (a `<=` in `IfcText::remap`).
#[test]
fn a_range_starting_at_a_removed_space_stays_where_it_was() {
    let (mut d, p) = root("");
    let a = d.create_text("a");
    d.append_child(p, a);
    let s = d.create_element("span");
    d.set_attribute(s, "style", "background:red");
    let sp = d.create_text(" ");
    d.append_child(s, sp);
    d.append_child(p, s);
    let br = d.create_element("br");
    d.append_child(p, br);
    let b = d.create_text("b");
    d.append_child(p, b);
    d.resolve_layout(800.0, 600.0);
    let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.text_content, "a\nb");
    let bg: Vec<_> = il
        .background_spans
        .iter()
        .map(|b| (b.start, b.end))
        .collect();
    assert_eq!(bg, vec![(1, 1)]);
    let sp_range = il.text_ranges.iter().find(|r| r.node_id == sp.0).unwrap();
    assert_eq!((sp_range.flat_start, sp_range.flat_end), (1, 1));
}

/// Collapsed runs and expanded tabs, both directions: a DOM offset inside a
/// collapsed run maps to the end of what was kept (`flat_for_dom`'s clamp),
/// and a flat offset inside an expanded tab rounds up to after the tab, as the
/// old tab walk did (`dom_for_flat`'s clamp).
#[test]
fn offset_maps_clamp_inside_collapsed_runs_and_tabs() {
    let (mut d, p) = root("");
    let t = d.create_text("a\t\tb   c\td");
    d.append_child(p, t);
    d.resolve_layout(800.0, 600.0);
    let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.text_content, "a b c d");
    let d2f: Vec<_> = (0..=10)
        .map(|o| dom_cursor_to_ifc_offset(&il.text_ranges, t.0, o).unwrap())
        .collect();
    assert_eq!(d2f, vec![0, 1, 2, 2, 3, 4, 4, 4, 5, 6, 7]);
    let f2d: Vec<_> = (0..=7)
        .map(|f| {
            ifc_offset_to_dom_cursor(&il.text_ranges, f, false)
                .unwrap()
                .1
        })
        .collect();
    assert_eq!(f2d, vec![0, 1, 3, 4, 7, 8, 9, 10]);

    let (mut d, p) = root("white-space:pre-wrap");
    let t = d.create_text("a\t\tb");
    d.append_child(p, t);
    d.resolve_layout(800.0, 600.0);
    let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.text_content, "a        b");
    let f2d: Vec<_> = (0..=10)
        .map(|f| {
            ifc_offset_to_dom_cursor(&il.text_ranges, f, false)
                .unwrap()
                .1
        })
        .collect();
    assert_eq!(f2d, vec![0, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4]);
}

/// The IFC root's own wavy underline ends at the end of the laid-out text,
/// not one byte past it for a trailing space `finish` removed.
#[test]
fn the_roots_wavy_underline_ends_at_the_collapsed_texts_end() {
    let (mut d, p) = root("text-decoration: underline wavy red");
    let t = d.create_text("ab ");
    d.append_child(p, t);
    d.resolve_layout(800.0, 600.0);
    let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.text_content, "ab");
    let root_deco = il
        .decoration_spans
        .last()
        .expect("the root's wavy underline");
    assert_eq!((root_deco.start, root_deco.end), (0, 2));
}
