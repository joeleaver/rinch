//! #580 — `text-underline-offset`: the declaration does nothing, the plumbing
//! behind it works. Both halves are measured here.
//!
//! # The declaration is dropped before rinch ever sees it
//!
//! `text-underline-offset` is declared `engines="gecko"` in
//! `stylo-0.11.0/properties/longhands/inherited_text.mako.rs`, and stylo's
//! `build.rs` generates exactly one engine's property set. So the servo build
//! rinch uses emits **no parser entry for it at all** — `text_underline_offset`
//! appears zero times in the generated `properties.rs`, the same as the
//! documented gecko-only `scrollbar-color`, against 52 occurrences of
//! `text_decoration_line` as a positive control. Stylo therefore discards
//! `text-underline-offset: 6px` as an unknown declaration at parse time, and
//! `ComputedStyle::from_stylo` has nothing to read.
//!
//! [`stylo_drops_the_declaration_so_the_computed_field_stays_none`] is the
//! witness for that, because a grep of a build artefact is evidence that
//! evaporates. When a parse route lands it fails and says so.
//!
//! # The plumbing is live the moment the field is not `None`
//!
//! The two pixel fixtures set `ComputedStyle::text_underline_offset` directly —
//! the only way to reach the consumer while no CSS can — and show the underline
//! move. They exist because the field's two `if let Some(offset)` consumers had
//! been called "dead plumbing" whose fixture "cannot be written", and a mutant
//! deleting either of them survived the whole suite as equivalent by
//! construction. They are not equivalent; they were only unreachable.
//!
//! # The value is PARLEY's offset, not CSS's
//!
//! Measured at `font-size: 20px`, `line-height: 40px`, the top row of the
//! underline stroke:
//!
//! | `text_underline_offset` | top row |
//! |---|---|
//! | `None` (the font's own metric) | 28 |
//! | `Some(0.0)` | 26 |
//! | `Some(6.0)` | 20 |
//! | `Some(12.0)` | 14 |
//! | `Some(-6.0)` | 32 |
//!
//! `Some(0.0)` is the baseline and a **positive value moves the line UP**,
//! because the field becomes `parley::style::StyleProperty::UnderlineOffset`,
//! which *replaces* the font's underline position, and `paint/text.rs` draws it
//! at `gy - offset` from a font metric that is negative below the baseline.
//!
//! CSS `text-underline-offset: 6px` means the opposite: the *auto* position,
//! pushed 6px further **from** the text. Feeding the CSS pixels into this field
//! would draw the underline 6px above the baseline, through the glyphs (glyph
//! ink occupies rows 13..=26 in the table above). So whoever plumbs the
//! declaration through owes paint a CSS *delta* applied against
//! `run_metrics.underline_offset`, which is per-font and per-size and therefore
//! is not knowable where the style is built. That is why this file pins the
//! consumer's actual behaviour rather than a CSS expectation it does not meet.
//!
//! Only the **differences** between two offsets are asserted, never an absolute
//! row: the absolute row is a pin on the local font set, the difference is the
//! declaration.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 400.0;
const VH: f32 = 120.0;
/// Declared, so the line box is a statement rather than a font measurement.
const LINE: &str = "line-height: 40px; font-size: 20px";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

// ---------------------------------------------------------------------------
// 1. What CSS can do today: nothing.
// ---------------------------------------------------------------------------

/// Every spelling of the declaration computes `None`, on a real inline box and
/// on a boxless one alike (#574's pairing) — because Stylo's servo build does
/// not parse the property, not because of anything about the host.
///
/// **If this test fails, a parse route for `text-underline-offset` has landed.**
/// That is the fix #580 asks for, and three doc comments assert the opposite and
/// must be corrected with it: the `text_underline_offset` line in
/// `computed_style/from_stylo/mod.rs`, the field's doc in
/// `computed_style/mod.rs`, and the paragraph in
/// `RinchDocument::same_inline_text_style`. The consumer also needs the CSS
/// delta described in this file's header before the declaration means what CSS
/// says it means.
#[test]
fn stylo_drops_the_declaration_so_the_computed_field_stays_none() {
    for host in ["inline", "contents"] {
        for decl in [
            "auto", "6px", "0.5em", "50%",
            // The initial value and a zero are different declarations that
            // today land on the same `None`; both are here so a parse route
            // that handles only one of them still trips this test.
            "0px",
        ] {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let c = el(&mut doc, body, "div", &format!("width: 400px; {LINE}"));
            let s = el(
                &mut doc,
                c,
                "span",
                &format!(
                    "display: {host}; text-decoration: underline; text-underline-offset: {decl}"
                ),
            );
            txt(&mut doc, s, "Hxy");
            doc.resolve_layout(VW, VH);
            let cs = &doc.tree.nodes[s.0].computed_style;

            // Positive control. `None` is also what an unstyled node's default
            // `ComputedStyle` holds, so without proof that the style pass
            // reached this node the assertion below would pass vacuously — and
            // would keep passing if a refactor stopped styling boxless
            // elements. The span's own declaration and its inherited font size
            // are that proof.
            assert!(
                cs.text_decoration.underline,
                "the style pass did not reach the `display: {host}` span, so this                  fixture is measuring nothing"
            );
            assert_eq!(
                cs.font_size, 20.0,
                "the `display: {host}` span did not inherit its container's                  font-size, so this fixture is measuring nothing"
            );

            assert_eq!(
                cs.text_underline_offset, None,
                "`text-underline-offset: {decl}` on a `display: {host}` host reached \
                 ComputedStyle. Read this test's doc comment before changing it."
            );
        }
    }
}

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Ink per pixel row — anything painted that is not the white page.
    fn row_ink(doc: &mut RinchDocument) -> Vec<usize> {
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut cx,
        );
        let px: Vec<[u8; 4]> = painter.pixels().as_chunks::<4>().0.to_vec();
        (0..VH as usize)
            .map(|y| {
                let row = y * VW as usize;
                px[row..row + VW as usize]
                    .iter()
                    .filter(|p| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
                    .count()
            })
            .collect()
    }

    /// Make `ComputedStyle::text_underline_offset` non-`None` on `node`, which no
    /// CSS can do (see the module header), and lay out again so the inline
    /// formatting context is rebuilt with it.
    ///
    /// Clearing `text_layout` is the load-bearing half. `layout_dirty` alone only
    /// lifts `resolve_layout`'s early return; Taffy then serves the measure it
    /// cached and paint reads the `InlineLayout` already on the node, so the patched
    /// style is never consulted — measured: every offset rendered identically until
    /// the cache was dropped. Marking the node or its IFC measure leaf dirty in
    /// Taffy is **not** additionally needed (also measured).
    fn force_offsets(doc: &mut RinchDocument, ifc_root: NodeId, patches: &[(NodeId, f32)]) {
        for &(node, offset) in patches {
            doc.tree.nodes[node.0].computed_style.text_underline_offset = Some(offset);
        }
        doc.tree.nodes[ifc_root.0].text_layout = None;
        doc.tree.layout_dirty = true;
        doc.resolve_layout(VW, VH);
    }

    /// One node, the common case.
    fn force_offset(doc: &mut RinchDocument, node: NodeId, ifc_root: NodeId, offset: f32) {
        force_offsets(doc, ifc_root, &[(node, offset)]);
    }

    /// The first row the underline occupies, found by difference: the same content
    /// is built twice, once with `text-decoration: underline` and once without, and
    /// the rows that gain ink are the underline's.
    ///
    /// A threshold on ink-per-row would be a pin on the text's *width*, i.e. on the
    /// local font. A difference against the undecorated twin is exact and font-free.
    fn underline_top_row(
        build: impl Fn(bool, f32) -> (RinchDocument, NodeId),
        offset: f32,
    ) -> usize {
        let (mut with, _) = build(true, offset);
        let (mut without, _) = build(false, offset);
        let a = row_ink(&mut with);
        let b = row_ink(&mut without);
        (0..VH as usize).find(|&y| a[y] > b[y]).unwrap_or_else(|| {
            panic!(
                "no row gained ink from `text-decoration: underline`; \
                 the fixture is measuring nothing"
            )
        })
    }

    // ---------------------------------------------------------------------------
    // 2. What the plumbing does once the field is non-`None`.
    // ---------------------------------------------------------------------------

    /// The IFC root's own offset reaches parley through `root_text_style`, and the
    /// underline moves **up** by exactly the difference between two offsets.
    ///
    /// Off the fixed point deliberately: `Some(0.0)` is where "the offset is
    /// honoured" and "the offset is ignored" very nearly agree (the font's own
    /// metric is only ~2px from the baseline), so the pair measured is 0 against 12
    /// and the assertion is on the 12px difference, which no font can change.
    /// A mutant deleting the `UnderlineOffset` assignment renders both at the
    /// font's metric, making the difference 0.
    #[test]
    fn the_ifc_roots_offset_moves_its_underline_by_exactly_that_much() {
        fn build(underline: bool, offset: f32) -> (RinchDocument, NodeId) {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let dec = if underline { "underline" } else { "none" };
            let c = el(
                &mut doc,
                body,
                "div",
                &format!("width: 400px; {LINE}; color: rgb(0, 0, 0); text-decoration: {dec}"),
            );
            txt(&mut doc, c, "Hxy");
            doc.resolve_layout(VW, VH);
            force_offset(&mut doc, c, c, offset);
            (doc, c)
        }

        let at_0 = underline_top_row(build, 0.0);
        let at_12 = underline_top_row(build, 12.0);

        assert_eq!(
            at_0 as i64 - at_12 as i64,
            12,
            "a 12px larger offset must lift the underline exactly 12px \
             (rows: offset 0 -> {at_0}, offset 12 -> {at_12})"
        );
    }

    /// The same through the other consumer: an inline element's own offset, which
    /// travels as a parley style span out of `inline_style_props`.
    ///
    /// The container declares no underline and no offset, so `root_text_style`
    /// cannot be the path that delivers it — this fixture fails only if
    /// `inline_style_props` stops pushing the property.
    #[test]
    fn an_inline_elements_offset_moves_its_underline_by_exactly_that_much() {
        fn build(underline: bool, offset: f32) -> (RinchDocument, NodeId) {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let c = el(
                &mut doc,
                body,
                "div",
                &format!("width: 400px; {LINE}; color: rgb(0, 0, 0)"),
            );
            let dec = if underline { "underline" } else { "none" };
            let s = el(&mut doc, c, "span", &format!("text-decoration: {dec}"));
            txt(&mut doc, s, "Hxy");
            doc.resolve_layout(VW, VH);
            force_offset(&mut doc, s, c, offset);
            (doc, s)
        }

        let at_0 = underline_top_row(build, 0.0);
        let at_12 = underline_top_row(build, 12.0);

        assert_eq!(
            at_0 as i64 - at_12 as i64,
            12,
            "an inline element's own offset must lift its underline exactly 12px \
             (rows: offset 0 -> {at_0}, offset 12 -> {at_12})"
        );
    }

    /// The skip predicate must count the offset among the properties that make a
    /// boxless wrapper worth a style span of its own.
    ///
    /// `display: contents` generates no box, so `walk_inline_children` pushes a
    /// parley span for such a wrapper only when
    /// `RinchDocument::same_inline_text_style` says its text style differs from its
    /// parent's — rsx emits one of these wrappers per `if`/`match`/`for`, and
    /// pushing a span for every undeclared one cost +8% of the layout pass (#574).
    /// Drop the offset from that comparison and a wrapper whose *only* difference is
    /// its offset is judged identical, skipped, and its declaration silently lost.
    ///
    /// The attribution is the point of the markup: `text-decoration: underline` is
    /// declared on **both** the container and the wrapper, so the other seven
    /// compared fields are equal and the offset is the only thing that can make the
    /// wrapper "different". Without that, the wrapper would differ in
    /// `text_decoration.underline` anyway — `text-decoration-line` does not inherit,
    /// so an undeclared child computes `none` against its parent's `underline` — the
    /// span would be pushed for that reason and this fixture would stop
    /// discriminating. **Measured, not reasoned:** drop the `text-decoration`
    /// from the wrapper's style and delete the `text_underline_offset` clause
    /// from `same_inline_text_style`, and all four tests in this file pass.
    #[test]
    fn a_contents_wrapper_is_not_skipped_when_only_its_offset_differs() {
        fn build(underline: bool, offset: f32) -> (RinchDocument, NodeId) {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let dec = if underline { "underline" } else { "none" };
            let c = el(
                &mut doc,
                body,
                "div",
                &format!("width: 400px; {LINE}; color: rgb(0, 0, 0); text-decoration: {dec}"),
            );
            let w = el(
                &mut doc,
                c,
                "span",
                &format!("display: contents; text-decoration: {dec}"),
            );
            txt(&mut doc, w, "Hxy");
            doc.resolve_layout(VW, VH);
            // The container keeps offset 0; only the wrapper carries `offset`.
            force_offsets(&mut doc, c, &[(c, 0.0), (w, offset)]);
            (doc, w)
        }

        let same_as_parent = underline_top_row(build, 0.0);
        let differs = underline_top_row(build, 12.0);

        assert_eq!(
            same_as_parent as i64 - differs as i64,
            12,
            "a `display: contents` wrapper whose only difference from its parent is \
             `text_underline_offset` must still get its own style span \
             (rows: wrapper at parent's offset -> {same_as_parent}, wrapper 12px higher -> {differs})"
        );
    }
}
