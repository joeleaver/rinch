//! #743: a percentage spacing is inherited as a percentage and resolved
//! against the inheriting element's own font-size (Chrome 153, measured);
//! negative percentages; `em` units; and the pinned gap #1363 — percentage
//! spacing does not follow an in-flight font-size transition.

use rinch_core::dom::DomDocument;
use rinch_core::dom::NodeId;
use rinch_dom::RinchDocument;

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

/// Chrome 153 (measured, `inherit-1362c.html`): a child with no own
/// `letter-spacing` inherits the *percentage itself* (getComputedStyle still
/// answers "10%" on the child), which is then resolved against the CHILD's
/// own font-size (40px), not the parent's (20px) — delta measured as 20px
/// over 5 characters (4px each), not 10px (2px each).
#[test]
fn inheritance_resolves_against_the_inheriting_elements_own_font_size() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = el(
        &mut doc,
        body,
        "div",
        "font-size: 20px; letter-spacing: 10%; white-space: pre; font-family: monospace;",
    );
    let child = el(&mut doc, parent, "span", "font-size: 40px;");
    txt(&mut doc, child, "abcde");
    doc.resolve_layout(800.0, 600.0);
    let cs = &doc.tree.get(child.0).unwrap().computed_style;
    assert!(
        (cs.letter_spacing - 4.0).abs() < 0.01,
        "expected 4.0 (10% re-resolved against the child's own 40px font-size, \
         matching Chrome 153), got {}",
        cs.letter_spacing
    );
}

#[test]
fn negative_percentage_resolves_negative() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "font-size: 20px; letter-spacing: -50%; white-space: pre; font-family: monospace;",
    );
    txt(&mut doc, c, "abcde");
    doc.resolve_layout(800.0, 600.0);
    let cs = &doc.tree.get(c.0).unwrap().computed_style;
    assert!(
        (cs.letter_spacing - (-10.0)).abs() < 0.01,
        "expected -10.0, got {}",
        cs.letter_spacing
    );
}

#[test]
fn em_unit_still_resolves_as_before() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "font-size: 20px; letter-spacing: 0.5em; white-space: pre; font-family: monospace;",
    );
    txt(&mut doc, c, "abcde");
    doc.resolve_layout(800.0, 600.0);
    let cs = &doc.tree.get(c.0).unwrap().computed_style;
    assert!(
        (cs.letter_spacing - 10.0).abs() < 0.01,
        "expected 10.0 (0.5em of 20px), got {}",
        cs.letter_spacing
    );
}

fn texted_div(css: &str, class: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let sheet = doc.create_text(css);
    doc.append_child(style_el, sheet);
    doc.append_child(body, style_el);

    let div = doc.create_element("div");
    doc.set_attribute(div, "class", class);
    let text = doc.create_text("abcde");
    doc.append_child(div, text);
    doc.append_child(body, div);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, div)
}

/// A percentage `letter-spacing` tracks an in-flight `font-size` transition,
/// as Chrome resolves the unresolved percentage against the animated size
/// every frame. It used to jump to the end target (40 * 50% = 20) on the
/// first frame while `font_size` was still near 10 (#1363's known gap); since
/// #787 the transition's `font-size` sample is in the cascade, so the
/// percentage is resolved against it.
#[test]
fn percentage_spacing_tracks_an_in_flight_font_size_transition_1363() {
    const CSS: &str = ".t { width: 400px; white-space: pre; font-family: monospace; \
                       font-size: 10px; letter-spacing: 50%; \
                       transition: font-size 150ms linear; } \
                       .t.big { font-size: 40px; }";

    let (mut doc, div) = texted_div(CSS, "t");
    let cs0 = doc.tree.get(div.0).unwrap().computed_style.clone();
    assert!((cs0.letter_spacing - 5.0).abs() < 0.01);

    // Pinned, so the cascade's sample and the tick's are of one instant
    // (#2267); a font-size of 10.3 is still well short of the end target.
    doc.set_animation_clock_ms(Some(1_000_000.0));
    doc.set_attribute(div, "class", "t big");
    doc.resolve_layout(800.0, 600.0);
    doc.set_animation_clock_ms(Some(1_000_000.0 + 3.0));
    doc.tick_transitions();
    doc.resolve_layout(800.0, 600.0);

    let cs1 = doc.tree.get(div.0).unwrap().computed_style.clone();
    // font_size is still near its START value (the transition has barely
    // ticked), and letter_spacing is half of it, not the END target (20).
    assert!(
        (cs1.font_size - 10.0).abs() < 1.0,
        "font_size should still be near its start value just after the \
         transition armed, got {}",
        cs1.font_size
    );
    assert!(
        (cs1.letter_spacing - cs1.font_size * 0.5).abs() < 0.02,
        "letter_spacing tracks the animated font-size ({}): got {}",
        cs1.font_size,
        cs1.letter_spacing
    );
}
