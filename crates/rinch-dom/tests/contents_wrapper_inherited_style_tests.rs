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
