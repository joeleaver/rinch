//! #724: a runtime `vertical-align` change moves the glyph.
//!
//! `ComputedStyle::same_text_layout_inputs` decides whether a restyle drops an
//! IFC's `InlineLayout`, which carries `vertical_align_spans`. Without
//! `vertical_align` in it, a class toggle that changes only `vertical-align`
//! kept the old layout and the glyph never moved (found by #1359's review).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::glyph_bounds_for_offset;

const VW: f32 = 400.0;
const VH: f32 = 200.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const BASE: &str = "font-family: ProbeFace; font-size: 16px; line-height: 16px";

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
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// REGRESSION FIXTURE: a `vertical-align` change applied to an EXISTING,
/// already-laid-out element (no other style on it moves) must shift the
/// glyph on the very next `resolve_layout` — this is what "changing
/// vertical-align at runtime repaints" means. The PR's own suite never
/// exercises this path (every test builds the final tree once).
#[test]
fn a_runtime_vertical_align_change_moves_the_glyph_on_the_next_layout() {
    let mut d = doc();
    let body = d.body();
    let line = el(&mut d, body, "div", &format!("{BASE}; white-space: nowrap"));
    text(&mut d, line, "A");
    let span = el(&mut d, line, "span", ""); // starts at baseline
    text(&mut d, span, "B");
    d.resolve_layout(VW, VH);

    // The line's height: a raised span grows its line (#1357).
    let before = d.tree.get(line.0).unwrap().layout.height;

    // Flip the SAME element to `vertical-align: super`. Nothing else about
    // its typography changes.
    d.set_attribute(span, "style", "vertical-align: super");
    d.resolve_layout(VW, VH);

    let after = d.tree.get(line.0).unwrap().layout.height;

    assert_ne!(
        before, after,
        "vertical-align: super set at runtime on an already-laid-out element \
         produced NO shift — ComputedStyle::same_text_layout_inputs does not \
         compare `vertical_align`, so invalidate_text_measure_for_node never \
         runs and the IFC's cached vertical_align_spans are never rebuilt"
    );
}

/// Same shape, the other direction: start with `vertical-align: super`, then
/// clear it back to `baseline`. The glyph must return to the unshifted
/// position.
#[test]
fn clearing_vertical_align_at_runtime_returns_the_glyph_to_baseline() {
    let mut d = doc();
    let body = d.body();
    let line = el(&mut d, body, "div", &format!("{BASE}; white-space: nowrap"));
    text(&mut d, line, "A");
    let span = el(&mut d, line, "span", "vertical-align: super");
    text(&mut d, span, "B");
    d.resolve_layout(VW, VH);

    let shifted = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap();

    let control = el(&mut d, line, "span", "");
    text(&mut d, control, "C");
    d.resolve_layout(VW, VH);
    let baseline = glyph_bounds_for_offset(&d, line.0 as u64, 2).unwrap(); // "C", never shifted

    assert_ne!(
        shifted.y, baseline.y,
        "precondition: the super span really is shifted vs. a plain span"
    );

    d.set_attribute(span, "style", "");
    d.resolve_layout(VW, VH);
    let cleared = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap();
    // The control again, on the line as it is now: the raised span grew the
    // line it was on (#1357), so the line's top is not where it was.
    let baseline = glyph_bounds_for_offset(&d, line.0 as u64, 2).unwrap();

    assert_eq!(
        cleared.y, baseline.y,
        "clearing vertical-align at runtime left the glyph shifted \
         (stale vertical_align_spans survived the restyle)"
    );
}

/// Nested `<sub><sup>` composes, as CSS 2.1 §10.8.1 has it: the inner
/// `super` raises the box from its parent's baseline, which the outer `sub`
/// has already lowered (#1357; before, the innermost span's shift won
/// outright). The inner `<sup>`'s parent is the `<sub>`, at `font-size:
/// smaller` (13.33px, #674): a raise of 5.27 against the `sub`'s 4.19 drop,
/// 1.09 up in all. Chrome 157 (`A<sub><sup>C</sup></sub>`, 16px/20px): the
/// inner `<sup>` box at y 3.75. Innermost-only, it would be 4px higher.
#[test]
fn nested_sub_in_sup_composes_the_shift() {
    use rinch_dom::text_query::glyph_bounds_for_offset;

    let mut d = doc();
    let body = d.body();
    let line = el(
        &mut d,
        body,
        "div",
        "font-family: ProbeFace; font-size: 16px; line-height: 20px; white-space: nowrap",
    );
    text(&mut d, line, "A");
    let outer_sub = el(&mut d, line, "sub", "");
    let inner_sup = el(&mut d, outer_sub, "sup", "");
    text(&mut d, inner_sup, "C");
    d.resolve_layout(VW, VH);

    let nested = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap(); // C
    assert!(
        (nested.y - 3.75).abs() <= 1.0,
        "nested <sub><sup> glyph at y {}, Chrome 3.75",
        nested.y
    );
}
