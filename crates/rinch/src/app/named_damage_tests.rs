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

// ── Nodes that name no rect of their own (#886 review) ─────────────────────

/// A small harness for fixtures that need their own page, size or scale.
struct Page {
    app: RinchApp,
    nodes: Rc<RefCell<Vec<NodeHandle>>>,
    size: (u32, u32),
    scale: f64,
}

type Build = Box<dyn Fn(&mut RenderScope, &RefCell<Vec<NodeHandle>>) -> NodeHandle>;

impl Page {
    fn mount(size: (u32, u32), scale: f64, build: Build) -> Self {
        let nodes: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
        let out = nodes.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| build(scope, &out));
        app.mount_component(size.0 as f32, size.1 as f32);
        let mut page = Self {
            app,
            nodes,
            size,
            scale,
        };
        for _ in 0..3 {
            page.frame();
        }
        page
    }

    fn node(&self, i: usize) -> NodeHandle {
        self.nodes.borrow()[i].clone()
    }

    fn frame(&mut self) -> (Vec<u8>, FrameStats) {
        self.app
            .resolve_and_repaint(self.size.0 as f32, self.size.1 as f32);
        let px = self
            .app
            .build_pixels(self.scale, self.size, false)
            .0
            .to_vec();
        (px, self.app.end_perf_frame().expect("mounted"))
    }

    fn full(&mut self) -> Vec<u8> {
        self.app.scene_dirty = true;
        self.app.has_previous_frame = false;
        let px = self
            .app
            .build_pixels(self.scale, self.size, false)
            .0
            .to_vec();
        let _ = self.app.end_perf_frame();
        px
    }

    /// Pixels that differ between `a` and `b` by more than `tolerance` in
    /// some channel.
    fn stale(a: &[u8], b: &[u8], tolerance: i32) -> usize {
        a.chunks(4)
            .zip(b.chunks(4))
            .filter(|(p, q)| (0..4).any(|k| (p[k] as i32 - q[k] as i32).abs() > tolerance))
            .count()
    }

    /// Change the page with `change`, paint the next frame, and assert it holds
    /// what a from-scratch frame holds — with a positive control that the
    /// change reached pixels at all.
    fn assert_repaints(&mut self, what: &str, change: impl FnOnce(&Self)) -> FrameStats {
        let (before, _) = self.frame();
        change(self);
        let (after, stats) = self.frame();
        let full = self.full();
        assert!(
            Self::stale(&before, &full, 2) > 0,
            "{what}: positive control, the change reaches pixels"
        );
        assert_eq!(
            Self::stale(&after, &full, 2),
            0,
            "{what}: the incremental frame is stale: {stats:?}"
        );
        stats
    }
}

fn el(scope: &mut RenderScope, tag: &str, style: &str) -> NodeHandle {
    let n = scope.create_element(tag);
    if !style.is_empty() {
        n.set_attribute("style", style);
    }
    n
}

/// A native `<select>` built from options, as a controlled select renders it.
fn select_page() -> Page {
    Page::mount(
        (300, 200),
        1.0,
        Box::new(|scope, out| {
            let root = el(scope, "div", "padding: 20px");
            let select = el(scope, "select", "width: 150px; font-size: 16px");
            for (i, label) in ["Alpha", "WWWWWW"].iter().enumerate() {
                let option = el(scope, "option", "");
                option.set_attribute("value", &i.to_string());
                let text = scope.create_text(label);
                option.append_child(&text);
                select.append_child(&option);
                out.borrow_mut().push(option);
                out.borrow_mut().push(text);
            }
            root.append_child(&select);
            root
        }),
    )
}

/// Selecting another option changes what the closed `<select>` shows. The
/// option has no box (`display: none`) and the select paints it, so the
/// select is the damage — it used to name nothing and paint nothing.
#[test]
fn selecting_an_option_repaints_its_select() {
    let mut page = select_page();
    let stats = page.assert_repaints("option selected", |p| {
        p.node(2).set_attribute("selected", "");
    });
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
}

/// Renaming the selected option changes the label the select shows.
#[test]
fn renaming_the_selected_option_repaints_its_select() {
    let mut page = select_page();
    let stats = page.assert_repaints("option text", |p| {
        p.node(1).set_text("MMMMMMM");
    });
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
}

/// A zero-size positioned anchor (a tooltip or popover anchor) moves, and its
/// overflowing child moves with it without being dirty itself. Its old pixels
/// are the anchor's subtree where it was last painted.
#[test]
fn a_zero_size_anchor_that_moves_takes_its_child_along() {
    for anchor in [
        "position: absolute; left: 20px; top: 20px; width: 0; height: 0",
        "position: relative; left: 20px; top: 20px; height: 0",
    ] {
        let style = anchor.to_string();
        let mut page = Page::mount(
            (300, 200),
            1.0,
            Box::new(move |scope, out| {
                let root = el(
                    scope,
                    "div",
                    "position: relative; width: 300px; height: 200px",
                );
                let anchor = el(scope, "div", &style);
                let child = el(
                    scope,
                    "div",
                    "position: absolute; left: 0; top: 0; width: 40px; height: 40px; \
                     background: rgb(255, 0, 0)",
                );
                anchor.append_child(&child);
                root.append_child(&anchor);
                out.borrow_mut().push(anchor);
                root
            }),
        );
        let stats = page.assert_repaints(anchor, |p| {
            p.node(0).set_style("left", "150px");
        });
        assert_eq!(stats.get(Counter::RepaintPartial), 1, "{anchor}: {stats:?}");
    }
}

/// A `display: contents` wrapper whose class change restyles its child only
/// through a descendant selector. The child is not pushed; the wrapper has no
/// box, so its subtree's painted bounds are the damage.
#[test]
fn a_contents_wrapper_restyling_its_child_repaints_the_child() {
    let mut page = Page::mount(
        (300, 200),
        1.0,
        Box::new(|scope, out| {
            let root = el(scope, "div", "padding: 10px");
            let style = scope.create_element("style");
            let css = scope.create_text(
                ".w.on .c { background: rgb(255, 0, 0); } \
                 .c { width: 60px; height: 30px; background: rgb(0, 0, 255); }",
            );
            style.append_child(&css);
            root.append_child(&style);
            let wrapper = el(scope, "div", "display: contents");
            wrapper.set_attribute("class", "w");
            let child = scope.create_element("div");
            child.set_attribute("class", "c");
            wrapper.append_child(&child);
            root.append_child(&wrapper);
            out.borrow_mut().push(wrapper);
            root
        }),
    );
    let stats = page.assert_repaints("contents wrapper", |p| {
        p.node(0).set_attribute("class", "w on");
    });
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
    // The child's 60x30 box plus the 4px margin: the wrapper's own subtree,
    // not the whole row its nearest boxed ancestor spans (the fallback for a
    // node that names nothing, which would also be correct but 4x larger).
    assert_eq!(stats.get(Counter::RepaintedPx), 68 * 38, "{stats:?}");
}

/// At a fractional scale, fractional boxes land on fractional device pixels.
/// Each damage rect is snapped out to whole pixels, so the rect cleared and the
/// rect clipped to agree; unsnapped, a cleared-but-not-repainted sliver shows.
#[test]
fn damage_rects_snap_to_whole_pixels_at_fractional_scales() {
    for scale in [1.25, 1.5] {
        let mut page = Page::mount(
            (400, 300),
            scale,
            Box::new(|scope, out| {
                let root = el(
                    scope,
                    "div",
                    "position: relative; width: 400px; height: 300px; \
                     background: rgb(240, 240, 250)",
                );
                let layer = el(
                    scope,
                    "div",
                    "position: absolute; left: 13.3px; top: 17.7px; width: 350.4px; \
                     height: 250.2px; border-radius: 37px; opacity: 0.6; \
                     background: rgb(200, 40, 40); border: 3px solid rgb(0, 0, 0)",
                );
                root.append_child(&layer);
                for i in 0..6 {
                    let x = 7.3 + i as f64 * 61.9;
                    let y = 11.1 + ((i * 53) % 260) as f64;
                    let dot = el(
                        scope,
                        "div",
                        &format!(
                            "position: absolute; left: {x}px; top: {y}px; width: 9.4px; \
                             height: 9.4px; border-radius: 50%; background: rgb(0, 0, 200)"
                        ),
                    );
                    root.append_child(&dot);
                    out.borrow_mut().push(dot);
                }
                root
            }),
        );
        let stats = page.assert_repaints(&format!("scale {scale}"), |p| {
            for i in 0..2 {
                p.node(i).set_style("background", "rgb(0, 180, 0)");
            }
        });
        assert_eq!(stats.get(Counter::RepaintPartial), 1, "{scale}: {stats:?}");
        assert_eq!(stats.get(Counter::DamageRects), 2, "{scale}: {stats:?}");
    }
}

// ── Review round 2 ─────────────────────────────────────────────────────────

/// A span that becomes `display: none` takes its text off the line. It has no
/// box of its own and, once hidden, is no IFC member, so it names no rect; its
/// paragraph's text layout is rebuilt, and the paragraph is the damage. It
/// used to paint nothing and leave the span's text on screen.
#[test]
fn a_span_set_to_display_none_leaves_its_line() {
    let mut page = Page::mount(
        (300, 200),
        1.0,
        Box::new(|scope, out| {
            let root = el(
                scope,
                "div",
                "position: relative; width: 300px; height: 200px",
            );
            let para = el(
                scope,
                "p",
                "position: absolute; left: 10px; top: 130px; margin: 0; font-size: 20px",
            );
            // The span alone on its line, and a span beside text that stays.
            let span = scope.create_element("span");
            let text = scope.create_text("Hidden soon");
            span.append_child(&text);
            para.append_child(&span);
            root.append_child(&para);
            let para2 = el(
                scope,
                "p",
                "position: absolute; left: 10px; top: 20px; margin: 0; font-size: 20px",
            );
            let keep = scope.create_text("Keep ");
            para2.append_child(&keep);
            let span2 = scope.create_element("span");
            let text2 = scope.create_text("this goes");
            span2.append_child(&text2);
            para2.append_child(&span2);
            root.append_child(&para2);
            out.borrow_mut().push(span);
            out.borrow_mut().push(span2);
            root
        }),
    );
    for i in 0..2 {
        let stats = page.assert_repaints(&format!("span {i} display none"), |p| {
            p.node(i).set_style("display", "none");
        });
        assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
        let stats = page.assert_repaints(&format!("span {i} shown again"), |p| {
            p.node(i).set_style("display", "inline");
        });
        assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
    }
}

/// A text edit in a flex item repaints the item, not the page, even with a
/// `<style>` element in the tree. The `<style>`'s CSS text used to be laid out
/// as a line of an IFC the `<style>` was mistaken for (its computed `display`
/// was the default, not the UA sheet's `none`), so every structural pass
/// flipped that text node's box and pushed it; the damage fallback then climbed
/// from it to the page root and made the edit a full repaint.
#[test]
fn a_text_edit_beside_a_style_element_repaints_a_small_region() {
    let mut page = Page::mount(
        (400, 300),
        1.0,
        Box::new(|scope, out| {
            let root = el(scope, "div", "padding: 10px");
            let style = scope.create_element("style");
            let css = scope.create_text(".q { background: rgb(0, 0, 255); }");
            style.append_child(&css);
            root.append_child(&style);
            let q = el(scope, "div", "width: 50px; height: 20px");
            q.set_attribute("class", "q");
            root.append_child(&q);
            let flex = el(scope, "div", "display: flex; font-size: 16px");
            let text = scope.create_text("flexy");
            flex.append_child(&text);
            root.append_child(&flex);
            out.borrow_mut().push(text);
            out.borrow_mut().push(style);
            root
        }),
    );
    {
        let d = page.app.doc.as_ref().unwrap().borrow();
        let style = d.tree.get(page.node(1).node_id().0).unwrap();
        assert_eq!(
            style.computed_style.display,
            rinch_dom::computed_style::DisplayValue::None,
            "a <style> element is not rendered"
        );
    }
    let stats = page.assert_repaints("flex text", |p| {
        p.node(0).set_text("WWWWWWW");
    });
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
    assert!(
        stats.get(Counter::RepaintedPx) <= 4000,
        "the flex item's line, not the page: {stats:?}"
    );
}

/// The fallback damages the owner where it was **painted** as well as where it
/// is: here the 40px `<select>` moves far down in the same frame its option
/// changes, because its 10px-tall holder takes a top margin, and the select itself is
/// not pushed (its parent-relative box did not change). The holder is, but
/// the select overflows it, and the page root has a fixed height so it is not
/// pushed either: only the owner's last-painted rect covers the select's old
/// lower part.
#[test]
fn an_option_change_while_its_select_moves_clears_the_old_select() {
    let mut page = Page::mount(
        (600, 400),
        1.0,
        Box::new(|scope, out| {
            let root = el(scope, "div", "padding: 10px; width: 200px; height: 380px");
            let holder = el(scope, "div", "height: 10px");
            let select = el(
                scope,
                "select",
                "width: 150px; height: 40px; font-size: 16px",
            );
            let option = el(scope, "option", "");
            let text = scope.create_text("Alpha");
            option.append_child(&text);
            select.append_child(&option);
            holder.append_child(&select);
            root.append_child(&holder);
            out.borrow_mut().push(holder);
            out.borrow_mut().push(text);
            root
        }),
    );
    let stats = page.assert_repaints("select moved and relabelled", |p| {
        p.node(0).set_style("margin-top", "250px");
        p.node(1).set_text("WWWWWWW");
    });
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
}
