//! `text-decoration-style: wavy` against the rest of CSS text decoration:
//! propagation from an ancestor, right-to-left and mixed-direction text, the
//! wave's shape and its `currentcolor` fallback (fixtures from the review of
//! #836). Local pixel oracles on a software-painted frame; every text-measured
//! fixture declares its `font-size` and `line-height`.
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 120.0;

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
fn pixels(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
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
    painter.pixels().as_chunks::<4>().0.to_vec()
}
fn reddish(px: &[[u8; 4]]) -> Vec<(usize, usize)> {
    px.iter()
        .enumerate()
        .filter(|(_, p)| p[3] > 0 && p[0] > 100 && p[0] as i16 - p[1] as i16 > 40)
        .map(|(i, _)| (i % VW as usize, i / VW as usize))
        .collect()
}
fn blueish(px: &[[u8; 4]]) -> Vec<(usize, usize)> {
    px.iter()
        .enumerate()
        .filter(|(_, p)| p[3] > 0 && p[2] > 100 && p[2] as i16 - p[0] as i16 > 40)
        .map(|(i, _)| (i % VW as usize, i / VW as usize))
        .collect()
}

/// `<div style="text-decoration: underline blue"><span wavy red>` — CSS propagates
/// the div's straight line to the span's glyphs; the span cannot remove it. So
/// both a blue straight line and a red wave must paint.
#[test]
fn a_propagated_straight_underline_survives_a_wavy_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 40px; font-size: 20px; color: black; \
         text-decoration: underline solid blue",
    );
    let s = el(&mut doc, c, "span", "text-decoration: underline wavy red");
    txt(&mut doc, s, "recieve");
    doc.resolve_layout(VW, VH);
    let px = pixels(&mut doc);
    let red = reddish(&px);
    let blue = blueish(&px);
    eprintln!("red {} blue {}", red.len(), blue.len());
    assert!(!red.is_empty(), "positive control: the wave painted");
    assert!(
        !blue.is_empty(),
        "the ancestor's straight underline was removed by the wavy child"
    );
}

/// Same, with the straight underline coming from an inline ancestor (`<u>`),
/// which is the editor's `underline` mark wrapping a spell decoration.
#[test]
fn an_inline_u_around_a_wavy_span_keeps_its_line() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 40px; font-size: 20px; color: black",
    );
    let u = el(&mut doc, c, "u", "text-decoration-color: blue");
    let s = el(&mut doc, u, "span", "text-decoration: underline wavy red");
    txt(&mut doc, s, "recieve");
    doc.resolve_layout(VW, VH);
    let px = pixels(&mut doc);
    let red = reddish(&px);
    let blue = blueish(&px);
    eprintln!("red {} blue {}", red.len(), blue.len());
    assert!(!red.is_empty(), "positive control: the wave painted");
    assert!(
        !blue.is_empty(),
        "the <u>'s underline was removed under the squiggle"
    );
}

/// A right-to-left word with a wavy underline.
#[test]
fn rtl_text_gets_a_wave() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 40px; font-size: 20px; color: black",
    );
    let s = el(&mut doc, c, "span", "text-decoration: underline wavy red");
    txt(
        &mut doc,
        s,
        "\u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{05E2}\u{05D5}\u{05DC}\u{05DD}",
    );
    doc.resolve_layout(VW, VH);
    let px = pixels(&mut doc);
    let red = reddish(&px);
    // positive control: the same text solid-underlined in red paints red.
    let mut doc2 = RinchDocument::new();
    let body2 = doc2.body();
    let c2 = el(
        &mut doc2,
        body2,
        "div",
        "width: 400px; line-height: 40px; font-size: 20px; color: black",
    );
    let s2 = el(
        &mut doc2,
        c2,
        "span",
        "text-decoration: underline solid red",
    );
    txt(
        &mut doc2,
        s2,
        "\u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{05E2}\u{05D5}\u{05DC}\u{05DD}",
    );
    doc2.resolve_layout(VW, VH);
    let solid = reddish(&pixels(&mut doc2));
    let span_x = |v: &[(usize, usize)]| (v.iter().map(|p| p.0).min(), v.iter().map(|p| p.0).max());
    eprintln!(
        "rtl wavy {} {:?}, solid {} {:?}",
        red.len(),
        span_x(&red),
        solid.len(),
        span_x(&solid)
    );
    assert!(!solid.is_empty(), "positive control");
    assert!(!red.is_empty(), "an RTL word got no squiggle at all");
    let (s0, s1) = span_x(&solid);
    let (w0, w1) = span_x(&red);
    assert!(
        w0.unwrap() + 3 >= s0.unwrap()
            && w1.unwrap() <= s1.unwrap() + 3
            && (w1.unwrap() - w0.unwrap()) + 6 >= (s1.unwrap() - s0.unwrap()),
        "the wave does not span the word the straight line spans"
    );
}

/// Mixed-direction: a Latin word followed by a Hebrew word in one wavy span.
#[test]
fn bidi_span_wave_covers_both_words() {
    let t = "abc \u{05E9}\u{05DC}\u{05D5}\u{05DD}";
    let mk = |style: &str| {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 40px; font-size: 20px; color: black",
        );
        let s = el(&mut doc, c, "span", style);
        txt(&mut doc, s, t);
        doc.resolve_layout(VW, VH);
        reddish(&pixels(&mut doc))
    };
    let solid = mk("text-decoration: underline solid red");
    let wavy = mk("text-decoration: underline wavy red");
    let w = |v: &[(usize, usize)]| {
        v.iter().map(|p| p.0).max().unwrap_or(0) - v.iter().map(|p| p.0).min().unwrap_or(0)
    };
    let lo = |v: &[(usize, usize)]| v.iter().map(|p| p.0).min().unwrap_or(0);
    let hi = |v: &[(usize, usize)]| v.iter().map(|p| p.0).max().unwrap_or(0);
    eprintln!("bidi solid width {} wavy width {}", w(&solid), w(&wavy));
    assert!(!solid.is_empty(), "positive control");
    assert!(w(&wavy) + 6 >= w(&solid), "bidi wave shorter than the text");
    assert!(
        lo(&wavy) + 3 >= lo(&solid) && hi(&wavy) <= hi(&solid) + 3,
        "bidi wave runs past the text: wave {}..{}, text {}..{}",
        lo(&wavy),
        hi(&wavy),
        lo(&solid),
        hi(&solid)
    );
}

#[test]
fn control_the_blue_line_paints_without_the_wavy_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 40px; font-size: 20px; color: black",
    );
    let u = el(&mut doc, c, "u", "text-decoration-color: blue");
    let s = el(&mut doc, u, "span", "");
    txt(&mut doc, s, "recieve");
    doc.resolve_layout(VW, VH);
    let blue = blueish(&pixels(&mut doc));
    assert!(
        !blue.is_empty(),
        "control: <u> with a blue decoration colour paints blue"
    );
}

/// The wave alternates: under the word, the topmost wave row goes up and down
/// with the 5px period. "More rows than a solid line" alone is satisfied by a
/// straight line with a hook at its start (the review's surviving mutant M4:
/// `up = !up` deleted).
#[test]
fn the_wave_alternates_along_the_word() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 40px; font-size: 20px; color: black",
    );
    let s = el(&mut doc, c, "span", "text-decoration: underline wavy red");
    txt(&mut doc, s, "recieve");
    doc.resolve_layout(VW, VH);
    let red = reddish(&pixels(&mut doc));
    assert!(!red.is_empty(), "positive control: the wave painted");
    let x0 = red.iter().map(|p| p.0).min().unwrap();
    let x1 = red.iter().map(|p| p.0).max().unwrap();
    assert!(
        x1 - x0 > 40,
        "the word is wide enough to hold several periods"
    );
    let top: Vec<usize> = (x0..=x1)
        .filter_map(|x| red.iter().filter(|p| p.0 == x).map(|p| p.1).min())
        .collect();
    let changes = top.windows(2).filter(|w| w[0] != w[1]).count();
    // ~2 changes per 5px period; a straight line with a hook changes once or twice.
    assert!(
        changes >= (x1 - x0) / 10,
        "the wave is flat: {changes} changes of its top row over {}px ({top:?})",
        x1 - x0
    );
}

/// `text-decoration-color` unset is `currentcolor`: the wave takes the text's
/// colour, not black (the review's surviving mutant M6 dropped that fallback).
#[test]
fn a_wave_with_no_decoration_colour_takes_the_text_colour() {
    // On an inline element, and on the IFC root itself — two separate sites.
    for on_root in [false, true] {
        let ink = |decl: &str| {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let root_style = format!(
                "width: 400px; line-height: 40px; font-size: 20px; color: red; {}",
                if on_root { decl } else { "" }
            );
            let c = el(&mut doc, body, "div", &root_style);
            let s = el(&mut doc, c, "span", if on_root { "" } else { decl });
            txt(&mut doc, s, "recieve");
            doc.resolve_layout(VW, VH);
            pixels(&mut doc)
        };
        let plain = ink("");
        let wavy = ink("text-decoration: underline wavy");
        // The wave's own pixels: ink in the wavy frame where the plain one has none.
        let wave: Vec<[u8; 4]> = plain
            .iter()
            .zip(&wavy)
            .filter(|(p, w)| p[3] == 0 && w[3] > 128)
            .map(|(_, w)| *w)
            .collect();
        let red = wave.iter().filter(|p| p[0] > 150 && p[1] < 80).count();
        let dark = wave.iter().filter(|p| p[0] < 80).count();
        assert!(
            wave.len() > 10,
            "positive control: the wave added ink (root={on_root})"
        );
        assert!(
            red > dark && dark == 0,
            "the wave is not the text colour (root={on_root}): {red} red, {dark} dark of {}",
            wave.len()
        );
    }
}
