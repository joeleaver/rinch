//! Window manager - tracks and manages multiple windows.

use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::Arc;
use std::task::Waker;
use std::time::Instant;

use anyrender_vello::VelloWindowRenderer;
use anyrender::WindowRenderer;
use peniko::Color;

use super::transparent_renderer::{TransparentRendererOptions, TransparentWindowRenderer};
use blitz_dom::{BaseDocument, Document, DocumentConfig, EventDriver, EventHandler, LocalName, QualName};
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, DomEvent, DomEventData, EventState,
    MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails, UiEvent,
};
use super::convert_events::{winit_ime_to_blitz, winit_key_event_to_blitz, winit_key_to_key_event_data};
use futures_util::task::ArcWake;
use rinch_core::element::WindowProps;
use rinch_core::events::{dispatch_input_event, EventHandlerId};

// Deferred input events: queued inside RinchEventHandler (where the document is borrowed)
// and drained after the borrow is released to avoid RefCell reentrancy panics.
thread_local! {
    static PENDING_INPUT_EVENTS: RefCell<Vec<(EventHandlerId, String)>> = RefCell::new(Vec::new());
}

fn queue_input_event(id: EventHandlerId, value: String) {
    PENDING_INPUT_EVENTS.with(|q| q.borrow_mut().push((id, value)));
}

fn drain_pending_input_events() {
    let events: Vec<_> = PENDING_INPUT_EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if !events.is_empty() {
        tracing::info!("Draining {} deferred input events", events.len());
    }
    for (id, value) in events {
        tracing::info!("  Dispatching deferred input: id={}, value_len={}", id.0, value.len());
        let result = dispatch_input_event(id, value);
        tracing::info!("  Dispatch result: {}", result);
    }
}
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Modifiers, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, ResizeDirection, Theme, Window, WindowAttributes, WindowId};

#[cfg(target_os = "windows")]
use winit::platform::windows::WindowAttributesExtWindows;

use super::devtools::DevToolsState;
use super::types::{ElementLayout, HoveredElementInfo, RinchEvent};

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

/// Event handler that captures DOM events and routes them to rinch callbacks.
pub struct RinchEventHandler;

impl EventHandler for RinchEventHandler {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        _event_state: &mut EventState,
    ) {
        // Log all events for debugging
        tracing::debug!("RinchEventHandler: event={:?}, target={}", event.data.name(), event.target);

        // Handle Input events - find data-oninput attribute and dispatch
        if let DomEventData::Input(ref input_event) = event.data {
            tracing::debug!("INPUT EVENT: value='{}', chain={:?}", input_event.value, chain);
            let inner = doc.inner();
            // Walk the event chain looking for a data-oninput attribute
            for &node_id in chain {
                if let Some(node) = inner.get_node(node_id) {
                    if let Some(attrs) = node.attrs() {
                        for attr in attrs {
                            tracing::trace!("  Checking attr: {}={}", attr.name.local.as_ref(), attr.value);
                            if attr.name.local.as_ref() == "data-oninput" {
                                if let Ok(handler_id) = attr.value.parse::<usize>() {
                                    tracing::debug!("  Found handler {}, deferring dispatch", handler_id);
                                    queue_input_event(
                                        EventHandlerId(handler_id),
                                        input_event.value.clone(),
                                    );
                                    return;
                                }
                            }
                        }
                    }
                }
            }
            tracing::debug!("  No data-oninput handler found in chain");
        }
    }
}

/// Shared reference to a blitz Document for use with BlitzDocumentAdapter.
pub type SharedDocument = Rc<RefCell<Box<dyn Document>>>;

/// A window managed by rinch with integrated blitz rendering.
pub struct ManagedWindow {
    /// The blitz document being rendered (shared for BlitzDocumentAdapter).
    pub doc: SharedDocument,
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
    /// Current resize direction for cursor feedback (None when not over resize edge).
    current_resize_direction: Option<ResizeDirection>,
    /// Smooth scroll velocity (pixels per frame).
    scroll_velocity: (f64, f64),
    /// Last scroll animation time.
    last_scroll_time: Option<Instant>,
    /// Performance stats tracking.
    pub perf_stats: PerfStats,
    /// Mapping from reactive text ID to blitz text node ID for fine-grained updates.
    /// This enables direct `set_node_text` calls without HTML regeneration.
    /// Maps reactive_id -> (span_id, text_node_id)
    /// We update the text node but mark the span dirty for re-render
    reactive_text_map: HashMap<usize, (usize, usize)>,
    /// Mapping from reactive style ID to (blitz element node ID, style property).
    /// This enables direct style attribute updates without HTML regeneration.
    reactive_style_map: HashMap<usize, (usize, String)>,
    /// Mapping from reactive attribute ID to (blitz element node ID, attribute name).
    /// This enables direct attribute updates without HTML regeneration.
    reactive_attr_map: HashMap<usize, (usize, String)>,
    /// Mapping from reactive boolean attribute ID to (blitz element node ID, attribute name).
    /// This enables direct boolean attribute updates (add/remove) without HTML regeneration.
    reactive_bool_attr_map: HashMap<usize, (usize, String)>,
    /// DevTools overlay element ID (if created).
    devtools_element_id: Option<usize>,
}

/// Performance statistics for a window.
pub struct PerfStats {
    /// Whether to show the performance overlay.
    pub show_overlay: bool,
    /// Last frame timestamps for FPS calculation.
    frame_times: std::collections::VecDeque<Instant>,
    /// Last render duration in ms.
    pub last_render_ms: f64,
    /// Render start time.
    render_start: Option<Instant>,
    /// Time of last frame (for idle detection).
    last_frame_time: Option<Instant>,
    /// Recent render times for averaging (in ms).
    render_times: std::collections::VecDeque<f64>,
}

impl Default for PerfStats {
    fn default() -> Self {
        Self {
            show_overlay: false,
            frame_times: std::collections::VecDeque::new(),
            last_render_ms: 0.0,
            render_start: None,
            last_frame_time: None,
            render_times: std::collections::VecDeque::new(),
        }
    }
}

impl PerfStats {
    /// Record the start of a render.
    pub fn begin_render(&mut self) {
        self.render_start = Some(Instant::now());
    }

    /// Record the end of a render and update stats.
    pub fn end_render(&mut self) {
        let now = Instant::now();

        if let Some(start) = self.render_start.take() {
            self.last_render_ms = start.elapsed().as_secs_f64() * 1000.0;

            // Track recent render times for averaging
            self.render_times.push_back(self.last_render_ms);
            while self.render_times.len() > 30 {
                self.render_times.pop_front();
            }
        }

        self.frame_times.push_back(now);
        self.last_frame_time = Some(now);

        // Keep only last 60 frames for averaging
        while self.frame_times.len() > 60 {
            self.frame_times.pop_front();
        }
    }

    /// Get current FPS based on recent frame times.
    /// Returns 0.0 if idle (no frames in last 500ms).
    pub fn fps(&self) -> f64 {
        // If no recent frames, return 0 (idle)
        if let Some(last) = self.last_frame_time {
            if last.elapsed().as_millis() > 500 {
                return 0.0; // Idle
            }
        }

        if self.frame_times.len() < 2 {
            return 0.0;
        }

        let oldest = self.frame_times.front().unwrap();
        let newest = self.frame_times.back().unwrap();
        let duration = newest.duration_since(*oldest).as_secs_f64();

        if duration > 0.0 {
            (self.frame_times.len() - 1) as f64 / duration
        } else {
            0.0
        }
    }

    /// Get average render time in milliseconds (how long each frame takes to render).
    /// This is more meaningful than frame time when the app is idle.
    pub fn avg_render_ms(&self) -> f64 {
        if self.render_times.is_empty() {
            return 0.0;
        }
        self.render_times.iter().sum::<f64>() / self.render_times.len() as f64
    }

    /// Get average frame time in milliseconds.
    pub fn avg_frame_ms(&self) -> f64 {
        let fps = self.fps();
        if fps > 0.0 {
            1000.0 / fps
        } else {
            0.0
        }
    }

    /// Check if the window is currently idle (no recent frames).
    pub fn is_idle(&self) -> bool {
        self.last_frame_time
            .map(|t| t.elapsed().as_millis() > 500)
            .unwrap_or(true)
    }
}

impl ManagedWindow {
    /// Create a new managed window.
    pub fn new(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<RinchEvent>,
        props: WindowProps,
        html_content: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        tracing::debug!(
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
            tracing::debug!("Enabled no_redirection_bitmap for transparent window");
        }

        // Create winit window
        let window = Arc::new(event_loop.create_window(attrs)?);

        // Log actual window state after creation
        tracing::debug!(
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

        // Parse HTML into document (this initializes fonts via parley/fontique)
        #[cfg(feature = "memory-profile")]
        let before_doc = super::memory_profile::MemorySnapshot::now();
        let doc: SharedDocument = Rc::new(RefCell::new(Box::new(HtmlDocument::from_html(&html_content, config))));
        #[cfg(feature = "memory-profile")]
        super::memory_profile::checkpoint_delta("After HtmlDocument::from_html (fonts/CSS)", before_doc);

        // Set the document title from HTML if present
        {
            let doc_ref = doc.borrow();
            let inner = doc_ref.inner();
            if let Some(title_node) = inner.find_title_node() {
                let title = title_node.text_content();
                window.set_title(&title);
            }
        }

        // Create renderer - use transparent renderer for transparent windows on Windows
        #[cfg(feature = "memory-profile")]
        let before_renderer = super::memory_profile::MemorySnapshot::now();
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
        #[cfg(feature = "memory-profile")]
        super::memory_profile::checkpoint_delta("After renderer creation", before_renderer);

        let is_visible = window.is_visible().unwrap_or(true);

        // Set up text selection callback so editor can use blitz's native selection
        let selection_doc = Rc::downgrade(&doc);
        rinch_core::events::set_selection_callback(move |action| {
            let Some(doc_rc) = selection_doc.upgrade() else { return Vec::new() };
            match action {
                rinch_core::SelectionAction::Set { anchor_node, anchor_offset, focus_node, focus_offset } => {
                    let mut doc_ref = doc_rc.borrow_mut();
                    let mut inner = doc_ref.inner_mut();
                    inner.set_text_selection(anchor_node, anchor_offset, focus_node, focus_offset);
                    Vec::new()
                }
                rinch_core::SelectionAction::ExtendToPoint { x, y } => {
                    let mut doc_ref = doc_rc.borrow_mut();
                    let mut inner = doc_ref.inner_mut();
                    inner.extend_text_selection_to_point(x, y);
                    Vec::new()
                }
                rinch_core::SelectionAction::Clear => {
                    let mut doc_ref = doc_rc.borrow_mut();
                    let mut inner = doc_ref.inner_mut();
                    inner.clear_text_selection();
                    Vec::new()
                }
                rinch_core::SelectionAction::QueryRanges => {
                    let doc_ref = doc_rc.borrow();
                    let inner = doc_ref.inner();
                    let raw_ranges = inner.get_text_selection_ranges();
                    // Map (node_id, start, end) to (block_index, start, end) using data-block-index
                    raw_ranges.into_iter().filter_map(|(node_id, start, end)| {
                        // Walk up from node to find data-block-index
                        let mut walk = Some(node_id);
                        while let Some(wid) = walk {
                            if let Some(wnode) = inner.get_node(wid) {
                                if let Some(el) = wnode.element_data() {
                                    for a in el.attrs() {
                                        if a.name.local.as_ref() == "data-block-index" {
                                            if let Ok(bi) = a.value.parse::<usize>() {
                                                return Some((bi, start, end));
                                            }
                                        }
                                    }
                                }
                                walk = wnode.parent;
                            } else {
                                break;
                            }
                        }
                        None
                    }).collect()
                }
            }
        });

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
            current_resize_direction: None,
            scroll_velocity: (0.0, 0.0),
            last_scroll_time: None,
            perf_stats: PerfStats::default(),
            reactive_text_map: HashMap::new(),
            reactive_style_map: HashMap::new(),
            reactive_attr_map: HashMap::new(),
            reactive_bool_attr_map: HashMap::new(),
            devtools_element_id: None,
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

    /// Handle a UI event with the custom RinchEventHandler to capture input events.
    fn handle_ui_event_with_handler(&mut self, event: UiEvent) {
        {
            let mut doc_ref = self.doc.borrow_mut();
            let mut driver = EventDriver::new(&mut **doc_ref, RinchEventHandler);
            // Wrap in catch_unwind to handle blitz panics from stale node references
            let result = catch_unwind(AssertUnwindSafe(|| {
                driver.handle_ui_event(event);
            }));
            if let Err(e) = result {
                tracing::warn!("handle_ui_event() panicked (likely stale node references): {:?}", e);
            }
        }
        // doc borrow is released — now safe to dispatch deferred input events
        // (handlers may mutate the DOM via signals/effects)
        drain_pending_input_events();
    }

    /// Tick the smooth scroll animation. Returns true if still animating.
    pub fn tick_scroll_animation(&mut self) -> bool {
        const DECEL_RATE: f64 = 8.0; // Deceleration rate (higher = faster stop)
        const MIN_VELOCITY: f64 = 0.5; // Stop when velocity is below this
        const TARGET_DT: f64 = 1.0 / 60.0; // Target 60fps for consistent feel

        let (vx, vy) = self.scroll_velocity;

        // Check if we have any velocity to animate
        if vx.abs() < MIN_VELOCITY && vy.abs() < MIN_VELOCITY {
            self.scroll_velocity = (0.0, 0.0);
            self.last_scroll_time = None;
            return false;
        }

        // Calculate delta time
        let now = Instant::now();
        let dt = self.last_scroll_time
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(TARGET_DT)
            .min(0.1); // Cap at 100ms to prevent huge jumps
        self.last_scroll_time = Some(now);

        // Apply scroll based on velocity and delta time
        let scroll_x = vx * dt * 60.0; // Scale to ~60fps equivalent
        let scroll_y = vy * dt * 60.0;

        if scroll_x.abs() > 0.01 || scroll_y.abs() > 0.01 {
            let (x, y) = self.mouse_pos;
            let event = blitz_traits::events::BlitzWheelEvent {
                delta: blitz_traits::events::BlitzWheelDelta::Pixels(scroll_x, scroll_y),
                coords: PointerCoords {
                    page_x: x,
                    page_y: y,
                    screen_x: x,
                    screen_y: y,
                    client_x: x,
                    client_y: y,
                },
                buttons: self.buttons,
                mods: Default::default(),
            };
            self.handle_ui_event_with_handler(UiEvent::Wheel(event));
        }

        // Apply exponential decay based on delta time
        let decay = (-DECEL_RATE * dt).exp();
        self.scroll_velocity.0 *= decay;
        self.scroll_velocity.1 *= decay;

        true // Still animating
    }

    /// Check if scroll animation is active.
    pub fn is_scroll_animating(&self) -> bool {
        self.scroll_velocity.0.abs() > 0.5 || self.scroll_velocity.1.abs() > 0.5
    }

    /// Toggle the DevTools overlay visibility.
    pub fn toggle_devtools(&mut self) {
        self.devtools.toggle();

        // Ensure devtools element exists
        if self.devtools_element_id.is_none() {
            self.create_devtools_overlay();
        }

        // Toggle visibility
        if let Some(devtools_id) = self.devtools_element_id {
            let display = if self.devtools.visible { "flex" } else { "none" };
            let mut doc_ref = self.doc.borrow_mut();
            let mut inner = doc_ref.inner_mut();
            let mut mutator = inner.mutate();
            mutator.set_style_property(devtools_id, "display", display);
        }

        self.request_redraw();
    }

    /// Create the DevTools overlay DOM elements.
    fn create_devtools_overlay(&mut self) {
        let mut doc_ref = self.doc.borrow_mut();
        let mut inner = doc_ref.inner_mut();
        let mut mutator = inner.mutate();

        // Create the main devtools container
        let container = mutator.create_element(
            QualName::new(None, blitz_dom::ns!(html), LocalName::from("div")),
            vec![],
        );

        // Set the container styles
        mutator.set_attribute(
            container,
            QualName::new(None, blitz_dom::ns!(), LocalName::from("id")),
            "rinch-devtools",
        );

        let panel_width = self.devtools.panel_width;
        let container_style = format!(
            "position: fixed; right: 0; top: 0; bottom: 0; width: {}px; \
             background: #1e1e1e; color: #d4d4d4; \
             font-family: Consolas, Monaco, monospace; font-size: 12px; \
             border-left: 1px solid #3c3c3c; display: flex; flex-direction: column; \
             z-index: 999999;",
            panel_width
        );
        mutator.set_attribute(
            container,
            QualName::new(None, blitz_dom::ns!(), LocalName::from("style")),
            &container_style,
        );

        // Create tab bar
        let tab_bar = mutator.create_element(
            QualName::new(None, blitz_dom::ns!(html), LocalName::from("div")),
            vec![],
        );
        mutator.set_attribute(
            tab_bar,
            QualName::new(None, blitz_dom::ns!(), LocalName::from("style")),
            "display: flex; background: #252525; border-bottom: 1px solid #3c3c3c;",
        );

        // Create tab buttons
        for (label, is_active) in [("Elements", true), ("Styles", false), ("Hooks", false)] {
            let btn = mutator.create_element(
                QualName::new(None, blitz_dom::ns!(html), LocalName::from("button")),
                vec![],
            );
            let btn_style = if is_active {
                "flex: 1; padding: 8px; border: none; background: #2a2a2a; color: #d4d4d4; cursor: pointer;"
            } else {
                "flex: 1; padding: 8px; border: none; background: transparent; color: #d4d4d4; cursor: pointer;"
            };
            mutator.set_attribute(
                btn,
                QualName::new(None, blitz_dom::ns!(), LocalName::from("style")),
                btn_style,
            );
            let text = mutator.create_text_node(label);
            mutator.append_children(btn, &[text]);
            mutator.append_children(tab_bar, &[btn]);
        }

        mutator.append_children(container, &[tab_bar]);

        // Create toolbar with inspect button
        let toolbar = mutator.create_element(
            QualName::new(None, blitz_dom::ns!(html), LocalName::from("div")),
            vec![],
        );
        mutator.set_attribute(
            toolbar,
            QualName::new(None, blitz_dom::ns!(), LocalName::from("style")),
            "padding: 4px 8px; background: #252525; border-bottom: 1px solid #3c3c3c; display: flex; gap: 8px;",
        );

        let inspect_btn = mutator.create_element(
            QualName::new(None, blitz_dom::ns!(html), LocalName::from("button")),
            vec![],
        );
        mutator.set_attribute(
            inspect_btn,
            QualName::new(None, blitz_dom::ns!(), LocalName::from("style")),
            "padding: 4px 8px; border: 1px solid #3c3c3c; border-radius: 3px; background: #2d2d2d; color: #d4d4d4; cursor: pointer;",
        );
        let inspect_text = mutator.create_text_node("Inspect");
        mutator.append_children(inspect_btn, &[inspect_text]);
        mutator.append_children(toolbar, &[inspect_btn]);
        mutator.append_children(container, &[toolbar]);

        // Create content panel
        let content = mutator.create_element(
            QualName::new(None, blitz_dom::ns!(html), LocalName::from("div")),
            vec![],
        );
        mutator.set_attribute(
            content,
            QualName::new(None, blitz_dom::ns!(), LocalName::from("style")),
            "flex: 1; overflow: auto; padding: 8px;",
        );

        // Add content text
        let title = mutator.create_element(
            QualName::new(None, blitz_dom::ns!(html), LocalName::from("div")),
            vec![],
        );
        mutator.set_attribute(
            title,
            QualName::new(None, blitz_dom::ns!(), LocalName::from("style")),
            "font-weight: bold; margin-bottom: 8px; color: #dcdcaa;",
        );
        let title_text = mutator.create_text_node("DOM Tree");
        mutator.append_children(title, &[title_text]);
        mutator.append_children(content, &[title]);

        let info = mutator.create_element(
            QualName::new(None, blitz_dom::ns!(html), LocalName::from("div")),
            vec![],
        );
        mutator.set_attribute(
            info,
            QualName::new(None, blitz_dom::ns!(), LocalName::from("style")),
            "color: #808080;",
        );
        let info_text = mutator.create_text_node("Press Alt+D to toggle layout overlay. Alt+I for inspect mode.");
        mutator.append_children(info, &[info_text]);
        mutator.append_children(content, &[info]);

        mutator.append_children(container, &[content]);

        // Find body and append devtools
        let body_id = find_body_in_doc(&mutator.doc);
        if let Some(body) = body_id {
            mutator.append_children(body, &[container]);
        }

        drop(mutator);
        drop(inner);
        drop(doc_ref);

        self.devtools_element_id = Some(container);
        tracing::info!("Created devtools overlay element {}", container);
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

        let mut doc_ref = self.doc.borrow_mut();
        let mut inner = doc_ref.inner_mut();
        inner.resolve(animation_time);

        let (width, height) = inner.viewport().window_size;
        let scale = inner.viewport().scale_f64();
        drop(inner);
        drop(doc_ref);

        // This is where GPU device, queue, textures, and Vello renderer are created
        #[cfg(feature = "memory-profile")]
        let before_resume = super::memory_profile::MemorySnapshot::now();
        self.renderer.resume(self.window.clone(), width, height);
        #[cfg(feature = "memory-profile")]
        super::memory_profile::checkpoint_delta("After renderer.resume (GPU init)", before_resume);

        if !self.renderer.is_active() {
            tracing::error!("Renderer failed to resume");
            return;
        }

        #[cfg(feature = "memory-profile")]
        let before_render = super::memory_profile::MemorySnapshot::now();
        {
            let doc_ref = self.doc.borrow();
            let inner = doc_ref.inner();
            self.renderer.render(|scene| paint_scene(scene, &inner, scale, width, height, 0, 0));
        }
        #[cfg(feature = "memory-profile")]
        super::memory_profile::checkpoint_delta("After first render", before_render);

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
            if self.doc.borrow_mut().poll(Some(cx)) {
                self.request_redraw();
                return true;
            }
        }
        false
    }

    /// Redraw the window.
    pub fn redraw(&mut self) {
        self.perf_stats.begin_render();

        let animation_time = self.current_animation_time();
        let is_visible = self.is_visible;

        let resolve_start = std::time::Instant::now();
        let mut doc_ref = self.doc.borrow_mut();
        let mut inner = doc_ref.inner_mut();

        // Save scroll position before resolve (resolve may reset it)
        let scroll_pos = inner.viewport_scroll();

        inner.resolve(animation_time);

        // Restore scroll position after resolve
        inner.set_viewport_scroll(scroll_pos);

        let resolve_elapsed = resolve_start.elapsed();

        let (width, height) = inner.viewport().window_size;
        let scale = inner.viewport().scale_f64();
        let is_animating = inner.is_animating();

        // If the DOM was rebuilt (e.g., editor re-render Effect), blitz's text selection
        // may reference nodes that no longer exist. Clear stale selections before paint
        // to prevent panics in compare_document_order (traversal.rs) when paint_scene
        // tries to collect inline roots between stale anchor/focus nodes.
        if rinch_core::take_pending_selection_clear() {
            inner.clear_text_selection();
        }

        let paint_start = std::time::Instant::now();
        self.renderer.render(|scene| {
            // Wrap paint_scene in catch_unwind to handle blitz panics from stale node references
            let result = catch_unwind(AssertUnwindSafe(|| {
                paint_scene(scene, &inner, scale, width, height, 0, 0);
            }));
            if let Err(e) = result {
                tracing::warn!("paint_scene() panicked (likely stale node references): {:?}", e);
            }
        });
        // Force window system to acknowledge the update
        self.window.pre_present_notify();
        let paint_elapsed = paint_start.elapsed();

        drop(inner);
        drop(doc_ref);

        self.perf_stats.end_render();

        // Log detailed timing every second
        static LAST_REDRAW_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let last = LAST_REDRAW_LOG.load(std::sync::atomic::Ordering::Relaxed);
        if now_ms - last > 1000 {
            LAST_REDRAW_LOG.store(now_ms, std::sync::atomic::Ordering::Relaxed);
            tracing::debug!(
                "REDRAW: resolve={:.2}ms paint={:.2}ms total={:.2}ms is_animating={} will_redraw={}",
                resolve_elapsed.as_secs_f64() * 1000.0,
                paint_elapsed.as_secs_f64() * 1000.0,
                self.perf_stats.last_render_ms,
                is_animating,
                is_visible && is_animating
            );
        }

        // Log perf stats periodically when overlay is enabled
        if self.perf_stats.show_overlay && self.perf_stats.frame_times.len() % 60 == 0 {
            tracing::debug!(
                "FPS: {:.1} | Frame: {:.2}ms | Render: {:.2}ms",
                self.perf_stats.fps(),
                self.perf_stats.avg_frame_ms(),
                self.perf_stats.last_render_ms
            );
        }

        if is_visible && is_animating {
            self.request_redraw();
        }
    }

    /// Toggle the performance overlay.
    pub fn toggle_perf_overlay(&mut self) {
        self.perf_stats.show_overlay = !self.perf_stats.show_overlay;
        tracing::info!("Performance overlay: {}", if self.perf_stats.show_overlay { "ON" } else { "OFF" });
        if self.perf_stats.show_overlay {
            tracing::info!("Stats will be logged every 60 frames");
        }
    }

    /// Debug: print scroll offsets of all nodes with non-zero scroll.
    pub fn print_scroll_offsets(&self) {
        let doc_ref = self.doc.borrow();
        let inner = doc_ref.inner();

        println!("=== SCROLL OFFSETS ===");
        println!("Viewport scroll: {:?}", inner.viewport_scroll());

        fn walk_nodes(inner: &blitz_dom::BaseDocument, node_id: usize, depth: usize) {
            let Some(node) = inner.get_node(node_id) else {
                return;
            };

            let scroll = &node.scroll_offset;
            if scroll.x != 0.0 || scroll.y != 0.0 {
                let indent = "  ".repeat(depth);
                let tag = if let Some(el) = node.element_data() {
                    let classes = el.attrs().iter()
                        .find(|a| a.name.local.as_ref() == "class")
                        .map(|a| a.value.to_string())
                        .unwrap_or_default();
                    format!("<{} class=\"{}\">", el.name.local, classes)
                } else if node.is_text_node() {
                    format!("#text: {:?}", node.text_content().chars().take(30).collect::<String>())
                } else {
                    "#other".to_string()
                };
                println!("{}[{}] {} scroll=({:.1}, {:.1})", indent, node_id, tag, scroll.x, scroll.y);
            }

            for &child_id in &node.children {
                walk_nodes(inner, child_id, depth + 1);
            }
        }

        walk_nodes(&inner, 0, 0);
        println!("======================");
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
                {
                    let mut doc_ref = self.doc.borrow_mut();
                    let mut inner = doc_ref.inner_mut();
                    inner.viewport_mut().window_size = (physical_size.width, physical_size.height);
                }
                let (width, height) = {
                    let doc_ref = self.doc.borrow();
                    doc_ref.inner().viewport().window_size
                };
                if width > 0 && height > 0 {
                    self.renderer.set_size(width, height);
                    self.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                {
                    let mut doc_ref = self.doc.borrow_mut();
                    doc_ref.inner_mut().viewport_mut().set_hidpi_scale(scale_factor as f32);
                }
                self.request_redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                let color_scheme = match theme {
                    Theme::Light => ColorScheme::Light,
                    Theme::Dark => ColorScheme::Dark,
                };
                let mut doc_ref = self.doc.borrow_mut();
                doc_ref.inner_mut().viewport_mut().color_scheme = color_scheme;
            }
            WindowEvent::ModifiersChanged(new_state) => {
                self.keyboard_modifiers = new_state;
                // Update global modifier state for event handlers
                rinch_core::events::set_modifier_state(rinch_core::events::ModifierState {
                    shift: new_state.state().shift_key(),
                    ctrl: new_state.state().control_key(),
                    alt: new_state.state().alt_key(),
                    meta: new_state.state().super_key(),
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let ctrl = self.keyboard_modifiers.state().control_key();
                let meta = self.keyboard_modifiers.state().super_key();
                let alt = self.keyboard_modifiers.state().alt_key();
                let shift = self.keyboard_modifiers.state().shift_key();

                // Check global keyboard interceptor first (only on key press)
                let intercepted = if event.state.is_pressed() {
                    let key_data = winit_key_to_key_event_data(&event, self.keyboard_modifiers.state());
                    rinch_core::dispatch_keyboard_event(&key_data)
                } else {
                    false
                };

                if !intercepted {
                    // Convert and dispatch keyboard event to blitz for text input handling
                    let blitz_event = winit_key_event_to_blitz(&event, self.keyboard_modifiers.state());
                    let ui_event = if event.state.is_pressed() {
                        UiEvent::KeyDown(blitz_event)
                    } else {
                        UiEvent::KeyUp(blitz_event)
                    };
                    self.handle_ui_event_with_handler(ui_event);

                    // Handle physical key shortcuts (only on key press)
                    if event.state.is_pressed() {
                        if let PhysicalKey::Code(key_code) = event.physical_key {
                            // Ctrl/Cmd keyboard shortcuts for zoom
                            if ctrl || meta {
                                match key_code {
                                    KeyCode::Equal => {
                                        self.doc.borrow_mut().inner_mut().viewport_mut().zoom_by(0.1);
                                        self.request_redraw();
                                    }
                                    KeyCode::Minus => {
                                        self.doc.borrow_mut().inner_mut().viewport_mut().zoom_by(-0.1);
                                        self.request_redraw();
                                    }
                                    KeyCode::Digit0 => {
                                        self.doc.borrow_mut().inner_mut().viewport_mut().set_zoom(1.0);
                                        self.request_redraw();
                                    }
                                    _ => {}
                                }
                            }

                            // Alt keyboard shortcuts for dev tools
                            if alt {
                                match key_code {
                                    KeyCode::KeyD => {
                                        self.doc.borrow_mut().inner_mut().devtools_mut().toggle_show_layout();
                                        self.request_redraw();
                                    }
                                    KeyCode::KeyI => {
                                        // Toggle hover highlight (inspect mode)
                                        self.doc.borrow_mut().inner_mut().devtools_mut().toggle_highlight_hover();
                                        self.devtools.toggle_inspect_mode();
                                        tracing::info!("Inspect mode: {}", self.devtools.inspect_mode);
                                        self.request_redraw();
                                    }
                                    KeyCode::KeyT => {
                                        self.doc.borrow().inner().print_taffy_tree();
                                    }
                                    KeyCode::KeyS => {
                                        // Debug: print scroll offsets of all nodes
                                        self.print_scroll_offsets();
                                    }
                                    KeyCode::KeyP => {
                                        self.toggle_perf_overlay();
                                        self.request_redraw();
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
                }

                self.request_redraw();
            }
            WindowEvent::Ime(ime_event) => {
                // Convert and dispatch IME event to blitz for text input
                let blitz_ime = winit_ime_to_blitz(ime_event);
                self.handle_ui_event_with_handler(UiEvent::Ime(blitz_ime));
                self.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos: winit::dpi::LogicalPosition<f32> = position.to_logical(self.window.scale_factor());
                self.mouse_pos = (pos.x, pos.y);

                // Update resize cursor for borderless windows
                self.update_resize_cursor();

                // Update drag state if dragging (for sliders, etc.)
                // Note: The drag callback updates signals, which triggers Effects via flush_effects().
                // Effects update styles via BlitzDocumentAdapter, which marks nodes dirty.
                // The on_signal_change callback sends a ReRender event, which will call
                // resolve_dirty_documents() with proper ancestor dirty marking and request_redraw.
                let _ = rinch_core::events::update_drag(pos.x, pos.y);

                let event = UiEvent::PointerMove(BlitzPointerEvent {
                    id: BlitzPointerId::Mouse,
                    is_primary: true,
                    coords: PointerCoords {
                        page_x: pos.x,
                        page_y: pos.y,
                        screen_x: pos.x,
                        screen_y: pos.y,
                        client_x: pos.x,
                        client_y: pos.y,
                    },
                    button: Default::default(),
                    buttons: self.buttons,
                    mods: Default::default(),
                    details: PointerDetails::default(),
                });
                self.handle_ui_event_with_handler(event);

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

                let (x, y) = self.mouse_pos;
                let event_data = BlitzPointerEvent {
                    id: BlitzPointerId::Mouse,
                    is_primary: true,
                    coords: PointerCoords {
                        page_x: x,
                        page_y: y,
                        screen_x: x,
                        screen_y: y,
                        client_x: x,
                        client_y: y,
                    },
                    button,
                    buttons: self.buttons,
                    mods: Default::default(),
                    details: PointerDetails::default(),
                };

                let event = match state {
                    ElementState::Pressed => UiEvent::PointerDown(event_data),
                    ElementState::Released => UiEvent::PointerUp(event_data),
                };

                // Before dispatching PointerDown to blitz, save text selection.
                // Blitz's handle_mousedown may clear selection for non-text targets
                // (like toolbar buttons). The saved snapshot lets toolbar commands
                // still access the selection that was active before the click.
                if matches!(state, ElementState::Pressed) {
                    rinch_core::events::save_selection_snapshot();
                }

                self.handle_ui_event_with_handler(event);

                if matches!(button, MouseEventButton::Main) {
                    match state {
                        ElementState::Pressed => {
                            // Check for window resize FIRST (highest priority)
                            if let Some(direction) = self.get_resize_direction() {
                                self.start_resize(direction);
                                return;
                            }

                            // Check for window drag (before click handlers)
                            // Window drag uses data-drag-window attribute
                            if self.should_drag_window() {
                                self.start_drag();
                                return;
                            }

                            // Handle accordion toggle FIRST (before any event dispatch that might modify DOM)
                            // This is safe because it uses the same hit test position
                            self.handle_accordion_toggle();

                            // Dispatch click handler on mousedown (starts drag for sliders)
                            if let Some((handler_id, ctx)) = self.get_clicked_handler() {
                                tracing::info!("Dispatching mousedown to handler {:?}", handler_id);
                                rinch_core::events::set_click_context(ctx);
                                rinch_core::events::dispatch_event(handler_id);
                            }
                        }
                        ElementState::Released => {
                            // Stop any active drag operation
                            rinch_core::events::stop_drag();
                            // Sync blitz's drag selection into editor on mouseup
                            rinch_core::events::fire_selection_sync();
                            // Clear saved selection snapshot on mouse up
                            rinch_core::events::clear_selection_snapshot();
                        }
                    }
                }

                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Convert delta to pixels
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        // 20 pixels per line (matching blitz)
                        (x as f64 * 20.0, y as f64 * 20.0)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        (pos.x, pos.y)
                    }
                };

                // Add to scroll velocity for smooth scrolling
                self.scroll_velocity.0 += dx;
                self.scroll_velocity.1 += dy;
                self.last_scroll_time = Some(Instant::now());
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// Update the window's HTML content and re-render.
    pub fn update_content(&mut self, html_content: String) {
        let total_start = std::time::Instant::now();

        // Get current viewport settings, scroll positions (viewport and container elements)
        let (viewport, scale, scroll_pos, container_scrolls) = {
            let doc_ref = self.doc.borrow();
            let inner = doc_ref.inner();
            let viewport = inner.viewport().clone();
            let scale = inner.viewport().scale_f64();
            let viewport_scroll = inner.viewport_scroll();

            // Collect scroll positions from scrollable container elements
            // We use CSS selectors to identify elements that might have scroll state
            let mut container_scrolls: Vec<(&'static str, blitz_dom::Point<f64>)> = Vec::new();

            // List of common scrollable container selectors to preserve scroll for
            const SCROLLABLE_SELECTORS: &[&str] = &[
                ".main-content",
                ".sidebar",
                ".scroll-container",
                "[data-scroll]",
                "main",
                "article",
            ];

            for selector in SCROLLABLE_SELECTORS {
                if let Ok(Some(node_id)) = inner.query_selector(selector) {
                    if let Some(node) = inner.get_node(node_id) {
                        // Only save if there's actual scroll offset
                        if node.scroll_offset.x != 0.0 || node.scroll_offset.y != 0.0 {
                            container_scrolls.push((selector, node.scroll_offset));
                        }
                    }
                }
            }

            (viewport, scale, viewport_scroll, container_scrolls)
        };

        // Create new document config with current viewport
        let config = DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        };

        // Create new document with updated HTML
        let parse_start = std::time::Instant::now();
        *self.doc.borrow_mut() = Box::new(HtmlDocument::from_html(&html_content, config));
        let parse_elapsed = parse_start.elapsed();

        // Re-resolve and redraw
        let animation_time = self.current_animation_time();
        let resolve_start = std::time::Instant::now();
        {
            let mut doc_ref = self.doc.borrow_mut();
            let mut inner = doc_ref.inner_mut();
            inner.resolve(animation_time);

            // Restore viewport scroll position
            inner.set_viewport_scroll(scroll_pos);

            // Restore container element scroll positions
            for (selector, scroll_offset) in container_scrolls {
                if let Ok(Some(node_id)) = inner.query_selector(selector) {
                    if let Some(node) = inner.get_node_mut(node_id) {
                        node.scroll_offset = scroll_offset;
                    }
                }
            }
        }
        let resolve_elapsed = resolve_start.elapsed();

        // Render the updated content
        let render_start = std::time::Instant::now();
        let doc_ref = self.doc.borrow();
        let inner = doc_ref.inner();
        let (width, height) = inner.viewport().window_size;
        self.renderer.render(|scene| paint_scene(scene, &inner, scale, width, height, 0, 0));
        let render_elapsed = render_start.elapsed();

        let total_elapsed = total_start.elapsed();

        // Log timing every second
        static LAST_UPDATE_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let last = LAST_UPDATE_LOG.load(std::sync::atomic::Ordering::Relaxed);
        if now_ms - last > 1000 {
            LAST_UPDATE_LOG.store(now_ms, std::sync::atomic::Ordering::Relaxed);
            tracing::debug!(
                "UPDATE_CONTENT: parse={:.2}ms resolve={:.2}ms render={:.2}ms total={:.2}ms html_len={}",
                parse_elapsed.as_secs_f64() * 1000.0,
                resolve_elapsed.as_secs_f64() * 1000.0,
                render_elapsed.as_secs_f64() * 1000.0,
                total_elapsed.as_secs_f64() * 1000.0,
                html_content.len()
            );
        }
    }

    /// Update hover state by toggling "hover" class on elements.
    ///
    /// Blitz's CSS :hover doesn't work reliably on fixed/positioned elements,
    /// so we implement hover tracking manually via class toggling.
    /// Get information about the element under the current mouse position.
    ///
    /// Returns element info for DevTools display.
    pub fn get_hovered_element_info(&self) -> Option<HoveredElementInfo> {
        let doc_ref = self.doc.borrow();
        let inner = doc_ref.inner();
        // Adjust for viewport scroll
        let scroll = inner.viewport_scroll();
        let hit_x = self.mouse_pos.0 + scroll.x as f32;
        let hit_y = self.mouse_pos.1 + scroll.y as f32;
        // Wrap hit() in catch_unwind to handle blitz panics from stale stacking context
        let hit_result = catch_unwind(AssertUnwindSafe(|| inner.hit(hit_x, hit_y)))
            .ok()
            .flatten()?;
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

    /// Get the event handler ID and click context of the element under the current mouse position.
    ///
    /// Returns `Some((id, context))` if there's an element with a `data-rid` attribute at the
    /// current mouse position, `None` otherwise.
    pub fn get_clicked_handler(&self) -> Option<(EventHandlerId, rinch_core::ClickContext)> {
        let doc_ref = self.doc.borrow();
        let inner = doc_ref.inner();

        // Hit test at current mouse position, adjusted for viewport scroll
        let scroll = inner.viewport_scroll();
        tracing::info!(
            "Click: mouse=({:.0}, {:.0}), viewport_scroll=({}, {})",
            self.mouse_pos.0, self.mouse_pos.1, scroll.x, scroll.y
        );
        let hit_x = self.mouse_pos.0 + scroll.x as f32;
        let hit_y = self.mouse_pos.1 + scroll.y as f32;
        tracing::info!("Click: hit coords=({:.0}, {:.0})", hit_x, hit_y);
        // Wrap hit() in catch_unwind to handle blitz panics from stale stacking context
        let hit_result = catch_unwind(AssertUnwindSafe(|| inner.hit(hit_x, hit_y)))
            .ok()
            .flatten()?;
        let node_id = hit_result.node_id;

        // Debug: log hit info with layout position
        if let Some(node) = inner.get_node(node_id) {
            let layout = &node.final_layout;
            let loc = layout.location;
            let size = layout.size;
            if let Some(element) = node.element_data() {
                let tag = element.name.local.as_ref();
                let classes = element.attrs().iter()
                    .find(|a| a.name.local.as_ref() == "class")
                    .map(|a| a.value.to_string())
                    .unwrap_or_default();
                tracing::info!(
                    "Hit test: node={} tag={} class={} layout=({:.0},{:.0}) size=({:.0}x{:.0})",
                    node_id, tag, classes, loc.x, loc.y, size.width, size.height
                );
            } else {
                tracing::info!(
                    "Hit test: node={} (no element) layout=({:.0},{:.0}) size=({:.0}x{:.0})",
                    node_id, loc.x, loc.y, size.width, size.height
                );
            }
        }

        // Walk up the tree looking for a data-rid attribute
        let mut current = Some(node_id);
        let mut depth = 0;
        while let Some(id) = current {
            if let Some(node) = inner.get_node(id) {
                if let Some(element) = node.element_data() {
                    let tag = element.name.local.as_ref();
                    let attrs: Vec<_> = element.attrs().iter().map(|a| format!("{}={}", a.name.local.as_ref(), &a.value)).collect();
                    tracing::trace!("  [{}] Hit walk: node={} tag={} attrs={:?}", depth, id, tag, attrs);
                    depth += 1;
                    // Check all attributes for data-rid
                    for attr in element.attrs() {
                        if attr.name.local.as_ref() == "data-rid" {
                            if let Ok(rid) = attr.value.parse::<usize>() {
                                // Calculate absolute position by walking up the parent chain
                                let mut abs_x = 0.0f32;
                                let mut abs_y = 0.0f32;
                                let mut walk = Some(id);
                                while let Some(walk_id) = walk {
                                    if let Some(walk_node) = inner.get_node(walk_id) {
                                        abs_x += walk_node.final_layout.location.x;
                                        abs_y += walk_node.final_layout.location.y;
                                        walk = walk_node.parent;
                                    } else {
                                        break;
                                    }
                                }

                                // Adjust for scroll offset
                                abs_x -= scroll.x as f32;
                                abs_y -= scroll.y as f32;

                                let layout = &node.final_layout;

                                // Resolve text position using blitz's layout engine
                                let text_hit = inner.find_text_position(hit_x, hit_y)
                                    .and_then(|(inline_root_id, byte_offset)| {
                                        // Walk up from inline root to find data-block-index
                                        let mut walk_id = Some(inline_root_id);
                                        while let Some(wid) = walk_id {
                                            if let Some(wnode) = inner.get_node(wid) {
                                                if let Some(el) = wnode.element_data() {
                                                    for a in el.attrs() {
                                                        if a.name.local.as_ref() == "data-block-index" {
                                                            if let Ok(bi) = a.value.parse::<usize>() {
                                                                return Some(rinch_core::TextHitInfo {
                                                                    block_index: bi,
                                                                    byte_offset,
                                                                    inline_root_node_id: inline_root_id,
                                                                    valid: true,
                                                                });
                                                            }
                                                        }
                                                    }
                                                }
                                                walk_id = wnode.parent;
                                            } else {
                                                break;
                                            }
                                        }
                                        None
                                    })
                                    .unwrap_or_default();

                                let ctx = rinch_core::ClickContext {
                                    mouse_x: self.mouse_pos.0,
                                    mouse_y: self.mouse_pos.1,
                                    element_x: abs_x,
                                    element_y: abs_y,
                                    element_width: layout.size.width,
                                    element_height: layout.size.height,
                                    text_hit,
                                };
                                return Some((EventHandlerId(rid), ctx));
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

    /// Handle accordion toggle when clicking an accordion control.
    ///
    /// If the clicked element has `data-accordion-toggle="true"`, find the parent
    /// `.rinch-accordion__item` and toggle its `data-active` attribute.
    pub fn handle_accordion_toggle(&mut self) -> bool {
        // First, find if we clicked on an accordion toggle (read-only phase)
        let toggle_info = {
            let doc_ref = self.doc.borrow();
            let inner = doc_ref.inner();

            // Hit test at current mouse position, adjusted for viewport scroll
            let scroll = inner.viewport_scroll();
            let hit_x = self.mouse_pos.0 + scroll.x as f32;
            let hit_y = self.mouse_pos.1 + scroll.y as f32;
            // Wrap hit() in catch_unwind to handle blitz panics from stale stacking context
            let Some(hit_result) = catch_unwind(AssertUnwindSafe(|| inner.hit(hit_x, hit_y)))
                .ok()
                .flatten()
            else {
                return false;
            };
            let node_id = hit_result.node_id;

            // Walk up the tree looking for data-accordion-toggle
            let mut current = Some(node_id);
            let mut found_toggle = false;
            let mut accordion_item_id = None;

            while let Some(id) = current {
                if let Some(node) = inner.get_node(id) {
                    if let Some(element) = node.element_data() {
                        // Check for data-accordion-toggle
                        if !found_toggle {
                            for attr in element.attrs() {
                                if attr.name.local.as_ref() == "data-accordion-toggle" {
                                    found_toggle = true;
                                    break;
                                }
                            }
                        }

                        // Check for accordion item class
                        if found_toggle && accordion_item_id.is_none() {
                            for attr in element.attrs() {
                                if attr.name.local.as_ref() == "class" {
                                    if attr.value.split_whitespace().any(|c| c == "rinch-accordion__item") {
                                        // Check current data-active state
                                        let is_active = element.attrs().iter().any(|a| {
                                            a.name.local.as_ref() == "data-active" && a.value.as_ref() as &str == "true"
                                        });
                                        accordion_item_id = Some((id, is_active));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }

            if found_toggle {
                accordion_item_id
            } else {
                None
            }
        };

        // Now toggle the accordion item if we found one (write phase)
        if let Some((item_id, is_active)) = toggle_info {
            let mut doc_ref = self.doc.borrow_mut();
            let mut inner = doc_ref.inner_mut();

            // Verify the node still exists before mutating
            if inner.get_node(item_id).is_none() {
                tracing::warn!("Accordion toggle: node {} no longer exists", item_id);
                return false;
            }

            let mut mutator = inner.mutate();

            // Toggle the data-active attribute
            let new_value = if is_active { "false" } else { "true" };
            mutator.set_attribute(item_id, attr_name("data-active"), new_value);

            // Don't call resolve() here - it will be called by the frame loop
            // Calling it here can crash if animations have stale node references
            drop(mutator);

            tracing::info!("Accordion toggle: item_id={}, now active={}", item_id, new_value);
            return true;
        }

        false
    }

    /// Check if the element under the current mouse position should trigger window dragging.
    ///
    /// Returns `true` if there's an element with `data-drag-window` attribute at the
    /// current mouse position.
    pub fn should_drag_window(&self) -> bool {
        let doc_ref = self.doc.borrow();
        let inner = doc_ref.inner();

        // Hit test at current mouse position, adjusted for viewport scroll
        let scroll = inner.viewport_scroll();
        let hit_x = self.mouse_pos.0 + scroll.x as f32;
        let hit_y = self.mouse_pos.1 + scroll.y as f32;
        // Wrap hit() in catch_unwind to handle blitz panics from stale stacking context
        let Some(hit_result) = catch_unwind(AssertUnwindSafe(|| inner.hit(hit_x, hit_y)))
            .ok()
            .flatten()
        else {
            return false;
        };
        let node_id = hit_result.node_id;

        // Walk up the tree looking for a data-drag-window attribute
        // Stop if we find a click handler (data-rid) first - those should be handled as clicks, not drags
        let mut current = Some(node_id);
        while let Some(id) = current {
            if let Some(node) = inner.get_node(id) {
                if let Some(element) = node.element_data() {
                    for attr in element.attrs() {
                        let name = attr.name.local.as_ref();
                        // If we find a click handler first, this is a click, not a drag
                        if name == "data-rid" {
                            return false;
                        }
                        // Found drag window marker
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

    /// Check if the mouse is near a window edge for resizing.
    ///
    /// Returns `Some(ResizeDirection)` if the mouse is within the resize border
    /// area at the window edges. Only active for borderless, resizable windows.
    ///
    /// The resize area is controlled by `WindowProps::resize_inset`:
    /// - `Some(inset)` - Resize handles are active within `inset` pixels from edges,
    ///   plus a small grab extension into the visible content area.
    /// - `None` - Custom resize handles are disabled (use native window chrome).
    pub fn get_resize_direction(&self) -> Option<ResizeDirection> {
        // Only enable custom resize for borderless, resizable windows
        if !self.props.borderless || !self.props.resizable {
            return None;
        }

        // Get the resize inset, or return None if custom resize is disabled
        let inset = match self.props.resize_inset {
            Some(i) => i,
            None => return None,
        };

        // Additional pixels into visible content for easier grabbing.
        // This needs to be large enough since transparent areas don't receive
        // mouse events on Windows - resize detection only works on visible content.
        const GRAB_EXTEND: f32 = 8.0;
        // Total resize area from window edge
        let border_size = inset + GRAB_EXTEND;
        // Corner hit area (larger for easier diagonal resize)
        let corner_size = inset + 16.0;

        let (mx, my) = self.mouse_pos;
        let scale = self.window.scale_factor() as f32;
        let size = self.window.inner_size();
        let width = size.width as f32 / scale;
        let height = size.height as f32 / scale;

        // Check if mouse is near edges
        let near_left = mx < border_size;
        let near_right = mx > width - border_size;
        let near_top = my < border_size;
        let near_bottom = my > height - border_size;

        // Check corners (larger hit area)
        let in_left_corner = mx < corner_size;
        let in_right_corner = mx > width - corner_size;
        let in_top_corner = my < corner_size;
        let in_bottom_corner = my > height - corner_size;

        // Corners take priority
        if in_top_corner && in_left_corner {
            return Some(ResizeDirection::NorthWest);
        }
        if in_top_corner && in_right_corner {
            return Some(ResizeDirection::NorthEast);
        }
        if in_bottom_corner && in_left_corner {
            return Some(ResizeDirection::SouthWest);
        }
        if in_bottom_corner && in_right_corner {
            return Some(ResizeDirection::SouthEast);
        }

        // Then edges
        if near_top {
            return Some(ResizeDirection::North);
        }
        if near_bottom {
            return Some(ResizeDirection::South);
        }
        if near_left {
            return Some(ResizeDirection::West);
        }
        if near_right {
            return Some(ResizeDirection::East);
        }

        None
    }

    /// Update the cursor based on whether the mouse is over a resize edge.
    ///
    /// Call this on CursorMoved events to provide visual feedback.
    /// Returns true if the cursor was changed (for resize hover), false otherwise.
    pub fn update_resize_cursor(&mut self) -> bool {
        let new_direction = self.get_resize_direction();

        if new_direction != self.current_resize_direction {
            self.current_resize_direction = new_direction;

            let cursor = match new_direction {
                Some(ResizeDirection::North) => CursorIcon::NResize,
                Some(ResizeDirection::South) => CursorIcon::SResize,
                Some(ResizeDirection::East) => CursorIcon::EResize,
                Some(ResizeDirection::West) => CursorIcon::WResize,
                Some(ResizeDirection::NorthEast) => CursorIcon::NeResize,
                Some(ResizeDirection::NorthWest) => CursorIcon::NwResize,
                Some(ResizeDirection::SouthEast) => CursorIcon::SeResize,
                Some(ResizeDirection::SouthWest) => CursorIcon::SwResize,
                None => CursorIcon::Default,
            };

            self.window.set_cursor(cursor);
            return new_direction.is_some();
        }

        new_direction.is_some()
    }

    /// Initiate window resizing in the given direction.
    pub fn start_resize(&self, direction: ResizeDirection) {
        if let Err(e) = self.window.drag_resize_window(direction) {
            tracing::warn!("Failed to start window resize: {:?}", e);
        }
    }

    /// Build the reactive text mapping by querying for `[data-rid-reactive]` elements.
    ///
    /// This should be called after the initial HTML is parsed into the blitz Document.
    /// It enables fine-grained updates by mapping reactive IDs to blitz node IDs.
    pub fn build_reactive_text_map(&mut self) {
        let start = std::time::Instant::now();
        self.reactive_text_map.clear();

        let doc_ref = self.doc.borrow();
        let inner = doc_ref.inner();

        // Walk the DOM looking for elements with data-rid-reactive attribute
        // Map stores (span_id, text_node_id) - we update the text node but mark the span dirty
        fn find_reactive_nodes(
            inner: &blitz_dom::BaseDocument,
            node_id: usize,
            map: &mut HashMap<usize, (usize, usize)>,
        ) {
            let Some(node) = inner.get_node(node_id) else {
                return;
            };

            // Check if this element has the reactive marker
            if let Some(element) = node.element_data() {
                for attr in element.attrs() {
                    if attr.name.local.as_ref() == "data-rid-reactive" {
                        if let Ok(reactive_id) = attr.value.parse::<usize>() {
                            // Find the first text node child - that's what we'll update
                            // But store the span's ID too for dirty marking
                            for &child_id in &node.children {
                                if let Some(child) = inner.get_node(child_id) {
                                    if child.is_text_node() {
                                        // Store (span_id, text_node_id)
                                        map.insert(reactive_id, (node_id, child_id));
                                        tracing::debug!(
                                            "Mapped reactive_id={} to span_id={}, text_node_id={}",
                                            reactive_id,
                                            node_id,
                                            child_id
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Recurse into children
            for &child_id in &node.children {
                find_reactive_nodes(inner, child_id, map);
            }
        }

        // Start from root
        if let Some(root) = inner.get_node(0) {
            for &child_id in &root.children {
                find_reactive_nodes(&inner, child_id, &mut self.reactive_text_map);
            }
        }

        let elapsed = start.elapsed();
        tracing::debug!(
            "Built reactive text map: {} entries in {:.2}ms",
            self.reactive_text_map.len(),
            elapsed.as_secs_f64() * 1000.0
        );
    }

    /// Apply a reactive text update without resolving layout.
    /// Call `finish_reactive_updates_with_dirty()` after applying all updates to resolve and redraw.
    ///
    /// Returns the span node ID for dirty marking if the update was applied, None if not found.
    /// NOTE: This currently requires blitz's set_node_text to insert damage on the parent,
    /// otherwise the text change won't trigger layout. See blitz fork for the fix.
    pub fn apply_reactive_text(&mut self, reactive_id: usize, text: &str) -> Option<usize> {
        let Some(&(span_id, text_node_id)) = self.reactive_text_map.get(&reactive_id) else {
            return None;
        };

        // Check if the node still exists before updating
        {
            let doc_ref = self.doc.borrow();
            let inner = doc_ref.inner();
            if inner.get_node(text_node_id).is_none() {
                tracing::warn!(
                    "apply_reactive_text: node {} no longer exists (reactive_id={})",
                    text_node_id, reactive_id
                );
                return None;
            }
        }

        // Directly update the text node in the blitz Document (no resolve yet)
        // The mutator's set_node_text inserts ALL_DAMAGE on the text node
        let mut doc_ref = self.doc.borrow_mut();
        let mut inner = doc_ref.inner_mut();
        let mut mutator = inner.mutate();
        mutator.set_node_text(text_node_id, text);

        // Return span_id for dirty marking (text nodes don't participate in style traversal)
        Some(span_id)
    }

    /// Finish reactive updates by marking dirty nodes, resolving layout, and requesting redraw.
    /// Pass the node IDs that were modified to ensure they get marked dirty for style traversal.
    pub fn finish_reactive_updates_with_dirty(&mut self, dirty_node_ids: &[usize]) {
        // First pass: mark ancestors dirty (immutable borrow, matching patching pattern)
        // Only mark nodes that still exist
        if !dirty_node_ids.is_empty() {
            let doc_ref = self.doc.borrow();
            let inner = doc_ref.inner();
            for &node_id in dirty_node_ids {
                if let Some(node) = inner.get_node(node_id) {
                    node.mark_ancestors_dirty();
                } else {
                    tracing::debug!("finish_reactive_updates: skipping dirty node {} (no longer exists)", node_id);
                }
            }
        }

        // Second pass: resolve layout (new mutable borrow)
        let animation_time = self.current_animation_time();
        {
            let mut doc_ref = self.doc.borrow_mut();
            let mut inner = doc_ref.inner_mut();
            inner.resolve(animation_time);
        }
        self.request_redraw();
    }


    /// Update a reactive text node directly in the blitz Document.
    /// This is a convenience method that applies and finishes a single update.
    /// For multiple updates, use `apply_reactive_text` + `finish_reactive_updates_with_dirty`.
    ///
    /// Returns true if the update was successful, false if the reactive ID wasn't found.
    #[allow(deprecated)]
    pub fn update_reactive_text(&mut self, reactive_id: usize, text: &str) -> bool {
        if let Some(span_id) = self.apply_reactive_text(reactive_id, text) {
            self.finish_reactive_updates_with_dirty(&[span_id]);
            true
        } else {
            false
        }
    }

    /// Get the number of reactive text mappings.
    pub fn reactive_text_count(&self) -> usize {
        self.reactive_text_map.len()
    }

    /// Build the reactive style mapping by querying for `[data-rid-style]` elements.
    ///
    /// This should be called after the initial HTML is parsed into the blitz Document.
    /// It enables fine-grained style updates by mapping style IDs to blitz element node IDs.
    pub fn build_reactive_style_map(&mut self) {
        let start = std::time::Instant::now();
        self.reactive_style_map.clear();

        let doc_ref = self.doc.borrow();
        let inner = doc_ref.inner();

        // Walk the DOM looking for elements with data-rid-style attribute
        fn find_reactive_style_nodes(
            inner: &blitz_dom::BaseDocument,
            node_id: usize,
            map: &mut HashMap<usize, (usize, String)>,
        ) {
            let Some(node) = inner.get_node(node_id) else {
                return;
            };

            // Check if this element has the reactive style marker
            if let Some(element) = node.element_data() {
                for attr in element.attrs() {
                    if attr.name.local.as_ref() == "data-rid-style" {
                        // Format: "id:property" or "id1:prop1,id2:prop2" for multiple
                        for entry in attr.value.split(',') {
                            if let Some((id_str, property)) = entry.split_once(':') {
                                if let Ok(style_id) = id_str.parse::<usize>() {
                                    map.insert(style_id, (node_id, property.to_string()));
                                    tracing::debug!(
                                        "Mapped reactive_style_id={} to blitz_node_id={}, property={}",
                                        style_id,
                                        node_id,
                                        property
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Recurse into children
            for &child_id in &node.children {
                find_reactive_style_nodes(inner, child_id, map);
            }
        }

        // Start from root
        if let Some(root) = inner.get_node(0) {
            for &child_id in &root.children {
                find_reactive_style_nodes(&inner, child_id, &mut self.reactive_style_map);
            }
        }

        let elapsed = start.elapsed();
        tracing::debug!(
            "Built reactive style map: {} entries in {:.2}ms",
            self.reactive_style_map.len(),
            elapsed.as_secs_f64() * 1000.0
        );
    }

    /// Apply a reactive style update without resolving layout.
    /// Call `finish_reactive_updates_with_dirty()` after applying all updates to resolve and redraw.
    ///
    /// Returns the blitz node ID if the update was applied, None if the style ID wasn't found.
    pub fn apply_reactive_style(&mut self, style_id: usize, property: &str, value: &str) -> Option<usize> {
        let Some(&(blitz_node_id, ref expected_property)) = self.reactive_style_map.get(&style_id) else {
            return None;
        };

        // Verify the property matches
        if expected_property != property {
            tracing::warn!(
                "Reactive style property mismatch: expected {}, got {}",
                expected_property,
                property
            );
            return None;
        }

        // Update the style attribute on the blitz element
        // We need to parse the existing style, update/add our property, and set it back
        let mut doc_ref = self.doc.borrow_mut();
        let mut inner = doc_ref.inner_mut();

        // Get current style attribute
        let current_style = if let Some(node) = inner.get_node(blitz_node_id) {
            if let Some(element) = node.element_data() {
                element.attrs()
                    .iter()
                    .find(|a| a.name.local.as_ref() == "style")
                    .map(|a| a.value.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        } else {
            return None;
        };

        // Parse existing styles into a map
        let mut styles: HashMap<String, String> = HashMap::new();
        for part in current_style.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some((k, v)) = part.split_once(':') {
                styles.insert(k.trim().to_string(), v.trim().to_string());
            }
        }

        // Update our property
        styles.insert(property.to_string(), value.to_string());

        // Serialize back to style string
        let new_style: String = styles
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("; ");

        // Set the new style attribute
        let mut mutator = inner.mutate();
        mutator.set_attribute(blitz_node_id, attr_name("style"), &new_style);

        Some(blitz_node_id)
    }

    /// Get the number of reactive style mappings.
    pub fn reactive_style_count(&self) -> usize {
        self.reactive_style_map.len()
    }

    /// Build the mapping from reactive attribute IDs to blitz node IDs.
    /// Called after HTML is rendered to enable fine-grained attribute updates.
    pub fn build_reactive_attr_map(&mut self) {
        let start = std::time::Instant::now();
        self.reactive_attr_map.clear();
        self.reactive_bool_attr_map.clear();

        let doc_ref = self.doc.borrow();
        let inner = doc_ref.inner();

        // Walk the DOM looking for elements with data-rid-attr and data-rid-bool-attr attributes
        fn find_reactive_attr_nodes(
            inner: &blitz_dom::BaseDocument,
            node_id: usize,
            attr_map: &mut HashMap<usize, (usize, String)>,
            bool_attr_map: &mut HashMap<usize, (usize, String)>,
        ) {
            let Some(node) = inner.get_node(node_id) else {
                return;
            };

            // Check if this element has reactive attribute markers
            if let Some(element) = node.element_data() {
                for attr in element.attrs() {
                    // Reactive string attributes
                    if attr.name.local.as_ref() == "data-rid-attr" {
                        // Format: "id:name" or "id1:name1,id2:name2" for multiple
                        for entry in attr.value.split(',') {
                            if let Some((id_str, name)) = entry.split_once(':') {
                                if let Ok(attr_id) = id_str.parse::<usize>() {
                                    attr_map.insert(attr_id, (node_id, name.to_string()));
                                    tracing::debug!(
                                        "Mapped reactive_attr_id={} to blitz_node_id={}, name={}",
                                        attr_id,
                                        node_id,
                                        name
                                    );
                                }
                            }
                        }
                    }
                    // Reactive boolean attributes
                    if attr.name.local.as_ref() == "data-rid-bool-attr" {
                        // Format: "id:name" or "id1:name1,id2:name2" for multiple
                        for entry in attr.value.split(',') {
                            if let Some((id_str, name)) = entry.split_once(':') {
                                if let Ok(attr_id) = id_str.parse::<usize>() {
                                    bool_attr_map.insert(attr_id, (node_id, name.to_string()));
                                    tracing::debug!(
                                        "Mapped reactive_bool_attr_id={} to blitz_node_id={}, name={}",
                                        attr_id,
                                        node_id,
                                        name
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Recurse into children
            for &child_id in &node.children {
                find_reactive_attr_nodes(inner, child_id, attr_map, bool_attr_map);
            }
        }

        // Start from root
        if let Some(root) = inner.get_node(0) {
            for &child_id in &root.children {
                find_reactive_attr_nodes(&inner, child_id, &mut self.reactive_attr_map, &mut self.reactive_bool_attr_map);
            }
        }

        let elapsed = start.elapsed();
        tracing::debug!(
            "Built reactive attr maps: {} attr, {} bool_attr entries in {:.2}ms",
            self.reactive_attr_map.len(),
            self.reactive_bool_attr_map.len(),
            elapsed.as_secs_f64() * 1000.0
        );
    }

    /// Apply a reactive attribute update without resolving layout.
    /// Call `finish_reactive_updates_with_dirty()` after applying all updates to resolve and redraw.
    ///
    /// Returns the blitz node ID if the update was applied, None if the attr ID wasn't found.
    pub fn apply_reactive_attr(&mut self, attr_id: usize, name: &str, value: &str) -> Option<usize> {
        let Some(&(blitz_node_id, ref expected_name)) = self.reactive_attr_map.get(&attr_id) else {
            return None;
        };

        // Verify the name matches
        if expected_name != name {
            tracing::warn!(
                "Reactive attr name mismatch: expected {}, got {}",
                expected_name,
                name
            );
            return None;
        }

        // Check if the node still exists before updating
        {
            let doc_ref = self.doc.borrow();
            let inner = doc_ref.inner();
            if inner.get_node(blitz_node_id).is_none() {
                tracing::warn!(
                    "apply_reactive_attr: node {} no longer exists (attr_id={})",
                    blitz_node_id, attr_id
                );
                return None;
            }
        }

        // Update the attribute on the blitz element
        // Note: blitz doesn't have remove_attribute, so we set empty string for "removal"
        // This works for most cases where we just need to change the attribute value
        let mut doc_ref = self.doc.borrow_mut();
        let mut inner = doc_ref.inner_mut();
        let mut mutator = inner.mutate();
        mutator.set_attribute(blitz_node_id, attr_name(name), value);

        Some(blitz_node_id)
    }

    /// Apply a reactive boolean attribute update without resolving layout.
    /// Call `finish_reactive_updates_with_dirty()` after applying all updates to resolve and redraw.
    ///
    /// Returns the blitz node ID if the update was applied, None if the attr ID wasn't found.
    pub fn apply_reactive_bool_attr(&mut self, attr_id: usize, name: &str, present: bool) -> Option<usize> {
        let Some(&(blitz_node_id, ref expected_name)) = self.reactive_bool_attr_map.get(&attr_id) else {
            return None;
        };

        // Verify the name matches
        if expected_name != name {
            tracing::warn!(
                "Reactive bool attr name mismatch: expected {}, got {}",
                expected_name,
                name
            );
            return None;
        }

        // Check if the node still exists before updating
        {
            let doc_ref = self.doc.borrow();
            let inner = doc_ref.inner();
            if inner.get_node(blitz_node_id).is_none() {
                tracing::warn!(
                    "apply_reactive_bool_attr: node {} no longer exists (attr_id={})",
                    blitz_node_id, attr_id
                );
                return None;
            }
        }

        // Update the attribute on the blitz element
        // For boolean attributes: present=true adds the attribute, present=false removes it
        // Since blitz doesn't have remove_attribute, we use a special marker value
        // that CSS selectors won't match: data-disabled="false" vs disabled=""
        let mut doc_ref = self.doc.borrow_mut();
        let mut inner = doc_ref.inner_mut();
        let mut mutator = inner.mutate();

        if present {
            // Add the boolean attribute with empty value (standard HTML boolean attr)
            mutator.set_attribute(blitz_node_id, attr_name(name), "");
        } else {
            // Set a marker value that indicates "not present"
            // For attributes like "disabled", having no attribute vs disabled="" is different
            // We'll use a data- attribute pattern to work around this limitation
            // The CSS selector [disabled] won't match [disabled="__removed__"]
            mutator.set_attribute(blitz_node_id, attr_name(name), "__removed__");
        }

        Some(blitz_node_id)
    }

    /// Resolve layout and request redraw.
    /// Used by FineGrainedRuntime after building DOM.
    pub fn resolve_and_request_redraw(&mut self) {
        let animation_time = self.current_animation_time();
        {
            let mut doc_ref = self.doc.borrow_mut();
            doc_ref.inner_mut().resolve(animation_time);
        }
        self.request_redraw();
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

    /// Get a shared reference to a window's document.
    /// Used by BlitzDocumentAdapter for direct DOM building.
    pub fn get_shared_document(&self, id: WindowId) -> Option<SharedDocument> {
        self.windows.get(&id).map(|w| w.doc.clone())
    }

    /// Remove and close a window.
    pub fn close_window(&mut self, id: WindowId) -> Option<ManagedWindow> {
        self.windows.remove(&id)
    }

    /// Check if any windows are still open.
    pub fn has_windows(&self) -> bool {
        !self.windows.is_empty()
    }

    /// Tick scroll animations for all windows.
    /// Returns true if any window is still animating.
    pub fn tick_scroll_animations(&mut self) -> bool {
        let mut any_animating = false;
        for window in self.windows.values_mut() {
            if window.tick_scroll_animation() {
                any_animating = true;
                window.request_redraw();
            }
        }
        any_animating
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

/// Create a QualName from a simple attribute name string.
fn attr_name(name: &str) -> blitz_dom::QualName {
    blitz_dom::QualName {
        prefix: None,
        // Use ns!() macro instead of Namespace::from("") to ensure we get
        // the exact same interned atom that HTML parsing uses
        ns: blitz_dom::ns!(),
        local: blitz_dom::LocalName::from(name),
    }
}

/// Find the body element ID in a BaseDocument.
fn find_body_in_doc(doc: &BaseDocument) -> Option<usize> {
    fn find_recursive(doc: &BaseDocument, node_id: usize) -> Option<usize> {
        let node = doc.get_node(node_id)?;
        if let Some(el) = node.element_data() {
            if el.name.local.as_ref() == "body" {
                return Some(node_id);
            }
        }
        for &child_id in &node.children {
            if let Some(found) = find_recursive(doc, child_id) {
                return Some(found);
            }
        }
        None
    }
    find_recursive(doc, 0)
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
