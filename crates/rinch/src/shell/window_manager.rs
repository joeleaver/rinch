//! Window manager - tracks and manages multiple windows.

use std::collections::HashMap;
use std::sync::Arc;
use std::task::Waker;
use std::time::Instant;

use anyrender_vello::VelloWindowRenderer;
use anyrender::WindowRenderer;
use peniko::Color;

use super::transparent_renderer::{TransparentRendererOptions, TransparentWindowRenderer};
use blitz_dom::{Document, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use blitz_traits::events::{
    BlitzMouseButtonEvent, BlitzWheelDelta, BlitzWheelEvent, MouseEventButton, MouseEventButtons,
    UiEvent,
};
use futures_util::task::ArcWake;
use rinch_core::element::WindowProps;
use rinch_core::events::EventHandlerId;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Modifiers, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Theme, Window, WindowAttributes, WindowId};

#[cfg(target_os = "windows")]
use winit::platform::windows::WindowAttributesExtWindows;

use super::devtools::DevToolsState;
use super::runtime::{ElementLayout, HoveredElementInfo, RinchEvent};

/// Renderer wrapper that supports both standard and transparent rendering.
pub enum RinchWindowRenderer {
    /// Standard Vello renderer (Vulkan backend, opaque).
    Standard(VelloWindowRenderer),
    /// Transparent renderer (DX12 + DirectComposition).
    Transparent(TransparentWindowRenderer),
}

impl RinchWindowRenderer {
    fn is_active(&self) -> bool {
        match self {
            RinchWindowRenderer::Standard(r) => r.is_active(),
            RinchWindowRenderer::Transparent(r) => r.is_active(),
        }
    }

    fn resume(&mut self, window: Arc<Window>, width: u32, height: u32) {
        match self {
            RinchWindowRenderer::Standard(r) => r.resume(window, width, height),
            RinchWindowRenderer::Transparent(r) => r.resume(window, width, height),
        }
    }

    fn suspend(&mut self) {
        match self {
            RinchWindowRenderer::Standard(r) => r.suspend(),
            RinchWindowRenderer::Transparent(r) => r.suspend(),
        }
    }

    fn set_size(&mut self, width: u32, height: u32) {
        match self {
            RinchWindowRenderer::Standard(r) => r.set_size(width, height),
            RinchWindowRenderer::Transparent(r) => r.set_size(width, height),
        }
    }

    fn render<F>(&mut self, draw_fn: F)
    where
        F: for<'a, 'b> FnOnce(&'a mut anyrender_vello::VelloScenePainter<'b, 'b>),
    {
        match self {
            RinchWindowRenderer::Standard(r) => r.render(draw_fn),
            RinchWindowRenderer::Transparent(r) => r.render(draw_fn),
        }
    }
}

/// A window managed by rinch with integrated blitz rendering.
pub struct ManagedWindow {
    /// The blitz document being rendered.
    pub doc: Box<dyn Document>,
    /// The window renderer (standard or transparent).
    pub renderer: RinchWindowRenderer,
    /// Waker for async document updates.
    pub waker: Option<Waker>,
    /// The underlying winit window.
    pub window: Arc<Window>,
    /// Event loop proxy for sending events.
    pub proxy: EventLoopProxy<RinchEvent>,
    /// The props used to create this window.
    pub props: WindowProps,
    /// Keyboard modifier state.
    pub keyboard_modifiers: Modifiers,
    /// Mouse button state.
    pub buttons: MouseEventButtons,
    /// Current mouse position.
    pub mouse_pos: (f32, f32),
    /// Animation start time.
    pub animation_timer: Option<Instant>,
    /// Window visibility state.
    pub is_visible: bool,
    /// DevTools state for this window.
    pub devtools: DevToolsState,
}

impl ManagedWindow {
    /// Create a new managed window.
    pub fn new(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<RinchEvent>,
        props: WindowProps,
        html_content: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        tracing::info!(
            "Creating window '{}': borderless={}, transparent={}, decorations={}",
            props.title,
            props.borderless,
            props.transparent,
            !props.borderless
        );

        // Build window attributes
        let mut attrs = WindowAttributes::default()
            .with_title(&props.title)
            .with_inner_size(LogicalSize::new(props.width, props.height))
            .with_resizable(props.resizable)
            .with_decorations(!props.borderless)
            .with_transparent(props.transparent)
            .with_visible(props.visible);

        if let (Some(x), Some(y)) = (props.x, props.y) {
            attrs = attrs.with_position(LogicalPosition::new(x, y));
        }

        // On Windows, transparent windows need WS_EX_NOREDIRECTIONBITMAP for true
        // desktop transparency with DirectComposition
        #[cfg(target_os = "windows")]
        if props.transparent {
            attrs = attrs.with_no_redirection_bitmap(true);
            tracing::info!("Enabled no_redirection_bitmap for transparent window");
        }

        // Create winit window
        let window = Arc::new(event_loop.create_window(attrs)?);

        // Log actual window state after creation
        tracing::info!(
            "Window created - is_decorated: {:?}, transparent: {:?}",
            window.is_decorated(),
            props.transparent // winit doesn't have is_transparent()
        );

        // Set up viewport
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let theme = window.theme().unwrap_or(Theme::Light);
        let color_scheme = match theme {
            Theme::Light => ColorScheme::Light,
            Theme::Dark => ColorScheme::Dark,
        };
        let viewport = Viewport::new(size.width, size.height, scale, color_scheme);

        // Create document config
        let config = DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        };

        // Parse HTML into document
        let doc: Box<dyn Document> = Box::new(HtmlDocument::from_html(&html_content, config));

        // Set the document title from HTML if present
        {
            let inner = doc.inner();
            if let Some(title_node) = inner.find_title_node() {
                let title = title_node.text_content();
                window.set_title(&title);
            }
        }

        // Create renderer - use transparent renderer for transparent windows on Windows
        let renderer = if props.transparent && cfg!(target_os = "windows") {
            RinchWindowRenderer::Transparent(TransparentWindowRenderer::with_options(
                TransparentRendererOptions {
                    // Fully transparent base for true window transparency
                    base_color: Color::TRANSPARENT,
                    transparent: true,
                    ..Default::default()
                },
            ))
        } else {
            RinchWindowRenderer::Standard(VelloWindowRenderer::new())
        };

        let is_visible = window.is_visible().unwrap_or(true);

        Ok(Self {
            doc,
            renderer,
            waker: None,
            window,
            proxy,
            props,
            keyboard_modifiers: Default::default(),
            buttons: MouseEventButtons::None,
            mouse_pos: (0.0, 0.0),
            animation_timer: None,
            is_visible,
            devtools: DevToolsState::new(),
        })
    }

    /// Get the window ID.
    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    /// Request a redraw.
    pub fn request_redraw(&self) {
        if self.renderer.is_active() {
            self.window.request_redraw();
        }
    }

    /// Get current animation time.
    fn current_animation_time(&mut self) -> f64 {
        match &self.animation_timer {
            Some(start) => Instant::now().duration_since(*start).as_secs_f64(),
            None => {
                self.animation_timer = Some(Instant::now());
                0.0
            }
        }
    }

    /// Resume rendering (called when window becomes active).
    pub fn resume(&mut self) {
        let window_id = self.window_id();
        let animation_time = self.current_animation_time();

        let mut inner = self.doc.inner_mut();
        inner.resolve(animation_time);

        let (width, height) = inner.viewport().window_size;
        let scale = inner.viewport().scale_f64();

        self.renderer.resume(self.window.clone(), width, height);
        if !self.renderer.is_active() {
            tracing::error!("Renderer failed to resume");
            return;
        }

        self.renderer.render(|scene| paint_scene(scene, &inner, scale, width, height));

        drop(inner);

        // Set up waker for async updates
        self.waker = Some(create_waker(&self.proxy, window_id));
    }

    /// Suspend rendering.
    pub fn suspend(&mut self) {
        self.waker = None;
        self.renderer.suspend();
    }

    /// Poll for document updates.
    pub fn poll(&mut self) -> bool {
        if let Some(waker) = &self.waker {
            let cx = std::task::Context::from_waker(waker);
            if self.doc.poll(Some(cx)) {
                self.request_redraw();
                return true;
            }
        }
        false
    }

    /// Redraw the window.
    pub fn redraw(&mut self) {
        let animation_time = self.current_animation_time();
        let is_visible = self.is_visible;

        let mut inner = self.doc.inner_mut();
        inner.resolve(animation_time);

        let (width, height) = inner.viewport().window_size;
        let scale = inner.viewport().scale_f64();
        let is_animating = inner.is_animating();

        self.renderer.render(|scene| paint_scene(scene, &inner, scale, width, height));

        drop(inner);

        if is_visible && is_animating {
            self.request_redraw();
        }
    }

    /// Handle a winit window event.
    pub fn handle_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                self.redraw();
            }
            WindowEvent::Occluded(is_occluded) => {
                self.is_visible = !is_occluded;
                if self.is_visible {
                    self.request_redraw();
                }
            }
            WindowEvent::Resized(physical_size) => {
                let mut inner = self.doc.inner_mut();
                inner.viewport_mut().window_size = (physical_size.width, physical_size.height);
                let (width, height) = inner.viewport().window_size;
                drop(inner);
                if width > 0 && height > 0 {
                    self.renderer.set_size(width, height);
                    self.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let mut inner = self.doc.inner_mut();
                inner.viewport_mut().set_hidpi_scale(scale_factor as f32);
                drop(inner);
                self.request_redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                let color_scheme = match theme {
                    Theme::Light => ColorScheme::Light,
                    Theme::Dark => ColorScheme::Dark,
                };
                let mut inner = self.doc.inner_mut();
                inner.viewport_mut().color_scheme = color_scheme;
            }
            WindowEvent::ModifiersChanged(new_state) => {
                self.keyboard_modifiers = new_state;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(key_code) = event.physical_key else {
                    return;
                };

                if event.state.is_pressed() {
                    let ctrl = self.keyboard_modifiers.state().control_key();
                    let meta = self.keyboard_modifiers.state().super_key();
                    let alt = self.keyboard_modifiers.state().alt_key();
                    let shift = self.keyboard_modifiers.state().shift_key();

                    // Ctrl/Cmd keyboard shortcuts for zoom
                    if ctrl || meta {
                        match key_code {
                            KeyCode::Equal => {
                                self.doc.inner_mut().viewport_mut().zoom_by(0.1);
                                self.request_redraw();
                            }
                            KeyCode::Minus => {
                                self.doc.inner_mut().viewport_mut().zoom_by(-0.1);
                                self.request_redraw();
                            }
                            KeyCode::Digit0 => {
                                self.doc.inner_mut().viewport_mut().set_zoom(1.0);
                                self.request_redraw();
                            }
                            _ => {}
                        }
                    }

                    // Alt keyboard shortcuts for dev tools
                    if alt {
                        match key_code {
                            KeyCode::KeyD => {
                                self.doc.inner_mut().devtools_mut().toggle_show_layout();
                                self.request_redraw();
                            }
                            KeyCode::KeyI => {
                                // Toggle hover highlight (inspect mode)
                                self.doc.inner_mut().devtools_mut().toggle_highlight_hover();
                                self.devtools.toggle_inspect_mode();
                                tracing::info!("Inspect mode: {}", self.devtools.inspect_mode);
                                self.request_redraw();
                            }
                            KeyCode::KeyT => {
                                self.doc.inner().print_taffy_tree();
                            }
                            _ => {}
                        }
                    }

                    // F12 to toggle devtools window
                    if key_code == KeyCode::F12 {
                        let _ = self.proxy.send_event(RinchEvent::ToggleDevTools {
                            source_window: self.window_id(),
                        });
                    }

                    // Send keyboard shortcut to runtime for menu accelerator matching
                    let _ = self.proxy.send_event(RinchEvent::KeyboardShortcut {
                        ctrl,
                        meta,
                        alt,
                        shift,
                        key: key_code,
                    });
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos: winit::dpi::LogicalPosition<f32> = position.to_logical(self.window.scale_factor());
                self.mouse_pos = (pos.x, pos.y);

                let event = UiEvent::MouseMove(BlitzMouseButtonEvent {
                    x: pos.x,
                    y: pos.y,
                    button: Default::default(),
                    buttons: self.buttons,
                    mods: Default::default(),
                });
                self.doc.handle_ui_event(event);

                // If in inspect mode, send hovered element info to DevTools
                if self.devtools.inspect_mode {
                    let element_info = self.get_hovered_element_info();
                    let _ = self.proxy.send_event(RinchEvent::UpdateDevToolsHover { element_info });
                }

                self.request_redraw();
            }
            WindowEvent::MouseInput { button, state, .. } => {
                let button = match button {
                    MouseButton::Left => MouseEventButton::Main,
                    MouseButton::Right => MouseEventButton::Secondary,
                    MouseButton::Middle => MouseEventButton::Auxiliary,
                    _ => return,
                };

                match state {
                    ElementState::Pressed => self.buttons |= button.into(),
                    ElementState::Released => self.buttons ^= button.into(),
                }

                let event_data = BlitzMouseButtonEvent {
                    x: self.mouse_pos.0,
                    y: self.mouse_pos.1,
                    button,
                    buttons: self.buttons,
                    mods: Default::default(),
                };

                let event = match state {
                    ElementState::Pressed => UiEvent::MouseDown(event_data),
                    ElementState::Released => UiEvent::MouseUp(event_data),
                };
                self.doc.handle_ui_event(event);
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let blitz_delta = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        BlitzWheelDelta::Lines(x as f64, y as f64)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        BlitzWheelDelta::Pixels(pos.x, pos.y)
                    }
                };

                let event = BlitzWheelEvent {
                    delta: blitz_delta,
                    x: self.mouse_pos.0,
                    y: self.mouse_pos.1,
                    button: MouseEventButton::Main,
                    buttons: self.buttons,
                    mods: Default::default(),
                };

                self.doc.handle_ui_event(UiEvent::Wheel(event));
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// Update the window's HTML content and re-render.
    pub fn update_content(&mut self, html_content: String) {
        // Get current viewport settings
        let (viewport, scale) = {
            let inner = self.doc.inner();
            (inner.viewport().clone(), inner.viewport().scale_f64())
        };

        // Create new document config with current viewport
        let config = DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        };

        // Create new document with updated HTML
        self.doc = Box::new(HtmlDocument::from_html(&html_content, config));

        // Re-resolve and redraw
        let animation_time = self.current_animation_time();
        {
            let mut inner = self.doc.inner_mut();
            inner.resolve(animation_time);
        }

        // Render the updated content
        let inner = self.doc.inner();
        let (width, height) = inner.viewport().window_size;
        self.renderer.render(|scene| paint_scene(scene, &inner, scale, width, height));
    }

    /// Get information about the element under the current mouse position.
    ///
    /// Returns element info for DevTools display.
    pub fn get_hovered_element_info(&self) -> Option<HoveredElementInfo> {
        let inner = self.doc.inner();
        let hit_result = inner.hit(self.mouse_pos.0, self.mouse_pos.1)?;
        let node_id = hit_result.node_id;

        let node = inner.get_node(node_id)?;
        let element = node.element_data()?;

        // Get tag name
        let tag_name = element.name.local.to_string();

        // Get id and classes
        let mut id = None;
        let mut classes = None;
        for attr in element.attrs() {
            let name = attr.name.local.as_ref();
            if name == "id" {
                id = Some(attr.value.to_string());
            } else if name == "class" {
                classes = Some(attr.value.to_string());
            }
        }

        // Get layout info - hit_result has x, y; get size from the node's layout
        let width = node.final_layout.size.width;
        let height = node.final_layout.size.height;

        let layout = ElementLayout {
            x: hit_result.x,
            y: hit_result.y,
            width,
            height,
        };

        // Extract computed styles from the node
        let mut styles = Vec::new();

        // Get size
        styles.push((
            "size".to_string(),
            format!("{:.0} × {:.0}", width, height),
        ));

        // Get padding from layout
        let padding = &node.final_layout.padding;
        if padding.top > 0.0 || padding.right > 0.0 || padding.bottom > 0.0 || padding.left > 0.0 {
            styles.push((
                "padding".to_string(),
                format!(
                    "{:.0} {:.0} {:.0} {:.0}",
                    padding.top, padding.right, padding.bottom, padding.left
                ),
            ));
        }

        // Get margin from layout
        let margin = &node.final_layout.margin;
        if margin.top > 0.0 || margin.right > 0.0 || margin.bottom > 0.0 || margin.left > 0.0 {
            styles.push((
                "margin".to_string(),
                format!(
                    "{:.0} {:.0} {:.0} {:.0}",
                    margin.top, margin.right, margin.bottom, margin.left
                ),
            ));
        }

        // Get border from layout
        let border = &node.final_layout.border;
        if border.top > 0.0 || border.right > 0.0 || border.bottom > 0.0 || border.left > 0.0 {
            styles.push((
                "border-width".to_string(),
                format!(
                    "{:.0} {:.0} {:.0} {:.0}",
                    border.top, border.right, border.bottom, border.left
                ),
            ));
        }

        Some(HoveredElementInfo {
            tag_name,
            id,
            classes,
            styles,
            layout,
        })
    }

    /// Get the event handler ID of the element under the current mouse position.
    ///
    /// Returns `Some(id)` if there's an element with a `data-rid` attribute at the
    /// current mouse position, `None` otherwise.
    pub fn get_clicked_handler(&self) -> Option<EventHandlerId> {
        let inner = self.doc.inner();

        // Hit test at current mouse position
        let hit_result = inner.hit(self.mouse_pos.0, self.mouse_pos.1)?;
        let node_id = hit_result.node_id;

        // Walk up the tree looking for a data-rid attribute
        let mut current = Some(node_id);
        while let Some(id) = current {
            if let Some(node) = inner.get_node(id) {
                if let Some(element) = node.element_data() {
                    // Check all attributes for data-rid
                    for attr in element.attrs() {
                        if attr.name.local.as_ref() == "data-rid" {
                            if let Ok(rid) = attr.value.parse::<usize>() {
                                return Some(EventHandlerId(rid));
                            }
                        }
                    }
                }
                current = node.parent;
            } else {
                break;
            }
        }

        None
    }

    /// Check if the element under the current mouse position should trigger window dragging.
    ///
    /// Returns `true` if there's an element with `data-drag-window` attribute at the
    /// current mouse position.
    pub fn should_drag_window(&self) -> bool {
        let inner = self.doc.inner();

        // Hit test at current mouse position
        let Some(hit_result) = inner.hit(self.mouse_pos.0, self.mouse_pos.1) else {
            return false;
        };
        let node_id = hit_result.node_id;

        // Walk up the tree looking for a data-drag-window attribute
        let mut current = Some(node_id);
        while let Some(id) = current {
            if let Some(node) = inner.get_node(id) {
                if let Some(element) = node.element_data() {
                    // Check for data-drag-window or draggable attribute
                    for attr in element.attrs() {
                        let name = attr.name.local.as_ref();
                        if name == "data-drag-window" || name == "draggable" {
                            return true;
                        }
                    }
                }
                current = node.parent;
            } else {
                break;
            }
        }

        false
    }

    /// Initiate window dragging.
    pub fn start_drag(&self) {
        if let Err(e) = self.window.drag_window() {
            tracing::warn!("Failed to start window drag: {:?}", e);
        }
    }
}

/// Manages all open windows in the application.
pub struct WindowManager {
    windows: HashMap<WindowId, ManagedWindow>,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }

    /// Create a new window.
    pub fn create_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<RinchEvent>,
        props: WindowProps,
        html_content: String,
    ) -> Result<WindowId, Box<dyn std::error::Error>> {
        let window = ManagedWindow::new(event_loop, proxy, props, html_content)?;
        let window_id = window.window_id();
        self.windows.insert(window_id, window);
        Ok(window_id)
    }

    /// Get a window by its ID.
    pub fn get(&self, id: WindowId) -> Option<&ManagedWindow> {
        self.windows.get(&id)
    }

    /// Get a mutable reference to a window by its ID.
    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut ManagedWindow> {
        self.windows.get_mut(&id)
    }

    /// Remove and close a window.
    pub fn close_window(&mut self, id: WindowId) -> Option<ManagedWindow> {
        self.windows.remove(&id)
    }

    /// Check if any windows are still open.
    pub fn has_windows(&self) -> bool {
        !self.windows.is_empty()
    }

    /// Resume all windows.
    pub fn resume_all(&mut self) {
        for window in self.windows.values_mut() {
            window.resume();
        }
    }

    /// Suspend all windows.
    pub fn suspend_all(&mut self) {
        for window in self.windows.values_mut() {
            window.suspend();
        }
    }

    /// Iterate over all windows.
    pub fn windows_iter(&self) -> impl Iterator<Item = (&WindowId, &ManagedWindow)> {
        self.windows.iter()
    }

    /// Get all window IDs.
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.windows.keys().copied().collect()
    }
}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a waker that sends poll events to the event loop.
fn create_waker(proxy: &EventLoopProxy<RinchEvent>, id: WindowId) -> Waker {
    struct WakerHandle {
        proxy: EventLoopProxy<RinchEvent>,
        id: WindowId,
    }

    impl ArcWake for WakerHandle {
        fn wake_by_ref(arc_self: &Arc<Self>) {
            let _ = arc_self.proxy.send_event(RinchEvent::Poll {
                window_id: arc_self.id,
            });
        }
    }

    futures_util::task::waker(Arc::new(WakerHandle {
        proxy: proxy.clone(),
        id,
    }))
}
