//! Platform-agnostic application logic.
//!
//! `RinchApp` holds the reactive document, scene graph, cursor state, devtools,
//! and all input-handling logic that is independent of the windowing backend.
//! The desktop shell (winit + wgpu) translates native events into
//! [`PlatformEvent`]s, feeds them to `RinchApp`, and processes the returned
//! [`AppAction`]s.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};
use rinch_core::events;
use rinch_core::hooks::{begin_render, end_render};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::{byte_offset_from_position, caret_position_for_offset_layout};
use rinch_editable::{
    InputHandler, Key as EditKey,
    Modifiers as EditModifiers,
};
#[cfg(feature = "debug")]
use rinch_dom::text_query::glyph_bounds_for_offset_layout;
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

// ── ContentEditable focus ────────────────────────────────────────────────────

/// A cursor position within the DOM: a specific text node and byte offset,
/// or a block element ID for empty blocks (offset always 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DomCursor {
    /// DOM node ID — either a text node or an empty block element.
    node_id: usize,
    /// Byte offset within the text node's content (always 0 for element cursors).
    offset: usize,
}

/// A snapshot of text node contents for undo.
#[derive(Debug, Clone)]
struct UndoEntry {
    cursor: DomCursor,
    anchor: DomCursor,
    text_snapshots: Vec<(usize, String)>, // (node_id, old_text_content)
    created_nodes: Vec<usize>,            // nodes created during the edit (removed on undo)
}

/// State for a focused contenteditable element.
pub(crate) struct ContentEditableFocus {
    /// The node ID of the focused contenteditable root element.
    ce_node_id: usize,
    /// Caret position.
    cursor: DomCursor,
    /// Selection anchor (same as cursor when no selection).
    anchor: DomCursor,
    /// Input handler for mapping keys to edit commands (from rinch_editable).
    input_handler: InputHandler,
    /// Undo stack for text changes.
    undo_stack: Vec<UndoEntry>,
}

impl std::fmt::Debug for ContentEditableFocus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentEditableFocus")
            .field("ce_node_id", &self.ce_node_id)
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .finish()
    }
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
    /// State for a focused contenteditable element.
    pub(crate) focused_contenteditable: Option<ContentEditableFocus>,
    /// Whether we're currently doing a mouse-drag text selection in a contenteditable.
    pub(crate) ce_selecting: bool,
    /// Whether a scroll-into-view is pending for the focused contenteditable.
    /// Set when cursor changes; applied after the next layout resolve.
    /// Uses `Cell` for interior mutability (set from `&self` methods).
    pub(crate) ce_scroll_pending: Cell<bool>,
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
            focused_contenteditable: None,
            ce_selecting: false,
            ce_scroll_pending: Cell::new(false),
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

        // Apply deferred scroll-into-view now that layout is fresh
        self.apply_ce_scroll_into_view();

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

                // Handle contenteditable text selection drag
                if self.ce_selecting {
                    if let Some(ref mut ce) = self.focused_contenteditable {
                        let ce_node_id = ce.ce_node_id;
                        if let Some(doc) = &self.doc {
                            let new_cursor = {
                                let d = doc.borrow();
                                Self::compute_dom_cursor_from_click(&d.tree, ce_node_id, x, y)
                            };
                            ce.cursor = new_cursor;
                            let anchor = ce.anchor;
                            self.set_contenteditable_attributes_dom(
                                ce_node_id, true, new_cursor, anchor,
                            );
                            self.scene_dirty = true;
                            actions.push(AppAction::RequestRedraw);
                            return actions;
                        }
                    }
                }

                // Update hover state for CSS :hover support
                if let Some(doc) = &self.doc {
                    let (hovered, cursor_style) = {
                        let d = doc.borrow();
                        let h = hit_test(&d.tree, x, y);
                        let cs = h
                            .and_then(|id| d.tree.get(id))
                            .map(|n| cursor_value_to_style(&n.computed_style.cursor))
                            .unwrap_or(rinch_platform::CursorStyle::Default);
                        (h, cs)
                    };
                    let changed = doc.borrow_mut().update_hover(hovered);
                    if changed {
                        actions.push(AppAction::RequestRedraw);
                    }
                    actions.push(AppAction::SetCursor(cursor_style));
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

                // Update :active and :focus pseudo-class state
                if let Some(doc) = &self.doc {
                    let hit = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    // :active applies while mouse is pressed
                    let active_changed = doc.borrow_mut().update_active(hit);
                    // :focus applies to the clicked element (persists after release)
                    let focus_changed = doc.borrow_mut().update_focus(hit);
                    if active_changed || focus_changed {
                        actions.push(AppAction::RequestRedraw);
                    }
                }

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
                self.ce_selecting = false;

                // Clear :active pseudo-class state on mouse release
                if let Some(doc) = &self.doc {
                    let changed = doc.borrow_mut().update_active(None);
                    if changed {
                        actions.push(AppAction::RequestRedraw);
                    }
                }
            }
            PlatformEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                self.cursor_pos = Some((x, y));
                if let Some(doc) = &self.doc {
                    let hit_node = hit_test(&doc.borrow().tree, x, y);
                    if let Some(hit_node) = hit_node {
                        let mut doc_mut = doc.borrow_mut();

                        // Vertical scrolling
                        if delta_y.abs() > 0.0 {
                            if let Some(scroll_node_id) = find_scroll_container(&doc_mut.tree, hit_node)
                            {
                                let content_height =
                                    compute_content_height(&doc_mut.tree, scroll_node_id);
                                let visible_height = compute_visible_content_area_height(
                                    &doc_mut.tree, scroll_node_id,
                                );
                                let max_scroll = (content_height - visible_height).max(0.0);

                                if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                    let new_y = (node.scroll_offset.1 - delta_y).clamp(0.0, max_scroll);
                                    if new_y != node.scroll_offset.1 {
                                        node.scroll_offset.1 = new_y;
                                        node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                        doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                    }
                                }
                            }
                        }

                        // Horizontal scrolling
                        if delta_x.abs() > 0.0 {
                            if let Some(scroll_node_id) = find_horizontal_scroll_container(&doc_mut.tree, hit_node)
                            {
                                let content_width =
                                    compute_content_width(&doc_mut.tree, scroll_node_id);
                                let visible_width = compute_visible_content_area_width(
                                    &doc_mut.tree, scroll_node_id,
                                );
                                let max_scroll = (content_width - visible_width).max(0.0);

                                if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                    let new_x = (node.scroll_offset.0 - delta_x).clamp(0.0, max_scroll);
                                    if new_x != node.scroll_offset.0 {
                                        node.scroll_offset.0 = new_x;
                                        node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                        doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                    }
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
                } else if self.focused_contenteditable.is_some() {
                    // Route keyboard events to the contenteditable editing state
                    if self.handle_contenteditable_key(key, text.as_deref(), shift, ctrl, alt) {
                        // Resolve layout immediately so the IFC text layout is
                        // rebuilt before the next paint.  Without this, the
                        // invalidated text_layout (set to None by set_text_content)
                        // causes a one-frame flicker where text is invisible.
                        let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                        self.resolve_and_repaint(w, h);
                        actions.push(AppAction::RequestRedraw);
                    }
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

        // ── Phase 1: contenteditable detection (short borrow) ───────
        // Do a quick read-only scan to decide if we hit a contenteditable.
        // Gather all needed data, then drop the borrow before mutating.
        enum CeAction {
            /// We hit a contenteditable element — focus it.
            Focus {
                ce_node_id: usize,
                dom_cursor: DomCursor,
                prev_node_id: Option<usize>,
            },
            /// We did NOT hit contenteditable — clear previous if any.
            Clear { prev_node_id: Option<usize> },
            /// No hit at all.
            NoHit,
        }

        let ce_action = {
            let d = doc.borrow();
            if let Some(hit_id) = hit_test(&d.tree, x, y) {
                let mut ce_result = None;
                let mut check = Some(hit_id);
                while let Some(nid) = check {
                    if let Some(node) = d.tree.get(nid) {
                        if let Some(ce_val) = node.attributes.get("contenteditable") {
                            let is_editable = matches!(ce_val.as_str(), "plaintext-only" | "true" | "");
                            if is_editable {
                                let dom_cursor = Self::compute_dom_cursor_from_click(
                                    &d.tree, nid, x, y,
                                );
                                ce_result = Some((nid, dom_cursor));
                            }
                            break;
                        }
                        check = node.parent;
                    } else {
                        break;
                    }
                }

                let prev_node_id =
                    self.focused_contenteditable.as_ref().map(|f| f.ce_node_id);

                if let Some((ce_node_id, dom_cursor)) = ce_result {
                    CeAction::Focus {
                        ce_node_id,
                        dom_cursor,
                        prev_node_id,
                    }
                } else {
                    CeAction::Clear { prev_node_id }
                }
            } else {
                CeAction::NoHit
            }
        }; // d dropped here

        // ── Phase 2: apply contenteditable mutations ────────────────
        match ce_action {
            CeAction::Focus {
                ce_node_id,
                mut dom_cursor,
                prev_node_id,
            } => {
                let input_handler = InputHandler::new()
                    .with_multiline(true)
                    .with_macos(cfg!(target_os = "macos"));

                // Handle double-click (word select) and triple-click (line select)
                let mut anchor = dom_cursor;
                match self.click_count {
                    2 => {
                        // Double-click: select word at cursor position
                        if let Some(doc) = &self.doc {
                            let d = doc.borrow();
                            if let Some(node) = d.tree.get(dom_cursor.node_id)
                                && let Some(text) = node.text_content()
                            {
                                let ws = Self::find_word_start(text, dom_cursor.offset);
                                let we = Self::find_word_end(text, dom_cursor.offset);
                                anchor = DomCursor { node_id: dom_cursor.node_id, offset: ws };
                                dom_cursor = DomCursor { node_id: dom_cursor.node_id, offset: we };
                            }
                        }
                    }
                    3 => {
                        // Triple-click: select all text in the CE
                        if let Some(doc) = &self.doc {
                            let d = doc.borrow();
                            if let Some(first) = Self::first_text_cursor(&d.tree, ce_node_id) {
                                anchor = first;
                            }
                            if let Some(last) = Self::last_text_cursor(&d.tree, ce_node_id) {
                                dom_cursor = last;
                            }
                        }
                    }
                    _ => {} // Single click: cursor already set
                }

                self.focused_contenteditable = Some(ContentEditableFocus {
                    ce_node_id,
                    cursor: dom_cursor,
                    anchor,
                    input_handler,
                    undo_stack: Vec::new(),
                });

                // Start mouse-drag selection tracking
                self.ce_selecting = true;

                // Clear regular input focus
                self.focused_input_handler_id = None;
                self.focused_input_value.clear();

                // Clear previous contenteditable focus attributes
                if let Some(prev_id) = prev_node_id
                    && prev_id != ce_node_id
                {
                    self.set_contenteditable_attributes(prev_id, false, 0, 0);
                }
                // Set cursor/selection attributes on the new focused node
                self.set_contenteditable_attributes_dom(
                    ce_node_id, true, dom_cursor, anchor,
                );
                self.scene_dirty = true;
                actions.push(AppAction::RequestRedraw);
                return actions;
            }
            CeAction::Clear { prev_node_id } => {
                if let Some(prev_id) = prev_node_id {
                    self.focused_contenteditable = None;
                    self.set_contenteditable_attributes(prev_id, false, 0, 0);
                    self.scene_dirty = true;
                }
            }
            CeAction::NoHit => {
                return actions;
            }
        }

        // ── Phase 3: normal click handling (data-oninput, data-rid) ─
        let d = doc.borrow();
        let Some(hit_id) = hit_test(&d.tree, x, y) else {
            return actions;
        };

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

    // ── Contenteditable helpers ────────────────────────────────────────

    /// Extract plain text content from a node and its children.
    /// Block-level elements are separated by newlines (flat text model).
    fn extract_text_content(tree: &rinch_dom::NodeTree, node_id: usize) -> String {
        let mut text = String::new();
        Self::collect_text_recursive(tree, node_id, &mut text);
        // Remove trailing newline if present
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    fn collect_text_recursive(tree: &rinch_dom::NodeTree, node_id: usize, out: &mut String) {
        if let Some(node) = tree.get(node_id) {
            if let Some(t) = node.text_content() {
                out.push_str(t);
            } else {
                // Check if this is a block element
                let is_block = node.tag().map(Self::is_block_element).unwrap_or(false);
                if is_block && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                for &child_id in &node.children {
                    Self::collect_text_recursive(tree, child_id, out);
                }
                // Add newline after block element content (if it had children)
                if is_block && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }

    /// Check if a tag name represents a block-level element.
    fn is_block_element(tag: &str) -> bool {
        matches!(
            tag,
            "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                | "li" | "ul" | "ol" | "section" | "article" | "blockquote"
                | "pre" | "hr" | "table" | "tr" | "header" | "footer"
                | "main" | "nav" | "aside" | "figure" | "figcaption"
                | "details" | "summary"
        )
    }

    fn is_heading(tag: &str) -> bool {
        matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
    }

    fn is_list_tag(tag: &str) -> bool {
        matches!(tag, "ul" | "ol")
    }

    /// Strip specific CSS properties from an inline style string.
    fn strip_css_properties(style: &str, properties: &[&str]) -> String {
        style
            .split(';')
            .filter(|decl| {
                let trimmed = decl.trim();
                if trimmed.is_empty() {
                    return false;
                }
                if let Some(prop) = trimmed.split(':').next() {
                    !properties.contains(&prop.trim())
                } else {
                    true
                }
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// Change a block element's tag while preserving children and attributes.
    /// Returns the new node's `NodeId`.
    fn convert_block_tag(d: &mut RinchDocument, block_id: usize, new_tag: &str) -> rinch_core::dom::NodeId {
        let old_tag = d.tree.get(block_id).and_then(|n| n.tag()).unwrap_or("").to_string();
        let new_el = d.create_element(new_tag);
        // Copy style/class attributes
        if let Some(style) = d.tree.get(block_id).and_then(|n| n.attributes.get("style")).cloned() {
            // When converting heading → non-heading, strip heading-specific CSS properties
            // (font-size, font-weight) so the div reverts to normal text styling
            let style = if Self::is_heading(&old_tag) && !Self::is_heading(new_tag) {
                Self::strip_css_properties(&style, &["font-size", "font-weight"])
            } else {
                style
            };
            if !style.trim().is_empty() {
                d.set_attribute(new_el, "style", &style);
            }
        }
        if let Some(class) = d.tree.get(block_id).and_then(|n| n.attributes.get("class")).cloned() {
            d.set_attribute(new_el, "class", &class);
        }
        // Move all children
        let children: Vec<usize> = d.tree.nodes[block_id].children.clone();
        for &child_id in &children {
            d.remove_node(rinch_core::dom::NodeId(child_id));
            d.append_child(new_el, rinch_core::dom::NodeId(child_id));
        }
        // Replace in parent: insert new element at same position, then remove old
        let parent_id = d.tree.get(block_id).and_then(|n| n.parent).unwrap_or(0);
        let next_sib = {
            let siblings = &d.tree.nodes[parent_id].children;
            let pos = siblings.iter().position(|&c| c == block_id);
            pos.and_then(|p| siblings.get(p + 1).copied())
        };
        if let Some(next) = next_sib {
            d.insert_before(
                rinch_core::dom::NodeId(parent_id),
                new_el,
                rinch_core::dom::NodeId(next),
            );
        } else {
            d.append_child(rinch_core::dom::NodeId(parent_id), new_el);
        }
        d.remove_node(rinch_core::dom::NodeId(block_id));
        new_el
    }

    /// Outdent a `<li>` from its parent list: convert to `<div>`, split list if needed.
    /// Works for any position (first, middle, last).
    /// Returns the new `<div>` node id.
    fn outdent_li(
        d: &mut RinchDocument,
        li_id: usize,
        list_id: usize,
        ce_root: usize,
    ) -> rinch_core::dom::NodeId {
        let list_tag = d.tree.get(list_id).and_then(|n| n.tag()).unwrap_or("ul").to_string();
        let grandparent_id = d.tree.get(list_id).and_then(|n| n.parent).unwrap_or(ce_root);
        let grandparent_tag = d.tree.get(grandparent_id).and_then(|n| n.tag()).unwrap_or("").to_string();

        // If nested (parent <ul> is inside another <li>), move <li> up one level
        // like Shift+Tab. Only convert to <div> when at the top level.
        if grandparent_tag == "li" {
            let parent_li_id = grandparent_id;
            let outer_list_id = d.tree.get(parent_li_id).and_then(|n| n.parent).unwrap_or(ce_root);

            // Collect siblings after current <li> in the nested list
            let nested_siblings = d.tree.nodes[list_id].children.clone();
            let pos = nested_siblings.iter().position(|&c| c == li_id).unwrap_or(0);
            let after_siblings: Vec<usize> = nested_siblings[pos + 1..].to_vec();

            // Move current <li> to after parent_li in the outer list
            d.remove_node(rinch_core::dom::NodeId(li_id));
            let parent_li_next = {
                let siblings = &d.tree.nodes[outer_list_id].children;
                let ppos = siblings.iter().position(|&c| c == parent_li_id);
                ppos.and_then(|p| siblings.get(p + 1).copied())
            };
            if let Some(next) = parent_li_next {
                d.insert_before(
                    rinch_core::dom::NodeId(outer_list_id),
                    rinch_core::dom::NodeId(li_id),
                    rinch_core::dom::NodeId(next),
                );
            } else {
                d.append_child(
                    rinch_core::dom::NodeId(outer_list_id),
                    rinch_core::dom::NodeId(li_id),
                );
            }

            // If there are siblings after, create new nested list under current li
            if !after_siblings.is_empty() {
                let new_nested = d.create_element(&list_tag);
                for &sib_id in &after_siblings {
                    d.remove_node(rinch_core::dom::NodeId(sib_id));
                    d.append_child(new_nested, rinch_core::dom::NodeId(sib_id));
                }
                d.append_child(rinch_core::dom::NodeId(li_id), new_nested);
            }

            // If the original nested list is now empty, remove it
            if d.tree.nodes[list_id].children.is_empty() {
                d.remove_node(rinch_core::dom::NodeId(list_id));
            }

            return rinch_core::dom::NodeId(li_id);
        }

        // Top-level: convert <li> to <div> and remove from list

        // Get position and collect siblings after this <li>
        let siblings = d.tree.nodes[list_id].children.clone();
        let pos = siblings.iter().position(|&c| c == li_id).unwrap_or(0);
        let after_siblings: Vec<usize> = siblings[pos + 1..].to_vec();

        // Convert <li> to <div>
        let new_el = Self::convert_block_tag(d, li_id, "div");
        // convert_block_tag replaces in parent, so new_el is now a child of list_id.
        // Remove it from the list.
        d.remove_node(new_el);

        if pos == 0 {
            // First item: insert <div> before the list
            d.insert_before(
                rinch_core::dom::NodeId(grandparent_id),
                new_el,
                rinch_core::dom::NodeId(list_id),
            );
        } else {
            // Non-first: insert <div> after the list
            // Find what comes after list_id in grandparent
            let gp_children = d.tree.nodes[grandparent_id].children.clone();
            let list_pos = gp_children.iter().position(|&c| c == list_id);
            let next_after_list = list_pos.and_then(|p| gp_children.get(p + 1).copied());
            if let Some(next_id) = next_after_list {
                d.insert_before(
                    rinch_core::dom::NodeId(grandparent_id),
                    new_el,
                    rinch_core::dom::NodeId(next_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(grandparent_id), new_el);
            }
        }

        // If there are siblings after, move them to a new list after the <div>
        if !after_siblings.is_empty() {
            let new_list = d.create_element(&list_tag);
            // Copy list style if any
            if let Some(style) = d.tree.get(list_id).and_then(|n| n.attributes.get("style")).cloned() {
                d.set_attribute(new_list, "style", &style);
            }
            for &sib_id in &after_siblings {
                d.remove_node(rinch_core::dom::NodeId(sib_id));
                d.append_child(new_list, rinch_core::dom::NodeId(sib_id));
            }
            // Insert new list after the <div>
            let gp_children = d.tree.nodes[grandparent_id].children.clone();
            let div_pos = gp_children.iter().position(|&c| c == new_el.0);
            let next_after_div = div_pos.and_then(|p| gp_children.get(p + 1).copied());
            if let Some(next_id) = next_after_div {
                d.insert_before(
                    rinch_core::dom::NodeId(grandparent_id),
                    new_list,
                    rinch_core::dom::NodeId(next_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(grandparent_id), new_list);
            }
        }

        // If original list is now empty, remove it
        if d.tree.nodes[list_id].children.is_empty() {
            d.remove_node(rinch_core::dom::NodeId(list_id));
        }

        new_el
    }

    /// Walk up from `node_id` to find the nearest block-level ancestor
    /// and its parent. Stops at `ce_root_id` (the contenteditable element).
    /// Returns `(block_element_id, parent_of_block_id)`.
    fn find_block_and_parent(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        ce_root_id: usize,
    ) -> Option<(usize, usize)> {
        // Never return the CE root itself as a block — it would be removed
        if node_id == ce_root_id {
            return None;
        }
        let mut current = node_id;
        loop {
            let parent = tree.get(current)?.parent?;
            let is_block = tree
                .get(current)?
                .tag()
                .map(Self::is_block_element)
                .unwrap_or(false);
            // Skip anonymous block boxes — they're layout-internal wrappers
            // that editing operations should see through transparently.
            let is_anon = tree.get(current).map(|n| n.is_anonymous_block_box).unwrap_or(false);

            if parent == ce_root_id {
                if is_block && !is_anon {
                    return Some((current, parent));
                }
                return None;
            }
            if is_block && !is_anon {
                return Some((current, parent));
            }
            current = parent;
        }
    }

    /// Walk up from a block to find a containing `<li>` whose parent is a list.
    /// This handles the case where `find_block_and_parent` returns a wrapper `<div>`
    /// (created by Tab indent) inside an `<li>` — we want to outdent the `<li>`, not
    /// merge the `<div>` with the previous block.
    fn find_li_ancestor_for_outdent(
        tree: &rinch_dom::NodeTree,
        block_id: usize,
        ce_root: usize,
    ) -> Option<(usize, usize)> {
        let mut current = tree.get(block_id)?.parent?;
        while current != ce_root {
            let tag = tree.get(current)?.tag().unwrap_or("");
            if tag == "li" {
                let parent = tree.get(current)?.parent?;
                let parent_tag = tree.get(parent)?.tag().unwrap_or("");
                if Self::is_list_tag(parent_tag) {
                    return Some((current, parent)); // (li_id, list_id)
                }
            }
            current = tree.get(current)?.parent?;
        }
        None
    }

    /// Find the start of the word containing the given byte position.
    fn find_word_start(text: &str, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let bytes = text.as_bytes();
        let mut i = pos.min(bytes.len());
        // Skip whitespace backwards
        while i > 0 && bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        // Skip word characters backwards
        while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        i
    }

    /// Find the end of the word containing the given byte position.
    fn find_word_end(text: &str, pos: usize) -> usize {
        let len = text.len();
        if pos >= len {
            return len;
        }
        let bytes = text.as_bytes();
        let mut i = pos;
        // Skip word characters forwards
        while i < len && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // Skip whitespace forwards (so next call starts at next word)
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    }

    /// Compute the absolute position of a node by walking up through parents.
    fn compute_absolute_position(tree: &rinch_dom::NodeTree, node_id: usize) -> (f32, f32) {
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut current = Some(node_id);
        while let Some(nid) = current {
            if let Some(node) = tree.get(nid) {
                x += node.layout.x;
                y += node.layout.y;
                if let Some(parent_id) = node.parent
                    && let Some(parent) = tree.get(parent_id)
                {
                    x -= parent.scroll_offset.0 as f32;
                    y -= parent.scroll_offset.1 as f32;
                }
                current = node.parent;
            } else {
                break;
            }
        }
        (x, y)
    }

    /// Convert a `DomCursor` to a global flat byte offset within the CE element.
    ///
    /// Walks the DOM tree depth-first from `ce_root`, accumulating text lengths
    /// and block separators until reaching `cursor.node_id`, then adds
    /// `cursor.offset`.  Used only for writing paint attributes.
    fn dom_cursor_to_global_offset(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
    ) -> usize {
        let mut offset = 0usize;
        let mut found = false;
        let mut ends_with_newline = false;
        Self::walk_for_global_offset(tree, ce_root, cursor, &mut offset, &mut found, &mut ends_with_newline);
        offset
    }

    /// Recursive helper for `dom_cursor_to_global_offset`.
    ///
    /// Uses the same `ends_with_newline` deduplication as paint.rs's
    /// `collect_text_len_recursive` to ensure global offsets are consistent
    /// between the cursor-setting code and the cursor-rendering code.
    fn walk_for_global_offset(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        cursor: DomCursor,
        offset: &mut usize,
        found: &mut bool,
        ends_with_newline: &mut bool,
    ) {
        if *found {
            return;
        }
        let Some(node) = tree.get(node_id) else { return };

        if let Some(text) = node.text_content() {
            if node_id == cursor.node_id {
                *offset += cursor.offset.min(text.len());
                *found = true;
                return;
            }
            *offset += text.len();
            *ends_with_newline = text.ends_with('\n');
            return;
        }

        // <br> elements contribute \n to IFC text — must match paint.rs
        if node.tag() == Some("br") {
            if node_id == cursor.node_id {
                // Cursor is AT this <br> — offset points to start of the \n,
                // not after it. This makes empty lines (e.g., br br sequences)
                // each get a distinct global offset.
                *found = true;
                return;
            }
            *offset += 1;
            *ends_with_newline = true;
            return;
        }

        let is_block = node.tag().map(Self::is_block_element).unwrap_or(false);
        if is_block && *offset > 0 && !*ends_with_newline {
            *offset += 1; // block separator \n
            *ends_with_newline = true;
        }

        // Element-level cursor (empty block) — found at the block boundary
        if node_id == cursor.node_id && node.is_element() {
            *found = true;
            return;
        }

        for &child_id in &node.children {
            Self::walk_for_global_offset(tree, child_id, cursor, offset, found, ends_with_newline);
            if *found {
                return;
            }
        }

        // Empty block elements must reset ends_with_newline so consecutive
        // empty blocks each get a unique global offset via their own separator.
        if is_block && node.children.is_empty() {
            *ends_with_newline = false;
        }

        if is_block && *offset > 0 && !*ends_with_newline {
            *offset += 1; // after-block separator \n
            *ends_with_newline = true;
        }
    }

    /// Compute a `DomCursor` from a click within a contenteditable element.
    ///
    /// Finds the IFC containing the click, uses Parley `from_point()` to get
    /// the IFC flat offset, then converts via `IfcTextRange` to a `DomCursor`.
    fn compute_dom_cursor_from_click(
        tree: &rinch_dom::NodeTree,
        ce_node_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> DomCursor {
        // Case 1: CE element itself has an IFC (inline-only content)
        if let Some(node) = tree.get(ce_node_id)
            && let Some(ref inline_layout) = node.text_layout
        {
            let (abs_x, abs_y) = Self::compute_absolute_position(tree, ce_node_id);
            let padding_left = node.computed_style.padding_left.to_px();
            let padding_top = node.computed_style.padding_top.to_px();
            let rel_x = click_x - abs_x - padding_left + node.scroll_offset.0 as f32;
            let rel_y = click_y - abs_y - padding_top + node.scroll_offset.1 as f32;
            let ifc_offset = rinch_dom::text_query::byte_offset_from_position(
                &inline_layout.layout, rel_x, rel_y,
            );
            if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                &inline_layout.text_ranges, ifc_offset, true,
            ) {
                return DomCursor { node_id: nid, offset: off };
            }
        }

        // Case 2: Recursive search through block children (handles any nesting depth)
        if let Some(cursor) = Self::find_cursor_in_block(tree, ce_node_id, click_x, click_y) {
            return cursor;
        }

        // Click was below all blocks — position at end of last text node
        if let Some(cursor) = Self::last_text_cursor(tree, ce_node_id) {
            return cursor;
        }

        // Fallback: first text node
        Self::first_text_cursor(tree, ce_node_id)
            .unwrap_or(DomCursor { node_id: ce_node_id, offset: 0 })
    }

    /// Recursively find the cursor position in a block element at the given click coordinates.
    /// Handles arbitrary nesting depth (nested lists, divs inside lis, etc.).
    fn find_cursor_in_block(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> Option<DomCursor> {
        let node = tree.get(node_id)?;

        // Check if this node has an IFC (inline formatting context)
        if let Some(ref inline_layout) = node.text_layout {
            let (abs_x, abs_y) = Self::compute_absolute_position(tree, node_id);
            let pad_left = node.computed_style.padding_left.to_px();
            let pad_top = node.computed_style.padding_top.to_px();
            let rel_x = click_x - abs_x - pad_left + node.scroll_offset.0 as f32;
            let rel_y = click_y - abs_y - pad_top + node.scroll_offset.1 as f32;
            let ifc_offset = rinch_dom::text_query::byte_offset_from_position(
                &inline_layout.layout, rel_x, rel_y,
            );
            if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                &inline_layout.text_ranges, ifc_offset, true,
            ) {
                return Some(DomCursor { node_id: nid, offset: off });
            }
        }

        // Check direct text children with cached_text_parley
        for &child_id in &node.children {
            if let Some(child) = tree.get(child_id)
                && let Some(ref cached_layout) = child.cached_text_parley
            {
                let (tc_abs_x, tc_abs_y) = Self::compute_absolute_position(tree, child_id);
                let rx = click_x - tc_abs_x;
                let ry = click_y - tc_abs_y;
                let byte_off = byte_offset_from_position(cached_layout, rx, ry);
                return Some(DomCursor { node_id: child_id, offset: byte_off });
            }
        }

        // Recurse into children by y-range
        let (_, node_abs_y) = Self::compute_absolute_position(tree, node_id);
        let scroll_y = node.scroll_offset.1 as f32;
        for &child_id in &node.children {
            if let Some(child) = tree.get(child_id) {
                let child_abs_y = node_abs_y + child.layout.y - scroll_y;
                let child_bottom = child_abs_y + child.layout.height;
                if click_y >= child_abs_y && click_y < child_bottom {
                    if let Some(cursor) = Self::find_cursor_in_block(tree, child_id, click_x, click_y) {
                        return Some(cursor);
                    }
                }
            }
        }

        // Fallback: first text node in this subtree
        Self::first_text_cursor(tree, node_id)
    }

    /// Find the first text node (depth-first) under `root` and return cursor at offset 0.
    /// For empty block elements, returns an element-level cursor at the block.
    fn first_text_cursor(tree: &rinch_dom::NodeTree, root: usize) -> Option<DomCursor> {
        let node = tree.get(root)?;
        if node.text_content().is_some() {
            return Some(DomCursor { node_id: root, offset: 0 });
        }
        for &child_id in &node.children {
            if let Some(c) = Self::first_text_cursor(tree, child_id) {
                return Some(c);
            }
        }
        // Empty block element — return element-level cursor
        if node.is_element() && node.children.is_empty() && node.tag().map(Self::is_block_element).unwrap_or(false) {
            return Some(DomCursor { node_id: root, offset: 0 });
        }
        None
    }

    /// Find the last text node (depth-first) under `root` and return cursor at end.
    /// For empty block elements, returns an element-level cursor at the block.
    fn last_text_cursor(tree: &rinch_dom::NodeTree, root: usize) -> Option<DomCursor> {
        let node = tree.get(root)?;
        // Check children in reverse
        for &child_id in node.children.iter().rev() {
            if let Some(c) = Self::last_text_cursor(tree, child_id) {
                return Some(c);
            }
        }
        if let Some(text) = node.text_content() {
            return Some(DomCursor { node_id: root, offset: text.len() });
        }
        // Empty block element — return element-level cursor
        if node.is_element() && node.children.is_empty() && node.tag().map(Self::is_block_element).unwrap_or(false) {
            return Some(DomCursor { node_id: root, offset: 0 });
        }
        None
    }

    /// Compute line-height in pixels from a block element's computed style.
    fn line_height_px(tree: &rinch_dom::NodeTree, block_id: usize) -> f32 {
        if let Some(node) = tree.get(block_id) {
            let cs = &node.computed_style;
            match cs.line_height {
                rinch_dom::computed_style::LineHeightValue::Absolute(px) => px,
                rinch_dom::computed_style::LineHeightValue::Relative(factor) => cs.font_size * factor,
                rinch_dom::computed_style::LineHeightValue::Normal => cs.font_size * 1.2,
            }
        } else {
            19.2 // fallback: 16px * 1.2
        }
    }

    /// Check if a DomCursor targets an element (empty block) rather than a text node.
    fn is_element_cursor(tree: &rinch_dom::NodeTree, cursor: &DomCursor) -> bool {
        tree.get(cursor.node_id)
            .map(|n| n.is_element())
            .unwrap_or(false)
    }

    /// Snapshot all text nodes under `root` for undo.
    fn snapshot_text_nodes(tree: &rinch_dom::NodeTree, root: usize) -> Vec<(usize, String)> {
        let mut result = Vec::new();
        Self::snapshot_text_nodes_recursive(tree, root, &mut result);
        result
    }

    fn snapshot_text_nodes_recursive(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        result: &mut Vec<(usize, String)>,
    ) {
        if let Some(node) = tree.get(node_id) {
            if let Some(text) = node.text_content() {
                result.push((node_id, text.to_string()));
            }
            for &child_id in &node.children {
                Self::snapshot_text_nodes_recursive(tree, child_id, result);
            }
        }
    }

    /// Collect all node IDs in a subtree (for undo diffing).
    fn collect_subtree_ids(tree: &rinch_dom::NodeTree, root: usize) -> Vec<usize> {
        let mut ids = Vec::new();
        Self::collect_subtree_ids_recursive(tree, root, &mut ids);
        ids
    }

    fn collect_subtree_ids_recursive(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        ids: &mut Vec<usize>,
    ) {
        ids.push(node_id);
        if let Some(node) = tree.get(node_id) {
            for &child_id in &node.children {
                Self::collect_subtree_ids_recursive(tree, child_id, ids);
            }
        }
    }

    /// Set/clear contenteditable cursor attributes on a DOM node.
    ///
    /// Converts `DomCursor` values to global flat offsets for paint compatibility.
    fn set_contenteditable_attributes_dom(
        &self,
        ce_node_id: usize,
        focused: bool,
        cursor: DomCursor,
        anchor: DomCursor,
    ) {
        if let Some(doc) = &self.doc {
            let (cursor_off, anchor_off) = if focused {
                let d = doc.borrow();
                let c = Self::dom_cursor_to_global_offset(&d.tree, ce_node_id, cursor);
                let a = Self::dom_cursor_to_global_offset(&d.tree, ce_node_id, anchor);
                (c, a)
            } else {
                (0, 0)
            };
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(ce_node_id) {
                if focused {
                    node.attributes
                        .insert("data-ce-focused".to_string(), "true".to_string());
                    node.attributes
                        .insert("data-ce-cursor".to_string(), cursor_off.to_string());
                    node.attributes.insert(
                        "data-ce-selection-start".to_string(),
                        anchor_off.to_string(),
                    );
                } else {
                    node.attributes.remove("data-ce-focused");
                    node.attributes.remove("data-ce-cursor");
                    node.attributes.remove("data-ce-selection-start");
                }
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
            }

            // Mark scroll-into-view as pending; applied after layout resolve
            // when text_layout is valid.
            if focused {
                self.ce_scroll_pending.set(true);
            }

            d.tree.dirty_nodes.insert(ce_node_id);
        }
    }

    /// Apply deferred scroll-into-view for the focused contenteditable.
    ///
    /// Must be called AFTER `resolve_layout` so that text_layout is valid.
    fn apply_ce_scroll_into_view(&mut self) {
        if !self.ce_scroll_pending.get() {
            return;
        }
        self.ce_scroll_pending.set(false);

        let Some(ref ce) = self.focused_contenteditable else { return };
        let cursor = ce.cursor;
        let ce_node_id = ce.ce_node_id;

        if let Some(doc) = &self.doc {
            let cursor_off = {
                let d = doc.borrow();
                Self::dom_cursor_to_global_offset(&d.tree, ce_node_id, cursor)
            };
            let mut d = doc.borrow_mut();
            if let Some(new_scroll) =
                compute_ce_scroll_target(&d.tree, ce_node_id, cursor, cursor_off)
            {
                if let Some(node) = d.tree.nodes.get_mut(ce_node_id) {
                    node.scroll_offset.1 = new_scroll;
                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                }
                d.tree.dirty_nodes.insert(ce_node_id);
            }
        }
    }

    /// Legacy wrapper — keeps old call sites working during migration.
    fn set_contenteditable_attributes(
        &self,
        node_id: usize,
        focused: bool,
        cursor: usize,
        selection_start: usize,
    ) {
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(node_id) {
                if focused {
                    node.attributes
                        .insert("data-ce-focused".to_string(), "true".to_string());
                    node.attributes
                        .insert("data-ce-cursor".to_string(), cursor.to_string());
                    node.attributes.insert(
                        "data-ce-selection-start".to_string(),
                        selection_start.to_string(),
                    );
                } else {
                    node.attributes.remove("data-ce-focused");
                    node.attributes.remove("data-ce-cursor");
                    node.attributes.remove("data-ce-selection-start");
                }
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
            }
            d.tree.dirty_nodes.insert(node_id);
        }
    }

    /// Handle a keyboard event for a focused contenteditable element.
    /// Returns true if the event was handled and a redraw is needed.
    fn handle_contenteditable_key(
        &mut self,
        key: KeyCode,
        text: Option<&str>,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> bool {
        let ce = match self.focused_contenteditable.as_mut() {
            Some(ce) => ce,
            None => return false,
        };

        let modifiers = EditModifiers {
            ctrl,
            shift,
            alt,
            meta: false,
        };

        // Map KeyCode to rinch_editable::Key
        let edit_key = match key {
            KeyCode::ArrowLeft => Some(EditKey::Left),
            KeyCode::ArrowRight => Some(EditKey::Right),
            KeyCode::ArrowUp => Some(EditKey::Up),
            KeyCode::ArrowDown => Some(EditKey::Down),
            KeyCode::Home => Some(EditKey::Home),
            KeyCode::End => Some(EditKey::End),
            KeyCode::Backspace => Some(EditKey::Backspace),
            KeyCode::Delete => Some(EditKey::Delete),
            KeyCode::Enter => Some(EditKey::Enter),
            KeyCode::Tab => Some(EditKey::Tab),
            KeyCode::Escape => Some(EditKey::Escape),
            KeyCode::KeyA if ctrl => Some(EditKey::A),
            KeyCode::KeyC if ctrl => Some(EditKey::C),
            KeyCode::KeyX if ctrl => Some(EditKey::X),
            KeyCode::KeyZ if ctrl => Some(EditKey::Z),
            KeyCode::KeyY if ctrl => Some(EditKey::Y),
            _ => None,
        };

        // Try mapping the key to an edit command via InputHandler
        let cmd = if let Some(ek) = edit_key {
            ce.input_handler.map_key(ek, modifiers)
        } else {
            None
        };

        // If no command from key mapping, try text input (printable characters)
        let cmd = cmd.or_else(|| {
            if !ctrl && !alt {
                text.and_then(|t| ce.input_handler.map_text(t))
            } else {
                None
            }
        });

        // Special handling for paste (Ctrl+V)
        let cmd = cmd.or_else(|| {
            if ctrl && key == KeyCode::KeyV {
                #[cfg(feature = "clipboard")]
                {
                    if let Ok(clipboard_text) = crate::clipboard::paste_text() {
                        return Some(rinch_editable::EditCommand::Paste(clipboard_text));
                    }
                }
                None
            } else {
                None
            }
        });

        let Some(cmd) = cmd else {
            return false;
        };

        // Extract CE info before borrowing self.doc
        let ce_node_id = ce.ce_node_id;
        let cursor = ce.cursor;
        let anchor = ce.anchor;
        let has_selection = cursor != anchor;

        use rinch_editable::EditCommand;
        let mut text_changed = false;

        // Push undo snapshot before any mutating command
        let is_mutating = matches!(
            cmd,
            EditCommand::InsertText(_)
            | EditCommand::Paste(_)
            | EditCommand::DeleteBackward
            | EditCommand::DeleteForward
            | EditCommand::InsertNewline
            | EditCommand::Cut
            | EditCommand::Indent
            | EditCommand::Outdent
        );
        let mut pre_edit_ids: Vec<usize> = Vec::new();
        if is_mutating {
            if let Some(doc) = &self.doc {
                let d = doc.borrow();
                let snapshots = Self::snapshot_text_nodes(&d.tree, ce_node_id);
                pre_edit_ids = Self::collect_subtree_ids(&d.tree, ce_node_id);
                let ce = self.focused_contenteditable.as_mut().unwrap();
                ce.undo_stack.push(UndoEntry {
                    cursor,
                    anchor,
                    text_snapshots: snapshots,
                    created_nodes: Vec::new(),
                });
                // Cap undo stack at 100 entries
                if ce.undo_stack.len() > 100 {
                    ce.undo_stack.remove(0);
                }
            }
        }

        match cmd {
            // ── Character insertion ──────────────────────────────────
            EditCommand::InsertText(ref insert_str) => {
                if has_selection {
                    self.ce_delete_selection();
                }
                let ce = self.focused_contenteditable.as_mut().unwrap();
                let cur = ce.cursor;
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
                    // Check if cursor is on a <br> element
                    let is_br = d.tree.get(cur.node_id)
                        .and_then(|n| n.tag())
                        .map(|t| t == "br")
                        .unwrap_or(false);

                    if is_br {
                        // <br> cursor: create text node and insert before the <br>
                        let parent_id = d.tree.get(cur.node_id)
                            .and_then(|n| n.parent)
                            .unwrap_or(ce_node_id);
                        let text_id = d.create_text(insert_str);
                        d.insert_before(
                            rinch_core::dom::NodeId(parent_id),
                            text_id,
                            rinch_core::dom::NodeId(cur.node_id),
                        );
                        ce.cursor = DomCursor { node_id: text_id.0, offset: insert_str.len() };
                        ce.anchor = ce.cursor;
                    } else if let Some(node) = d.tree.get(cur.node_id)
                        && let Some(current) = node.text_content().map(|s| s.to_string())
                    {
                        let mut new_text = String::with_capacity(current.len() + insert_str.len());
                        let off = cur.offset.min(current.len());
                        new_text.push_str(&current[..off]);
                        new_text.push_str(insert_str);
                        new_text.push_str(&current[off..]);
                        d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                        ce.cursor = DomCursor { node_id: cur.node_id, offset: off + insert_str.len() };
                        ce.anchor = ce.cursor;
                    } else {
                        // Cursor is on an element node (empty block).
                        // Create a text node with the inserted text and remove min-height.
                        let text_id = d.create_text(insert_str);
                        d.append_child(rinch_core::dom::NodeId(cur.node_id), text_id);
                        d.set_style(rinch_core::dom::NodeId(cur.node_id), "min-height", "0");
                        ce.cursor = DomCursor { node_id: text_id.0, offset: insert_str.len() };
                        ce.anchor = ce.cursor;
                    }
                }
                text_changed = true;
            }
            EditCommand::Paste(ref paste_text) => {
                if has_selection {
                    self.ce_delete_selection();
                }
                let ce = self.focused_contenteditable.as_mut().unwrap();
                let cur = ce.cursor;
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
                    // Check if cursor is on a <br> element
                    let is_br = d.tree.get(cur.node_id)
                        .and_then(|n| n.tag())
                        .map(|t| t == "br")
                        .unwrap_or(false);

                    if is_br {
                        // <br> cursor: create text node and insert before the <br>
                        let parent_id = d.tree.get(cur.node_id)
                            .and_then(|n| n.parent)
                            .unwrap_or(ce_node_id);
                        let text_id = d.create_text(paste_text);
                        d.insert_before(
                            rinch_core::dom::NodeId(parent_id),
                            text_id,
                            rinch_core::dom::NodeId(cur.node_id),
                        );
                        ce.cursor = DomCursor { node_id: text_id.0, offset: paste_text.len() };
                        ce.anchor = ce.cursor;
                    } else if let Some(node) = d.tree.get(cur.node_id)
                        && let Some(current) = node.text_content().map(|s| s.to_string())
                    {
                        let mut new_text = String::with_capacity(current.len() + paste_text.len());
                        let off = cur.offset.min(current.len());
                        new_text.push_str(&current[..off]);
                        new_text.push_str(paste_text);
                        new_text.push_str(&current[off..]);
                        d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                        ce.cursor = DomCursor { node_id: cur.node_id, offset: off + paste_text.len() };
                        ce.anchor = ce.cursor;
                    } else {
                        // Cursor is on an element node (empty block) — create text child
                        let text_id = d.create_text(paste_text);
                        d.append_child(rinch_core::dom::NodeId(cur.node_id), text_id);
                        d.set_style(rinch_core::dom::NodeId(cur.node_id), "min-height", "0");
                        ce.cursor = DomCursor { node_id: text_id.0, offset: paste_text.len() };
                        ce.anchor = ce.cursor;
                    }
                }
                text_changed = true;
            }

            // ── Backspace ────────────────────────────────────────────
            EditCommand::DeleteBackward => {
                if has_selection {
                    self.ce_delete_selection();
                    text_changed = true;
                } else if let Some(doc) = &self.doc {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    let cur = ce.cursor;

                    // Check if cursor is on a <br> element
                    let is_br_cursor = doc.borrow().tree.get(cur.node_id)
                        .and_then(|n| n.tag())
                        .map(|t| t == "br")
                        .unwrap_or(false);

                    if is_br_cursor {
                        // Remove the <br> and move cursor to end of prev text or start of next
                        let d = doc.borrow();
                        let prev = Self::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                        let next = Self::next_text_node(&d.tree, ce_node_id, cur.node_id);
                        let new_cursor = if let Some(prev_id) = prev {
                            // Check if prev is also a <br>
                            let prev_is_br = d.tree.get(prev_id)
                                .and_then(|n| n.tag())
                                .map(|t| t == "br")
                                .unwrap_or(false);
                            if prev_is_br {
                                DomCursor { node_id: prev_id, offset: 0 }
                            } else {
                                let len = d.tree.get(prev_id)
                                    .and_then(|n| n.text_content())
                                    .map(|s| s.len())
                                    .unwrap_or(0);
                                DomCursor { node_id: prev_id, offset: len }
                            }
                        } else if let Some(next_id) = next {
                            let next_is_br = d.tree.get(next_id)
                                .and_then(|n| n.tag())
                                .map(|t| t == "br")
                                .unwrap_or(false);
                            if next_is_br {
                                DomCursor { node_id: next_id, offset: 0 }
                            } else {
                                DomCursor { node_id: next_id, offset: 0 }
                            }
                        } else {
                            // No adjacent text — create empty placeholder text node
                            // so the CE is never completely empty
                            DomCursor { node_id: 0, offset: 0 } // placeholder, set below
                        };
                        drop(d);
                        let mut d = doc.borrow_mut();
                        d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                        if new_cursor.node_id == 0 {
                            // Create an empty text node in the CE root
                            let text_id = d.create_text("");
                            d.append_child(rinch_core::dom::NodeId(ce_node_id), text_id);
                            ce.cursor = DomCursor { node_id: text_id.0, offset: 0 };
                        } else {
                            ce.cursor = new_cursor;
                        }
                        ce.anchor = ce.cursor;
                        text_changed = true;
                    } else if Self::is_element_cursor(&doc.borrow().tree, &cur) {
                        // ── Cursor at empty block element ──
                        let d_ref = doc.borrow();
                        let cur_block = Self::find_block_and_parent(&d_ref.tree, cur.node_id, ce_node_id);
                        if let Some((cur_block_id, block_parent_id)) = cur_block {
                            let cur_tag = d_ref.tree.get(cur_block_id).and_then(|n| n.tag()).unwrap_or("").to_string();

                            // ── Backspace on empty <li>: outdent (any position) ──
                            if cur_tag == "li" && Self::is_list_tag(d_ref.tree.get(block_parent_id).and_then(|n| n.tag()).unwrap_or("")) {
                                let list_id = block_parent_id;
                                drop(d_ref);
                                let mut d = doc.borrow_mut();
                                let new_el = Self::outdent_li(&mut d, cur_block_id, list_id, ce_node_id);
                                ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else if let Some((li_id, list_id)) = Self::find_li_ancestor_for_outdent(&d_ref.tree, cur_block_id, ce_node_id) {
                                // Cursor is in a wrapper element inside an <li> — outdent the <li>
                                drop(d_ref);
                                let mut d = doc.borrow_mut();
                                let new_el = Self::outdent_li(&mut d, li_id, list_id, ce_node_id);
                                ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else if Self::is_heading(&cur_tag) || cur_tag == "blockquote" {
                                // ── Backspace on empty heading/blockquote: convert to <div> ──
                                drop(d_ref);
                                let mut d = doc.borrow_mut();
                                let new_el = Self::convert_block_tag(&mut d, cur_block_id, "div");
                                ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else {
                                // Default: remove the empty block, cursor to end of previous block
                                let siblings = &d_ref.tree.nodes[block_parent_id].children;
                                let pos = siblings.iter().position(|&c| c == cur_block_id);
                                let prev_block_id = pos.and_then(|p| if p > 0 { Some(siblings[p - 1]) } else { None });
                                let prev_cursor = prev_block_id
                                    .and_then(|pb| Self::last_text_cursor(&d_ref.tree, pb));
                                drop(d_ref);
                                let mut d = doc.borrow_mut();
                                d.remove_node(rinch_core::dom::NodeId(cur_block_id));
                                if let Some(pc) = prev_cursor {
                                    ce.cursor = pc;
                                } else if let Some(pb) = prev_block_id {
                                    ce.cursor = DomCursor { node_id: pb, offset: 0 };
                                }
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            }
                        } else if cur.node_id == ce_node_id {
                            // Cursor is on the CE root element itself — recover by
                            // finding the last cursor target in the CE.
                            let last = Self::last_text_cursor(&d_ref.tree, ce_node_id);
                            drop(d_ref);
                            if let Some(lc) = last {
                                ce.cursor = lc;
                                ce.anchor = ce.cursor;
                            }
                        }
                    } else if cur.offset > 0 {
                        // ── Delete char before cursor in current text node ──
                        let mut d = doc.borrow_mut();
                        if let Some(node) = d.tree.get(cur.node_id)
                            && let Some(current) = node.text_content().map(|s| s.to_string())
                        {
                            let off = cur.offset.min(current.len());
                            let prev_char_start = current[..off]
                                .char_indices()
                                .next_back()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                            let mut new_text = String::with_capacity(current.len());
                            new_text.push_str(&current[..prev_char_start]);
                            new_text.push_str(&current[off..]);
                            if new_text.is_empty() {
                                // Text node is now empty — find nearest cursor target.
                                // Use prev/next_text_node which traverses the full CE
                                // subtree and includes <br> elements as valid targets.
                                let prev = Self::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                                let next = Self::next_text_node(&d.tree, ce_node_id, cur.node_id);

                                if let Some(prev_id) = prev {
                                    let prev_is_br = d.tree.get(prev_id)
                                        .and_then(|n| n.tag())
                                        .map(|t| t == "br")
                                        .unwrap_or(false);
                                    d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                    if prev_is_br {
                                        ce.cursor = DomCursor { node_id: prev_id, offset: 0 };
                                    } else {
                                        let len = d.tree.get(prev_id)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.len())
                                            .unwrap_or(0);
                                        ce.cursor = DomCursor { node_id: prev_id, offset: len };
                                    }
                                } else if let Some(next_id) = next {
                                    d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                    ce.cursor = DomCursor { node_id: next_id, offset: 0 };
                                } else {
                                    // CE is completely empty — keep as empty text node
                                    d.set_text_content(rinch_core::dom::NodeId(cur.node_id), "");
                                    ce.cursor = DomCursor { node_id: cur.node_id, offset: 0 };
                                }
                            } else {
                                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                                ce.cursor = DomCursor { node_id: cur.node_id, offset: prev_char_start };
                            }
                            ce.anchor = ce.cursor;
                            text_changed = true;
                        }
                    } else {
                        // ── At start of text node — find previous text node and merge ──
                        let d = doc.borrow();
                        if let Some(prev) = Self::prev_text_node(&d.tree, ce_node_id, cur.node_id) {
                            // Check if we're crossing a block boundary
                            let cur_block = Self::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);
                            let prev_block = Self::find_block_and_parent(&d.tree, prev, ce_node_id);
                            let cross_block = cur_block.is_some()
                                && cur_block.map(|(b, _)| b) != prev_block.map(|(b, _)| b);

                            // ── Special backspace behaviors before cross-block merge ──
                            if let Some((cur_block_id, cur_block_parent)) = cur_block {
                                let cur_tag = d.tree.get(cur_block_id).and_then(|n| n.tag()).unwrap_or("").to_string();
                                let parent_tag = d.tree.get(cur_block_parent).and_then(|n| n.tag()).unwrap_or("").to_string();

                                // Backspace at start of <li>: always outdent (any position)
                                if cur_tag == "li" && Self::is_list_tag(&parent_tag) {
                                    let list_id = cur_block_parent;
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el = Self::outdent_li(&mut d, cur_block_id, list_id, ce_node_id);
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else if let Some((li_id, list_id)) = Self::find_li_ancestor_for_outdent(&d.tree, cur_block_id, ce_node_id) {
                                    // Cursor is in a wrapper element inside an <li> — outdent the <li>
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el = Self::outdent_li(&mut d, li_id, list_id, ce_node_id);
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else if Self::is_heading(&cur_tag) || cur_tag == "blockquote" {
                                    // Backspace at start of heading/blockquote: convert to <div>
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el = Self::convert_block_tag(&mut d, cur_block_id, "div");
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else {
                                    // Normal cross-block merge or same-block merge
                                    drop(d);
                                    let mut d = doc.borrow_mut();

                                    if cross_block {
                                        let (prev_block_id, _) = prev_block.unwrap();

                                        // Find merge point: end of last text in prev block
                                        let merge_cursor = Self::last_text_cursor(&d.tree, prev_block_id)
                                            .unwrap_or(DomCursor { node_id: prev, offset: 0 });

                                        // Collect current block's children
                                        let cur_children: Vec<usize> = d.tree.nodes[cur_block_id].children.clone();

                                        // Merge first text child with prev block's last text, move rest.
                                        let mut first = true;
                                        for &child_id in &cur_children {
                                            if first {
                                                first = false;
                                                let child_is_text = d.tree.get(child_id)
                                                    .and_then(|n| n.text_content())
                                                    .is_some();
                                                let merge_is_text = d.tree.get(merge_cursor.node_id)
                                                    .and_then(|n| n.text_content())
                                                    .is_some();
                                                if child_is_text && merge_is_text {
                                                    let child_text = d.tree.get(child_id)
                                                        .and_then(|n| n.text_content())
                                                        .map(|s| s.to_string())
                                                        .unwrap_or_default();
                                                    let merge_text = d.tree.get(merge_cursor.node_id)
                                                        .and_then(|n| n.text_content())
                                                        .map(|s| s.to_string())
                                                        .unwrap_or_default();
                                                    let merged = format!("{}{}", merge_text, child_text);
                                                    d.set_text_content(
                                                        rinch_core::dom::NodeId(merge_cursor.node_id),
                                                        &merged,
                                                    );
                                                    d.remove_node(rinch_core::dom::NodeId(child_id));
                                                    continue;
                                                }
                                            }
                                            // Move remaining children to prev block
                                            d.remove_node(rinch_core::dom::NodeId(child_id));
                                            d.append_child(
                                                rinch_core::dom::NodeId(prev_block_id),
                                                rinch_core::dom::NodeId(child_id),
                                            );
                                        }

                                        // Remove the now-empty current block
                                        d.remove_node(rinch_core::dom::NodeId(cur_block_id));

                                        ce.cursor = merge_cursor;
                                        ce.anchor = ce.cursor;
                                    } else {
                                        // Check if prev is a <br> — just remove it
                                        let prev_is_br = d.tree.get(prev)
                                            .and_then(|n| n.tag())
                                            .map(|t| t == "br")
                                            .unwrap_or(false);

                                        if prev_is_br {
                                            d.remove_node(rinch_core::dom::NodeId(prev));
                                            ce.cursor = cur;
                                            ce.anchor = ce.cursor;
                                        } else {
                                            // Same block or inline — merge text nodes
                                            let prev_text = d.tree.get(prev)
                                                .and_then(|n| n.text_content())
                                                .map(|s| s.to_string())
                                                .unwrap_or_default();
                                            let prev_len = prev_text.len();
                                            let cur_text = d.tree.get(cur.node_id)
                                                .and_then(|n| n.text_content())
                                                .map(|s| s.to_string())
                                                .unwrap_or_default();
                                            let merged = format!("{}{}", prev_text, cur_text);
                                            d.set_text_content(rinch_core::dom::NodeId(prev), &merged);
                                            d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                            ce.cursor = DomCursor { node_id: prev, offset: prev_len };
                                            ce.anchor = ce.cursor;
                                        }
                                    }
                                    text_changed = true;
                                }
                            } // close if let Some((cur_block_id, ...))
                            else {
                                // No block found — inline merge
                                drop(d);
                                let mut d = doc.borrow_mut();
                                let prev_is_br = d.tree.get(prev)
                                    .and_then(|n| n.tag())
                                    .map(|t| t == "br")
                                    .unwrap_or(false);
                                if prev_is_br {
                                    d.remove_node(rinch_core::dom::NodeId(prev));
                                    ce.cursor = cur;
                                    ce.anchor = ce.cursor;
                                } else {
                                    let prev_text = d.tree.get(prev)
                                        .and_then(|n| n.text_content())
                                        .map(|s| s.to_string())
                                        .unwrap_or_default();
                                    let prev_len = prev_text.len();
                                    let cur_text_str = d.tree.get(cur.node_id)
                                        .and_then(|n| n.text_content())
                                        .map(|s| s.to_string())
                                        .unwrap_or_default();
                                    let merged = format!("{}{}", prev_text, cur_text_str);
                                    d.set_text_content(rinch_core::dom::NodeId(prev), &merged);
                                    d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                    ce.cursor = DomCursor { node_id: prev, offset: prev_len };
                                    ce.anchor = ce.cursor;
                                }
                                text_changed = true;
                            }
                        } else {
                            // No previous text node — cursor is at very start of CE.
                            // Still handle heading/li/blockquote conversion.
                            let cur_block = Self::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);
                            if let Some((cur_block_id, cur_block_parent)) = cur_block {
                                let cur_tag = d.tree.get(cur_block_id).and_then(|n| n.tag()).unwrap_or("").to_string();
                                let parent_tag = d.tree.get(cur_block_parent).and_then(|n| n.tag()).unwrap_or("").to_string();

                                if cur_tag == "li" && Self::is_list_tag(&parent_tag) {
                                    // Outdent li (any position)
                                    let list_id = cur_block_parent;
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el = Self::outdent_li(&mut d, cur_block_id, list_id, ce_node_id);
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else if let Some((li_id, list_id)) = Self::find_li_ancestor_for_outdent(&d.tree, cur_block_id, ce_node_id) {
                                    // Cursor is in a wrapper element inside an <li> — outdent the <li>
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el = Self::outdent_li(&mut d, li_id, list_id, ce_node_id);
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else if Self::is_heading(&cur_tag) || cur_tag == "blockquote" {
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el = Self::convert_block_tag(&mut d, cur_block_id, "div");
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor { node_id: new_el.0, offset: 0 };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                }
                            }
                        }
                    }
                }
            }

            // ── Delete ───────────────────────────────────────────────
            EditCommand::DeleteForward => {
                if has_selection {
                    self.ce_delete_selection();
                    text_changed = true;
                } else if let Some(doc) = &self.doc {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    let cur = ce.cursor;

                    // Check if cursor is on a <br> element
                    let is_br_cursor = doc.borrow().tree.get(cur.node_id)
                        .and_then(|n| n.tag())
                        .map(|t| t == "br")
                        .unwrap_or(false);

                    if is_br_cursor {
                        // Remove the <br> and move cursor to start of next text or end of prev
                        let d = doc.borrow();
                        let next = Self::next_text_node(&d.tree, ce_node_id, cur.node_id);
                        let prev = Self::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                        let new_cursor = if let Some(next_id) = next {
                            let next_is_br = d.tree.get(next_id)
                                .and_then(|n| n.tag())
                                .map(|t| t == "br")
                                .unwrap_or(false);
                            if next_is_br {
                                DomCursor { node_id: next_id, offset: 0 }
                            } else {
                                DomCursor { node_id: next_id, offset: 0 }
                            }
                        } else if let Some(prev_id) = prev {
                            let len = d.tree.get(prev_id)
                                .and_then(|n| n.text_content())
                                .map(|s| s.len())
                                .unwrap_or(0);
                            DomCursor { node_id: prev_id, offset: len }
                        } else {
                            DomCursor { node_id: 0, offset: 0 } // placeholder, set below
                        };
                        drop(d);
                        let mut d = doc.borrow_mut();
                        d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                        if new_cursor.node_id == 0 {
                            let text_id = d.create_text("");
                            d.append_child(rinch_core::dom::NodeId(ce_node_id), text_id);
                            ce.cursor = DomCursor { node_id: text_id.0, offset: 0 };
                        } else {
                            ce.cursor = new_cursor;
                        }
                        ce.anchor = ce.cursor;
                        text_changed = true;
                    } else if Self::is_element_cursor(&doc.borrow().tree, &cur) {
                        // ── Element cursor (empty block) — remove this block,
                        //    move cursor to start of next block ──
                        let d_ref = doc.borrow();
                        let cur_block = Self::find_block_and_parent(&d_ref.tree, cur.node_id, ce_node_id);
                        if let Some((cur_block_id, block_parent_id)) = cur_block {
                            let siblings = &d_ref.tree.nodes[block_parent_id].children;
                            let pos = siblings.iter().position(|&c| c == cur_block_id);
                            let next_block_id = pos.and_then(|p| {
                                siblings.get(p + 1).copied()
                            });
                            let next_cursor = next_block_id
                                .and_then(|nb| Self::first_text_cursor(&d_ref.tree, nb));
                            drop(d_ref);
                            let mut d = doc.borrow_mut();
                            d.remove_node(rinch_core::dom::NodeId(cur_block_id));
                            if let Some(nc) = next_cursor {
                                ce.cursor = nc;
                            } else if let Some(nb) = next_block_id {
                                ce.cursor = DomCursor { node_id: nb, offset: 0 };
                            }
                            ce.anchor = ce.cursor;
                            text_changed = true;
                        }
                    } else {
                        let mut d = doc.borrow_mut();
                        if let Some(node) = d.tree.get(cur.node_id)
                            && let Some(current) = node.text_content().map(|s| s.to_string())
                        {
                            let off = cur.offset.min(current.len());
                            if off < current.len() {
                                // Delete char after cursor
                                let next_char_end = current[off..]
                                    .char_indices()
                                    .nth(1)
                                    .map(|(i, _)| off + i)
                                    .unwrap_or(current.len());
                                let mut new_text = String::with_capacity(current.len());
                                new_text.push_str(&current[..off]);
                                new_text.push_str(&current[next_char_end..]);
                                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                                text_changed = true;
                            } else {
                                // At end of text node — find next and merge
                                drop(d);
                                let d = doc.borrow();
                                if let Some(next) = Self::next_text_node(&d.tree, ce_node_id, cur.node_id) {
                                    let next_is_br = d.tree.get(next)
                                        .and_then(|n| n.tag())
                                        .map(|t| t == "br")
                                        .unwrap_or(false);
                                    let next_is_empty_block = Self::is_element_cursor(&d.tree, &DomCursor { node_id: next, offset: 0 });

                                    // Check if we're crossing a block boundary
                                    let cur_block = Self::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);
                                    let next_block = if next_is_br || next_is_empty_block { None } else {
                                        Self::find_block_and_parent(&d.tree, next, ce_node_id)
                                    };
                                    let cross_block = next_block.is_some()
                                        && cur_block.map(|(b, _)| b) != next_block.map(|(b, _)| b);

                                    drop(d);
                                    let mut d = doc.borrow_mut();

                                    if next_is_br || next_is_empty_block {
                                        // Remove the <br> (inline CE) or empty block element
                                        d.remove_node(rinch_core::dom::NodeId(next));
                                    } else if cross_block {
                                        // Cross-block delete: merge next block into current block
                                        let (next_block_id, _) = next_block.unwrap();
                                        let (cur_block_id, _) = cur_block.unwrap();

                                        // Collect next block's children
                                        let next_children: Vec<usize> = d.tree.nodes[next_block_id].children.clone();

                                        // Merge first text child of next block with current text node
                                        let mut first = true;
                                        for &child_id in &next_children {
                                            if first {
                                                first = false;
                                                let child_is_text = d.tree.get(child_id)
                                                    .and_then(|n| n.text_content())
                                                    .is_some();
                                                if child_is_text {
                                                    let child_text = d.tree.get(child_id)
                                                        .and_then(|n| n.text_content())
                                                        .map(|s| s.to_string())
                                                        .unwrap_or_default();
                                                    let merged = format!("{}{}", current, child_text);
                                                    d.set_text_content(
                                                        rinch_core::dom::NodeId(cur.node_id),
                                                        &merged,
                                                    );
                                                    d.remove_node(rinch_core::dom::NodeId(child_id));
                                                    continue;
                                                }
                                            }
                                            // Move remaining children to current block
                                            d.remove_node(rinch_core::dom::NodeId(child_id));
                                            d.append_child(
                                                rinch_core::dom::NodeId(cur_block_id),
                                                rinch_core::dom::NodeId(child_id),
                                            );
                                        }

                                        // Remove the now-empty next block
                                        d.remove_node(rinch_core::dom::NodeId(next_block_id));
                                        // Cursor stays at current position
                                    } else {
                                        // Same block or inline — merge text nodes
                                        let next_text = d.tree.get(next)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        let merged = format!("{}{}", current, next_text);
                                        d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &merged);
                                        d.remove_node(rinch_core::dom::NodeId(next));
                                    }
                                    text_changed = true;
                                }
                            }
                        }
                    }
                }
            }

            // ── Enter ────────────────────────────────────────────────
            EditCommand::InsertNewline => {
                if has_selection {
                    self.ce_delete_selection();
                }
                let ce = self.focused_contenteditable.as_mut().unwrap();
                let cur = ce.cursor;
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();

                    // Check if cursor is inside a block element
                    let block_info = Self::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);

                    if let Some((block_id, block_parent_id)) = block_info {
                        let block_tag = d.tree.get(block_id).and_then(|n| n.tag()).unwrap_or("div").to_string();

                        // If cursor is in a wrapper element inside an <li>,
                        // redirect to the <li> for Enter behavior (create new <li>, not <div>).
                        let (block_id, block_parent_id, block_tag) = if block_tag != "li" {
                            if let Some((li_id, list_id)) = Self::find_li_ancestor_for_outdent(&d.tree, block_id, ce_node_id) {
                                (li_id, list_id, "li".to_string())
                            } else {
                                (block_id, block_parent_id, block_tag)
                            }
                        } else {
                            (block_id, block_parent_id, block_tag)
                        };

                        // ── Enter in empty <li>: exit the list ──
                        if block_tag == "li" {
                            let is_empty_li = if Self::is_element_cursor(&d.tree, &cur) {
                                true
                            } else {
                                let text = d.tree.get(cur.node_id)
                                    .and_then(|n| n.text_content())
                                    .unwrap_or("");
                                text.is_empty() && d.tree.nodes[block_id].children.len() <= 1
                            };

                            if is_empty_li && Self::is_list_tag(&d.tree.get(block_parent_id).and_then(|n| n.tag()).unwrap_or("")) {
                                let list_id = block_parent_id;
                                let list_tag = d.tree.get(list_id).and_then(|n| n.tag()).unwrap_or("ul").to_string();
                                let grandparent_id = d.tree.get(list_id).and_then(|n| n.parent).unwrap_or(ce_node_id);

                                // Collect siblings after the empty <li>
                                let siblings = d.tree.nodes[list_id].children.clone();
                                let li_pos = siblings.iter().position(|&c| c == block_id).unwrap_or(0);
                                let after_siblings: Vec<usize> = siblings[li_pos + 1..].to_vec();

                                // Create a new <div> to replace the exited <li>
                                let new_div = d.create_element("div");
                                let line_h = Self::line_height_px(&d.tree, block_id);
                                d.set_style(new_div, "min-height", &format!("{:.1}px", line_h));

                                // Remove the empty <li>
                                d.remove_node(rinch_core::dom::NodeId(block_id));

                                // Insert <div> after the list in grandparent
                                let list_next_sib = {
                                    let gp_children = &d.tree.nodes[grandparent_id].children;
                                    let lpos = gp_children.iter().position(|&c| c == list_id);
                                    lpos.and_then(|p| gp_children.get(p + 1).copied())
                                };
                                if let Some(next) = list_next_sib {
                                    d.insert_before(
                                        rinch_core::dom::NodeId(grandparent_id),
                                        new_div,
                                        rinch_core::dom::NodeId(next),
                                    );
                                } else {
                                    d.append_child(rinch_core::dom::NodeId(grandparent_id), new_div);
                                }

                                // If there are siblings after, move them to a new list after <div>
                                if !after_siblings.is_empty() {
                                    let new_list = d.create_element(&list_tag);
                                    for &sib_id in &after_siblings {
                                        d.remove_node(rinch_core::dom::NodeId(sib_id));
                                        d.append_child(new_list, rinch_core::dom::NodeId(sib_id));
                                    }
                                    // Insert new list after <div>
                                    let div_next = {
                                        let gp_children = &d.tree.nodes[grandparent_id].children;
                                        let dpos = gp_children.iter().position(|&c| c == new_div.0);
                                        dpos.and_then(|p| gp_children.get(p + 1).copied())
                                    };
                                    if let Some(next) = div_next {
                                        d.insert_before(
                                            rinch_core::dom::NodeId(grandparent_id),
                                            new_list,
                                            rinch_core::dom::NodeId(next),
                                        );
                                    } else {
                                        d.append_child(rinch_core::dom::NodeId(grandparent_id), new_list);
                                    }
                                }

                                // If original list is now empty, remove it
                                if d.tree.nodes[list_id].children.is_empty() {
                                    d.remove_node(rinch_core::dom::NodeId(list_id));
                                }

                                // Cursor → start of new <div>
                                ce.cursor = DomCursor { node_id: new_div.0, offset: 0 };
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else {
                                // Non-empty li or not inside a list — split into new li
                                let new_tag = "li";

                                let cur_text = if Self::is_element_cursor(&d.tree, &cur) {
                                    String::new()
                                } else {
                                    d.tree.get(cur.node_id)
                                        .and_then(|n| n.text_content())
                                        .map(|s| s.to_string())
                                        .unwrap_or_default()
                                };
                                let off = cur.offset.min(cur_text.len());
                                let after = &cur_text[off..];

                                let new_block_id = d.create_element(new_tag);
                                if after.is_empty() {
                                    let line_h = Self::line_height_px(&d.tree, block_id);
                                    d.set_style(new_block_id, "min-height", &format!("{:.1}px", line_h));
                                } else {
                                    let new_text_id = d.create_text(after);
                                    d.append_child(new_block_id, new_text_id);
                                    if off == 0 {
                                        d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                        if d.tree.nodes[block_id].children.is_empty() {
                                            let line_h = Self::line_height_px(&d.tree, block_id);
                                            d.set_style(
                                                rinch_core::dom::NodeId(block_id),
                                                "min-height",
                                                &format!("{:.1}px", line_h),
                                            );
                                        }
                                    } else {
                                        d.set_text_content(
                                            rinch_core::dom::NodeId(cur.node_id),
                                            &cur_text[..off],
                                        );
                                    }
                                }

                                let next_sib = d.tree.nodes[block_parent_id]
                                    .children
                                    .iter()
                                    .position(|&c| c == block_id)
                                    .and_then(|pos| {
                                        d.tree.nodes[block_parent_id].children.get(pos + 1).copied()
                                    });
                                if let Some(next) = next_sib {
                                    d.insert_before(
                                        rinch_core::dom::NodeId(block_parent_id),
                                        new_block_id,
                                        rinch_core::dom::NodeId(next),
                                    );
                                } else {
                                    d.append_child(
                                        rinch_core::dom::NodeId(block_parent_id),
                                        new_block_id,
                                    );
                                }

                                if let Some(first) = Self::first_text_cursor(&d.tree, new_block_id.0) {
                                    ce.cursor = first;
                                } else {
                                    ce.cursor = DomCursor { node_id: new_block_id.0, offset: 0 };
                                }
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            }
                        } else {
                            // Non-li block: heading → div, else preserve tag
                            let new_tag = if Self::is_heading(&block_tag) { "div" } else { &block_tag };

                            let cur_text = if Self::is_element_cursor(&d.tree, &cur) {
                                String::new()
                            } else {
                                d.tree.get(cur.node_id)
                                    .and_then(|n| n.text_content())
                                    .map(|s| s.to_string())
                                    .unwrap_or_default()
                            };
                            let off = cur.offset.min(cur_text.len());
                            let after = &cur_text[off..];

                            let new_block_id = d.create_element(new_tag);
                            if after.is_empty() {
                                let line_h = Self::line_height_px(&d.tree, block_id);
                                d.set_style(new_block_id, "min-height", &format!("{:.1}px", line_h));
                            } else {
                                let new_text_id = d.create_text(after);
                                d.append_child(new_block_id, new_text_id);
                                if off == 0 {
                                    d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                    if d.tree.nodes[block_id].children.is_empty() {
                                        let line_h = Self::line_height_px(&d.tree, block_id);
                                        d.set_style(
                                            rinch_core::dom::NodeId(block_id),
                                            "min-height",
                                            &format!("{:.1}px", line_h),
                                        );
                                    }
                                } else {
                                    d.set_text_content(
                                        rinch_core::dom::NodeId(cur.node_id),
                                        &cur_text[..off],
                                    );
                                }
                            }

                            // Insert new block after current block
                            let next_sib = d.tree.nodes[block_parent_id]
                                .children
                                .iter()
                                .position(|&c| c == block_id)
                                .and_then(|pos| {
                                    d.tree.nodes[block_parent_id].children.get(pos + 1).copied()
                                });
                            if let Some(next) = next_sib {
                                d.insert_before(
                                    rinch_core::dom::NodeId(block_parent_id),
                                    new_block_id,
                                    rinch_core::dom::NodeId(next),
                                );
                            } else {
                                d.append_child(
                                    rinch_core::dom::NodeId(block_parent_id),
                                    new_block_id,
                                );
                            }

                            // Move cursor to start of new block
                            if let Some(first) = Self::first_text_cursor(&d.tree, new_block_id.0) {
                                ce.cursor = first;
                            } else {
                                ce.cursor = DomCursor { node_id: new_block_id.0, offset: 0 };
                            }
                            ce.anchor = ce.cursor;
                            text_changed = true;
                        }
                    } else {
                        // Inline-only CE — insert <br> at CE root level,
                        // splitting any inline ancestors (spans) along the way.

                        // If cursor is on a <br>, insert a new <br> before it
                        let is_br = d.tree.get(cur.node_id)
                            .and_then(|n| n.tag())
                            .map(|t| t == "br")
                            .unwrap_or(false);

                        if is_br {
                            let parent_id = d.tree.get(cur.node_id)
                                .and_then(|n| n.parent)
                                .unwrap_or(ce_node_id);
                            let new_br = d.create_element("br");
                            d.insert_before(
                                rinch_core::dom::NodeId(parent_id),
                                new_br,
                                rinch_core::dom::NodeId(cur.node_id),
                            );
                            // Cursor stays on the same <br> — visually moves down
                            ce.cursor = cur;
                            ce.anchor = ce.cursor;
                        } else {

                        let cur_text = d.tree.get(cur.node_id)
                            .and_then(|n| n.text_content())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let off = cur.offset.min(cur_text.len());

                        // Split text node at cursor
                        let after = cur_text[off..].to_string();
                        d.set_text_content(
                            rinch_core::dom::NodeId(cur.node_id),
                            &cur_text[..off],
                        );

                        let after_text_id = d.create_text(&after);

                        // Walk up from cursor.node_id to the direct child of CE root,
                        // cloning inline ancestors and moving post-cursor content.
                        let mut current_after = after_text_id;
                        let mut child = cur.node_id;
                        loop {
                            let parent_id = d.tree.get(child)
                                .and_then(|n| n.parent)
                                .unwrap_or(ce_node_id);
                            if parent_id == ce_node_id {
                                break; // child is direct child of CE root
                            }

                            // Parent is an inline element — clone it
                            let parent_tag = d.tree.get(parent_id)
                                .and_then(|n| n.tag())
                                .unwrap_or("span")
                                .to_string();
                            let clone_id = d.create_element(&parent_tag);

                            // Copy style and class attributes
                            if let Some(style) = d.tree.get(parent_id)
                                .and_then(|n| n.attributes.get("style"))
                                .map(|s| s.to_string())
                            {
                                d.set_attribute(clone_id, "style", &style);
                            }
                            if let Some(class) = d.tree.get(parent_id)
                                .and_then(|n| n.attributes.get("class"))
                                .map(|s| s.to_string())
                            {
                                d.set_attribute(clone_id, "class", &class);
                            }

                            // Move siblings after `child` from parent into clone
                            let siblings_after: Vec<usize> = {
                                let children = &d.tree.nodes[parent_id].children;
                                let pos = children.iter().position(|&c| c == child).unwrap_or(0);
                                children[pos + 1..].to_vec()
                            };
                            // First add after-content to clone
                            d.append_child(clone_id, current_after);
                            // Then move siblings
                            for &sib_id in &siblings_after {
                                d.remove_node(rinch_core::dom::NodeId(sib_id));
                                d.append_child(clone_id, rinch_core::dom::NodeId(sib_id));
                            }

                            current_after = clone_id;
                            child = parent_id;
                        }

                        // Now `child` is a direct child of CE root.
                        // Insert <br> after `child`, then `current_after` after <br>.
                        let br_id = d.create_element("br");
                        let next_sib = d.tree.nodes[ce_node_id]
                            .children
                            .iter()
                            .position(|&c| c == child)
                            .and_then(|pos| {
                                d.tree.nodes[ce_node_id].children.get(pos + 1).copied()
                            });
                        if let Some(next) = next_sib {
                            d.insert_before(
                                rinch_core::dom::NodeId(ce_node_id),
                                current_after,
                                rinch_core::dom::NodeId(next),
                            );
                            d.insert_before(
                                rinch_core::dom::NodeId(ce_node_id),
                                br_id,
                                current_after,
                            );
                        } else {
                            d.append_child(rinch_core::dom::NodeId(ce_node_id), br_id);
                            d.append_child(rinch_core::dom::NodeId(ce_node_id), current_after);
                        }

                        ce.cursor = DomCursor { node_id: after_text_id.0, offset: 0 };
                        ce.anchor = ce.cursor;

                        } // close else (non-br inline Enter)
                    }
                }
                text_changed = true;
            }

            // ── Cursor movement ──────────────────────────────────────
            EditCommand::MoveLeft | EditCommand::MoveRight
            | EditCommand::MoveWordLeft | EditCommand::MoveWordRight
            | EditCommand::MoveUp | EditCommand::MoveDown
            | EditCommand::MoveToLineStart | EditCommand::MoveToLineEnd
            | EditCommand::SelectLeft | EditCommand::SelectRight
            | EditCommand::SelectWordLeft | EditCommand::SelectWordRight
            | EditCommand::SelectUp | EditCommand::SelectDown
            | EditCommand::SelectToLineStart | EditCommand::SelectToLineEnd => {
                let is_select = matches!(cmd,
                    EditCommand::SelectLeft | EditCommand::SelectRight
                    | EditCommand::SelectWordLeft | EditCommand::SelectWordRight
                    | EditCommand::SelectUp | EditCommand::SelectDown
                    | EditCommand::SelectToLineStart | EditCommand::SelectToLineEnd
                );

                if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    let new_cursor = Self::move_dom_cursor(
                        &d.tree, ce_node_id, cursor, &cmd,
                    );
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = new_cursor;
                    if !is_select {
                        ce.anchor = new_cursor;
                    }
                }
            }

            // ── Select All ───────────────────────────────────────────
            EditCommand::SelectAll => {
                if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    if let Some(first) = Self::first_text_cursor(&d.tree, ce_node_id) {
                        ce.anchor = first;
                    }
                    if let Some(last) = Self::last_text_cursor(&d.tree, ce_node_id) {
                        ce.cursor = last;
                    }
                }
            }

            // ── Copy ─────────────────────────────────────────────────
            EditCommand::Copy => {
                #[cfg(feature = "clipboard")]
                if has_selection {
                    if let Some(doc) = &self.doc {
                        let d = doc.borrow();
                        let text = Self::extract_selection_text(
                            &d.tree, ce_node_id, anchor, cursor,
                        );
                        let _ = crate::clipboard::copy_text(&text);
                    }
                }
            }

            // ── Cut ──────────────────────────────────────────────────
            EditCommand::Cut => {
                #[cfg(feature = "clipboard")]
                if has_selection {
                    if let Some(doc) = &self.doc {
                        let d = doc.borrow();
                        let text = Self::extract_selection_text(
                            &d.tree, ce_node_id, anchor, cursor,
                        );
                        let _ = crate::clipboard::copy_text(&text);
                    }
                    self.ce_delete_selection();
                    text_changed = true;
                }
            }

            // ── Undo ──────────────────────────────────────────────────
            EditCommand::Undo => {
                let ce = self.focused_contenteditable.as_mut().unwrap();
                if let Some(entry) = ce.undo_stack.pop() {
                    let restore_cursor = entry.cursor;
                    let restore_anchor = entry.anchor;
                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        // Remove nodes that were created during the edit
                        for &node_id in &entry.created_nodes {
                            if d.tree.get(node_id).is_some() {
                                d.remove_node(rinch_core::dom::NodeId(node_id));
                            }
                        }
                        // Restore text content
                        for (node_id, old_text) in &entry.text_snapshots {
                            if d.tree.get(*node_id).is_some() {
                                d.set_text_content(
                                    rinch_core::dom::NodeId(*node_id),
                                    old_text,
                                );
                            }
                        }
                    }
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = restore_cursor;
                    ce.anchor = restore_anchor;
                    text_changed = true;
                }
            }

            // ── Tab indent ────────────────────────────────────────────
            EditCommand::Indent => {
                if let Some(doc) = &self.doc {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    let cur = ce.cursor;
                    let d_ref = doc.borrow();

                    // Find the <li> the cursor is in
                    let block_info = Self::find_block_and_parent(&d_ref.tree, cur.node_id, ce_node_id);
                    if let Some((li_id, list_id)) = block_info {
                        let li_tag = d_ref.tree.get(li_id).and_then(|n| n.tag()).unwrap_or("");
                        let list_tag = d_ref.tree.get(list_id).and_then(|n| n.tag()).unwrap_or("").to_string();

                        // Resolve the actual <li> and list — either directly or via ancestor walk
                        let resolved = if li_tag == "li" && Self::is_list_tag(&list_tag) {
                            Some((li_id, list_id, list_tag.clone()))
                        } else {
                            // Cursor may be in a wrapper <div> inside an <li>
                            Self::find_li_ancestor_for_outdent(&d_ref.tree, li_id, ce_node_id)
                                .map(|(real_li, real_list)| {
                                    let tag = d_ref.tree.get(real_list).and_then(|n| n.tag()).unwrap_or("ul").to_string();
                                    (real_li, real_list, tag)
                                })
                        };

                        if let Some((real_li_id, real_list_id, real_list_tag)) = resolved {
                            // Find previous sibling <li>
                            let siblings = d_ref.tree.nodes[real_list_id].children.clone();
                            let pos = siblings.iter().position(|&c| c == real_li_id).unwrap_or(0);

                            if pos > 0 {
                                let prev_li = siblings[pos - 1];
                                // Check if prev_li already has a nested list as last child
                                let prev_children = d_ref.tree.nodes[prev_li].children.clone();
                                let nested_list = prev_children.last().and_then(|&last| {
                                    d_ref.tree.get(last)
                                        .and_then(|n| n.tag())
                                        .and_then(|t| if Self::is_list_tag(t) { Some(last) } else { None })
                                });

                                drop(d_ref);
                                let mut d = doc.borrow_mut();

                                if let Some(existing_nested) = nested_list {
                                    // Move li into existing nested list
                                    d.remove_node(rinch_core::dom::NodeId(real_li_id));
                                    d.append_child(rinch_core::dom::NodeId(existing_nested), rinch_core::dom::NodeId(real_li_id));
                                } else {
                                    // Create new nested list, move li into it, append to prev_li
                                    let new_nested = d.create_element(&real_list_tag);
                                    d.set_attribute(new_nested, "style", "padding-left: 40px");
                                    d.remove_node(rinch_core::dom::NodeId(real_li_id));
                                    d.append_child(new_nested, rinch_core::dom::NodeId(real_li_id));
                                    d.append_child(rinch_core::dom::NodeId(prev_li), new_nested);
                                }

                                // No flex style needed: the layout engine creates anonymous
                                // block boxes for mixed inline+block content automatically.

                                // Cursor stays in the same text node
                                ce.cursor = cur;
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else {
                                return false; // Can't indent first item
                            }
                        } else {
                            return false; // Not in a list item
                        }
                    } else {
                        return false; // Not in a block
                    }
                } else {
                    return false;
                }
            }

            // ── Shift+Tab outdent ────────────────────────────────────────
            EditCommand::Outdent => {
                if let Some(doc) = &self.doc {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    let cur = ce.cursor;
                    let d_ref = doc.borrow();

                    // Find the <li> the cursor is in
                    let block_info = Self::find_block_and_parent(&d_ref.tree, cur.node_id, ce_node_id);
                    if let Some((li_id, nested_list_id)) = block_info {
                        let li_tag = d_ref.tree.get(li_id).and_then(|n| n.tag()).unwrap_or("");
                        let nested_list_tag = d_ref.tree.get(nested_list_id).and_then(|n| n.tag()).unwrap_or("").to_string();

                        // Resolve the actual <li> and its parent list
                        let resolved = if li_tag == "li" && Self::is_list_tag(&nested_list_tag) {
                            Some((li_id, nested_list_id, nested_list_tag.clone()))
                        } else {
                            Self::find_li_ancestor_for_outdent(&d_ref.tree, li_id, ce_node_id)
                                .map(|(real_li, real_list)| {
                                    let tag = d_ref.tree.get(real_list).and_then(|n| n.tag()).unwrap_or("ul").to_string();
                                    (real_li, real_list, tag)
                                })
                        };

                        if let Some((real_li_id, real_nested_list_id, real_nested_list_tag)) = resolved {
                            // Check if this list is nested inside another <li>
                            let parent_li = d_ref.tree.get(real_nested_list_id).and_then(|n| n.parent);
                            let parent_li_tag = parent_li
                                .and_then(|p| d_ref.tree.get(p))
                                .and_then(|n| n.tag())
                                .unwrap_or("");

                            if parent_li_tag == "li" {
                                let parent_li_id = parent_li.unwrap();
                                let outer_list_id = d_ref.tree.get(parent_li_id)
                                    .and_then(|n| n.parent).unwrap_or(ce_node_id);

                                // Collect siblings after current <li> in the nested list
                                let nested_siblings = d_ref.tree.nodes[real_nested_list_id].children.clone();
                                let pos = nested_siblings.iter().position(|&c| c == real_li_id).unwrap_or(0);
                                let after_siblings: Vec<usize> = nested_siblings[pos + 1..].to_vec();

                                drop(d_ref);
                                let mut d = doc.borrow_mut();

                                // Move current <li> to after parent_li in the outer list
                                d.remove_node(rinch_core::dom::NodeId(real_li_id));
                                let parent_li_next = {
                                    let siblings = &d.tree.nodes[outer_list_id].children;
                                    let ppos = siblings.iter().position(|&c| c == parent_li_id);
                                    ppos.and_then(|p| siblings.get(p + 1).copied())
                                };
                                if let Some(next) = parent_li_next {
                                    d.insert_before(
                                        rinch_core::dom::NodeId(outer_list_id),
                                        rinch_core::dom::NodeId(real_li_id),
                                        rinch_core::dom::NodeId(next),
                                    );
                                } else {
                                    d.append_child(
                                        rinch_core::dom::NodeId(outer_list_id),
                                        rinch_core::dom::NodeId(real_li_id),
                                    );
                                }

                                // If there are siblings after, create new nested list under current li
                                if !after_siblings.is_empty() {
                                    let new_nested = d.create_element(&real_nested_list_tag);
                                    for &sib_id in &after_siblings {
                                        d.remove_node(rinch_core::dom::NodeId(sib_id));
                                        d.append_child(new_nested, rinch_core::dom::NodeId(sib_id));
                                    }
                                    d.append_child(rinch_core::dom::NodeId(real_li_id), new_nested);
                                }

                                // If the original nested list is now empty, remove it
                                if d.tree.nodes[real_nested_list_id].children.is_empty() {
                                    d.remove_node(rinch_core::dom::NodeId(real_nested_list_id));
                                }

                                // Cursor stays in the same text node
                                ce.cursor = cur;
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else {
                                return false; // Already top-level
                            }
                        } else {
                            return false; // Not in a list item
                        }
                    } else {
                        return false; // Not in a block
                    }
                } else {
                    return false;
                }
            }

            // ── Unhandled commands (Escape, Redo, etc.) ───────────────
            _ => {
                return false;
            }
        }

        // Record any newly created nodes in the undo entry
        if is_mutating && !pre_edit_ids.is_empty() {
            if let Some(doc) = &self.doc {
                let d = doc.borrow();
                let post_edit_ids = Self::collect_subtree_ids(&d.tree, ce_node_id);
                let mut created = Vec::new();
                for &id in &post_edit_ids {
                    if !pre_edit_ids.contains(&id) {
                        created.push(id);
                    }
                }
                if !created.is_empty() {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    if let Some(entry) = ce.undo_stack.last_mut() {
                        entry.created_nodes = created;
                    }
                }
            }
        }

        // Update cursor/selection attributes on the DOM node
        let ce = self.focused_contenteditable.as_ref().unwrap();
        let final_cursor = ce.cursor;
        let final_anchor = ce.anchor;
        let ce_nid = ce.ce_node_id;
        self.set_contenteditable_attributes_dom(ce_nid, true, final_cursor, final_anchor);

        // Dispatch oninput event if text changed
        if text_changed {
            if let Some(doc) = &self.doc {
                let handler_id = {
                    let d = doc.borrow();
                    d.tree
                        .get(ce_nid)
                        .and_then(|n| n.attributes.get("data-oninput"))
                        .and_then(|s| s.parse::<usize>().ok())
                };
                if let Some(hid) = handler_id {
                    let dispatch_text = {
                        let d = doc.borrow();
                        Self::extract_text_content(&d.tree, ce_nid)
                    };
                    events::dispatch_input_event(
                        events::EventHandlerId(hid),
                        dispatch_text,
                    );
                }
            }
        }

        self.scene_dirty = true;
        true
    }

    // ── DOM cursor navigation helpers ───────────────────────────────────

    /// Move a `DomCursor` according to an `EditCommand` movement direction.
    fn move_dom_cursor(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
        cmd: &rinch_editable::EditCommand,
    ) -> DomCursor {
        use rinch_editable::EditCommand;
        match cmd {
            EditCommand::MoveLeft | EditCommand::SelectLeft => {
                Self::move_cursor_left(tree, ce_root, cursor, false)
            }
            EditCommand::MoveRight | EditCommand::SelectRight => {
                Self::move_cursor_right(tree, ce_root, cursor, false)
            }
            EditCommand::MoveWordLeft | EditCommand::SelectWordLeft => {
                Self::move_cursor_left(tree, ce_root, cursor, true)
            }
            EditCommand::MoveWordRight | EditCommand::SelectWordRight => {
                Self::move_cursor_right(tree, ce_root, cursor, true)
            }
            EditCommand::MoveUp | EditCommand::SelectUp => {
                Self::move_cursor_vertical(tree, ce_root, cursor, -1)
            }
            EditCommand::MoveDown | EditCommand::SelectDown => {
                Self::move_cursor_vertical(tree, ce_root, cursor, 1)
            }
            EditCommand::MoveToLineStart | EditCommand::SelectToLineStart => {
                Self::move_cursor_home(tree, ce_root, cursor)
            }
            EditCommand::MoveToLineEnd | EditCommand::SelectToLineEnd => {
                Self::move_cursor_end(tree, ce_root, cursor)
            }
            _ => cursor,
        }
    }

    /// Move cursor left by one character (or one word if `word` is true).
    fn move_cursor_left(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
        word: bool,
    ) -> DomCursor {
        // Element cursor (empty block) — move to end of previous node
        if Self::is_element_cursor(tree, &cursor) {
            if let Some(prev) = Self::prev_text_node(tree, ce_root, cursor.node_id) {
                let len = tree.get(prev).and_then(|n| n.text_content()).map(|t| t.len()).unwrap_or(0);
                return DomCursor { node_id: prev, offset: len };
            }
            return cursor;
        }

        if let Some(node) = tree.get(cursor.node_id)
            && let Some(text) = node.text_content()
        {
            if cursor.offset > 0 {
                if word {
                    let new_off = Self::find_word_start(text, cursor.offset);
                    return DomCursor { node_id: cursor.node_id, offset: new_off };
                }
                // Move back one character
                let new_off = text[..cursor.offset]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                return DomCursor { node_id: cursor.node_id, offset: new_off };
            }
        }
        // At start of node (or on a <br>) — move to previous position
        let cursor_is_br = tree.get(cursor.node_id)
            .and_then(|n| n.tag()).map(|t| t == "br").unwrap_or(false);

        let Some(prev) = Self::prev_text_node(tree, ce_root, cursor.node_id) else {
            return cursor;
        };
        let prev_is_br = tree.get(prev).and_then(|n| n.tag()).map(|t| t == "br").unwrap_or(false);

        if prev_is_br {
            if cursor_is_br || word {
                // Cursor is on a <br> or in word mode — skip the line-terminator <br>,
                // but stop at blank lines (consecutive <br>s).
                let Some(before) = Self::prev_text_node(tree, ce_root, prev) else {
                    return cursor;
                };
                let before_is_br = tree.get(before).and_then(|n| n.tag()).map(|t| t == "br").unwrap_or(false);
                if before_is_br {
                    // prev is preceded by another <br> — prev is a blank line, stop there
                    return DomCursor { node_id: prev, offset: 0 };
                }
                let len = tree.get(before).and_then(|n| n.text_content()).map(|t| t.len()).unwrap_or(0);
                if word && len > 0 {
                    let text = tree.get(before).and_then(|n| n.text_content()).unwrap();
                    return DomCursor { node_id: before, offset: Self::find_word_start(text, len) };
                }
                return DomCursor { node_id: before, offset: len };
            }
            // Cursor is on text, prev is <br>. Check what's before the <br>:
            // If another <br>, this <br> is a blank line — stop here.
            // If text, this <br> is a line terminator — skip to end of that text.
            let before = Self::prev_text_node(tree, ce_root, prev);
            let before_is_br = before.and_then(|id| tree.get(id))
                .and_then(|n| n.tag()).map(|t| t == "br").unwrap_or(false);
            if before_is_br || before.is_none() {
                // Blank line — stop at the <br>
                return DomCursor { node_id: prev, offset: 0 };
            }
            // Line terminator — skip to end of text before it
            let before_id = before.unwrap();
            let len = tree.get(before_id).and_then(|n| n.text_content()).map(|t| t.len()).unwrap_or(0);
            if word && len > 0 {
                let text = tree.get(before_id).and_then(|n| n.text_content()).unwrap();
                return DomCursor { node_id: before_id, offset: Self::find_word_start(text, len) };
            }
            return DomCursor { node_id: before_id, offset: len };
        }

        // Prev is a text node — go to its end
        let len = tree.get(prev).and_then(|n| n.text_content()).map(|t| t.len()).unwrap_or(0);
        if word && len > 0 {
            let text = tree.get(prev).and_then(|n| n.text_content()).unwrap();
            return DomCursor { node_id: prev, offset: Self::find_word_start(text, len) };
        }
        DomCursor { node_id: prev, offset: len }
    }

    /// Move cursor right by one character (or one word if `word` is true).
    fn move_cursor_right(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
        word: bool,
    ) -> DomCursor {
        // Element cursor (empty block) — move to start of next node
        if Self::is_element_cursor(tree, &cursor) {
            if let Some(next) = Self::next_text_node(tree, ce_root, cursor.node_id) {
                return DomCursor { node_id: next, offset: 0 };
            }
            return cursor;
        }

        if let Some(node) = tree.get(cursor.node_id)
            && let Some(text) = node.text_content()
        {
            if cursor.offset < text.len() {
                if word {
                    let new_off = Self::find_word_end(text, cursor.offset);
                    return DomCursor { node_id: cursor.node_id, offset: new_off };
                }
                let new_off = text[cursor.offset..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| cursor.offset + i)
                    .unwrap_or(text.len());
                return DomCursor { node_id: cursor.node_id, offset: new_off };
            }
        }
        // At end of node (or on a <br>) — move to next position
        let cursor_is_br = tree.get(cursor.node_id)
            .and_then(|n| n.tag()).map(|t| t == "br").unwrap_or(false);

        let Some(next) = Self::next_text_node(tree, ce_root, cursor.node_id) else {
            return cursor;
        };
        let next_is_br = tree.get(next).and_then(|n| n.tag()).map(|t| t == "br").unwrap_or(false);

        if next_is_br && !cursor_is_br && !word {
            // At end of text node, next is a <br> (line terminator).
            // Skip the line-terminator <br> and land on whatever follows.
            let Some(after) = Self::next_text_node(tree, ce_root, next) else {
                return cursor;
            };
            if word {
                if let Some(text) = tree.get(after).and_then(|n| n.text_content()) {
                    return DomCursor { node_id: after, offset: Self::find_word_end(text, 0) };
                }
            }
            return DomCursor { node_id: after, offset: 0 };
        }

        if next_is_br && word {
            if cursor_is_br {
                // Already on a blank line, next is another <br> — stop there
                return DomCursor { node_id: next, offset: 0 };
            }
            // Cursor on text, next is <br>. Check what follows the <br>.
            let after = Self::next_text_node(tree, ce_root, next);
            let after_is_br = after
                .and_then(|id| tree.get(id))
                .and_then(|n| n.tag())
                .map(|t| t == "br")
                .unwrap_or(false);
            if after_is_br {
                // Blank line ahead — stop at it
                return DomCursor { node_id: after.unwrap(), offset: 0 };
            }
            // Line terminator — skip and proceed to next text
            if let Some(after_id) = after {
                if let Some(text) = tree.get(after_id).and_then(|n| n.text_content()) {
                    return DomCursor { node_id: after_id, offset: Self::find_word_end(text, 0) };
                }
            }
            return cursor;
        }

        // Next is a text node (or we're on a <br> and next is whatever)
        if word {
            if let Some(text) = tree.get(next).and_then(|n| n.text_content()) {
                return DomCursor { node_id: next, offset: Self::find_word_end(text, 0) };
            }
        }
        DomCursor { node_id: next, offset: 0 }
    }

    /// Move cursor up or down by one line using Parley layout.
    fn move_cursor_vertical(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
        direction: i32, // -1 = up, +1 = down
    ) -> DomCursor {
        // <br> cursors are inline — skip the element cursor path and use IFC movement
        let is_br_cursor = tree.get(cursor.node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "br")
            .unwrap_or(false);

        // Element cursor (empty block) — jump to adjacent block
        if !is_br_cursor && Self::is_element_cursor(tree, &cursor) {
            let upward = direction < 0;
            // Walk up the ancestor chain trying each level
            let mut walk_id = cursor.node_id;
            while walk_id != ce_root {
                if let Some(result) = Self::move_to_adjacent_block(tree, ce_root, walk_id, 0.0, upward) {
                    return result;
                }
                match tree.get(walk_id).and_then(|n| n.parent) {
                    Some(pid) => walk_id = pid,
                    None => break,
                }
            }
            return cursor;
        }

        // Find the IFC containing this cursor's node
        if let Some((ifc_root_id, inline_layout)) = Self::find_ifc_for_node(tree, ce_root, cursor.node_id) {
            let ranges = &inline_layout.text_ranges;
            if let Some(ifc_offset) = rinch_dom::text_query::dom_cursor_to_ifc_offset(
                ranges, cursor.node_id, cursor.offset,
            ) {
                let layout = &inline_layout.layout;

                // Get current caret geometry
                let parley_cursor = parley::Cursor::from_byte_index(
                    layout, ifc_offset, parley::layout::Affinity::Downstream,
                );
                let geom = parley_cursor.geometry(layout, 0.0);
                let x = geom.x0 as f32;

                // Find current line and target line
                let lines: Vec<_> = layout.lines().collect();
                let mut current_line_idx = 0;
                for (i, _line) in lines.iter().enumerate() {
                    // Check if offset falls in this line by comparing with next line's text start
                    if i == lines.len() - 1 || ifc_offset < lines[i + 1].text_range().start {
                        current_line_idx = i;
                        break;
                    }
                }

                let target_line_idx = if direction < 0 {
                    if current_line_idx == 0 {
                        // Already on first line — walk up ancestor chain
                        let upward = true;
                        let mut walk_id = ifc_root_id;
                        while walk_id != ce_root {
                            if let Some(result) = Self::move_to_adjacent_block(tree, ce_root, walk_id, x, upward) {
                                return result;
                            }
                            match tree.get(walk_id).and_then(|n| n.parent) {
                                Some(pid) => walk_id = pid,
                                None => break,
                            }
                        }
                        return cursor;
                    }
                    current_line_idx - 1
                } else {
                    if current_line_idx >= lines.len() - 1 {
                        // Already on last line — walk up ancestor chain
                        let upward = false;
                        let mut walk_id = ifc_root_id;
                        while walk_id != ce_root {
                            if let Some(result) = Self::move_to_adjacent_block(tree, ce_root, walk_id, x, upward) {
                                return result;
                            }
                            match tree.get(walk_id).and_then(|n| n.parent) {
                                Some(pid) => walk_id = pid,
                                None => break,
                            }
                        }
                        return cursor;
                    }
                    current_line_idx + 1
                };

                // Get target line's y position and use from_point
                let target_line = &lines[target_line_idx];
                let target_metrics = target_line.metrics();
                let target_y = target_metrics.baseline - target_metrics.ascent + target_metrics.ascent * 0.5;

                let new_parley_cursor = parley::Cursor::from_point(layout, x, target_y);
                let new_ifc_offset = new_parley_cursor.index();

                if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                    ranges, new_ifc_offset, true,
                ) {
                    return DomCursor { node_id: nid, offset: off };
                }
            }
        }

        // Fallback for blocks without IFC (cached_text_parley)
        // Get x position from cached layout, then jump to adjacent block
        if let Some(node) = tree.get(cursor.node_id)
            && let Some(ref cached_layout) = node.cached_text_parley
        {
            let (x, _y) = rinch_dom::text_query::caret_position_for_offset_layout(
                cached_layout, cursor.offset,
            );
            // Walk up the ancestor chain from the text node's parent
            if let Some(parent_id) = node.parent {
                let upward = direction < 0;
                let mut walk_id = parent_id;
                while walk_id != ce_root {
                    if let Some(result) = Self::move_to_adjacent_block(tree, ce_root, walk_id, x, upward) {
                        return result;
                    }
                    match tree.get(walk_id).and_then(|n| n.parent) {
                        Some(pid) => walk_id = pid,
                        None => break,
                    }
                }
            }
        }

        cursor
    }

    /// Move cursor to Home (start of line).
    fn move_cursor_home(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
    ) -> DomCursor {
        // Element cursor (empty block) — already at start
        if Self::is_element_cursor(tree, &cursor) {
            return cursor;
        }
        if let Some((_ifc_root_id, inline_layout)) = Self::find_ifc_for_node(tree, ce_root, cursor.node_id) {
            let ranges = &inline_layout.text_ranges;
            if let Some(ifc_offset) = rinch_dom::text_query::dom_cursor_to_ifc_offset(
                ranges, cursor.node_id, cursor.offset,
            ) {
                let layout = &inline_layout.layout;
                let parley_cursor = parley::Cursor::from_byte_index(
                    layout, ifc_offset, parley::layout::Affinity::Downstream,
                );
                let geom = parley_cursor.geometry(layout, 0.0);
                // Use from_point with x=0 to get line start
                let line_start_cursor = parley::Cursor::from_point(layout, 0.0, geom.y0 as f32 + 1.0);
                let new_offset = line_start_cursor.index();
                if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                    ranges, new_offset, false,
                ) {
                    return DomCursor { node_id: nid, offset: off };
                }
            }
        }
        // Fallback for blocks without IFC — move to start of text node
        DomCursor { node_id: cursor.node_id, offset: 0 }
    }

    /// Move cursor to End (end of line).
    fn move_cursor_end(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
    ) -> DomCursor {
        // Element cursor (empty block) — already at end
        if Self::is_element_cursor(tree, &cursor) {
            return cursor;
        }
        if let Some((_ifc_root_id, inline_layout)) = Self::find_ifc_for_node(tree, ce_root, cursor.node_id) {
            let ranges = &inline_layout.text_ranges;
            if let Some(ifc_offset) = rinch_dom::text_query::dom_cursor_to_ifc_offset(
                ranges, cursor.node_id, cursor.offset,
            ) {
                let layout = &inline_layout.layout;
                let parley_cursor = parley::Cursor::from_byte_index(
                    layout, ifc_offset, parley::layout::Affinity::Downstream,
                );
                let geom = parley_cursor.geometry(layout, 0.0);
                // Use from_point with large x to get line end
                let line_end_cursor = parley::Cursor::from_point(layout, 1e6, geom.y0 as f32 + 1.0);
                let new_offset = line_end_cursor.index();
                if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                    ranges, new_offset, false,
                ) {
                    return DomCursor { node_id: nid, offset: off };
                }
            }
        }
        // Fallback for blocks without IFC — move to end of text node
        if let Some(node) = tree.get(cursor.node_id)
            && let Some(text) = node.text_content()
        {
            return DomCursor { node_id: cursor.node_id, offset: text.len() };
        }
        cursor
    }

    /// Find the IFC (InlineLayout) that contains the given node.
    /// Returns (ifc_root_node_id, &InlineLayout).
    fn find_ifc_for_node(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        node_id: usize,
    ) -> Option<(usize, &rinch_dom::InlineLayout)> {
        // Walk up from node_id to find nearest ancestor with text_layout
        let mut current = Some(node_id);
        while let Some(nid) = current {
            if let Some(node) = tree.get(nid) {
                if node.text_layout.is_some() {
                    return Some((nid, node.text_layout.as_ref().unwrap()));
                }
                if nid == ce_root {
                    break;
                }
                current = node.parent;
            } else {
                break;
            }
        }
        // Also check parent if node is a text node
        if let Some(node) = tree.get(node_id)
            && let Some(parent_id) = node.parent
        {
            let mut current = Some(parent_id);
            while let Some(nid) = current {
                if let Some(pnode) = tree.get(nid) {
                    if pnode.text_layout.is_some() {
                        return Some((nid, pnode.text_layout.as_ref().unwrap()));
                    }
                    if nid == ce_root {
                        break;
                    }
                    current = pnode.parent;
                } else {
                    break;
                }
            }
        }
        None
    }

    /// Move cursor to adjacent block's IFC when at the top/bottom of current IFC.
    fn move_to_adjacent_block(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        current_ifc_root: usize,
        x: f32,
        upward: bool,
    ) -> Option<DomCursor> {
        // Find the parent of the IFC root, then find the adjacent sibling
        let parent_id = tree.get(current_ifc_root)?.parent?;

        // Ensure parent is within the CE boundary — walk up from parent to check
        // ce_root is an ancestor. This prevents escaping the CE when the IFC root
        // IS the CE div (inline-only CE like Test 1).
        let mut ancestor = Some(parent_id);
        let mut within_ce = false;
        while let Some(nid) = ancestor {
            if nid == ce_root { within_ce = true; break; }
            ancestor = tree.get(nid).and_then(|n| n.parent);
        }
        if !within_ce {
            return None;
        }

        let siblings = &tree.get(parent_id)?.children;
        let pos = siblings.iter().position(|&c| c == current_ifc_root)?;

        let adj_id = if upward {
            if pos == 0 { return None; }
            siblings[pos - 1]
        } else {
            if pos + 1 >= siblings.len() { return None; }
            siblings[pos + 1]
        };

        // Find IFC in the adjacent block
        if let Some(adj_node) = tree.get(adj_id) {
            if let Some(ref il) = adj_node.text_layout {
                // Adjacent block has full IFC
                let target_y = if upward {
                    let height = il.layout.height();
                    height - 1.0
                } else {
                    1.0
                };
                let new_cursor = parley::Cursor::from_point(&il.layout, x, target_y);
                if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                    &il.text_ranges, new_cursor.index(), true,
                ) {
                    return Some(DomCursor { node_id: nid, offset: off });
                }
            } else if adj_node.children.is_empty() && adj_node.tag().map(Self::is_block_element).unwrap_or(false) {
                // Empty block element — return element-level cursor
                return Some(DomCursor { node_id: adj_id, offset: 0 });
            } else {
                // Recursively find the first/last text cursor in the subtree
                return Self::find_cursor_in_subtree(tree, adj_id, x, upward);
            }
        }
        None
    }

    /// Recursively find a text cursor position within a subtree.
    /// When `upward` is true, finds the LAST text block (deepest last child);
    /// when false, finds the FIRST text block (deepest first child).
    fn find_cursor_in_subtree(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        x: f32,
        upward: bool,
    ) -> Option<DomCursor> {
        let node = tree.get(node_id)?;

        // Check if this node itself has an IFC
        if let Some(ref il) = node.text_layout {
            let target_y = if upward { il.layout.height() - 1.0 } else { 1.0 };
            let new_cursor = parley::Cursor::from_point(&il.layout, x, target_y);
            if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                &il.text_ranges, new_cursor.index(), true,
            ) {
                return Some(DomCursor { node_id: nid, offset: off });
            }
        }

        // Check if this node has cached_text_parley (direct text node)
        if let Some(ref cached_layout) = node.cached_text_parley {
            let target_y = if upward { cached_layout.height() - 1.0 } else { 1.0 };
            let off = rinch_dom::text_query::byte_offset_from_position(
                cached_layout, x, target_y,
            );
            return Some(DomCursor { node_id: node_id, offset: off });
        }

        // Empty block element
        if node.children.is_empty() && node.tag().map(Self::is_block_element).unwrap_or(false) {
            return Some(DomCursor { node_id: node_id, offset: 0 });
        }

        // Recurse into children (last-to-first for upward, first-to-last for downward)
        let children = &node.children;
        if upward {
            for &child_id in children.iter().rev() {
                if let Some(result) = Self::find_cursor_in_subtree(tree, child_id, x, upward) {
                    return Some(result);
                }
            }
        } else {
            for &child_id in children.iter() {
                if let Some(result) = Self::find_cursor_in_subtree(tree, child_id, x, upward) {
                    return Some(result);
                }
            }
        }

        None
    }

    // ── DOM traversal helpers ────────────────────────────────────────────

    /// Find the previous text node (or `<br>`) in document order within the CE.
    fn prev_text_node(tree: &rinch_dom::NodeTree, ce_root: usize, node_id: usize) -> Option<usize> {
        let mut all_text = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_text);
        let pos = all_text.iter().position(|&id| id == node_id)?;
        if pos > 0 { Some(all_text[pos - 1]) } else { None }
    }

    /// Find the next text node (or `<br>`) in document order within the CE.
    fn next_text_node(tree: &rinch_dom::NodeTree, ce_root: usize, node_id: usize) -> Option<usize> {
        let mut all_text = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_text);
        let pos = all_text.iter().position(|&id| id == node_id)?;
        if pos + 1 < all_text.len() { Some(all_text[pos + 1]) } else { None }
    }

    /// Collect all cursor-target node IDs in document order under `root`.
    /// Cursor targets are: text nodes, `<br>` elements (inline-only CE),
    /// and empty block elements (element cursors for blank lines).
    fn collect_text_node_ids(tree: &rinch_dom::NodeTree, root: usize, out: &mut Vec<usize>) {
        let Some(node) = tree.get(root) else { return };
        if node.text_content().is_some() {
            out.push(root);
            return;
        }
        if node.tag() == Some("br") {
            out.push(root);
            return;
        }
        // Empty block element — cursor target for element cursors
        if node.children.is_empty() && node.tag().map(Self::is_block_element).unwrap_or(false) {
            out.push(root);
            return;
        }
        for &child_id in &node.children {
            Self::collect_text_node_ids(tree, child_id, out);
        }
    }

    // ── Selection helpers ────────────────────────────────────────────────

    /// Delete the current selection, updating the CE cursor.
    fn ce_delete_selection(&mut self) {
        let ce = match self.focused_contenteditable.as_mut() {
            Some(ce) => ce,
            None => return,
        };
        let ce_node_id = ce.ce_node_id;
        let cursor = ce.cursor;
        let anchor = ce.anchor;
        if cursor == anchor {
            return;
        }

        if let Some(doc) = &self.doc {
            // Determine document order (start, end)
            let (start, end) = Self::order_cursors(
                &doc.borrow().tree, ce_node_id, cursor, anchor,
            );

            if start.node_id == end.node_id {
                // Same node — simple substring removal
                let mut d = doc.borrow_mut();
                if let Some(node) = d.tree.get(start.node_id)
                    && let Some(text) = node.text_content().map(|s| s.to_string())
                {
                    let s = start.offset.min(text.len());
                    let e = end.offset.min(text.len());
                    let mut new_text = String::with_capacity(text.len() - (e - s));
                    new_text.push_str(&text[..s]);
                    new_text.push_str(&text[e..]);
                    d.set_text_content(rinch_core::dom::NodeId(start.node_id), &new_text);
                }
                let ce = self.focused_contenteditable.as_mut().unwrap();
                ce.cursor = start;
                ce.anchor = start;
                // Don't return yet — cleanup_empty_cursor_node runs at the end
            } else {
                // Cross-node deletion: truncate start, remove middle, truncate end, merge
                let mut all_text = Vec::new();
                let start_is_text;
                let end_is_text;
                let start_remaining;
                let end_remaining;
                {
                    let d = doc.borrow();
                    Self::collect_text_node_ids(&d.tree, ce_node_id, &mut all_text);
                    start_is_text = d.tree.get(start.node_id)
                        .and_then(|n| n.text_content()).is_some();
                    end_is_text = d.tree.get(end.node_id)
                        .and_then(|n| n.text_content()).is_some();
                    start_remaining = if start_is_text {
                        d.tree.get(start.node_id)
                            .and_then(|n| n.text_content())
                            .map(|t| t[..start.offset.min(t.len())].to_string())
                            .unwrap_or_default()
                    } else { String::new() };
                    end_remaining = if end_is_text {
                        d.tree.get(end.node_id)
                            .and_then(|n| n.text_content())
                            .map(|t| t[end.offset.min(t.len())..].to_string())
                            .unwrap_or_default()
                    } else { String::new() };
                }
                let start_pos = all_text.iter().position(|&id| id == start.node_id).unwrap_or(0);
                let end_pos = all_text.iter().position(|&id| id == end.node_id).unwrap_or(all_text.len());

                let merged = format!("{}{}", start_remaining, end_remaining);
                let new_cursor;

                {
                    let mut d = doc.borrow_mut();
                    if start_is_text {
                        // Start is a text node — merge into it, remove middle + end
                        d.set_text_content(rinch_core::dom::NodeId(start.node_id), &merged);
                        for &mid_id in &all_text[start_pos + 1..=end_pos] {
                            d.remove_node(rinch_core::dom::NodeId(mid_id));
                        }
                        new_cursor = DomCursor { node_id: start.node_id, offset: start.offset };
                    } else if end_is_text {
                        // Start is element cursor, end is text — remove start + middle, truncate end
                        d.set_text_content(rinch_core::dom::NodeId(end.node_id), &end_remaining);
                        for &mid_id in &all_text[start_pos..end_pos] {
                            d.remove_node(rinch_core::dom::NodeId(mid_id));
                        }
                        new_cursor = DomCursor { node_id: end.node_id, offset: 0 };
                    } else {
                        // Both are element cursors — remove everything between them
                        for &mid_id in &all_text[start_pos..=end_pos] {
                            d.remove_node(rinch_core::dom::NodeId(mid_id));
                        }
                        // Find a valid cursor: previous text node or first in CE
                        let prev_target = if start_pos > 0 {
                            let prev_id = all_text[start_pos - 1];
                            let len = d.tree.get(prev_id)
                                .and_then(|n| n.text_content())
                                .map(|t| t.len()).unwrap_or(0);
                            Some(DomCursor { node_id: prev_id, offset: len })
                        } else {
                            Self::first_text_cursor(&d.tree, ce_node_id)
                        };
                        new_cursor = prev_target.unwrap_or(
                            DomCursor { node_id: ce_node_id, offset: 0 }
                        );
                    }
                }

                let ce = self.focused_contenteditable.as_mut().unwrap();
                ce.cursor = new_cursor;
                ce.anchor = new_cursor;
            }
        }
        // Clean up empty text nodes (they break IFC navigation)
        self.cleanup_empty_cursor_node();
    }

    /// If the cursor is on an empty text node, move it to an adjacent sibling
    /// and remove the empty node.  Empty text nodes have no IfcTextRange and
    /// break IFC-based navigation (up/down).
    fn cleanup_empty_cursor_node(&mut self) {
        let ce = match self.focused_contenteditable.as_ref() {
            Some(ce) => ce,
            None => return,
        };
        let cur = ce.cursor;
        let Some(doc) = &self.doc else { return };
        let needs_cleanup = {
            let d = doc.borrow();
            d.tree.get(cur.node_id)
                .and_then(|n| n.text_content())
                .map(|t| t.is_empty())
                .unwrap_or(false)
        };
        if !needs_cleanup {
            return;
        }
        let mut sibling_cursor = None;
        {
            let d = doc.borrow();
            if let Some(pid) = d.tree.get(cur.node_id).and_then(|n| n.parent) {
                let siblings = d.tree.nodes[pid].children.clone();
                if let Some(idx) = siblings.iter().position(|&c| c == cur.node_id) {
                    // Try next sibling (e.g., a <br> on blank lines)
                    if idx + 1 < siblings.len() {
                        let next = siblings[idx + 1];
                        let next_is_br = d.tree.get(next)
                            .and_then(|n| n.tag())
                            .map(|t| t == "br")
                            .unwrap_or(false);
                        if next_is_br {
                            sibling_cursor = Some(DomCursor { node_id: next, offset: 0 });
                        } else if d.tree.get(next).and_then(|n| n.text_content()).is_some() {
                            sibling_cursor = Some(DomCursor { node_id: next, offset: 0 });
                        }
                    }
                    // Try prev sibling
                    if sibling_cursor.is_none() && idx > 0 {
                        let prev_sib = siblings[idx - 1];
                        let prev_is_br = d.tree.get(prev_sib)
                            .and_then(|n| n.tag())
                            .map(|t| t == "br")
                            .unwrap_or(false);
                        if prev_is_br {
                            sibling_cursor = Some(DomCursor { node_id: prev_sib, offset: 0 });
                        } else if let Some(tc) = d.tree.get(prev_sib).and_then(|n| n.text_content()) {
                            sibling_cursor = Some(DomCursor { node_id: prev_sib, offset: tc.len() });
                        }
                    }
                }
            }
        }
        if let Some(sc) = sibling_cursor {
            let mut d = doc.borrow_mut();
            d.remove_node(rinch_core::dom::NodeId(cur.node_id));
            let ce = self.focused_contenteditable.as_mut().unwrap();
            ce.cursor = sc;
            ce.anchor = sc;
        }
    }

    /// Order two cursors into (start, end) in document order.
    fn order_cursors(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        a: DomCursor,
        b: DomCursor,
    ) -> (DomCursor, DomCursor) {
        if a.node_id == b.node_id {
            return if a.offset <= b.offset { (a, b) } else { (b, a) };
        }
        // Walk document order to determine which comes first
        let mut all_text = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_text);
        let a_pos = all_text.iter().position(|&id| id == a.node_id);
        let b_pos = all_text.iter().position(|&id| id == b.node_id);
        match (a_pos, b_pos) {
            (Some(ap), Some(bp)) if ap <= bp => (a, b),
            _ => (b, a),
        }
    }

    /// Extract text between two cursors (for copy/cut).
    #[allow(dead_code)]
    fn extract_selection_text(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        anchor: DomCursor,
        cursor: DomCursor,
    ) -> String {
        let (start, end) = Self::order_cursors(tree, ce_root, anchor, cursor);

        if start.node_id == end.node_id {
            if let Some(node) = tree.get(start.node_id)
                && let Some(text) = node.text_content()
            {
                let s = start.offset.min(text.len());
                let e = end.offset.min(text.len());
                return text[s..e].to_string();
            }
            return String::new();
        }

        let mut all_text = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_text);
        let start_pos = all_text.iter().position(|&id| id == start.node_id).unwrap_or(0);
        let end_pos = all_text.iter().position(|&id| id == end.node_id).unwrap_or(all_text.len());

        let mut result = String::new();
        for &nid in &all_text[start_pos..=end_pos.min(all_text.len() - 1)] {
            if let Some(node) = tree.get(nid) {
                if node.tag() == Some("br") {
                    result.push('\n');
                } else if node.is_element() && node.children.is_empty()
                    && node.tag().map(Self::is_block_element).unwrap_or(false)
                {
                    // Empty block element — represents a blank line
                    result.push('\n');
                } else if let Some(text) = node.text_content() {
                    if nid == start.node_id {
                        result.push_str(&text[start.offset.min(text.len())..]);
                    } else if nid == end.node_id {
                        result.push_str(&text[..end.offset.min(text.len())]);
                    } else {
                        result.push_str(text);
                    }
                }
            }
        }
        result
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
                // Simulate a full click (press + release)
                let click_actions = self.handle_click(x, y, scale_factor);
                actions.extend(click_actions);
                // Clear selection drag — Click is a complete press+release, not a drag start
                self.ce_selecting = false;
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseDown { x, y } => {
                // Same as PlatformEvent::MouseDown{Left}: handle click and start selection tracking
                let click_actions = self.handle_click(x, y, scale_factor);
                actions.extend(click_actions);
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseUp { x: _, y: _ } => {
                self.scrollbar_drag = None;
                self.ce_selecting = false;
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseMove { x, y } => {
                self.cursor_pos = Some((x, y));

                // Handle contenteditable text selection drag
                if self.ce_selecting {
                    if let Some(ref mut ce) = self.focused_contenteditable {
                        let ce_node_id = ce.ce_node_id;
                        if let Some(doc) = &self.doc {
                            let new_cursor = {
                                let d = doc.borrow();
                                Self::compute_dom_cursor_from_click(&d.tree, ce_node_id, x, y)
                            };
                            ce.cursor = new_cursor;
                            let anchor = ce.anchor;
                            self.set_contenteditable_attributes_dom(
                                ce_node_id, true, new_cursor, anchor,
                            );
                            self.scene_dirty = true;
                            actions.push(AppAction::RequestRedraw);
                            return DebugResult::Json { data: json!(null) };
                        }
                    }
                }

                // Update hover state
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
                            let visible_height = compute_visible_content_area_height(
                                &doc_mut.tree, scroll_node_id,
                            );
                            let max_scroll = (content_height - visible_height).max(0.0);

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
                        if self.focused_contenteditable.is_some() {
                            // Route to contenteditable handler
                            let key_code = match ch {
                                '\n' => KeyCode::Enter,
                                '\t' => KeyCode::Tab,
                                '\x08' => KeyCode::Backspace,
                                _ => KeyCode::Space, // Use Space as a safe unmapped key
                            };
                            let text_str = ch.to_string();
                            self.handle_contenteditable_key(key_code, Some(&text_str), false, false, false);
                        } else {
                            // Fallback to handle_text_input for non-intercepted chars
                            self.handle_text_input(&ch.to_string());
                        }
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
                    if self.focused_contenteditable.is_some() {
                        // Route to contenteditable handler
                        let key_code = match key.as_str() {
                            "ArrowUp" => KeyCode::ArrowUp,
                            "ArrowDown" => KeyCode::ArrowDown,
                            "ArrowLeft" => KeyCode::ArrowLeft,
                            "ArrowRight" => KeyCode::ArrowRight,
                            "Home" => KeyCode::Home,
                            "End" => KeyCode::End,
                            "Enter" => KeyCode::Enter,
                            "Backspace" => KeyCode::Backspace,
                            "Delete" => KeyCode::Delete,
                            "Tab" => KeyCode::Tab,
                            "Escape" => KeyCode::Escape,
                            // Map single letter keys to their KeyCode variants
                            "a" | "A" => KeyCode::KeyA,
                            "b" | "B" => KeyCode::KeyB,
                            "c" | "C" => KeyCode::KeyC,
                            "d" | "D" => KeyCode::KeyD,
                            "e" | "E" => KeyCode::KeyE,
                            "f" | "F" => KeyCode::KeyF,
                            "g" | "G" => KeyCode::KeyG,
                            "h" | "H" => KeyCode::KeyH,
                            "i" | "I" => KeyCode::KeyI,
                            "j" | "J" => KeyCode::KeyJ,
                            "k" | "K" => KeyCode::KeyK,
                            "l" | "L" => KeyCode::KeyL,
                            "m" | "M" => KeyCode::KeyM,
                            "n" | "N" => KeyCode::KeyN,
                            "o" | "O" => KeyCode::KeyO,
                            "p" | "P" => KeyCode::KeyP,
                            "q" | "Q" => KeyCode::KeyQ,
                            "r" | "R" => KeyCode::KeyR,
                            "s" | "S" => KeyCode::KeyS,
                            "t" | "T" => KeyCode::KeyT,
                            "u" | "U" => KeyCode::KeyU,
                            "v" | "V" => KeyCode::KeyV,
                            "w" | "W" => KeyCode::KeyW,
                            "x" | "X" => KeyCode::KeyX,
                            "y" | "Y" => KeyCode::KeyY,
                            "z" | "Z" => KeyCode::KeyZ,
                            _ => KeyCode::Space, // Safe fallback for other keys
                        };
                        let text = match key.as_str() {
                            "Enter" => Some("\n".to_string()),
                            k if k.len() == 1 => Some(k.to_string()),
                            _ => None,
                        };
                        self.handle_contenteditable_key(key_code, text.as_deref(), shift, ctrl, false);
                    } else {
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

    let nx = offset_x + node.layout.x;
    let ny = offset_y + node.layout.y;
    let nw = node.layout.width;
    let nh = node.layout.height;

    if x < nx || x > nx + nw || y < ny || y > ny + nh {
        return None;
    }

    // Check children in reverse order (topmost first)
    // Children are always checked even if this element has pointer-events: none
    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;
    let children: Vec<_> = node.children.clone();
    for &child_id in children.iter().rev() {
        if let Some(hit) = hit_test_node(tree, child_id, nx - sx, ny - sy, x, y) {
            return Some(hit);
        }
    }

    // Skip this element if pointer-events: none (children still checked above)
    if matches!(
        node.computed_style.pointer_events,
        rinch_dom::computed_style::PointerEventsValue::None
    ) {
        return None;
    }

    Some(node_id)
}

/// Convert a CursorValue from computed style to a platform CursorStyle.
/// Convert a CursorValue from computed style to a platform CursorStyle.
fn cursor_value_to_style(cursor: &rinch_dom::computed_style::CursorValue) -> rinch_platform::CursorStyle {
    use rinch_dom::computed_style::CursorValue as CV;
    use rinch_platform::CursorStyle as CS;
    match cursor {
        CV::Auto => CS::Auto,
        CV::Default => CS::Default,
        CV::Pointer => CS::Pointer,
        CV::Text => CS::Text,
        CV::Move => CS::Move,
        CV::NotAllowed => CS::NotAllowed,
        CV::Crosshair => CS::Crosshair,
        CV::Grab => CS::Grab,
        CV::Grabbing => CS::Grabbing,
        CV::ColResize => CS::ColResize,
        CV::RowResize => CS::RowResize,
        CV::NResize => CS::NResize,
        CV::SResize => CS::SResize,
        CV::EResize => CS::EResize,
        CV::WResize => CS::WResize,
        CV::NeResize => CS::NeResize,
        CV::NwResize => CS::NwResize,
        CV::SeResize => CS::SeResize,
        CV::SwResize => CS::SwResize,
        CV::EwResize => CS::EwResize,
        CV::NsResize => CS::NsResize,
        CV::ZoomIn => CS::ZoomIn,
        CV::ZoomOut => CS::ZoomOut,
        CV::Wait => CS::Wait,
        CV::Progress => CS::Progress,
        CV::Help => CS::Help,
        CV::None => CS::None,
    }
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

/// The visible content area height: layout.height minus padding and border.
/// Children are positioned relative to the content box, so this is the actual
/// viewport height for scroll calculations.
pub(crate) fn compute_visible_content_area_height(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    let cs = &node.computed_style;
    let pad_top = cs.padding_top.to_px() as f64;
    let pad_bottom = cs.padding_bottom.to_px() as f64;
    let border_top = cs.border_top_width.to_px() as f64;
    let border_bottom = cs.border_bottom_width.to_px() as f64;
    (node.layout.height as f64 - pad_top - pad_bottom - border_top - border_bottom).max(0.0)
}

/// Find the nearest ancestor (or self) that is a horizontal scroll container.
pub(crate) fn find_horizontal_scroll_container(tree: &rinch_dom::NodeTree, start: usize) -> Option<usize> {
    use rinch_dom::computed_style::OverflowValue;

    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        let overflow_x = &node.computed_style.overflow_x;
        match overflow_x {
            OverflowValue::Scroll | OverflowValue::Auto => return Some(node_id),
            OverflowValue::Hidden => {
                let content_w = compute_content_width(tree, node_id);
                if content_w > node.layout.width as f64 {
                    return Some(node_id);
                }
            }
            _ => {}
        }
        current = node.parent;
    }
    // Fall back to body if content overflows
    let body = tree.get(tree.body_id)?;
    let content_w = compute_content_width(tree, tree.body_id);
    if content_w > body.layout.width as f64 {
        return Some(tree.body_id);
    }
    None
}

/// Compute the total content width of a node from its children's layout bounds.
pub(crate) fn compute_content_width(tree: &rinch_dom::NodeTree, node_id: usize) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    let mut max_right: f64 = 0.0;
    for &child_id in &node.children {
        if let Some(child) = tree.get(child_id) {
            let right = (child.layout.x + child.layout.width) as f64;
            if right > max_right {
                max_right = right;
            }
        }
    }
    max_right
}

/// The visible content area width: layout.width minus padding and border.
/// Children are positioned relative to the content box, so this is the actual
/// viewport width for scroll calculations.
pub(crate) fn compute_visible_content_area_width(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    let cs = &node.computed_style;
    let pad_left = cs.padding_left.to_px() as f64;
    let pad_right = cs.padding_right.to_px() as f64;
    let border_left = cs.border_left_width.to_px() as f64;
    let border_right = cs.border_right_width.to_px() as f64;
    (node.layout.width as f64 - pad_left - pad_right - border_left - border_right).max(0.0)
}

/// Compute the scroll target for a contenteditable element so the cursor stays visible.
///
/// Returns `Some(new_scroll_y)` if scrolling is needed, `None` if already visible
/// or the element is not a scroll container.
fn compute_ce_scroll_target(
    tree: &rinch_dom::NodeTree,
    ce_node_id: usize,
    cursor: DomCursor,
    cursor_off: usize,
) -> Option<f64> {
    use rinch_dom::computed_style::{LineHeightValue, OverflowValue};

    let node = tree.get(ce_node_id)?;

    // Only applies to scroll containers
    if !matches!(
        node.computed_style.overflow_y,
        OverflowValue::Auto | OverflowValue::Scroll
    ) {
        return None;
    }

    let current_scroll = node.scroll_offset.1;
    let visible_height = compute_visible_content_area_height(tree, ce_node_id);
    if visible_height <= 0.0 {
        return None;
    }

    let cs = &node.computed_style;

    // Helper: compute line height from a computed style
    let line_height = |cs: &rinch_dom::computed_style::ComputedStyle| -> f64 {
        let lh = match cs.line_height {
            LineHeightValue::Relative(r) => cs.font_size * r,
            LineHeightValue::Absolute(a) => a,
            LineHeightValue::Normal => cs.font_size * 1.2,
        };
        lh as f64
    };

    // Find the cursor's Y position and height relative to the content box.
    let (cursor_y, cursor_height) = if cursor.node_id == ce_node_id {
        // Cursor at the CE root itself (empty CE) — position 0
        (0.0_f64, line_height(cs))
    } else if let Some(ref inline_layout) = node.text_layout {
        // Inline CE with IFC layout — use caret position query
        let offset = cursor_off.min(inline_layout.text_content.len());
        let (_, y) = caret_position_for_offset_layout(&inline_layout.layout, offset);
        (y as f64, line_height(cs))
    } else {
        // Block CE — find which direct child of the CE root contains cursor.node_id
        let block_child_id = {
            let mut current = cursor.node_id;
            loop {
                match tree.get(current) {
                    Some(n) if n.parent == Some(ce_node_id) => break Some(current),
                    Some(n) => match n.parent {
                        Some(p) => current = p,
                        None => break None,
                    },
                    None => break None,
                }
            }
        };

        let child_id = block_child_id?;
        let child = tree.get(child_id)?;

        // Try to get line-level precision within the block using its text layout
        let child_pad_top = child.computed_style.padding_top.to_px() as f64;
        if let Some(ref text_layout) = child.text_layout {
            // Compute local offset within this child's IFC
            // Walk prior siblings to find accumulated offset
            let mut accumulated = 0usize;
            let mut first_block = true;
            for &sib_id in &node.children {
                if sib_id == child_id {
                    break;
                }
                if !first_block {
                    accumulated += 1; // \n separator
                }
                first_block = false;
                accumulated += flat_text_len_for_subtree(tree, sib_id);
            }
            if !first_block {
                accumulated += 1; // \n before this block
            }
            let local_offset = cursor_off.saturating_sub(accumulated);
            let clamped = local_offset.min(text_layout.text_content.len());
            let (_, y) = caret_position_for_offset_layout(&text_layout.layout, clamped);
            (
                child.layout.y as f64 + child_pad_top + y as f64,
                line_height(&child.computed_style),
            )
        } else {
            // Fallback: use the child block's full layout bounds
            (child.layout.y as f64, child.layout.height as f64)
        }
    };

    // Determine if scrolling is needed
    let margin = 4.0_f64;
    let new_scroll = if cursor_y < current_scroll + margin {
        // Cursor above visible area — scroll up
        (cursor_y - margin).max(0.0)
    } else if cursor_y + cursor_height > current_scroll + visible_height - margin {
        // Cursor below visible area — scroll down
        cursor_y + cursor_height - visible_height + margin
    } else {
        return None; // already visible
    };

    // Clamp to valid range
    let content_height = compute_content_height(tree, ce_node_id);
    let max_scroll = (content_height - visible_height).max(0.0);
    Some(new_scroll.clamp(0.0, max_scroll))
}

/// Compute the flat text length for a subtree (for scroll offset calculation).
/// Matches the logic in paint.rs's `get_flat_text_len`.
fn flat_text_len_for_subtree(tree: &rinch_dom::NodeTree, node_id: usize) -> usize {
    let mut len = 0usize;
    let mut ends_with_newline = false;
    flat_text_len_recursive(tree, node_id, &mut len, &mut ends_with_newline);
    if ends_with_newline && len > 0 {
        len -= 1;
    }
    len
}

fn flat_text_len_recursive(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    len: &mut usize,
    ends_with_newline: &mut bool,
) {
    let Some(node) = tree.get(node_id) else { return };
    if let Some(t) = node.text_content() {
        *len += t.len();
        *ends_with_newline = t.ends_with('\n');
    } else if node.tag() == Some("br") {
        *len += 1;
        *ends_with_newline = true;
    } else {
        let is_block = node.tag().map(|t| matches!(t,
            "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "ul" | "ol" | "li" | "blockquote" | "section" | "article"
            | "pre" | "hr" | "table" | "tr" | "header" | "footer"
            | "main" | "nav" | "aside" | "figure" | "figcaption"
            | "details" | "summary"
        )).unwrap_or(false);
        if is_block && !*ends_with_newline && *len > 0 {
            *len += 1;
            *ends_with_newline = true;
        }
        for &child_id in &node.children {
            flat_text_len_recursive(tree, child_id, len, ends_with_newline);
        }
    }
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
        let visible_height = compute_visible_content_area_height(tree, node_id);

        if content_height > visible_height {
            let scrollbar_hit_width: f32 = 16.0;
            let scrollbar_left = nx + nw - scrollbar_hit_width;

            if x >= scrollbar_left && x <= nx + nw && y >= ny && y <= ny + nh {
                return Some((node_id, content_height, visible_height));
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
