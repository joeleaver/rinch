//! #698 — `letter-spacing` and `word-spacing` reach layout and paint.
//!
//! Both properties parse and land in `ComputedStyle`, and before this they
//! reached exactly one producer — `ComputedStyle::build_parley_layout`, whose
//! only callers are two MCP debug tools. The IFC layout, the `TextMeasure`
//! Taffy measure, the `text-overflow: ellipsis` rebuilds and paint's on-demand
//! fallback all built their Parley styles without them, so a declaration of
//! either changed nothing on the screen, silently.
//!
//! # Every fixture runs over BOTH properties, and that is not decoration
//!
//! They are threaded side by side at nine sites, so the obvious mutant reverts
//! both at once and dies on a fixture that only declares `letter-spacing`.
//! Split them and the word half is a no-op at eight of the nine — **measured**:
//! PR #744's reviewer ran the single-property mutants at full scope
//! (`cargo test -p rinch-dom -p rinch`, 78 binaries) and every `word_spacing`
//! one survived except `build_inline_layout`'s, while every `letter_spacing`
//! twin died. A refactor could have dropped eight of the nine word pushes and
//! shipped green.
//!
//! So [`CASES`] holds both, [`TEXT`] is content whose advance **both** of them
//! change, and every fixture loops. The cost is one `for` per fixture; the
//! alternative was a property threaded nine times and witnessed once.
//!
//! # The numbers, measured in Chrome 150 (headless, standards mode)
//!
//! `20px/40px monospace`, `white-space: pre`, box width of an `inline-block`.
//! `a b c` is 5 characters of which 2 are spaces, so it is measurable under
//! either declaration:
//!
//! | shape | `letter-spacing: 4px` | `word-spacing: 10px` |
//! |---|---|---|
//! | `a b c` on the container | **+20** | **+20** |
//! | the same on an inner `<span>` only | **+20** | **+20** |
//! | an inner `<span>` at `normal` inside a spaced container | **-20** | **-20** |
//!
//! (`a b c` alone is 60.21875 wide; every row above is a difference from that.)
//!
//! Two facts come out of the first row and both are load-bearing here:
//!
//! - **letter-spacing is added after every character *including the last*.**
//!   Five characters at 4px is +20, not the +16 that four inter-character gaps
//!   would give. css-text-3 §8.2 spells it that way and Chrome implements it,
//!   and so does parley — `LayoutData::finish` adds the spacing to every
//!   cluster's advance. This is the fixed point the obvious fixture sits on:
//!   a test that only asserted "wider" would pass against either rule. It is
//!   also why [`Case::ink`] is smaller than [`Case::line`] for letter-spacing
//!   and equal to it for word-spacing — the trailing step carries no glyph,
//!   and word-spacing has no trailing step to carry.
//! - **a span-scoped declaration covers its own characters only**, its last one
//!   included.
//!
//! Word-spacing is added to space and no-break-space clusters, so `a b c` has
//! two of them.
//!
//! Only **differences** are asserted, never an absolute width: an absolute row
//! is a pin on the local font set, a difference is the declaration. Every
//! fixture declares `font-size` and `line-height` for the same reason.
//!
//! # Percentages are not expressed, and #743 says why that is wrong
//!
//! `letter-spacing: 50%` / `word-spacing: 50%` compute to `0` here, because
//! `computed_style::from_stylo::typography` keeps only the length part of a
//! `LengthPercentage`. **Chrome resolves the percentage against the element's
//! own font-size** — measured, `50%` at `font-size: 20px` adds 10px per
//! character — which is a constant the conversion already holds, so the
//! comment there calling it unrepresentable is wrong about why. That is
//! issue **#743**, filed from this work and not fixed by it.
//! `calc_tests::calc_letter_and_word_spacing_keep_px_part` pins the current
//! behaviour and is the fixture to change with it.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 400.0;
const VH: f32 = 200.0;
/// Declared, so a line box is a statement rather than a font measurement.
const BASE: &str = "font-size: 20px; line-height: 40px; font-family: monospace";

/// Five characters, **two of them spaces**, so one string is measurable under
/// either property and every fixture can run over both. See the module header.
const TEXT: &str = "a b c";

/// One of the two properties, with the numbers Chrome 150 gives it over
/// [`TEXT`].
#[derive(Clone, Copy)]
struct Case {
    /// Names the property in an assertion message.
    name: &'static str,
    /// The declaration under test.
    decl: &'static str,
    /// The same property put back to `normal`, which computes to 0.
    reset: &'static str,
    /// How much wider [`TEXT`]'s line box gets: one step per affected cluster,
    /// the last one included.
    line: f32,
    /// How much further right [`TEXT`]'s **last glyph's ink** lands. One step
    /// fewer than `line` for letter-spacing, because the step after the last
    /// character is advance with no glyph in it; all of it for word-spacing,
    /// because every space in `a b c` precedes the last character.
    ink: i32,
    /// Content long enough to truncate under `text-overflow: ellipsis` in a
    /// 150px box, with enough of the affected cluster to shorten visibly.
    long: &'static str,
}

const CASES: [Case; 2] = [
    Case {
        name: "letter-spacing",
        decl: "letter-spacing: 4px",
        reset: "letter-spacing: normal",
        line: 20.0,
        ink: 16,
        long: "abcdefghijklmnopqrstuvwxyz",
    },
    Case {
        name: "word-spacing",
        decl: "word-spacing: 10px",
        reset: "word-spacing: normal",
        line: 20.0,
        ink: 20,
        long: "a b c d e f g h i j k l m n o p q r",
    },
];

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

// ── the IFC root: `build_inline_layout`'s `root_text_style` ────────────────

#[test]
fn spacing_widens_an_ifc_line_once_per_affected_cluster() {
    let plain = ifc_line_width("", TEXT);
    assert!(plain > 0.0, "positive control: the plain line has no width");
    for c in CASES {
        let spaced = ifc_line_width(c.decl, TEXT);
        assert!(
            (spaced - plain - c.line).abs() < 0.5,
            "{} must widen {TEXT:?}'s line by {}px (Chrome 150: 60.21875 -> {}); \
             got {plain} -> {spaced}",
            c.decl,
            c.line,
            60.21875 + c.line
        );
    }
}

#[test]
fn the_two_spacings_compose() {
    let plain = ifc_line_width("", TEXT);
    let both = ifc_line_width("letter-spacing: 4px; word-spacing: 10px", TEXT);
    assert!(
        (both - plain - 40.0).abs() < 0.5,
        "5 characters at 4px plus 2 spaces at 10px is +40px \
         (Chrome 150: 60.21875 -> 100.21875); got {plain} -> {both}"
    );
}

// ── the per-span properties: `inline_style_props` ──────────────────────────

/// `x <span>a b c</span> y`, where only the span carries the declaration.
fn span_scoped_line_width(span_style: &str) -> f32 {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        &format!("width: 380px; white-space: pre; {BASE}"),
    );
    txt(&mut doc, c, "x ");
    let s = el(&mut doc, c, "span", span_style);
    txt(&mut doc, s, TEXT);
    txt(&mut doc, c, " y");
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[c.0]
        .text_layout
        .as_ref()
        .expect("no inline layout")
        .layout
        .width()
}

#[test]
fn a_span_scoped_spacing_covers_only_its_own_run() {
    let plain = span_scoped_line_width("");
    for c in CASES {
        let scoped = span_scoped_line_width(c.decl);
        assert!(
            (scoped - plain - c.line).abs() < 0.5,
            "a span-scoped {} covers the span's own {TEXT:?} and nothing else, \
             so +{}px (Chrome 150, measured on this exact shape); \
             got {plain} -> {scoped}",
            c.decl,
            c.line
        );
    }
}

/// `normal` is a **reset**, not an absence, and it is the case a `!= 0.0` guard
/// on the push would lose.
///
/// Both properties inherit, so a span inside a spaced container carries the
/// container's value; declaring `normal` computes it back to 0, and parley only
/// hears about that if the 0 is actually pushed. Chrome 150 on this shape:
/// **-20** for either property, exactly the span's own contribution.
///
/// Every other fixture in this file sits on the fixed point where "push it" and
/// "push it only when non-zero" agree, because their spacing is non-zero
/// everywhere it is declared.
#[test]
fn a_span_declaring_normal_resets_an_inherited_spacing() {
    fn width(container_extra: &str, span_style: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!("width: 380px; white-space: pre; {BASE}; {container_extra}"),
        );
        txt(&mut doc, c, "x ");
        let s = el(&mut doc, c, "span", span_style);
        txt(&mut doc, s, TEXT);
        txt(&mut doc, c, " y");
        doc.resolve_layout(VW, VH);
        doc.tree.nodes[c.0]
            .text_layout
            .as_ref()
            .expect("no inline layout")
            .layout
            .width()
    }
    for c in CASES {
        let inherited = width(c.decl, "");
        let reset = width(c.decl, c.reset);
        assert!(
            (reset - inherited + c.line).abs() < 0.5,
            "a span at {} inside a container at {} must lose the spacing on its \
             own {TEXT:?}, so -{}px (Chrome 150: -20 for either property); \
             got {inherited} -> {reset}",
            c.reset,
            c.decl,
            c.line
        );
    }
}

// ── atomic inlines and invalidation ────────────────────────────────────────

#[test]
fn an_atomic_inlines_measured_box_follows_the_spacing() {
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
        txt(&mut doc, b, TEXT);
        doc.resolve_layout(VW, VH);
        doc.tree.nodes[b.0].layout.width
    }
    let plain = box_width("");
    assert!(plain > 0.0, "positive control: the plain box has no width");
    for c in CASES {
        let spaced = box_width(c.decl);
        assert!(
            (spaced - plain - c.line).abs() < 0.5,
            "an inline-block shrink-wraps its text, so {} must widen the box by \
             {}px; got {plain} -> {spaced}",
            c.decl,
            c.line
        );
    }
}

/// A restyle that changes only the spacing must re-measure the box — the
/// property has to be in **both** invalidation predicates
/// (`same_text_layout_inputs` rebuilds the glyphs, `same_measured_text_inputs`
/// re-runs Taffy), and dropping it from either one alone strands this fixture.
#[test]
fn a_spacing_restyle_relays_out_at_the_same_viewport() {
    for c in CASES {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = el(&mut doc, body, "div", "width: 380px");
        let b = el(
            &mut doc,
            container,
            "span",
            &format!("display: inline-block; white-space: pre; {BASE}"),
        );
        txt(&mut doc, b, TEXT);
        doc.resolve_layout(VW, VH);
        let before = doc.tree.nodes[b.0].layout.width;
        assert!(before > 0.0, "positive control: nothing was laid out");

        doc.set_attribute(
            b,
            "style",
            &format!(
                "display: inline-block; white-space: pre; {BASE}; {}",
                c.decl
            ),
        );
        doc.resolve_layout(VW, VH);
        let after = doc.tree.nodes[b.0].layout.width;
        assert!(
            (after - before - c.line).abs() < 0.5,
            "a restyle that only adds {} must re-measure the box (+{}px); \
             got {before} -> {after}",
            c.decl,
            c.line
        );
    }
}

// ── the `TextMeasure` context and its two consumers ────────────────────────

/// A text node that is a **flex item** is not in an inline formatting context:
/// Taffy measures it as a leaf out of its `TextMeasure` context, which carries
/// its own copy of the inherited typography. That context is the third producer
/// on the "one list", and it has two consumers — the incremental measure
/// function in `layout_engine.rs` reached here, and the one in `ifc.rs` reached
/// by [`an_inline_flex_boxs_text_is_measured_with_the_spacing`].
#[test]
fn a_flex_items_text_is_measured_with_the_spacing() {
    fn width(extra: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!("display: flex; width: 380px; white-space: pre; {BASE}; {extra}"),
        );
        txt(&mut doc, c, TEXT);
        doc.resolve_layout(VW, VH);
        let t = doc.tree.nodes[c.0].children[0];
        assert!(
            doc.tree.nodes[c.0].text_layout.is_none(),
            "this fixture wanted the TextMeasure path and got an inline \
             formatting context instead — it is measuring the wrong producer"
        );
        doc.tree.nodes[t].layout.width
    }
    let plain = width("");
    assert!(
        plain > 0.0,
        "positive control: the text measured to nothing"
    );
    for c in CASES {
        let spaced = width(c.decl);
        assert!(
            (spaced - plain - c.line).abs() < 0.5,
            "a flex item's text must be measured with {} (+{}px); \
             got {plain} -> {spaced}",
            c.decl,
            c.line
        );
    }
}

/// The other `TextMeasure` consumer: an atomic inline is measured as a Taffy
/// **compute root** through `measure_inline_blocks`, which registers its own
/// measure function in `ifc.rs`. An `inline-flex` box whose only child is text
/// reaches it, where the plain flex container above reaches the incremental one.
#[test]
fn an_inline_flex_boxs_text_is_measured_with_the_spacing() {
    fn width(extra: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", "width: 380px");
        let b = el(
            &mut doc,
            c,
            "span",
            &format!("display: inline-flex; white-space: pre; {BASE}; {extra}"),
        );
        txt(&mut doc, b, TEXT);
        doc.resolve_layout(VW, VH);
        doc.tree.nodes[b.0].layout.width
    }
    let plain = width("");
    assert!(plain > 0.0, "positive control: the box measured to nothing");
    for c in CASES {
        let spaced = width(c.decl);
        assert!(
            (spaced - plain - c.line).abs() < 0.5,
            "an inline-flex box shrink-wraps its text, so {} must widen it by \
             {}px; got {plain} -> {spaced}",
            c.decl,
            c.line
        );
    }
}

// ── the two `text-overflow: ellipsis` rebuild paths ────────────────────────

/// The number of glyphs a `parley::Layout` holds, across every line and run.
fn glyph_count(layout: &parley::layout::Layout<peniko::Brush>) -> usize {
    layout
        .lines()
        .flat_map(|line| line.items())
        .map(|item| match item {
            parley::layout::PositionedLayoutItem::GlyphRun(g) => g.glyphs().count(),
            _ => 0,
        })
        .sum()
}

/// The string the IFC-root `text-overflow: ellipsis` path truncates `text` to
/// in a 150px box under `extra`, and the width it lays that string out at.
fn ellipsis_ifc(extra: &str, text: &str) -> (String, f32) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        &format!(
            "width: 150px; overflow: hidden; white-space: nowrap; \
             text-overflow: ellipsis; {BASE}; {extra}"
        ),
    );
    txt(&mut doc, c, text);
    doc.resolve_layout(VW, VH);
    let il = doc.tree.nodes[c.0]
        .text_layout
        .as_ref()
        .expect("no inline layout");
    (il.text_content.clone(), il.layout.width())
}

/// `text-overflow: ellipsis` binary-searches for the longest prefix that fits,
/// building a throwaway layout per probe. Spacing has to be in **all** of them:
/// the prefix probes decide how much fits, and the final layout is what paints.
///
/// This is the IFC-root path (`build_ellipsis_layout`).
///
/// Two assertions, because the path has two halves and one mutant each, and
/// **the second is self-calibrating rather than a threshold**. A prefix probe
/// without the spacing keeps fitting the unspaced number of characters, so the
/// *content* stays long. A final layout without it draws the characters that
/// were chosen at exactly the width those same characters measure unspaced —
/// so laying the truncated string out again with no declaration gives the
/// mutant's answer directly, and the real one has to beat it by the spacing it
/// applied. Measured here: `abcdefgh…` is 144.369 spaced against 108.369
/// unspaced, `a b c d…` is 126.328 against 96.328.
///
/// A fixed lower bound would have had to be different per property — the two
/// truncations land 18px apart — and would have pinned the local font set.
#[test]
fn an_ifc_roots_ellipsis_truncation_is_measured_with_the_spacing() {
    for c in CASES {
        let (plain_text, _) = ellipsis_ifc("", c.long);
        let (spaced_text, spaced_w) = ellipsis_ifc(c.decl, c.long);
        assert!(
            plain_text.ends_with('\u{2026}') && spaced_text.ends_with('\u{2026}'),
            "{}: positive control — nothing was truncated at all; got \
             {plain_text:?} and {spaced_text:?}",
            c.name
        );
        assert!(
            spaced_text.chars().count() + 2 <= plain_text.chars().count(),
            "{}: spaced clusters are wider, so fewer of them fit before the \
             ellipsis; got {plain_text:?} -> {spaced_text:?}",
            c.decl
        );
        assert!(
            spaced_w <= 150.0,
            "{}: the truncation must fit the 150px box; got {spaced_w} for \
             {spaced_text:?}",
            c.decl
        );
        let unspaced = ifc_line_width("", &spaced_text);
        assert!(
            spaced_w - unspaced > 20.0,
            "{}: the final layout must shape {spaced_text:?} WITH the \
             declaration — the same string unspaced is {unspaced}, which is \
             what a final builder that dropped it would return; got {spaced_w}",
            c.decl
        );
    }
}

/// The other `text-overflow: ellipsis` path: a text node measured through its
/// `TextMeasure` context (a flex item, as above) is truncated by the
/// `ellipsis_rebuilds` loop rather than by `build_ellipsis_layout`. Same three
/// throwaway builders, a different function.
///
/// Counted in glyphs because this path keeps no truncated string — the result
/// is a bare `parley::Layout` on the text node. The reference for the width
/// half therefore comes from the IFC path, and **the coupling is asserted
/// rather than assumed**: the two paths must truncate the same input in the
/// same box to the same number of glyphs, which is a cross-path invariant worth
/// pinning on its own.
#[test]
fn a_flex_items_ellipsis_truncation_is_measured_with_the_spacing() {
    fn truncate(extra: &str, text: &str) -> (usize, f32) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!(
                "display: flex; width: 150px; overflow: hidden; \
                 white-space: nowrap; text-overflow: ellipsis; {BASE}; {extra}"
            ),
        );
        txt(&mut doc, c, text);
        doc.resolve_layout(VW, VH);
        let t = doc.tree.nodes[c.0].children[0];
        let l = doc.tree.nodes[t]
            .cached_text_parley
            .as_ref()
            .expect("the flex item's text was never cached");
        (glyph_count(l), l.width())
    }
    for c in CASES {
        let (plain_n, _) = truncate("", c.long);
        let (spaced_n, spaced_w) = truncate(c.decl, c.long);
        let full = c.long.chars().count();
        assert!(
            plain_n > 0 && plain_n < full,
            "{}: positive control — {plain_n} glyphs is not a truncation of \
             {full} characters",
            c.name
        );
        assert!(
            spaced_n + 2 <= plain_n,
            "{}: fewer spaced glyphs must fit in the same 150px box; \
             got {plain_n} -> {spaced_n}",
            c.decl
        );
        assert!(
            spaced_w <= 150.0,
            "{}: the truncation must fit the 150px box; got {spaced_w}",
            c.decl
        );

        // The reference string, and the invariant that lets it be one.
        let (ifc_text, _) = ellipsis_ifc(c.decl, c.long);
        assert_eq!(
            ifc_text.chars().count(),
            spaced_n,
            "{}: the two ellipsis paths truncated the same input in the same \
             box differently ({ifc_text:?} against {spaced_n} glyphs), so the \
             width reference below is not this layout's string",
            c.decl
        );
        let unspaced = ifc_line_width("", &ifc_text);
        assert!(
            spaced_w - unspaced > 20.0,
            "{}: the final layout must shape {ifc_text:?} WITH the declaration \
             — the same string unspaced is {unspaced}, which is what a final \
             builder that dropped it would return; got {spaced_w}",
            c.decl
        );
    }
}

// ── paint ─────────────────────────────────────────────────────────────────

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

    /// Paints `TEXT` in an IFC root under `extra` and returns the rightmost
    /// inked column. `drop_layout` reaches paint's **on-demand fallback** by
    /// throwing the root's `InlineLayout` away first.
    fn painted_right_edge(extra: &str, drop_layout: bool) -> usize {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!("width: 380px; white-space: pre; color: black; {BASE}; {extra}"),
        );
        txt(&mut doc, c, TEXT);
        doc.resolve_layout(VW, VH);
        if drop_layout {
            doc.tree
                .get_mut(c.0)
                .expect("the container exists")
                .text_layout = None;
        }
        rightmost_ink(&mut doc).expect("nothing was painted — this fixture is measuring nothing")
    }

    /// The box growing is not the glyphs moving. This is the "#661 class":
    /// a property threaded into layout and not into paint leaves the box saying
    /// one thing and the ink saying another.
    ///
    /// The two properties move the ink by **different** amounts over the same
    /// string, which is [`Case::ink`]: letter-spacing's step after the last
    /// character is advance with no glyph in it, so the ink moves four steps
    /// where the line box grew by five; word-spacing's two steps both fall
    /// before the last character, so the ink moves by all of it.
    #[test]
    fn paint_moves_the_glyphs_not_just_the_box() {
        let plain = painted_right_edge("", false);
        for c in CASES {
            let spaced = painted_right_edge(c.decl, false);
            let delta = spaced as i32 - plain as i32;
            assert!(
                (delta - c.ink).abs() <= 2,
                "{} must push the last glyph's ink about {}px right; \
                 got {plain} -> {spaced} ({delta})",
                c.decl,
                c.ink
            );
        }
    }

    /// Paint's **on-demand fallback** — the branch that builds its own Parley
    /// layout because the text node has no cached one and its IFC root has no
    /// live `InlineLayout`. It is a fifth producer, and it shapes the glyphs
    /// that reach the screen, so a property missing from it paints a box that
    /// was measured one way with ink laid out another.
    ///
    /// The state is reached by construction, exactly as
    /// `one_draw_per_box_tests::children_of_an_ifc_root_with_no_live_layout_still_paint`
    /// reaches it: drop the IFC root's layout after resolving, so paint
    /// descends to the text node instead of drawing the inline layout. That is
    /// the only route to this branch in the whole `rinch-dom` suite — without
    /// this fixture the two pushes there have no witness at all.
    #[test]
    fn the_on_demand_paint_fallback_moves_the_glyphs_too() {
        let plain = painted_right_edge("", true);
        for c in CASES {
            let spaced = painted_right_edge(c.decl, true);
            let delta = spaced as i32 - plain as i32;
            assert!(
                (delta - c.ink).abs() <= 2,
                "the fallback must shape with {} too, pushing the last glyph's \
                 ink about {}px right; got {plain} -> {spaced} ({delta})",
                c.decl,
                c.ink
            );
        }
    }
}
