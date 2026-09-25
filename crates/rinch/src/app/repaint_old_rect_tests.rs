//! A box that moves between two paints is cleared where it was **painted**,
//! however many layout passes ran in between.
//!
//! `prev_layout` used to be overwritten by every `read_layout_results`, so a
//! second resolve before the paint — the editor's caret pass runs one per
//! frame, and two pointer moves between paints do it too — lost the rect the
//! box's old pixels were in. The caret, a selection rect or a dragged panel
//! then stayed painted where it had been, which the editor overlay and the
//! inset fast path (#280) papered over with a full repaint per keystroke and
//! per drag move. Now only the paint that consumes the region advances it
//! (`NodeTree::consume_paint_dirty`), and both full repaints are gone.
//!
//! Every fixture here is a **local** pixel oracle rather than a whole-screen
//! comparison alone: it names the rect the old pixels were in and asserts that
//! the incremental frame holds exactly what a from-scratch frame holds there,
//! with a positive control that the old frame did have ink in it. A
//! whole-frame SSIM cannot see a caret-sized ghost. Each also asserts through
//! the perf counters that the frame really was incremental — an accidental
//! full repaint would make the pixel half pass vacuously.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};

const SIZE: (u32, u32) = (600, 400);

/// A from-scratch frame of the current state.
fn full_frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

/// The frame a running app paints next: the dirty region only. Returns the
/// pixels and that frame's counters.
fn incremental_frame(app: &mut RinchApp) -> (Vec<u8>, FrameStats) {
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let stats = app.end_perf_frame().expect("mounted");
    (px, stats)
}

/// Pixels of `rect` (x0, y0, x1, y1 in physical px, clamped) that differ
/// between `a` and `b` by more than a rounding step.
fn diff_in(a: &[u8], b: &[u8], rect: (i32, i32, i32, i32)) -> usize {
    let (w, h) = (SIZE.0 as i32, SIZE.1 as i32);
    let mut n = 0;
    for y in rect.1.max(0)..rect.3.min(h) {
        for x in rect.0.max(0)..rect.2.min(w) {
            let i = ((y * w + x) * 4) as usize;
            if (0..4).any(|k| (a[i + k] as i32 - b[i + k] as i32).abs() > 2) {
                n += 1;
            }
        }
    }
    n
}

/// Pixels of `rect` that are not white.
fn ink_in(px: &[u8], rect: (i32, i32, i32, i32)) -> usize {
    let (w, h) = (SIZE.0 as i32, SIZE.1 as i32);
    let mut n = 0;
    for y in rect.1.max(0)..rect.3.min(h) {
        for x in rect.0.max(0)..rect.2.min(w) {
            let i = ((y * w + x) * 4) as usize;
            if px[i] < 250 || px[i + 1] < 250 || px[i + 2] < 250 {
                n += 1;
            }
        }
    }
    n
}

fn assert_incremental(stats: &FrameStats) {
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
}

// ── Absolute boxes moved by their insets (the #280 fast path) ─────────────

/// An absolute box, with `extra` appended to its style, in a small positioned
/// holder it overflows — so the holder's own box, which a removal dirties,
/// does not reach the rects under test and cannot cover a missing one.
/// Returns the app and the box.
fn panel(extra: &'static str) -> (RinchApp, NodeHandle) {
    let (app, b, _) = panel_with(extra, None);
    (app, b)
}

/// [`panel`], with an optional child of the box styled `child`.
fn panel_with(
    extra: &'static str,
    child: Option<&'static str>,
) -> (RinchApp, NodeHandle, Option<NodeHandle>) {
    type Slot = Option<(NodeHandle, Option<NodeHandle>)>;
    let slot: Rc<RefCell<Slot>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 10px; height: 10px");
        outer.append_child(&root);
        let b = scope.create_element("div");
        b.set_attribute(
            "style",
            &format!(
                "position: absolute; left: 20px; top: 150px; width: 40px; height: 40px; \
                 background: rgb(0, 0, 200); {extra}"
            ),
        );
        root.append_child(&b);
        let c = child.map(|style| {
            let c = scope.create_element("div");
            c.set_attribute("style", style);
            b.append_child(&c);
            c
        });
        *slot_in.borrow_mut() = Some((b, c));
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let (b, c) = slot.borrow().clone().unwrap();
    (app, b, c)
}

fn resolve(app: &mut RinchApp) {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
}

/// Two moves, each resolved, before one paint — what two pointer moves of a
/// `FloatingPanel` drag between redraws do. The box's first position (the one
/// on screen) must be cleared although the second resolve saw it at the
/// intermediate one.
#[test]
fn two_resolves_between_paints_still_clear_the_painted_rect() {
    let (mut app, b) = panel("");
    let before = full_frame(&mut app);
    let old = (20, 150, 60, 190);
    assert!(
        ink_in(&before, old) > 1000,
        "positive control: the box is painted at its old rect"
    );

    b.set_style("left", "200px");
    resolve(&mut app);
    b.set_style("left", "400px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);

    assert_eq!(ink_in(&inc, old), 0, "the box ghosts where it was painted");
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
    assert!(
        ink_in(&inc, (400, 150, 440, 190)) > 1000,
        "and it is drawn where it went"
    );
}

/// A dragged box carries its shadow. The shadow reaches far past the 4px the
/// dirty margin pads a box by, so the region has to be grown by what the
/// moved subtree paints, at the old rect and the new one alike.
#[test]
fn a_moved_box_clears_its_old_shadow_and_draws_its_new_one() {
    let (mut app, b) = panel("box-shadow: 0 0 24px 8px rgb(0, 0, 0)");
    let before = full_frame(&mut app);
    // A band of shadow left of the box, 8..16px outside it: past the 4px
    // margin and inside `spread + blur / 2`.
    let old_shadow = (4, 150, 12, 190);
    assert!(
        ink_in(&before, old_shadow) > 100,
        "positive control: the shadow is painted there"
    );

    b.set_style("left", "300px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);

    assert_eq!(diff_in(&inc, &full, old_shadow), 0, "the old shadow ghosts");
    let new_shadow = (284, 150, 292, 190);
    assert!(
        ink_in(&full, new_shadow) > 100,
        "positive control: the new shadow is there"
    );
    assert_eq!(
        diff_in(&inc, &full, new_shadow),
        0,
        "the new shadow is clipped by the region"
    );
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

/// A box moved and then removed before the paint: its pixels are at the rect
/// it was painted in, not the one it was removed from, and nothing but the
/// removal records either.
#[test]
fn a_box_moved_then_removed_before_the_paint_is_cleared_where_it_was_painted() {
    let (mut app, b) = panel("");
    let before = full_frame(&mut app);
    let old = (20, 150, 60, 190);
    assert!(ink_in(&before, old) > 1000, "positive control");

    // Up and to the right: the region is one bounding rect, and the holder
    // the removal dirties sits at the origin, so a destination below-right
    // of the old rect would put the old rect inside that union by accident
    // and pass with the old rect never recorded.
    b.set_style("left", "300px");
    b.set_style("top", "20px");
    resolve(&mut app);
    b.remove();
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    assert_eq!(
        ink_in(&inc, old),
        0,
        "the removed box ghosts where it was painted"
    );
    let full = full_frame(&mut app);
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

/// A `transform` change moves where a box is painted without changing its
/// layout, so `prev_layout` says nothing about it: the old painted rect comes
/// from the transform it was painted with (`PaintedState::transform`).
///
/// Older than this file (paint audit F9): the region used to take only the
/// node's *current* transformed rect, so a transform that moved a box off its
/// old pixels left them painted — masked in practice because the big sliding
/// panels that animate `transform` (a `Drawer`) cover half the window and
/// repaint in full.
#[test]
fn a_transform_change_clears_the_old_painted_rect() {
    // Two steps, one per direction: a transformed box whose transform goes
    // away (painted transformed, now the identity), then an untransformed one
    // that gains one. Neither starts from the identity it ends at, which would
    // pass with the painted transform ignored.
    let (mut app, b) = panel("transform: translate(40px, 0px)");
    for (transform, old) in [
        ("none", (60, 150, 100, 190)),
        ("translate(280px, -130px)", (20, 150, 60, 190)),
    ] {
        let before = full_frame(&mut app);
        assert!(ink_in(&before, old) > 1000, "positive control: {old:?}");
        b.set_style("transform", transform);
        resolve(&mut app);
        let (inc, stats) = incremental_frame(&mut app);
        assert_incremental(&stats);
        assert_eq!(
            ink_in(&inc, old),
            0,
            "the box ghosts where it was painted before `transform: {transform}`"
        );
        let full = full_frame(&mut app);
        assert_eq!(
            diff_in(&inc, &full, (0, 0, 600, 400)),
            0,
            "incremental frame != full frame"
        );
    }
}

// ── A move and an ink change in one frame (review of #880) ───────────────
//
// The forced full repaint on the inset path also hid these: a box moves and,
// before the next paint, what it paints changes too. The old rect has to be
// the one it was painted with — its own ink, and its children where *they*
// were drawn — not today's ink moved back by the box's delta.

/// Outside the 4px margin, inside `spread + blur / 2` of the box at (20, 150).
const SHADOW_BAND: (i32, i32, i32, i32) = (4, 150, 12, 190);
const SHADOW: &str = "box-shadow: 0 0 24px 8px rgb(0, 0, 0)";
/// An absolute child overflowing the box to its right.
const CHILD: &str = "position: absolute; left: 60px; top: 0px; width: 40px; height: 40px; \
                     background: rgb(200, 0, 0)";
/// Where [`CHILD`] is painted while the box is at `left: 200px`.
const CHILD_OLD: (i32, i32, i32, i32) = (260, 150, 300, 190);

/// An incremental frame after `step`, checked against a from-scratch frame
/// over `rect` (which the positive control says held ink before) and over the
/// whole surface. Every move here is diagonal, away from `rect`: the region
/// is one bounding rect, and a move that keeps `rect` inside the union of the
/// new rects covers a missing old rect by accident.
fn assert_clean_after(
    app: &mut RinchApp,
    rect: (i32, i32, i32, i32),
    step: impl FnOnce(&mut RinchApp),
) {
    let before = full_frame(app);
    assert!(
        ink_in(&before, rect) > 100,
        "positive control: {rect:?} holds ink"
    );
    step(app);
    let (inc, stats) = incremental_frame(app);
    assert_incremental(&stats);
    let full = full_frame(app);
    assert_eq!(diff_in(&inc, &full, rect), 0, "ghost in {rect:?}");
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

/// A drag ends: the box moves one last time and drops its drag shadow, in
/// one resolve. The shadow it was painted with is gone from the tree.
#[test]
fn a_box_that_moves_and_drops_its_shadow_clears_the_old_shadow() {
    let (mut app, b) = panel(SHADOW);
    assert_clean_after(&mut app, SHADOW_BAND, |app| {
        b.set_style("left", "300px");
        b.set_style("box-shadow", "none");
        resolve(app);
    });
}

/// The same, with the move resolved before the shadow goes.
#[test]
fn a_box_that_moves_then_drops_its_shadow_clears_the_old_shadow() {
    let (mut app, b) = panel(SHADOW);
    assert_clean_after(&mut app, SHADOW_BAND, |app| {
        b.set_style("left", "300px");
        resolve(app);
        b.set_style("box-shadow", "none");
        resolve(app);
    });
}

/// The box moves diagonally away, then its overflowing child is removed: the
/// child was painted at the box's *old* position plus its offset.
#[test]
fn a_child_removed_after_its_parent_moved_is_cleared_where_it_was_painted() {
    let (mut app, b, c) = panel_with("left: 200px", Some(CHILD));
    assert_clean_after(&mut app, CHILD_OLD, |app| {
        b.set_style("top", "300px");
        b.set_style("left", "20px");
        resolve(app);
        c.unwrap().remove();
        resolve(app);
    });
}

/// The box moves (resolved), then is removed with its overflowing child
/// still inside. The child was painted at the box's *old* position: its
/// painted rect is summed through the box's painted state, so the removal has
/// to record the child before it forgets the box's.
#[test]
fn a_moved_box_removed_with_its_child_clears_the_child_where_it_was_painted() {
    let (mut app, b, _) = panel_with("left: 200px", Some(CHILD));
    assert_clean_after(&mut app, CHILD_OLD, |app| {
        b.set_style("top", "300px");
        b.set_style("left", "20px");
        resolve(app);
        b.remove();
        resolve(app);
    });
}

/// The box moves and takes its overflowing child with it, untouched. The
/// child is not paint-dirty at all: only the moved box's subtree reach
/// (`opacity_layer_bounds`) says it was painted outside the box.
#[test]
fn an_overflowing_child_moves_with_its_parent_and_leaves_nothing_behind() {
    let (mut app, b, _) = panel_with("left: 200px", Some(CHILD));
    assert_clean_after(&mut app, CHILD_OLD, |app| {
        b.set_style("top", "300px");
        b.set_style("left", "20px");
        resolve(app);
    });
}

/// The box moves and its overflowing child flips to the other side, in one
/// resolve — a popover re-anchored while its arrow flips.
#[test]
fn a_child_that_flips_side_while_its_parent_moves_is_cleared() {
    let (mut app, b, c) = panel_with("left: 200px", Some(CHILD));
    assert_clean_after(&mut app, CHILD_OLD, |app| {
        b.set_style("top", "300px");
        b.set_style("left", "20px");
        c.unwrap().set_style("left", "-100px");
        resolve(app);
    });
}

/// The box moves and its overflowing child goes `display: none`.
#[test]
fn a_child_hidden_while_its_parent_moves_is_cleared() {
    let (mut app, b, c) = panel_with("left: 200px", Some(CHILD));
    assert_clean_after(&mut app, CHILD_OLD, |app| {
        b.set_style("top", "300px");
        b.set_style("left", "20px");
        c.unwrap().set_style("display", "none");
        resolve(app);
    });
}

/// The box moves and its overflowing child goes `visibility: hidden`.
#[test]
fn a_child_made_invisible_while_its_parent_moves_is_cleared() {
    let (mut app, b, c) = panel_with("left: 200px", Some(CHILD));
    assert_clean_after(&mut app, CHILD_OLD, |app| {
        b.set_style("top", "300px");
        b.set_style("left", "20px");
        c.unwrap().set_style("visibility", "hidden");
        resolve(app);
    });
}

/// A shadowed box moves (resolved), then is removed before the paint.
#[test]
fn a_shadowed_box_moved_then_removed_clears_its_old_shadow() {
    let (mut app, b) = panel(SHADOW);
    assert_clean_after(&mut app, SHADOW_BAND, |app| {
        b.set_style("left", "300px");
        b.set_style("top", "20px");
        resolve(app);
        b.remove();
        resolve(app);
    });
}

/// With no move at all: a shadowed box goes `display: none`. Its box is
/// zeroed, and the shadow outside it used to stay (pre-existing).
#[test]
fn a_shadowed_box_hidden_clears_its_shadow() {
    let (mut app, b) = panel(SHADOW);
    assert_clean_after(&mut app, SHADOW_BAND, |app| {
        b.set_style("display", "none");
        resolve(app);
    });
}

/// With no move at all: a shadowed box is removed. The removal recorded its
/// border box only, and the shadow outside it used to stay (pre-existing).
#[test]
fn a_shadowed_box_removed_clears_its_shadow() {
    let (mut app, b) = panel(SHADOW);
    assert_clean_after(&mut app, SHADOW_BAND, |app| {
        b.remove();
        resolve(app);
    });
}

/// A shadow added to a box that stays put reaches past the 4px margin, and
/// used to be clipped by the region (pre-existing).
#[test]
fn a_shadow_added_in_place_is_painted_whole() {
    let (mut app, b) = panel("");
    let _ = full_frame(&mut app);
    b.set_style("box-shadow", "0 0 24px 8px rgb(0, 0, 0)");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert!(
        ink_in(&full, SHADOW_BAND) > 100,
        "positive control: the new shadow is there"
    );
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

// ── Round 2 of the review of #880 ─────────────────────────────────────────

/// Mount whatever `build` makes; it returns the root and the handles a test
/// needs.
fn mount_with(
    build: impl Fn(&mut RenderScope) -> (NodeHandle, Vec<NodeHandle>) + 'static,
) -> (RinchApp, Vec<NodeHandle>) {
    let slot: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let (root, hs) = build(scope);
        *slot_in.borrow_mut() = hs;
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let hs = slot.borrow().clone();
    (app, hs)
}

/// A `div` styled `style`, appended to `parent`.
fn el(scope: &mut RenderScope, parent: &NodeHandle, style: &str) -> NodeHandle {
    let e = scope.create_element("div");
    e.set_attribute("style", style);
    parent.append_child(&e);
    e
}

/// An in-flow `position: relative` row in a narrow column is shifted down by
/// reflow (a spacer above it grows). Its absolute badge hangs far outside the
/// column, so neither the row's box nor the column's reaches it: only the
/// row's subtree walk does. `relative` is in the walk's gate for this.
#[test]
fn a_relative_row_shifted_by_reflow_takes_its_overflowing_badge_along() {
    let (mut app, hs) = mount_with(|scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let col = el(scope, &outer, "width: 100px");
        let spacer = el(scope, &col, "height: 100px");
        let row = el(
            scope,
            &col,
            "position: relative; height: 40px; background: rgb(0, 200, 0)",
        );
        el(
            scope,
            &row,
            "position: absolute; left: 200px; top: 0px; width: 40px; height: 40px; \
             background: rgb(200, 0, 0)",
        );
        (outer, vec![spacer])
    });
    assert_clean_after(&mut app, (200, 100, 240, 140), |app| {
        hs[0].set_style("height", "200px");
        resolve(app);
    });
}

/// A `position: relative` box moved by its own insets — an author move, not
/// reflow — takes its overflowing absolute child along.
#[test]
fn a_relative_box_moved_by_its_insets_takes_its_overflowing_child_along() {
    let (mut app, hs) = mount_with(|scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let p = el(
            scope,
            &outer,
            "position: relative; left: 200px; top: 150px; width: 40px; height: 40px; \
             background: rgb(0, 0, 200)",
        );
        el(
            scope,
            &p,
            "position: absolute; left: 60px; top: 0px; width: 40px; height: 40px; \
             background: rgb(200, 0, 0)",
        );
        (outer, vec![p])
    });
    assert_clean_after(&mut app, (260, 150, 300, 190), |app| {
        hs[0].set_style("left", "20px");
        hs[0].set_style("top", "300px");
        resolve(app);
    });
}

/// An inline-block chip moved by its IFC — the text before it grew, which
/// does not push the chip paint-dirty — and painted there by an incremental
/// frame; then its overflowing child is removed. The child's painted rect is
/// summed through the chip's `prev_layout`, which only the IFC's own position
/// write keeps level (`write_inline_positions`). Every frame here is
/// incremental: a full repaint in between would hide nothing, but it is not
/// what a running app paints.
#[test]
fn a_child_of_an_ifc_moved_chip_is_cleared_where_it_was_painted() {
    let (mut app, hs) = mount_with(|scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let p = el(
            scope,
            &outer,
            "width: 200px; font-size: 16px; line-height: 20px; font-family: sans-serif",
        );
        let t = scope.create_text("a");
        p.append_child(&t);
        let chip = scope.create_element("span");
        chip.set_attribute(
            "style",
            "display: inline-block; position: relative; width: 40px; height: 16px; \
             background: rgb(0, 0, 200)",
        );
        p.append_child(&chip);
        let kid = el(
            scope,
            &chip,
            "position: absolute; left: 300px; top: 100px; width: 40px; height: 40px; \
             background: rgb(200, 0, 0)",
        );
        (outer, vec![kid, t])
    });
    let _ = full_frame(&mut app);
    // Longer text pushes the chip right; its kid moves with it.
    hs[1].set_text("aaaaaaaaaaaa");
    resolve(&mut app);
    let (shown, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let kid = {
        let d = app.doc.as_ref().unwrap().borrow();
        rinch_dom::paint::painted_border_box(&d.tree, hs[0].node_id().0, 1.0)
    };
    let kid = (kid.x0 as i32, kid.y0 as i32, kid.x1 as i32, kid.y1 as i32);
    assert!(
        ink_in(&shown, kid) > 1000,
        "positive control: the moved kid is painted at {kid:?}"
    );
    hs[0].remove();
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert_eq!(diff_in(&inc, &full, kid), 0, "the kid ghosts at {kid:?}");
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

/// A shadow **offset up and left** reaches past the left and top of the box
/// by its offset: the left reach is `reach - offset_x`, so a sign slip there
/// is invisible at offset 0 — where every other fixture sits. Dropped in
/// place rather than moved: a moved positioned box walks its subtree, which
/// measures the shadow itself and would cover a wrong own-ink reach. Here the
/// ink the box was *painted* with is the only thing that reaches the band.
/// The whole-frame check catches the top band too.
#[test]
fn a_shadow_offset_up_and_left_dropped_in_place_is_cleared() {
    let (mut app, b) = panel("box-shadow: -16px -16px 0 0 rgb(0, 0, 0)");
    assert_clean_after(&mut app, (4, 136, 14, 172), |app| {
        b.set_style("box-shadow", "none");
        resolve(app);
    });
}

/// The same the other way round: a shadow offset down and right, on a box
/// that is removed. The band right of the box is outside the union of the
/// holder and the bare box.
#[test]
fn a_removed_box_clears_a_shadow_offset_down_and_right() {
    let (mut app, b) = panel("box-shadow: 16px 16px 0 0 rgb(0, 0, 0)");
    assert_clean_after(&mut app, (66, 168, 76, 204), |app| {
        b.remove();
        resolve(app);
    });
}

/// An `outline` is painted outside the border box, `width + offset` away —
/// dropped in place, for the same reason as the offset shadow above.
#[test]
fn an_outline_dropped_in_place_is_cleared() {
    let (mut app, b) = panel("outline: 10px solid rgb(0, 0, 0); outline-offset: 4px");
    assert_clean_after(&mut app, (6, 152, 15, 188), |app| {
        b.set_style("outline", "none");
        resolve(app);
    });
}

// ── Cost ─────────────────────────────────────────────────────────────────

/// `compute_dirty_region` on a reflow that shifts every row: a header grows
/// above `rows` flex rows of `per_row` children each. Best of 10, in µs, and
/// the number of paint-dirty entries it walked.
fn reflow_region_micros(rows: usize, per_row: usize, width: u32) -> (u128, usize) {
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", &format!("width: {width}px"));
        let head = scope.create_element("div");
        head.set_attribute("style", "height: 10px");
        root.append_child(&head);
        for _ in 0..rows {
            let r = scope.create_element("div");
            r.set_attribute("style", "height: 20px; display: flex");
            for _ in 0..per_row {
                let s = scope.create_element("div");
                s.set_attribute("style", "width: 1px; height: 4px; background: red");
                r.append_child(&s);
            }
            root.append_child(&r);
        }
        *slot_in.borrow_mut() = Some(head);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let _ = full_frame(&mut app);
    let head = slot.borrow().clone().unwrap();
    let mut best = u128::MAX;
    let mut dirty = 0;
    for i in 0..10 {
        head.set_style("height", if i % 2 == 0 { "30px" } else { "10px" });
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        let doc = app.doc.as_ref().unwrap().clone();
        let t = std::time::Instant::now();
        {
            let d = doc.borrow();
            dirty = d.tree.paint_dirty_nodes.len();
            let _ = rinch_dom::paint::compute_dirty_region(&d.tree, 1.0, 600.0, 400.0);
        }
        best = best.min(t.elapsed().as_micros());
        let _ = incremental_frame(&mut app);
    }
    (best, dirty)
}

/// The review of #880 measured this shape at 29 µs on `main` and 3579 µs on
/// the first cut of #880 (100 × 500, release), from a subtree walk per
/// shifted row. Run in release with `-- --ignored --nocapture`.
#[test]
#[ignore = "timing harness; prints, asserts nothing"]
fn reflow_region_cost() {
    // The first two are the review's shapes: the rows span the window, so
    // the region passes the full-repaint fraction at the first rect and the
    // measuring stops. The third keeps the region under it (a 50px-wide
    // column), so every dirty row is measured — the cost of the walk itself.
    for (rows, per_row, width) in [(400, 10, 600), (100, 500, 600), (100, 500, 50)] {
        let (us, dirty) = reflow_region_micros(rows, per_row, width);
        eprintln!(
            "REFLOW rows={rows} per_row={per_row} width={width} dirty_nodes={dirty} \
             compute_dirty_region best {us} us"
        );
    }
}

// ── The rich-text editor's overlays ──────────────────────────────────────

#[cfg(feature = "desktop")]
mod editor {
    use super::*;
    use rinch_editor_core::{Pos, Selection};

    struct Page {
        app: RinchApp,
        handle: crate::editor::EditorHandle,
    }

    fn ev(app: &mut RinchApp, event: PlatformEvent) {
        let _ = app.handle_event(event, SIZE, 1.0);
    }

    /// One key press and release, through the arbiter to the focused editor —
    /// the path a real keystroke takes (`dispatch_new_editor_key`, then
    /// `refresh_editor_overlays`, then `resolve_and_repaint`).
    fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
        let modifiers = Modifiers::default();
        ev(
            app,
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: text.map(str::to_string),
                modifiers,
                repeat: KeyRepeat::Unknown,
            },
        );
        ev(
            app,
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers,
            },
        );
    }

    /// What the shell does between the input and the paint: `AboutToWait`'s
    /// resolve (which short-circuits when the handler already resolved).
    fn about_to_wait(app: &mut RinchApp) {
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    }

    fn page() -> Page {
        let handle = crate::editor::create_editor();
        assert!(handle.load_html("<p>hello world, a line of text</p><p>second line</p>"));
        let h = handle.clone();
        let ed = Rc::new(std::cell::Cell::new(0usize));
        let ed_in = ed.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "padding: 20px; width: 400px; font-size: 16px; line-height: 20px; \
                 font-family: sans-serif",
            );
            let e = h.mount(scope);
            ed_in.set(e.node_id().0);
            root.append_child(&e);
            root
        });
        app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        app.focus_target = FocusTarget::Editor(ed.get());
        Page { app, handle }
    }

    /// A page with the caret at `pos`, painted from scratch; returns that
    /// frame and the caret's painted rect in it.
    fn painted_at(pos: usize) -> (Page, Vec<u8>, (i32, i32, i32, i32)) {
        let mut p = page();
        p.handle.set_selection(Selection::cursor(Pos(pos)));
        p.app.refresh_editor_overlays();
        about_to_wait(&mut p.app);
        let before = full_frame(&mut p.app);
        let old = caret_rect(&p.app);
        assert!(
            caret_px(&before, old) > 5,
            "positive control: the caret is painted at {old:?}"
        );
        (p, before, old)
    }

    /// The caret overlay's painted rect, in physical px, padded by a pixel.
    fn caret_rect(app: &RinchApp) -> (i32, i32, i32, i32) {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let id = d
            .tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.contains_key("data-pm-caret"))
            .map(|(id, _)| id)
            .expect("the editor has a caret overlay");
        let r = rinch_dom::paint::painted_border_box(&d.tree, id, 1.0);
        assert!(
            r.width() > 0.0 && r.height() > 0.0,
            "the caret has a box: {r:?}"
        );
        (
            r.x0.floor() as i32 - 1,
            r.y0.floor() as i32 - 1,
            r.x1.ceil() as i32 + 1,
            r.y1.ceil() as i32 + 1,
        )
    }

    /// The caret's exact colour (`#1a73e8`, opaque), which nothing else in
    /// this page paints.
    fn caret_px(px: &[u8], rect: (i32, i32, i32, i32)) -> usize {
        let (w, h) = (SIZE.0 as i32, SIZE.1 as i32);
        let mut n = 0;
        for y in rect.1.max(0)..rect.3.min(h) {
            for x in rect.0.max(0)..rect.2.min(w) {
                let i = ((y * w + x) * 4) as usize;
                if px[i] == 26 && px[i + 1] == 115 && px[i + 2] == 232 {
                    n += 1;
                }
            }
        }
        n
    }

    /// After an input: the frame is a small region, the caret moved, none of
    /// it is left at `old`, and the frame equals a from-scratch one.
    fn assert_moved_cleanly(p: &mut Page, old: (i32, i32, i32, i32)) {
        about_to_wait(&mut p.app);
        let (inc, stats) = incremental_frame(&mut p.app);
        assert_incremental(&stats);
        assert!(
            stats.get(Counter::RepaintedPx) < stats.get(Counter::SurfacePx) / 10,
            "an editor input repaints a small region: {stats:?}"
        );
        let new = caret_rect(&p.app);
        assert_ne!(old, new, "positive control: the caret moved");
        assert_eq!(
            caret_px(&inc, old),
            0,
            "the caret ghosts at its old column {old:?}"
        );
        assert!(
            caret_px(&inc, new) > 5,
            "and is drawn at the new one {new:?}"
        );
        let full = full_frame(&mut p.app);
        assert_eq!(
            diff_in(&inc, &full, (0, 0, 600, 400)),
            0,
            "incremental frame != full frame"
        );
    }

    /// Arrow keys move the caret with no document change. Several between
    /// two paints (key repeat outruns a slow frame): each handler resolves,
    /// so the caret is laid out at every intermediate column and only the
    /// first one was ever painted. One press is not enough to see it — the
    /// last step then *is* the whole move — and neither are two: a step is
    /// about the dirty margin's 4px, so the region grown around the last step
    /// reaches back over the one before it.
    #[test]
    fn arrow_keys_between_paints_repaint_a_region_and_leave_no_ghost() {
        let (mut p, _, old) = painted_at(3);
        for _ in 0..8 {
            key(&mut p.app, KeyCode::ArrowRight, None);
        }
        assert_eq!(p.handle.selection(), Selection::cursor(Pos(11)));
        assert_moved_cleanly(&mut p, old);
    }

    /// A typed character changes the document and the caret with it, and
    /// used to repaint the whole window. The edited paragraph is paint-dirty
    /// in its own right and its box covers the caret's old column, so this
    /// one pins the region and the counters more than the old rect.
    #[test]
    fn a_keystroke_repaints_a_region_and_leaves_no_ghost() {
        let (mut p, _, old) = painted_at(6);
        key(&mut p.app, KeyCode::KeyW, Some("W"));
        assert_eq!(
            p.handle.selection(),
            Selection::cursor(Pos(7)),
            "the key typed"
        );
        assert_moved_cleanly(&mut p, old);
    }

    /// Several keystrokes between two paints — a fast typist or a slow frame.
    #[test]
    fn several_keystrokes_between_paints_leave_no_ghost() {
        let (mut p, _, old) = painted_at(6);
        for (k, t) in [
            (KeyCode::KeyW, "W"),
            (KeyCode::KeyA, "A"),
            (KeyCode::KeyW, "W"),
        ] {
            key(&mut p.app, k, Some(t));
        }
        assert_moved_cleanly(&mut p, old);
    }

    /// The overlays are placed by a `transform` since #906, and a transformed
    /// box is a stacking context that paint enters through its clip chain —
    /// so a caret scrolled under its editor's `overflow` edge must still be
    /// cut there, and moving it must still repaint a region equal to a
    /// from-scratch frame.
    ///
    /// The editor is a 60px scroller 40px down the window. The caret is put
    /// on the third line (placing it scrolls the editor to show it), then the
    /// user scrolls to 110px, so the caret's box (window rows 7..33 as
    /// measured, about 77px down the content) lies wholly above the
    /// scroller's top edge. Off the fixed point: the caret's translation is
    /// not zero, so its untransformed `top: 0` box (window rows -70..-44) and
    /// its painted one are different rects. A
    /// local oracle over the band above the scroller, where correct output
    /// holds no caret colour; and an ArrowRight (which scrolls the caret back
    /// into view) repaints a frame equal to a full one.
    #[test]
    fn a_caret_scrolled_past_the_editor_edge_is_clipped_and_moves_cleanly() {
        let handle = crate::editor::create_editor();
        let mut html = String::new();
        for i in 0..8 {
            html.push_str(&format!("<p>line {i} of the scrolled editor</p>"));
        }
        assert!(handle.load_html(&html));
        let h = handle.clone();
        let ed = Rc::new(std::cell::Cell::new(0usize));
        let ed_in = ed.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "padding-top: 40px; width: 400px; font-size: 16px; line-height: 20px; \
                 font-family: sans-serif",
            );
            let e = h.mount(scope);
            e.set_attribute(
                "style",
                "height: 60px; overflow-y: auto; padding: 0; border: 0; border-radius: 0",
            );
            ed_in.set(e.node_id().0);
            root.append_child(&e);
            root
        });
        app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        app.focus_target = FocusTarget::Editor(ed.get());
        let mut p = Page { app, handle };
        p.handle
            .set_selection(Selection::cursor(Pos(1 + 31 * 2 + 3)));
        p.app.refresh_editor_overlays();
        about_to_wait(&mut p.app);
        let shown = full_frame(&mut p.app);
        let r0 = caret_rect(&p.app);
        assert!(
            caret_px(&shown, r0) > 5,
            "positive control: the caret is painted at {r0:?} before the scroll"
        );
        {
            let doc = p.app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.tree.nodes[ed.get()].scroll_offset.1 = 110.0;
            d.tree.dirty_nodes.insert(ed.get());
            d.tree.hit_cache.invalidate();
        }
        about_to_wait(&mut p.app);
        let full = full_frame(&mut p.app);
        let r = caret_rect(&p.app);
        assert!(
            r.3 <= 41 && r.1 >= 5,
            "precondition: the scroll put the caret's box {r:?} above the scroller (y = 40)"
        );
        assert_eq!(
            caret_px(&full, (0, 0, 600, 40)),
            0,
            "the transformed caret escaped its editor's clip"
        );
        key(&mut p.app, KeyCode::ArrowRight, None);
        about_to_wait(&mut p.app);
        let (inc, stats) = incremental_frame(&mut p.app);
        assert_eq!(stats.get(Counter::PaintFrames), 1, "{stats:?}");
        let moved = caret_rect(&p.app);
        assert!(
            caret_px(&inc, moved) > 5,
            "positive control: the caret was scrolled back into view at {moved:?}"
        );
        assert_eq!(caret_px(&inc, (0, 0, 600, 40)), 0, "and nothing above it");
        let full = full_frame(&mut p.app);
        assert_eq!(
            diff_in(&inc, &full, (0, 0, 600, 400)),
            0,
            "incremental frame != full frame"
        );
    }

    /// The node outline lands on the node it selects. Since #906 every
    /// overlay sits at `left: 0; top: 0` and is placed by its `transform`, so
    /// the anchor is load-bearing: without it the outline falls to its static
    /// position after the last block (measured by the review of #953: 200px
    /// low) and nothing else noticed. A selected `<hr>`'s outline must cover
    /// the `<hr>`'s own painted box, within one pixel. (Measured: 1px right
    /// of and below it, the container's border width — the offset the caret
    /// overlay has too, which CLAUDE.md records against `caret_rect`.) The page sits at a fractional
    /// offset (`padding: 20.3px 13.7px`) so the pin is not on the whole-px
    /// fixed point, and the `<hr>` is below a paragraph so the translation is
    /// not zero; a pixel check that the outline's colour is drawn there
    /// guards the geometry read.
    #[test]
    fn a_selected_rule_is_outlined_where_the_rule_is_painted() {
        let handle = crate::editor::create_editor();
        assert!(handle.load_html("<p>above the rule</p><hr><p>below the rule</p><p>tail</p>"));
        let h = handle.clone();
        let ed = Rc::new(std::cell::Cell::new(0usize));
        let ed_in = ed.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "padding: 20.3px 13.7px; width: 400px; font-size: 16px; line-height: 20px; \
                 font-family: sans-serif",
            );
            let e = h.mount(scope);
            ed_in.set(e.node_id().0);
            root.append_child(&e);
            root
        });
        app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        app.focus_target = FocusTarget::Editor(ed.get());
        let mut p = Page { app, handle };
        // "above the rule" is 14 chars: the paragraph spans 0..16, the rule
        // starts at 16.
        let doc = p.handle.doc();
        let sel = Selection::node_at(&doc, Pos(16)).expect("a node selection at the rule");
        assert_eq!(
            sel.node(&doc).map(|n| n.type_name().to_string()).as_deref(),
            Some("horizontal_rule"),
            "precondition: the selection is the rule"
        );
        p.handle.set_selection(sel);
        p.app.refresh_editor_overlays();
        about_to_wait(&mut p.app);
        let frame = full_frame(&mut p.app);
        let d = p.app.doc.as_ref().unwrap().borrow();
        let find = |pred: &dyn Fn(&rinch_dom::node::Node) -> bool| {
            d.tree
                .nodes
                .iter()
                .find(|(_, n)| pred(n))
                .map(|(id, _)| rinch_dom::paint::painted_border_box(&d.tree, id, 1.0))
                .expect("found")
        };
        let rule = find(&|n| n.tag() == Some("hr"));
        let outline = find(&|n| n.attributes.contains_key("data-pm-selected"));
        assert!(
            rule.y0 > 40.0 && rule.height() > 0.0,
            "precondition: the rule is laid out below the first paragraph: {rule:?}"
        );
        // Not the bottom edge: the outline is `box-sizing: border-box` with a
        // 2px border, so over a 2px rule it is 4px tall whatever its height.
        assert!(
            outline.y1 >= rule.y1,
            "the outline covers the rule's bottom"
        );
        for (a, b, edge) in [
            (outline.x0, rule.x0, "left"),
            (outline.y0, rule.y0, "top"),
            (outline.x1, rule.x1, "right"),
        ] {
            assert!(
                (a - b).abs() <= 1.0,
                "the outline's {edge} edge {a} is off the rule's {b}: outline {outline:?}, rule {rule:?}"
            );
        }
        // The outline's 2px `#1a73e8` border, drawn at its left edge.
        let (x, y) = (outline.x0 as i32, ((outline.y0 + outline.y1) / 2.0) as i32);
        let i = ((y * SIZE.0 as i32 + x) * 4) as usize;
        assert!(
            frame[i + 2] > 200 && frame[i] < 100,
            "positive control: the outline's border is painted at ({x}, {y}): {:?}",
            &frame[i..i + 4]
        );
    }

    /// A drag-select: the selection rects are created, grow and shrink frame
    /// by frame, and the caret goes away and comes back. Every frame is a
    /// region, and every one matches a from-scratch frame.
    #[test]
    fn a_selection_growing_and_collapsing_repaints_regions_that_match_full_frames() {
        let (mut p, _, _) = painted_at(2);
        for sel in [
            Selection::text(Pos(2), Pos(8)),
            Selection::text(Pos(2), Pos(30)),
            Selection::text(Pos(2), Pos(5)),
            Selection::cursor(Pos(12)),
        ] {
            p.handle.set_selection(sel.clone());
            p.app.refresh_editor_overlays();
            about_to_wait(&mut p.app);
            let (inc, stats) = incremental_frame(&mut p.app);
            assert_incremental(&stats);
            let full = full_frame(&mut p.app);
            assert_eq!(
                diff_in(&inc, &full, (0, 0, 600, 400)),
                0,
                "incremental frame != full frame after {sel:?}"
            );
        }
    }
}

// ── `text-shadow` is ink too (#980) ──────────────────────────────────────

/// A box at (20, 150) holding a line of text whose `text-shadow` falls 60px
/// below it and is blurred by 6px — wholly outside the box and past the 4px
/// dirty margin, so only the shadow's own ink reach can name its pixels.
fn text_panel(shadow: &str) -> (RinchApp, NodeHandle) {
    let shadow = shadow.to_string();
    let (app, hs) = mount_with(move |scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let root = el(
            scope,
            &outer,
            "position: relative; width: 10px; height: 10px",
        );
        let b = el(
            scope,
            &root,
            &format!(
                "position: absolute; left: 20px; top: 150px; width: 200px; height: 20px; \
                 font-size: 16px; line-height: 20px; color: rgb(0, 0, 0); \
                 text-shadow: {shadow}"
            ),
        );
        let t = scope.create_text("HHHH HHHH");
        b.append_child(&t);
        (outer, vec![b])
    });
    (app, hs[0].clone())
}

/// The band the shadow of [`text_panel`]'s text is painted in.
const TEXT_SHADOW_BAND: (i32, i32, i32, i32) = (20, 206, 80, 234);

/// A **static** row shifted down by reflow (a spacer above it grows), its
/// text's shadow 60px below its box: nothing walks a static box's subtree, so
/// only the row's own text-shadow ink names where the old shadow was painted.
/// (A moved *positioned* box is grown by its subtree's reach already, so it
/// cannot tell whether the shadow is ink — review of #1020, F6.)
#[test]
fn a_static_row_shifted_by_reflow_clears_its_old_text_shadow() {
    let (mut app, hs) = mount_with(|scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let col = el(scope, &outer, "width: 300px");
        let spacer = el(scope, &col, "height: 100px");
        let row = el(
            scope,
            &col,
            "height: 20px; font-size: 16px; line-height: 20px; color: rgb(0, 0, 0); \
             text-shadow: 0 60px 6px rgb(255, 0, 0)",
        );
        let t = scope.create_text("HHHH HHHH");
        row.append_child(&t);
        (outer, vec![spacer])
    });
    // The text at y 100..120 casts its shadow at 160..180, softened by 6px.
    assert_clean_after(&mut app, (0, 154, 90, 186), |app| {
        hs[0].set_style("height", "40px");
        resolve(app);
    });
}

/// A widely blurred shadow's soft edge reaches past the hard shadow and the
/// 4px dirty margin: only the blur's share of the ink names it (review of
/// #1020, F5).
#[test]
fn a_widely_blurred_text_shadow_dropped_in_place_is_cleared() {
    let (mut app, b) = text_panel("0 60px 20px rgb(255, 0, 0)");
    // The hard shadow is at y 210..230; its blurred tail is checked at 236..250.
    assert_clean_after(&mut app, (20, 236, 120, 250), |app| {
        b.set_style("text-shadow", "none");
        resolve(app);
    });
}

/// A text-shadow dropped in place is cleared.
#[test]
fn a_text_shadow_dropped_in_place_is_cleared() {
    let (mut app, b) = text_panel("0 60px 6px rgb(255, 0, 0)");
    assert_clean_after(&mut app, TEXT_SHADOW_BAND, |app| {
        b.set_style("text-shadow", "none");
        resolve(app);
    });
}

/// A text-shadow added in place is painted whole, blur and all.
#[test]
fn a_text_shadow_added_in_place_is_painted_whole() {
    let (mut app, b) = text_panel("none");
    let _ = full_frame(&mut app);
    b.set_style("text-shadow", "0 60px 6px rgb(255, 0, 0)");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert!(
        ink_in(&full, TEXT_SHADOW_BAND) > 100,
        "positive control: the new shadow is there"
    );
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

// ── #1048: a span's own `text-shadow` is ink of the paragraph it flows in ──

/// [`text_panel`], but the box declares no shadow and its text sits in a
/// `<span>` that does: a flowed inline owns no box, so only the IFC root's
/// rect, grown by its **members'** shadow reach, can name the shadow's pixels.
fn span_text_panel(shadow: &str) -> (RinchApp, NodeHandle) {
    let shadow = shadow.to_string();
    let (app, hs) = mount_with(move |scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let root = el(
            scope,
            &outer,
            "position: relative; width: 10px; height: 10px",
        );
        let b = el(
            scope,
            &root,
            "position: absolute; left: 20px; top: 150px; width: 200px; height: 20px; \
             font-size: 16px; line-height: 20px; color: rgb(0, 0, 0)",
        );
        let span = scope.create_element("span");
        span.set_attribute("style", &format!("text-shadow: {shadow}"));
        b.append_child(&span);
        let t = scope.create_text("HHHH HHHH");
        span.append_child(&t);
        (outer, vec![span])
    });
    (app, hs[0].clone())
}

/// A span's text-shadow dropped in place is cleared (#1048).
///
/// Kills: the span's damage falling back to the paragraph's bare box (the
/// shadow 60px below it is never repainted).
#[test]
fn a_span_text_shadow_dropped_in_place_is_cleared() {
    let (mut app, span) = span_text_panel("0 60px 6px rgb(255, 0, 0)");
    assert_clean_after(&mut app, TEXT_SHADOW_BAND, |app| {
        span.set_style("text-shadow", "none");
        resolve(app);
    });
}

/// A span's text-shadow added in place is painted whole (#1048).
///
/// Kills: the span's new reach missing from the damage, or the paint prune
/// testing the paragraph's own ink only (its box is outside the band, so the
/// incremental frame never draws the shadow there).
#[test]
fn a_span_text_shadow_added_in_place_is_painted_whole() {
    let (mut app, span) = span_text_panel("none");
    let _ = full_frame(&mut app);
    span.set_style("text-shadow", "0 60px 6px rgb(255, 0, 0)");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert!(
        ink_in(&full, TEXT_SHADOW_BAND) > 100,
        "positive control: the span's new shadow is there"
    );
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

/// A static paragraph shifted down by reflow, its span's shadow 60px below its
/// box: the paragraph's painted ink must hold its span's shadow reach (#1048).
///
/// Kills: `PaintedState` recording the IFC root's own ink only (the old shadow
/// is left behind).
#[test]
fn a_static_row_shifted_by_reflow_clears_its_spans_old_text_shadow() {
    let (mut app, hs) = mount_with(|scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let col = el(scope, &outer, "width: 300px");
        let spacer = el(scope, &col, "height: 100px");
        let row = el(
            scope,
            &col,
            "height: 20px; font-size: 16px; line-height: 20px; color: rgb(0, 0, 0)",
        );
        let span = scope.create_element("span");
        span.set_attribute("style", "text-shadow: 0 60px 6px rgb(255, 0, 0)");
        row.append_child(&span);
        let t = scope.create_text("HHHH HHHH");
        span.append_child(&t);
        (outer, vec![spacer])
    });
    assert_clean_after(&mut app, (0, 154, 90, 186), |app| {
        hs[0].set_style("height", "40px");
        resolve(app);
    });
}

/// The same paragraph pushed **down** by reflow: its column's box ends above
/// the new shadow, so only the paragraph's current ink — its span's shadow
/// reach — names where the shadow is drawn now (#1048).
///
/// Kills: a paint-dirty IFC root's damage grown by its own ink only (the moved
/// shadow is never drawn in the incremental frame).
#[test]
fn a_static_row_pushed_down_by_reflow_draws_its_spans_shadow_where_it_is_now() {
    let (mut app, hs) = mount_with(|scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let col = el(scope, &outer, "width: 300px");
        let spacer = el(scope, &col, "height: 100px");
        let row = el(
            scope,
            &col,
            "height: 20px; font-size: 16px; line-height: 20px; color: rgb(0, 0, 0)",
        );
        let span = scope.create_element("span");
        span.set_attribute("style", "text-shadow: 0 60px 6px rgb(255, 0, 0)");
        row.append_child(&span);
        let t = scope.create_text("HHHH HHHH");
        span.append_child(&t);
        (outer, vec![spacer])
    });
    let _ = full_frame(&mut app);
    // The row moves from 100..120 to 160..180; its shadow to 220..240.
    hs[0].set_style("height", "160px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert!(
        ink_in(&full, (0, 214, 90, 246)) > 100,
        "positive control: the shadow is drawn at its new place"
    );
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

/// A span's shadow given in place, then dropped in the frame reflow moves its
/// paragraph: the paragraph was not itself dirty for the restyle (only its
/// span was), so its painted ink must have been brought up to the span's new
/// shadow when that frame was consumed (#1048).
///
/// Kills: `consume_paint_dirty` not refreshing the IFC root's ink when a
/// box-less member is consumed (the moved paragraph's old shadow stays).
#[test]
fn a_span_shadow_added_then_dropped_as_reflow_moves_it_is_cleared() {
    let (mut app, hs) = mount_with(|scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let col = el(scope, &outer, "width: 300px");
        let spacer = el(scope, &col, "height: 100px");
        let row = el(
            scope,
            &col,
            "height: 20px; font-size: 16px; line-height: 20px; color: rgb(0, 0, 0)",
        );
        let span = scope.create_element("span");
        row.append_child(&span);
        let t = scope.create_text("HHHH HHHH");
        span.append_child(&t);
        (outer, vec![spacer, span])
    });
    let _ = full_frame(&mut app);
    hs[1].set_style("text-shadow", "0 60px 6px rgb(255, 0, 0)");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    assert!(
        ink_in(&inc, (0, 154, 90, 186)) > 100,
        "positive control: the shadow is drawn"
    );
    // Moved and dropped in one frame: neither the paragraph's ink now nor the
    // span's names where the old shadow is, only the paragraph's painted ink.
    hs[0].set_style("height", "40px");
    hs[1].set_style("text-shadow", "none");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

/// A box under a span's shadow repaints, and nothing else does: the paint
/// prune must enter the paragraph for its span's shadow, though its box is
/// nowhere near the damage (#1048).
///
/// Kills: the prune testing the paragraph's own ink only (the shadow is
/// cleared with the box's old background and never drawn back).
#[test]
fn a_box_under_a_span_shadow_repaints_the_shadow_over_it() {
    let (mut app, hs) = mount_with(|scope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "position: relative; width: 600px; height: 400px");
        let under = el(
            scope,
            &outer,
            "position: absolute; left: 10px; top: 200px; width: 200px; height: 40px; \
             background: rgb(200, 200, 255)",
        );
        let row = el(
            scope,
            &outer,
            "position: absolute; left: 20px; top: 150px; width: 200px; height: 20px; \
             font-size: 16px; line-height: 20px; color: rgb(0, 0, 0)",
        );
        let span = scope.create_element("span");
        span.set_attribute("style", "text-shadow: 0 60px 0 rgb(255, 0, 0)");
        row.append_child(&span);
        let t = scope.create_text("HHHH HHHH");
        span.append_child(&t);
        (outer, vec![under])
    });
    let _ = full_frame(&mut app);
    hs[0].set_style("background", "rgb(200, 255, 200)");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    let (w, (x0, y0, x1, y1)) = (SIZE.0 as i32, TEXT_SHADOW_BAND);
    let red = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| ((y * w + x) * 4) as usize))
        .filter(|&i| full[i] > 200 && full[i + 1] < 80 && full[i + 2] < 80)
        .count();
    assert!(
        red > 50,
        "positive control: the shadow is over the box ({red})"
    );
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
}

// ── #997: backface-visibility and transform-origin z ─────────────────────────
/// A change that hides or shows a box through its backface, or moves it by its
/// origin's z, repaints incrementally and matches a full frame. The first pins
/// `PaintedTransform::backface_hidden`: a painted frame that claimed "hidden"
/// for a box it drew would name no old rect, and the box would ghost.
mod backface_997 {
    use super::*;

    fn check_flip(start: &'static str, prop: &str, val: &str, want_ink_after: bool) {
        let (mut app, b) = panel(start);
        let before = full_frame(&mut app);
        let rect = (20, 150, 60, 190);
        let had = ink_in(&before, rect);
        b.set_style(prop, val);
        resolve(&mut app);
        let (inc, stats) = incremental_frame(&mut app);
        let full = full_frame(&mut app);
        assert_ne!(
            had > 0,
            want_ink_after,
            "[{start}]: the change must change the rect"
        );
        assert_eq!(
            stats.get(Counter::RepaintFull),
            0,
            "[{start}] repainted in full"
        );
        assert_eq!(ink_in(&full, rect) > 0, want_ink_after, "full frame");
        assert_eq!(diff_in(&inc, &full, (0, 0, 600, 400)), 0, "inc != full");
    }

    #[test]
    fn backface_toggle_to_hidden_clears() {
        check_flip(
            "transform: rotateY(180deg)",
            "backface-visibility",
            "hidden",
            false,
        );
    }
    #[test]
    fn backface_toggle_to_visible_draws() {
        check_flip(
            "transform: rotateY(180deg); backface-visibility: hidden",
            "backface-visibility",
            "visible",
            true,
        );
    }
    #[test]
    fn transform_turns_past_90_clears() {
        check_flip(
            "transform: rotateY(60deg); backface-visibility: hidden",
            "transform",
            "rotateY(120deg)",
            false,
        );
    }
    #[test]
    fn transform_turns_back_draws() {
        check_flip(
            "transform: rotateY(120deg); backface-visibility: hidden",
            "transform",
            "rotateY(60deg)",
            true,
        );
    }
    #[test]
    fn origin_z_change_repaints() {
        check_flip(
            "transform: rotateY(45deg); transform-origin: 50% 50% 0px",
            "transform-origin",
            "50% 50% 200px",
            false,
        );
    }

    /// **Not Chrome's (#386, #415).** Chrome 153 does not draw a
    /// `position: fixed` child of a turned hidden-backface box: the transformed
    /// box is its containing block. rinch models no such containment and hands
    /// a fixed entry the body's transform, so the zero matrix never reaches it
    /// and it is drawn. This pins the divergence; containment flips it.
    #[test]
    fn a_fixed_child_of_a_hidden_backface_is_still_drawn() {
        let (mut app, _b, _c) = panel_with(
            "transform: rotateY(180deg); backface-visibility: hidden",
            Some(
                "position: fixed; left: 300px; top: 300px; width: 40px; height: 40px; background: rgb(0, 200, 0)",
            ),
        );
        let full = full_frame(&mut app);
        let ink = ink_in(&full, (300, 300, 340, 340));
        assert!(ink > 0, "the fixed child is drawn until #415 contains it");
    }
}
