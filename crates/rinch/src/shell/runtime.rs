//! Runtime - application event loop and lifecycle.

use crate::menu::MenuManager;
use muda::MenuEvent;
use rinch_core::element::{Element, WindowProps};
use rinch_core::events::{clear_handlers, dispatch_event, EventHandlerId};
use rinch_core::hooks::{begin_render, clear_hooks, end_render};
use std::cell::RefCell;
use std::rc::Rc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use super::window_manager::WindowManager;

/// Events used internally by rinch.
#[derive(Debug, Clone)]
pub enum RinchEvent {
    /// Poll a window for document updates.
    Poll { window_id: WindowId },
    /// A menu item was activated.
    MenuEvent(muda::MenuId),
    /// Request a re-render of all windows.
    ReRender,
    /// An element was clicked (with handler ID and source window).
    ElementClicked { handler_id: EventHandlerId, window_id: WindowId },
    /// Toggle the DevTools window.
    ToggleDevTools { source_window: WindowId },
    /// Update DevTools with hovered element info.
    UpdateDevToolsHover { element_info: Option<HoveredElementInfo> },
    /// A keyboard shortcut was pressed - check against menu shortcuts.
    KeyboardShortcut {
        ctrl: bool,
        meta: bool,
        alt: bool,
        shift: bool,
        key: winit::keyboard::KeyCode,
    },
    /// Process pending window requests (open/close).
    ProcessWindowRequests,
    /// Minimize a window.
    MinimizeWindow { window_id: WindowId },
    /// Toggle maximize state of a window.
    ToggleMaximizeWindow { window_id: WindowId },
    /// Close a window (from window controls).
    CloseWindowControl { window_id: WindowId },
}

/// Information about a hovered element for DevTools display.
#[derive(Debug, Clone)]
pub struct HoveredElementInfo {
    /// The element's tag name (e.g., "div", "button").
    pub tag_name: String,
    /// The element's id attribute, if any.
    pub id: Option<String>,
    /// The element's class attribute, if any.
    pub classes: Option<String>,
    /// Key style properties.
    pub styles: Vec<(String, String)>,
    /// Layout information.
    pub layout: ElementLayout,
}

/// Layout information for an element.
#[derive(Debug, Clone)]
pub struct ElementLayout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Pending window to be created when the event loop resumes.
struct PendingWindow {
    props: WindowProps,
    html_content: String,
}

/// Shared state for the render context.
struct RenderContextInner {
    proxy: Option<EventLoopProxy<RinchEvent>>,
    needs_render: bool,
}

/// Context for triggering re-renders from anywhere in the app.
#[derive(Clone)]
pub struct RenderContext {
    inner: Rc<RefCell<RenderContextInner>>,
}

impl RenderContext {
    fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(RenderContextInner {
                proxy: None,
                needs_render: false,
            })),
        }
    }

    fn set_proxy(&self, proxy: EventLoopProxy<RinchEvent>) {
        self.inner.borrow_mut().proxy = Some(proxy);
    }

    /// Request a re-render of the UI.
    pub fn request_render(&self) {
        let mut inner = self.inner.borrow_mut();
        if !inner.needs_render {
            inner.needs_render = true;
            if let Some(proxy) = &inner.proxy {
                let _ = proxy.send_event(RinchEvent::ReRender);
            }
        }
    }

    fn clear_render_flag(&self) {
        self.inner.borrow_mut().needs_render = false;
    }
}

// Thread-local render context for triggering re-renders
thread_local! {
    static RENDER_CONTEXT: RefCell<Option<RenderContext>> = const { RefCell::new(None) };
}

/// Request a re-render of the UI.
///
/// Call this after modifying state that affects the UI.
pub fn request_render() {
    RENDER_CONTEXT.with(|ctx| {
        if let Some(ctx) = ctx.borrow().as_ref() {
            ctx.request_render();
        }
    });
}

/// The rinch application runtime.
pub struct Runtime {
    window_manager: WindowManager,
    menu_manager: MenuManager,
    pending_windows: Vec<PendingWindow>,
    pending_menu: Option<Element>,
    proxy: Option<EventLoopProxy<RinchEvent>>,
    menus_initialized: bool,
    app_fn: Option<Box<dyn Fn() -> Element>>,
    render_context: RenderContext,
    #[cfg(feature = "hot-reload")]
    hot_reloader: Option<super::hot_reload::HotReloader>,
    /// The DevTools window ID, if open.
    devtools_window: Option<WindowId>,
    /// The window being inspected by DevTools.
    devtools_target: Option<WindowId>,
    /// Current hovered element info for DevTools display.
    hovered_element: Option<HoveredElementInfo>,
    /// Mapping from WindowHandle to winit WindowId for programmatic window management.
    window_handles: std::collections::HashMap<crate::windows::WindowHandle, WindowId>,
    /// Reverse mapping from winit WindowId to WindowHandle.
    window_ids_to_handles: std::collections::HashMap<WindowId, crate::windows::WindowHandle>,
}

impl Runtime {
    fn new() -> Self {
        let render_context = RenderContext::new();

        // Set global render context
        RENDER_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = Some(render_context.clone());
        });

        Self {
            window_manager: WindowManager::new(),
            menu_manager: MenuManager::new(),
            pending_windows: Vec::new(),
            pending_menu: None,
            proxy: None,
            menus_initialized: false,
            app_fn: None,
            render_context,
            #[cfg(feature = "hot-reload")]
            hot_reloader: None,
            devtools_window: None,
            devtools_target: None,
            hovered_element: None,
            window_handles: std::collections::HashMap::new(),
            window_ids_to_handles: std::collections::HashMap::new(),
        }
    }

    /// Enable hot reloading with the given configuration.
    ///
    /// This must be called after the event loop proxy is set.
    #[cfg(feature = "hot-reload")]
    pub fn enable_hot_reload(&mut self, config: super::hot_reload::HotReloadConfig) {
        if let Some(proxy) = &self.proxy {
            match super::hot_reload::HotReloader::new(proxy.clone(), config) {
                Ok(reloader) => {
                    tracing::info!("Hot reload enabled");
                    self.hot_reloader = Some(reloader);
                }
                Err(e) => {
                    tracing::error!("Failed to enable hot reload: {:?}", e);
                }
            }
        } else {
            tracing::warn!("Cannot enable hot reload: event loop proxy not set");
        }
    }

    /// Store the app function for re-rendering.
    fn set_app_fn<F: Fn() -> Element + 'static>(&mut self, app: F) {
        self.app_fn = Some(Box::new(app));
    }

    /// Queue a window to be created.
    fn queue_window(&mut self, props: WindowProps, html_content: String) {
        self.pending_windows.push(PendingWindow { props, html_content });
    }

    /// Process the element tree and extract windows/menus.
    fn process_element(&mut self, element: Element) {
        match element {
            Element::Window(props, children) => {
                let html = children_to_html(&children);
                self.queue_window(props, html);
            }
            Element::AppMenu(_, _) => {
                // Store the menu element for later building
                self.pending_menu = Some(element);
            }
            Element::Fragment(children) => {
                for child in children {
                    self.process_element(child);
                }
            }
            _ => {}
        }
    }

    fn create_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
        let Some(proxy) = self.proxy.clone() else {
            tracing::error!("No event loop proxy available");
            return;
        };

        for pending in self.pending_windows.drain(..) {
            match self.window_manager.create_window(
                event_loop,
                proxy.clone(),
                pending.props.clone(),
                pending.html_content,
            ) {
                Ok(id) => {
                    tracing::info!("Created window {:?}: {}", id, pending.props.title);
                }
                Err(e) => {
                    tracing::error!("Failed to create window: {}", e);
                }
            }
        }
    }

    fn initialize_menus(&mut self) {
        if self.menus_initialized {
            return;
        }

        // Build menu from pending element
        if let Some(menu_element) = self.pending_menu.take() {
            self.menu_manager.build_from_element(&menu_element);
        }

        // Initialize menu for macOS (app-level menu bar)
        #[cfg(target_os = "macos")]
        {
            self.menu_manager.init_for_app();
            tracing::info!("Initialized macOS app menu");
        }

        // Initialize menu for each window (Windows/Linux)
        // Skip borderless windows - native menus require window decorations
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        {
            for (_, window) in self.window_manager.windows_iter() {
                if window.props.borderless {
                    tracing::info!(
                        "Skipping native menu for borderless window (native menus require decorations)"
                    );
                    continue;
                }
                self.menu_manager.init_for_window(&window.window);
                tracing::info!("Initialized menu for window");
            }
        }

        self.menus_initialized = true;
    }

    fn poll_menu_events(&mut self) {
        // Poll for menu events
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if self.menu_manager.handle_event(&event) {
                // Callback was invoked - request re-render in case state changed
                self.render_context.request_render();
            }
        }
    }

    /// Process any pending window requests (open/close).
    fn process_window_requests(&mut self, event_loop: &ActiveEventLoop) {
        use crate::windows::{take_window_requests, WindowRequest};

        let requests = take_window_requests();
        if requests.is_empty() {
            return;
        }

        let Some(proxy) = self.proxy.clone() else {
            tracing::error!("No event loop proxy available for window requests");
            return;
        };

        for request in requests {
            match request {
                WindowRequest::Open(open_req) => {
                    match self.window_manager.create_window(
                        event_loop,
                        proxy.clone(),
                        open_req.props.clone(),
                        open_req.html_content,
                    ) {
                        Ok(window_id) => {
                            tracing::info!(
                                "Opened window {:?} with handle {:?}: {}",
                                window_id,
                                open_req.handle,
                                open_req.props.title
                            );
                            // Track the handle <-> window_id mappings
                            self.window_handles.insert(open_req.handle, window_id);
                            self.window_ids_to_handles.insert(window_id, open_req.handle);
                            // Resume the window to start rendering
                            if let Some(window) = self.window_manager.get_mut(window_id) {
                                window.resume();
                                // Set initial window state
                                Self::update_window_state_for_handle(open_req.handle, window);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to open window: {:?}", e);
                        }
                    }
                }
                WindowRequest::Close(close_req) => {
                    if let Some(window_id) = self.window_handles.remove(&close_req.handle) {
                        tracing::info!(
                            "Closing window {:?} with handle {:?}",
                            window_id,
                            close_req.handle
                        );
                        self.window_ids_to_handles.remove(&window_id);
                        crate::windows::remove_window_state(close_req.handle);
                        self.window_manager.close_window(window_id);
                    } else {
                        tracing::warn!(
                            "Attempted to close unknown window handle {:?}",
                            close_req.handle
                        );
                    }
                }
            }
        }
    }

    /// Update the window state for a given handle.
    fn update_window_state_for_handle(
        handle: crate::windows::WindowHandle,
        managed_window: &super::window_manager::ManagedWindow,
    ) {
        let window = &managed_window.window;
        let size = window.inner_size();
        let position = window.outer_position().unwrap_or_default();
        let maximized = window.is_maximized();
        let minimized = window.is_minimized().unwrap_or(false);

        let state = crate::windows::WindowState {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            maximized,
            minimized,
        };

        crate::windows::update_window_state(handle, state);
    }

    /// Re-render all windows by re-running the app function.
    fn re_render(&mut self) {
        let Some(app_fn) = &self.app_fn else {
            tracing::warn!("No app function stored for re-render");
            return;
        };

        // Clear old event handlers
        clear_handlers();

        // Re-run the app function to get new element tree
        begin_render();
        let root = app_fn();
        end_render();

        // Extract HTML for each window
        let mut window_contents: Vec<(WindowProps, String)> = Vec::new();

        fn extract_windows(element: Element, contents: &mut Vec<(WindowProps, String)>) {
            match element {
                Element::Window(props, children) => {
                    let html = children_to_html(&children);
                    contents.push((props, html));
                }
                Element::Fragment(children) => {
                    for child in children {
                        extract_windows(child, contents);
                    }
                }
                _ => {}
            }
        }

        extract_windows(root, &mut window_contents);

        // Update each window's content
        // For now, we assume windows are in the same order
        let window_ids: Vec<WindowId> = self.window_manager.window_ids();

        for (id, (_props, html)) in window_ids.iter().zip(window_contents.iter()) {
            if let Some(window) = self.window_manager.get_mut(*id) {
                window.update_content(html.clone());
            }
        }

        self.render_context.clear_render_flag();
    }

    /// Handle a click event by dispatching to the registered handler.
    fn handle_element_click(&mut self, handler_id: EventHandlerId, window_id: WindowId) {
        tracing::debug!("Dispatching click event to handler {:?} from window {:?}", handler_id, window_id);

        // Track the current window so event handlers can call window control functions
        crate::windows::set_current_window_id(Some(window_id));

        if dispatch_event(handler_id) {
            // Handler was called - request re-render in case state changed
            self.render_context.request_render();
        }

        // Clear current window tracking
        crate::windows::set_current_window_id(None);
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
            transparent: false,
            always_on_top: true,
            visible: true,
        };

        let proxy = self.proxy.clone().expect("Proxy should be set");
        match self.window_manager.create_window(event_loop, proxy, props, html) {
            Ok(window_id) => {
                self.devtools_window = Some(window_id);
                if let Some(window) = self.window_manager.get_mut(window_id) {
                    window.resume();
                }
            }
            Err(e) => {
                tracing::error!("Failed to create DevTools window: {:?}", e);
            }
        }
    }

    /// Generate an HTML representation of the DOM tree for the target window.
    fn generate_dom_tree_html(&self) -> String {
        let Some(target_id) = self.devtools_target else {
            return r#"<p style="color: #808080;">No window selected.</p>"#.to_string();
        };

        let Some(window) = self.window_manager.get(target_id) else {
            return r#"<p style="color: #808080;">Target window not found.</p>"#.to_string();
        };

        let inner = window.doc.inner();
        let mut html = String::new();

        // Start from the root and walk the tree
        fn render_node(
            inner: &blitz_dom::BaseDocument,
            node_id: usize,
            depth: usize,
            html: &mut String,
            max_depth: usize,
        ) {
            if depth > max_depth {
                return;
            }

            let Some(node) = inner.get_node(node_id) else {
                return;
            };

            let indent = "  ".repeat(depth);

            if let Some(element) = node.element_data() {
                let tag = &element.name.local;

                // Get id and class for display
                let mut id_str = String::new();
                let mut class_str = String::new();
                for attr in element.attrs() {
                    let name = attr.name.local.as_ref();
                    if name == "id" {
                        id_str = format!(r#" <span class="attr-id">#{}</span>"#, attr.value);
                    } else if name == "class" {
                        let classes: Vec<&str> = attr.value.split_whitespace().collect();
                        if !classes.is_empty() {
                            class_str = format!(
                                r#" <span class="attr-class">.{}</span>"#,
                                classes.join(".")
                            );
                        }
                    }
                }

                html.push_str(&format!(
                    r#"{}<div class="tree-node"><span class="tag">&lt;{}&gt;</span>{}{}</div>"#,
                    indent, tag, id_str, class_str
                ));
                html.push('\n');

                // Recurse into children
                for &child_id in &node.children {
                    render_node(inner, child_id, depth + 1, html, max_depth);
                }
            } else if node.is_text_node() {
                let text = node.text_content();
                let trimmed = text.trim();
                if !trimmed.is_empty() && trimmed.len() < 50 {
                    html.push_str(&format!(
                        r#"{}<div class="tree-text">"{}"</div>"#,
                        indent,
                        html_escape(trimmed)
                    ));
                    html.push('\n');
                }
            } else {
                // Other node types - just recurse
                for &child_id in &node.children {
                    render_node(inner, child_id, depth + 1, html, max_depth);
                }
            }
        }

        // Helper to escape HTML
        fn html_escape(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        }

        // Get root node and render
        if let Some(root) = inner.get_node(0) {
            for &child_id in &root.children {
                render_node(&inner, child_id, 0, &mut html, 10);
            }
        }

        if html.is_empty() {
            r#"<p style="color: #808080;">Empty document.</p>"#.to_string()
        } else {
            format!(r#"<div class="dom-tree">{}</div>"#, html)
        }
    }

    /// Generate HTML content for the DevTools window.
    fn generate_devtools_html(&self) -> String {
        use rinch_core::get_hooks_debug_info;

        let hooks_info = get_hooks_debug_info();
        let hooks_html: String = if hooks_info.is_empty() {
            r#"<p style="color: #808080;">No hooks registered.</p>"#.to_string()
        } else {
            hooks_info
                .iter()
                .enumerate()
                .map(|(i, info)| {
                    format!(
                        r#"<div class="hook-item">
                            <span class="hook-index">#{}</span>
                            <span class="hook-type">{}</span>
                            <span class="hook-value-type">{}</span>
                        </div>"#,
                        i, info.hook_type, info.value_type
                    )
                })
                .collect()
        };

        // Generate element info section
        let element_html = match &self.hovered_element {
            Some(info) => {
                let id_str = info.id.as_deref().unwrap_or("-");
                let classes_str = info.classes.as_deref().unwrap_or("-");

                // Generate styles HTML
                let styles_html: String = if info.styles.is_empty() {
                    String::new()
                } else {
                    let style_items: String = info
                        .styles
                        .iter()
                        .map(|(name, value)| {
                            format!(
                                r#"<div class="style-item"><span class="style-name">{}</span>: <span class="style-value">{}</span></div>"#,
                                name, value
                            )
                        })
                        .collect();
                    format!(
                        r#"<div class="element-styles">
                            <div class="layout-title">Computed Styles</div>
                            {}
                        </div>"#,
                        style_items
                    )
                };

                format!(
                    r#"<div class="element-info">
                        <div class="element-tag">&lt;{}&gt;</div>
                        <div class="element-attr"><span class="attr-name">id:</span> <span class="attr-value">{}</span></div>
                        <div class="element-attr"><span class="attr-name">class:</span> <span class="attr-value">{}</span></div>
                        <div class="element-layout">
                            <div class="layout-title">Layout</div>
                            <div class="layout-grid">
                                <div>x: {:.0}</div>
                                <div>y: {:.0}</div>
                                <div>w: {:.0}</div>
                                <div>h: {:.0}</div>
                            </div>
                        </div>
                        {}
                    </div>"#,
                    info.tag_name,
                    id_str,
                    classes_str,
                    info.layout.x,
                    info.layout.y,
                    info.layout.width,
                    info.layout.height,
                    styles_html
                )
            }
            None => r#"<p style="color: #808080;">Enable inspect mode (Alt+I) and hover over elements.</p>"#.to_string(),
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
        .element-info {{
            background: #2d2d2d;
            padding: 12px;
            border-radius: 4px;
        }}
        .element-tag {{
            color: #569cd6;
            font-size: 16px;
            font-weight: bold;
            margin-bottom: 8px;
        }}
        .element-attr {{
            margin-bottom: 4px;
        }}
        .attr-name {{
            color: #9cdcfe;
        }}
        .attr-value {{
            color: #ce9178;
        }}
        .element-layout {{
            margin-top: 12px;
            padding-top: 8px;
            border-top: 1px solid #3c3c3c;
        }}
        .layout-title {{
            color: #4ec9b0;
            font-size: 11px;
            margin-bottom: 4px;
        }}
        .layout-grid {{
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 4px;
            color: #b5cea8;
        }}
        .element-styles {{
            margin-top: 12px;
            padding-top: 8px;
            border-top: 1px solid #3c3c3c;
        }}
        .style-item {{
            margin-bottom: 2px;
        }}
        .style-name {{
            color: #9cdcfe;
        }}
        .style-value {{
            color: #ce9178;
        }}
        .dom-tree {{
            background: #252526;
            padding: 8px;
            border-radius: 4px;
            max-height: 200px;
            overflow-y: auto;
            font-size: 11px;
        }}
        .tree-node {{
            padding: 2px 0;
            white-space: nowrap;
        }}
        .tree-node .tag {{
            color: #569cd6;
        }}
        .tree-node .attr-id {{
            color: #9cdcfe;
        }}
        .tree-node .attr-class {{
            color: #ce9178;
        }}
        .tree-text {{
            color: #808080;
            font-style: italic;
            padding: 2px 0;
        }}
    </style>
</head>
<body>
    <div class="header">Rinch DevTools</div>
    <div class="tabs">
        <div class="tab active">Elements</div>
        <div class="tab">Hooks</div>
    </div>
    <div class="panel">
        <div class="section">
            <div class="section-title">DOM Tree</div>
            {}
        </div>
        <div class="section">
            <div class="section-title">Hovered Element</div>
            {}
        </div>
        <div class="section">
            <div class="section-title">Registered Hooks ({} total)</div>
            {}
        </div>
        <div class="section">
            <div class="section-title">Keyboard Shortcuts</div>
            <div class="shortcuts">
                <div class="shortcut-row">
                    <span class="shortcut">F12</span>
                    <span class="shortcut-desc">Toggle DevTools</span>
                </div>
                <div class="shortcut-row">
                    <span class="shortcut">Alt+D</span>
                    <span class="shortcut-desc">Toggle layout debug</span>
                </div>
                <div class="shortcut-row">
                    <span class="shortcut">Alt+I</span>
                    <span class="shortcut-desc">Toggle inspect mode</span>
                </div>
                <div class="shortcut-row">
                    <span class="shortcut">Alt+T</span>
                    <span class="shortcut-desc">Print Taffy tree</span>
                </div>
                <div class="shortcut-row">
                    <span class="shortcut">Ctrl/Cmd + +/-/0</span>
                    <span class="shortcut-desc">Zoom in/out/reset</span>
                </div>
            </div>
        </div>
        <p class="info">Press F12 again to close this window.</p>
    </div>
</body>
</html>"#,
            self.generate_dom_tree_html(),
            element_html,
            hooks_info.len(),
            hooks_html
        )
    }
}

impl ApplicationHandler<RinchEvent> for Runtime {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Create any pending windows
        self.create_pending_windows(event_loop);

        // Initialize menus after windows are created
        self.initialize_menus();

        // Resume existing windows (activates rendering)
        self.window_manager.resume_all();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.window_manager.suspend_all();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        // Handle close request at runtime level
        if matches!(event, WindowEvent::CloseRequested) {
            tracing::info!("Window {:?} close requested", window_id);

            // Check if this is the DevTools window being closed
            if self.devtools_window == Some(window_id) {
                self.devtools_window = None;
                self.devtools_target = None;
            }

            // Clean up window state tracking if this is a programmatically opened window
            if let Some(handle) = self.window_ids_to_handles.remove(&window_id) {
                self.window_handles.remove(&handle);
                crate::windows::remove_window_state(handle);
            }

            self.window_manager.close_window(window_id);

            if !self.window_manager.has_windows() {
                event_loop.exit();
            }
            return;
        }

        // Track window state changes for programmatically opened windows
        if let Some(&handle) = self.window_ids_to_handles.get(&window_id) {
            match &event {
                WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
                    if let Some(window) = self.window_manager.get(window_id) {
                        Self::update_window_state_for_handle(handle, window);
                    }
                }
                _ => {}
            }
        }

        // Forward other events to the window
        if let Some(window) = self.window_manager.get_mut(window_id) {
            // Check for mouse down events that might trigger window dragging
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                // Check if we should start window dragging
                if window.should_drag_window() {
                    window.start_drag();
                    return; // Don't process further - drag takes over
                }
            }

            // Check for click events that might trigger handlers
            if let WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } = &event
            {
                // Check if we clicked on an element with a handler
                if let Some(handler_id) = window.get_clicked_handler() {
                    if let Some(proxy) = &self.proxy {
                        let _ = proxy.send_event(RinchEvent::ElementClicked { handler_id, window_id });
                    }
                }
            }

            window.handle_event(event);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RinchEvent) {
        match event {
            RinchEvent::Poll { window_id } => {
                if let Some(window) = self.window_manager.get_mut(window_id) {
                    window.poll();
                }
            }
            RinchEvent::MenuEvent(id) => {
                // Find the menu item and trigger its callback
                let event = muda::MenuEvent { id };
                if self.menu_manager.handle_event(&event) {
                    // Callback was invoked - request re-render
                    self.render_context.request_render();
                }
            }
            RinchEvent::ReRender => {
                tracing::debug!("Re-rendering...");
                self.re_render();
            }
            RinchEvent::ElementClicked { handler_id, window_id } => {
                self.handle_element_click(handler_id, window_id);
            }
            RinchEvent::ToggleDevTools { source_window } => {
                self.toggle_devtools(event_loop, source_window);
            }
            RinchEvent::UpdateDevToolsHover { element_info } => {
                self.hovered_element = element_info;
                // Update DevTools window content
                if let Some(devtools_id) = self.devtools_window {
                    let html = self.generate_devtools_html();
                    if let Some(window) = self.window_manager.get_mut(devtools_id) {
                        window.update_content(html);
                    }
                }
            }
            RinchEvent::KeyboardShortcut {
                ctrl,
                meta,
                alt,
                shift,
                key,
            } => {
                // Check if keyboard shortcut matches a menu item
                if let Some(menu_id) = self.menu_manager.match_shortcut(ctrl, meta, alt, shift, key)
                {
                    let event = muda::MenuEvent { id: menu_id };
                    if self.menu_manager.handle_event(&event) {
                        // Callback was invoked - request re-render
                        self.render_context.request_render();
                    }
                }
            }
            RinchEvent::ProcessWindowRequests => {
                self.process_window_requests(event_loop);
            }
            RinchEvent::MinimizeWindow { window_id } => {
                if let Some(window) = self.window_manager.get(window_id) {
                    window.window.set_minimized(true);
                }
            }
            RinchEvent::ToggleMaximizeWindow { window_id } => {
                if let Some(window) = self.window_manager.get(window_id) {
                    let is_maximized = window.window.is_maximized();
                    window.window.set_maximized(!is_maximized);
                }
            }
            RinchEvent::CloseWindowControl { window_id } => {
                // Clean up window state tracking if this is a programmatically opened window
                if let Some(handle) = self.window_ids_to_handles.remove(&window_id) {
                    self.window_handles.remove(&handle);
                    crate::windows::remove_window_state(handle);
                }

                self.window_manager.close_window(window_id);

                if !self.window_manager.has_windows() {
                    event_loop.exit();
                }
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Poll menu events
        self.poll_menu_events();

        // Poll hot reloader for file changes
        #[cfg(feature = "hot-reload")]
        if let Some(reloader) = &mut self.hot_reloader {
            reloader.poll();
        }
    }
}

/// Convert element children to an HTML string for blitz.
fn children_to_html(children: &[Element]) -> String {
    let mut html = String::new();
    for child in children {
        match child {
            Element::Html(content) => {
                html.push_str(content);
            }
            Element::Fragment(kids) => {
                html.push_str(&children_to_html(kids));
            }
            _ => {}
        }
    }
    html
}

/// Run the application with the given root element.
pub fn run<F>(app: F)
where
    F: Fn() -> Element + 'static,
{
    run_internal(app, false);
}

/// Run the application with hot reloading enabled.
///
/// When files in the `src` directory change, the UI will automatically re-render.
/// This is useful during development to see changes without restarting.
///
/// # Example
///
/// ```ignore
/// fn main() {
///     rinch::run_with_hot_reload(app);
/// }
/// ```
#[cfg(feature = "hot-reload")]
pub fn run_with_hot_reload<F>(app: F)
where
    F: Fn() -> Element + 'static,
{
    run_internal(app, true);
}

fn run_internal<F>(app: F, #[allow(unused)] enable_hot_reload: bool)
where
    F: Fn() -> Element + 'static,
{
    // Initialize tracing
    let _ = tracing_subscriber::fmt::try_init();

    // Clear any stale state from previous runs
    clear_handlers();
    clear_hooks();

    // Build the initial element tree
    begin_render();
    let root = app();
    end_render();

    // Create runtime and process elements
    let mut runtime = Runtime::new();
    runtime.set_app_fn(app);
    runtime.process_element(root);

    // Create event loop
    let event_loop = EventLoop::<RinchEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();
    runtime.proxy = Some(proxy.clone());
    runtime.render_context.set_proxy(proxy.clone());

    // Set proxy for window management API
    crate::windows::set_event_proxy(proxy);

    // Enable hot reload if requested
    #[cfg(feature = "hot-reload")]
    if enable_hot_reload {
        runtime.enable_hot_reload(super::hot_reload::HotReloadConfig::default());
    }

    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut runtime).expect("Event loop error");
}
