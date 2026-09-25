//! Damage is clipped by what clips the damaged box — and by nothing that does
//! not (#909).
//!
//! A change inside a scroller used to name damage wherever the changed boxes
//! were, clipped or not: a keyed `for` reorder in a 300x400 scroller repainted
//! down to the window's bottom edge. Each damage rect is now intersected with
//! its node's clip chain (`paint::clip_chain_bounds`): the current chain for
//! where the box paints now, the chain **as it was painted** for where its old
//! pixels are, and a removed node's rect at the moment it leaves.
//!
//! Every fixture is a local pixel oracle in `repaint_old_rect_tests`'s idiom:
//! the incremental frame must equal a from-scratch frame of the same state,
//! with a positive control that the pixels in question were really there, and
//! a counter check that the frame really was incremental (an accidental full
//! repaint passes every pixel assertion vacuously). Under-clipping only costs
//! pixels repainted; **over**-clipping leaves a ghost, so each fixture puts the
//! ink somewhere a too-eager clip would cut away, and names the mutant.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};

const SIZE: (u32, u32) = (600, 400);

fn full_frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

fn incremental_frame(app: &mut RinchApp) -> (Vec<u8>, FrameStats) {
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let stats = app.end_perf_frame().expect("mounted");
    (px, stats)
}

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

fn resolve(app: &mut RinchApp) {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
}

const WHOLE: (i32, i32, i32, i32) = (0, 0, 600, 400);

/// `outer` (600x400, `outer_style` appended) > `clipper` (at 20,20, 200x100,
/// `clipper_style` appended) > `box` (40x40 blue, `box_style` appended), plus
/// an optional `<style>` sheet. Returns the app, the clipper and the box.
fn mount(
    sheet: &'static str,
    outer_style: &'static str,
    clipper_style: &'static str,
    box_style: &'static str,
) -> (RinchApp, NodeHandle, NodeHandle) {
    type Slot = Option<(NodeHandle, NodeHandle)>;
    let slot: Rc<RefCell<Slot>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute(
            "style",
            &format!("width: 600px; height: 400px; {outer_style}"),
        );
        if !sheet.is_empty() {
            let style = scope.create_element("style");
            let css = scope.create_text(sheet);
            style.append_child(&css);
            outer.append_child(&style);
        }
        let clipper = scope.create_element("div");
        clipper.set_attribute("class", "clipper");
        clipper.set_attribute(
            "style",
            &format!(
                "margin-left: 20px; margin-top: 20px; width: 200px; height: 100px; {clipper_style}"
            ),
        );
        outer.append_child(&clipper);
        let b = scope.create_element("div");
        b.set_attribute(
            "style",
            &format!("width: 40px; height: 40px; background: rgb(0, 0, 200); {box_style}"),
        );
        clipper.append_child(&b);
        *slot_in.borrow_mut() = Some((clipper, b));
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let (c, b) = slot.borrow().clone().unwrap();
    (app, c, b)
}

/// **The issue's shape, with its pixels checked.** A box inside a clipper moves
/// from on screen to below the clip: its old pixels are cleared (inside the
/// clip), its new rect names nothing (outside it), and the frame repaints only
/// the old rect — not the column down to where the box went.
#[test]
fn a_box_moved_out_of_its_clip_is_cleared_and_names_nothing_past_it() {
    let (mut app, _clipper, b) = mount("", "", "overflow: hidden", "margin-top: 10px");
    let before = full_frame(&mut app);
    let old = (20, 30, 60, 70);
    assert!(ink_in(&before, old) > 1000, "positive control");

    b.set_style("margin-top", "300px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
    assert_eq!(ink_in(&inc, old), 0);
    // The box's new rect is 320..360, all of it clipped away: nothing at or
    // past the clipper's bottom (120 + the 4px margin) is damage.
    let px = stats.get(Counter::RepaintedPx);
    assert!(
        px <= 48 * 48,
        "repainted {px} px: damage reaches past the clip"
    );
}

/// **An absolute box escapes a clipper below its containing block.** The
/// clipper is static, the containing block is `outer` above it, so the box at
/// `top: 200px` paints outside the clipper's box. Moving it must clear it
/// there and draw it at its new place.
///
/// Kills: the chain walk taking every clipping ancestor regardless of
/// `position: absolute` (both rects clipped to the clipper: the old box ghosts
/// and the new one is never drawn).
#[test]
fn an_absolute_box_escaping_the_clip_is_damaged_outside_it() {
    let (mut app, _c, b) = mount(
        "",
        "position: relative",
        "overflow: hidden",
        "position: absolute; left: 300px; top: 200px",
    );
    let before = full_frame(&mut app);
    // At (320, 220): rinch resolves an absolute box against its direct
    // parent when its containing block is further up (#386) — and paints it
    // unclipped all the same, since the clip chain is truncated at the block.
    let old = (320, 220, 360, 260);
    assert!(ink_in(&before, old) > 1000, "positive control");

    b.set_style("left", "400px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert!(
        ink_in(&full, (420, 220, 460, 260)) > 1000,
        "positive control"
    );
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
}

/// **A clipper that is the containing block still clips.** Same shape, but the
/// clipper is `position: relative`: the absolute box is inside its clip, and
/// what the clip cuts away is not damage. The negative half of the fixture
/// above — the escape is decided by the containing block, not by `absolute`
/// alone.
///
/// Kills: an absolute box treated as escaping every clipper (the move names
/// its full rects, 80 x 48 more pixels than the clip lets through).
#[test]
fn an_absolute_box_inside_its_clipping_containing_block_is_clipped() {
    let (mut app, _c, b) = mount(
        "",
        "",
        "overflow: hidden; position: relative",
        "position: absolute; left: 180px; top: 80px",
    );
    let before = full_frame(&mut app);
    // 20px of the 40px box is visible each way at the clipper's corner.
    assert!(
        ink_in(&before, (200, 100, 220, 120)) > 300,
        "positive control"
    );

    b.set_style("left", "170px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
    // Unclipped, the damage is two 48x48 rects reaching 24px past the clip on
    // two sides; clipped, it stops at the clipper's box (220, 120).
    let px = stats.get(Counter::RepaintedPx);
    assert!(
        px <= 50 * 40 + 50,
        "repainted {px} px: the clip was not applied"
    );
}

/// **A fixed box escapes every clipper.**
///
/// Kills: the chain walk not stopping at `position: fixed`.
#[test]
fn a_fixed_box_inside_a_clipper_is_damaged_outside_it() {
    let (mut app, _c, b) = mount(
        "",
        "",
        "overflow: hidden",
        "position: fixed; left: 300px; top: 200px",
    );
    let before = full_frame(&mut app);
    assert!(
        ink_in(&before, (300, 200, 340, 240)) > 1000,
        "positive control"
    );

    b.set_style("top", "300px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert!(
        ink_in(&full, (300, 300, 340, 340)) > 1000,
        "positive control"
    );
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
}

/// **A clipper that starts clipping in the frame its child moves.** The box
/// overflowed the (then unclipped) clipper and was drawn below it; now the
/// clipper clips and the box moves. Its old pixels were drawn unclipped, so
/// they are cleared unclipped: the painted frame reads what the clipper was
/// painted with, not what it is now.
///
/// Kills: the painted frame reading a dirty ancestor's current style (the old
/// rect is clipped by a clip that was not there when it was drawn: it ghosts).
#[test]
fn a_clipper_that_starts_clipping_still_clears_what_it_did_not_clip() {
    // A flex column, so the box's margin does not collapse through the
    // (unclipped) clipper.
    let (mut app, clipper, b) = mount(
        "",
        "",
        "display: flex; flex-direction: column",
        "margin-top: 150px; flex-shrink: 0",
    );
    let before = full_frame(&mut app);
    let old = (20, 170, 60, 210);
    assert!(
        ink_in(&before, old) > 1000,
        "positive control: drawn below it"
    );

    clipper.set_style("overflow", "hidden");
    b.set_style("margin-top", "140px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert_eq!(ink_in(&full, old), 0, "positive control: clipped away now");
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
}

/// **A whole-document restyle would leave painted states stale.** A stylesheet
/// appended after the first paint takes the clipper's `overflow: hidden` away;
/// that restyle pushes no node, so unless the paint consuming it re-reads
/// every painted state, the clipper's would still say it clipped when the
/// full repaint drew the box unclipped below it. Later the clipper changes
/// and the box moves: the old pixels must be cleared where the full repaint
/// put them.
///
/// Kills: `consume_paint_dirty` not refreshing every painted state on a
/// whole-document restyle (the stale `clips` clips the old rect away: the box
/// ghosts below the clipper).
#[test]
fn a_painted_state_older_than_a_whole_document_restyle_is_not_trusted() {
    let (mut app, clipper, b) = mount(
        ".clipper { overflow: hidden; }",
        "",
        "display: flex; flex-direction: column",
        "margin-top: 150px; flex-shrink: 0",
    );
    let first = full_frame(&mut app);
    assert_eq!(ink_in(&first, (20, 170, 60, 210)), 0, "clipped at first");

    let doc = app.doc.as_ref().unwrap().clone();
    {
        use rinch_core::dom::DomDocument;
        let mut d = doc.borrow_mut();
        let body = d.body();
        let s = d.create_element("style");
        let t = d.create_text(".clipper { overflow: visible; }");
        d.append_child(s, t);
        d.append_child(body, s);
    }
    resolve(&mut app);
    assert!(
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .whole_document_damaged,
        "positive control: that was a whole-document restyle"
    );
    let restyled = full_frame(&mut app);
    let old = (20, 170, 60, 210);
    assert!(
        ink_in(&restyled, old) > 1000,
        "positive control: unclipped now"
    );

    clipper.set_style("background", "rgb(250, 250, 250)");
    b.set_style("margin-top", "250px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert_eq!(ink_in(&full, old), 0, "positive control: the box left");
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
}

/// **Through a transform.** The clipper is moved 300px right by a transform;
/// its clip is in screen space there.
///
/// Kills: the clip rect taken untransformed (the damage is clipped to where
/// the clipper would be without its transform, 20..220, and the box, which
/// paints at 320..360, ghosts).
#[test]
fn a_transformed_clipper_clips_damage_in_screen_space() {
    let (mut app, _c, b) = mount(
        "",
        "",
        "overflow: hidden; transform: translateX(300px)",
        "margin-top: 10px",
    );
    let before = full_frame(&mut app);
    let old = (320, 30, 360, 70);
    assert!(ink_in(&before, old) > 1000, "positive control");

    b.set_style("margin-top", "50px");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
}

/// **A removal is clipped as it was painted.** An absolute box escaping the
/// clipper (containing block `outer`) is removed: its pixels outside the
/// clipper are cleared.
///
/// Kills: the removal's clip taking every clipper regardless of the escape.
#[test]
fn a_removed_box_that_escaped_the_clip_is_cleared() {
    let (mut app, _c, b) = mount(
        "",
        "position: relative",
        "overflow: hidden",
        "position: absolute; left: 300px; top: 200px",
    );
    let before = full_frame(&mut app);
    let old = (320, 220, 360, 260);
    assert!(ink_in(&before, old) > 1000, "positive control");

    b.remove();
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    let full = full_frame(&mut app);
    assert_eq!(ink_in(&inc, old), 0, "the removed box ghosts");
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
}

// ── Moves between parents (review of #958) ─────────────────────────────────

fn el(scope: &mut RenderScope, parent: &NodeHandle, style: &str) -> NodeHandle {
    let e = scope.create_element("div");
    e.set_attribute("style", style);
    parent.append_child(&e);
    e
}

type Build = Box<dyn Fn(&mut RenderScope, &NodeHandle) -> Vec<NodeHandle>>;

/// A 600x400 root built by `build`, mounted, laid out and painted in full.
fn mount_with(build: Build) -> (RinchApp, Vec<NodeHandle>) {
    let slot: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(vec![]));
    let s2 = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px;");
        *s2.borrow_mut() = build(scope, &outer);
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    resolve(&mut app);
    let _ = full_frame(&mut app);
    let v = slot.borrow().clone();
    (app, v)
}

/// The next frame is incremental and equals a from-scratch frame, and `old`
/// — where the moved row was — holds no ink.
fn assert_cleared(app: &mut RinchApp, old: (i32, i32, i32, i32)) {
    let (inc, stats) = incremental_frame(app);
    assert_incremental(&stats);
    let full = full_frame(app);
    assert_eq!(ink_in(&full, old), 0, "positive control: nothing there now");
    assert_eq!(ink_in(&inc, old), 0, "the moved row ghosts where it was");
    assert_eq!(
        diff_in(&inc, &full, WHOLE),
        0,
        "incremental frame != full frame"
    );
}

const ROW40: &str = "width: 40px; height: 40px; background: rgb(0, 0, 200); flex-shrink: 0;";

/// **A row moved INTO a clipper.** Its old pixels were drawn below the
/// clipper, under the old parent's chain. Placed and clipped along the *new*
/// parent's chain, its old rect is cut away by the new parent's clip and the
/// row ghosts at (0, 100). A move between parents is a removal from the old
/// one: its old rect is recorded under the chain it was painted under.
///
/// Kills: the move verbs detaching without recording the old rect.
#[test]
fn a_row_moved_into_a_clipper_is_cleared_where_it_was() {
    let (mut app, h) = mount_with(Box::new(|s, o| {
        let c = el(s, o, "width: 200px; height: 100px; overflow: hidden;");
        let r = el(s, o, ROW40);
        vec![c, r]
    }));
    h[0].append_child(&h[1]);
    resolve(&mut app);
    assert_cleared(&mut app, (0, 100, 40, 140));
}

/// The same into a scroller whose content already fills it, so the row lands
/// below the scroller's viewport, entirely clipped.
#[test]
fn a_row_moved_into_a_full_scroller_is_cleared_where_it_was() {
    let (mut app, h) = mount_with(Box::new(|s, o| {
        let c = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: auto; display: flex; flex-direction: column;",
        );
        el(
            s,
            &c,
            "width: 100px; height: 100px; background: rgb(200, 0, 0); flex-shrink: 0;",
        );
        let r = el(s, o, ROW40);
        vec![c, r]
    }));
    h[0].append_child(&h[1]);
    resolve(&mut app);
    assert_cleared(&mut app, (0, 100, 40, 140));
}

/// **A row moved between two clippers.** Its old rect used to be placed along
/// the new parent's chain (a wrong position, before #909 too): the pixels
/// under the first clipper stayed.
#[test]
fn a_row_moved_between_clippers_is_cleared_where_it_was() {
    let (mut app, h) = mount_with(Box::new(|s, o| {
        let a = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: hidden; display: flex; flex-direction: column;",
        );
        el(s, &a, "height: 70px; flex-shrink: 0;");
        let r = el(s, &a, ROW40);
        let b = el(
            s,
            o,
            "width: 200px; height: 100px; overflow: hidden; margin-left: 300px;",
        );
        vec![a, r, b]
    }));
    h[2].append_child(&h[1]);
    resolve(&mut app);
    assert_cleared(&mut app, (0, 70, 40, 100));
}

const CLIPPER: &str = "width: 200px; height: 100px; overflow: hidden;";

/// The same move by `insert_before` (review of #958, q10).
///
/// Kills: `insert_before` not recording the moved row's old pixels.
#[test]
fn a_row_inserted_before_into_a_clipper_is_cleared_where_it_was() {
    let (mut app, h) = mount_with(Box::new(|s, o| {
        let c = el(s, o, CLIPPER);
        let first = el(
            s,
            &c,
            "width: 20px; height: 20px; background: rgb(0, 200, 0);",
        );
        let r = el(s, o, ROW40);
        vec![c, first, r]
    }));
    assert!(
        ink_in(&full_frame(&mut app), (0, 100, 40, 140)) > 1000,
        "positive control"
    );
    h[0].insert_before(&h[2], &h[1]);
    resolve(&mut app);
    assert_cleared(&mut app, (0, 100, 40, 140));
}

/// The same move by `DomDocument::insert_child` (review of #958, q13).
///
/// Kills: `insert_child` not recording the moved row's old pixels.
#[test]
fn a_row_inserted_as_child_into_a_clipper_is_cleared_where_it_was() {
    let (mut app, h) = mount_with(Box::new(|s, o| {
        let c = el(s, o, CLIPPER);
        el(
            s,
            &c,
            "width: 20px; height: 20px; background: rgb(0, 200, 0);",
        );
        let r = el(s, o, ROW40);
        vec![c, r]
    }));
    assert!(
        ink_in(&full_frame(&mut app), (0, 100, 40, 140)) > 1000,
        "positive control"
    );
    {
        use rinch_core::dom::DomDocument;
        let doc = app.doc.as_ref().unwrap().clone();
        doc.borrow_mut()
            .insert_child(h[0].node_id(), h[1].node_id(), 0);
    }
    resolve(&mut app);
    assert_cleared(&mut app, (0, 100, 40, 140));
}

/// A row from outside a clipper `replace_with`s a node inside it (review of
/// #958, q4): the incoming node's old pixels are recorded under its old chain.
///
/// Kills: `replace_node` not recording its incoming node's old pixels.
#[test]
fn a_row_replacing_a_node_inside_a_clipper_is_cleared_where_it_was() {
    let (mut app, h) = mount_with(Box::new(|s, o| {
        let c = el(s, o, CLIPPER);
        let old = el(
            s,
            &c,
            "width: 40px; height: 40px; background: rgb(200, 0, 0);",
        );
        let r = el(s, o, ROW40);
        vec![c, old, r]
    }));
    assert!(
        ink_in(&full_frame(&mut app), (0, 100, 40, 140)) > 1000,
        "positive control"
    );
    h[1].replace_with(&h[2]);
    resolve(&mut app);
    assert_cleared(&mut app, (0, 100, 40, 140));
}

/// #994: a `display: contents; position: relative` wrapper is no containing
/// block, so an absolute under it escapes the static `overflow: hidden` box
/// above it — in paint (`Collector::span`) **and** in its damage
/// (`clip_chain_bounds`). The damage walk used to spell its own rule
/// (`position != Static || transformed`) and read the wrapper as the
/// containing block, clipping the moved box's damage to nothing
/// (`repaint_none`): the old box ghosted and the new one was never drawn.
///
/// Only the `position` wrapper: a `transform` on a contents element still
/// makes it a stacking context in rinch, which traps the absolute in paint
/// (a separate, pre-existing issue).
#[test]
fn a_moved_absolute_under_a_positioned_contents_wrapper_is_repainted() {
    for wrapper_style in ["display: contents; position: relative"] {
        type Slot = Option<NodeHandle>;
        let slot: Rc<RefCell<Slot>> = Rc::new(RefCell::new(None));
        let slot_in = slot.clone();
        let ws = wrapper_style.to_string();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let outer = scope.create_element("div");
            outer.set_attribute("style", "width: 600px; height: 400px");
            let clipper = scope.create_element("div");
            clipper.set_attribute(
                "style",
                "margin-left: 20px; margin-top: 20px; width: 200px; height: 100px; overflow: hidden",
            );
            outer.append_child(&clipper);
            let w = scope.create_element("div");
            w.set_attribute("style", &ws);
            clipper.append_child(&w);
            let b = scope.create_element("div");
            b.set_attribute(
                "style",
                "width: 40px; height: 40px; background: rgb(0, 0, 200); position: absolute; left: 300px; top: 200px",
            );
            w.append_child(&b);
            *slot_in.borrow_mut() = Some(b);
            outer
        });
        app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        let b = slot.borrow().clone().unwrap();
        let before = full_frame(&mut app);
        assert!(
            ink_in(&before, (300, 200, 340, 240)) > 1000,
            "{wrapper_style}: positive control — drawn at the ICB, outside the clip"
        );
        b.set_style("left", "400px");
        resolve(&mut app);
        let (inc, stats) = incremental_frame(&mut app);
        assert_incremental(&stats);
        let full = full_frame(&mut app);
        assert!(
            ink_in(&full, (400, 200, 440, 240)) > 1000,
            "positive control"
        );
        assert_eq!(
            diff_in(&inc, &full, WHOLE),
            0,
            "{wrapper_style}: incremental frame != full frame (ghost / missing box)"
        );
    }
}

/// #994: a warm hit memo follows a `display` flip of a positioned wrapper
/// between `block` (the absolute's containing block, below the clipper) and
/// `contents` (no containing block: the absolute escapes the clipper).
#[test]
fn the_hit_test_follows_a_contents_flip_of_a_positioned_wrapper() {
    type Slot = Option<(NodeHandle, NodeHandle)>;
    let slot: Rc<RefCell<Slot>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let clipper = scope.create_element("div");
        clipper.set_attribute(
            "style",
            "margin-left: 20px; margin-top: 20px; width: 200px; height: 100px; overflow: hidden",
        );
        outer.append_child(&clipper);
        let w = scope.create_element("div");
        w.set_attribute("style", "position: relative; height: 60px");
        clipper.append_child(&w);
        let b = scope.create_element("div");
        b.set_attribute(
            "style",
            "width: 300px; height: 250px; background: rgb(0, 0, 200); position: absolute; left: 10px; top: 20px",
        );
        w.append_child(&b);
        *slot_in.borrow_mut() = Some((w, b));
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    resolve(&mut app);
    let (w, b) = slot.borrow().clone().unwrap();
    let abs = b.node_id().0;
    assert_eq!(
        app.move_hit(50.0, 60.0),
        Some(abs),
        "positive control inside the clip"
    );
    assert_ne!(
        app.move_hit(250.0, 200.0),
        Some(abs),
        "block wrapper: clipped"
    );
    w.set_style("display", "contents");
    resolve(&mut app);
    assert_eq!(
        app.move_hit(250.0, 200.0),
        Some(abs),
        "contents wrapper: escapes"
    );
    w.set_style("display", "block");
    resolve(&mut app);
    assert_ne!(
        app.move_hit(250.0, 200.0),
        Some(abs),
        "block again: clipped"
    );
}
