//! The shell half of `rinch_dom::perf`: what a painted frame records.
//!
//! `crates/rinch-dom/tests/perf_counter_baselines.rs` pins the style, text and
//! layout counters on a bare document; these pin the counters only the
//! desktop shell can fill — full-repaint reasons, the repainted area, hit
//! testing, the reactive runtime — through a real `RinchApp` on the software
//! painter, driving frames the way `RinchRuntime::paint` does:
//! `resolve_and_repaint`, `build_pixels`, `end_perf_frame`.
//!
//! Like the rinch-dom file, the numbers are exact: a fix that makes a path
//! cheaper updates them (that is its proof), and a counter that stops
//! counting fails instead of reading as a saving.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::Cell;

const SIZE: (u32, u32) = (800, 600);

const CSS: &str = "
    .row { height: 20px; }
    .row:hover { background-color: rgb(200, 0, 0); }
    .panel { position: relative; width: 400px; height: 300px; }
    .float { position: absolute; left: 10px; top: 10px; width: 50px; height: 50px;
             background: rgb(0, 0, 200); }
";

/// Ten rows with a `:hover` rule, and a positioned panel holding an absolute
/// box. Returns the app and the absolute box's id.
fn mount() -> (RinchApp, NodeHandle) {
    let float: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let float_out = float.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = scope.create_text(CSS);
        style.append_child(&css);
        root.append_child(&style);
        for i in 0..10 {
            let row = scope.create_element("div");
            row.set_attribute("class", "row");
            let t = scope.create_text(&format!("row {i}"));
            row.append_child(&t);
            root.append_child(&row);
        }
        let panel = scope.create_element("div");
        panel.set_attribute("class", "panel");
        let f = scope.create_element("div");
        f.set_attribute("class", "float");
        panel.append_child(&f);
        root.append_child(&panel);
        *float_out.borrow_mut() = Some(f);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let f = float.borrow().clone().expect("mounted");
    (app, f)
}

/// One redraw, as `RinchRuntime::paint` performs it.
fn frame(app: &mut RinchApp) -> FrameStats {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let _ = app.build_pixels(1.0, SIZE, false);
    app.end_perf_frame().expect("a mounted app has a document")
}

fn frame_at(app: &mut RinchApp, size: (u32, u32)) -> FrameStats {
    app.resize_layout(size.0, size.1);
    let _ = app.build_pixels(1.0, size, false);
    app.end_perf_frame().expect("a mounted app has a document")
}

#[test]
fn the_first_frame_is_a_full_repaint_for_that_reason() {
    let (mut app, _) = mount();
    let s = frame(&mut app);
    assert_eq!(s.get(Counter::PaintFrames), 1);
    assert_eq!(s.get(Counter::RepaintFull), 1);
    assert_eq!(s.get(Counter::RepaintFullFirstFrame), 1);
    assert_eq!(s.get(Counter::RepaintedPx), 800 * 600);
    assert_eq!(s.get(Counter::SurfacePx), 800 * 600);
    assert!(s.get(Counter::PaintNodesVisited) > 0);
    assert!(s.get(Counter::TimePaintNs) > 0);
}

#[test]
fn a_redraw_with_nothing_dirty_paints_nothing() {
    let (mut app, _) = mount();
    frame(&mut app);
    let _ = app.build_pixels(1.0, SIZE, false);
    let s = app.end_perf_frame().unwrap();
    assert_eq!(s.get(Counter::PaintFrames), 0);
    assert_eq!(s.get(Counter::PaintCachedFrames), 1);
    assert_eq!(s.get(Counter::PaintNodesVisited), 0);
}

/// A colour-only hover repaints a dirty region, not the window.
#[test]
fn a_hover_repaints_a_region_and_counts_its_hit_tests() {
    let (mut app, _) = mount();
    frame(&mut app);
    app.handle_event(PlatformEvent::MouseMove { x: 20.0, y: 30.0 }, SIZE, 1.0);
    let s = frame(&mut app);
    // Exact, like the rinch-dom baselines: a counter that stops counting must
    // fail here, not read as a saving.
    for (c, want) in [
        (Counter::RepaintPartial, 1),
        (Counter::RepaintFull, 0),
        (Counter::PaintFrames, 1),
        // One row's 20px box plus the 4px dirty margin on each side.
        (Counter::RepaintedPx, 800 * 28),
        (Counter::SurfacePx, 800 * 600),
        (Counter::TaffyRootComputes, 0),
        // Paint-only: no text input changed, so no text layout was dropped.
        (Counter::LayoutSkippedTextOnly, 0),
        (Counter::LayoutSkippedPaintOnly, 1),
        (Counter::ElementsCascaded, 1),
        // The row's text is not re-shaped: only its background changed. It
        // was 1 while every restyle dropped every text layout under the
        // restyled node (update-path audit F1.1).
        (Counter::ShapeIfcBuild, 0),
        // One hit test, shared by hover and cursor; the `data-onmousemove`
        // walk is skipped because no node carries the attribute (update-path
        // audit F2.1 — it used to be 2 tests and 26 visits).
        (Counter::HitTests, 1),
        // The body, the root div and the row under the pointer: every other
        // row's subtree extent misses the point.
        (Counter::HitTestNodesVisited, 3),
        // The layout before this move invalidated the hit cache, so the move
        // computes the extents it consults once (the body's in-flow children
        // and what is under them) and builds the body's sequence once; paint
        // builds its own: 1 + 1.
        (Counter::HitExtentsComputed, 14),
        (Counter::StackingOrderBuilds, 2),
        (Counter::PaintNodesVisited, 15),
    ] {
        assert_eq!(s.get(c), want, "{}: {s:?}", c.name());
    }
}

/// Moving an absolute box by its insets takes the inset fast path, which
/// used to throw the whole previous frame away (#280). It repaints a region
/// now: the box's old rect and its new one.
#[test]
fn the_inset_fast_path_repaints_a_region() {
    let (mut app, float) = mount();
    frame(&mut app);
    // A warm-up move. The first Taffy pass after mount also reports the
    // direct text children of every IFC root, whose `layout` the IFC pass
    // wrote and the Taffy read then zeroes — a one-off, whole-width region
    // that has nothing to do with the box (pre-existing).
    float.set_style("left", "20px");
    frame(&mut app);
    float.set_style("left", "40px");
    let s = frame(&mut app);
    assert_eq!(s.get(Counter::RepaintFull), 0, "{s:?}");
    assert_eq!(s.get(Counter::RepaintPartial), 1, "{s:?}");
    assert!(
        s.get(Counter::RepaintedPx) < s.get(Counter::SurfacePx) / 10,
        "{s:?}"
    );
}

#[test]
fn a_resize_is_a_full_repaint_for_that_reason() {
    let (mut app, _) = mount();
    frame(&mut app);
    let s = frame_at(&mut app, (801, 600));
    assert_eq!(s.get(Counter::RepaintFullResize), 1, "{s:?}");
    assert_eq!(s.get(Counter::FullRestyleViewport), 1, "{s:?}");
}

/// A previous frame thrown away by something that recorded no reason still
/// counts, as `repaint_full_invalidated`.
#[test]
fn an_unattributed_invalidation_is_still_counted() {
    let (mut app, _) = mount();
    frame(&mut app);
    app.has_previous_frame = false;
    app.mark_scene_dirty();
    let _ = app.build_pixels(1.0, SIZE, false);
    let s = app.end_perf_frame().unwrap();
    assert_eq!(s.get(Counter::RepaintFullInvalidated), 1, "{s:?}");
}

/// The reactive counters are folded in per frame, as deltas.
#[test]
fn effect_runs_are_folded_into_the_frame() {
    let (mut app, _) = mount();
    frame(&mut app);
    let sig = rinch_core::reactive::Signal::new(0);
    let _e = rinch_core::reactive::Effect::new(move || {
        let _ = sig.get();
    });
    let before = app.end_perf_frame().unwrap();
    assert!(
        before.get(Counter::EffectRuns) >= 1,
        "the effect's first run"
    );
    sig.set(1);
    sig.set(2);
    let s = app.end_perf_frame().unwrap();
    assert_eq!(s.get(Counter::EffectRuns), 2, "{s:?}");
    assert_eq!(s.get(Counter::SignalNotifies), 2, "{s:?}");
}

#[test]
fn last_frame_total_and_reset() {
    let (mut app, _) = mount();
    let first = frame(&mut app);
    assert_eq!(app.last_frame_perf(), first);
    frame(&mut app);
    assert!(app.total_perf().get(Counter::PaintFrames) >= 1);
    app.reset_perf();
    assert!(app.total_perf().is_empty());
    assert!(app.last_frame_perf().is_empty());
}

// ── Pointer moves (update-path audit F2.1 / F2.2) ──────────────────────────

const ROWS: usize = 500;

/// A 400px scroller holding `ROWS` 20px rows, each a text row with a nested
/// span, so an unpruned walk visits every row's whole subtree.
fn mount_long_list(with_mousemove: Option<Rc<Cell<u32>>>) -> (RinchApp, NodeHandle) {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let scroller = scope.create_element("div");
        scroller.set_attribute("style", "height: 400px; overflow-y: auto");
        if let Some(count) = with_mousemove.clone() {
            let id = scope.register_handler(move || count.set(count.get() + 1));
            scroller.set_attribute("data-onmousemove", &id.0.to_string());
        }
        for i in 0..ROWS {
            let row = scope.create_element("div");
            row.set_attribute("style", "height: 20px");
            let span = scope.create_element("span");
            let t = scope.create_text(&format!("row {i}"));
            span.append_child(&t);
            row.append_child(&span);
            scroller.append_child(&row);
        }
        root.append_child(&scroller);
        *out2.borrow_mut() = Some(scroller.clone());
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let scroller = out.borrow().clone().expect("mounted");
    (app, scroller)
}

fn pointer_move(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, SIZE, 1.0);
}

/// A move over a 500-row scroller visits what is under the pointer, not the
/// list: every other row is skipped on its memoised subtree extent, and the
/// body's stacking sequence is reused from the move before. On `main` before
/// this change the same warm move ran two hit tests that each visited every
/// row and its text, and built the stacking sequence twice.
#[test]
fn a_move_over_a_long_list_visits_only_what_is_under_it() {
    let (mut app, scroller) = mount_long_list(None);
    frame(&mut app);
    // Scrolled to the middle, so the rows under the pointer are not the first.
    scroller.set_scroll_top(4000.0);
    frame(&mut app);

    // The first move after a layout pays for the memo, once.
    pointer_move(&mut app, 50.0, 105.0);
    let cold = app.end_perf_frame().unwrap();
    assert_eq!(cold.get(Counter::HitTests), 1, "{cold:?}");
    assert_eq!(cold.get(Counter::StackingOrderBuilds), 1, "{cold:?}");
    assert!(
        cold.get(Counter::HitExtentsComputed) >= ROWS as u64,
        "the cold move computes each row's extent: {cold:?}"
    );

    // Every move after it, over a still document, is O(depth).
    pointer_move(&mut app, 60.0, 107.0);
    let warm = app.end_perf_frame().unwrap();
    for (c, want) in [
        (Counter::HitTests, 1),
        // body → root div → scroller → the one row under the pointer (its
        // span and text are laid out by the row's IFC, which answers for them).
        (Counter::HitTestNodesVisited, 4),
        (Counter::HitExtentsComputed, 0),
        (Counter::StackingOrderBuilds, 0),
        (Counter::LayoutResolves, 0),
    ] {
        assert_eq!(warm.get(c), want, "{}: {warm:?}", c.name());
    }
}

/// With no `data-onmousemove` anywhere, a move runs one hit test (hover's),
/// not two.
#[test]
fn a_move_with_no_mousemove_handler_hit_tests_once() {
    let (mut app, _) = mount_long_list(None);
    frame(&mut app);
    pointer_move(&mut app, 50.0, 105.0);
    let s = app.end_perf_frame().unwrap();
    assert_eq!(s.get(Counter::HitTests), 1, "{s:?}");
}

/// A `data-onmousemove` handler still fires on every move, and hover shares
/// its hit test when the handler leaves the document alone.
#[test]
fn a_mousemove_handler_fires_and_shares_the_hover_hit_test() {
    let fired = Rc::new(Cell::new(0));
    let (mut app, _) = mount_long_list(Some(fired.clone()));
    frame(&mut app);
    pointer_move(&mut app, 50.0, 105.0);
    pointer_move(&mut app, 51.0, 106.0);
    let s = app.end_perf_frame().unwrap();
    assert_eq!(fired.get(), 2, "the handler fires once per move");
    assert_eq!(s.get(Counter::HitTests), 2, "one per move, shared: {s:?}");
}

/// A handler that changes the document between the dispatch and hover makes
/// hover test again rather than reuse an answer from the tree as it was.
#[test]
fn a_mousemove_handler_that_mutates_makes_hover_retest() {
    let target: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let t2 = target.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let a = scope.create_element("div");
        a.set_attribute("style", "height: 100px");
        let t3 = t2.clone();
        let id = scope.register_handler(move || {
            if let Some(n) = t3.borrow().as_ref() {
                n.set_attribute("data-touched", "1");
            }
        });
        a.set_attribute("data-onmousemove", &id.0.to_string());
        root.append_child(&a);
        *t2.borrow_mut() = Some(a);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    frame(&mut app);
    pointer_move(&mut app, 20.0, 20.0);
    let s = app.end_perf_frame().unwrap();
    assert_eq!(
        target
            .borrow()
            .as_ref()
            .unwrap()
            .get_attribute("data-touched")
            .as_deref(),
        Some("1")
    );
    assert_eq!(
        s.get(Counter::HitTests),
        2,
        "the mutation forced a re-test: {s:?}"
    );
}

/// A component drag (`Drag::absolute`) lays out once per frame, not once per
/// pointer event: N moves queued before the frame end in one layout. It used
/// to call `resolve_and_repaint` inside the `MouseMove` arm.
#[test]
fn queued_drag_moves_lay_out_once() {
    let (mut app, float) = mount();
    frame(&mut app);
    let x = rinch_core::reactive::Signal::new(0.0f32);
    let _e = rinch_core::reactive::Effect::new(move || {
        float.set_style("width", &format!("{}px", 50.0 + x.get()));
    });
    frame(&mut app);
    rinch_core::Drag::absolute()
        .on_move(move |px, _| x.set(px))
        .start();
    for i in 0..5 {
        pointer_move(&mut app, 100.0 + i as f32, 100.0);
    }
    app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    let s = frame(&mut app);
    rinch_core::Drag::cancel();
    assert_eq!(x.get(), 104.0, "every move reached on_move");
    assert_eq!(s.get(Counter::LayoutResolves), 1, "{s:?}");
    assert_eq!(s.get(Counter::TaffyRootComputes), 1, "{s:?}");
}

/// Wall-clock cost of a warm pointer move over a long list — the number the
/// counters above stand in for. Release only:
/// `cargo test --release -p rinch --lib pointer_move_timing -- --ignored --nocapture`
#[test]
#[ignore]
fn pointer_move_timing() {
    let (mut app, scroller) = mount_long_list(None);
    frame(&mut app);
    scroller.set_scroll_top(4000.0);
    frame(&mut app);
    pointer_move(&mut app, 50.0, 105.0);
    let rounds = 2000;
    let t = std::time::Instant::now();
    for i in 0..rounds {
        // Inside one row, so hover does not change and nothing re-lays out.
        pointer_move(&mut app, 40.0 + (i % 50) as f32, 105.0);
    }
    let per = t.elapsed().as_secs_f64() * 1e6 / rounds as f64;
    let s = app.end_perf_frame().unwrap();
    eprintln!(
        "[pointer_move_timing] {ROWS} rows: {per:.2} us per move; {} hit tests, {} nodes visited",
        s.get(Counter::HitTests),
        s.get(Counter::HitTestNodesVisited)
    );
}
