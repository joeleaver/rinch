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
        (Counter::LayoutSkippedTextOnly, 1),
        (Counter::ElementsCascaded, 1),
        // The row's text is re-shaped though only its background changed
        // (update-path audit F1.1). The minimum is 0.
        (Counter::ShapeIfcBuild, 1),
        // A mouse move hit-tests twice: once for `data-onmousemove` dispatch
        // and once for hover (update-path audit F2.1). The minimum is 1.
        (Counter::HitTests, 2),
        (Counter::HitTestNodesVisited, 26),
        // Each hit test builds the body's stacking sequence, and so does
        // paint: 2 + 1.
        (Counter::StackingOrderBuilds, 3),
        (Counter::PaintNodesVisited, 15),
    ] {
        assert_eq!(s.get(c), want, "{}: {s:?}", c.name());
    }
}

/// Moving an absolute box by its insets takes the inset fast path, which
/// throws the whole previous frame away (#280). Recorded under its own
/// reason so the fix for #280 can show the count go to zero.
#[test]
fn the_inset_fast_path_is_a_full_repaint_for_that_reason() {
    let (mut app, float) = mount();
    frame(&mut app);
    float.set_style("left", "40px");
    let s = frame(&mut app);
    assert_eq!(s.get(Counter::RepaintFull), 1, "{s:?}");
    assert_eq!(s.get(Counter::RepaintFullInsetFastPath), 1, "{s:?}");
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
