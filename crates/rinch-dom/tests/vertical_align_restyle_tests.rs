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

    let before = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap(); // "B"

    // Flip the SAME element to `vertical-align: super`. Nothing else about
    // its typography changes.
    d.set_attribute(span, "style", "vertical-align: super");
    d.resolve_layout(VW, VH);

    let after = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap(); // "B"

    assert_ne!(
        before.y, after.y,
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

    assert_eq!(
        cleared.y, baseline.y,
        "clearing vertical-align at runtime left the glyph shifted \
         (stale vertical_align_spans survived the restyle)"
    );
}

/// Confirms the doc comment on `InlineLayout::vertical_align_shift_at`:
/// nested `<sub><sup>` does NOT compose (no "shift the shifted baseline").
/// The innermost span wins outright, matching the push-order / first-match
/// claim in the doc. This is a documented limitation, not something this
/// fixture argues should change — it exists to prove the documented
/// behaviour is what actually happens, not merely asserted in prose.
#[test]
fn nested_sub_in_sup_does_not_compose_the_shift() {
    use rinch_dom::text_query::glyph_bounds_for_offset;

    let mut d = doc();
    let body = d.body();
    let line = el(&mut d, body, "div", &format!("{BASE}; white-space: nowrap"));
    text(&mut d, line, "A");
    let plain = el(&mut d, line, "span", "");
    text(&mut d, plain, "B");
    let outer_sub = el(&mut d, line, "sub", "");
    let inner_sup = el(&mut d, outer_sub, "sup", "");
    text(&mut d, inner_sup, "C");
    d.resolve_layout(VW, VH);

    let base = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap(); // B
    let nested = glyph_bounds_for_offset(&d, line.0 as u64, 2).unwrap(); // C

    // `<sup>` inside `<sub>` also inherits #674's `font-size: smaller`
    // TWICE (it compounds per CLAUDE.md), so the inner sup's own
    // "parent" (the outer `<sub>`) is already at 16/1.2 = 13.3333px, not
    // 16px — the sup ratio is applied against THAT, not the line's base
    // font-size. -(405/1024) * (16/1.2) = -5.2734375. rinch's mechanism
    // takes the innermost span only (no composition of the outer sub's
    // own shift at all) — assert the ACTUAL behaviour so a change in
    // either the composition rule or the font-size-smaller compounding
    // is caught.
    let shift = nested.y - base.y;
    let sup_alone = -5.2734375_f32;
    assert!(
        (shift - sup_alone).abs() < 0.01,
        "nested <sub><sup> shift = {shift}, expected the innermost-wins \
         value {sup_alone} (sup alone) — if this fails, the shift is no \
         longer innermost-only and CLAUDE.md / the doc comment on \
         InlineLayout::vertical_align_shift_at need to catch up (or the \
         compose-the-shifted-baseline gap was closed, which is good news \
         that still needs the doc fixed)"
    );
}
