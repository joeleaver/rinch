//! Fine-grained runtime integration.
//!
//! This module provides the integration between fine-grained reactive rendering
//! and the actual window system. It enables:
//!
//! - Components that run once (not on every signal change)
//! - Effects that surgically update the DOM directly in blitz
//! - No HTML serialization after initial window creation
//!
//! # Architecture
//!
//! Unlike traditional virtual DOM approaches, this runtime builds DOM directly
//! in blitz's Document. All mutations (text, attributes, structural changes)
//! go through the same path - directly to blitz via the DocumentMutator API.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use rinch_core::dom::{clear_render_scope, set_render_scope, DomDocument, NodeHandle, RenderScope};

// Thread-local to store the current window's document adapter for theme updates
thread_local! {
    static CURRENT_DOC_ADAPTER: RefCell<Option<super::blitz_document::SharedBlitzDocument>> = const { RefCell::new(None) };
}

/// Set the current document adapter (called by runtime when creating windows).
pub fn set_current_doc_adapter(adapter: super::blitz_document::SharedBlitzDocument) {
    CURRENT_DOC_ADAPTER.with(|a| {
        *a.borrow_mut() = Some(adapter);
    });
}

/// Update the theme CSS dynamically.
///
/// This regenerates the theme CSS and updates the style element in the document.
/// Call this when theme signals change (dark mode toggle, primary color change).
#[cfg(feature = "theme")]
pub fn update_theme(props: &rinch_core::element::ThemeProviderProps) {
    use rinch_theme::{generate_theme_css, ColorName, RadiusSize, Theme};

    let mut builder = Theme::builder();

    if let Some(ref color) = props.primary_color {
        if let Ok(name) = color.parse::<ColorName>() {
            builder = builder.primary_color(name);
        }
    }

    if let Some(ref radius) = props.default_radius {
        if let Ok(size) = radius.parse::<RadiusSize>() {
            builder = builder.default_radius(size);
        }
    }

    if let Some(ref family) = props.font_family {
        builder = builder.font_family(family.clone());
    }

    if let Some(ref family) = props.font_family_monospace {
        builder = builder.font_family_monospace(family.clone());
    }

    if props.dark_mode {
        builder = builder.dark_mode(true);
    }

    if let Some(shade) = props.primary_shade {
        builder = builder.primary_shade(shade as usize);
    }

    let theme = builder.build();
    let mut css = generate_theme_css(&theme);

    // Include widget CSS when widgets feature is enabled
    #[cfg(feature = "widgets")]
    {
        css.push_str(&rinch_widgets::generate_widget_css());
    }

    // Update the document's theme style element
    CURRENT_DOC_ADAPTER.with(|a| {
        if let Some(adapter) = a.borrow().as_ref() {
            adapter.borrow().update_theme_css(&css);
        }
    });
}

/// No-op when theme feature is disabled.
#[cfg(not(feature = "theme"))]
pub fn update_theme(_props: &rinch_core::element::ThemeProviderProps) {}
use rinch_core::element::WindowProps;
use rinch_core::events::clear_handlers;
use rinch_core::hooks::{begin_render, clear_hooks, end_render};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use super::blitz_document::{BlitzDocumentAdapter, SharedBlitzDocument};
use super::types::RinchEvent;
use super::window_manager::WindowManager;
use crate::menu::MenuManager;

/// Type alias for a component function.
pub type ComponentFn = Box<dyn FnOnce(&mut RenderScope) -> NodeHandle>;

/// A pending window waiting to be created.
struct PendingWindow {
    props: WindowProps,
    component: ComponentFn,
}

/// Runtime for fine-grained applications.
///
/// This runtime creates windows and builds DOM directly in blitz's Document.
/// There is no virtual DOM layer - all operations go straight to blitz.
pub struct FineGrainedRuntime {
    window_manager: WindowManager,
    #[allow(dead_code)]
    menu_manager: MenuManager,
    proxy: Option<EventLoopProxy<RinchEvent>>,
    /// Windows waiting to be created (component not yet run).
    pending_windows: Vec<PendingWindow>,
    /// Document adapters for each window (for checking dirty state).
    /// Keyed by WindowId.
    document_adapters: std::collections::HashMap<WindowId, SharedBlitzDocument>,
    /// Render scopes for each window (must be kept alive for effects to work).
    /// When RenderScope is dropped, its effects are disposed. We keep them alive
    /// for the lifetime of the window.
    render_scopes: std::collections::HashMap<WindowId, Rc<RefCell<RenderScope>>>,
    /// The DevTools window ID, if open.
    devtools_window: Option<WindowId>,
    /// The window being inspected by DevTools.
    devtools_target: Option<WindowId>,
    /// Last time DevTools was refreshed.
    last_devtools_refresh: Option<Instant>,
}

impl FineGrainedRuntime {
    pub fn new() -> Self {
        Self {
            window_manager: WindowManager::new(),
            menu_manager: MenuManager::new(),
            proxy: None,
            pending_windows: Vec::new(),
            document_adapters: std::collections::HashMap::new(),
            render_scopes: std::collections::HashMap::new(),
            devtools_window: None,
            devtools_target: None,
            last_devtools_refresh: None,
        }
    }

    /// Add a pending window with its component.
    pub fn add_pending_window(&mut self, props: WindowProps, component: ComponentFn) {
        self.pending_windows.push(PendingWindow { props, component });
    }

    /// Create pending windows - this is where components actually run.
    fn create_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
        let Some(proxy) = self.proxy.clone() else {
            tracing::error!("No event loop proxy available");
            return;
        };

        // Take pending windows to process
        let pending = std::mem::take(&mut self.pending_windows);

        for PendingWindow { props, component } in pending {
            // Create window with minimal HTML structure
            let minimal_html = create_minimal_html(&props);

            match self.window_manager.create_window(
                event_loop,
                proxy.clone(),
                props.clone(),
                minimal_html,
            ) {
                Ok(window_id) => {
                    tracing::debug!("Created window {:?}: {}", window_id, props.title);

                    // Get shared reference to the blitz document
                    let doc_rc = self.window_manager.get_shared_document(window_id);
                    let Some(doc_rc) = doc_rc else {
                        tracing::error!("Failed to get document for window {:?}", window_id);
                        continue;
                    };

                    // Create the document adapter
                    let adapter = Rc::new(RefCell::new(BlitzDocumentAdapter::new(Rc::downgrade(&doc_rc))));
                    self.document_adapters.insert(window_id, adapter.clone());

                    // Set the current doc adapter for theme updates
                    set_current_doc_adapter(adapter.clone());

                    // Get body ID for the RenderScope
                    let body_id = adapter.borrow().body_id();

                    // Create RenderScope pointing to the blitz document adapter
                    let scope = Rc::new(RefCell::new(RenderScope::new(
                        adapter.clone() as Rc<RefCell<dyn DomDocument>>,
                        body_id,
                    )));

                    // Set thread-local context for widgets
                    set_render_scope(scope.clone());

                    // Begin render context for hooks
                    begin_render();

                    // Run the component - this builds DOM directly in blitz
                    let root = {
                        let mut scope_ref = scope.borrow_mut();
                        component(&mut scope_ref)
                    };

                    // Append root to body
                    {
                        adapter.borrow_mut().append_child(body_id, root.node_id());
                    }

                    // End render context
                    end_render();

                    // Clear thread-local context
                    clear_render_scope();

                    // Store the render scope to keep effects alive for the window's lifetime
                    self.render_scopes.insert(window_id, scope);

                    // Resolve layout now that DOM is built
                    if let Some(managed) = self.window_manager.get_mut(window_id) {
                        managed.resolve_and_request_redraw();
                    }

                    // Clear dirty state after initial resolve - the initial resolve
                    // does full style computation, not incremental. We only want
                    // incremental updates for subsequent changes.
                    adapter.borrow_mut().take_dirty_nodes();
                    adapter.borrow().clear_dirty();

                    tracing::debug!("Initialized window {:?} with direct DOM building", window_id);
                }
                Err(e) => {
                    tracing::error!("Failed to create window: {}", e);
                }
            }
        }
    }

    /// Check if any document is dirty and resolve layout if needed.
    fn resolve_dirty_documents(&mut self) {
        for (&window_id, adapter) in &self.document_adapters {
            let is_dirty = adapter.borrow().is_dirty();
            tracing::debug!("resolve_dirty_documents: window={:?}, is_dirty={}", window_id, is_dirty);
            if is_dirty {
                // Get dirty node IDs for marking ancestors
                let dirty_nodes = adapter.borrow_mut().take_dirty_nodes();
                let dirty_node_ids: Vec<usize> = dirty_nodes.iter().map(|n| n.0).collect();
                tracing::info!("resolve_dirty_documents: {} dirty nodes: {:?}", dirty_node_ids.len(), &dirty_node_ids[..dirty_node_ids.len().min(10)]);

                // Mark ancestors dirty and resolve layout.
                // This is needed for blitz's incremental style system to pick up style changes.
                // Since we initialize styles on element creation (in BlitzDocumentAdapter::create_element),
                // all nodes should have styles and mark_ancestors_dirty should not panic.
                if let Some(managed) = self.window_manager.get_mut(window_id) {
                    managed.finish_reactive_updates_with_dirty(&dirty_node_ids);
                }

                // Drop tombstoned nodes AFTER resolve() completes.
                // This ensures blitz never encounters stale slab keys during
                // style resolution or paint traversal.
                adapter.borrow().drop_tombstoned_nodes();

                adapter.borrow().clear_dirty();
            }
        }
    }

    /// Toggle the DevTools window.
    fn toggle_devtools(&mut self, event_loop: &ActiveEventLoop, source_window: WindowId) {
        // If DevTools is already open, close it
        if let Some(devtools_id) = self.devtools_window.take() {
            tracing::info!("Closing DevTools window");
            if let Some(mut window) = self.window_manager.close_window(devtools_id) {
                window.suspend();
            }
            self.devtools_target = None;
            self.last_devtools_refresh = None;
            return;
        }

        // Create a new DevTools window
        tracing::info!("Opening DevTools window");
        self.devtools_target = Some(source_window);

        let html = self.generate_devtools_html();
        let props = WindowProps {
            title: "Rinch DevTools".into(),
            width: 400,
            height: 600,
            x: None,
            y: None,
            borderless: false,
            resizable: true,
            visible: true,
            ..Default::default()
        };

        let proxy = self.proxy.clone().expect("Proxy should be set");
        match self.window_manager.create_window(event_loop, proxy, props, html) {
            Ok(window_id) => {
                self.devtools_window = Some(window_id);
                self.last_devtools_refresh = Some(Instant::now());
                if let Some(window) = self.window_manager.get_mut(window_id) {
                    window.resume();
                }
            }
            Err(e) => {
                tracing::error!("Failed to create DevTools window: {:?}", e);
            }
        }
    }

    /// Generate the HTML content for the DevTools window.
    fn generate_devtools_html(&self) -> String {
        use rinch_core::hooks::get_hooks_debug_info;

        let hooks_info = get_hooks_debug_info();
        let hooks_html: String = if hooks_info.is_empty() {
            r#"<p style="color: #808080;">No hooks registered.</p>"#.to_string()
        } else {
            hooks_info
                .iter()
                .enumerate()
                .map(|(i, info)| {
                    // HTML-escape the type strings (they may contain & < >)
                    let hook_type = html_escape(&info.hook_type);
                    let value_type = html_escape(&info.value_type);
                    format!(
                        r#"<div class="hook-item">
                            <span class="hook-index">#{}</span>
                            <span class="hook-type">{}</span>
                            <span class="hook-value-type">{}</span>
                        </div>"#,
                        i, hook_type, value_type
                    )
                })
                .collect()
        };

        // Get performance stats from the target window if available
        let perf_html = if let Some(target_id) = self.devtools_target {
            if let Some(window) = self.window_manager.get(target_id) {
                let fps = window.perf_stats.fps();
                let frame_ms = window.perf_stats.avg_frame_ms();
                let render_ms = window.perf_stats.avg_render_ms();
                let status = if window.perf_stats.is_idle() { "idle" } else { "active" };
                format!(
                    r#"<div class="perf-stats">
                        <div class="perf-row"><span class="perf-label">Status:</span> <span class="perf-value">{}</span></div>
                        <div class="perf-row"><span class="perf-label">FPS:</span> <span class="perf-value">{:.1}</span></div>
                        <div class="perf-row"><span class="perf-label">Frame:</span> <span class="perf-value">{:.2}ms</span></div>
                        <div class="perf-row"><span class="perf-label">Render:</span> <span class="perf-value">{:.2}ms</span></div>
                    </div>"#,
                    status, fps, frame_ms, render_ms
                )
            } else {
                r#"<p style="color: #808080;">No target window.</p>"#.to_string()
            }
        } else {
            r#"<p style="color: #808080;">No target window.</p>"#.to_string()
        };

        format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <title>Rinch DevTools</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: 'Consolas', 'Monaco', 'Courier New', monospace;
            font-size: 12px;
            background: #1e1e1e;
            color: #d4d4d4;
            padding: 0;
        }}
        .header {{
            background: #323233;
            padding: 8px 12px;
            border-bottom: 1px solid #3c3c3c;
            font-weight: bold;
            color: #ffffff;
        }}
        .tabs {{
            display: flex;
            background: #252526;
            border-bottom: 1px solid #3c3c3c;
        }}
        .tab {{
            padding: 8px 16px;
            cursor: pointer;
            border-bottom: 2px solid transparent;
        }}
        .tab.active {{
            background: #1e1e1e;
            border-bottom-color: #007acc;
            color: #ffffff;
        }}
        .panel {{
            padding: 12px;
        }}
        .section {{
            margin-bottom: 16px;
        }}
        .section-title {{
            color: #4ec9b0;
            font-weight: bold;
            margin-bottom: 8px;
            padding-bottom: 4px;
            border-bottom: 1px solid #3c3c3c;
        }}
        .hook-item {{
            background: #2d2d2d;
            padding: 8px;
            margin-bottom: 4px;
            border-radius: 4px;
            display: flex;
            gap: 8px;
            align-items: center;
        }}
        .hook-index {{
            color: #808080;
            min-width: 24px;
        }}
        .hook-type {{
            color: #569cd6;
            font-weight: bold;
        }}
        .hook-value-type {{
            color: #ce9178;
            font-size: 11px;
        }}
        .info {{
            color: #808080;
            font-size: 11px;
            margin-top: 8px;
        }}
        .shortcut {{
            background: #3c3c3c;
            padding: 2px 6px;
            border-radius: 3px;
            font-size: 11px;
        }}
        .shortcuts {{
            display: flex;
            flex-direction: column;
            gap: 4px;
        }}
        .shortcut-row {{
            display: flex;
            align-items: center;
            gap: 8px;
        }}
        .shortcut-desc {{
            color: #d4d4d4;
        }}
        .perf-stats {{
            background: #2d2d2d;
            padding: 12px;
            border-radius: 4px;
        }}
        .perf-row {{
            display: flex;
            justify-content: space-between;
            margin-bottom: 4px;
        }}
        .perf-label {{
            color: #9cdcfe;
        }}
        .perf-value {{
            color: #b5cea8;
            font-weight: bold;
        }}
    </style>
</head>
<body>
    <div class="header">Rinch DevTools</div>
    <div class="tabs">
        <div class="tab active">Performance</div>
        <div class="tab">Hooks</div>
        <div class="tab">Shortcuts</div>
    </div>
    <div class="panel">
        <div class="section">
            <div class="section-title">Performance</div>
            {perf_html}
        </div>
        <div class="section">
            <div class="section-title">Hooks ({} registered)</div>
            {hooks_html}
        </div>
        <div class="section">
            <div class="section-title">Keyboard Shortcuts</div>
            <div class="shortcuts">
                <div class="shortcut-row"><span class="shortcut">Alt+D</span> <span class="shortcut-desc">Toggle layout debug</span></div>
                <div class="shortcut-row"><span class="shortcut">Alt+I</span> <span class="shortcut-desc">Toggle inspect mode</span></div>
                <div class="shortcut-row"><span class="shortcut">Alt+P</span> <span class="shortcut-desc">Toggle perf stats</span></div>
                <div class="shortcut-row"><span class="shortcut">Alt+T</span> <span class="shortcut-desc">Print Taffy tree</span></div>
                <div class="shortcut-row"><span class="shortcut">Alt+S</span> <span class="shortcut-desc">Print scroll offsets</span></div>
                <div class="shortcut-row"><span class="shortcut">F12</span> <span class="shortcut-desc">Toggle DevTools</span></div>
            </div>
        </div>
    </div>
</body>
</html>"#,
            hooks_info.len()
        )
    }

    /// Refresh the DevTools window content with current stats.
    fn refresh_devtools(&mut self) {
        let Some(devtools_id) = self.devtools_window else {
            return;
        };

        // Update the last refresh time
        self.last_devtools_refresh = Some(Instant::now());

        // Generate new HTML content
        let html = self.generate_devtools_html();

        // Update the devtools window content
        if let Some(window) = self.window_manager.get_mut(devtools_id) {
            window.update_content(html);
        }
    }
}

impl Default for FineGrainedRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationHandler<RinchEvent> for FineGrainedRuntime {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.create_pending_windows(event_loop);
        self.window_manager.resume_all();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.window_manager.suspend_all();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if matches!(event, WindowEvent::CloseRequested) {
            tracing::info!("Window {:?} close requested", window_id);

            // Clean up document adapter and render scope
            self.document_adapters.remove(&window_id);
            self.render_scopes.remove(&window_id);

            self.window_manager.close_window(window_id);

            if !self.window_manager.has_windows() {
                event_loop.exit();
            }
            return;
        }

        if let Some(window) = self.window_manager.get_mut(window_id) {
            window.handle_event(event);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: RinchEvent) {
        match event {
            RinchEvent::Poll { window_id } => {
                if let Some(window) = self.window_manager.get_mut(window_id) {
                    window.poll();
                }
            }
            RinchEvent::ReRender => {
                // Check for dirty documents and resolve
                self.resolve_dirty_documents();
            }
            RinchEvent::ToggleDevTools { source_window } => {
                // Ignore F12 from the DevTools window itself to prevent immediate close
                if Some(source_window) != self.devtools_window {
                    self.toggle_devtools(_event_loop, source_window);
                }
            }
            RinchEvent::RefreshDevTools => {
                self.refresh_devtools();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Resolve any dirty documents
        self.resolve_dirty_documents();

        // Tick scroll animations
        let any_animating = self.window_manager.tick_scroll_animations();

        // Refresh DevTools periodically if open (every 500ms)
        let devtools_needs_refresh = if self.devtools_window.is_some() {
            match self.last_devtools_refresh {
                Some(last) => last.elapsed().as_millis() >= 500,
                None => true,
            }
        } else {
            false
        };

        if devtools_needs_refresh {
            self.refresh_devtools();
        }

        // Keep polling if animating or devtools is open (for live updates)
        if any_animating || self.devtools_window.is_some() {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// Create minimal HTML for initial window creation.
///
/// The actual DOM content will be built directly into this document.
fn create_minimal_html(props: &WindowProps) -> String {
    // Include theme CSS if available
    #[cfg(feature = "theme")]
    let theme_css = rinch_core::get_current_theme_css().unwrap_or_default();
    #[cfg(not(feature = "theme"))]
    let theme_css = String::new();

    let style_block = if theme_css.is_empty() {
        String::new()
    } else {
        format!("<style id=\"rinch-theme\">{}</style>", theme_css)
    };

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>{}</title>
    {}
    <style>
        html, body {{
            margin: 0;
            padding: 0;
            width: 100%;
            height: 100%;
            font-family: system-ui, -apple-system, sans-serif;
        }}
    </style>
</head>
<body>
</body>
</html>"#,
        props.title, style_block
    )
}

// ============================================================================
// Public API
// ============================================================================

/// A fine-grained window (kept for API compatibility).
/// In the new architecture, this is just a thin wrapper.
pub struct FineGrainedWindow {
    pub props: WindowProps,
}

impl FineGrainedWindow {
    pub fn new(props: WindowProps) -> Self {
        Self { props }
    }
}

/// Builder for fine-grained applications.
pub struct FineGrainedApp {
    windows: Vec<(WindowProps, ComponentFn)>,
}

impl FineGrainedApp {
    /// Create a new fine-grained app builder.
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
        }
    }

    /// Add a window with a fine-grained component.
    pub fn window<F>(mut self, props: WindowProps, component: F) -> Self
    where
        F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    {
        self.windows.push((props, Box::new(component)));
        self
    }

    /// Run the application.
    pub fn run(self) {
        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

        // Clear stale state
        clear_handlers();
        clear_hooks();

        // Create runtime
        let mut runtime = FineGrainedRuntime::new();

        // Add pending windows (components will run when windows are created)
        for (props, component) in self.windows {
            runtime.add_pending_window(props, component);
        }

        // Create event loop
        let event_loop = EventLoop::<RinchEvent>::with_user_event()
            .build()
            .expect("Failed to create event loop");

        let proxy = event_loop.create_proxy();
        runtime.proxy = Some(proxy.clone());

        // Set up signal change callback to trigger re-render
        let render_proxy = proxy.clone();
        rinch_core::set_on_signal_change(move || {
            let _ = render_proxy.send_event(RinchEvent::ReRender);
        });

        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop.run_app(&mut runtime).expect("Event loop error");
    }
}

impl Default for FineGrainedApp {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to run a single-window fine-grained app.
pub fn run_fine_grained<F>(title: &str, width: u32, height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let props = WindowProps {
        title: title.to_string(),
        width,
        height,
        ..Default::default()
    };

    FineGrainedApp::new().window(props, component).run()
}

/// Escape HTML special characters in a string.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
