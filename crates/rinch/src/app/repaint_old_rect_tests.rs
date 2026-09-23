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
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
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
        *slot_in.borrow_mut() = Some(b);
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let b = slot.borrow().clone().unwrap();
    (app, b)
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
/// layout, so `prev_layout` says nothing about it: the old painted rect has
/// to come from somewhere else, and today nothing supplies it.
///
/// **Known failing, and older than the fix this file pins** (paint audit F9):
/// `compute_dirty_region` adds only the node's *current* transformed rect,
/// so a transform that moves a box off its old pixels leaves them painted.
/// It is usually masked because the big sliding panels that animate
/// `transform` (a `Drawer`) cover half the window and repaint in full. The
/// cure is to record the painted (transformed) bbox wherever a transform
/// changes — the cascade, `tick_transitions`, `tick_animations` — or to keep
/// a per-node painted transform beside `prev_layout`. Un-ignore when it lands.
#[test]
#[ignore = "pre-existing: a transform change does not clear its old painted rect (audit F9)"]
fn a_transform_change_clears_the_old_painted_rect() {
    let (mut app, b) = panel("transform: translate(0px, 0px)");
    let before = full_frame(&mut app);
    let old = (20, 150, 60, 190);
    assert!(ink_in(&before, old) > 1000, "positive control");

    b.set_style("transform", "translate(280px, -130px)");
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    assert_incremental(&stats);
    assert_eq!(
        ink_in(&inc, old),
        0,
        "the box ghosts where it was painted before the transform"
    );
    let full = full_frame(&mut app);
    assert_eq!(
        diff_in(&inc, &full, (0, 0, 600, 400)),
        0,
        "incremental frame != full frame"
    );
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
