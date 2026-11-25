use super::*;

// ── The auxiliary walks (#203) ───────────────────────────────────────
//
// Same criterion as the transform block above, applied to the three walks
// that search the tree for something other than "which node was clicked":
// the wheel's scroll container on each axis, and the scrollbar strips. Each
// of them ran its own copy of the offset arithmetic with no transform and —
// for the two scroll searches — no `position: fixed` exception, so they
// could answer with a container the pointer is nowhere near. That failure is
// silent: the wheel scrolls the wrong thing, or the bar cannot be grabbed.
//
// Expected rects hand-computed from the CSS; each test also states where the
// untransformed walk would have looked and asserts that is *not* the answer.

/// A scroller with `overflow: auto` and content taller than it, so both
/// scroll searches and the scrollbar search have something to find.
fn scroller(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = child_of(doc, parent, style);
    child_of(doc, el, "width: 100%; height: 2000px");
    el
}

/// The thumb is stroked under the container's own `node_transform`
/// (`paint_scrollbars`), so the grab strip is the one on screen. Under
/// `scale(2)` that is twice as far right and twice as tall as the layout
/// rect's — the case an origin-only fix would still get wrong, because the
/// strip's *position within* the container has to scale too.
#[test]
fn scrollbar_is_grabbed_where_its_thumb_paints() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sc = scroller(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 200px; height: 100px; \
         overflow: auto; transform: scale(2); transform-origin: 0 0",
    );
    doc.resolve_layout(800.0, 600.0);

    // Layout box (0,0)-(200,100); the 16px vertical strip is x∈[184,200].
    // Doubled about the origin: painted box (0,0)-(400,200), strip
    // x∈[368,400], y∈[0,200].
    let hit =
        super::find_scrollbar_hit(&doc.tree, 390.0, 100.0).expect("the painted strip is grabbable");
    assert_eq!(hit.node_id, sc.0);
    assert!(matches!(hit.axis, super::ScrollAxis::Vertical));

    // The untransformed strip: inside the painted container, but 200px left
    // of the thumb anyone can see.
    assert!(
        super::find_scrollbar_hit(&doc.tree, 192.0, 50.0).is_none(),
        "the layout-box strip is not where the thumb is drawn"
    );
    // And just left of the painted strip is content, not the bar.
    assert!(
        super::find_scrollbar_hit(&doc.tree, 360.0, 100.0).is_none(),
        "the strip is 16 container-pixels wide, i.e. 32 painted ones"
    );
}

/// The silent-wrong-target failure. Two same-sized scrollers, one moved by a
/// `translate`: a point over the *untranslated* one used to be claimed by
/// the translated one, because it was tested first (reverse tree order) and
/// its layout box still sat there. The wheel then scrolled a container the
/// pointer was 400px away from, with nothing to show for it.
#[test]
fn wheel_over_a_translated_scroller_scrolls_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    // `still` first in DOM order, `moved` second — so `moved` is tested
    // first, and a transform-blind walk hands it every point in (0,0)-(200,200).
    let still = scroller(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 200px; height: 200px; overflow: auto",
    );
    let moved = scroller(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 200px; height: 200px; \
         overflow: auto; transform: translate(400px, 0)",
    );
    doc.resolve_layout(800.0, 600.0);

    // Over the translated scroller's painted box (400,0)-(600,200).
    assert_eq!(
        super::find_scroll_container_at_point(&doc.tree, 450.0, 100.0),
        Some(moved.0),
        "a wheel over the painted box scrolls the box it is over"
    );
    // Over the still one. The translated scroller's *layout* box also covers
    // this point, and it is tested first — this is the assertion that fails
    // without the transform.
    assert_eq!(
        super::find_scroll_container_at_point(&doc.tree, 100.0, 100.0),
        Some(still.0),
        "the translated scroller's layout box must not claim this point"
    );
}

/// The horizontal twin, one assertion deep — the two functions share
/// `descend`, so the point is that the horizontal search is wired to it too.
#[test]
fn horizontal_wheel_over_a_translated_scroller_scrolls_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let el = child_of(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 200px; height: 200px; \
         overflow-x: auto; transform: translate(400px, 0)",
    );
    child_of(&mut doc, el, "width: 2000px; height: 50px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        super::find_horizontal_scroll_container_at_point(&doc.tree, 450.0, 100.0),
        Some(el.0),
        "found at the painted box"
    );
    assert_eq!(
        super::find_horizontal_scroll_container_at_point(&doc.tree, 100.0, 100.0),
        None,
        "and not at the untransformed one"
    );
}

/// The `Fixed` exception the two scroll searches never had — wrong today
/// with no transform anywhere in sight. A fixed overlay inside a scrolled
/// container had the container's scroll subtracted from its rect, so the
/// further the page scrolled the further the overlay's own scroller drifted
/// from where it is painted, and the wheel fell through to the page behind.
#[test]
fn a_fixed_scroller_is_found_after_its_container_scrolls() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let page = scroller(
        &mut doc,
        body,
        "position: absolute; left: 0; top: 0; width: 400px; height: 300px; overflow: auto",
    );
    let overlay = scroller(
        &mut doc,
        page,
        "position: fixed; left: 20px; top: 20px; width: 200px; height: 100px; overflow: auto",
    );
    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[page.0].scroll_offset.1 = 150.0;

    // The overlay's viewport box is (20,20)-(220,120) whatever `page` does.
    assert_eq!(
        super::find_scroll_container_at_point(&doc.tree, 100.0, 60.0),
        Some(overlay.0),
        "a fixed overlay stays put while the page under it scrolls"
    );
    // Where the un-excepted walk put it: 150px up, off the top of itself.
    assert_eq!(
        super::find_scroll_container_at_point(&doc.tree, 100.0, 250.0),
        Some(page.0),
        "and the point 150px lower belongs to the page, not to the overlay"
    );
}

/// The scrollbar search kept its `Fixed` exception but composed no
/// transform, so a fixed toolbar's own scroller was grabbable only if the
/// body had no transform of its own — the same body-transform re-seed
/// `body_transform_applies_to_a_hoisted_fixed_descendant` pins for hits.
#[test]
fn a_fixed_scrollers_bar_follows_the_body_transform() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "transform: translate(40px, 30px)");
    let overlay = scroller(
        &mut doc,
        body,
        "position: fixed; left: 0; top: 0; width: 200px; height: 100px; \
         overflow: auto; z-index: 5",
    );
    doc.resolve_layout(800.0, 600.0);

    // Viewport box (0,0)-(200,100) shifted by the body's translate:
    // (40,30)-(240,130); the vertical strip is x∈[224,240].
    let hit = super::find_scrollbar_hit(&doc.tree, 232.0, 80.0)
        .expect("the bar is where the body's transform puts it");
    assert_eq!(hit.node_id, overlay.0);
    assert!(
        super::find_scrollbar_hit(&doc.tree, 192.0, 50.0).is_none(),
        "and not at the un-shifted viewport box"
    );
}

// ── #769: the wheel and the painted bar must agree about a padded or
// bordered container's overflow ──────────────────────────────────────
//
// `scrollbars()` compares the content extent against the container's
// **content** box (border box less its own padding and border);
// `find_scroll_container`/`find_horizontal_scroll_container` and their
// `_at_point` twins used to compare against the **border** box instead —
// strictly larger whenever there is padding or a border — so a container
// whose content overflowed the content box but not the border box got a
// painted, draggable bar the wheel routed straight past.

/// The exact #769 repro. `width: 200px; height: 100px; padding: 20px` has
/// a content box of 160x60; a 160x80 child overflows that content box
/// (80 > 60) but not the 100px border box (80 < 100) — the window the
/// border-box comparison missed entirely.
#[test]
fn a_padded_containers_overflow_matches_its_own_painted_bar() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "box-sizing: border-box; width: 200px; height: 100px; padding: 20px; overflow: auto",
    );
    let child = child_of(&mut doc, container, "width: 160px; height: 80px");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        rinch_dom::paint::scrollbar::scrollbars(&doc.tree, container.0, 1.0)
            .vertical
            .is_some(),
        "positive control: the bar paints (content box 160x60, child 160x80)"
    );
    assert_eq!(
        super::find_scroll_container(&doc.tree, child.0),
        Some(container.0),
        "the wheel must claim the container the bar is painted on, not \
         fall through past it to the body"
    );
    assert_eq!(
        super::find_scroll_container_at_point(&doc.tree, 10.0, 50.0),
        Some(container.0),
        "the geometric search must agree too"
    );
}

/// The horizontal twin, same shape: `width: 100px; height: 200px;
/// padding: 20px` has a content box of 60x160; an 80px-wide child
/// overflows it (80 > 60) but not the 100px border box (80 < 100).
#[test]
fn a_padded_containers_horizontal_overflow_matches_its_own_painted_bar() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "box-sizing: border-box; width: 100px; height: 200px; padding: 20px; overflow-x: auto",
    );
    let child = child_of(&mut doc, container, "width: 80px; height: 160px");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        rinch_dom::paint::scrollbar::scrollbars(&doc.tree, container.0, 1.0)
            .horizontal
            .is_some(),
        "positive control: the bar paints (content box 60x160, child 80px wide)"
    );
    assert_eq!(
        super::find_horizontal_scroll_container(&doc.tree, child.0),
        Some(container.0),
        "the wheel must claim the container the bar is painted on"
    );
    assert_eq!(
        super::find_horizontal_scroll_container_at_point(&doc.tree, 50.0, 10.0),
        Some(container.0),
        "the geometric search must agree too"
    );
}

/// The reverse fixed point, off the mutant's blind spot: content that
/// fits **both** boxes must not scroll and must not paint a bar either —
/// a padded container is not a scroller just because it has padding.
#[test]
fn a_padded_container_whose_content_fits_both_boxes_does_not_scroll() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "box-sizing: border-box; width: 200px; height: 100px; padding: 20px; overflow: auto",
    );
    let child = child_of(&mut doc, container, "width: 160px; height: 60px");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        rinch_dom::paint::scrollbar::scrollbars(&doc.tree, container.0, 1.0)
            .vertical
            .is_none(),
        "positive control: content fits the content box exactly, no bar"
    );
    assert_eq!(
        super::find_scroll_container(&doc.tree, child.0),
        None,
        "nothing above the body scrolls, and this fixture's body does not overflow"
    );
}

/// A padded `<body>` whose content overflows its content box but not the
/// viewport (its border box) must be the container the wheel scrolls,
/// exactly as `scrollbars()` paints its bar. The walk from a body child
/// meets the body as an ancestor, so this does **not** reach the
/// fallback onto `tree.body_id`; `padded_scroll_overflow_769_tests`'
/// `the_body_fallback_uses_the_content_box` (and its horizontal twin)
/// start at `<html>` for that.
#[test]
fn a_padded_bodys_overflow_matches_its_own_painted_bar() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "padding: 20px; overflow: auto");
    // Viewport 800x600 -> content box 760x560. A 580px-tall child
    // overflows that (580 > 560) but not the 600px border box (580 < 600).
    let child = child_of(&mut doc, body, "width: 100%; height: 580px");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        rinch_dom::paint::scrollbar::scrollbars(&doc.tree, body.0, 1.0)
            .vertical
            .is_some(),
        "positive control: the bar paints (content box 560px tall, child 580px)"
    );
    assert_eq!(
        super::find_scroll_container(&doc.tree, child.0),
        Some(body.0),
        "the wheel must find the body the bar is painted on"
    );
}
