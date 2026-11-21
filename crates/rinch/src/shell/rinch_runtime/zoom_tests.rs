//! `rinch::window::set_zoom`: the app's zoom multiplies the display's scale.
//! The same surface is laid out at a smaller logical viewport, a pointer is
//! read back through the zoomed scale so a press lands on what is drawn
//! under it, and a move to a display with another scale keeps the zoom.
//! Driven through the runtime without an OS window, as the shell drives a
//! window: with no window its scale is the zoom over a display at 1x.

use super::*;
use crate::window::{scale_factor, window_size, zoom};
use rinch_platform::to_logical_point;
use std::cell::Cell;
use winit::dpi::PhysicalSize;

thread_local! {
    static CLICKS: Cell<u32> = const { Cell::new(0) };
}

/// The zoom is the thread's: put it back for the next test this thread runs.
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        crate::zoom::set_zoom(1.0);
        CLICKS.with(|c| c.set(0));
    }
}

/// A clickable box at CSS (100, 0)-(200, 50).
fn component(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("style", "width: 100%; height: 100%");
    let target = scope.create_element("div");
    target.set_attribute(
        "style",
        "position: absolute; left: 100px; top: 0px; width: 100px; height: 50px",
    );
    let click = scope.register_handler(|| CLICKS.with(|c| c.set(c.get() + 1)));
    target.set_attribute("data-rid", &click.0.to_string());
    root.append_child(&target);
    root
}

/// A runtime whose main window's surface is 800x600 at the display's 1x.
fn runtime() -> RinchRuntime {
    let mut rt = RinchRuntime::new("t", 800, 600, component, &[]);
    rt.main.app.set_device_pixel_ratio(1.0);
    rt.main.app.mount_component(800.0, 600.0);
    rt
}

fn settle(rt: &mut RinchRuntime) {
    let (size, scale) = (rt.main.window_size(), rt.main.scale_factor());
    let _ = rt
        .main
        .app
        .handle_event(PlatformEvent::AboutToWait, size, scale);
}

/// A press and release at the physical point `at`, converted as the shell
/// converts winit's pointer positions.
fn click_at_physical(rt: &mut RinchRuntime, at: (f64, f64)) {
    let (size, scale) = (rt.main.window_size(), rt.main.scale_factor());
    let (x, y) = to_logical_point(at, scale);
    let (x, y) = (x as f32, y as f32);
    let button = PlatformMouseButton::Left;
    let _ = rt
        .main
        .app
        .handle_event(PlatformEvent::MouseMove { x, y }, size, scale);
    let _ = rt
        .main
        .app
        .handle_event(PlatformEvent::MouseDown { x, y, button }, size, scale);
    let _ = rt
        .main
        .app
        .handle_event(PlatformEvent::MouseUp { x, y, button }, size, scale);
}

fn viewport(rt: &RinchRuntime) -> (f32, f32) {
    let d = rt.main.app.doc.as_ref().unwrap().borrow();
    (d.tree.viewport.width, d.tree.viewport.height)
}

#[test]
fn a_zoom_lays_the_same_surface_out_smaller() {
    let _reset = Reset;
    let mut rt = runtime();
    settle(&mut rt);
    assert_eq!(viewport(&rt), (800.0, 600.0));

    assert_eq!(crate::zoom::set_zoom(1.5), 1.5);
    let _ = rt.main.rezoom();
    settle(&mut rt);

    assert_eq!(
        rt.main.scale_factor(),
        1.5,
        "the display's 1x times the zoom"
    );
    assert_eq!(
        rt.main.app.device_pixel_ratio, 1.5,
        "the document is drawn at it"
    );
    assert_eq!(
        viewport(&rt),
        (533.0, 400.0),
        "800x600 surface pixels at 1.5 are 533x400 CSS pixels"
    );
    assert_eq!(window_size(), (533.0, 400.0), "what the app reads");
    assert_eq!(scale_factor(), 1.5, "the published scale includes the zoom");
    assert_eq!(zoom(), 1.5);

    crate::zoom::set_zoom(0.5);
    let _ = rt.main.rezoom();
    settle(&mut rt);
    assert_eq!(viewport(&rt), (1600.0, 1200.0), "zoomed out, more fits");
}

#[test]
fn a_press_lands_on_what_is_drawn_under_it() {
    let _reset = Reset;
    let mut rt = runtime();
    crate::zoom::set_zoom(1.5);
    let _ = rt.main.rezoom();
    settle(&mut rt);

    // The box is drawn at physical (150, 0)-(300, 75) at 1.5.
    click_at_physical(&mut rt, (290.0, 30.0));
    assert_eq!(
        CLICKS.with(Cell::get),
        1,
        "physical 290 is CSS 193, inside the box (outside it at 1x)"
    );
    click_at_physical(&mut rt, (120.0, 30.0));
    assert_eq!(
        CLICKS.with(Cell::get),
        1,
        "physical 120 is CSS 80, left of the box (inside it at 1x)"
    );
}

#[test]
fn a_new_display_scale_keeps_the_zoom() {
    let _reset = Reset;
    let mut rt = runtime();
    crate::zoom::set_zoom(1.5);
    let _ = rt.main.rezoom();

    // The window moves to a 2x display: winit says so, then sends the
    // surface's new size.
    let (size, scale) = ((1600, 1200), 3.0);
    let _ = rt
        .main
        .app
        .handle_event(display_scale_changed(2.0), size, scale);
    let resized = rt
        .main
        .surface_resized(PhysicalSize::new(size.0, size.1), scale);
    let _ = rt.main.app.handle_event(resized, size, scale);
    let _ = rt
        .main
        .app
        .handle_event(PlatformEvent::AboutToWait, size, scale);

    assert_eq!(rt.main.app.device_pixel_ratio, 3.0, "2x times the zoom");
    assert_eq!(
        viewport(&rt),
        (533.0, 400.0),
        "the same CSS viewport as at 1x"
    );
    assert_eq!(zoom(), 1.5, "the zoom is kept");
}
