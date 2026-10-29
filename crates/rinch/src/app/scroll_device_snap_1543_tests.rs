//! #1543 at the shell: the scroll writers keep the offset on the device pixel
//! grid, so scrolled content moves by whole device pixels (the pixel half is
//! `rinch-dom/tests/scroll_device_snap_1543_tests.rs`).
//!
//! - The wheel keeps what a delta leaves below the grid, so a run of sub-pixel
//!   deltas (a slow touchpad, which Wayland reports in fractions) still moves
//!   the region rather than snapping back to where it was at every step.
//! - Scrolling down to reveal a box rounds **up** to the grid, so the box's
//!   bottom edge is not left a fraction of a pixel under the scroller's edge.
//!
//! The scale is 1.25 (Windows 125%), set the way the shell sets it.

use super::*;
use rinch_macros::rsx;

const VIEWPORT: (f32, f32) = (800.0, 600.0);
const SCALE: f64 = 1.25;

fn mount() -> (RinchApp, u64, u64) {
    let mut app = RinchApp::new(|__scope: &mut RenderScope| {
        rsx! {
            div { class: "scroller", style: "width: 200px; height: 100px; overflow-y: auto;",
                div { style: "height: 150.3px;" }
                div { class: "target", style: "height: 20px;" }
                div { style: "height: 400px;" }
            }
        }
    });
    app.set_device_pixel_ratio(SCALE);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let doc = app.doc.as_ref().expect("mounted").clone();
    let d = doc.borrow();
    let find = |class: &str| {
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("class").is_some_and(|c| c == class))
            .map(|(id, _)| id)
            .expect("the node is in the document")
    };
    let (s, t) = (find("scroller"), find("target"));
    drop(d);
    (app, s, t)
}

fn scroll_top(app: &RinchApp, id: u64) -> f64 {
    let doc = app.doc.as_ref().expect("a mounted document");
    doc.borrow().scroll_top(rinch_core::dom::NodeId(id))
}

fn on_grid(v: f64) -> bool {
    let d = v * SCALE;
    (d - d.round()).abs() < 1e-6
}

fn wheel(app: &mut RinchApp, delta_y: f64) {
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 50.0,
            y: 50.0,
            delta_x: 0.0,
            delta_y,
        },
        (VIEWPORT.0 as u32, VIEWPORT.1 as u32),
        SCALE,
    );
}

/// Five deltas of 0.3px down: 1.875 device px in all, so the region has moved
/// one device pixel (0.8px). Without the remainder each delta floors back to
/// 0 and the region never moves.
#[test]
fn sub_pixel_wheel_deltas_add_up() {
    let (mut app, s, _) = mount();
    for _ in 0..5 {
        wheel(&mut app, -0.3);
        assert!(
            on_grid(scroll_top(&app, s)),
            "off the grid: {}",
            scroll_top(&app, s)
        );
    }
    let top = scroll_top(&app, s);
    assert!(
        (top - 0.8).abs() < 1e-9,
        "five 0.3px deltas scrolled to {top}, not 0.8"
    );
    // And back up: the same deltas the other way return to the top.
    for _ in 0..5 {
        wheel(&mut app, 0.3);
    }
    assert_eq!(scroll_top(&app, s), 0.0);
}

/// An ordinary notch that is not a whole number of device pixels lands on the
/// grid at or below where it was aimed.
#[test]
fn a_wheel_notch_lands_on_the_grid() {
    let (mut app, s, _) = mount();
    wheel(&mut app, -33.3);
    let top = scroll_top(&app, s);
    // 41.625 device px -> 41 -> 32.8.
    assert!(on_grid(top) && (top - 32.8).abs() < 1e-9, "got {top}");
}

/// The target's bottom is at 170.3px, 70.3px past the 100px viewport: 87.875
/// device px of scroll. A floor would stop at 87 (69.6px) and leave 0.7px of
/// the box hidden; the reveal rounds up to 88 (70.4px).
#[test]
fn scrolling_a_box_into_view_from_above_rounds_up_to_the_grid() {
    let (mut app, s, t) = mount();
    {
        let doc = app.doc.as_ref().expect("mounted").clone();
        let mut d = doc.borrow_mut();
        d.tree.scroll_into_view_requests.push(t);
        // A style write, so the frame runs the layout pass the scroll is
        // applied after.
        d.set_style(rinch_core::dom::NodeId(t), "outline", "1px solid red");
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let top = scroll_top(&app, s);
    assert!(on_grid(top), "off the grid: {top}");
    assert!(
        top >= 70.3,
        "the box's bottom is still under the edge: scrolled to {top}"
    );
    assert!((top - 70.4).abs() < 1e-9, "got {top}");
}

/// The horizontal half of the wheel keeps its own remainder.
#[test]
fn sub_pixel_horizontal_wheel_deltas_add_up() {
    let mut app = RinchApp::new(|__scope: &mut RenderScope| {
        rsx! {
            div { class: "scroller", style: "width: 200px; height: 100px; overflow-x: auto; overflow-y: hidden;",
                div { style: "width: 600px; height: 50px;" }
            }
        }
    });
    app.set_device_pixel_ratio(SCALE);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let s = {
        let doc = app.doc.as_ref().expect("mounted").clone();
        let d = doc.borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("class").is_some_and(|c| c == "scroller"))
            .map(|(id, _)| id)
            .expect("the scroller")
    };
    let left = |app: &RinchApp| {
        let doc = app.doc.as_ref().expect("mounted");
        doc.borrow().tree.nodes[s].scroll_offset.0
    };
    for _ in 0..5 {
        app.handle_event(
            PlatformEvent::MouseWheel {
                x: 50.0,
                y: 50.0,
                delta_x: -0.3,
                delta_y: 0.0,
            },
            (VIEWPORT.0 as u32, VIEWPORT.1 as u32),
            SCALE,
        );
        assert!(on_grid(left(&app)), "off the grid: {}", left(&app));
    }
    assert!(
        (left(&app) - 0.8).abs() < 1e-9,
        "scrolled to {}, not 0.8",
        left(&app)
    );
}

// ---- review-1545 probes ----

/// A remainder left by the wheel survives another writer: after
/// `set_scroll_top`, a tiny delta should not jump a whole device pixel.
#[test]
fn review_stale_remainder_after_set_scroll_top() {
    let (mut app, s, _) = mount();
    wheel(&mut app, -0.7); // 0.875 device px -> stays at 0, remainder 0.7
    assert_eq!(scroll_top(&app, s), 0.0);
    {
        let doc = app.doc.as_ref().unwrap().clone();
        doc.borrow_mut()
            .set_scroll_top(rinch_core::dom::NodeId(s), 40.0);
    }
    wheel(&mut app, -0.2); // 40.2 -> 50.25 device -> floor 50 -> 40.0 expected
    let top = scroll_top(&app, s);
    assert!(
        (top - 40.0).abs() < 1e-9,
        "a 0.2px delta after set_scroll_top(40) moved to {top}"
    );
}

/// A display-scale change re-snaps the stored offset (or not).
#[test]
fn review_scale_change_resnaps() {
    let (mut app, s, _) = mount();
    wheel(&mut app, -33.3); // 32.8 at 1.25
    app.set_device_pixel_ratio(1.5);
    let top = scroll_top(&app, s);
    let d = top * 1.5;
    assert!(
        (d - d.round()).abs() < 1e-6,
        "after a scale change the offset {top} is off the 1.5 grid"
    );
}

/// A reveal near the bottom of the range must not round up past the range.
#[test]
fn review_reveal_round_up_stays_in_range() {
    let mut app = RinchApp::new(|__scope: &mut RenderScope| {
        rsx! {
            div { class: "scroller", style: "width: 200px; height: 100px; overflow-y: auto;",
                div { style: "height: 150.3px;" }
                div { class: "target", style: "height: 20.3px;" }
            }
        }
    });
    app.set_device_pixel_ratio(SCALE);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let doc = app.doc.as_ref().unwrap().clone();
    let find = |class: &str| {
        let d = doc.borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("class").is_some_and(|c| c == class))
            .map(|(id, _)| id)
            .unwrap()
    };
    let (s, t) = (find("scroller"), find("target"));
    {
        let mut d = doc.borrow_mut();
        d.tree.scroll_into_view_requests.push(t);
        d.set_style(rinch_core::dom::NodeId(t), "outline", "1px solid red");
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let (top, max) = {
        let d = doc.borrow();
        let id = rinch_core::dom::NodeId(s);
        (d.scroll_top(id), d.scroll_height(id) - d.client_height(id))
    };
    assert!(top <= max + 1e-9, "revealed past the range: {top} > {max}");
    assert!(on_grid(top), "off grid {top}");
    assert!(
        max - top < 1.0 / SCALE,
        "a whole device pixel short of the range end: {top} vs {max}"
    );
}

/// Dragging the thumb and returning the pointer to where it was pressed puts
/// the offset back exactly (no drift).
#[test]
fn review_scrollbar_drag_round_trip_no_drift() {
    let (mut app, s, _) = mount();
    let doc = app.doc.as_ref().unwrap().clone();
    let (x, y0) = {
        let d = doc.borrow();
        let n = &d.tree.nodes[s];
        let _ = n;
        (205.0f32, 12.0f32)
    };
    let _ = (x, y0);
    // The scroller sits at the body's origin plus margin; find its painted box.
    let b = rinch_dom::paint::painted_border_box(&doc.borrow().tree, s, 1.0);
    let bx = (b.x1 - 3.0) as f32;
    let by = (b.y0 + 5.0) as f32;
    let ev = |app: &mut RinchApp, e: PlatformEvent| {
        app.handle_event(e, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), SCALE);
    };
    ev(&mut app, PlatformEvent::MouseMove { x: bx, y: by });
    ev(
        &mut app,
        PlatformEvent::MouseDown {
            x: bx,
            y: by,
            button: rinch_platform::MouseButton::Left,
        },
    );
    assert!(
        app.scrollbar_drag.is_some(),
        "the press did not grab the thumb"
    );
    let start = scroll_top(&app, s);
    let mut seen = Vec::new();
    for dy in [3.3f32, 7.1, 11.9, 5.2, 0.0] {
        ev(&mut app, PlatformEvent::MouseMove { x: bx, y: by + dy });
        let t = scroll_top(&app, s);
        assert!(on_grid(t), "off grid {t}");
        seen.push(t);
    }
    assert_eq!(scroll_top(&app, s), start, "drift: {seen:?}");
}

/// Scrolling UP to reveal a box floors, so its top edge is not left under the
/// scroller's top edge.
#[test]
fn review_scrolling_up_to_reveal_keeps_the_top_visible() {
    let (mut app, s, t) = mount();
    {
        let doc = app.doc.as_ref().unwrap().clone();
        doc.borrow_mut()
            .set_scroll_top(rinch_core::dom::NodeId(s), 160.0);
    }
    assert_eq!(scroll_top(&app, s), 160.0);
    {
        let doc = app.doc.as_ref().unwrap().clone();
        let mut d = doc.borrow_mut();
        d.tree.scroll_into_view_requests.push(t);
        d.set_style(rinch_core::dom::NodeId(t), "outline", "1px solid red");
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let top = scroll_top(&app, s);
    assert!(on_grid(top), "off grid {top}");
    assert!(top <= 150.3, "the box's top is under the edge: {top}");
}

/// A track press whose jump snaps to the offset the scroller already has
/// moves nothing and fires no `onscroll` (the press compares the snapped jump).
///
/// At a coarse grid (scale 0.01: one device pixel is 100 CSS px) the range end
/// 950 snaps to 900, which leaves a few pixels of empty track after the thumb.
/// A press there jumps to about 940, which snaps back to 900. Compared
/// unsnapped, 940 != 900: the press wrote, re-snapped and fired `onscroll`.
#[test]
fn a_track_press_that_snaps_to_where_it_is_fires_nothing() {
    use rinch_core::events::{ScrollCallback, ScrollEvent, register_scroll_handler};
    use rinch_dom::paint::scrollbar::{MARGIN, THICKNESS};
    use std::cell::Cell;
    use std::rc::Rc;
    let fired = Rc::new(Cell::new(0usize));
    let fired_in = fired.clone();
    let handler = register_scroll_handler(ScrollCallback::from(move |_: ScrollEvent| {
        fired_in.set(fired_in.get() + 1);
    }));
    let hid = handler.0.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 200px; height: 100px; overflow-y: auto",
        );
        scroller.set_attribute("id", "scroller");
        scroller.set_attribute("data-onscroll", &hid);
        let inner = scope.create_element("div");
        inner.set_attribute("style", "width: 40px; height: 1050px");
        scroller.append_child(&inner);
        root.append_child(&scroller);
        root
    });
    app.set_device_pixel_ratio(0.01);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let doc = app.doc.as_ref().expect("mounted").clone();
    let s = doc
        .borrow()
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("scroller"))
        .map(|(id, _)| id)
        .expect("the scroller");
    doc.borrow_mut()
        .set_scroll_top(rinch_core::dom::NodeId(s), 950.0);
    assert_eq!(
        scroll_top(&app, s),
        900.0,
        "premise: the range end snaps to 900"
    );
    let t = {
        let d = doc.borrow();
        rinch_dom::paint::scrollbar::scrollbars(&d.tree, s, 1.0)
            .vertical
            .expect("a vertical bar")
    };
    let along = t.track_start + t.track_len - 1.0;
    assert!(
        !t.thumb_contains(900.0, along),
        "premise: the press is on the empty track after the thumb"
    );
    let jump = t.scroll_for_click(along);
    assert!(
        jump > 900.0 && jump < 1000.0,
        "premise: the jump {jump} snaps to 900"
    );
    let before = fired.get();
    let across = (MARGIN + THICKNESS / 2.0) as f32;
    app.handle_event(
        PlatformEvent::MouseDown {
            x: 200.0 - across,
            y: along as f32,
            button: rinch_platform::MouseButton::Left,
        },
        (VIEWPORT.0 as u32, VIEWPORT.1 as u32),
        1.0,
    );
    assert!(
        app.scrollbar_drag.is_some(),
        "premise: the press is on the bar"
    );
    assert_eq!(scroll_top(&app, s), 900.0);
    assert_eq!(
        fired.get(),
        before,
        "a jump that snaps to where it is fired onscroll"
    );
}
