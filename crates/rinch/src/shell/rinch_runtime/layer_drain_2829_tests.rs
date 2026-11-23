//! #2829: a turn of native events on a runtime with a **live** layer
//! surface, run through the drain ([`RinchRuntime::run_native_events`]),
//! with an OS window that stands in for the fork's. The window records each
//! size request and applies it at once, as a layer surface with no request
//! in flight does; it is no Wayland window, so the fork's
//! `set_layer_shell_changes_and_size` falls back to `request_surface_size`
//! for it and the turn's one request is the one size request it records.
//! The commits themselves, on a compositor, are pinned live by
//! `tests/layer_one_commit_2464.rs`.

use super::*;
use crate::shell::layer_shell::LayerTurn;
use crate::shell::window_geometry;
use rinch_core::element::{ContentSizing, LayerAnchor, LayerMargins, LayerSurface, WindowProps};
use std::sync::{Arc, Mutex};
use winit::dpi::{PhysicalInsets, PhysicalPosition, PhysicalSize};

/// What the stand-in window was asked, and the size it has.
#[derive(Debug, Default)]
struct Surface {
    size: PhysicalSize<u32>,
    requests: Vec<Size>,
}

/// An OS window that applies each size request at once.
#[derive(Debug)]
struct AtOnceWindow {
    scale: f64,
    surface: Arc<Mutex<Surface>>,
}

impl winit::window::Window for AtOnceWindow {
    fn window_type(&self) -> winit::window::WindowType {
        winit::window::WindowType::Window
    }
    fn id(&self) -> winit::window::WindowId {
        winit::window::WindowId::from_raw(2829)
    }
    fn scale_factor(&self) -> f64 {
        self.scale
    }
    fn request_redraw(&self) {}
    fn pre_present_notify(&self) {}
    fn reset_dead_keys(&self) {}
    fn surface_position(&self) -> PhysicalPosition<i32> {
        PhysicalPosition::new(0, 0)
    }
    fn outer_position(&self) -> Result<PhysicalPosition<i32>, winit::error::RequestError> {
        Ok(PhysicalPosition::new(0, 0))
    }
    fn set_outer_position(&self, _: Position) {}
    fn surface_size(&self) -> PhysicalSize<u32> {
        self.surface.lock().unwrap().size
    }
    fn request_surface_size(&self, size: Size) -> Option<PhysicalSize<u32>> {
        let mut surface = self.surface.lock().unwrap();
        surface.requests.push(size);
        surface.size = size.to_physical(self.scale);
        Some(surface.size)
    }
    fn outer_size(&self) -> PhysicalSize<u32> {
        self.surface_size()
    }
    fn safe_area(&self) -> PhysicalInsets<u32> {
        PhysicalInsets::new(0, 0, 0, 0)
    }
    fn set_min_surface_size(&self, _: Option<Size>) {}
    fn set_max_surface_size(&self, _: Option<Size>) {}
    fn surface_resize_increments(&self) -> Option<PhysicalSize<u32>> {
        None
    }
    fn set_surface_resize_increments(&self, _: Option<Size>) {}
    fn set_title(&self, _: &str) {}
    fn set_transparent(&self, _: bool) {}
    fn set_blur(&self, _: bool) {}
    fn set_visible(&self, _: bool) {}
    fn is_visible(&self) -> Option<bool> {
        Some(true)
    }
    fn set_resizable(&self, _: bool) {}
    fn is_resizable(&self) -> bool {
        false
    }
    fn set_enabled_buttons(&self, _: winit::window::WindowButtons) {}
    fn enabled_buttons(&self) -> winit::window::WindowButtons {
        winit::window::WindowButtons::empty()
    }
    fn set_minimized(&self, _: bool) {}
    fn is_minimized(&self) -> Option<bool> {
        Some(false)
    }
    fn set_maximized(&self, _: bool) {}
    fn is_maximized(&self) -> bool {
        false
    }
    fn set_fullscreen(&self, _: Option<winit::monitor::Fullscreen>) {}
    fn fullscreen(&self) -> Option<winit::monitor::Fullscreen> {
        None
    }
    fn set_decorations(&self, _: bool) {}
    fn is_decorated(&self) -> bool {
        false
    }
    fn set_window_level(&self, _: winit::window::WindowLevel) {}
    fn set_window_icon(&self, _: Option<winit::icon::Icon>) {}
    fn request_ime_update(
        &self,
        _: winit::window::ImeRequest,
    ) -> Result<(), winit::window::ImeRequestError> {
        Err(winit::window::ImeRequestError::NotSupported)
    }
    fn ime_capabilities(&self) -> Option<winit::window::ImeCapabilities> {
        None
    }
    fn focus_window(&self) {}
    fn has_focus(&self) -> bool {
        false
    }
    fn request_user_attention(&self, _: Option<winit::window::UserAttentionType>) {}
    fn set_theme(&self, _: Option<winit::window::Theme>) {}
    fn theme(&self) -> Option<winit::window::Theme> {
        None
    }
    fn set_content_protected(&self, _: bool) {}
    fn title(&self) -> String {
        String::new()
    }
    fn set_cursor(&self, _: winit::cursor::Cursor) {}
    fn set_cursor_position(&self, _: Position) -> Result<(), winit::error::RequestError> {
        Ok(())
    }
    fn set_cursor_grab(
        &self,
        _: winit::window::CursorGrabMode,
    ) -> Result<(), winit::error::RequestError> {
        Ok(())
    }
    fn set_cursor_visible(&self, _: bool) {}
    fn drag_window(&self) -> Result<(), winit::error::RequestError> {
        Ok(())
    }
    fn drag_resize_window(
        &self,
        _: winit::window::ResizeDirection,
    ) -> Result<(), winit::error::RequestError> {
        Ok(())
    }
    fn show_window_menu(&self, _: Position) {}
    fn set_cursor_hittest(&self, _: bool) -> Result<(), winit::error::RequestError> {
        Ok(())
    }
    fn current_monitor(&self) -> Option<winit::monitor::MonitorHandle> {
        None
    }
    fn available_monitors(&self) -> Box<dyn Iterator<Item = winit::monitor::MonitorHandle>> {
        Box::new(std::iter::empty())
    }
    fn primary_monitor(&self) -> Option<winit::monitor::MonitorHandle> {
        None
    }
    fn rwh_06_display_handle(&self) -> &dyn winit::raw_window_handle::HasDisplayHandle {
        unimplemented!("no display behind the stand-in window")
    }
    fn rwh_06_window_handle(&self) -> &dyn winit::raw_window_handle::HasWindowHandle {
        unimplemented!("no window behind the stand-in window")
    }
}

/// An event loop with no OS behind it, for the drain.
#[derive(Debug, Default)]
struct NoOsLoop;

impl winit::event_loop::ActiveEventLoop for NoOsLoop {
    fn create_proxy(&self) -> winit::event_loop::EventLoopProxy {
        unimplemented!("not asked by these turns")
    }
    fn create_window(
        &self,
        _: winit::window::WindowAttributes,
    ) -> Result<Box<dyn winit::window::Window>, winit::error::CreateWindowError> {
        unimplemented!("not asked by these turns")
    }
    fn create_custom_cursor(
        &self,
        _: winit::cursor::CustomCursorSource,
    ) -> Result<winit::cursor::CustomCursor, winit::error::CustomCursorError> {
        unimplemented!("not asked by these turns")
    }
    fn available_monitors(&self) -> Box<dyn Iterator<Item = winit::monitor::MonitorHandle>> {
        Box::new(std::iter::empty())
    }
    fn primary_monitor(&self) -> Option<winit::monitor::MonitorHandle> {
        None
    }
    fn listen_device_events(&self, _: winit::event_loop::DeviceEvents) {}
    fn system_theme(&self) -> Option<winit::window::Theme> {
        None
    }
    fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
    fn control_flow(&self) -> winit::event_loop::ControlFlow {
        winit::event_loop::ControlFlow::Wait
    }
    fn exit(&self) {}
    fn exiting(&self) -> bool {
        false
    }
    fn owned_display_handle(&self) -> winit::event_loop::OwnedDisplayHandle {
        unimplemented!("not asked by these turns")
    }
    fn rwh_06_handle(&self) -> &dyn winit::raw_window_handle::HasDisplayHandle {
        unimplemented!("not asked by these turns")
    }
}

fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

fn logical(width: f64, height: f64) -> Size {
    Size::Logical(LogicalSize::new(width, height))
}

/// Give `w` a live layer surface of the physical `size` at `scale`: the
/// stand-in window, and the layer-shell state `take_surface_kind` sets for
/// the app's own surface. Answers what the window records.
fn go_live(w: &mut ShellWindow, size: (u32, u32), scale: f64) -> Arc<Mutex<Surface>> {
    let surface = Arc::new(Mutex::new(Surface {
        size: PhysicalSize::new(size.0, size.1),
        requests: Vec::new(),
    }));
    w.window = Some(WinitWindow::new(Box::new(AtOnceWindow {
        scale,
        surface: surface.clone(),
    })));
    w.layer = Some(LayerState::app_placed());
    surface
}

fn requests(surface: &Arc<Mutex<Surface>>) -> Vec<Size> {
    surface.lock().unwrap().requests.clone()
}

/// A runtime whose main window is mounted at `scale` and is a live layer
/// surface of 800 x 40 logical px.
fn live_main(scale: f64) -> (RinchRuntime, Arc<Mutex<Surface>>) {
    let mut runtime = RinchRuntime::new("t", 800, 40, component, &[]);
    runtime.main.app.set_device_pixel_ratio(scale);
    runtime.main.app.mount_component(800.0, 40.0);
    let physical = ((800.0 * scale) as u32, (40.0 * scale) as u32);
    let surface = go_live(&mut runtime.main, physical, scale);
    (runtime, surface)
}

const MAIN: EventFor = EventFor::Sender(None);

/// Item 1: a turn's sizes are staged, the newest winning, and go out as
/// the turn's one request when the drain ends — not each as its event is
/// handled (a commit each, #2826).
#[test]
fn a_turns_sizes_are_one_request_when_the_drain_ends() {
    let (mut runtime, surface) = live_main(1.0);
    runtime.run_native_events(
        vec![
            (MAIN, RinchNativeEvent::SetWindowSize(800.0, 48.0)),
            (MAIN, RinchNativeEvent::SetWindowSize(800.0, 60.0)),
        ],
        &NoOsLoop,
    );
    assert_eq!(requests(&surface), vec![logical(800.0, 60.0)]);
    assert_eq!(runtime.main.take_layer_turn(), LayerTurn::Nothing);
}

/// Item 1: a size the drain's one request applied at once has no
/// `SurfaceResized` from winit, so the drain hands one to the window's
/// resize path: the surface size the app reads is the new one. At scale
/// 1.5, so the physical size is not the logical one.
#[test]
fn a_size_applied_at_once_reaches_the_resize_path() {
    let (mut runtime, surface) = live_main(1.5);
    runtime.run_native_events(
        vec![(MAIN, RinchNativeEvent::SetWindowSize(640.0, 60.0))],
        &NoOsLoop,
    );
    assert_eq!(requests(&surface), vec![logical(640.0, 60.0)]);
    assert_eq!(crate::window::window_physical_size(), (960, 90));
    assert_eq!(crate::window::window_size(), (640.0, 60.0));
}

/// Item 4: a debug screenshot taken inside the drain, after a
/// `SetWindowSize` in the same drain, is painted at the new size: the turn
/// staged so far is sent first, and its size handed to the resize path.
#[test]
fn a_screenshot_in_the_drain_is_painted_at_the_size_asked_before_it() {
    let (mut runtime, surface) = live_main(1.0);
    // What the drain's `SetWindowSize` arm does on a live layer surface.
    assert!(runtime.main.stage_layer_size(logical(800.0, 72.0)));
    runtime.ready_for_screenshot(&NoOsLoop);
    assert_eq!(requests(&surface), vec![logical(800.0, 72.0)]);
    assert_eq!(
        runtime.main.window.as_ref().unwrap().logical_size(),
        (800, 72),
        "what paint draws at"
    );
    assert_eq!(crate::window::window_physical_size(), (800, 72));
    // Nothing is left for the drain's end to send again.
    runtime.run_native_events(Vec::new(), &NoOsLoop);
    assert_eq!(requests(&surface).len(), 1);
}

/// Item 3: a size staged for a surface that is hidden before the drain
/// sends it is what the window comes back at, and what
/// `current_window_size()` reports while it is hidden.
#[test]
fn a_staged_size_dropped_by_hiding_is_the_published_size() {
    let _guard = window_geometry::published_test_guard();
    let mut runtime = RinchRuntime::new("t", 800, 40, component, &[]);
    runtime.main.layer = Some(LayerState::app_placed());
    assert!(runtime.main.stage_layer_size(logical(800.0, 60.0)));
    runtime.hide_window();
    assert_eq!(runtime.main.saved_geometry.size, Some(logical(800.0, 60.0)));
    assert_eq!(crate::windows::current_window_size(), Some((800.0, 60.0)));
}

thread_local! {
    static ROW: std::cell::RefCell<Option<NodeHandle>> = const { std::cell::RefCell::new(None) };
}

/// 100 x 20 boxes in a wrapping row: ten in one row on a 1000-wide output,
/// six to a row (two rows) when the row is narrowed to 650.
fn wrapping_boxes(scope: &mut RenderScope) -> NodeHandle {
    let row = scope.create_element("div");
    row.set_attribute("style", "display: flex; flex-wrap: wrap");
    for _ in 0..10 {
        let cell = scope.create_element("div");
        cell.set_attribute("style", "width: 100px; height: 20px; flex: none");
        row.append_child(&cell);
    }
    ROW.with(|r| *r.borrow_mut() = Some(row.clone()));
    row
}

fn top_left(left_margin: i32) -> LayerSurface {
    LayerSurface {
        anchor: LayerAnchor {
            top: true,
            left: true,
            ..LayerAnchor::NONE
        },
        margins: LayerMargins {
            left: left_margin,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Item 2: a content-sized layer surface (#2552) whose configuration
/// changes in a turn is measured then, and its new size goes out in the
/// turn's one request with the layer changes — not in a request of its own
/// after the drain (`follow_content_size`), a second commit.
///
/// A unit test cannot offer the layer shell without a `TestApp` (the
/// published window roles are the process's), so the content's largest size
/// here does not follow the new margins: the content changes in the same
/// turn instead, which takes the same path (`content_size_to_ask` after the
/// configuration's new negotiation). The new margins narrowing it is checked
/// live on a compositor (#2829's PR).
#[test]
fn a_content_sized_surface_asks_its_new_size_with_the_layer_changes() {
    crate::outputs::publish(
        vec![crate::outputs::Output {
            id: crate::outputs::OutputId::from_raw(1),
            name: Some("DP-1".into()),
            position: (0.0, 0.0),
            size: (1000.0, 600.0),
            scale: 1.0,
        }],
        crate::outputs::OutputTracking::Live,
    );
    let props = WindowProps {
        size_to_content: Some(ContentSizing::new()),
        wayland_layer: Some(top_left(0)),
        ..Default::default()
    };
    let mut runtime = super::entry::build_runtime(wrapping_boxes, &props, None, &[]);
    runtime.main.mount_document_before_window(1.0);
    let made = runtime.main.start_surface_negotiation();
    assert_eq!(made, Some((1000, 20)), "one row on the whole output");
    let surface = go_live(&mut runtime.main, (1000, 20), 1.0);
    // The compositor's grant of the size it was made at.
    runtime.window_event_for(
        WindowRole::Main,
        &NoOsLoop,
        WindowEvent::SurfaceResized(PhysicalSize::new(1000, 20)),
    );
    // The turn: the content narrows to 650 and the margins change.
    ROW.with(|r| {
        r.borrow()
            .as_ref()
            .unwrap()
            .set_attribute("style", "display: flex; flex-wrap: wrap; max-width: 650px")
    });
    runtime.run_native_events(
        vec![(
            MAIN,
            RinchNativeEvent::SetWindowLayer(Box::new(top_left(350))),
        )],
        &NoOsLoop,
    );
    assert_eq!(
        requests(&surface),
        vec![logical(600.0, 40.0)],
        "the new size, in the turn's one request"
    );
    assert_eq!(runtime.main.take_layer_turn(), LayerTurn::Nothing);
    // Nothing left to ask after the drain.
    runtime.follow_content_size(WindowRole::Main, &NoOsLoop);
    assert_eq!(requests(&surface).len(), 1);
    crate::outputs::publish(Vec::new(), crate::outputs::OutputTracking::NotListed);
}

/// Item 2's control: a turn that changes no layer state stages nothing for
/// a content-sized surface; its new size is `follow_content_size`'s, after
/// the drain, as before.
#[test]
fn a_turn_without_layer_changes_leaves_the_content_size_to_follow() {
    let props = WindowProps {
        size_to_content: Some(ContentSizing::new()),
        wayland_layer: Some(top_left(0)),
        width: 1000,
        height: 600,
        ..Default::default()
    };
    let mut runtime = super::entry::build_runtime(wrapping_boxes, &props, None, &[]);
    runtime.main.mount_document_before_window(1.0);
    assert_eq!(runtime.main.start_surface_negotiation(), Some((1000, 20)));
    let surface = go_live(&mut runtime.main, (1000, 20), 1.0);
    runtime.window_event_for(
        WindowRole::Main,
        &NoOsLoop,
        WindowEvent::SurfaceResized(PhysicalSize::new(1000, 20)),
    );
    ROW.with(|r| {
        r.borrow()
            .as_ref()
            .unwrap()
            .set_attribute("style", "display: flex; flex-wrap: wrap; max-width: 650px")
    });
    runtime.run_native_events(Vec::new(), &NoOsLoop);
    assert!(requests(&surface).is_empty(), "nothing in the drain");
    runtime.follow_content_size(WindowRole::Main, &NoOsLoop);
    assert_eq!(requests(&surface), vec![logical(600.0, 40.0)]);
}

/// Puts the thread's zoom back for the next test it runs.
struct ZoomReset;
impl Drop for ZoomReset {
    fn drop(&mut self) {
        crate::zoom::set_zoom(1.0);
    }
}

/// Review of #2893 against #2861: at zoom 1.5 the content-sized surface's
/// size staged with the layer changes is the content's CSS size times the
/// zoom (winit's logical size is the display's), as `follow_content_size`
/// asks it. Without the zoom it is 1/1.5 too small and the panel is cut.
#[test]
fn a_content_sized_surface_at_a_zoom_asks_its_zoomed_size_with_the_layer_changes() {
    let _reset = ZoomReset;
    assert_eq!(crate::zoom::set_zoom(1.5), 1.5);
    let props = WindowProps {
        size_to_content: Some(ContentSizing::new()),
        wayland_layer: Some(top_left(0)),
        width: 1500,
        height: 600,
        ..Default::default()
    };
    let mut runtime = super::entry::build_runtime(wrapping_boxes, &props, None, &[]);
    runtime.main.mount_document_before_window(1.5);
    // 1500 display px at zoom 1.5 is 1000 CSS px: one row.
    assert_eq!(runtime.main.start_surface_negotiation(), Some((1000, 20)));
    let surface = go_live(&mut runtime.main, (1500, 30), 1.0);
    runtime.window_event_for(
        WindowRole::Main,
        &NoOsLoop,
        WindowEvent::SurfaceResized(PhysicalSize::new(1500, 30)),
    );
    ROW.with(|r| {
        r.borrow()
            .as_ref()
            .unwrap()
            .set_attribute("style", "display: flex; flex-wrap: wrap; max-width: 650px")
    });
    runtime.run_native_events(
        vec![(
            MAIN,
            RinchNativeEvent::SetWindowLayer(Box::new(top_left(350))),
        )],
        &NoOsLoop,
    );
    // 600 x 40 CSS px, at zoom 1.5.
    assert_eq!(requests(&surface), vec![logical(900.0, 60.0)]);
}
