use super::*;

// ── CSS 2.1 Appendix E step 8: positioned descendants with `z-index: auto`
//
// These drive the real `hit_test`, which reads
// `rinch_dom::stacking::stacking_paint_order` backwards. The ordering itself
// is pinned in `rinch-dom/tests/stacking_tests.rs`, together with the pixels
// the same sequence produces read forwards; what is pinned here is that the
// taps land where those pixels are.

/// The bug: a floating action button over a scrolling list was untappable.
/// The FAB is `position: absolute` with no `z-index`, so hit testing walked
/// the z == 0 stacking contexts — which at the time included the scroller,
/// because Rinch made one for every `overflow` (#324 stage B stopped) —
/// before the plain children, and every tap fell through to the row
/// underneath.
#[test]
fn a_tap_on_a_positioned_z_auto_box_over_a_scroller_hits_the_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "overflow: auto; width: 200px; height: 200px",
    );
    doc.append_child(body, scroller);
    let row = child_of(&mut doc, scroller, "width: 200px; height: 400px");

    let fab = doc.create_element("div");
    doc.set_attribute(
        fab,
        "style",
        "position: absolute; left: 150px; top: 150px; width: 56px; height: 56px",
    );
    doc.append_child(body, fab);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 170.0, 170.0),
        Some(fab.0),
        "the FAB paints over the scroller there, so the tap is the FAB's"
    );
    assert_eq!(
        hit_test(&doc.tree, 40.0, 40.0),
        Some(row.0),
        "and clear of the FAB the list still takes its own taps"
    );
}

/// A box hoisted past a clipping ancestor is **not tappable outside it**.
///
/// Since #324 stage B an `overflow` box is not a stacking context, so the
/// walk goes straight through it and `check_children` — the gate that used to
/// contain this — never sees it. What contains it now is the entry's clip
/// chain, applied here. `rinch-dom`'s `clip_chain_tests` pin the same rule in
/// pixels; this pins the *real* walk, which lives in this crate and which
/// those tests can only model.
///
/// The container is **scrolled**, and the panel reaches well past it on both
/// axes: a probe inside the clip proves nothing on its own, and at scroll 0 a
/// clip taken at the wrong origin is the same rect.
#[test]
fn a_tap_outside_a_hoisted_boxs_clipping_ancestor_misses_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    // Not at the document origin: a chain rect that forgot the accumulated
    // offset would still be right against a container at y = 0.
    let spacer = child_of(&mut doc, body, "width: 300px; height: 60px");

    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "position: relative; overflow: hidden; width: 200px; height: 100px",
    );
    doc.append_child(body, scroller);

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: absolute; left: 0; top: 0; width: 400px; height: 400px; \
         z-index: 5",
    );
    doc.append_child(scroller, panel);

    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[scroller.0].scroll_offset = (0.0, 40.0);

    // The scroller's box is x in [0, 200), y in [60, 160). Scrolled 40, the
    // panel covers y in [20, 420) — so it reaches outside on three sides.
    assert_eq!(
        hit_test(&doc.tree, 100.0, 100.0),
        Some(panel.0),
        "inside the scroller the panel takes the tap"
    );
    assert_eq!(
        hit_test(&doc.tree, 100.0, 30.0),
        Some(spacer.0),
        "above the scroller's own top edge the tap reaches the spacer behind \
         it — the panel paints from y = 20 unclipped and is not there to be \
         tapped"
    );
    assert_eq!(
        hit_test(&doc.tree, 100.0, 200.0),
        Some(doc.tree.body_id),
        "…nor below its bottom edge"
    );
    assert_eq!(
        hit_test(&doc.tree, 300.0, 100.0),
        Some(doc.tree.body_id),
        "…nor past its right edge: the chain clips both axes"
    );
}

/// …and the chain stops where CSS says it does. An absolute is not clipped by
/// an `overflow` ancestor below its containing block, so it stays tappable
/// out there.
///
/// Three levels, because that is the shallowest arrangement in which the rule
/// has anything to say: `outer` clips and is above the containing block,
/// `mid` is the containing block, `inner` clips and is below it.
#[test]
fn a_tap_outside_a_clip_below_the_containing_block_still_hits_the_absolute() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let outer = doc.create_element("div");
    doc.set_attribute(
        outer,
        "style",
        "position: relative; overflow: hidden; width: 200px; height: 200px",
    );
    doc.append_child(body, outer);
    let mid = doc.create_element("div");
    doc.set_attribute(
        mid,
        "style",
        "position: relative; width: 100px; height: 100px",
    );
    doc.append_child(outer, mid);
    let inner = doc.create_element("div");
    doc.set_attribute(
        inner,
        "style",
        "overflow: hidden; width: 50px; height: 50px",
    );
    doc.append_child(mid, inner);
    let abs = doc.create_element("div");
    doc.set_attribute(
        abs,
        "style",
        "position: absolute; left: 0; top: 0; width: 300px; height: 300px; z-index: 5",
    );
    doc.append_child(inner, abs);

    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[outer.0].scroll_offset = (0.0, 30.0);

    assert_eq!(
        hit_test(&doc.tree, 25.0, 25.0),
        Some(abs.0),
        "inside every box"
    );
    assert_eq!(
        hit_test(&doc.tree, 150.0, 150.0),
        Some(abs.0),
        "outside `inner`'s 50x50 box: `inner` is below the containing block, \
         so it is not in the absolute's containing-block chain"
    );
    assert_eq!(
        hit_test(&doc.tree, 150.0, 180.0),
        Some(abs.0),
        "…and still inside `outer`, whose clip does not move with its own \
         30px scroll offset"
    );
    assert_eq!(
        hit_test(&doc.tree, 250.0, 100.0),
        Some(doc.tree.body_id),
        "but `outer` does clip it, so past its right edge the tap is the \
         body's"
    );
}

/// **Every** link of the chain is applied, not just the outermost.
///
/// Two nested clipping boxes, and the probe that matters is between them:
/// inside the outer, outside the inner. With one clip, or with only the first
/// link checked, that point answers the panel; it must not. Every other chain
/// fixture has a single effective link, which is the fixed point this leaves.
#[test]
fn every_link_of_a_two_clip_chain_is_applied_to_a_tap() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let outer = doc.create_element("div");
    doc.set_attribute(
        outer,
        "style",
        "overflow: hidden; width: 300px; height: 300px",
    );
    doc.append_child(body, outer);
    let inner = doc.create_element("div");
    doc.set_attribute(
        inner,
        "style",
        "overflow: hidden; width: 100px; height: 100px",
    );
    doc.append_child(outer, inner);

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: relative; z-index: 5; width: 400px; height: 400px",
    );
    doc.append_child(inner, panel);

    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[inner.0].scroll_offset = (0.0, 20.0);

    assert_eq!(
        hit_test(&doc.tree, 50.0, 50.0),
        Some(panel.0),
        "inside both clips"
    );
    assert_eq!(
        hit_test(&doc.tree, 50.0, 95.0),
        Some(panel.0),
        "right down to the inner box's own bottom edge — its 20px scroll \
         offset does not move it"
    );
    assert_eq!(
        hit_test(&doc.tree, 150.0, 150.0),
        Some(outer.0),
        "outside the inner clip and inside the outer one: the tap is the \
         outer box's. Check only the first link and this answers the panel"
    );
    assert_eq!(
        hit_test(&doc.tree, 350.0, 150.0),
        Some(doc.tree.body_id),
        "and outside both it is the body's"
    );
}

/// The same rule against in-flow content rather than a stacking context: a
/// positioned box is step 8, an in-flow block is step 4, so the box takes
/// the tap even where the block is written after it.
#[test]
fn a_tap_on_a_positioned_z_auto_box_over_later_in_flow_content_hits_the_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let fab = doc.create_element("div");
    doc.set_attribute(
        fab,
        "style",
        "position: absolute; left: 20px; top: 20px; width: 40px; height: 40px",
    );
    doc.append_child(body, fab);
    let block = child_of(&mut doc, body, "width: 200px; height: 100px");

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(hit_test(&doc.tree, 30.0, 30.0), Some(fab.0));
    assert_eq!(hit_test(&doc.tree, 120.0, 50.0), Some(block.0));
}

/// A stacking context nested under a positioned `z-index: auto` box is the
/// *ancestor's*, so its `z-index` still outranks a later sibling of the box
/// it is written in — and is tapped accordingly.
#[test]
fn a_z_indexed_box_under_a_positioned_z_auto_box_is_still_ordered_by_its_z() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: absolute; left: 0; top: 0; width: 100px; height: 100px",
    );
    doc.append_child(body, panel);
    let badge = child_of(
        &mut doc,
        panel,
        "position: absolute; z-index: 3; left: 0; top: 0; width: 40px; height: 40px",
    );

    let cover = doc.create_element("div");
    doc.set_attribute(
        cover,
        "style",
        "position: absolute; left: 0; top: 0; width: 100px; height: 100px",
    );
    doc.append_child(body, cover);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 20.0, 20.0),
        Some(badge.0),
        "z-index: 3 beats a z == 0 sibling written later, wherever it is nested"
    );
    assert_eq!(
        hit_test(&doc.tree, 80.0, 80.0),
        Some(cover.0),
        "outside the badge, the later z == 0 box wins on tree order"
    );
}

/// #204: an absolute box with no positioned ancestor resolves against the
/// initial containing block. Layout keeps `LayoutResult` parent-relative and
/// writes the delta, so hit testing needs no rule of its own — this pins
/// that it agrees with paint anyway, which is the whole point of the delta.
#[test]
fn icb_absolute_is_hit_over_the_whole_viewport() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    // Unpositioned, small, and pushed away from the origin: before the fix
    // the overlay's hit area was this box.
    let host = child_of(
        &mut doc,
        body,
        "width: 100px; height: 50px; margin-left: 200px; margin-top: 100px",
    );
    let overlay = child_of(&mut doc, host, "position: absolute; inset: 0");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        hit_test(&doc.tree, 10.0, 10.0),
        Some(overlay.0),
        "the overlay covers the viewport, so a click near the origin lands on it"
    );
    assert_eq!(
        hit_test(&doc.tree, 780.0, 580.0),
        Some(overlay.0),
        "and so does one near the far corner"
    );
}

/// A hoisted inline-block — `position: relative` in a padded IFC — is
/// tapped where its IFC **paints** it: the root's content origin plus the
/// parley position. Not one padding+border up-left at the plain border-box
/// chain position, where the pre-#365 ghost copy used to paint.
///
/// This is also the pin for `stacking.rs`'s entry-push rule: the hoisted
/// `PaintEntry`'s offset must stay border-box-relative, because this
/// file's `descend` adds `ifc_content_box_offset` itself when it enters
/// the entry node. Add the offset at the entry push in `collect_hoisted`
/// and this test fails by exactly one padding+border — the offset
/// double-added. Before this test existed, that mutation survived the
/// entire workspace.
#[test]
fn a_hoisted_inline_block_is_tapped_where_its_ifc_paints_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "width: 400px; padding: 40px; border: 2px solid rgb(0,0,255); font-size: 16px",
    );
    doc.append_child(body, container);
    let t = doc.create_text("Press ");
    doc.append_child(container, t);
    let button = doc.create_element("button");
    doc.set_attribute(
        button,
        "style",
        "position: relative; width: 60px; height: 24px",
    );
    doc.append_child(container, button);
    doc.resolve_layout(800.0, 600.0);

    // Where the IFC paints the button: body origin + container origin +
    // padding+border + the parley x/y the IFC wrote into the button's own
    // layout. Derived from the tree, not copied from a run.
    let b = doc.tree.get(button.0).unwrap();
    let c = doc.tree.get(container.0).unwrap();
    let cs = &c.computed_style;
    let pad_x = cs.padding_left.to_px() + cs.border_left_width.to_px();
    let pad_y = cs.padding_top.to_px() + cs.border_top_width.to_px();
    let body_l = doc.tree.get(doc.tree.body_id).unwrap().layout;
    let px = body_l.x + c.layout.x + pad_x + b.layout.x + b.layout.width / 2.0;
    let py = body_l.y + c.layout.y + pad_y + b.layout.y + b.layout.height / 2.0;
    assert_eq!(
        hit_test(&doc.tree, px, py),
        Some(button.0),
        "the button is tapped at the position its IFC paints it"
    );
    // One padding+border up-left — the ghost position — must NOT tap the
    // button: nothing paints it there.
    assert_ne!(
        hit_test(&doc.tree, px - pad_x, py - pad_y),
        Some(button.0),
        "the border-box chain position taps something else — nothing \
         paints the button there"
    );
}

/// A chained box under a **transform** is tapped where it paints.
///
/// The chain lives in the collecting root's own untransformed space, and so
/// does the `x`/`y` this walk holds — but `vx`/`vy`, the viewport point kept
/// beside them for `position: fixed` subtrees, does not. The two coincide
/// everywhere except under a transform, which is why nothing else in the
/// file tells them apart: swap the filter to `vx`/`vy` and every other test
/// still passes while a hoisted box becomes untappable *inside its own
/// clip*.
///
/// The arrangement is ordinary — a transformed panel (the
/// `translate(-50%,-50%)` centred-modal idiom two tests up) holding a
/// scroller with `position: relative` rows.
#[test]
fn a_tap_on_a_chained_box_under_a_transform_lands_where_it_paints() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let tx = doc.create_element("div");
    doc.set_attribute(
        tx,
        "style",
        "position: relative; z-index: 1; transform: translate(100px, 100px); \
         width: 400px; height: 400px",
    );
    doc.append_child(body, tx);

    let clipbox = doc.create_element("div");
    doc.set_attribute(
        clipbox,
        "style",
        "overflow: hidden; width: 100px; height: 100px",
    );
    doc.append_child(tx, clipbox);

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: relative; z-index: 5; width: 300px; height: 300px",
    );
    doc.append_child(clipbox, panel);

    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[clipbox.0].scroll_offset = (0.0, 20.0);

    assert_eq!(
        hit_test(&doc.tree, 150.0, 150.0),
        Some(panel.0),
        "inside the clip at its transformed position — the chain is tested \
         in the root's own space, so the probe point must be too"
    );
    assert_eq!(
        hit_test(&doc.tree, 250.0, 250.0),
        Some(tx.0),
        "outside the clip and inside the transformed root: the panel reaches \
         here unclipped and must not take the tap"
    );
    assert_eq!(
        hit_test(&doc.tree, 50.0, 50.0),
        Some(doc.tree.body_id),
        "and above the transform entirely"
    );
}

/// A chain link is inclusive on both edges, like the `check_children` gate
/// it stands in for — so a tap on the last pixel of a clipping ancestor
/// still reaches the box hoisted past it.
///
/// `ClipRect::contains` says so in a doc comment and nothing measured it:
/// every other chain probe sits comfortably inside or outside, which is the
/// fixed point where `<` and `<=` agree.
#[test]
fn a_chain_link_is_inclusive_on_its_own_edge() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "position: relative");

    let clipbox = doc.create_element("div");
    doc.set_attribute(
        clipbox,
        "style",
        "overflow: hidden; width: 100px; height: 100px",
    );
    doc.append_child(body, clipbox);

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: relative; z-index: 5; width: 400px; height: 400px",
    );
    doc.append_child(clipbox, panel);

    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[clipbox.0].scroll_offset = (0.0, 20.0);

    assert_eq!(
        hit_test(&doc.tree, 100.0, 50.0),
        Some(panel.0),
        "x = 100 is the clip's own right edge, and the gate this stands in \
         for is inclusive there"
    );
    assert_eq!(
        hit_test(&doc.tree, 50.0, 100.0),
        Some(panel.0),
        "…and so is its bottom edge, which its scroll offset does not move"
    );
    assert_eq!(
        hit_test(&doc.tree, 101.0, 50.0),
        Some(doc.tree.body_id),
        "one pixel past it is out, so the pair above is an edge and not just \
         a generous rect"
    );
}
