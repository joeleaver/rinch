use super::*;

// ── CSS transforms (#199) ────────────────────────────────────────────
//
// The correctness criterion for every test below: a point hits a node iff
// that point lands inside where the node is *painted*. Paint draws each box
// under `compose_node_transform` (rinch-dom/src/paint/mod.rs), so the
// expected rects here are hand-computed from the CSS transform, not from
// any hit-test helper.

/// Build a document with a `400x300` relative container as `body`'s only
/// child, returning `(doc, container)`. The container sits at the viewport
/// origin so absolute child offsets *are* viewport coordinates.
fn container_doc() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "position: relative; width: 400px; height: 300px",
    );
    doc.append_child(body, container);
    (doc, container)
}

/// The motivating bug: the `left:50%; top:50%; translate(-50%,-50%)` modal
/// idiom rendered centered but hit-tested at its *un*translated layout box,
/// so every control inside it was unclickable.
#[test]
fn translate_centered_modal_is_hit_at_its_visual_position() {
    let (mut doc, container) = container_doc();
    // Layout box (200,150)-(300,210); painted box shifted by (-50,-30) to
    // (150,120)-(250,180), i.e. centered on the container.
    let modal = child_of(
        &mut doc,
        container,
        "position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); \
         width: 100px; height: 60px",
    );
    // In-flow child at the modal's origin: layout (200,150)-(240,170),
    // painted (150,120)-(190,140).
    let button = child_of(&mut doc, modal, "width: 40px; height: 20px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 200.0, 150.0),
        Some(modal.0),
        "the modal's visual centre (= the container's centre) must hit the modal"
    );
    assert_eq!(
        hit_test(&doc.tree, 170.0, 130.0),
        Some(button.0),
        "the button's painted centre must hit the button"
    );
    assert_eq!(
        hit_test(&doc.tree, 275.0, 195.0),
        Some(container.0),
        "a point in the modal's untransformed layout box must miss the modal"
    );
    assert_eq!(
        hit_test(&doc.tree, 220.0, 160.0),
        Some(modal.0),
        "the button's untransformed layout centre must miss the button"
    );
}

/// A percentage translate that is *not* the first function in the list
/// composes in the frame the functions before it establish (#212) — and
/// paint and hit testing have to say so together.
///
/// `scale(2) translateX(50%)` on a 100px-wide box moves it 100px, not 50:
/// the offset is scaled by the `scale` that precedes it, exactly as a
/// `translateX(50px)` in the same position would be. The whole point of
/// #203 is that the box a click reports is the box paint drew, so this
/// asserts both ends against one hand-computed rect.
#[test]
fn a_scaled_percentage_translate_paints_and_hits_in_the_same_place() {
    let (mut doc, container) = container_doc();
    // Layout box (100,60)-(200,100), origin at its centre (150,80).
    // m = [2,0,0,2, 100, 0]  (the 50% of 100px doubled by the scale), so
    // the composed map is p -> (2x - 50, 2y - 80) and the painted box is
    // (150,40)-(350,120). Before the fix m[4] was 50, putting it at
    // (100,40)-(300,120).
    let boxed = child_of(
        &mut doc,
        container,
        "position: absolute; left: 100px; top: 60px; width: 100px; height: 40px; \
         transform: scale(2) translateX(50%)",
    );
    doc.resolve_layout(800.0, 600.0);

    let (px, py, pw, ph) = super::painted_element_box(&doc.tree, boxed.0);
    for (got, want, what) in [
        (px, 150.0, "x"),
        (py, 40.0, "y"),
        (pw, 200.0, "width"),
        (ph, 80.0, "height"),
    ] {
        assert!(
            (got - want).abs() < 0.01,
            "painted {what}: got {got}, expected {want}"
        );
    }

    assert_eq!(
        hit_test(&doc.tree, 340.0, 80.0),
        Some(boxed.0),
        "a point inside the painted box's right end must hit it"
    );
    assert_eq!(
        hit_test(&doc.tree, 110.0, 80.0),
        Some(container.0),
        "a point where the un-scaled offset used to put the box must miss it"
    );
}

/// The same reconvergence with a rotation, where the offset changes
/// direction rather than magnitude.
///
/// `rotate(45deg) translateX(50%)` sends the box's centre 50px along the
/// rotated x-axis, i.e. to `(150, 80) + (35.36, 35.36)`. Before the fix the
/// offset was applied in the outer frame, landing the centre at `(200, 80)`
/// — far enough away that each position misses the other's quad.
#[test]
fn a_rotated_percentage_translate_paints_and_hits_in_the_same_place() {
    let (mut doc, container) = container_doc();
    let boxed = child_of(
        &mut doc,
        container,
        "position: absolute; left: 100px; top: 60px; width: 100px; height: 40px; \
         transform: rotate(45deg) translateX(50%)",
    );
    doc.resolve_layout(800.0, 600.0);

    // The bounding box of an affine image of a rectangle is centred on the
    // image of the rectangle's centre, so this reads paint's answer for
    // where the box went without hand-computing the rotated corners.
    let (px, py, pw, ph) = super::painted_element_box(&doc.tree, boxed.0);
    let (cx, cy) = (px + pw / 2.0, py + ph / 2.0);
    let offset = 50.0 * std::f32::consts::FRAC_1_SQRT_2;
    assert!(
        (cx - (150.0 + offset)).abs() < 0.01 && (cy - (80.0 + offset)).abs() < 0.01,
        "painted centre: got ({cx}, {cy}), expected ({}, {})",
        150.0 + offset,
        80.0 + offset
    );

    assert_eq!(
        hit_test(&doc.tree, cx, cy),
        Some(boxed.0),
        "the painted centre must hit the box"
    );
    assert_eq!(
        hit_test(&doc.tree, 200.0, 80.0),
        Some(container.0),
        "where the un-rotated offset used to put the box must now miss it"
    );
}

/// Nested transforms compose: the child is hit where the parent's translate
/// and its own scale put it, not at either one alone.
#[test]
fn nested_transforms_compose() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(
        outer,
        "style",
        "position: relative; width: 400px; height: 300px; transform: translate(100px, 50px)",
    );
    doc.append_child(body, outer);
    // Layout (0,0)-(100,50) inside `outer`; scale(2) about its centre
    // (50,25) → (-50,-25)-(150,75); + outer's translate → (50,25)-(250,125).
    let inner = child_of(
        &mut doc,
        outer,
        "position: absolute; left: 0; top: 0; width: 100px; height: 50px; transform: scale(2)",
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 60.0, 30.0),
        Some(inner.0),
        "top-left of the composed quad must hit the inner box"
    );
    assert_eq!(
        hit_test(&doc.tree, 240.0, 120.0),
        Some(inner.0),
        "bottom-right of the composed quad must hit the inner box"
    );
    assert_eq!(
        hit_test(&doc.tree, 260.0, 130.0),
        Some(outer.0),
        "just outside the composed quad falls through to the translated parent"
    );
    assert_eq!(
        hit_test(&doc.tree, 20.0, 20.0),
        Some(body.0),
        "the parent's own layout box (before its translate) must not be hit"
    );
}

/// A scaled box is hit over its scaled bounds, about its transform-origin.
#[test]
fn scaled_node_is_hit_over_its_scaled_bounds() {
    let (mut doc, container) = container_doc();
    // Default origin 50% 50%: (100,100)-(150,150) scaled 2x about (125,125)
    // → (75,75)-(175,175).
    let centred = child_of(
        &mut doc,
        container,
        "position: absolute; left: 100px; top: 100px; width: 50px; height: 50px; \
         transform: scale(2)",
    );
    // origin top left: (300,100)-(350,150) → (300,100)-(400,200).
    let corner = child_of(
        &mut doc,
        container,
        "position: absolute; left: 300px; top: 100px; width: 50px; height: 50px; \
         transform: scale(2); transform-origin: top left",
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 80.0, 80.0),
        Some(centred.0),
        "inside the scaled bounds but outside the layout box must hit"
    );
    assert_eq!(hit_test(&doc.tree, 125.0, 125.0), Some(centred.0));
    assert_eq!(
        hit_test(&doc.tree, 180.0, 180.0),
        Some(container.0),
        "outside the scaled bounds must miss"
    );
    assert_eq!(
        hit_test(&doc.tree, 390.0, 190.0),
        Some(corner.0),
        "a top-left-origin scale grows down/right only"
    );
    assert_eq!(
        hit_test(&doc.tree, 285.0, 85.0),
        Some(container.0),
        "a top-left-origin scale must not grow up/left (that would be a 50% origin)"
    );
}

/// Rotation is supported by the paint transform (`GenericTransformOperation::Rotate`),
/// so hit testing must follow the rotated quad.
#[test]
fn rotated_node_is_hit_inside_its_rotated_quad() {
    let (mut doc, container) = container_doc();
    // (100,140)-(300,160) rotated 90° about its centre (200,150)
    // → a 20x200 quad, (190,50)-(210,250).
    let bar = child_of(
        &mut doc,
        container,
        "position: absolute; left: 100px; top: 140px; width: 200px; height: 20px; \
         transform: rotate(90deg)",
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 200.0, 150.0),
        Some(bar.0),
        "the centre is fixed by the rotation"
    );
    assert_eq!(
        hit_test(&doc.tree, 200.0, 60.0),
        Some(bar.0),
        "the top of the rotated quad must hit"
    );
    assert_eq!(
        hit_test(&doc.tree, 200.0, 240.0),
        Some(bar.0),
        "the bottom of the rotated quad must hit"
    );
    assert_eq!(
        hit_test(&doc.tree, 110.0, 150.0),
        Some(container.0),
        "the unrotated box's left end is outside the rotated quad"
    );
}

/// Stacking-context order is unchanged by the transform mapping: a
/// transformed SC is tested at its visual position but in its z-index slot,
/// and an untransformed higher-z sibling still wins.
#[test]
fn transformed_sc_keeps_z_index_order() {
    let (mut doc, container) = container_doc();
    let back = child_of(
        &mut doc,
        container,
        "position: absolute; left: 100px; top: 0; width: 200px; height: 200px; z-index: 1",
    );
    // Layout (300,50)-(400,150); painted (200,50)-(300,150).
    let mid = child_of(
        &mut doc,
        container,
        "position: absolute; left: 300px; top: 50px; width: 100px; height: 100px; \
         z-index: 2; transform: translate(-100px, 0)",
    );
    let front = child_of(
        &mut doc,
        container,
        "position: absolute; left: 250px; top: 100px; width: 100px; height: 100px; z-index: 3",
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 210.0, 60.0),
        Some(mid.0),
        "the transformed SC beats the lower-z sibling it visually covers"
    );
    assert_eq!(
        hit_test(&doc.tree, 260.0, 110.0),
        Some(front.0),
        "an untransformed higher-z sibling still wins over the transformed SC"
    );
    assert_eq!(
        hit_test(&doc.tree, 390.0, 60.0),
        Some(container.0),
        "the transformed SC is not hit at its untransformed layout box"
    );
    assert_eq!(hit_test(&doc.tree, 110.0, 190.0), Some(back.0));
}

/// A non-invertible transform (`scale(0)`) paints nothing, so it and its
/// subtree are unhittable — and the traversal must survive it (no panic, no
/// NaN leaking into the sibling that is tested next).
#[test]
fn scale_zero_subtree_is_unhittable() {
    let (mut doc, container) = container_doc();
    let collapsed = child_of(
        &mut doc,
        container,
        "position: absolute; left: 100px; top: 100px; width: 100px; height: 100px; \
         transform: scale(0)",
    );
    let inside = child_of(&mut doc, collapsed, "width: 50px; height: 50px");
    let sibling = child_of(
        &mut doc,
        container,
        "position: absolute; left: 250px; top: 100px; width: 50px; height: 50px",
    );
    doc.resolve_layout(800.0, 600.0);

    for (x, y) in [(150.0, 150.0), (100.0, 100.0), (105.0, 105.0)] {
        let hit = hit_test(&doc.tree, x, y);
        assert_eq!(
            hit,
            Some(container.0),
            "({x}, {y}) must fall through the scale(0) subtree to the container"
        );
        assert_ne!(hit, Some(collapsed.0));
        assert_ne!(hit, Some(inside.0));
    }
    assert_eq!(
        hit_test(&doc.tree, 275.0, 125.0),
        Some(sibling.0),
        "the sibling tested after the degenerate subtree still hits"
    );
}

/// `backface-visibility: hidden` on a box whose own transform turns its
/// back to the viewer (#997): Chrome 153's `elementFromPoint` finds neither
/// it nor anything in it. The same box turned only 80° is hit.
#[test]
fn a_hidden_backface_subtree_is_unhittable() {
    let (mut doc, container) = container_doc();
    let turned = child_of(
        &mut doc,
        container,
        "position: absolute; left: 100px; top: 100px; width: 100px; height: 40px; \
         transform: rotateY(180deg); backface-visibility: hidden",
    );
    let inside = child_of(
        &mut doc,
        turned,
        "width: 100px; height: 40px; transform: rotateY(180deg)",
    );
    let edge_on = child_of(
        &mut doc,
        container,
        "position: absolute; left: 300px; top: 100px; width: 100px; height: 40px; \
         transform: rotateY(80deg); backface-visibility: hidden",
    );
    doc.resolve_layout(800.0, 600.0);

    for (x, y) in [(150.0, 120.0), (110.0, 105.0), (190.0, 135.0)] {
        let hit = hit_test(&doc.tree, x, y);
        assert_eq!(hit, Some(container.0), "({x}, {y}) falls through");
        assert_ne!(hit, Some(inside.0));
    }
    assert_eq!(hit_test(&doc.tree, 350.0, 120.0), Some(edge_on.0));
}

/// `position: fixed` inside a transformed ancestor: the transform makes
/// the ancestor its containing block (#1372), so it is placed against the
/// ancestor and painted under its transform — and hit there, where paint
/// puts it. Chrome 151 (`elementFromPoint`): `(60, 70)` is the body,
/// `(260, 170)` the fixed box, whose box is `250,160,100,40`. (Before
/// #1372 rinch placed it at its viewport box, untransformed, in paint and
/// hit testing alike.)
#[test]
fn fixed_in_transformed_ancestor_is_hit_where_paint_puts_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(
        outer,
        "style",
        "position: relative; width: 400px; height: 300px; transform: translate(200px, 100px)",
    );
    doc.append_child(body, outer);
    let fixed = child_of(
        &mut doc,
        outer,
        "position: fixed; left: 50px; top: 60px; width: 100px; height: 40px; z-index: 5",
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 260.0, 170.0),
        Some(fixed.0),
        "the fixed box is hit where the ancestor's transform carries it"
    );
    assert_ne!(
        hit_test(&doc.tree, 60.0, 70.0),
        Some(fixed.0),
        "and not at its insets in the viewport"
    );
    let r = rinch_dom::paint::painted_border_box(&doc.tree, fixed.0, 1.0);
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (250.0, 160.0, 350.0, 200.0));
}

/// A transform on `body` and on an ancestor between: the ancestor contains
/// the fixed box (#1372), and both transforms carry it. Chrome 151, with
/// `body { margin: 0; transform: translate(40px, 30px) }`: the box is
/// `290,190,100,40`; `(140, 110)` — its inset box under the body's
/// translate alone, where rinch drew it before #1372 — and `(100, 80)` are
/// the body.
#[test]
fn body_transform_applies_to_a_hoisted_fixed_descendant() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "transform: translate(40px, 30px)");
    let outer = child_of(
        &mut doc,
        body,
        "position: relative; width: 400px; height: 300px; transform: translate(200px, 100px)",
    );
    let fixed = child_of(
        &mut doc,
        outer,
        "position: fixed; left: 50px; top: 60px; width: 100px; height: 40px; z-index: 5",
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 300.0, 200.0),
        Some(fixed.0),
        "the fixed box is hit under both transforms"
    );
    for (x, y) in [(140.0, 110.0), (100.0, 80.0)] {
        assert_ne!(hit_test(&doc.tree, x, y), Some(fixed.0), "({x}, {y})");
    }
}

/// Pins the `display: contents` half of the zero-size guard in `local_point`.
/// A box collapsed on one axis keeps its transform box — `paint_node`'s
/// `(w == 0) != (h == 0)` branch composes the transform and paints the
/// overflowing children under it — so the guard must *not* fire here.
#[test]
fn one_axis_zero_size_node_still_transforms_its_children() {
    let (mut doc, container) = container_doc();
    let zerobox = child_of(
        &mut doc,
        container,
        "position: absolute; left: 100px; top: 100px; width: 200px; height: 0; \
         transform: translate(50px, 20px)",
    );
    // Overflows the zero-height parent: layout (100,100)-(140,130),
    // painted (150,120)-(190,150).
    let child = child_of(
        &mut doc,
        zerobox,
        "position: absolute; left: 0; top: 0; width: 40px; height: 30px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(zerobox.0).unwrap().layout.height,
        0.0,
        "the test needs the one-axis-collapsed paint branch"
    );

    assert_eq!(
        hit_test(&doc.tree, 170.0, 135.0),
        Some(child.0),
        "the child is hit under the collapsed parent's transform"
    );
    assert_eq!(
        hit_test(&doc.tree, 120.0, 115.0),
        Some(container.0),
        "the child's untransformed layout box must miss"
    );
}

/// Pins the zero-size half of the same guard. A `display: contents` node has
/// no box, so `paint_node` passes the parent transform straight through to
/// its children and the node's own `transform` never renders — hit testing
/// must ignore it too.
#[test]
fn display_contents_node_ignores_its_own_transform() {
    let (mut doc, container) = container_doc();
    let contents = child_of(
        &mut doc,
        container,
        "display: contents; transform: translate(60px, 40px)",
    );
    let child = child_of(&mut doc, contents, "width: 40px; height: 30px");
    doc.resolve_layout(800.0, 600.0);
    let cn = doc.tree.get(contents.0).unwrap();
    assert_eq!(
        (cn.layout.width, cn.layout.height),
        (0.0, 0.0),
        "the test needs display:contents to have no box"
    );
    assert!(
        !cn.computed_style.transform.is_identity,
        "the test needs a real transform declared on the display:contents node"
    );

    assert_eq!(
        hit_test(&doc.tree, 20.0, 15.0),
        Some(child.0),
        "the child is hit at its laid-out box, untouched by the contents node's transform"
    );
    assert_eq!(
        hit_test(&doc.tree, 80.0, 55.0),
        Some(container.0),
        "where that transform would have put the child must miss"
    );
}

/// A `transform` on a plain inline span does not apply (#1080, CSS
/// Transforms 1 §1): Chrome 153 puts an `inline-block` inside
/// `span { transform: translateX(30px) }` exactly where it is with no
/// transform, so paint draws it there and hit testing must find it there.
/// Fails before #1080: the probe at the untranslated box missed and the
/// one 30px right of it hit.
#[test]
fn an_inline_spans_transform_does_not_move_its_inline_block_child() {
    let (mut doc, container) = container_doc();
    doc.set_attribute(
        container,
        "style",
        "position: relative; width: 400px; height: 300px; font-size: 16px; line-height: 20px",
    );
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "transform: translateX(30px)");
    doc.append_child(container, span);
    let ib = doc.create_element("span");
    doc.set_attribute(
        ib,
        "style",
        "display: inline-block; width: 20px; height: 10px",
    );
    doc.append_child(span, ib);
    doc.resolve_layout(800.0, 600.0);
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, ib.0, 1.0);
    let (x, y) = (x as f32, y as f32);
    assert!(
        !doc.tree
            .get(span.0)
            .unwrap()
            .computed_style
            .transform
            .is_identity,
        "the test needs a real transform declared on the span"
    );
    assert_eq!(
        hit_test(&doc.tree, x + 10.0, y + 5.0),
        Some(ib.0),
        "the inline-block is hit at its laid-out box"
    );
    assert_ne!(
        hit_test(&doc.tree, x + 40.0, y + 5.0),
        Some(ib.0),
        "where the span's transform would have put it must miss"
    );
}

/// The split-inline twin (#513: a block inside the transformed span, which
/// keeps a box of its own). Chrome 153: `elementFromPoint(10, 25)` is the
/// block and `(40, 25)` is the span — the transform moves nothing.
#[test]
fn a_split_inlines_transform_does_not_move_its_block_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let d = child_of(
        &mut doc,
        body,
        "height: 80px; font-size: 16px; line-height: 20px",
    );
    let ab = doc.create_text("ab");
    doc.append_child(d, ab);
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "transform: translateX(30px)");
    doc.append_child(d, span);
    let xy = doc.create_text("xy");
    doc.append_child(span, xy);
    let b = child_of(&mut doc, span, "width: 20px; height: 10px");
    let zz = doc.create_text("zz");
    doc.append_child(span, zz);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        hit_test(&doc.tree, 10.0, 25.0),
        Some(b.0),
        "the block, in place"
    );
    assert_ne!(hit_test(&doc.tree, 40.0, 25.0), Some(b.0), "not 30px right");
}
