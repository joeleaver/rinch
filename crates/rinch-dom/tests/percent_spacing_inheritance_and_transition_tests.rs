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

/// KNOWN GAP (see review-1362 report): `tick_transitions`'s
/// `apply_value_to_style` writes only `style.font_size` on a `FontSize`
/// transition frame; it never re-derives `letter_spacing`/`word_spacing`
/// from the (px, pct) pair, because `ComputedStyle` does not retain the
/// pair after conversion. A full restyle (the class change itself)
/// re-derives both fields from the *target* font-size immediately, so a
/// percentage spacing jumps to its end value in the very first transition
/// frame while `font_size` is still near its start value — Chrome instead
/// keeps the computed value as an unresolved percentage and re-resolves it
/// every frame, so the spacing visibly tracks the animating font-size there.
/// This fixture is NOT a regression from #743 breaking anything that worked
/// before (the percentage was always silently 0 pre-#743, so there was
/// nothing to track) — it demonstrates a gap the fix exposes rather than
/// causes, worth its own follow-up.
#[test]
fn percentage_spacing_does_not_track_an_in_flight_font_size_transition_1363() {
    const CSS: &str = ".t { width: 400px; white-space: pre; font-family: monospace; \
                       font-size: 10px; letter-spacing: 50%; \
                       transition: font-size 150ms linear; } \
                       .t.big { font-size: 40px; }";

    let (mut doc, div) = texted_div(CSS, "t");
    let cs0 = doc.tree.get(div.0).unwrap().computed_style.clone();
    assert!((cs0.letter_spacing - 5.0).abs() < 0.01);

    doc.set_attribute(div, "class", "t big");
    doc.resolve_layout(800.0, 600.0);

    let cs1 = doc.tree.get(div.0).unwrap().computed_style.clone();
    // font_size is still near its START value (the transition has barely
    // ticked), but letter_spacing has already jumped to the END target
    // (40 * 0.5 = 20) rather than tracking font_size's current value
    // (10 * 0.5 = 5). This assertion documents the current (gap) behaviour;
    // it should be replaced with a tracking assertion once fixed.
    assert!(
        (cs1.font_size - 10.0).abs() < 1.0,
        "font_size should still be near its start value just after the \
         transition armed, got {}",
        cs1.font_size
    );
    assert!(
        (cs1.letter_spacing - 20.0).abs() < 0.01,
        "documents the gap: letter_spacing already reads the END target \
         (20.0) while font_size ({}) has barely moved from its start (10.0) \
         -- it does not track the animating font-size the way Chrome's \
         unresolved-percentage model does. got letter_spacing={}",
        cs1.font_size,
        cs1.letter_spacing
    );
}
