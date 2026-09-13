//! Issue #616 — `font-weight: normal` must override a UA `bold`.
//!
//! `<b>`/`<strong>` get `font-weight: bold` from the UA stylesheet. An author
//! declaration of `normal` computes to 400 and should beat it, exactly as
//! `300` or `bold` do. The same shape applies to `font-style: normal` on
//! `<em>`/`<i>` and to `text-decoration: none` on `<u>`/`<s>`.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::values::FontStyleValue;

/// `el(&mut doc, parent, tag, style)` — the shape issue #616 reports with.
fn el(
    doc: &mut RinchDocument,
    parent: rinch_core::dom::NodeId,
    tag: &str,
    style: &str,
) -> rinch_core::dom::NodeId {
    let id = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(id, "style", style);
    }
    doc.append_child(parent, id);
    id
}

fn text(doc: &mut RinchDocument, parent: rinch_core::dom::NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn weight(doc: &RinchDocument, id: rinch_core::dom::NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().computed_style.font_weight
}

fn style_of(doc: &RinchDocument, id: rinch_core::dom::NodeId) -> FontStyleValue {
    doc.tree.get(id.0).unwrap().computed_style.font_style
}

// ===== font-weight =====

#[test]
fn inline_font_weight_normal_beats_the_ua_bold() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let bare = el(&mut doc, c, "b", "");
    let normal = el(&mut doc, c, "b", "font-weight: normal");
    let numeric = el(&mut doc, c, "strong", "font-weight: 300");
    let keyword = el(&mut doc, c, "span", "font-weight: bold");
    let strong_normal = el(&mut doc, c, "strong", "font-weight: normal");
    let four_hundred = el(&mut doc, c, "b", "font-weight: 400");
    doc.resolve_layout(800.0, 600.0);

    // The UA rule still applies where nothing overrides it.
    assert_eq!(weight(&doc, bare), 700.0, "bare <b> keeps the UA bold");
    // The bug: an inline `normal` lost to the UA rule and read 700.
    assert_eq!(
        weight(&doc, normal),
        400.0,
        "`font-weight: normal` on <b> must compute to 400"
    );
    assert_eq!(
        weight(&doc, strong_normal),
        400.0,
        "`font-weight: normal` on <strong> must compute to 400"
    );
    // These already worked; they are the control for the fix not regressing.
    assert_eq!(weight(&doc, numeric), 300.0, "numeric value is honoured");
    assert_eq!(weight(&doc, keyword), 700.0, "`bold` on a <span> is 700");
    assert_eq!(
        weight(&doc, four_hundred),
        400.0,
        "the documented `font-weight: 400` workaround still works"
    );
}

#[test]
fn a_stylesheet_font_weight_normal_beats_the_ua_bold() {
    let mut doc = RinchDocument::new();
    doc.load_css(".plain { font-weight: normal; }");
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let bare = el(&mut doc, c, "b", "");
    let plain = el(&mut doc, c, "b", "");
    doc.set_attribute(plain, "class", "plain");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(weight(&doc, bare), 700.0, "bare <b> keeps the UA bold");
    assert_eq!(
        weight(&doc, plain),
        400.0,
        "an author class declaring `normal` must beat the UA bold too"
    );
}

#[test]
fn an_inherited_400_does_not_suppress_the_ua_bold() {
    // The fix must not key off "the computed value is 400" alone: a <b> whose
    // parent declares `font-weight: normal`, with no declaration of its own,
    // still gets the UA bold. This is the case the old tag fixup got right and
    // which a naive fix breaks.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrapper = el(&mut doc, body, "div", "width: 400px; font-weight: normal");
    let b = el(&mut doc, wrapper, "b", "");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        weight(&doc, b),
        700.0,
        "<b> under a `font-weight: normal` parent is still bold"
    );
}

#[test]
fn a_nested_b_inside_a_normal_b_is_bold_again() {
    // `font-weight` inherits, so the inner <b> inherits 400 and its own UA
    // rule takes it back to 700 — the browser's behaviour.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let outer = el(&mut doc, c, "b", "font-weight: normal");
    let inner = el(&mut doc, outer, "b", "");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(weight(&doc, outer), 400.0, "outer <b> is normal");
    assert_eq!(weight(&doc, inner), 700.0, "inner <b> is bold again");
}

#[test]
fn font_weight_normal_survives_a_restyle() {
    // A later `set_attribute` re-runs the cascade for that subtree; the
    // declaration must keep winning, and removing it must restore the UA bold.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let b = el(&mut doc, c, "b", "font-weight: normal");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(weight(&doc, b), 400.0, "normal wins on the first pass");

    doc.set_attribute(b, "style", "font-weight: normal; color: red");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(weight(&doc, b), 400.0, "normal still wins after a restyle");

    doc.set_attribute(b, "style", "color: red");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        weight(&doc, b),
        700.0,
        "removing the declaration restores the UA bold"
    );
}

/// Issue #616's second observation: a descendant block's text rendered at
/// normal weight while the `<b>`'s own text stayed bold.
///
/// `font-weight` inherits, and the inheritance happens inside Stylo's cascade —
/// so a descendant's computed weight comes from the `<b>`'s *cascaded* value,
/// which honoured the declaration all along. Only the `<b>`'s own
/// `ComputedStyle` was patched back to 700, after the cascade, where no
/// descendant could see it. One cause, two symptoms: the element and its
/// descendants must agree.
#[test]
fn a_descendant_agrees_with_its_b_about_the_weight() {
    fn pair(b_style: &str) -> (f32, f32) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let b = el(&mut doc, body, "b", b_style);
        let child = el(&mut doc, b, "div", "");
        text(&mut doc, child, "child");
        doc.resolve_layout(800.0, 600.0);
        (weight(&doc, b), weight(&doc, child))
    }

    let (b_bold, child_bold) = pair("");
    assert_eq!(
        (b_bold, child_bold),
        (700.0, 700.0),
        "under a bare <b>, the element and its descendant are both bold"
    );

    let (b_normal, child_normal) = pair("font-weight: normal");
    assert_eq!(
        (b_normal, child_normal),
        (400.0, 400.0),
        "under `font-weight: normal`, both are normal"
    );
}

/// A `<b style="font-weight: normal">` must measure the same as a plain
/// `<span>` with the same text, and *narrower* than a bare `<b>`. This is the
/// consumer-agreement half of #616: the computed style and the Parley text
/// style the inline layout builds must read the same weight, so a fix to the
/// computed value has to move the measured advance too.
///
/// Asserted as a comparison between the three, never against a literal — an
/// absolute advance is a pin on the local font set. The row is `display: flex`
/// so the child is a flex item measured at its own intrinsic width; a declared
/// `line-height` keeps the height a declaration rather than a glyph metric.
#[test]
fn a_normal_weight_b_measures_like_a_span_not_like_a_bold_b() {
    const SAMPLE: &str = "Wavy milliliters WWW mmm";

    fn measure(tag: &str, style: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let row = el(
            &mut doc,
            body,
            "div",
            "display: flex; line-height: 20px; font-size: 16px",
        );
        let inner = el(&mut doc, row, tag, style);
        text(&mut doc, inner, SAMPLE);
        doc.resolve_layout(800.0, 600.0);
        doc.tree.get(inner.0).unwrap().layout.width
    }

    let span = measure("span", "");
    let bold_b = measure("b", "");
    let normal_b = measure("b", "font-weight: normal");

    assert!(
        bold_b > span,
        "sanity: the bold face must measure wider than the normal one \
         (bold {bold_b}, span {span}) — otherwise this fixture discriminates nothing"
    );
    assert_eq!(
        normal_b, span,
        "a `font-weight: normal` <b> must lay out at the normal weight \
         (normal <b> {normal_b}, <span> {span}, bare <b> {bold_b})"
    );
}

// ===== Tags with no UA font-weight of their own =====

/// rinch's UA stylesheet gives `<h1>`–`<h6>` and `<th>` no `font-weight`, so
/// they are 400 with or without a declaration. Pinned so that anyone adding
/// the missing UA rules (a separate change) has to decide about #616's shape
/// for them at the same time.
#[test]
fn headings_and_th_carry_no_ua_font_weight_in_rinch() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let h1 = el(&mut doc, c, "h1", "");
    let h1_normal = el(&mut doc, c, "h1", "font-weight: normal");
    let h1_bold = el(&mut doc, c, "h1", "font-weight: bold");
    let th = el(&mut doc, c, "th", "");
    let th_normal = el(&mut doc, c, "th", "font-weight: normal");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(weight(&doc, h1), 400.0, "rinch's <h1> has no UA bold");
    assert_eq!(weight(&doc, h1_normal), 400.0);
    assert_eq!(weight(&doc, h1_bold), 700.0, "an explicit bold works");
    assert_eq!(weight(&doc, th), 400.0, "rinch's <th> has no UA bold");
    assert_eq!(weight(&doc, th_normal), 400.0);
}

// ===== The sibling properties with the same shape =====

#[test]
fn inline_font_style_normal_beats_the_ua_italic() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let bare = el(&mut doc, c, "em", "");
    let upright = el(&mut doc, c, "em", "font-style: normal");
    let i_upright = el(&mut doc, c, "i", "font-style: normal");
    let oblique = el(&mut doc, c, "em", "font-style: oblique");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        style_of(&doc, bare),
        FontStyleValue::Italic,
        "bare <em> keeps the UA italic"
    );
    assert_eq!(
        style_of(&doc, upright),
        FontStyleValue::Normal,
        "`font-style: normal` on <em> must compute to Normal"
    );
    assert_eq!(
        style_of(&doc, i_upright),
        FontStyleValue::Normal,
        "`font-style: normal` on <i> must compute to Normal"
    );
    assert_eq!(
        style_of(&doc, oblique),
        FontStyleValue::Oblique,
        "a non-italic, non-normal value is honoured"
    );
}

#[test]
fn an_inherited_upright_does_not_suppress_the_ua_italic() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrapper = el(&mut doc, body, "div", "width: 400px; font-style: normal");
    let em = el(&mut doc, wrapper, "em", "");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        style_of(&doc, em),
        FontStyleValue::Italic,
        "<em> under a `font-style: normal` parent is still italic"
    );
}

#[test]
fn text_decoration_none_beats_the_ua_underline_and_line_through() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let u_bare = el(&mut doc, c, "u", "");
    let u_none = el(&mut doc, c, "u", "text-decoration: none");
    let s_bare = el(&mut doc, c, "s", "");
    let s_none = el(&mut doc, c, "s", "text-decoration: none");
    doc.resolve_layout(800.0, 600.0);

    let deco = |id: rinch_core::dom::NodeId| {
        let d = &doc.tree.get(id.0).unwrap().computed_style.text_decoration;
        (d.underline, d.strikethrough)
    };

    assert_eq!(deco(u_bare), (true, false), "bare <u> is underlined");
    assert_eq!(
        deco(u_none),
        (false, false),
        "`text-decoration: none` on <u> must clear the underline"
    );
    assert_eq!(deco(s_bare), (false, true), "bare <s> is struck through");
    assert_eq!(
        deco(s_none),
        (false, false),
        "`text-decoration: none` on <s> must clear the strikethrough"
    );
}
