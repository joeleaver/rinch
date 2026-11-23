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

// --- The real-window paths: an OS window that is only its scale and size.

/// A winit window that is a display scale and a surface size, recording the
/// sizes asked of it: the zoom's real-window paths (`WinitWindow`'s scale,
/// a layer surface's `Moved` and gesture) driven without a compositor.
#[derive(Debug)]
struct FakeOsWindow {
    scale: f64,
    size: PhysicalSize<u32>,
    requested: std::sync::Mutex<Vec<winit::dpi::Size>>,
    ime: std::sync::Mutex<Vec<winit::window::ImeRequest>>,
}

impl FakeOsWindow {
    fn new(scale: f64, size: (u32, u32)) -> Self {
        FakeOsWindow {
            scale,
            size: PhysicalSize::new(size.0, size.1),
            requested: Default::default(),
            ime: Default::default(),
        }
    }
}

#[allow(unused_variables)]
mod fake_os_window {
    use super::FakeOsWindow;
    use winit::cursor::Cursor;
    use winit::dpi::{PhysicalInsets, PhysicalPosition, PhysicalSize, Position, Size};
    use winit::error::RequestError;
    use winit::icon::Icon;
    use winit::monitor::{Fullscreen, MonitorHandle};
    use winit::raw_window_handle as rwh_06;
    use winit::window::*;

    impl Window for FakeOsWindow {
        fn window_type(&self) -> WindowType {
            unimplemented!("FakeOsWindow::window_type")
        }
        fn id(&self) -> WindowId {
            unimplemented!("FakeOsWindow::id")
        }
        fn scale_factor(&self) -> f64 {
            self.scale
        }
        fn request_redraw(&self) {}
        fn pre_present_notify(&self) {
            unimplemented!("FakeOsWindow::pre_present_notify")
        }
        fn reset_dead_keys(&self) {
            unimplemented!("FakeOsWindow::reset_dead_keys")
        }
        fn surface_position(&self) -> PhysicalPosition<i32> {
            unimplemented!("FakeOsWindow::surface_position")
        }
        fn outer_position(&self) -> Result<PhysicalPosition<i32>, RequestError> {
            unimplemented!("FakeOsWindow::outer_position")
        }
        fn set_outer_position(&self, position: Position) {
            unimplemented!("FakeOsWindow::set_outer_position")
        }
        fn surface_size(&self) -> PhysicalSize<u32> {
            self.size
        }
        fn request_surface_size(&self, size: Size) -> Option<PhysicalSize<u32>> {
            self.requested.lock().unwrap().push(size);
            None
        }
        fn outer_size(&self) -> PhysicalSize<u32> {
            unimplemented!("FakeOsWindow::outer_size")
        }
        fn safe_area(&self) -> PhysicalInsets<u32> {
            unimplemented!("FakeOsWindow::safe_area")
        }
        fn set_min_surface_size(&self, min_size: Option<Size>) {
            unimplemented!("FakeOsWindow::set_min_surface_size")
        }
        fn set_max_surface_size(&self, max_size: Option<Size>) {
            unimplemented!("FakeOsWindow::set_max_surface_size")
        }
        fn surface_resize_increments(&self) -> Option<PhysicalSize<u32>> {
            unimplemented!("FakeOsWindow::surface_resize_increments")
        }
        fn set_surface_resize_increments(&self, increments: Option<Size>) {
            unimplemented!("FakeOsWindow::set_surface_resize_increments")
        }
        fn set_title(&self, title: &str) {
            unimplemented!("FakeOsWindow::set_title")
        }
        fn set_transparent(&self, transparent: bool) {
            unimplemented!("FakeOsWindow::set_transparent")
        }
        fn set_blur(&self, blur: bool) {
            unimplemented!("FakeOsWindow::set_blur")
        }
        fn set_visible(&self, visible: bool) {
            unimplemented!("FakeOsWindow::set_visible")
        }
        fn is_visible(&self) -> Option<bool> {
            unimplemented!("FakeOsWindow::is_visible")
        }
        fn set_resizable(&self, resizable: bool) {
            unimplemented!("FakeOsWindow::set_resizable")
        }
        fn is_resizable(&self) -> bool {
            unimplemented!("FakeOsWindow::is_resizable")
        }
        fn set_enabled_buttons(&self, buttons: WindowButtons) {
            unimplemented!("FakeOsWindow::set_enabled_buttons")
        }
        fn enabled_buttons(&self) -> WindowButtons {
            unimplemented!("FakeOsWindow::enabled_buttons")
        }
        fn set_minimized(&self, minimized: bool) {
            unimplemented!("FakeOsWindow::set_minimized")
        }
        fn is_minimized(&self) -> Option<bool> {
            Some(false)
        }
        fn set_maximized(&self, maximized: bool) {
            unimplemented!("FakeOsWindow::set_maximized")
        }
        fn is_maximized(&self) -> bool {
            false
        }
        fn set_fullscreen(&self, fullscreen: Option<Fullscreen>) {
            unimplemented!("FakeOsWindow::set_fullscreen")
        }
        fn fullscreen(&self) -> Option<Fullscreen> {
            unimplemented!("FakeOsWindow::fullscreen")
        }
        fn set_decorations(&self, decorations: bool) {
            unimplemented!("FakeOsWindow::set_decorations")
        }
        fn is_decorated(&self) -> bool {
            unimplemented!("FakeOsWindow::is_decorated")
        }
        fn set_window_level(&self, level: WindowLevel) {
            unimplemented!("FakeOsWindow::set_window_level")
        }
        fn set_window_icon(&self, window_icon: Option<Icon>) {
            unimplemented!("FakeOsWindow::set_window_icon")
        }
        fn request_ime_update(&self, request: ImeRequest) -> Result<(), ImeRequestError> {
            self.ime.lock().unwrap().push(request);
            Ok(())
        }
        fn ime_capabilities(&self) -> Option<ImeCapabilities> {
            unimplemented!("FakeOsWindow::ime_capabilities")
        }
        fn focus_window(&self) {
            unimplemented!("FakeOsWindow::focus_window")
        }
        fn has_focus(&self) -> bool {
            unimplemented!("FakeOsWindow::has_focus")
        }
        fn request_user_attention(&self, request_type: Option<UserAttentionType>) {
            unimplemented!("FakeOsWindow::request_user_attention")
        }
        fn set_theme(&self, theme: Option<Theme>) {
            unimplemented!("FakeOsWindow::set_theme")
        }
        fn theme(&self) -> Option<Theme> {
            unimplemented!("FakeOsWindow::theme")
        }
        fn set_content_protected(&self, protected: bool) {
            unimplemented!("FakeOsWindow::set_content_protected")
        }
        fn title(&self) -> String {
            unimplemented!("FakeOsWindow::title")
        }
        fn set_cursor(&self, cursor: Cursor) {
            unimplemented!("FakeOsWindow::set_cursor")
        }
        fn set_cursor_position(&self, position: Position) -> Result<(), RequestError> {
            unimplemented!("FakeOsWindow::set_cursor_position")
        }
        fn set_cursor_grab(&self, mode: CursorGrabMode) -> Result<(), RequestError> {
            unimplemented!("FakeOsWindow::set_cursor_grab")
        }
        fn set_cursor_visible(&self, visible: bool) {
            unimplemented!("FakeOsWindow::set_cursor_visible")
        }
        fn drag_window(&self) -> Result<(), RequestError> {
            unimplemented!("FakeOsWindow::drag_window")
        }
        fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), RequestError> {
            unimplemented!("FakeOsWindow::drag_resize_window")
        }
        fn show_window_menu(&self, position: Position) {
            unimplemented!("FakeOsWindow::show_window_menu")
        }
        fn set_cursor_hittest(&self, hittest: bool) -> Result<(), RequestError> {
            unimplemented!("FakeOsWindow::set_cursor_hittest")
        }
        fn current_monitor(&self) -> Option<MonitorHandle> {
            unimplemented!("FakeOsWindow::current_monitor")
        }
        fn available_monitors(&self) -> Box<dyn Iterator<Item = MonitorHandle>> {
            unimplemented!("FakeOsWindow::available_monitors")
        }
        fn primary_monitor(&self) -> Option<MonitorHandle> {
            unimplemented!("FakeOsWindow::primary_monitor")
        }
        fn rwh_06_display_handle(&self) -> &dyn rwh_06::HasDisplayHandle {
            unimplemented!("FakeOsWindow::rwh_06_display_handle")
        }
        fn rwh_06_window_handle(&self) -> &dyn rwh_06::HasWindowHandle {
            unimplemented!("FakeOsWindow::rwh_06_window_handle")
        }
    }
}

/// `rt`'s main window given an OS window at the display scale `os` with a
/// surface of `size` physical px.
fn with_os_window(rt: &mut RinchRuntime, os: f64, size: (u32, u32)) {
    rt.main.window = Some(WinitWindow::new(Box::new(FakeOsWindow::new(os, size))));
}

#[test]
fn a_real_window_draws_at_the_display_scale_times_the_zoom() {
    let _reset = Reset;
    let mut rt = runtime();
    with_os_window(&mut rt, 2.0, (1600, 1200));
    crate::zoom::set_zoom(1.5);
    let w = rt.main.window.as_ref().unwrap();
    assert_eq!(
        rinch_platform::PlatformWindow::scale_factor(w),
        3.0,
        "the document's scale: the display's 2x times the zoom"
    );
    assert_eq!(
        w.os_scale_factor(),
        2.0,
        "winit's own, for its logical units"
    );
    assert_eq!(rt.main.scale_factor(), 3.0);
    assert_eq!(rt.main.os_scale_factor(), 2.0);
    assert_eq!(
        rinch_platform::PlatformWindow::logical_size(w),
        (533, 400),
        "the same surface is a smaller CSS viewport"
    );
}

/// A layer surface's `Moved` carries its margins times the display's scale
/// (the fork's), never the zoom: read with the zoomed scale, the origin
/// would be 100 / 1.5 and the surface would jump on the first `Moved`.
#[test]
fn a_layer_surface_moved_is_read_at_the_display_scale() {
    let _reset = Reset;
    crate::zoom::set_zoom(1.5);
    for os in [1.0, 2.0] {
        let mut rt = runtime();
        with_os_window(&mut rt, os, (400, 400));
        rt.main.layer = Some(LayerState::new(Some((100.0, 100.0))));
        rt.main
            .layer_moved(((100.0 * os) as i32, (100.0 * os) as i32));
        assert_eq!(
            rt.main.layer.unwrap().applied(),
            Some((100.0, 100.0)),
            "at display {os}x"
        );
    }
    // With no window yet, the display's scale is 1x whatever the zoom.
    let mut rt = runtime();
    rt.main.layer = Some(LayerState::new(Some((100.0, 100.0))));
    rt.main.layer_moved((100, 100));
    assert_eq!(rt.main.layer.unwrap().applied(), Some((100.0, 100.0)));
}

/// A layer drag at zoom 1.5: the pointer the gesture starts from (CSS px)
/// is turned into the display's px, the unit every later pointer position
/// arrives in, so a pointer that has not moved moves nothing.
#[test]
fn a_layer_gesture_starts_where_the_pointer_is_on_the_display() {
    let _reset = Reset;
    crate::zoom::set_zoom(1.5);
    let mut rt = runtime();
    with_os_window(&mut rt, 1.0, (300, 300));
    rt.main.layer = Some(LayerState::new(Some((100.0, 100.0))));
    rt.main
        .pressed_pointers
        .note(0, Some(MouseButton::Left), true);
    // The pointer at CSS (20, 20) over the surface: display (30, 30).
    rt.main.app.cursor_pos = Some((20.0, 20.0));
    assert!(rt.main.start_layer_gesture(GestureKind::Move));
    assert_eq!(
        rt.main.follow_layer_gesture((30.0, 30.0)),
        Some(None),
        "a move asks for no size"
    );
    assert_eq!(
        rt.main.layer.unwrap().position,
        Some((100.0, 100.0)),
        "the pointer has not moved, so neither has the surface"
    );
}

/// The size a content-sized window is made at is the content's CSS size in
/// the display's logical px: times the zoom, as `follow_content_size` asks
/// later, so the first buffer is the content's size (#2652, #2552).
#[test]
fn a_content_sized_window_is_made_at_its_content_times_the_zoom() {
    let attrs = with_content_size(WindowAttributes::default(), Some((200, 100)), 1.5);
    assert_eq!(
        attrs.surface_size,
        Some(Size::Logical(LogicalSize::new(300.0, 150.0)))
    );
    let attrs = with_content_size(WindowAttributes::default(), None, 1.5);
    assert_eq!(attrs.surface_size, None, "no content size: unchanged");
}

/// 120 wide, 1000 tall: more than the largest size.
fn tall(scope: &mut RenderScope) -> NodeHandle {
    let node = scope.create_element("div");
    node.set_attribute("style", "width: 120px; height: 1000px");
    node
}

/// A content-sized window at zoom 2 is capped at its largest size on the
/// display (here the props' 300 x 600 logical px), not at twice it: its
/// document's limit is that size in CSS px, and it asks for its content's
/// size in the display's px at its creation.
#[test]
fn a_content_sized_window_asks_for_no_more_of_the_display_at_a_zoom() {
    use rinch_core::element::{ContentSizing, WindowProps};
    let _reset = Reset;
    crate::zoom::set_zoom(2.0);
    let props = WindowProps {
        size_to_content: Some(ContentSizing::new().with_max_width(300)),
        width: 800,
        height: 600,
        ..Default::default()
    };
    let mut app = app_for_window(tall, &props);
    app.window_props = Some(props);
    let mut w = ShellWindow::new(app, "t", 800, 600);
    // Mounted at the display's 1x times the zoom (no outputs are listed
    // here for `mount_scale` to read the display's from).
    w.mount_before_window(crate::zoom::apply(1.0));
    assert_eq!(w.content_max(), Some((150.0, 300.0)), "300 x 600 / 2");
    let content = w.start_surface_negotiation();
    assert_eq!(
        content,
        Some((120, 300)),
        "120 wide, capped at 300 CSS tall"
    );
    let attrs = with_content_size(WindowAttributes::default(), content, crate::zoom::current());
    assert_eq!(
        attrs.surface_size,
        Some(Size::Logical(LogicalSize::new(240.0, 600.0))),
        "within the display's 300 x 600"
    );
}

/// With no outputs listed yet, a content-sized second window is measured at
/// the window that opens it's scale (the display's times the zoom), not at
/// its own document's unset 1.0 over the zoom (#2886): its first frame is
/// laid out at the scale it is drawn at.
#[test]
fn with_no_outputs_a_content_sized_window_mounts_at_the_opening_windows_scale() {
    use rinch_core::element::{ContentSizing, WindowProps};
    let _reset = Reset;
    crate::zoom::set_zoom(1.5);
    let props = WindowProps {
        size_to_content: Some(ContentSizing::new()),
        ..Default::default()
    };
    let mut app = app_for_window(tall, &props);
    app.window_props = Some(props);
    let mut w = ShellWindow::new(app, "t", 800, 600);
    // The main window on a display at 2x, zoomed 1.5.
    let main = crate::zoom::apply(2.0);
    let scale = w.mount_scale(main);
    assert_eq!(scale, 3.0, "the display's 2 times the zoom");
    w.mount_before_window(scale);
    w.start_at_surface_scale(true);
    assert_eq!(
        w.app.device_pixel_ratio, 3.0,
        "and its surface starts there"
    );
}

/// A hidden window is prepared at the size it will be shown at: its saved
/// size is the display's logical px, a smaller CSS viewport at a zoom.
#[test]
fn a_hidden_window_is_prepared_at_its_saved_size_over_the_zoom() {
    let _reset = Reset;
    let mut rt = runtime();
    crate::zoom::set_zoom(1.5);
    rt.main.saved_geometry.size = Some(Size::Logical(LogicalSize::new(800.0, 600.0)));
    assert_eq!(rt.main.hidden_size(1.5), (533, 400));
    rt.main.saved_geometry.size = Some(Size::Physical(winit::dpi::PhysicalSize::new(1600, 1200)));
    assert_eq!(
        rt.main.hidden_size(3.0),
        (533, 400),
        "physical 1600 x 1200 at 2x times 1.5"
    );
}

/// With no window, the debug screenshot's size is the requested size laid
/// out at the zoom, as the window will be.
#[cfg(feature = "debug")]
#[test]
fn the_screenshot_fallback_size_follows_the_zoom() {
    let _reset = Reset;
    let rt = runtime();
    crate::zoom::set_zoom(1.5);
    assert_eq!(rt.main.logical_size(), (533, 400));
}

/// A 200 x 40 text target at the top left whose caret is at CSS (10, 20),
/// 2 x 16.
fn text_target(scope: &mut RenderScope) -> NodeHandle {
    use crate::focus_registry::{FocusEntry, register_focus_target};
    let root = scope.create_element("div");
    let div = scope.create_element("div");
    div.set_attribute("style", "width: 200px; height: 40px");
    div.set_attribute("tabindex", "0");
    register_focus_target(
        &div,
        FocusEntry::new()
            .on_ime(|_| {})
            .caret_rect(|| Some((10.0, 20.0, 2.0, 16.0))),
    );
    root.append_child(&div);
    root
}

/// The caret's box (CSS px) handed to the input method in the display's
/// logical px: times the zoom, so the candidate box sits under the caret.
#[test]
fn the_ime_area_is_the_caret_in_the_display_px() {
    use winit::window::ImeRequest;
    let _reset = Reset;
    crate::zoom::set_zoom(1.5);
    let mut rt = RinchRuntime::new("t", 800, 600, text_target, &[]);
    rt.main.app.set_device_pixel_ratio(1.5);
    rt.main.app.mount_component(533.0, 400.0);
    with_os_window(&mut rt, 1.0, (800, 600));
    let (size, scale) = (rt.main.window_size(), rt.main.scale_factor());
    assert_eq!(scale, 1.5);
    let button = PlatformMouseButton::Left;
    for event in [
        PlatformEvent::AboutToWait,
        PlatformEvent::WindowFocus(true),
        PlatformEvent::MouseDown {
            x: 100.0,
            y: 20.0,
            button,
        },
        PlatformEvent::MouseUp {
            x: 100.0,
            y: 20.0,
            button,
        },
    ] {
        let _ = rt.main.app.handle_event(event, size, scale);
    }
    assert!(rt.main.app.ime_state().enabled, "the text target has focus");
    rt.main.sync_ime();
    let w = rt.main.window.as_ref().unwrap();
    let fake = (&*w.window as &dyn std::any::Any)
        .downcast_ref::<FakeOsWindow>()
        .unwrap();
    let requests = fake.ime.lock().unwrap();
    let Some(ImeRequest::Enable(enable)) = requests.first() else {
        panic!("IME enabled: {requests:?}");
    };
    assert_eq!(
        enable.request_data().cursor_area,
        Some((
            winit::dpi::Position::Logical(LogicalPosition::new(15.0, 30.0)),
            Size::Logical(LogicalSize::new(3.0, 24.0)),
        )),
        "CSS (10, 20) 2 x 16 at zoom 1.5"
    );
}
