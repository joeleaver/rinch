use super::*;

/// Absolute (border-box) origin of a node as a **plain parent-chain sum**:
/// layout offsets only, no transform composed, no `position: fixed`
/// exception, no IFC content-box offset.
///
/// This is a deliberate independent oracle, not a helper — it is the walk
/// the production code used to run, kept so the tests below can state the
/// box a transform-blind walk would have produced and assert it is *not*
/// the answer. Do not "fix" it to call `painted_element_box`: it exists
/// precisely to disagree with it. (`paint::tests::untransformed_origin`
/// serves the same purpose in rinch-dom.)
fn abs_origin(doc: &RinchDocument, node_id: u64) -> (f32, f32) {
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let body = doc.body().0;
    let mut cur = Some(node_id);
    while let Some(id) = cur {
        let n = doc.tree.get(id).unwrap();
        x += n.layout.x;
        y += n.layout.y;
        if id == body {
            break;
        }
        cur = n.parent;
    }
    (x, y)
}

/// #536: paint and hit testing must agree on which box a clipping
/// container clips to. The container's own border strip is drawn, never
/// clipped, and (since this fix) never reachable by an overflowing
/// child's content either — so a click there must resolve to the
/// container, not fall through to clipped content that happens to still
/// be geometrically under the border-box point.
///
/// The mutant this kills: reverting `check_children`'s bounds test from
/// the padding box back to the plain border box (`hit_test_node`'s
/// pre-#536 shape) makes this click descend into the overflowing child —
/// whose own border box covers the whole container including the border
/// strip — and return the child's id instead of the container's.
#[test]
fn a_click_on_a_clipping_containers_own_border_hits_the_container_not_overflowing_content() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "box-sizing: border-box; width: 100px; height: 100px; border: 10px solid blue; overflow: hidden",
    );
    doc.append_child(body, container);
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 300px; height: 300px");
    doc.append_child(container, child);
    doc.resolve_layout(800.0, 600.0);

    // (95, 60): inside the border box, inside the right border strip,
    // outside the padding box — the point #536's issue measured as
    // visually red (the child) against a browser's blue (the border).
    assert_eq!(
        hit_test(&doc.tree, 95.0, 60.0),
        Some(container.0),
        "a click on the right border strip must hit the container"
    );
    assert_eq!(
        hit_test(&doc.tree, 60.0, 95.0),
        Some(container.0),
        "...and the bottom border strip"
    );
    // Sanity: well inside the padding box, the child is still what's hit.
    assert_eq!(
        hit_test(&doc.tree, 50.0, 50.0),
        Some(child.0),
        "content area is unaffected — the child is still reachable there"
    );
}

/// #1798 review: a masked box's overflowing child is not hit outside the
/// box's border box, as in Chrome 155 (`elementFromPoint` answers the
/// page there), but is hit inside it even where the mask is transparent.
#[test]
fn a_masked_boxs_overflow_is_not_hit_outside_its_border_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let m = doc.create_element("div");
    doc.set_attribute(
        m,
        "style",
        "position: absolute; left: 20px; top: 20px; width: 100px; height: 50px; \
         mask-image: linear-gradient(90deg, #000 50%, transparent 50%)",
    );
    doc.append_child(body, m);
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 300px; height: 100px");
    doc.append_child(m, child);
    doc.resolve_layout(400.0, 400.0);
    assert_eq!(hit_test(&doc.tree, 30.0, 30.0), Some(child.0));
    assert_eq!(hit_test(&doc.tree, 100.0, 30.0), Some(child.0));
    assert_ne!(hit_test(&doc.tree, 200.0, 30.0), Some(child.0));
    assert_ne!(hit_test(&doc.tree, 30.0, 90.0), Some(child.0));
}

/// #535: `check_children`'s bounds test is per axis. A container holding
/// `overflow-x: clip; overflow-y: visible` must still let a click reach an
/// overflowing child **below** it (the open axis), while a click to its
/// **right** (the clipping axis) still falls through to the body — paint
/// and hit testing have to agree on both halves of that, not just the one
/// #536 already covers (clipping on both axes at once).
///
/// The mutant this kills: reverting `check_children` to the pre-#535 `!
/// node.clips_overflow() || <border-box-both-axes test>` shape makes the
/// vertical probe miss (it tests `y` against the container's own height
/// even though `overflow-y: visible` says not to) and return the body
/// instead of the child.
#[test]
fn a_click_past_an_open_axis_still_reaches_overflowing_content_but_the_clipped_axis_does_not() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "width: 100px; height: 100px; overflow-x: clip; overflow-y: visible",
    );
    doc.append_child(body, container);
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 200px; height: 200px");
    doc.append_child(container, child);
    doc.resolve_layout(800.0, 600.0);

    // (50, 150): inside the container's columns, below its rows — the
    // open (y) axis's overhang. `overflow-y: visible` says this is
    // reachable, as CSS paints it.
    assert_eq!(
        hit_test(&doc.tree, 50.0, 150.0),
        Some(child.0),
        "overflow-y: visible lets a click reach the vertical overhang"
    );
    // (150, 50): right of the container's columns — the clipping (x)
    // axis's overhang. `overflow-x: clip` still cuts this, whatever y
    // says, so the click falls through to the body.
    assert_eq!(
        hit_test(&doc.tree, 150.0, 50.0),
        Some(body.0),
        "overflow-x: clip still cuts the horizontal overhang"
    );
    // Sanity: inside the container's own box either way.
    assert_eq!(hit_test(&doc.tree, 50.0, 50.0), Some(child.0));
}

/// [`a_click_past_an_open_axis_still_reaches_overflowing_content_but_the_clipped_axis_does_not`],
/// mirrored: `overflow-y: clip; overflow-x: visible` reaches its
/// *horizontal* overhang and still cuts its vertical one. Pins that the
/// per-axis fix is symmetric and not an `x`-only special case.
#[test]
fn a_click_past_an_open_x_axis_still_reaches_overflowing_content_but_clipped_y_does_not() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "width: 100px; height: 100px; overflow-y: clip; overflow-x: visible",
    );
    doc.append_child(body, container);
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 200px; height: 200px");
    doc.append_child(container, child);
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 150.0, 50.0),
        Some(child.0),
        "overflow-x: visible lets a click reach the horizontal overhang"
    );
    assert_eq!(
        hit_test(&doc.tree, 50.0, 150.0),
        Some(body.0),
        "overflow-y: clip still cuts the vertical overhang"
    );
    assert_eq!(hit_test(&doc.tree, 50.0, 50.0), Some(child.0));
}

/// Regression (found while fixing #61): an inline-block button laid out in a
/// text flow (an IFC) inside a padded block container must be hit-testable
/// across its whole *painted* box. Two pre-existing bugs conspired to swallow
/// its clicks: (A) the surrounding IFC text nodes have their layout stretched
/// to the entire container and shadowed the button, and (B) hit-testing used
/// the container's border-box origin while the button is painted at the
/// content-box origin, so the hit rect was offset up-left by the padding.
#[test]
fn inline_block_in_ifc_is_hittable_over_its_painted_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    // Padding is the crux of bug (B).
    doc.set_attribute(
        container,
        "style",
        "width: 400px; padding: 20px; font-size: 16px",
    );
    doc.append_child(body, container);

    // "Click the " <button>OK</button> " now" — text on BOTH sides so a
    // full-container text node exists after the button in DOM order (bug A).
    let before = doc.create_text("Click the ");
    doc.append_child(container, before);
    let button = doc.create_element("button"); // inline-block by default
    doc.set_attribute(button, "style", "width: 60px; height: 24px");
    doc.append_child(container, button);
    let label = doc.create_text("OK");
    doc.append_child(button, label);
    let after = doc.create_text(" now");
    doc.append_child(container, after);

    doc.resolve_layout(800.0, 600.0);

    // The container establishes the IFC; the button is detached from Taffy
    // and positioned by Parley relative to the *content* box.
    assert!(
        doc.tree.get(container.0).unwrap().text_layout.is_some(),
        "container should be the IFC root"
    );
    let bl = doc.tree.get(button.0).unwrap().layout;
    let (abs_x, abs_y) = abs_origin(&doc, button.0);
    // Painted box origin = border-box origin + container content-box offset.
    let paint_x = abs_x + 20.0;
    let paint_y = abs_y + 20.0;

    // Center of the painted button hits the button (not a text sibling).
    assert_eq!(
        hit_test(
            &doc.tree,
            paint_x + bl.width / 2.0,
            paint_y + bl.height / 2.0
        ),
        Some(button.0),
        "painted center of the inline-block button must hit the button"
    );
    // The bottom edge — the part bug (B) shifted out of the hit rect — hits too.
    assert_eq!(
        hit_test(
            &doc.tree,
            paint_x + bl.width / 2.0,
            paint_y + bl.height - 1.0
        ),
        Some(button.0),
        "bottom edge of the painted button must still hit the button"
    );
    // A point on the leading text well left of the button must NOT hit it.
    assert_ne!(
        hit_test(&doc.tree, paint_x - bl.width, paint_y + bl.height / 2.0),
        Some(button.0),
        "a point on the leading text must not resolve to the button"
    );
}
