//! #698 — `letter-spacing` and `word-spacing` reach layout and paint.
//!
//! Both properties parse and land in `ComputedStyle`, and before this they
//! reached exactly one producer — `ComputedStyle::build_parley_layout`, whose
//! only callers are two MCP debug tools. The IFC layout, the `TextMeasure`
//! Taffy measure, the `text-overflow: ellipsis` rebuilds and paint's on-demand
//! fallback all built their Parley styles without them, so a declaration of
//! either changed nothing on the screen, silently.
//!
//! # The numbers, measured in Chrome 150 (headless, standards mode)
//!
//! A `20px/40px monospace` `inline-block` with `white-space: pre`, width of the
//! box, `getBoundingClientRect().width`:
//!
//! | content | declaration | width | delta |
//! |---|---|---|---|
//! | `abcde` | none | 60.21875 | — |
//! | `abcde` | `letter-spacing: 4px` | 80.21875 | **+20** |
//! | `a b c` | none | 60.21875 | — |
//! | `a b c` | `word-spacing: 10px` | 80.21875 | **+20** |
//! | `a b c` | both | 100.21875 | **+40** |
//! | `ab<span>cd</span>ef` | none | 72.28125 | — |
//! | `ab<span style="letter-spacing: 3px">cd</span>ef` | | 78.28125 | **+6** |
//!
//! Two facts come out of that table and both are load-bearing here:
//!
//! - **letter-spacing is added after every character *including the last*.**
//!   Five characters at 4px is +20, not the +16 that four inter-character gaps
//!   would give. css-text-3 §8.2 spells it that way and Chrome implements it,
//!   and so does parley — `LayoutData::finish` adds the spacing to every
//!   cluster's advance. This is the fixed point the obvious fixture sits on:
//!   a test that only asserted "wider" would pass against either rule.
//! - **a span-scoped declaration covers its own characters only**, its last one
//!   included: 2 characters at 3px is +6.
//!
//! Word-spacing is added to space and no-break-space clusters, so "a b c" has
//! two of them.
//!
//! Only **differences** are asserted, never an absolute width: an absolute row
//! is a pin on the local font set, a difference is the declaration. Every
//! fixture declares `font-size` and `line-height` for the same reason.
//!
//! # Percentages are not expressed
//!
//! `letter-spacing: 50%` / `word-spacing: 50%` are font-relative at used-value
//! time and the px-only spacing rinch hands parley cannot carry them;
//! `computed_style::from_stylo::typography` keeps the length part of a mixed
//! calc and drops the percentage. `calc_tests::calc_letter_and_word_spacing_keep_px_part`
//! pins that, and it is unchanged by #698.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 400.0;
const VH: f32 = 200.0;
/// Declared, so a line box is a statement rather than a font measurement.
const BASE: &str = "font-size: 20px; line-height: 40px; font-family: monospace";

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

/// The width of the Parley line an IFC root laid out, for `text` under `extra`.
fn ifc_line_width(extra: &str, text: &str) -> f32 {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        &format!("width: 380px; white-space: pre; {BASE}; {extra}"),
    );
    txt(&mut doc, c, text);
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[c.0]
        .text_layout
        .as_ref()
        .expect("the IFC root has no inline layout — this fixture is measuring nothing")
        .layout
        .width()
}

#[test]
fn letter_spacing_widens_an_ifc_line_once_per_character() {
    let plain = ifc_line_width("", "abcde");
    let spaced = ifc_line_width("letter-spacing: 4px", "abcde");
    assert!(plain > 0.0, "positive control: the plain line has no width");
    assert!(
        (spaced - plain - 20.0).abs() < 0.5,
        "letter-spacing: 4px over 5 characters must widen the line by 20px \
         (Chrome 150: 60.21875 -> 80.21875); got {plain} -> {spaced}"
    );
}

#[test]
fn word_spacing_widens_an_ifc_line_once_per_space() {
    let plain = ifc_line_width("", "a b c");
    let spaced = ifc_line_width("word-spacing: 10px", "a b c");
    assert!(plain > 0.0, "positive control: the plain line has no width");
    assert!(
        (spaced - plain - 20.0).abs() < 0.5,
        "word-spacing: 10px over 2 spaces must widen the line by 20px \
         (Chrome 150: 60.21875 -> 80.21875); got {plain} -> {spaced}"
    );
}

#[test]
fn the_two_spacings_compose() {
    let plain = ifc_line_width("", "a b c");
    let both = ifc_line_width("letter-spacing: 4px; word-spacing: 10px", "a b c");
    assert!(
        (both - plain - 40.0).abs() < 0.5,
        "5 characters at 4px plus 2 spaces at 10px is +40px \
         (Chrome 150: 60.21875 -> 100.21875); got {plain} -> {both}"
    );
}

#[test]
fn a_span_scoped_letter_spacing_covers_only_its_own_run() {
    fn width(span_style: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!("width: 380px; white-space: pre; {BASE}"),
        );
        txt(&mut doc, c, "ab");
        let s = el(&mut doc, c, "span", span_style);
        txt(&mut doc, s, "cd");
        txt(&mut doc, c, "ef");
        doc.resolve_layout(VW, VH);
        doc.tree.nodes[c.0]
            .text_layout
            .as_ref()
            .expect("no inline layout")
            .layout
            .width()
    }
    let plain = width("");
    let scoped = width("letter-spacing: 3px");
    assert!(
        (scoped - plain - 6.0).abs() < 0.5,
        "a span-scoped letter-spacing: 3px covers its own 2 characters only, \
         so +6px (Chrome 150: 72.28125 -> 78.28125); got {plain} -> {scoped}"
    );
}

/// `normal` is a **reset**, not an absence, and it is the case a `!= 0.0` guard
/// on the push would lose.
///
/// Both properties inherit, so a span inside a spaced container carries the
/// container's value; declaring `normal` computes it back to 0, and parley only
/// hears about that if the 0 is actually pushed. Chrome 150, `20px/40px
/// monospace`, container at `letter-spacing: 7px`, content
/// `ab<span>cd</span>ef`: 114.25 with the span inheriting, 100.28125 with the
/// span at `normal` — **-14**, exactly the span's own two characters.
///
/// Every other fixture in this file sits on the fixed point where "push it" and
/// "push it only when non-zero" agree, because their spacing is non-zero
/// everywhere it is declared.
#[test]
fn a_span_declaring_normal_resets_an_inherited_letter_spacing() {
    fn width(span_style: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!("width: 380px; white-space: pre; letter-spacing: 7px; {BASE}"),
        );
        txt(&mut doc, c, "ab");
        let s = el(&mut doc, c, "span", span_style);
        txt(&mut doc, s, "cd");
        txt(&mut doc, c, "ef");
        doc.resolve_layout(VW, VH);
        doc.tree.nodes[c.0]
            .text_layout
            .as_ref()
            .expect("no inline layout")
            .layout
            .width()
    }
    let inherited = width("");
    let reset = width("letter-spacing: normal");
    assert!(
        (reset - inherited + 14.0).abs() < 0.5,
        "a span at letter-spacing: normal inside a 7px container must lose the \
         spacing on its own 2 characters, so -14px \
         (Chrome 150: 114.25 -> 100.28125); got {inherited} -> {reset}"
    );
}

#[test]
fn an_atomic_inlines_measured_box_follows_letter_spacing() {
    fn box_width(extra: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", "width: 380px");
        let b = el(
            &mut doc,
            c,
            "span",
            &format!("display: inline-block; white-space: pre; {BASE}; {extra}"),
        );
        txt(&mut doc, b, "abcde");
        doc.resolve_layout(VW, VH);
        doc.tree.nodes[b.0].layout.width
    }
    let plain = box_width("");
    let spaced = box_width("letter-spacing: 4px");
    assert!(plain > 0.0, "positive control: the plain box has no width");
    assert!(
        (spaced - plain - 20.0).abs() < 0.5,
        "an inline-block shrink-wraps its text, so letter-spacing: 4px over 5 \
         characters must widen the box by 20px; got {plain} -> {spaced}"
    );
}

#[test]
fn a_letter_spacing_restyle_relays_out_at_the_same_viewport() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 380px");
    let b = el(
        &mut doc,
        c,
        "span",
        &format!("display: inline-block; white-space: pre; {BASE}"),
    );
    txt(&mut doc, b, "abcde");
    doc.resolve_layout(VW, VH);
    let before = doc.tree.nodes[b.0].layout.width;
    assert!(before > 0.0, "positive control: nothing was laid out");

    doc.set_attribute(
        b,
        "style",
        &format!("display: inline-block; white-space: pre; {BASE}; letter-spacing: 4px"),
    );
    doc.resolve_layout(VW, VH);
    let after = doc.tree.nodes[b.0].layout.width;
    assert!(
        (after - before - 20.0).abs() < 0.5,
        "a restyle that only adds letter-spacing: 4px must re-measure the box \
         (+20px over 5 characters); got {before} -> {after}"
    );
}

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// The rightmost column carrying ink that is not the white page.
    fn rightmost_ink(doc: &mut RinchDocument) -> Option<usize> {
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
        (0..VW as usize).rev().find(|&x| {
            (0..VH as usize).any(|y| {
                let p = px[y * VW as usize + x];
                p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240)
            })
        })
    }

    fn painted_right_edge(extra: &str) -> usize {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!("width: 380px; white-space: pre; color: black; {BASE}; {extra}"),
        );
        txt(&mut doc, c, "abcde");
        doc.resolve_layout(VW, VH);
        rightmost_ink(&mut doc).expect("nothing was painted — this fixture is measuring nothing")
    }

    /// The box growing is not the glyphs moving. This is the "#661 class":
    /// a property threaded into layout and not into paint leaves the box saying
    /// one thing and the ink saying another.
    ///
    /// The **ink** moves by four steps, not five: the trailing spacing after the
    /// last character is advance with no glyph in it, so the rightmost inked
    /// column is 4 x 4px further right where the line box is 5 x 4px wider.
    #[test]
    fn paint_moves_the_glyphs_not_just_the_box() {
        let plain = painted_right_edge("");
        let spaced = painted_right_edge("letter-spacing: 4px");
        assert!(
            spaced as i32 - plain as i32 >= 14 && spaced as i32 - plain as i32 <= 18,
            "letter-spacing: 4px must push the last glyph's ink 4 steps right \
             (~16px); got {plain} -> {spaced}"
        );
    }
}
