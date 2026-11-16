//! What one turn of a single-window `TestApp` does, pinned by its frame
//! counters (#2793): an idle pump, a pump after a signal write, and a click.
//! The numbers were taken on rinch's `main` (3e1b1c21), before the harness
//! drove child windows, with this test and its counter hook alone; the
//! harness leaves them as they were. A later change that makes a
//! single-window app pay for child windows (or for anything else) is caught
//! here, and the child windows' part of a turn, which costs no counter when
//! no child is open, is pinned as not entered.

use rinch_core::Signal;
use rinch_core::dom::RenderScope;
use rinch_dom::perf::{Counter, FrameStats};

use super::TestApp;
use crate::app::perf_expect::expect_frame;

/// A window with a button and a box whose width a signal sets. No text, so
/// no number depends on the host's fonts.
fn mounted() -> (TestApp, Signal<u32>) {
    let width = std::rc::Rc::new(std::cell::Cell::new(None));
    let w = width.clone();
    let mut app = TestApp::mount(move |scope: &mut RenderScope| {
        let wide = Signal::new(40_u32);
        w.set(Some(wide));
        let root = scope.create_element("div");
        let button = scope.create_element("button");
        button.set_attribute("id", "b");
        button.set_attribute("style", "width: 50px; height: 20px");
        let handler = scope.register_handler(move || wide.set(wide.get() + 10));
        button.set_attribute("data-rid", &handler.0.to_string());
        root.append_child(&button);
        let bar = scope.create_element("div");
        bar.set_attribute("id", "bar");
        let b = bar.clone();
        scope.create_effect(move || {
            b.set_attribute("style", &format!("width: {}px; height: 10px", wide.get()));
        });
        root.append_child(&bar);
        root
    });
    app.settle();
    let signal = width.get().expect("mounted");
    (app, signal)
}

/// Every counter one pump made: the frames it painted, and what came after
/// them.
fn pumped(app: &mut TestApp, pump: impl FnOnce(&mut TestApp)) -> FrameStats {
    app.painted = FrameStats::default();
    app.app.reset_perf();
    pump(app);
    let mut total = std::mem::take(&mut app.painted);
    if let Some(rest) = app.app.end_perf_frame() {
        total.accumulate(&rest);
    }
    total
}

#[test]
fn a_single_window_turn_does_what_it_did() {
    let (mut app, wide) = mounted();

    let idle = pumped(&mut app, |app| {
        app.turn(false);
        app.serve_window_requests();
    });
    expect_frame("idle pump", &idle, &IDLE);

    let write = pumped(&mut app, |app| {
        wide.set(80);
        app.turn(false);
        app.serve_window_requests();
    });
    expect_frame("pump after a signal write", &write, &WRITE);

    let click = pumped(&mut app, |app| app.click_on("#b"));
    expect_frame("click", &click, &CLICK);
    // The child windows' part of a turn is not entered while none is open.
    assert_eq!(app.child_turns, 0);
}

const IDLE: [(Counter, u64); 0] = [];
const WRITE: [(Counter, u64); 21] = [
    (Counter::StyleResolves, 1),
    (Counter::ElementsCascaded, 1),
    (Counter::StyleNodesVisited, 1),
    (Counter::StyleInvalidations, 1),
    (Counter::TaffyStyleSyncs, 1),
    (Counter::TaffyStyleChanges, 1),
    (Counter::LayoutResolves, 1),
    (Counter::TaffyRootComputes, 1),
    (Counter::TaffyMeasureCalls, 1),
    (Counter::PaintFrames, 1),
    (Counter::RepaintPartial, 1),
    (Counter::DamageRects, 1),
    (Counter::RepaintedPx, 1512),
    (Counter::SurfacePx, 480000),
    (Counter::PaintNodesVisited, 5),
    (Counter::StackingOrderBuilds, 1),
    (Counter::ClipMasks, 1),
    (Counter::ClipMaskPx, 1892),
    (Counter::PaintSurfaceAllocs, 1),
    (Counter::EffectRuns, 1),
    (Counter::SignalNotifies, 1),
];
const CLICK: [(Counter, u64); 24] = [
    (Counter::StyleResolves, 1),
    (Counter::ElementsCascaded, 1),
    (Counter::StyleNodesVisited, 1),
    (Counter::StyleInvalidations, 1),
    (Counter::TaffyStyleSyncs, 1),
    (Counter::TaffyStyleChanges, 1),
    (Counter::LayoutResolves, 1),
    (Counter::TaffyRootComputes, 1),
    (Counter::TaffyMeasureCalls, 1),
    (Counter::PaintFrames, 1),
    (Counter::RepaintPartial, 1),
    (Counter::DamageRects, 1),
    (Counter::RepaintedPx, 1692),
    (Counter::SurfacePx, 480000),
    (Counter::PaintNodesVisited, 5),
    (Counter::StackingOrderBuilds, 4),
    (Counter::ClipMasks, 1),
    (Counter::ClipMaskPx, 2112),
    (Counter::HitTests, 4),
    (Counter::HitTestNodesVisited, 16),
    (Counter::HitExtentsComputed, 8),
    (Counter::ScrollProbeNodesVisited, 4),
    (Counter::EffectRuns, 1),
    (Counter::SignalNotifies, 1),
];

/// The counter the test above reads does count: an open child's part of a
/// turn is entered, every turn.
#[test]
fn an_open_childs_part_of_a_turn_is_counted() {
    let mut app = TestApp::mount(|scope: &mut RenderScope| {
        crate::open_window(
            rinch_core::element::WindowProps::default(),
            |s: &mut RenderScope| s.create_element("div"),
        );
        scope.create_element("div")
    });
    let before = app.child_turns;
    app.turn(false);
    assert_eq!(app.child_turns, before + 1);
}
