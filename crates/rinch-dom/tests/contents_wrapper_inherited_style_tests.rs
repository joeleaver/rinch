//! A `display: contents` wrapper's inherited text properties reach the text it
//! wraps (#574).
//!
//! `walk_inline_children`'s `InlineFlowRole::Contents` arm recursed into a
//! transparent wrapper **without pushing a style span**, where its
//! `display: inline` neighbour two matches above builds one (font size, weight,
//! style, colour, decoration, underline offset, line height) and pushes it. A
//! boxless element generates no box but is still in the inheritance chain, so
//! its children inherit from it — CSS is unambiguous about that, and every one
//! of those declarations was silently dropped for any inline content that
//! flowed into an IFC through such a wrapper.
//!
//! **The oracle is a real `<span>` carrying the same declaration**, built in
//! the same test. That is what makes these assertions numbers rather than
//! opinions: a boxless wrapper and an inline box must style their text
//! identically, because the only thing that differs between them is a box, and
//! none of these properties is about a box. A third arm — a wrapper carrying no
//! declaration at all — is the other half of the pin: without it, "always push
//! the container's own style" would pass every oracle comparison here.
//!
//! **This is not a colour bug.** It was found through colour and it is measured
//! here through four properties, because the loss is of the whole span: the
//! `font-size` case changes glyph *metrics*, not just palette, and is the one
//! that would have shipped as a layout defect.
//!
//! **Every fixture here uses an all-inline container**, where the wrapper's text
//! flows into the container's own IFC. The other shape — a *mixed* container,
//! where the run is boxed in an anonymous block box — is deliberately not
//! pinned here: on `main` the wrapper's text is laid out there as its own Taffy
//! block, which takes the wrapper's style by another route and renders
//! *correctly by accident* on the wrong line, so a fixture for it would pin the
//! accident rather than this fix. It belongs with #568, which is what puts that
//! text on the right line and thereby routes it through this code at all.
//!
//! **A residual gap this does not fix, so that nobody discovers it and blames
//! this change**: rinch does not grow a line box to fit a larger inline font or
//! line-height *shared with other text on the line*. A wrapper at
//! `line-height: 60px` beside plain text leaves the container at 22px — and so
//! does a **real `<span>`** carrying the same declaration. Identical behaviour,
//! which is exactly this file's claim; the line-box gap is pre-existing, shared
//! with real inline boxes, and owned by neither #574 nor #568. Where the styled
//! element *is* the whole line, both do raise it, to 60 — which is why the
//! `line-height` fixture below is built that way.

#![cfg(feature = "software-renderer")]

use parley::fontique::{Blob, FontInfoOverride, GenericFamily};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn pixels(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
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

/// Pixels that are recognisably red.
fn red(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| p[0] > 150 && p[1] < 80 && p[2] < 80 && p[3] > 0)
        .count()
}

/// Glyph coverage: any pixel darker than the white ground. A heavier weight, a
/// larger size and an underline all raise it; nothing else in these fixtures
/// draws.
fn ink(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| p[3] > 0 && (p[0] as u16 + p[1] as u16 + p[2] as u16) < 700)
        .count()
}

/// Which element carries the declaration under test.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Host {
    /// The subject: a boxless `display: contents` wrapper.
    Wrapper,
    /// The oracle: a real inline box carrying the same declaration.
    Span,
    /// The control: a wrapper carrying nothing, so the declaration is absent.
    None,
}

/// `<div>before <HOST>MIDDLE</HOST> after[ <div/>]</div>`, painted.
///
/// With `mixed`, a block sibling makes the container mixed content, so the
/// inline run is boxed in an anonymous block box (#568) rather than flowing
/// into the container's own IFC.
fn painted(host: Host, decl: &str, mixed: bool) -> Vec<[u8; 4]> {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 20px; font-size: 16px; color: black",
    );
    txt(&mut doc, c, "before ");
    let style = match host {
        Host::Wrapper => format!("display: contents; {decl}"),
        Host::Span => decl.to_string(),
        Host::None => "display: contents".to_string(),
    };
    let w = el(&mut doc, c, "span", &style);
    txt(&mut doc, w, "MIDDLE");
    txt(&mut doc, c, " after");
    if mixed {
        el(&mut doc, c, "div", "height: 10px");
    }
    doc.resolve_layout(VW, VH);
    pixels(&mut doc)
}

/// The whole shape of every fixture below: the wrapper must measure exactly
/// what the real `<span>` measures, and both must differ from the undeclared
/// control.
fn assert_matches_the_span(decl: &str, mixed: bool, measure: fn(&[[u8; 4]]) -> usize) {
    let wrapper = measure(&painted(Host::Wrapper, decl, mixed));
    let span = measure(&painted(Host::Span, decl, mixed));
    let none = measure(&painted(Host::None, decl, mixed));
    let shape = if mixed { "mixed" } else { "all-inline" };
    assert_ne!(
        span, none,
        "{decl} in a {shape} container: the fixture is not discriminating — the \
         real <span> measures the same as no declaration at all ({span})"
    );
    assert_eq!(
        wrapper, span,
        "{decl} in a {shape} container: a boxless wrapper must style its text \
         exactly as an inline box does. wrapper={wrapper}, <span>={span}, \
         undeclared={none} — a wrapper equal to `undeclared` is the whole \
         style span being dropped (#574)"
    );
}

/// The same shape with the declaration two wrappers deep, against the same
/// oracle — a real `<span>` two levels down carrying it.
fn assert_matches_the_span_nested(decl: &str, measure: fn(&[[u8; 4]]) -> usize) {
    fn build(host: Host, decl: &str) -> Vec<[u8; 4]> {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px; color: black",
        );
        txt(&mut doc, c, "before ");
        let outer = el(&mut doc, c, "span", "display: contents");
        let style = match host {
            Host::Wrapper => format!("display: contents; {decl}"),
            Host::Span => decl.to_string(),
            Host::None => "display: contents".to_string(),
        };
        let inner = el(&mut doc, outer, "span", &style);
        txt(&mut doc, inner, "MIDDLE");
        txt(&mut doc, c, " after");
        doc.resolve_layout(VW, VH);
        pixels(&mut doc)
    }
    let wrapper = measure(&build(Host::Wrapper, decl));
    let span = measure(&build(Host::Span, decl));
    let none = measure(&build(Host::None, decl));
    assert_ne!(
        span, none,
        "{decl} nested: the fixture is not discriminating ({span})"
    );
    assert_eq!(
        wrapper, span,
        "{decl} nested two wrappers deep: wrapper={wrapper}, <span>={span}, \
         undeclared={none}"
    );
}

// ── colour ────────────────────────────────────────────────────────────────

/// Found through colour, so colour is pinned first — in the container's own
/// IFC.
#[test]
fn a_wrappers_colour_reaches_its_text_in_an_all_inline_container() {
    assert_matches_the_span("color: red", false, red);
}

/// And through a **chain** of wrappers — one level is where "push the wrapper's
/// style" and "push the nearest declaration" agree.
#[test]
fn a_wrappers_colour_reaches_a_nested_wrappers_text() {
    // Two wrappers deep: the chain must not swallow the declaration.
    assert_matches_the_span_nested("color: red", red);
}

// ── and it is not about colour ────────────────────────────────────────────

/// `font-weight` rides the same span. Without a non-colour property here, "push
/// only the brush" would pass the two fixtures above.
#[test]
fn a_wrappers_font_weight_reaches_its_text() {
    assert_matches_the_span("font-weight: 900", false, ink);
    assert_matches_the_span_nested("font-weight: 900", ink);
}

/// `text-decoration` too — a property that draws something the glyphs do not.
#[test]
fn a_wrappers_text_decoration_reaches_its_text() {
    assert_matches_the_span("text-decoration: underline", false, ink);
    assert_matches_the_span_nested("text-decoration: underline", ink);
    // `line-through` is a **separate field** on `TextDecoration`, reachable
    // from CSS independently of `underline` (`from_stylo/typography.rs`), and
    // `inline_style_props` pushes it from its own `if`. Testing only
    // `underline` leaves that second push unpinned: deleting it survives the
    // whole workspace suite. Measured, not supposed.
    assert_matches_the_span("text-decoration: line-through", false, ink);
}

/// `font-style: italic` — a different glyph set, so a different ink coverage.
/// Added to close the last property `inline_style_props` pushes that no
/// fixture named; a mutant dropping it from the skip comparison would
/// otherwise survive.
#[test]
fn a_wrappers_font_style_reaches_its_text() {
    assert_matches_the_span("font-style: italic", false, ink);
    assert_matches_the_span_nested("font-style: italic", ink);
}

/// **`font-size` is the one that matters most**: it changes glyph metrics, so
/// dropping it is a layout defect and not a palette one. This is the fixture
/// that says #574 was never cosmetic.
#[test]
fn a_wrappers_font_size_reaches_its_text() {
    assert_matches_the_span("font-size: 32px", false, ink);
    assert_matches_the_span_nested("font-size: 32px", ink);
}

/// **`line-height`, and it is here because a mutant found it.** The span is
/// pushed only when the wrapper actually changes the text style — an
/// optimisation, because rsx emits a wrapper for every `if` / `match` / `for`
/// and virtually none declares anything — and the comparison that decides that
/// must cover every property the span carries. Dropping `line-height` from it
/// leaves a wrapper that declares *only* `line-height` silently unstyled, and
/// nothing else in this file could see it: `line-height` changes the **line
/// box**, not the glyphs, so every ink and colour oracle above is blind to it.
///
/// The container is deliberately given no `line-height` of its own, **and the
/// wrapper is the only content on the line** — measured, because rinch's line
/// box does not take the maximum of the per-span line heights on a mixed line:
/// a real `<span style="line-height: 60px">` beside plain text leaves the
/// container at 22, the same as no declaration, so an oracle built that way
/// would not discriminate and the fixture would pin nothing. (That is a
/// pre-existing property of the `display: inline` arm, not something this
/// change introduces.) With the span as the whole line it is 60 against 22.
///
/// Kills: dropping any property from `same_inline_text_style` that
/// `inline_style_props` pushes — this one for `line-height` specifically.
#[test]
fn a_wrappers_line_height_reaches_its_text() {
    /// Returns the container's height, which is what a line-height changes.
    fn container_height(host: Host) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; font-size: 16px; color: black",
        );
        let style = match host {
            Host::Wrapper => "display: contents; line-height: 60px".to_string(),
            Host::Span => "line-height: 60px".to_string(),
            Host::None => "display: contents".to_string(),
        };
        let w = el(&mut doc, c, "span", &style);
        txt(&mut doc, w, "MIDDLE");
        doc.resolve_layout(VW, VH);
        doc.tree.get(c.0).unwrap().layout.height
    }

    let wrapper = container_height(Host::Wrapper);
    let span = container_height(Host::Span);
    let none = container_height(Host::None);
    assert_ne!(
        span, none,
        "the fixture is not discriminating: a real <span> with \
         line-height: 60px measures the same as no declaration ({span})"
    );
    assert_eq!(
        wrapper, span,
        "a boxless wrapper's line-height must reach its text exactly as an \
         inline box's does: wrapper={wrapper}, <span>={span}, undeclared={none}"
    );
}

/// **`letter-spacing` and `word-spacing`, and they are here for the same reason
/// `line-height` is.** #698 gave `inline_style_props` the pair, so the skip
/// comparison owes them a clause each — and a mutant deleting both clauses
/// survived every fixture in the `rinch-dom` suite, this file included, until
/// this one existed. Measured, not supposed.
///
/// It could not have been caught anywhere else. The skip only gates a boxless
/// wrapper and the split-inline bridge; a real `display: inline` span pushes its
/// properties unconditionally, so #698's own span fixture is blind to it. And
/// the oracles above are blind too: spacing moves the same glyphs apart, so the
/// ink count and the colour count barely change. The **line width** is what it
/// changes.
///
/// **Both properties, because one of them is not the other.** The clauses are
/// adjacent lines and the obvious mutant deletes both, which either one alone
/// would kill; PR #744's reviewer split them and found the word clause
/// unwitnessed across all 78 test binaries while its letter twin died. So each
/// row below carries content the property under test can actually move.
///
/// Chrome 150, `16px/20px` in a 400px container, `before <HOST>…</HOST> after`,
/// width of the host's text range:
///
/// | declaration | content | wrapper | real `<span>` |
/// |---|---|---|---|
/// | `letter-spacing: 8px` | `MIDDLE` (6 characters) | +48 | +48 |
/// | `word-spacing: 9px` | `M I D` (2 spaces) | +18 | +18 |
///
/// Kills: dropping **either** clause from `RinchDocument::same_inline_text_style`.
#[test]
fn a_wrappers_letter_and_word_spacing_reach_its_text() {
    /// The width of the container's own inline line box, which is what spacing
    /// changes.
    fn line_width(host: Host, decl: &str, content: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px; color: black",
        );
        txt(&mut doc, c, "before ");
        let style = match host {
            Host::Wrapper => format!("display: contents; {decl}"),
            Host::Span => decl.to_string(),
            Host::None => "display: contents".to_string(),
        };
        let w = el(&mut doc, c, "span", &style);
        txt(&mut doc, w, content);
        txt(&mut doc, c, " after");
        doc.resolve_layout(VW, VH);
        doc.tree
            .get(c.0)
            .unwrap()
            .text_layout
            .as_ref()
            .expect("the container has no inline layout")
            .layout
            .width()
    }

    for (decl, content, delta) in [
        ("letter-spacing: 8px", "MIDDLE", 48.0_f32),
        ("word-spacing: 9px", "M I D", 18.0),
    ] {
        let wrapper = line_width(Host::Wrapper, decl, content);
        let span = line_width(Host::Span, decl, content);
        let none = line_width(Host::None, decl, content);
        assert!(
            (span - none - delta).abs() < 0.5,
            "the fixture is not discriminating: a real <span> at {decl} over \
             {content:?} must widen the line by {delta}px; got {none} -> {span}"
        );
        assert!(
            (wrapper - span).abs() < 0.01,
            "a boxless wrapper's {decl} must reach its text exactly as an \
             inline box's does: wrapper={wrapper}, <span>={span}, \
             undeclared={none}"
        );
    }
}

// ── the control that stops the fix over-reaching ──────────────────────────

/// A wrapper that declares nothing must change nothing.
///
/// The fix pushes a style span for **every** contents wrapper, and rsx emits
/// one for `if` / `match` / `for` and for every reactive component — a huge
/// population that declares nothing at all. Those spans carry the wrapper's
/// *inherited* values, which are the enclosing IFC's own, so the painted result
/// must be identical to the same markup with no wrapper at all.
///
/// Kills: a fix that pushes the wrapper's `ComputedStyle` defaults rather than
/// its inherited values — `font_size` or `color` resetting to an initial value
/// would show here and nowhere else in this file.
#[test]
fn a_wrapper_declaring_nothing_paints_exactly_like_no_wrapper() {
    fn unwrapped() -> Vec<[u8; 4]> {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px; color: black",
        );
        txt(&mut doc, c, "before ");
        txt(&mut doc, c, "MIDDLE");
        txt(&mut doc, c, " after");
        doc.resolve_layout(VW, VH);
        pixels(&mut doc)
    }

    let plain = unwrapped();
    let wrapped = painted(Host::None, "", false);
    assert_eq!(
        ink(&wrapped),
        ink(&plain),
        "a wrapper with no declarations must be invisible"
    );
    assert_eq!(
        wrapped, plain,
        "and pixel-identical, not merely equal in coverage"
    );
}

/// A wrapper *inside* a wrapper: the inner declaration wins over the outer, as
/// it would between two nested `<span>`s.
///
/// One level is the arity fixed point — with a single wrapper, "push the
/// wrapper's style" and "push the nearest declaration" agree.
#[test]
fn a_nested_wrappers_declaration_overrides_the_one_around_it() {
    fn build(inner_decl: &str) -> Vec<[u8; 4]> {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px; color: black",
        );
        txt(&mut doc, c, "before ");
        let outer = el(&mut doc, c, "span", "display: contents; color: red");
        let inner = el(
            &mut doc,
            outer,
            "span",
            &format!("display: contents; {inner_decl}"),
        );
        txt(&mut doc, inner, "MIDDLE");
        txt(&mut doc, c, " after");
        doc.resolve_layout(VW, VH);
        pixels(&mut doc)
    }

    assert!(
        red(&build("")) > 0,
        "control: the outer wrapper's red reaches through the inner one"
    );
    assert_eq!(
        red(&build("color: black")),
        0,
        "the inner wrapper's own colour must win over the outer's"
    );
}

// ── font-family (#677) ──────────────────────────────────────────────────────
//
// `inline_style_props` pushed every other inherited typography property and
// never `font-family`, so a `display: inline` element's own family was
// dropped on the floor and every text inside an IFC shaped in the root's
// family regardless of what any span's computed style said — `<code>` inside
// a paragraph being the canonical, shipped case (`rinch-components`' `Code`
// and `Kbd` both declare `font-family: var(--rinch-font-family-monospace)`).
// `same_inline_text_style_but_wrap`, the skip predicate a `display: contents`
// wrapper is compared against, carried the matching hole.
//
// These fixtures use two bundled faces registered under their own blob ids
// (the technique #1204's fixtures use), rather than a CSS generic keyword
// resolved through the host's installed fonts, so the result does not depend
// on what the test machine happens to have installed.

const SANS_677: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");
const MONO_677: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

struct Faces677 {
    sans: u64,
    mono_generic: u64,
    named: u64,
}

/// `sans-serif` is Space Grotesk, the `monospace` generic and a named family
/// `"Probe677Named"` are both a second registration of Inter — two different
/// blob ids, so a glyph run's `font().data.id()` says which one shaped it.
fn document_677() -> (RinchDocument, Faces677) {
    let mut doc = RinchDocument::new();
    let collection = &mut doc.font_cx.collection;
    let mut register = |data: &'static [u8], name: &str| {
        let blob = Blob::new(std::sync::Arc::new(data));
        let id = blob.id();
        let families = collection.register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some(name),
                ..Default::default()
            }),
        );
        (id, families.into_iter().map(|(f, _)| f).collect::<Vec<_>>())
    };
    let (sans, sans_families) = register(SANS_677, "Probe677Sans");
    let (mono_generic, mono_families) = register(MONO_677, "Probe677MonoGeneric");
    let (named, _) = register(MONO_677, "Probe677Named");
    collection.set_generic_families(GenericFamily::SansSerif, sans_families.iter().copied());
    collection.set_generic_families(GenericFamily::Monospace, mono_families.iter().copied());
    (
        doc,
        Faces677 {
            sans,
            mono_generic,
            named,
        },
    )
}

/// Every font blob a line of `div`'s IFC root draws a glyph with.
fn blob_ids_677(doc: &RinchDocument, div: NodeId) -> std::collections::BTreeSet<u64> {
    let layout = doc
        .tree
        .get(div.0)
        .and_then(|n| n.text_layout.as_ref())
        .expect("div is an IFC root");
    let mut out = std::collections::BTreeSet::new();
    for line in layout.layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                out.insert(run.run().font().data.id());
            }
        }
    }
    out
}

/// `before <HOST>MIDDLE</HOST> after`, with `HOST`'s declaration under test.
/// Returns the set of blob ids every glyph run in the container's IFC used.
fn ids_for_677(host: Host, decl: &str) -> (std::collections::BTreeSet<u64>, Faces677) {
    let (mut doc, faces) = document_677();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 30px; font-size: 24px; color: black; \
         font-family: sans-serif",
    );
    txt(&mut doc, c, "before ");
    let style = match host {
        Host::Wrapper => format!("display: contents; {decl}"),
        Host::Span => decl.to_string(),
        Host::None => "display: contents".to_string(),
    };
    let w = el(&mut doc, c, "span", &style);
    txt(&mut doc, w, "MIDDLE");
    txt(&mut doc, c, " after");
    doc.resolve_layout(VW, VH);
    (blob_ids_677(&doc, c), faces)
}

/// **The primary repro: a plain `<span>`, no wrapper at all.** This is the
/// bug exactly as filed — `inline_style_props` is the only producer in play.
/// At HEAD the span's text shapes with the root's `sans` face and this fails;
/// fixed, it shapes with `named`.
#[test]
fn a_spans_own_font_family_is_not_ignored() {
    let (ids, faces) = ids_for_677(Host::Span, "font-family: \"Probe677Named\"");
    assert!(
        ids.contains(&faces.sans),
        "the root's own text (\"before \"/\" after\") must still be in the \
         root's face: {ids:?}"
    );
    assert!(
        ids.contains(&faces.named),
        "the span's own font-family must reach its text: got {ids:?}, \
         wanted the named face {} among them (#677)",
        faces.named
    );
}

/// A **generic** keyword on a span — not a named family — must resolve
/// through its own generic slot, not the root's.
#[test]
fn a_generic_font_family_on_a_span_resolves_through_its_own_slot() {
    let (ids, faces) = ids_for_677(Host::Span, "font-family: monospace");
    assert!(ids.contains(&faces.sans), "root text: {ids:?}");
    assert!(
        ids.contains(&faces.mono_generic),
        "`font-family: monospace` on a span must resolve to the `monospace` \
         generic's face, not fall through to the root's `sans-serif`: {ids:?}"
    );
}

/// `wrapper`, `span` and `none`, all built as siblings in **one** document so
/// their blob ids are comparable (a fresh `RinchDocument` mints fresh blob ids
/// on every `Blob::new`, even for byte-identical data, so two separate
/// documents' ids are never comparable to each other).
fn ids_for_three_677(
    decl: &str,
) -> (
    std::collections::BTreeSet<u64>,
    std::collections::BTreeSet<u64>,
    std::collections::BTreeSet<u64>,
    Faces677,
) {
    fn container(doc: &mut RinchDocument, body: NodeId, host: Host, decl: &str) -> NodeId {
        let c = el(
            doc,
            body,
            "div",
            "width: 400px; line-height: 30px; font-size: 24px; color: black; \
             font-family: sans-serif",
        );
        txt(doc, c, "before ");
        let style = match host {
            Host::Wrapper => format!("display: contents; {decl}"),
            Host::Span => decl.to_string(),
            Host::None => "display: contents".to_string(),
        };
        let w = el(doc, c, "span", &style);
        txt(doc, w, "MIDDLE");
        txt(doc, c, " after");
        c
    }
    let (mut doc, faces) = document_677();
    let body = doc.body();
    let wrapper_div = container(&mut doc, body, Host::Wrapper, decl);
    let span_div = container(&mut doc, body, Host::Span, decl);
    let none_div = container(&mut doc, body, Host::None, decl);
    doc.resolve_layout(VW, VH * 3.0);
    (
        blob_ids_677(&doc, wrapper_div),
        blob_ids_677(&doc, span_div),
        blob_ids_677(&doc, none_div),
        faces,
    )
}

/// The `display: contents` wrapper case — `same_inline_text_style_but_wrap`'s
/// matching hole. A boxless wrapper's `font-family` must reach its text
/// exactly as a real `<span>`'s does.
#[test]
fn a_wrappers_font_family_reaches_its_text() {
    let (wrapper, span, none, faces) = ids_for_three_677("font-family: \"Probe677Named\"");
    assert!(
        span.contains(&faces.named) && !none.contains(&faces.named),
        "the fixture is not discriminating: span={span:?}, none={none:?}"
    );
    assert_eq!(
        wrapper, span,
        "a boxless wrapper's font-family must reach its text exactly as an \
         inline box's does: wrapper={wrapper:?}, <span>={span:?}, \
         undeclared={none:?}"
    );
}

/// Two wrappers deep, against the same real-`<span>` oracle, all three built
/// as siblings in one document for the same reason [`ids_for_three_677`] is.
#[test]
fn a_wrappers_font_family_reaches_a_nested_wrappers_text() {
    fn container(doc: &mut RinchDocument, body: NodeId, host: Host) -> NodeId {
        let c = el(
            doc,
            body,
            "div",
            "width: 400px; line-height: 30px; font-size: 24px; color: black; \
             font-family: sans-serif",
        );
        txt(doc, c, "before ");
        let outer = el(doc, c, "span", "display: contents");
        let style = match host {
            Host::Wrapper => "display: contents; font-family: \"Probe677Named\"".to_string(),
            Host::Span => "font-family: \"Probe677Named\"".to_string(),
            Host::None => "display: contents".to_string(),
        };
        let inner = el(doc, outer, "span", &style);
        txt(doc, inner, "MIDDLE");
        txt(doc, c, " after");
        c
    }
    let (mut doc, faces) = document_677();
    let body = doc.body();
    let wrapper_div = container(&mut doc, body, Host::Wrapper);
    let span_div = container(&mut doc, body, Host::Span);
    let none_div = container(&mut doc, body, Host::None);
    doc.resolve_layout(VW, VH * 3.0);
    let wrapper = blob_ids_677(&doc, wrapper_div);
    let span = blob_ids_677(&doc, span_div);
    let none = blob_ids_677(&doc, none_div);
    assert!(
        span.contains(&faces.named) && !none.contains(&faces.named),
        "the fixture is not discriminating: span={span:?}, none={none:?}"
    );
    assert_eq!(
        wrapper, span,
        "font-family nested two wrappers deep: wrapper={wrapper:?}, \
         <span>={span:?}, undeclared={none:?}"
    );
}

/// The exact `<code>` mark stack the editor's default stylesheet declares
/// (`rinch-editor-view/src/styles.rs`): `<code>` inside a paragraph is the
/// canonical case the issue names. Several of its named fallbacks
/// (`"Liberation Mono"`, notably) are real fonts on common Linux hosts, so
/// this does not assert *which* face is picked — only that the span's text no
/// longer shapes in the same face as the surrounding paragraph, which is
/// exactly the bug: at HEAD every run in the container used `faces.sans`.
#[test]
fn the_editors_code_mark_stack_resolves_on_the_span_not_the_root() {
    let decl = "font-family: ui-monospace, \"SF Mono\", Menlo, Consolas, \
                \"Liberation Mono\", monospace";
    let (ids, faces) = ids_for_677(Host::Span, decl);
    assert!(ids.contains(&faces.sans), "root text: {ids:?}");
    assert!(
        ids.len() > 1,
        "a <code> mark's stack must shape its own text in a face other than \
         the surrounding paragraph's: got only {ids:?}"
    );
}

/// `text-overflow: ellipsis` takes a cheap per-line rebuild only when
/// `ellipsis_rebuild_is_faithful` — which reads `same_inline_text_style_but_wrap`
/// — says every run shares the root's text style; that rebuild strips spans
/// down to the root's own style (#1091's "every run in the root's text
/// style"). A span with its own `font-family` is therefore not faithful and
/// must take the safe "whole text, flat" rebuild instead of the fast one,
/// which would otherwise silently redraw the span's text in the root's face.
/// This does not assert which path was taken (that is `ellipsis_rebuild_is_faithful`'s
/// own business) — only that painting does not panic and the container still
/// paints *some* ink, i.e. the span is not simply dropped by whichever path
/// ellipsis truncation takes with a per-span font-family in play.
#[test]
fn a_spans_font_family_does_not_break_ellipsis_truncation() {
    let (mut doc, _faces) = document_677();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 120px; line-height: 30px; font-size: 24px; color: black; \
         font-family: sans-serif; white-space: nowrap; overflow: hidden; \
         text-overflow: ellipsis",
    );
    txt(&mut doc, c, "before ");
    let w = el(&mut doc, c, "span", "font-family: \"Probe677Named\"");
    txt(&mut doc, w, "MIDDLE and then some more text to overflow");
    txt(&mut doc, c, " after");
    doc.resolve_layout(VW, VH);
    let px = pixels(&mut doc);
    assert!(ink(&px) > 0, "ellipsis truncation must still paint text");
}
