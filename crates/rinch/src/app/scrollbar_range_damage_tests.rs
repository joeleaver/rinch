//! A scroll container's bars are damaged when its **range** changes without
//! the container itself changing (batch #2062, #770's follow-up).
//!
//! A bar's thumb is drawn from the container's scroll range, and the range is
//! its content's extent: a box moved past the edge, a row grown, a child
//! removed. None of those makes the container paint-dirty — its own box did
//! not move — so the damage named only the changed box, and a partial frame
//! kept the old bar: no thumb where one had appeared, or the old thumb's
//! length. #770 made it reachable from every absolute box on a page (an
//! absolute box now counts in the range of the scroll container at its
//! containing block, the page's being the `<body>`'s): the split runtime's
//! random sequences (`gpu_split_runtime_review_random_sequences`) moved a
//! 200px bar to `left: 210px` in a 400px page and the partial frame showed no
//! page bar where the full frame drew one. It was reachable before #770 too,
//! with a narrow in-flow row of a scroller growing (`a_row_that_grows_*`).
//!
//! Each case compares a partial frame against a from-scratch frame of the
//! same state in the strip the bar is drawn in, with a positive control that
//! the strip changed and a counter half that the frame really was partial.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::Cell;

const SIZE: (u32, u32) = (400, 300);
const SURFACE: u64 = 400 * 300;

/// The page's nodes: the moving bar, the scroller's first and last rows, and an
/// absolute box in a small holder.
#[derive(Clone, Copy, Default)]
struct Ids {
    bar: u64,
    row: u64,
    tail: u64,
    deep: u64,
}

/// The split runtime's page (`split_runtime_1865_tests::page`): an absolute
/// 400x300 root over the window, an absolute bar on it — and a 150x100
/// scroller whose narrow rows leave its bar strip to its own background.
fn mount() -> (RinchApp, Ids) {
    let ids: Rc<Cell<Ids>> = Rc::new(Cell::new(Ids::default()));
    let out = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
             background: rgb(60, 70, 90)",
        );
        let bar = scope.create_element("div");
        bar.set_attribute(
            "style",
            "position: absolute; left: 30px; top: 100px; width: 200px; height: 14px; \
             background: rgb(20, 20, 20)",
        );
        root.append_child(&bar);
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 240px; top: 20px; width: 150px; height: 100px; \
             overflow-y: auto; background: rgb(240, 240, 240)",
        );
        let mut got = Ids {
            bar: bar.node_id().0,
            ..Ids::default()
        };
        for i in 0..4 {
            let row = scope.create_element("div");
            row.set_attribute(
                "style",
                "width: 60px; height: 30px; background: rgb(0, 160, 0)",
            );
            scroller.append_child(&row);
            match i {
                0 => got.row = row.node_id().0,
                3 => got.tail = row.node_id().0,
                _ => {}
            }
        }
        root.append_child(&scroller);
        // A small `relative` holder with an absolute child: removing the
        // child leaves a parent whose own box holds.
        let holder = scope.create_element("div");
        holder.set_attribute(
            "style",
            "position: absolute; left: 20px; top: 150px; width: 20px; height: 20px; \
             background: rgb(200, 0, 200)",
        );
        let deep = scope.create_element("div");
        deep.set_attribute(
            "style",
            "position: absolute; left: 5px; top: 5px; width: 10px; height: 10px; \
             background: rgb(255, 255, 0)",
        );
        holder.append_child(&deep);
        root.append_child(&holder);
        got.deep = deep.node_id().0;
        out.set(got);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let ids = ids.get();
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

/// Pixels of `rect` (x0, y0, x1, y1) that differ by more than a rounding step.
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

fn set_style(app: &RinchApp, id: u64, property: &str, value: &str) {
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut()
        .set_style(rinch_core::dom::NodeId(id), property, value);
}

#[track_caller]
fn assert_partial(stats: &FrameStats) {
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
    assert!(stats.get(Counter::RepaintedPx) < SURFACE / 2, "{stats:?}");
}

/// The page's horizontal bar: the bottom 16px of the window.
const PAGE_BOTTOM: (i32, i32, i32, i32) = (0, 284, 400, 300);

/// The split runtime's seed 1, frame 1: the bar moves to `left: 210px`, past
/// the page's right edge, and the page gets a horizontal bar (#770). Then it
/// moves back, and the bar goes.
#[test]
fn an_absolute_moved_past_the_page_edge_damages_the_page_bar() {
    let (mut app, ids) = mount();
    let (before, _) = frame(&mut app);
    set_style(&app, ids.bar, "left", "210px");
    set_style(&app, ids.bar, "top", "59px");
    let (moved, stats) = frame(&mut app);
    assert_partial(&stats);
    let want = full_frame(&mut app);
    assert!(
        diff_in(&before, &want, PAGE_BOTTOM) > 0,
        "positive control: the page bar appeared"
    );
    assert_eq!(
        diff_in(&moved, &want, PAGE_BOTTOM),
        0,
        "the partial frame draws the page bar the full frame draws"
    );

    set_style(&app, ids.bar, "left", "30px");
    let (back, stats) = frame(&mut app);
    assert_partial(&stats);
    let want = full_frame(&mut app);
    assert!(
        diff_in(&moved, &want, PAGE_BOTTOM) > 0,
        "positive control: the page bar went"
    );
    assert_eq!(diff_in(&back, &want, PAGE_BOTTOM), 0);
}

/// An absolute box reaching below the page is removed: the page's vertical
/// bar goes, and the parent it left (a small holder, whose own box held)
/// is the only thing that can name the page's range.
#[test]
fn a_removed_absolute_damages_the_page_bar() {
    let (mut app, ids) = mount();
    set_style(&app, ids.deep, "top", "300px");
    let (before, _) = frame(&mut app);
    let want = full_frame(&mut app);
    assert_eq!(diff_in(&before, &want, PAGE_RIGHT), 0);
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        let parent = d.tree.get(ids.deep).and_then(|n| n.parent).unwrap();
        d.remove_child(
            rinch_core::dom::NodeId(parent),
            rinch_core::dom::NodeId(ids.deep),
        );
    }
    let (removed, stats) = frame(&mut app);
    assert_partial(&stats);
    let want = full_frame(&mut app);
    assert!(
        diff_in(&before, &want, PAGE_RIGHT) > 0,
        "positive control: the page bar went"
    );
    assert_eq!(diff_in(&removed, &want, PAGE_RIGHT), 0);
}

/// The page's vertical bar: the right 16px of the window, above the
/// scroller.
const PAGE_RIGHT: (i32, i32, i32, i32) = (384, 130, 400, 300);

/// The scroller's bar strip: its right 16px.
const SCROLLER_STRIP: (i32, i32, i32, i32) = (374, 20, 390, 121);

/// A row of a scroller grows (in flow, narrow, so nothing it damages reaches
/// the bar): the range grows and the thumb shortens. Reachable before #770.
#[test]
fn a_row_that_grows_damages_its_scrollers_bar() {
    let (mut app, ids) = mount();
    let (before, _) = frame(&mut app);
    set_style(&app, ids.row, "height", "90px");
    let (grown, stats) = frame(&mut app);
    assert_partial(&stats);
    let want = full_frame(&mut app);
    assert!(
        diff_in(&before, &want, SCROLLER_STRIP) > 0,
        "positive control: the thumb changed"
    );
    assert_eq!(diff_in(&grown, &want, SCROLLER_STRIP), 0);
}

/// A row is removed: the range shrinks and the thumb lengthens, with no box
/// left to name the change but the removed one.
#[test]
fn a_removed_row_damages_its_scrollers_bar() {
    let (mut app, ids) = mount();
    let (before, _) = frame(&mut app);
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        let parent = d.tree.get(ids.tail).and_then(|n| n.parent).unwrap();
        d.remove_child(
            rinch_core::dom::NodeId(parent),
            rinch_core::dom::NodeId(ids.tail),
        );
    }
    let (removed, stats) = frame(&mut app);
    assert_partial(&stats);
    let want = full_frame(&mut app);
    assert!(
        diff_in(&before, &want, SCROLLER_STRIP) > 0,
        "positive control: the thumb changed"
    );
    assert_eq!(diff_in(&removed, &want, SCROLLER_STRIP), 0);
}

/// A 150x100 scroller at `top: 20.3px` holding a 10.3px row and a 90.1px one
/// (narrow, so neither reaches the bar strip): 100.4px of content in 100px,
/// which rounds to no range (#2116). Returns the app and the second row.
fn mount_subpixel() -> (RinchApp, u64) {
    let row_id: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let out = row_id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
             background: rgb(60, 70, 90)",
        );
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 240px; top: 20.3px; width: 150px; height: 100px; \
             overflow-y: auto; background: rgb(240, 240, 240)",
        );
        for h in ["10.3px", "90.1px"] {
            let row = scope.create_element("div");
            row.set_attribute(
                "style",
                &format!("width: 60px; height: {h}; background: rgb(0, 160, 0)"),
            );
            scroller.append_child(&row);
            out.set(row.node_id().0);
        }
        root.append_child(&scroller);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let row = row_id.get();
    frame(&mut app);
    frame(&mut app);
    frame(&mut app);
    (app, row)
}

/// The row grows by a sub-pixel amount that crosses the whole-pixel range
/// (100.4 → 100.55: none → 1px, Chrome's rounding, #2116) while every rounded
/// rect holds — the row stays `{y: 10, h: 90}`. The bar appears all the same,
/// so a geometry change is read from the unrounded layout too.
#[test]
fn a_sub_pixel_growth_that_crosses_the_whole_pixel_range_damages_the_bar() {
    let (mut app, row) = mount_subpixel();
    let (before, _) = frame(&mut app);
    let layout_before = {
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        d.tree.get(row).unwrap().layout
    };
    set_style(&app, row, "height", "90.25px");
    let (grown, stats) = frame(&mut app);
    {
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        assert_eq!(
            d.tree.get(row).unwrap().layout,
            layout_before,
            "fixture: the rounded rect holds"
        );
    }
    assert_partial(&stats);
    let want = full_frame(&mut app);
    assert!(
        diff_in(&before, &want, SCROLLER_STRIP) > 0,
        "positive control: the bar appeared"
    );
    assert_eq!(diff_in(&grown, &want, SCROLLER_STRIP), 0);
}
