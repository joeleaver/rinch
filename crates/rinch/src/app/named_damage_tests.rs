//! Every change the shell makes names its damage, and the damage is a short
//! list of rects rather than one bounding box.
//!
//! An empty dirty set used to mean "repaint everything": a window focus change,
//! a focus move, clearing a text selection, an IME preedit, a scroll into view,
//! the drag ghost and the inspect highlight each set the scene dirty without
//! naming a node, and the software frame fell back to a full repaint. Now a
//! frame repaints what its damage names — paint-dirty nodes, removed rects,
//! the overlays' old and new rects — and one that names nothing keeps its
//! pixels (`repaint_none`). A full repaint needs a reason
//! (`repaint_full_first_frame`, `_resize`, `_theme`, `_restyle`,
//! `_invalidated`, `_region_too_large`), and whatever still marks the scene
//! dirty without naming anything is counted as `repaint_full_unattributed`.
//!
//! Each converted site has a **local pixel oracle**: the incremental frame
//! must hold exactly what a from-scratch frame holds in the rect the change
//! touched, with a positive control that the rect did change. A counter half
//! asserts the frame really was partial and small, since an accidental full
//! repaint makes the pixel half pass vacuously.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::Cell;

const SIZE: (u32, u32) = (600, 400);
const SURFACE: u64 = 600 * 400;

const CSS: &str = "
    body { margin: 0; }
    .page { position: relative; width: 600px; height: 400px; }
    .abs { position: absolute; }
    .sel { user-select: text; left: 20px; top: 20px; width: 300px;
           font-size: 20px; line-height: 24px; color: rgb(0, 0, 0); }
    .f { left: 20px; top: 80px; width: 100px; height: 30px;
         background: rgb(220, 220, 220); }
    .f:focus { outline: 4px solid rgb(255, 0, 0); }
    .field { left: 20px; top: 140px; width: 200px; height: 30px;
             font-size: 16px; color: rgb(0, 0, 0); }
    .src { left: 400px; top: 20px; width: 60px; height: 30px;
           background: rgb(0, 0, 255); }
    .a { left: 10px; top: 300px; width: 20px; height: 20px;
         background: rgb(0, 0, 0); }
    .b { left: 560px; top: 360px; width: 20px; height: 20px;
         background: rgb(0, 0, 0); }
    .scroller { left: 400px; top: 150px; width: 150px; height: 100px;
                overflow-y: auto; }
    .row { height: 30px; }
    .odd { background: rgb(0, 160, 0); }
    .even { background: rgb(250, 200, 0); }
";

/// The fixture's nodes, by role.
#[derive(Clone, Copy, Default)]
struct Ids {
    sel: usize,
    focusable: usize,
    field: usize,
    src: usize,
    a: usize,
    b: usize,
    far_row: usize,
}

fn mount() -> (RinchApp, Ids) {
    let ids: Rc<Cell<Ids>> = Rc::new(Cell::new(Ids::default()));
    let out = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("class", "page");
        let style = scope.create_element("style");
        let css = scope.create_text(CSS);
        style.append_child(&css);
        root.append_child(&style);
        let el = |scope: &mut RenderScope, class: &str| {
            let n = scope.create_element("div");
            n.set_attribute("class", class);
            n
        };
        let mut got = Ids::default();

        let sel = scope.create_element("p");
        sel.set_attribute("class", "abs sel");
        sel.set_attribute("style", "user-select: text");
        let t = scope.create_text("Select this text please");
        sel.append_child(&t);
        root.append_child(&sel);
        got.sel = sel.node_id().0;

        let f = el(scope, "abs f");
        f.set_attribute("tabindex", "0");
        root.append_child(&f);
        got.focusable = f.node_id().0;

        let field = scope.create_element("input");
        field.set_attribute("class", "abs field");
        field.set_attribute("value", "hello");
        let hid = rinch_core::events::register_input_handler(
            rinch_core::events::InputCallback::new(|_v: String| {}),
        );
        field.set_attribute("data-oninput", &hid.0.to_string());
        root.append_child(&field);
        got.field = field.node_id().0;

        let src = el(scope, "abs src");
        src.set_attribute("draggable", "true");
        root.append_child(&src);
        got.src = src.node_id().0;

        let a = el(scope, "abs a");
        root.append_child(&a);
        got.a = a.node_id().0;
        let b = el(scope, "abs b");
        root.append_child(&b);
        got.b = b.node_id().0;

        let scroller = el(scope, "abs scroller");
        for i in 0..20 {
            let row = el(scope, if i % 2 == 0 { "row even" } else { "row odd" });
            scroller.append_child(&row);
            if i == 15 {
                got.far_row = row.node_id().0;
            }
        }
        root.append_child(&scroller);

        out.set(got);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let ids = ids.get();
    // Settle: the first frame is full, and the first Taffy pass after mount
    // reports every IFC's direct text children once.
    frame(&mut app);
    frame(&mut app);
    frame(&mut app);
    (app, ids)
}

/// One redraw, as `RinchRuntime::paint` performs it. Pixels and counters.
fn frame(app: &mut RinchApp) -> (Vec<u8>, FrameStats) {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    (px, app.end_perf_frame().expect("mounted"))
}

/// A from-scratch frame of the current state.
fn full_frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

/// Pixels of `rect` (x0, y0, x1, y1, physical px) that differ between `a`
/// and `b` by more than a rounding step.
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

fn whole() -> (i32, i32, i32, i32) {
    (0, 0, SIZE.0 as i32, SIZE.1 as i32)
}

fn box_of(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    hit_testing::painted_element_box(&d.tree, id)
}

fn center(app: &RinchApp, id: usize) -> (f32, f32) {
    let (x, y, w, h) = box_of(app, id);
    (x + w / 2.0, y + h / 2.0)
}

fn rect_around(app: &RinchApp, id: usize, grow: i32) -> (i32, i32, i32, i32) {
    let (x, y, w, h) = box_of(app, id);
    (
        x as i32 - grow,
        y as i32 - grow,
        (x + w) as i32 + grow,
        (y + h) as i32 + grow,
    )
}

fn press(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        SIZE,
        1.0,
    );
}

fn release(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        SIZE,
        1.0,
    );
}

fn click(app: &mut RinchApp, (x, y): (f32, f32)) {
    press(app, x, y);
    release(app, x, y);
}

fn pointer_move(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, SIZE, 1.0);
}

/// A point on the page that holds nothing.
const EMPTY: (f32, f32) = (300.0, 250.0);

fn assert_partial_and_small(stats: &FrameStats) {
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
    assert!(
        stats.get(Counter::RepaintedPx) < SURFACE / 10,
        "a small change repaints a small area: {stats:?}"
    );
}

// ── Focus ──────────────────────────────────────────────────────────────────

/// Clicking a focusable box paints its `:focus` outline, and clicking away
/// removes it: both frames repaint the box and its outline, nothing else, and
/// the blur leaves no ring behind.
#[test]
fn a_focus_ring_comes_and_goes_in_a_region() {
    let (mut app, ids) = mount();
    let ring = rect_around(&app, ids.focusable, 6);

    let (before, _) = frame(&mut app);
    let c = center(&app, ids.focusable);
    click(&mut app, c);
    let (focused, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert!(
        diff_in(&before, &focused, ring) > 0,
        "positive control: the focus paints an outline"
    );
    assert_eq!(diff_in(&focused, &full_frame(&mut app), whole()), 0);

    click(&mut app, EMPTY);
    let (blurred, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(
        diff_in(&blurred, &full_frame(&mut app), whole()),
        0,
        "the blurred ring is gone"
    );
    assert_eq!(diff_in(&blurred, &before, ring), 0);
}

/// Blurring a text field takes its caret with it. The caret is painted from
/// the field's `data-focused` attribute, which the runtime clears directly, so
/// the field has to be named as damage (`clear_input_focus_attrs`).
#[test]
fn a_blurred_field_leaves_no_caret() {
    let (mut app, ids) = mount();
    let field = rect_around(&app, ids.field, 2);
    let (before, _) = frame(&mut app);
    let c = center(&app, ids.field);
    click(&mut app, c);
    let (focused, _) = frame(&mut app);
    assert!(
        diff_in(&before, &focused, field) > 0,
        "positive control: focusing the field draws a caret"
    );

    click(&mut app, EMPTY);
    let (blurred, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(diff_in(&blurred, &full_frame(&mut app), whole()), 0);
    assert_eq!(diff_in(&blurred, &before, field), 0, "no caret left behind");
}

/// An IME composition is drawn from `data-preedit`, which the runtime writes
/// directly: the field is the damage, and the frame is partial.
#[test]
fn an_ime_preedit_repaints_its_field() {
    let (mut app, ids) = mount();
    let field = rect_around(&app, ids.field, 2);
    let c = center(&app, ids.field);
    click(&mut app, c);
    let (focused, _) = frame(&mut app);

    app.handle_event(PlatformEvent::Ime(ImeEvent::Enabled), SIZE, 1.0);
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "WWW".into(),
            cursor: None,
        }),
        SIZE,
        1.0,
    );
    let (composing, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert!(
        diff_in(&focused, &composing, field) > 0,
        "positive control: the composition is drawn"
    );
    assert_eq!(diff_in(&composing, &full_frame(&mut app), whole()), 0);

    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: String::new(),
            cursor: None,
        }),
        SIZE,
        1.0,
    );
    let (cleared, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(diff_in(&cleared, &full_frame(&mut app), whole()), 0);
}

/// Alt-tabbing away and back changes nothing on screen: nothing painted from
/// the window's focus state. It used to repaint the whole window, twice.
#[test]
fn a_window_focus_change_repaints_nothing() {
    let (mut app, _) = mount();
    for focused in [false, true] {
        app.handle_event(PlatformEvent::WindowFocus(focused), SIZE, 1.0);
        assert!(
            app.scene_dirty,
            "the window focus change still asks for a frame"
        );
        let (_, stats) = frame(&mut app);
        assert_eq!(stats.get(Counter::RepaintNone), 1, "{stats:?}");
        assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
        assert_eq!(stats.get(Counter::RepaintedPx), 0, "{stats:?}");
        assert_eq!(stats.get(Counter::PaintNodesVisited), 0, "{stats:?}");
    }
}

// ── Read-only text selection ───────────────────────────────────────────────

/// Dragging across `user-select: text` paints a highlight, and a click
/// elsewhere clears it. The highlight is painted from attributes the runtime
/// writes straight onto the paragraph, so the paragraph is the damage both
/// ways.
#[test]
fn a_text_selection_is_drawn_and_cleared_in_a_region() {
    let (mut app, ids) = mount();
    let para = rect_around(&app, ids.sel, 2);
    let (x, y, w, h) = box_of(&app, ids.sel);
    let (before, _) = frame(&mut app);

    press(&mut app, x + 2.0, y + h / 2.0);
    pointer_move(&mut app, x + w * 0.6, y + h / 2.0);
    release(&mut app, x + w * 0.6, y + h / 2.0);
    let (selected, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert!(
        diff_in(&before, &selected, para) > 0,
        "positive control: the selection is highlighted"
    );
    assert_eq!(diff_in(&selected, &full_frame(&mut app), whole()), 0);

    click(&mut app, EMPTY);
    let (cleared, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(diff_in(&cleared, &full_frame(&mut app), whole()), 0);
    assert_eq!(
        diff_in(&cleared, &before, para),
        0,
        "no highlight left behind"
    );
}

// ── Scroll into view ───────────────────────────────────────────────────────

/// A scroll into view moves every box in the container, so the container is
/// the damage. It used to name nothing and rely on an empty dirty set meaning
/// "everything"; with anything else dirty in the same frame the scroller kept
/// its old content on screen.
#[test]
fn a_scroll_into_view_repaints_the_scroller() {
    let (mut app, ids) = mount();
    let scroller = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.get(ids.far_row).and_then(|n| n.parent).unwrap()
    };
    let area = rect_around(&app, scroller, 0);
    let (before, _) = frame(&mut app);
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.request_scroll_into_view(rinch_core::dom::NodeId(ids.far_row));
        // Something unrelated changes in the same frame, far away.
        d.set_style(
            rinch_core::dom::NodeId(ids.a),
            "background",
            "rgb(255, 0, 0)",
        );
    }
    let (scrolled, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert!(
        diff_in(&before, &scrolled, area) > 0,
        "positive control: the scroller moved"
    );
    assert_eq!(diff_in(&scrolled, &full_frame(&mut app), whole()), 0);
}

// ── Overlays ───────────────────────────────────────────────────────────────

/// The drag ghost is blitted over the document after paint. While a drag is
/// up the frame repaints the ghost's old rect and its new one, not the window
/// — and leaves no trail.
#[test]
fn a_moving_drag_ghost_repaints_its_old_and_new_rects() {
    let (mut app, ids) = mount();
    let (sx, sy) = center(&app, ids.src);
    press(&mut app, sx, sy);
    pointer_move(&mut app, sx - 20.0, sy + 20.0);
    assert!(app.active_dnd.is_some(), "the drag activated");
    let (first, _) = frame(&mut app);
    let old_ghost = app.last_ghost_rect.expect("the ghost was drawn");
    let old = (
        old_ghost.x0 as i32,
        old_ghost.y0 as i32,
        old_ghost.x1 as i32,
        old_ghost.y1 as i32,
    );

    pointer_move(&mut app, sx - 200.0, sy + 150.0);
    let (moved, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(
        stats.get(Counter::DamageRects),
        2,
        "old and new ghost: {stats:?}"
    );
    assert!(
        diff_in(&first, &moved, old) > 0,
        "positive control: the ghost was in its old rect"
    );
    assert_eq!(
        diff_in(&moved, &full_frame(&mut app), whole()),
        0,
        "no ghost trail"
    );

    // Dropping takes the ghost away, again in a region.
    release(&mut app, sx - 200.0, sy + 150.0);
    assert!(app.active_dnd.is_none());
    let (dropped, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(diff_in(&dropped, &full_frame(&mut app), whole()), 0);
}

/// The inspect highlight is painted over the document; moving it repaints
/// where it was and where it goes.
#[test]
fn a_moving_inspect_highlight_repaints_its_old_and_new_rects() {
    let (mut app, _) = mount();
    app.inspect_highlight = Some((50.0, 200.0, 60.0, 30.0));
    app.request_repaint();
    let (first, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    let old = (48, 198, 112, 232);

    app.inspect_highlight = Some((300.0, 20.0, 60.0, 30.0));
    app.request_repaint();
    let (moved, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(stats.get(Counter::DamageRects), 2, "{stats:?}");
    assert!(
        diff_in(&first, &moved, old) > 0,
        "positive control: the highlight was there"
    );
    assert_eq!(diff_in(&moved, &full_frame(&mut app), whole()), 0);

    app.inspect_highlight = None;
    app.request_repaint();
    let (gone, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(diff_in(&gone, &full_frame(&mut app), whole()), 0);
}

// ── Multi-rect damage ──────────────────────────────────────────────────────

/// Two small changes in opposite corners repaint their own areas, not the
/// span between them. One bounding rect covered 578x88 px here.
#[test]
fn two_distant_changes_repaint_their_sum_not_their_span() {
    let (mut app, ids) = mount();
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.set_style(
            rinch_core::dom::NodeId(ids.a),
            "background",
            "rgb(255, 0, 0)",
        );
        d.set_style(
            rinch_core::dom::NodeId(ids.b),
            "background",
            "rgb(255, 0, 0)",
        );
    }
    let (px, stats) = frame(&mut app);
    assert_partial_and_small(&stats);
    assert_eq!(stats.get(Counter::DamageRects), 2, "{stats:?}");
    // Each 20x20 box plus the 4px anti-aliasing margin on every side.
    assert_eq!(stats.get(Counter::RepaintedPx), 2 * 28 * 28, "{stats:?}");
    assert_eq!(diff_in(&px, &full_frame(&mut app), whole()), 0);
}

// ── What still repaints in full, and why ───────────────────────────────────

/// A stylesheet added after mount restyles the whole document without naming
/// a node: that is a full repaint, counted as `repaint_full_restyle`, and the
/// frame shows the new rule everywhere.
#[test]
fn a_stylesheet_append_repaints_in_full() {
    let (mut app, ids) = mount();
    let a = rect_around(&app, ids.a, 0);
    let (before, _) = frame(&mut app);
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        let style = d.create_element("style");
        let css = d.create_text(".a { background: rgb(0, 200, 0) !important; }");
        d.append_child(style, css);
        let body = d.body();
        d.append_child(body, style);
    }
    let (after, stats) = frame(&mut app);
    assert_eq!(stats.get(Counter::RepaintFullRestyle), 1, "{stats:?}");
    assert!(diff_in(&before, &after, a) > 0, "the new rule is drawn");
    assert_eq!(diff_in(&after, &full_frame(&mut app), whole()), 0);
}

/// `mark_scene_dirty` says "something changed" without saying what: with
/// nothing else damaged, the frame repaints in full and says so.
/// `request_repaint` with nothing damaged paints nothing.
#[test]
fn unattributed_and_attributed_requests_are_told_apart() {
    let (mut app, _) = mount();
    app.mark_scene_dirty();
    let (_, stats) = frame(&mut app);
    assert_eq!(stats.get(Counter::RepaintFullUnattributed), 1, "{stats:?}");

    app.request_repaint();
    let (_, stats) = frame(&mut app);
    assert_eq!(stats.get(Counter::RepaintNone), 1, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
}
