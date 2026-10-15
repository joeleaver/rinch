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

fn mount() -> (RinchApp, usize, usize) {
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

fn scroll_top(app: &RinchApp, id: usize) -> f64 {
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
