//! Platform-agnostic application logic.
//!
//! `RinchApp` holds the reactive document, scene graph, cursor state, devtools,
//! and all input-handling logic that is independent of the windowing backend.
//! The desktop shell (winit + wgpu) translates native events into
//! [`PlatformEvent`]s, feeds them to `RinchApp`, and processes the returned
//! [`AppAction`]s.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};
use rinch_core::events;
use rinch_core::hooks::{begin_render, end_render};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::byte_offset_from_position;
#[cfg(feature = "debug")]
use rinch_dom::text_query::{caret_position_for_offset_layout, glyph_bounds_for_offset_layout};
use rinch_platform::{
    AppAction, Instant, KeyCode, Modifiers, MouseButton, PlatformEvent, UserEvent,
};
use vello::Scene;

#[cfg(feature = "desktop")]
use crate::shell::devtools::DevToolsState;

#[cfg(feature = "debug")]
use {
    rinch_debug::{CommandReceiver, DebugCommandKind, DebugResult},
    serde_json::json,
};

// ── ScrollbarDrag ────────────────────────────────────────────────────────────

/// State for an active scrollbar drag operation.
pub(crate) struct ScrollbarDrag {
    /// The node ID of the scroll container being scrolled.
    pub node_id: usize,
    /// The Y coordinate where the drag started (screen pixels).
    pub start_y: f32,
    /// The scroll_offset.1 value when the drag started.
    pub start_scroll: f64,
    /// Content height of the scroll container (for ratio calculation).
    pub content_height: f64,
    /// Container height of the scroll container.
    pub container_height: f64,
}

// ── RinchApp ─────────────────────────────────────────────────────────────────

/// Platform-agnostic application state.
///
/// The runtime shell creates a `RinchApp`, mounts the component, and then
/// repeatedly calls [`handle_event`](RinchApp::handle_event) for each
/// platform event. The returned [`AppAction`]s tell the shell what to do
/// (request redraw, exit, etc.).
#[allow(dead_code)] // Some fields are only used behind feature gates (debug, theme)
pub struct RinchApp {
    /// Component function to render (consumed on mount).
    #[allow(clippy::type_complexity)]
    pub(crate) component: Option<Box<dyn FnOnce(&mut RenderScope) -> NodeHandle>>,
    /// The document (shared with RenderScope).
    pub(crate) doc: Option<Rc<RefCell<RinchDocument>>>,
    /// Render scope (kept alive for effects).
    pub(crate) _render_scope: Option<Rc<RefCell<RenderScope>>>,
    /// Vello scene (reused across frames).
    pub(crate) scene: Scene,
    /// Parley layout context for paint-time text layout.
    pub(crate) paint_layout_cx: parley::LayoutContext<peniko::Brush>,
    /// Current cursor position.
    pub(crate) cursor_pos: Option<(f32, f32)>,
    /// Active scrollbar drag state.
    pub(crate) scrollbar_drag: Option<ScrollbarDrag>,
    /// DevTools state.
    #[cfg(feature = "desktop")]
    pub(crate) devtools: DevToolsState,
    /// Last theme CSS loaded into the document (for change detection).
    pub(crate) last_theme_css: Option<String>,
    /// Timestamp of last mouse click (for multi-click detection).
    pub(crate) last_click_time: Instant,
    /// Position of last mouse click.
    pub(crate) last_click_pos: (f32, f32),
    /// Current click count (1 = single, 2 = double, 3 = triple).
    pub(crate) click_count: u8,
    /// Font context for hit testing (reused across frames).
    pub(crate) hit_test_font_cx: parley::FontContext,
    /// Window properties for configuring borderless, transparent, etc.
    pub(crate) window_props: Option<rinch_core::element::WindowProps>,
    /// Current keyboard modifier state.
    pub(crate) modifiers: Modifiers,
    /// Whether the Vello scene needs to be rebuilt.
    pub(crate) scene_dirty: bool,
    /// The data-oninput handler ID for the currently focused text input.
    pub(crate) focused_input_handler_id: Option<usize>,
    /// Current accumulated text value for the focused text input.
    pub(crate) focused_input_value: String,
    /// Font data to register on the document when it is created (for WASM).
    pub(crate) pending_fonts: Vec<&'static [u8]>,
    /// Debug command receiver.
    #[cfg(feature = "debug")]
    pub(crate) debug_cmd_rx: Option<CommandReceiver>,
    /// Debug server handle (kept alive).
    #[cfg(feature = "debug")]
    pub(crate) _debug_server: Option<rinch_debug::DebugServer>,
}

impl RinchApp {
    /// Create a new `RinchApp` from a component function.
    pub fn new(component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        Self {
            component: Some(Box::new(component)),
            doc: None,
            _render_scope: None,
            scene: Scene::new(),
            paint_layout_cx: parley::LayoutContext::new(),
            cursor_pos: None,
            scrollbar_drag: None,
            #[cfg(feature = "desktop")]
            devtools: DevToolsState::new(),
            last_theme_css: None,
            last_click_time: Instant::now(),
            last_click_pos: (0.0, 0.0),
            click_count: 0,
            hit_test_font_cx: parley::FontContext::new(),
            window_props: None,
            modifiers: Modifiers::default(),
            scene_dirty: true,
            focused_input_handler_id: None,
            focused_input_value: String::new(),
            pending_fonts: Vec::new(),
            #[cfg(feature = "debug")]
            debug_cmd_rx: None,
            #[cfg(feature = "debug")]
            _debug_server: None,
        }
    }

    /// Set window properties (must be called before [`mount_component`]).
    pub fn set_window_props(&mut self, props: rinch_core::element::WindowProps) {
        self.window_props = Some(props);
    }

    /// Access the document (if mounted).
    pub fn doc(&self) -> Option<&Rc<RefCell<RinchDocument>>> {
        self.doc.as_ref()
    }

    /// Register font data for text rendering.
    ///
    /// On WASM, system fonts are not available, so fonts must be registered
    /// explicitly. Call this **before** [`mount_component`] so the fonts are
    /// available during the initial layout pass.
    ///
    /// The data should be a TrueType (.ttf) or OpenType (.otf) font file.
    /// The font is registered and set as a fallback for all scripts.
    pub fn register_font_data(&mut self, data: &'static [u8]) {
        self.pending_fonts.push(data);
        // Also register immediately on the hit-test font context
        Self::register_font_on_context(&mut self.hit_test_font_cx, data);
    }

    /// Register font data on a FontContext (internal helper).
    fn register_font_on_context(font_cx: &mut parley::FontContext, data: &'static [u8]) {
        use parley::fontique::{Blob, FallbackKey, GenericFamily, Script};
        use std::sync::Arc;

        let blob = Blob::new(Arc::new(data));
        let families = font_cx.collection.register_fonts(blob, None);
        let family_ids: Vec<_> = families.iter().map(|(id, _)| *id).collect();

        // Set as fallback for all scripts
        for (script, _) in Script::all_samples() {
            font_cx
                .collection
                .append_fallbacks(FallbackKey::new(*script, None), family_ids.iter().copied());
        }

        // Map to generic font families so CSS generic names like "sans-serif"
        // and "system-ui" resolve to this font. Critical for WASM where no
        // system fonts exist and the default theme font stack ends with
        // "sans-serif".
        for generic in [GenericFamily::SansSerif, GenericFamily::SystemUi] {
            font_cx
                .collection
                .append_generic_families(generic, family_ids.iter().copied());
        }
    }

    /// Diagnostic: test font resolution by building a small Parley layout.
    /// Returns a string like "width=X, height=Y, glyphs=N" or error info.
    pub fn diagnose_fonts(&self, font_stack: &str) -> String {
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
            let text = "Test";
            let mut builder = layout_cx.ranged_builder(&mut d.font_cx, text, 1.0, true);
            builder.push_default(parley::style::StyleProperty::FontSize(16.0));
            builder.push_default(parley::style::StyleProperty::FontStack(
                parley::style::FontStack::Source(std::borrow::Cow::Owned(font_stack.to_string())),
            ));
            let mut layout = builder.build(text);
            layout.break_all_lines(None);
            let mut glyph_count = 0;
            for line in layout.lines() {
                for item in line.items() {
                    if let parley::layout::PositionedLayoutItem::GlyphRun(_run) = item {
                        glyph_count += _run.glyphs().count();
                    }
                }
            }
            format!(
                "font_stack='{}' width={:.1} height={:.1} glyphs={}",
                font_stack,
                layout.width(),
                layout.height(),
                glyph_count
            )
        } else {
            "no doc mounted".to_string()
        }
    }

    /// Whether the window should have a transparent background.
    pub fn is_transparent(&self) -> bool {
        self.window_props.as_ref().is_some_and(|p| p.transparent)
    }

    // ── Component mounting ───────────────────────────────────────────────

    /// Mount the component, building the initial DOM.
    ///
    /// Called once after the window and renderer are ready.
    pub fn mount_component(&mut self, viewport_width: f32, viewport_height: f32) {
        let doc = Rc::new(RefCell::new(RinchDocument::new()));

        // Register any pending fonts on the document's font context (for WASM)
        if !self.pending_fonts.is_empty() {
            let mut d = doc.borrow_mut();
            for font_data in &self.pending_fonts {
                Self::register_font_on_context(&mut d.font_cx, font_data);
            }
        }

        // Load theme + widget CSS into the document's stylesheet
        {
            let mut d = doc.borrow_mut();
            #[cfg(feature = "theme")]
            {
                let theme_css = rinch_core::get_current_theme_css().unwrap_or_default();
                if !theme_css.is_empty() {
                    d.load_css(&theme_css);
                }
            }
            // Set viewport size so vh/vw units resolve correctly during DOM construction
            d.set_viewport(viewport_width, viewport_height);
        }

        // Remember the initial theme CSS so we can detect changes later
        #[cfg(feature = "theme")]
        {
            self.last_theme_css = Some(rinch_core::get_current_theme_css().unwrap_or_default());
        }

        // Create RenderScope
        let doc_as_dom: Rc<RefCell<dyn DomDocument>> = doc.clone();
        let body_id = doc.borrow().body();
        let scope = Rc::new(RefCell::new(RenderScope::new(doc_as_dom, body_id)));

        // Set thread-local context
        set_render_scope(scope.clone());
        begin_render();

        // Run the component
        let component = self.component.take().expect("component already consumed");
        let root = {
            let mut scope_ref = scope.borrow_mut();
            component(&mut scope_ref)
        };

        // Append root to body
        doc.borrow_mut().append_child(body_id, root.node_id());

        end_render();
        clear_render_scope();

        // Initial layout
        {
            let mut d = doc.borrow_mut();
            d.resolve_layout(viewport_width, viewport_height);
            let _ = d.take_dirty_nodes();
        }

        self.scene_dirty = true;
        self.doc = Some(doc);
        self._render_scope = Some(scope);
    }

    // ── Layout / repaint ─────────────────────────────────────────────────

    /// Re-resolve layout after signal changes. Returns `true` if a redraw
    /// is needed.
    pub fn resolve_and_repaint(&mut self, viewport_width: f32, viewport_height: f32) -> bool {
        let frame_start = Instant::now();
        let Some(doc) = &self.doc else {
            return false;
        };

        // Check if theme CSS has changed (e.g. primary color or dark mode toggled)
        #[cfg(feature = "theme")]
        {
            let current_theme = rinch_core::get_current_theme_css().unwrap_or_default();
            let theme_changed = self.last_theme_css.as_deref() != Some(current_theme.as_str());

            if theme_changed {
                self.last_theme_css = Some(current_theme.clone());
                if !current_theme.is_empty() {
                    let mut d = doc.borrow_mut();
                    d.update_theme_variables(&current_theme);
                    d.recompute_all_styles_full();
                }
            }
        }

        // Resolve layout
        {
            let mut d = doc.borrow_mut();
            let _ = d.take_dirty_nodes();
            d.resolve_layout(viewport_width, viewport_height);
        }

        self.scene_dirty = true;

        // Log frame time if RINCH_PERF is set
        if std::env::var("RINCH_PERF").is_ok() {
            let elapsed = frame_start.elapsed();
            let fps = 1.0 / elapsed.as_secs_f64();
            eprintln!(
                "[PERF] resolve: {:.2}ms ({:.0} fps)",
                elapsed.as_secs_f64() * 1000.0,
                fps
            );
        }

        true
    }

    /// Resize the document layout.
    pub fn resize_layout(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            d.resolve_layout(width as f32, height as f32);
            let _ = d.take_dirty_nodes();
            self.scene_dirty = true;
        }
    }

    /// Build the Vello scene from the current document state.
    ///
    /// The scene is built into `self.scene` and a reference is returned.
    pub fn build_scene(&mut self, scale: f64, size: (u32, u32)) -> &Scene {
        if !self.scene_dirty {
            return &self.scene;
        }

        self.scene.reset();
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            let d = &mut *d;
            rinch_dom::paint::paint_document(
                &d.tree,
                &mut self.scene,
                scale,
                (size.0 as f32, size.1 as f32),
                &mut d.font_cx,
                &mut d.layout_cx,
            );
        }
        self.scene_dirty = false;
        &self.scene
    }

    /// Check if there are dirty nodes that need repaint.
    pub fn has_dirty_nodes(&self) -> bool {
        self.doc
            .as_ref()
            .map(|doc| !doc.borrow().tree.dirty_nodes.is_empty())
            .unwrap_or(false)
    }

    // ── Event handling ───────────────────────────────────────────────────

    /// Process a platform event and return a list of actions for the shell.
    #[allow(clippy::too_many_lines)]
    pub fn handle_event(
        &mut self,
        event: PlatformEvent,
        window_size: (u32, u32),
        scale_factor: f64,
    ) -> Vec<AppAction> {
        let mut actions = Vec::new();

        match event {
            PlatformEvent::Resumed => {
                // Handled by the shell (window creation)
            }
            PlatformEvent::CloseRequested => {
                actions.push(AppAction::Exit);
            }
            PlatformEvent::Resized { width, height } => {
                self.resize_layout(width, height);
                actions.push(AppAction::RequestRedraw);
            }
            PlatformEvent::RedrawRequested => {
                // Paint is handled by the shell after building the scene
            }
            PlatformEvent::MouseMove { x, y } => {
                self.cursor_pos = Some((x, y));

                // Handle scrollbar drag
                if let Some(drag) = &self.scrollbar_drag {
                    let node_id = drag.node_id;
                    let dy = y - drag.start_y;
                    let track_height = drag.container_height - 4.0;
                    let max_scroll = drag.content_height - drag.container_height;
                    let scroll_delta = (dy as f64 / track_height) * drag.content_height;
                    let new_scroll = (drag.start_scroll + scroll_delta).clamp(0.0, max_scroll);

                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        if let Some(node) = d.tree.nodes.get_mut(node_id) {
                            node.scroll_offset.1 = new_scroll;
                            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                            d.tree.dirty_nodes.insert(node_id);
                        }
                    }
                    actions.push(AppAction::RequestRedraw);
                    return actions;
                }

                // Update hover state for CSS :hover support
                if let Some(doc) = &self.doc {
                    let hovered = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    let changed = doc.borrow_mut().update_hover(hovered);
                    if changed {
                        actions.push(AppAction::RequestRedraw);
                    }
                }
            }
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            } => {
                // Multi-click detection
                let now = Instant::now();
                let elapsed = now.duration_since(self.last_click_time);
                let (last_x, last_y) = self.last_click_pos;
                let distance = ((x - last_x).powi(2) + (y - last_y).powi(2)).sqrt();

                const DOUBLE_CLICK_TIMEOUT: rinch_platform::Duration =
                    rinch_platform::Duration::from_millis(500);
                const DOUBLE_CLICK_DISTANCE: f32 = 5.0;

                if elapsed < DOUBLE_CLICK_TIMEOUT && distance < DOUBLE_CLICK_DISTANCE {
                    self.click_count = (self.click_count % 3) + 1;
                } else {
                    self.click_count = 1;
                }

                self.last_click_time = now;
                self.last_click_pos = (x, y);

                // Check scrollbar hit first
                let scrollbar_hit = if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    find_scrollbar_hit(&d.tree, x, y)
                } else {
                    None
                };

                if let Some((node_id, content_height, container_height)) = scrollbar_hit {
                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        let node_abs_y = compute_absolute_y(&d.tree, node_id);
                        let margin = 2.0_f64;
                        let track_top = node_abs_y as f64 + margin;
                        let track_height = container_height - margin * 2.0;
                        let max_scroll = content_height - container_height;
                        let click_ratio = ((y as f64 - track_top) / track_height).clamp(0.0, 1.0);
                        let new_scroll = click_ratio * max_scroll;

                        if let Some(node) = d.tree.nodes.get_mut(node_id) {
                            node.scroll_offset.1 = new_scroll;
                            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                            d.tree.dirty_nodes.insert(node_id);
                        }

                        self.scrollbar_drag = Some(ScrollbarDrag {
                            node_id,
                            start_y: y,
                            start_scroll: new_scroll,
                            content_height,
                            container_height,
                        });
                    }
                    actions.push(AppAction::RequestRedraw);
                } else {
                    let drag_action = self.handle_click(x, y, scale_factor);
                    actions.extend(drag_action);
                }
            }
            PlatformEvent::MouseDown { .. } => {
                // Non-left button clicks: no-op for now
            }
            PlatformEvent::MouseUp { .. } => {
                self.scrollbar_drag = None;
            }
            PlatformEvent::MouseWheel {
                x,
                y,
                delta_x: _,
                delta_y,
            } => {
                self.cursor_pos = Some((x, y));
                if let Some(doc) = &self.doc {
                    let hit_node = hit_test(&doc.borrow().tree, x, y);
                    if let Some(hit_node) = hit_node {
                        let mut doc_mut = doc.borrow_mut();
                        if let Some(scroll_node_id) = find_scroll_container(&doc_mut.tree, hit_node)
                        {
                            let content_height =
                                compute_content_height(&doc_mut.tree, scroll_node_id);
                            let container_height = doc_mut
                                .tree
                                .get(scroll_node_id)
                                .map(|n| n.layout.height as f64)
                                .unwrap_or(0.0);
                            let max_scroll = (content_height - container_height).max(0.0);

                            if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                let new_y = (node.scroll_offset.1 - delta_y).clamp(0.0, max_scroll);
                                if new_y != node.scroll_offset.1 {
                                    node.scroll_offset.1 = new_y;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                }
                            }
                        }
                        drop(doc_mut);
                        actions.push(AppAction::RequestRedraw);
                    }
                }
            }
            PlatformEvent::ModifiersChanged(mods) => {
                self.modifiers = mods;
            }
            PlatformEvent::KeyDown {
                key,
                text,
                modifiers,
            } => {
                let shift = modifiers.shift;
                let ctrl = modifiers.primary();
                let alt = modifiers.alt;

                // Build key string for keyboard interceptor - handle ALL key types
                let key_str: Option<String> = match key {
                    // Named keys
                    KeyCode::ArrowLeft => Some("ArrowLeft".into()),
                    KeyCode::ArrowRight => Some("ArrowRight".into()),
                    KeyCode::ArrowUp => Some("ArrowUp".into()),
                    KeyCode::ArrowDown => Some("ArrowDown".into()),
                    KeyCode::Home => Some("Home".into()),
                    KeyCode::End => Some("End".into()),
                    KeyCode::Enter => Some("Enter".into()),
                    KeyCode::Backspace => Some("Backspace".into()),
                    KeyCode::Delete => Some("Delete".into()),
                    KeyCode::Tab => Some("Tab".into()),
                    KeyCode::Escape => Some("Escape".into()),
                    KeyCode::PageUp => Some("PageUp".into()),
                    KeyCode::PageDown => Some("PageDown".into()),
                    KeyCode::Space => Some("Space".into()),
                    // Ctrl+key combos: derive key letter from KeyCode
                    KeyCode::KeyA if ctrl => Some("a".into()),
                    KeyCode::KeyB if ctrl => Some("b".into()),
                    KeyCode::KeyC if ctrl => Some("c".into()),
                    KeyCode::KeyD if ctrl => Some("d".into()),
                    KeyCode::KeyE if ctrl => Some("e".into()),
                    KeyCode::KeyH if ctrl => Some("h".into()),
                    KeyCode::KeyI if ctrl => Some("i".into()),
                    KeyCode::KeyU if ctrl => Some("u".into()),
                    KeyCode::KeyV if ctrl => Some("v".into()),
                    KeyCode::KeyX if ctrl => Some("x".into()),
                    KeyCode::KeyY if ctrl => Some("y".into()),
                    KeyCode::KeyZ if ctrl => Some("z".into()),
                    // Regular character input: use text field (filter control chars)
                    _ => text.as_ref().and_then(|t| {
                        if !t.is_empty() && t.chars().all(|c| !c.is_control()) {
                            Some(t.clone())
                        } else {
                            None
                        }
                    }),
                };

                tracing::trace!(?key, ?text, ?key_str, shift, ctrl, alt, "KeyDown event");

                // Try keyboard interceptor first for ALL keys
                let handled_by_interceptor = if let Some(ref ks) = key_str {
                    let key_data = events::KeyEventData {
                        key: ks.clone(),
                        code: format!("{:?}", key),
                        ctrl,
                        shift,
                        alt,
                        meta: false,
                    };
                    events::dispatch_keyboard_event(&key_data)
                } else {
                    false
                };

                if handled_by_interceptor {
                    actions.push(AppAction::RequestRedraw);
                } else {
                    #[cfg(feature = "desktop")]
                    if key == KeyCode::F12 {
                        self.devtools.toggle();
                        tracing::info!(
                            "DevTools: {}",
                            if self.devtools.visible {
                                "opened"
                            } else {
                                "closed"
                            }
                        );
                        actions.push(AppAction::RequestRedraw);
                        return actions;
                    }

                    match key {
                        KeyCode::Backspace => self.handle_backspace(),
                        KeyCode::Delete => self.handle_delete(),
                        KeyCode::ArrowLeft => self.handle_arrow_left(shift, ctrl),
                        KeyCode::ArrowRight => self.handle_arrow_right(shift, ctrl),
                        KeyCode::Home => self.handle_home(shift),
                        KeyCode::End => self.handle_end(shift),
                        KeyCode::KeyA if ctrl => self.handle_select_all(),
                        KeyCode::KeyC if ctrl => self.handle_copy(),
                        KeyCode::KeyV if ctrl => self.handle_paste(),
                        KeyCode::KeyX if ctrl => self.handle_cut(),
                        KeyCode::Enter if !ctrl => self.handle_enter(),
                        KeyCode::ArrowUp => self.handle_arrow_up(shift),
                        KeyCode::ArrowDown => self.handle_arrow_down(shift),
                        _ => {
                            if !ctrl
                                && let Some(t) = &text
                                && !t.is_empty()
                            {
                                self.handle_text_input(t);
                            }
                        }
                    }
                }
            }
            PlatformEvent::ScaleFactorChanged(_) => {
                // The shell handles reconfiguring the renderer; we just need a redraw.
                actions.push(AppAction::RequestRedraw);
            }
            PlatformEvent::UserEvent(UserEvent::ReRender) => {
                let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                if self.resolve_and_repaint(w, h) {
                    actions.push(AppAction::RequestRedraw);
                }
            }
            PlatformEvent::UserEvent(UserEvent::MinimizeWindow) => {
                actions.push(AppAction::SetMinimized(true));
            }
            PlatformEvent::UserEvent(UserEvent::ToggleMaximizeWindow) => {
                // The shell must query `is_maximized` and toggle.
                // We cannot know from here, so emit a special action.
                // The shell interprets this as "toggle".
                actions.push(AppAction::SetMaximized(true)); // placeholder; shell toggles
            }
            PlatformEvent::UserEvent(UserEvent::CloseWindow) => {
                actions.push(AppAction::Exit);
            }
            PlatformEvent::UserEvent(UserEvent::DebugCommand) => {
                #[cfg(feature = "debug")]
                self.handle_debug_commands(&mut actions, scale_factor, window_size);
            }
            PlatformEvent::AboutToWait => {
                if self.has_dirty_nodes() {
                    let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                    if self.resolve_and_repaint(w, h) {
                        actions.push(AppAction::RequestRedraw);
                    }
                }
            }
        }

        actions
    }

    // ── Click handling ───────────────────────────────────────────────────

    fn handle_click(&mut self, x: f32, y: f32, _scale_factor: f64) -> Vec<AppAction> {
        let mut actions = Vec::new();
        let Some(doc) = &self.doc else {
            return actions;
        };
        let d = doc.borrow();

        if let Some(hit_id) = hit_test(&d.tree, x, y) {
            // Walk up from hit target to detect text input focus (data-oninput).
            // This must happen before the data-rid walk which may return early.
            let mut found_input_focus = false;
            {
                let mut check = Some(hit_id);
                while let Some(nid) = check {
                    if let Some(node) = d.tree.get(nid) {
                        if let Some(oninput_str) = node.attributes.get("data-oninput") {
                            if let Ok(handler_id) = oninput_str.parse::<usize>() {
                                self.focused_input_handler_id = Some(handler_id);
                                self.focused_input_value =
                                    node.attributes.get("value").cloned().unwrap_or_default();
                                found_input_focus = true;
                            }
                            break;
                        }
                        check = node.parent;
                    } else {
                        break;
                    }
                }
            }
            if !found_input_focus {
                self.focused_input_handler_id = None;
                self.focused_input_value.clear();
            }

            let mut current = Some(hit_id);
            while let Some(node_id) = current {
                if let Some(node) = d.tree.get(node_id) {
                    // Check for click handler
                    if let Some(rid_str) = node.attributes.get("data-rid")
                        && let Ok(handler_id) = rid_str.parse::<usize>()
                    {
                        let text_hit = Self::compute_text_hit_info(&d.tree, hit_id, x, y);

                        let (elem_x, elem_y, elem_w, elem_h) = {
                            let mut ax = node.layout.x;
                            let mut ay = node.layout.y;
                            let mut pid = node.parent;
                            while let Some(p) = pid {
                                if let Some(pn) = d.tree.get(p) {
                                    ax += pn.layout.x;
                                    ay += pn.layout.y;
                                    ax -= pn.scroll_offset.0 as f32;
                                    ay -= pn.scroll_offset.1 as f32;
                                    pid = pn.parent;
                                } else {
                                    break;
                                }
                            }
                            (ax, ay, node.layout.width, node.layout.height)
                        };

                        events::set_click_context(events::ClickContext {
                            mouse_x: x,
                            mouse_y: y,
                            element_x: elem_x,
                            element_y: elem_y,
                            element_width: elem_w,
                            element_height: elem_h,
                            text_hit,
                        });

                        drop(d);
                        events::dispatch_event(events::EventHandlerId(handler_id));
                        let _ = rinch_core::take_pending_focus_request();
                        actions.push(AppAction::RequestRedraw);
                        return actions;
                    }
                    // Check for drag-window region
                    if node.attributes.contains_key("data-drag-window") {
                        drop(d);
                        actions.push(AppAction::DragWindow);
                        return actions;
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
        }
        actions
    }

    /// Compute text hit info for click-to-position in rich text editors.
    fn compute_text_hit_info(
        tree: &rinch_dom::NodeTree,
        hit_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> events::TextHitInfo {
        let mut block_index = 0usize;
        let mut block_node_id = None;
        let mut current = Some(hit_id);

        while let Some(node_id) = current {
            if let Some(node) = tree.get(node_id) {
                if let Some(idx_str) = node.attributes.get("data-block-index")
                    && let Ok(idx) = idx_str.parse::<usize>()
                {
                    block_index = idx;
                    block_node_id = Some(node_id);
                    break;
                }
                current = node.parent;
            } else {
                break;
            }
        }

        let Some(block_id) = block_node_id else {
            return events::TextHitInfo::default();
        };

        let Some(block_node) = tree.get(block_id) else {
            return events::TextHitInfo::default();
        };

        let mut abs_x = block_node.layout.x;
        let mut abs_y = block_node.layout.y;
        let mut parent_id = block_node.parent;
        while let Some(pid) = parent_id {
            if let Some(pn) = tree.get(pid) {
                abs_x += pn.layout.x;
                abs_y += pn.layout.y;
                abs_x -= pn.scroll_offset.0 as f32;
                abs_y -= pn.scroll_offset.1 as f32;
                parent_id = pn.parent;
            } else {
                break;
            }
        }

        let rel_x = (click_x - abs_x).max(0.0);
        let rel_y = (click_y - abs_y).max(0.0);

        let byte_offset = if let Some(ref layout) = block_node.text_layout {
            byte_offset_from_position(&layout.layout, rel_x, rel_y)
        } else if let Some(ref layout) = block_node.cached_text_parley {
            byte_offset_from_position(layout, rel_x, rel_y)
        } else {
            let mut offset = 0usize;
            for &child_id in &block_node.children {
                if let Some(child) = tree.nodes.get(child_id)
                    && let Some(ref layout) = child.cached_text_parley
                {
                    offset = byte_offset_from_position(layout, rel_x, rel_y);
                    break;
                }
            }
            offset
        };

        events::TextHitInfo {
            block_index,
            byte_offset,
            inline_root_node_id: block_id,
            valid: true,
        }
    }

    // ── No-op keyboard stubs ─────────────────────────────────────────────
    // The keyboard interceptor handles all input; these are stubs for
    // the fallback path when no interceptor is registered.

    fn handle_text_input(&mut self, text: &str) {
        if let Some(handler_id) = self.focused_input_handler_id {
            self.focused_input_value.push_str(text);
            let value = self.focused_input_value.clone();
            self.update_focused_input_dom_value(handler_id, &value);
            events::dispatch_input_event(events::EventHandlerId(handler_id), value);
        }
    }
    fn handle_backspace(&mut self) {
        if let Some(handler_id) = self.focused_input_handler_id {
            self.focused_input_value.pop();
            let value = self.focused_input_value.clone();
            self.update_focused_input_dom_value(handler_id, &value);
            events::dispatch_input_event(events::EventHandlerId(handler_id), value);
        }
    }
    fn handle_delete(&mut self) {}
    fn handle_arrow_left(&mut self, _shift: bool, _ctrl: bool) {}
    fn handle_arrow_right(&mut self, _shift: bool, _ctrl: bool) {}
    fn handle_enter(&mut self) {
        // Check if a text input is focused and has an onsubmit handler
        if self.focused_input_handler_id.is_some()
            && let Some(doc) = &self.doc
        {
            let d = doc.borrow();
            // Find the focused input node by its data-oninput handler
            let submit_handler_id = d.tree.nodes.iter().find_map(|(_, node)| {
                // Find node with matching data-oninput
                node.attributes
                    .get("data-oninput")
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&h| Some(h) == self.focused_input_handler_id)
                    .and_then(|_| {
                        // Found the focused input - check for data-onsubmit
                        node.attributes
                            .get("data-onsubmit")
                            .and_then(|s| s.parse::<usize>().ok())
                    })
            });
            drop(d);
            if let Some(handler_id) = submit_handler_id {
                events::dispatch_event(events::EventHandlerId(handler_id));
            }
        }
    }
    fn handle_arrow_up(&mut self, _shift: bool) {}
    fn handle_arrow_down(&mut self, _shift: bool) {}
    fn handle_home(&mut self, _shift: bool) {}
    fn handle_end(&mut self, _shift: bool) {}
    fn handle_select_all(&mut self) {}
    fn handle_copy(&mut self) {}
    fn handle_paste(&mut self) {}
    fn handle_cut(&mut self) {}

    /// Update the DOM `value` attribute on the focused input element.
    ///
    /// This keeps the DOM in sync with the accumulated text so that
    /// subsequent clicks re-read the correct value, and the renderer
    /// paints the current text.
    fn update_focused_input_dom_value(&self, handler_id: usize, value: &str) {
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            // Find the node ID first (immutable scan)
            let target_id = d.tree.nodes.iter().find_map(|(id, node)| {
                node.attributes
                    .get("data-oninput")
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&h| h == handler_id)
                    .map(|_| id)
            });
            // Then mutate with the known ID
            if let Some(node_id) = target_id {
                if let Some(node) = d.tree.nodes.get_mut(node_id) {
                    node.attributes
                        .insert("value".to_string(), value.to_string());
                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                }
                d.tree.dirty_nodes.insert(node_id);
            }
        }
    }

    /// Calculate byte offset from click coordinates relative to text start.
    #[allow(dead_code)]
    fn byte_offset_from_xy(
        layout: &parley::layout::Layout<peniko::Brush>,
        click_x: f32,
        click_y: f32,
    ) -> usize {
        byte_offset_from_position(layout, click_x, click_y)
    }

    // ── Debug commands ───────────────────────────────────────────────────

    #[cfg(feature = "debug")]
    pub(crate) fn handle_debug_commands(
        &mut self,
        actions: &mut Vec<AppAction>,
        scale_factor: f64,
        window_size: (u32, u32),
    ) {
        let Some(rx) = self.debug_cmd_rx.take() else {
            return;
        };

        while let Ok(cmd) = rx.0.try_recv() {
            let response = self.execute_debug_command(cmd.kind, actions, scale_factor, window_size);
            let _ = cmd.response_tx.send(response);
        }

        self.debug_cmd_rx = Some(rx);
    }

    #[cfg(feature = "debug")]
    pub(crate) fn execute_debug_command(
        &mut self,
        kind: DebugCommandKind,
        actions: &mut Vec<AppAction>,
        scale_factor: f64,
        window_size: (u32, u32),
    ) -> DebugResult {
        match kind {
            DebugCommandKind::Screenshot => {
                // Screenshot is handled by the shell -- we signal that we need
                // a screenshot capture. The shell will paint + capture.
                // For now, return an error indicating the shell must handle this.
                DebugResult::Error {
                    message: "__SCREENSHOT_DELEGATE__".into(),
                }
            }
            DebugCommandKind::DomTree => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                DebugResult::Json {
                    data: rinch_dom::testing::serialize_tree(&d.tree),
                }
            }
            DebugCommandKind::QuerySelector { selector } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                let ids = rinch_dom::testing::query_selector(&d.tree, &selector);
                let nodes: Vec<_> = ids
                    .iter()
                    .filter_map(|&id| rinch_dom::testing::get_node_detail(&d.tree, id))
                    .collect();
                DebugResult::Json { data: json!(nodes) }
            }
            DebugCommandKind::GetNode { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                match rinch_dom::testing::get_node_detail(&d.tree, id) {
                    Some(detail) => DebugResult::Json { data: detail },
                    None => DebugResult::Error {
                        message: format!("Node {} not found", id),
                    },
                }
            }
            DebugCommandKind::GetTextContent { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                DebugResult::Json {
                    data: json!(rinch_dom::testing::get_text_content(&d.tree, id)),
                }
            }
            DebugCommandKind::Click { x, y } => {
                let click_actions = self.handle_click(x, y, scale_factor);
                actions.extend(click_actions);
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseMove { x, y } => {
                self.cursor_pos = Some((x, y));
                if let Some(doc) = &self.doc {
                    let hovered = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    let changed = doc.borrow_mut().update_hover(hovered);
                    if changed {
                        actions.push(AppAction::RequestRedraw);
                    }
                }
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::Scroll {
                x,
                y,
                delta_x: _delta_x,
                delta_y,
            } => {
                self.cursor_pos = Some((x, y));

                if let Some(doc) = &self.doc {
                    let hit_node = hit_test(&doc.borrow().tree, x, y);
                    if let Some(hit_node) = hit_node {
                        let mut doc_mut = doc.borrow_mut();
                        if let Some(scroll_node_id) = find_scroll_container(&doc_mut.tree, hit_node)
                        {
                            let content_height =
                                compute_content_height(&doc_mut.tree, scroll_node_id);
                            let container_height = doc_mut
                                .tree
                                .get(scroll_node_id)
                                .map(|n| n.layout.height as f64)
                                .unwrap_or(0.0);
                            let max_scroll = (content_height - container_height).max(0.0);

                            if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                let new_y = (node.scroll_offset.1 + delta_y).clamp(0.0, max_scroll);
                                if new_y != node.scroll_offset.1 {
                                    node.scroll_offset.1 = new_y;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                }
                            }
                        }
                        drop(doc_mut);
                    }
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::TypeText { text } => {
                for ch in text.chars() {
                    let key = match ch {
                        ' ' => "Space".to_string(),
                        '\n' => "Enter".to_string(),
                        '\t' => "Tab".to_string(),
                        c => c.to_string(),
                    };
                    let key_data = events::KeyEventData {
                        key: key.clone(),
                        code: key,
                        ctrl: false,
                        shift: false,
                        alt: false,
                        meta: false,
                    };
                    let handled = events::dispatch_keyboard_event(&key_data);
                    if !handled {
                        // Fallback to handle_text_input for non-intercepted chars
                        self.handle_text_input(&ch.to_string());
                    }
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::WaitFrame => {
                let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                self.resolve_and_repaint(w, h);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::GetComputedStyles { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                match d.tree.get(id) {
                    Some(node) => DebugResult::Json {
                        data: json!(&node.computed_style),
                    },
                    None => DebugResult::Error {
                        message: format!("Node {} not found", id),
                    },
                }
            }
            DebugCommandKind::CloseApp => {
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    std::process::exit(0);
                });
                DebugResult::Json {
                    data: json!({"status": "closing"}),
                }
            }
            DebugCommandKind::KeyPress { key, shift, ctrl } => {
                let key_data = events::KeyEventData {
                    key: key.clone(),
                    code: key.clone(),
                    ctrl,
                    shift,
                    alt: false,
                    meta: false,
                };
                let handled = events::dispatch_keyboard_event(&key_data);

                if !handled {
                    match key.as_str() {
                        "ArrowUp" => self.handle_arrow_up(shift),
                        "ArrowDown" => self.handle_arrow_down(shift),
                        "ArrowLeft" => self.handle_arrow_left(shift, ctrl),
                        "ArrowRight" => self.handle_arrow_right(shift, ctrl),
                        "Home" => self.handle_home(shift),
                        "End" => self.handle_end(shift),
                        "Enter" => self.handle_enter(),
                        "Backspace" => self.handle_backspace(),
                        "Delete" => self.handle_delete(),
                        _ => {}
                    }
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::GetCaretPosition {
                node_id,
                byte_offset,
            } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };

                let scale = scale_factor as f32;

                let d = doc.borrow();
                let Some(node) = d.tree.get(node_id) else {
                    return DebugResult::Error {
                        message: format!("Node {} not found", node_id),
                    };
                };

                let mut abs_x = node.layout.x as f64;
                let mut abs_y = node.layout.y as f64;
                let mut parent_id = node.parent;
                while let Some(pid) = parent_id {
                    if let Some(parent_node) = d.tree.get(pid) {
                        abs_x += parent_node.layout.x as f64;
                        abs_y += parent_node.layout.y as f64;
                        abs_x -= parent_node.scroll_offset.0;
                        abs_y -= parent_node.scroll_offset.1;
                        parent_id = parent_node.parent;
                    } else {
                        break;
                    }
                }

                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = node.attributes.get("value").cloned().unwrap_or_default();
                    if value.is_empty() {
                        let padding_left =
                            node.computed_style.padding_left.to_px() as f64 * scale as f64;
                        let padding_top =
                            node.computed_style.padding_top.to_px() as f64 * scale as f64;
                        return DebugResult::Json {
                            data: json!({
                                "x": abs_x + padding_left,
                                "y": abs_y + padding_top,
                            }),
                        };
                    }

                    let computed_style = node.computed_style.clone();
                    let input_width = node.layout.width;
                    drop(d);

                    let layout = computed_style.build_parley_layout(
                        &value,
                        scale,
                        &mut self.hit_test_font_cx,
                        &mut self.paint_layout_cx,
                        Some(input_width),
                    );

                    let (x, y) = caret_position_for_offset_layout(&layout, byte_offset);
                    let padding_left = computed_style.padding_left.to_px() as f64 * scale as f64;
                    let padding_top = computed_style.padding_top.to_px() as f64 * scale as f64;

                    return DebugResult::Json {
                        data: json!({
                            "x": abs_x + padding_left + x as f64,
                            "y": abs_y + padding_top + y as f64,
                        }),
                    };
                }

                if let Some(ref inline_layout) = node.text_layout {
                    let (x, y) =
                        caret_position_for_offset_layout(&inline_layout.layout, byte_offset);
                    return DebugResult::Json {
                        data: json!({
                            "x": abs_x + x as f64,
                            "y": abs_y + y as f64,
                        }),
                    };
                }

                DebugResult::Error {
                    message: "Node does not have text layout".into(),
                }
            }
            DebugCommandKind::GetGlyphBounds {
                node_id,
                byte_offset,
            } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };

                let scale = scale_factor as f32;

                let d = doc.borrow();
                let Some(node) = d.tree.get(node_id) else {
                    return DebugResult::Error {
                        message: format!("Node {} not found", node_id),
                    };
                };

                let mut abs_x = node.layout.x as f64;
                let mut abs_y = node.layout.y as f64;
                let mut parent_id = node.parent;
                while let Some(pid) = parent_id {
                    if let Some(parent_node) = d.tree.get(pid) {
                        abs_x += parent_node.layout.x as f64;
                        abs_y += parent_node.layout.y as f64;
                        abs_x -= parent_node.scroll_offset.0;
                        abs_y -= parent_node.scroll_offset.1;
                        parent_id = parent_node.parent;
                    } else {
                        break;
                    }
                }

                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = node.attributes.get("value").cloned().unwrap_or_default();
                    if value.is_empty() {
                        return DebugResult::Error {
                            message: "No text content".into(),
                        };
                    }

                    let computed_style = node.computed_style.clone();
                    let input_width = node.layout.width;
                    drop(d);

                    let layout = computed_style.build_parley_layout(
                        &value,
                        scale,
                        &mut self.hit_test_font_cx,
                        &mut self.paint_layout_cx,
                        Some(input_width),
                    );

                    match glyph_bounds_for_offset_layout(&layout, byte_offset) {
                        Some(bounds) => {
                            let padding_left =
                                computed_style.padding_left.to_px() as f64 * scale as f64;
                            let padding_top =
                                computed_style.padding_top.to_px() as f64 * scale as f64;
                            return DebugResult::Json {
                                data: json!({
                                    "x": abs_x + padding_left + bounds.x as f64,
                                    "y": abs_y + padding_top + bounds.y as f64,
                                    "width": bounds.width,
                                    "height": bounds.height,
                                }),
                            };
                        }
                        None => {
                            return DebugResult::Error {
                                message: "Byte offset out of bounds".into(),
                            };
                        }
                    }
                }

                if let Some(ref inline_layout) = node.text_layout {
                    match glyph_bounds_for_offset_layout(&inline_layout.layout, byte_offset) {
                        Some(bounds) => {
                            return DebugResult::Json {
                                data: json!({
                                    "x": abs_x + bounds.x as f64,
                                    "y": abs_y + bounds.y as f64,
                                    "width": bounds.width,
                                    "height": bounds.height,
                                }),
                            };
                        }
                        None => {
                            return DebugResult::Error {
                                message: "Byte offset out of bounds".into(),
                            };
                        }
                    }
                }

                DebugResult::Error {
                    message: "Node does not have text layout".into(),
                }
            }
        }
    }
}

// ── Free functions (platform-agnostic hit testing) ───────────────────────────

/// Simple hit testing: find the deepest node whose layout rect contains (x, y).
pub(crate) fn hit_test(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> Option<usize> {
    hit_test_node(tree, tree.body_id, 0.0, 0.0, x, y)
}

fn hit_test_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
) -> Option<usize> {
    let node = tree.get(node_id)?;

    // Skip elements with pointer-events: none
    if let Some(style) = node.attributes.get("style")
        && (style.contains("pointer-events: none") || style.contains("pointer-events:none"))
    {
        return None;
    }

    let nx = offset_x + node.layout.x;
    let ny = offset_y + node.layout.y;
    let nw = node.layout.width;
    let nh = node.layout.height;

    if x < nx || x > nx + nw || y < ny || y > ny + nh {
        return None;
    }

    // Check children in reverse order (topmost first)
    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;
    let children: Vec<_> = node.children.clone();
    for &child_id in children.iter().rev() {
        if let Some(hit) = hit_test_node(tree, child_id, nx - sx, ny - sy, x, y) {
            return Some(hit);
        }
    }

    Some(node_id)
}

/// Find the nearest ancestor (or self) that is a scroll container.
pub(crate) fn find_scroll_container(tree: &rinch_dom::NodeTree, start: usize) -> Option<usize> {
    use rinch_dom::computed_style::OverflowValue;

    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        let overflow_y = &node.computed_style.overflow_y;
        match overflow_y {
            OverflowValue::Scroll | OverflowValue::Auto => return Some(node_id),
            OverflowValue::Hidden => {
                let content_h = compute_content_height(tree, node_id);
                if content_h > node.layout.height as f64 {
                    return Some(node_id);
                }
            }
            _ => {}
        }
        current = node.parent;
    }
    // Fall back to body if content overflows
    let body = tree.get(tree.body_id)?;
    let content_h = compute_content_height(tree, tree.body_id);
    if content_h > body.layout.height as f64 {
        return Some(tree.body_id);
    }
    None
}

/// Compute the total content height of a node from its children's layout bounds.
pub(crate) fn compute_content_height(tree: &rinch_dom::NodeTree, node_id: usize) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    let mut max_bottom: f64 = 0.0;
    for &child_id in &node.children {
        if let Some(child) = tree.get(child_id) {
            let bottom = (child.layout.y + child.layout.height) as f64;
            if bottom > max_bottom {
                max_bottom = bottom;
            }
        }
    }
    max_bottom
}

/// Check if a point (x, y) hits a scrollbar.
pub(crate) fn find_scrollbar_hit(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<(usize, f64, f64)> {
    find_scrollbar_hit_node(tree, tree.body_id, 0.0, 0.0, x, y)
}

fn find_scrollbar_hit_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
) -> Option<(usize, f64, f64)> {
    let node = tree.get(node_id)?;
    let nx = offset_x + node.layout.x;
    let ny = offset_y + node.layout.y;
    let nw = node.layout.width;
    let nh = node.layout.height;

    if x < nx || x > nx + nw || y < ny || y > ny + nh {
        return None;
    }

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;
    let children: Vec<_> = node.children.clone();
    for &child_id in children.iter().rev() {
        if let Some(hit) = find_scrollbar_hit_node(tree, child_id, nx - sx, ny - sy, x, y) {
            return Some(hit);
        }
    }

    use rinch_dom::computed_style::OverflowValue;
    let overflow_y = &node.computed_style.overflow_y;

    if matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto) {
        let content_height = compute_content_height(tree, node_id);
        let container_height = nh as f64;

        if content_height > container_height {
            let scrollbar_hit_width: f32 = 16.0;
            let scrollbar_left = nx + nw - scrollbar_hit_width;

            if x >= scrollbar_left && x <= nx + nw && y >= ny && y <= ny + nh {
                return Some((node_id, content_height, container_height));
            }
        }
    }

    None
}

/// Compute the absolute Y position of a node by walking up its parent chain.
pub(crate) fn compute_absolute_y(tree: &rinch_dom::NodeTree, node_id: usize) -> f32 {
    let mut y = 0.0_f32;
    let mut current = Some(node_id);
    while let Some(id) = current {
        if let Some(node) = tree.get(id) {
            y += node.layout.y;
            if let Some(parent_id) = node.parent
                && let Some(parent) = tree.get(parent_id)
            {
                y -= parent.scroll_offset.1 as f32;
            }
            current = node.parent;
        } else {
            break;
        }
    }
    y
}
