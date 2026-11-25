use super::*;

// ── #545: a fixed box belongs to its nearest ancestor stacking context ──
//
// The rinch-dom side of this rule is pinned in `stacking_tests.rs`, against
// a *reduced* copy of the walk below. These two exercise the production
// walk, because the exemption they turn on lives here and not in the shared
// sequence.

/// A `position: fixed` box is not clipped by the stacking context that owns
/// it — that context is not its containing block — so this root's bounds
/// gate must not apply to it.
///
/// Before #545 a fixed box only ever landed in the body's sequence, whose
/// bounds are the viewport, so the gate could never wrongly exclude one.
/// Now that it can land under a clipper, the exemption is what keeps a fixed
/// modal inside an `overflow: hidden; z-index: 1` panel tappable. Chromium
/// answers the modal at this point (`elementFromPoint`, same CSS); without
/// the exemption rinch answered the body.
#[test]
fn a_fixed_box_escapes_the_bounds_gate_of_the_context_that_owns_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let clipper = doc.create_element("div");
    doc.set_attribute(
        clipper,
        "style",
        "position: relative; z-index: 1; overflow: hidden; width: 200px; height: 200px",
    );
    doc.append_child(body, clipper);

    // Tall enough to scroll, and scrolled, so the fixture is not sitting on
    // the scroll-0 fixed point where a clip taken at the wrong origin still
    // lands in the right place.
    let tall = doc.create_element("div");
    doc.set_attribute(tall, "style", "width: 10px; height: 800px");
    doc.append_child(clipper, tall);

    let modal = doc.create_element("div");
    doc.set_attribute(
        modal,
        "style",
        "position: fixed; left: 300px; top: 300px; width: 200px; height: 200px",
    );
    doc.append_child(clipper, modal);

    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[clipper.0].scroll_offset = (0.0, 50.0);

    assert_eq!(
        hit_test(&doc.tree, 400.0, 400.0),
        Some(modal.0),
        "a point on the modal and 100px clear of the clipper still reaches it"
    );
    // Not a walk that answers `modal` for everything: a point on neither
    // resolves to the body.
    assert_eq!(hit_test(&doc.tree, 600.0, 100.0), Some(doc.body().0));
    // …and the clipper still clips its own content, so the exemption is for
    // the fixed box alone and did not open the gate for the subtree.
    assert_eq!(
        hit_test(&doc.tree, 5.0, 250.0),
        Some(doc.body().0),
        "the 800px-tall child is still clipped to the 200px box"
    );
}

/// The ordering half, through the production walk: a `z-index: 99` fixed
/// backdrop and the `z-index: 100` panel it sits under are comparable, even
/// with a stacking context between the two and the body.
///
/// The wrapper's `z-index: 1` is deliberate — at `0` it would sort with the
/// body's own content and a backdrop that wrongly escaped would land above
/// it either way, so the sample could not tell the two apart.
#[test]
fn a_fixed_backdrop_and_its_panel_are_comparable_under_a_wrapper() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let wrapper = doc.create_element("div");
    doc.set_attribute(
        wrapper,
        "style",
        "position: relative; z-index: 1; overflow: hidden; width: 400px; height: 300px",
    );
    doc.append_child(body, wrapper);

    let menu = doc.create_element("div");
    doc.set_attribute(
        menu,
        "style",
        "position: relative; width: 120px; height: 32px",
    );
    doc.append_child(wrapper, menu);

    let backdrop = doc.create_element("div");
    doc.set_attribute(
        backdrop,
        "style",
        "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 99",
    );
    doc.append_child(menu, backdrop);

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: absolute; top: 32px; left: 0; width: 160px; height: 120px; z-index: 100",
    );
    doc.append_child(menu, panel);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 80.0, 80.0),
        Some(panel.0),
        "a tap on the panel runs the panel — the 99 and the 100 are in one \
         sequence, so 100 wins"
    );
    assert_eq!(
        hit_test(&doc.tree, 300.0, 250.0),
        Some(backdrop.0),
        "…and a tap inside the wrapper but clear of the panel still reaches \
         the dismiss region, which is what a popup inside a Modal needs"
    );
}

/// #591 PR 1: `overflow: hidden` on a `<span>` must not make a positioned
/// inline-block inside it untappable.
///
/// A flowed inline element owns no box, so the clip `clip_shape` used to
/// derive from it was a `0x0` rect at its parent's origin, and the stacking
/// collector pushed that onto the chain of a positioned box hoisted out from
/// under the span whose entry carries the live chain — this `relative`
/// button included (an `absolute` truncated at a containing block above the
/// span never carried it). `hit_test_node` rejects an
/// entry whose chain does not contain the probe point, so the button could be
/// painted (its IFC draws it, and paint never consults the chain for it) and
/// never tapped. `overflow` does not apply to an inline box (css-overflow-3
/// §3), and `Node::clips_overflow` now says so; this is the real walk's pin
/// for it, beside `clip_predicate_tests`' reduced model.
#[test]
fn a_button_inside_an_overflow_hidden_span_is_still_tapped() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "width: 400px; font-size: 16px; line-height: 20px",
    );
    doc.append_child(body, container);
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "overflow: hidden");
    doc.append_child(container, span);
    let t = doc.create_text("Press ");
    doc.append_child(span, t);
    let button = doc.create_element("button");
    doc.set_attribute(
        button,
        "style",
        "position: relative; width: 60px; height: 24px",
    );
    doc.append_child(span, button);
    doc.resolve_layout(800.0, 600.0);

    let (px, py, _) =
        rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, button.0, 1.0);
    let b = doc.tree.get(button.0).unwrap();
    assert!(b.layout.width > 0.0, "precondition: the button has a box");
    let (cx, cy) = (
        px as f32 + b.layout.width / 2.0,
        py as f32 + b.layout.height / 2.0,
    );
    assert_eq!(
        hit_test(&doc.tree, cx, cy),
        Some(button.0),
        "the button is tapped at its painted centre even though its span declares overflow: hidden"
    );
}

/// #591: an absolutely positioned child of a `<span>` is tapped where it
/// paints. The box is hoisted to the body's sequence through the inline
/// element (the stacking collector descends through it), and `hit_test_node`
/// probes that sequence backwards — so once the IFC root lays the box out,
/// the tap reaches it with no hit-testing change. Under a padded container,
/// and with the absolute's position read from paint's own sum, so a stale
/// wrapper box (PR 1) or a wrong hoist offset would both show.
#[test]
fn an_absolute_inside_an_inline_is_tapped_where_it_paints() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "position: relative; width: 400px; font-size: 16px; line-height: 20px; \
         padding: 7px 11px; border: 3px solid rgb(0, 0, 0)",
    );
    doc.append_child(body, container);
    let span = doc.create_element("span");
    doc.append_child(container, span);
    let t = doc.create_text("text");
    doc.append_child(span, t);
    let abs = doc.create_element("div");
    doc.set_attribute(
        abs,
        "style",
        "position: absolute; width: 40px; height: 30px; background: rgb(255, 0, 255)",
    );
    doc.append_child(span, abs);
    let label = doc.create_text("block");
    doc.append_child(abs, label);
    let tail = doc.create_text("tail");
    doc.append_child(span, tail);
    doc.resolve_layout(800.0, 600.0);

    let (px, py, _) =
        rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, abs.0, 1.0);
    let a = doc.tree.get(abs.0).unwrap();
    assert_eq!(
        (a.layout.width, a.layout.height),
        (40.0, 30.0),
        "precondition: the absolute is laid out (#591)"
    );
    let (cx, cy) = (
        px as f32 + a.layout.width / 2.0,
        py as f32 + a.layout.height / 2.0,
    );
    let hit = hit_test(&doc.tree, cx, cy);
    assert!(
        hit == Some(abs.0) || hit == Some(label.0),
        "the absolute (or its own label) is tapped at its painted centre, got {hit:?}"
    );
    // And nothing else is: one padding+border up-left, the ghost position a
    // border-box chain would give, must not tap it.
    let off = hit_test(&doc.tree, cx - 14.0 - 40.0, cy - 10.0 - 30.0);
    assert!(
        off != Some(abs.0) && off != Some(label.0),
        "a point outside the box does not tap it, got {off:?}"
    );
}
