//! Platform-agnostic application logic.
//!
//! `RinchApp` holds the reactive document, scene graph, cursor state, devtools,
//! and all input-handling logic that is independent of the windowing backend.
//! The desktop shell (winit + wgpu) translates native events into
//! [`PlatformEvent`]s, feeds them to `RinchApp`, and processes the returned
//! [`AppAction`]s.

mod click_handling;
mod contenteditable;
#[cfg(feature = "debug")]
mod debug_commands;
mod event_dispatch;
pub(crate) mod hit_testing;
mod html_parser;

use contenteditable::*;
pub(crate) use hit_testing::*;
use html_parser::*;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::ce_ops::CeOps;
use rinch_core::ce::{self, ContentEditableApi};
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};
use rinch_core::events;
use rinch_dom::RinchDocument;
#[cfg(feature = "debug")]
use rinch_dom::text_query::glyph_bounds_for_offset_layout;
use rinch_dom::text_query::{byte_offset_from_position, caret_position_for_offset_layout};
use rinch_editable::{
    EditCommand, EditableDocument, EditableState, InputHandler, Key as EditKey,
    Modifiers as EditModifiers, Selection, StringDocument,
};
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

// ── Drag-and-drop state ──────────────────────────────────────────────────────

/// Pending drag: mousedown happened on a draggable element but the movement
/// threshold has not yet been crossed.
pub(crate) struct PendingDrag {
    /// The DOM node with `draggable="true"`.
    pub node_id: usize,
    /// Mouse position at mousedown (physical pixels).
    pub mousedown_pos: (f32, f32),
}

/// Active drag: movement threshold was crossed, snapshot captured.
pub(crate) struct ActiveDrag {
    /// The draggable source element.
    pub node_id: usize,
    /// Captured Vello scene of the source element's subtree (at origin).
    pub snapshot: Scene,
    /// Offset within element where the grab happened (physical px, relative to element top-left).
    pub anchor: (f32, f32),
    /// Current cursor position (physical pixels).
    pub cursor: (f32, f32),
    /// Node ID of the current drop target (if hovering over one).
    pub over_target: Option<usize>,
}

/// Movement threshold in physical pixels before a drag activates.
const DRAG_THRESHOLD: f32 = 5.0;

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
    /// Render scope (kept alive for effects).
    /// Declared before `doc` so it drops first — Scope::dispose() runs effects
    /// that may access the document via Weak<RefCell<DomDocument>>, and the doc
    /// must still be alive at that point.
    pub(crate) _render_scope: Option<Rc<RefCell<RenderScope>>>,
    /// The document (shared with RenderScope).
    pub(crate) doc: Option<Rc<RefCell<RinchDocument>>>,
    /// Vello scene (reused across frames).
    pub(crate) scene: Scene,
    /// Parley layout context for paint-time text layout.
    pub(crate) paint_layout_cx: parley::LayoutContext<peniko::Brush>,
    /// Current cursor position.
    pub(crate) cursor_pos: Option<(f32, f32)>,
    /// Active scrollbar drag state.
    pub(crate) scrollbar_drag: Option<ScrollbarDrag>,
    /// Pending drag-and-drop: mousedown on draggable, awaiting threshold.
    pub(crate) pending_drag: Option<PendingDrag>,
    /// Active drag-and-drop: threshold crossed, snapshot captured.
    pub(crate) active_dnd: Option<ActiveDrag>,
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
    /// Editable state for the focused text input (cursor, selection, undo).
    pub(crate) focused_input_state: Option<EditableState<StringDocument>>,
    /// DOM node ID of the currently focused text input.
    pub(crate) focused_input_node_id: Option<usize>,
    /// State for a focused contenteditable element.
    pub(crate) focused_contenteditable: Option<ContentEditableFocus>,
    /// Active CE API instance for the focused contentEditable element.
    pub(crate) ce_ops: Option<Rc<RefCell<CeOps>>>,
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
            _render_scope: None,
            doc: None,
            scene: Scene::new(),
            paint_layout_cx: parley::LayoutContext::new(),
            cursor_pos: None,
            scrollbar_drag: None,
            pending_drag: None,
            active_dnd: None,
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
            focused_input_state: None,
            focused_input_node_id: None,
            focused_contenteditable: None,
            ce_ops: None,
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

        // Set up network image loader if feature enabled (replaces default FileImageLoader)
        #[cfg(feature = "image-network")]
        {
            doc.borrow_mut().tree.image_loader =
                Some(std::sync::Arc::new(crate::image_loader::NetworkImageLoader));
        }

        // Register any pending fonts on the document's font context (for WASM)
        if !self.pending_fonts.is_empty() {
            let mut d = doc.borrow_mut();
            for font_data in &self.pending_fonts {
                Self::register_font_on_context(&mut d.font_cx, font_data);
            }
        }

        // Load theme + component CSS into the document's stylesheet
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

        // Run the component
        let component = self.component.take().expect("component already consumed");
        let root = {
            let mut scope_ref = scope.borrow_mut();
            component(&mut scope_ref)
        };

        // Append root to body
        doc.borrow_mut().append_child(body_id, root.node_id());

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
        let Some(doc) = &self.doc else {
            return false;
        };

        // Check if theme CSS has changed (e.g. primary color or dark mode toggled)
        #[allow(unused_assignments)]
        let mut theme_changed = false;
        #[cfg(feature = "theme")]
        {
            let current_theme = rinch_core::get_current_theme_css().unwrap_or_default();
            theme_changed = self.last_theme_css.as_deref() != Some(current_theme.as_str());

            if theme_changed {
                self.last_theme_css = Some(current_theme.clone());
                if !current_theme.is_empty() {
                    let mut d = doc.borrow_mut();
                    d.update_theme_variables(&current_theme);
                    d.recompute_all_styles_full();
                }
            }
        }

        // Short-circuit when nothing needs resolving — avoids redundant tree walks
        // when a ReRender event arrives after the drag handler already resolved.
        {
            let d = doc.borrow();
            if d.tree.dirty_nodes.is_empty() && !d.tree.styles_dirty && !theme_changed {
                return false;
            }
        }

        let frame_start = Instant::now();

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

        // Render drag-and-drop snapshot overlay
        if let Some(ref drag) = self.active_dnd {
            use peniko::kurbo::Affine;
            let tx = (drag.cursor.0 - drag.anchor.0) as f64;
            let ty = (drag.cursor.1 - drag.anchor.1) as f64;
            self.scene
                .append(&drag.snapshot, Some(Affine::translate((tx, ty))));
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

    // ── Input keyboard handling ─────────────────────────────────────────
    // Routes editing commands through EditableState<StringDocument> for
    // proper cursor tracking, selection, and undo support.

    /// Central dispatch: execute an EditCommand on the focused input's EditableState.
    fn handle_input_edit_command(&mut self, cmd: EditCommand) {
        let Some(handler_id) = self.focused_input_handler_id else {
            return;
        };
        let Some(state) = self.focused_input_state.as_mut() else {
            return;
        };

        let old_text = state.document.to_text();
        let clipboard_text = state.execute(cmd);
        let new_text = state.document.to_text();

        // Handle clipboard output (Copy/Cut)
        if let Some(text) = clipboard_text {
            #[cfg(feature = "clipboard")]
            {
                let _ = crate::clipboard::copy_text(&text);
            }
            let _ = text;
        }

        // Sync value to our buffer
        self.focused_input_value = new_text.clone();
        self.sync_input_cursor_to_dom();

        // Fire oninput if text changed
        if new_text != old_text {
            events::dispatch_input_event(events::EventHandlerId(handler_id), new_text);
        }
    }

    fn handle_text_input(&mut self, text: &str) {
        if self.focused_input_state.is_some() {
            self.handle_input_edit_command(EditCommand::InsertText(text.to_string()));
        } else if let Some(handler_id) = self.focused_input_handler_id {
            // Fallback for inputs without EditableState (shouldn't happen)
            self.focused_input_value.push_str(text);
            let value = self.focused_input_value.clone();
            self.update_focused_input_dom_value(handler_id, &value);
            events::dispatch_input_event(events::EventHandlerId(handler_id), value);
        }
    }
    fn handle_backspace(&mut self) {
        if self.focused_input_state.is_some() {
            self.handle_input_edit_command(EditCommand::DeleteBackward);
        } else if let Some(handler_id) = self.focused_input_handler_id {
            self.focused_input_value.pop();
            let value = self.focused_input_value.clone();
            self.update_focused_input_dom_value(handler_id, &value);
            events::dispatch_input_event(events::EventHandlerId(handler_id), value);
        }
    }
    fn handle_delete(&mut self) {
        self.handle_input_edit_command(EditCommand::DeleteForward);
    }
    fn handle_arrow_left(&mut self, shift: bool, ctrl: bool) {
        let cmd = match (shift, ctrl) {
            (true, true) => EditCommand::SelectWordLeft,
            (true, false) => EditCommand::SelectLeft,
            (false, true) => EditCommand::MoveWordLeft,
            (false, false) => EditCommand::MoveLeft,
        };
        self.handle_input_edit_command(cmd);
    }
    fn handle_arrow_right(&mut self, shift: bool, ctrl: bool) {
        let cmd = match (shift, ctrl) {
            (true, true) => EditCommand::SelectWordRight,
            (true, false) => EditCommand::SelectRight,
            (false, true) => EditCommand::MoveWordRight,
            (false, false) => EditCommand::MoveRight,
        };
        self.handle_input_edit_command(cmd);
    }
    fn handle_enter(&mut self) {
        // Check if a text input is focused and has an onsubmit handler
        if self.focused_input_handler_id.is_some() {
            // Use stored node_id if available, else linear scan
            let submit_handler_id = if let Some(node_id) = self.focused_input_node_id {
                if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    d.tree.nodes.get(node_id).and_then(|node| {
                        node.attributes
                            .get("data-onsubmit")
                            .and_then(|s| s.parse::<usize>().ok())
                    })
                } else {
                    None
                }
            } else if let Some(doc) = &self.doc {
                let d = doc.borrow();
                d.tree.nodes.iter().find_map(|(_, node)| {
                    node.attributes
                        .get("data-oninput")
                        .and_then(|s| s.parse::<usize>().ok())
                        .filter(|&h| Some(h) == self.focused_input_handler_id)
                        .and_then(|_| {
                            node.attributes
                                .get("data-onsubmit")
                                .and_then(|s| s.parse::<usize>().ok())
                        })
                })
            } else {
                None
            };
            if let Some(handler_id) = submit_handler_id {
                events::dispatch_event(events::EventHandlerId(handler_id));

                // After onsubmit, the handler may have changed the signal (e.g., cleared it).
                // Re-read value and rebuild EditableState to stay in sync.
                self.resync_input_state_from_dom();
            }
        }
    }
    fn handle_arrow_up(&mut self, _shift: bool) {}
    fn handle_arrow_down(&mut self, _shift: bool) {}
    fn handle_home(&mut self, shift: bool) {
        let cmd = if shift {
            EditCommand::SelectToLineStart
        } else {
            EditCommand::MoveToLineStart
        };
        self.handle_input_edit_command(cmd);
    }
    fn handle_end(&mut self, shift: bool) {
        let cmd = if shift {
            EditCommand::SelectToLineEnd
        } else {
            EditCommand::MoveToLineEnd
        };
        self.handle_input_edit_command(cmd);
    }
    fn handle_select_all(&mut self) {
        self.handle_input_edit_command(EditCommand::SelectAll);
    }
    fn handle_copy(&mut self) {
        self.handle_input_edit_command(EditCommand::Copy);
    }
    fn handle_paste(&mut self) {
        let clip_text = {
            #[cfg(feature = "clipboard")]
            {
                crate::clipboard::paste_text().unwrap_or_default()
            }
            #[cfg(not(feature = "clipboard"))]
            {
                String::new()
            }
        };
        if !clip_text.is_empty() {
            self.handle_input_edit_command(EditCommand::Paste(clip_text));
        }
    }
    fn handle_cut(&mut self) {
        self.handle_input_edit_command(EditCommand::Cut);
    }

    // ── Input cursor DOM sync ─────────────────────────────────────────

    /// Write cursor/selection attributes to the focused input's DOM node.
    fn sync_input_cursor_to_dom(&self) {
        let Some(node_id) = self.focused_input_node_id else {
            return;
        };
        let Some(state) = &self.focused_input_state else {
            return;
        };
        let Some(doc) = &self.doc else { return };

        let mut d = doc.borrow_mut();
        if let Some(node) = d.tree.nodes.get_mut(node_id) {
            node.attributes
                .insert("value".to_string(), state.document.to_text());
            node.attributes
                .insert("data-focused".to_string(), "true".to_string());
            node.attributes.insert(
                "data-cursor-pos".to_string(),
                state.selection.head.0.to_string(),
            );
            node.attributes.insert(
                "data-selection-start".to_string(),
                state.selection.anchor.0.to_string(),
            );
            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
        }
        d.tree.dirty_nodes.insert(node_id);
    }

    /// Clear focus-related attributes from the previously focused input node.
    fn clear_input_focus_attrs(&self) {
        let Some(node_id) = self.focused_input_node_id else {
            return;
        };
        let Some(doc) = &self.doc else { return };

        let mut d = doc.borrow_mut();
        if let Some(node) = d.tree.nodes.get_mut(node_id) {
            node.attributes.remove("data-focused");
            node.attributes.remove("data-cursor-pos");
            node.attributes.remove("data-selection-start");
            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
        }
        d.tree.dirty_nodes.insert(node_id);
    }

    /// Programmatically focus an input element by node ID.
    ///
    /// Looks up the node in the DOM, checks for `data-oninput`, and sets up
    /// the full input focus state (handler ID, editable state, cursor).
    /// This is the programmatic equivalent of clicking on an input element.
    pub(crate) fn try_focus_input(&mut self, node_id: usize) {
        let Some(doc) = &self.doc else { return };
        let d = doc.borrow();
        let Some(node) = d.tree.get(node_id) else { return };

        let Some(oninput_str) = node.attributes.get("data-oninput") else {
            return;
        };
        let Ok(handler_id) = oninput_str.parse::<usize>() else {
            return;
        };
        let value = node.attributes.get("value").cloned().unwrap_or_default();
        drop(d);

        // Clear previous input focus if switching to a different node
        if self.focused_input_node_id.is_some() && self.focused_input_node_id != Some(node_id) {
            self.clear_input_focus_attrs();
        }

        self.focused_input_handler_id = Some(handler_id);
        self.focused_input_value = value.clone();
        self.focused_input_node_id = Some(node_id);

        // Create EditableState with cursor at end
        let mut state = EditableState::new(StringDocument::with_text(&value));
        state.selection = Selection::cursor(value.len());
        self.focused_input_state = Some(state);
        self.sync_input_cursor_to_dom();
        self.scene_dirty = true;
    }

    /// Re-read the DOM value and rebuild EditableState after an onsubmit handler
    /// may have changed the signal (e.g., cleared the input).
    fn resync_input_state_from_dom(&mut self) {
        let Some(node_id) = self.focused_input_node_id else {
            return;
        };
        if let Some(doc) = &self.doc {
            let d = doc.borrow();
            if let Some(node) = d.tree.nodes.get(node_id) {
                let value = node.attributes.get("value").cloned().unwrap_or_default();
                self.focused_input_value = value.clone();
                let mut state = EditableState::new(StringDocument::with_text(&value));
                // Place cursor at end after resync
                state.selection = Selection::cursor(value.len());
                self.focused_input_state = Some(state);
            }
        }
        self.sync_input_cursor_to_dom();
    }

    // ── Embed API helpers ─────────────────────────────────────────────

    /// Whether a text input element is currently focused.
    pub fn has_focused_input(&self) -> bool {
        self.focused_input_handler_id.is_some()
    }

    /// Whether a contenteditable element is currently focused.
    pub fn has_focused_contenteditable(&self) -> bool {
        self.focused_contenteditable.is_some()
    }

    /// Query the computed layout rect of a `GameViewport` component by name.
    ///
    /// Walks the DOM tree looking for a node with attribute
    /// `data-viewport={name}` and returns its absolute layout rect in logical
    /// pixels.  Returns `None` if no matching viewport is found.
    pub fn viewport_rect(&self, name: &str) -> Option<(f32, f32, f32, f32)> {
        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        for (node_id, node) in &d.tree.nodes {
            if node.attributes.get("data-viewport").map(|v| v.as_str()) == Some(name) {
                // Compute absolute position by walking up the parent chain
                let mut abs_x = 0.0_f32;
                let mut abs_y = 0.0_f32;
                let mut current = Some(node_id);
                while let Some(id) = current {
                    if let Some(n) = d.tree.get(id) {
                        abs_x += n.layout.x;
                        abs_y += n.layout.y;
                        if let Some(parent_id) = n.parent
                            && let Some(parent) = d.tree.get(parent_id)
                        {
                            abs_x -= parent.scroll_offset.0 as f32;
                            abs_y -= parent.scroll_offset.1 as f32;
                        }
                        current = n.parent;
                    } else {
                        break;
                    }
                }
                return Some((abs_x, abs_y, node.layout.width, node.layout.height));
            }
        }
        None
    }

    /// Create and register a CeOps instance for the focused CE element.
    ///
    /// Called when a contentEditable element gains focus. Registers the
    /// CeOps as the active CE API so the editor bridge can access it.
    pub(crate) fn register_ce_ops(
        &mut self,
        ce_node_id: usize,
        cursor: contenteditable::DomCursor,
    ) {
        if let Some(doc) = &self.doc {
            let ce_cursor = rinch_core::ce::DomCursor {
                node_id: cursor.node_id,
                offset: cursor.offset,
            };
            let ops = Rc::new(RefCell::new(CeOps::new(doc.clone(), ce_node_id, ce_cursor)));
            ce::set_active_ce_api(ops.clone());
            self.ce_ops = Some(ops);
        }
    }

    /// Sync cursor state from ContentEditableFocus to CeOps.
    ///
    /// Called after app.rs handles input that changes cursor position.
    pub(crate) fn sync_ce_ops_cursor(&self) {
        if let Some(ce) = &self.focused_contenteditable
            && let Some(ops) = &self.ce_ops
        {
            if let Ok(mut ops) = ops.try_borrow_mut() {
                let cursor = rinch_core::ce::DomCursor {
                    node_id: ce.cursor.node_id,
                    offset: ce.cursor.offset,
                };
                let anchor = rinch_core::ce::DomCursor {
                    node_id: ce.anchor.node_id,
                    offset: ce.anchor.offset,
                };
                ops.sync_cursor(cursor, anchor);
            }
        }
    }

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
}
