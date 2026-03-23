//! Platform-agnostic application logic.
//!
//! `RinchApp` holds the reactive document, scene graph, cursor state, devtools,
//! and all input-handling logic that is independent of the windowing backend.
//! The desktop shell (winit + wgpu) translates native events into
//! [`PlatformEvent`]s, feeds them to `RinchApp`, and processes the returned
//! [`AppAction`]s.

mod click_handling;
pub(crate) mod contenteditable;
#[cfg(feature = "debug")]
mod debug_commands;
mod event_dispatch;
pub(crate) mod hit_testing;
mod html_parser;
mod text_selection;

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
use rinch_dom::paint::painter::Painter;
#[cfg(not(feature = "gpu"))]
use rinch_dom::paint::skia_painter::TinySkiaPainter;
#[cfg(feature = "gpu")]
use rinch_dom::paint::vello_painter::VelloPainter;
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
#[cfg(feature = "gpu")]
use vello::Scene;

/// Viewport rectangle as (x, y, width, height) in logical pixels.
pub type ViewportRect = (f32, f32, f32, f32);

#[cfg(feature = "desktop")]
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
    /// Captured painting of the source element's subtree (at origin).
    #[cfg(feature = "gpu")]
    pub snapshot: VelloPainter,
    /// Captured RGBA pixels of the source element's subtree (software backend).
    #[cfg(not(feature = "gpu"))]
    pub snapshot_pixels: Vec<u8>,
    /// Width of the snapshot pixmap in physical pixels.
    #[cfg(not(feature = "gpu"))]
    pub snapshot_width: u32,
    /// Height of the snapshot pixmap in physical pixels.
    #[cfg(not(feature = "gpu"))]
    pub snapshot_height: u32,
    /// Offset within element where the grab happened (physical px, relative to element top-left).
    pub anchor: (f32, f32),
    /// Current cursor position (physical pixels).
    pub cursor: (f32, f32),
    /// Node ID of the current drop target (if hovering over one).
    pub over_target: Option<usize>,
}

/// Movement threshold in physical pixels before a drag activates.
const DRAG_THRESHOLD: f32 = 5.0;

// ── Read-only text selection ────────────────────────────────────────────────

/// State for read-only text selection (non-contenteditable).
pub(crate) struct TextSelection {
    /// The IFC root node where the selection lives (the block element containing the text).
    pub(crate) ifc_node_id: usize,
    /// Selection anchor (byte offset in IFC flat text, set on mousedown).
    pub(crate) anchor_offset: usize,
    /// Selection focus/cursor (byte offset, updated on mousemove).
    pub(crate) focus_offset: usize,
}

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
    /// GPU painter (reused across frames). Wraps vello::Scene.
    #[cfg(feature = "gpu")]
    pub(crate) painter: VelloPainter,
    /// Software painter (reused across frames). Uses tiny-skia for CPU rendering.
    #[cfg(not(feature = "gpu"))]
    pub(crate) skia_painter: Option<TinySkiaPainter>,
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
    /// Whether we have a previous frame's pixels for dirty region caching.
    #[cfg(not(feature = "gpu"))]
    pub(crate) has_previous_frame: bool,
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
    /// The render surface ID currently under the mouse cursor (for enter/leave events).
    pub(crate) hovered_surface: Option<usize>,
    /// State for read-only text selection (non-contenteditable).
    pub(crate) text_selection: Option<TextSelection>,
    /// Whether we're currently mouse-drag selecting text (read-only, non-CE).
    pub(crate) text_selecting: bool,
    /// Node ID of the current file-drop hover target (for enter/leave during OS drag).
    pub(crate) file_hover_target: Option<usize>,
    /// Inspect highlight rectangle (absolute x, y, w, h in logical pixels).
    /// Set by the runtime when inspect mode is active and a node is hovered.
    pub(crate) inspect_highlight: Option<(f32, f32, f32, f32)>,
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
            #[cfg(feature = "gpu")]
            painter: VelloPainter::new(),
            #[cfg(not(feature = "gpu"))]
            skia_painter: None,
            paint_layout_cx: parley::LayoutContext::new(),
            cursor_pos: None,
            scrollbar_drag: None,
            pending_drag: None,
            active_dnd: None,
            last_theme_css: None,
            last_click_time: Instant::now(),
            last_click_pos: (0.0, 0.0),
            click_count: 0,
            hit_test_font_cx: parley::FontContext::new(),
            window_props: None,
            modifiers: Modifiers::default(),
            scene_dirty: true,
            #[cfg(not(feature = "gpu"))]
            has_previous_frame: false,
            focused_input_handler_id: None,
            focused_input_value: String::new(),
            focused_input_state: None,
            focused_input_node_id: None,
            focused_contenteditable: None,
            ce_ops: None,
            ce_selecting: false,
            ce_scroll_pending: Cell::new(false),
            hovered_surface: None,
            text_selection: None,
            text_selecting: false,
            file_hover_target: None,
            inspect_highlight: None,
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
        self.doc = Some(doc.clone());
        self._render_scope = Some(scope);

        // Register CE API factory so contenteditable elements can be
        // accessed via NodeHandle::with_ce_api() before they gain focus.
        let weak_doc = Rc::downgrade(&doc);
        rinch_core::set_ce_api_factory(move |node_id: usize| {
            let doc = weak_doc.upgrade()?;
            // Verify this node has contenteditable="true"
            let is_ce = {
                let d = doc.borrow();
                d.tree
                    .get(node_id)
                    .and_then(|n| n.attributes.get("contenteditable"))
                    .map(|v| v == "true" || v.is_empty())
                    .unwrap_or(false)
            };
            if !is_ce {
                return None;
            }
            // Create CeOps with cursor at first text node (or element root)
            let cursor = {
                let d = doc.borrow();
                Self::first_text_cursor(&d.tree, node_id)
                    .map(|c| rinch_core::ce::DomCursor {
                        node_id: c.node_id,
                        offset: c.offset,
                    })
                    .unwrap_or(rinch_core::ce::DomCursor { node_id, offset: 0 })
            };
            let ops = Rc::new(RefCell::new(crate::ce_ops::CeOps::new(
                doc.clone(),
                node_id,
                cursor,
            )));
            Some(ops as Rc<RefCell<dyn rinch_core::ContentEditableApi>>)
        });
    }

    // ── Layout / repaint ─────────────────────────────────────────────────

    /// Re-resolve layout after signal changes. Returns `true` if a redraw
    /// is needed.
    pub fn resolve_and_repaint(&mut self, viewport_width: f32, viewport_height: f32) -> bool {
        let Some(doc) = self.doc.clone() else {
            return false;
        };
        let doc = &doc;

        // Check if theme CSS has changed (e.g. primary color or dark mode toggled)
        #[allow(unused_assignments, unused_mut)]
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
                // Force full repaint — recompute_all_styles_full() updates computed
                // styles but doesn't populate paint_dirty_nodes, so the software
                // renderer's dirty region optimization would skip most of the screen.
                #[cfg(not(feature = "gpu"))]
                {
                    self.has_previous_frame = false;
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

        // Pre-layout: update CE virtualization using previous frame's layout
        // positions so newly materialized blocks get measured in this pass.
        if let Some(ce_ops) = self.ce_ops.clone() {
            let mut ops = ce_ops.borrow_mut();
            if let Some(vw) = &mut ops.virtual_window {
                if vw.is_active() {
                    // Build protected list: cursor block + pending nav blocks
                    let mut protected: Vec<usize> = vw.pending_nav_blocks.clone();
                    if let Some(ce) = &self.focused_contenteditable {
                        let d = doc.borrow();
                        if let Some((block_id, _)) =
                            Self::find_block_and_parent(&d.tree, ce.cursor.node_id, ce.ce_node_id)
                        {
                            if !protected.contains(&block_id) {
                                protected.push(block_id);
                            }
                        }
                    }
                    let mut d = doc.borrow_mut();
                    vw.pre_layout_update(&mut d, &protected);
                    vw.pending_nav_blocks.clear();
                }
            }
        }

        // Resolve layout
        {
            let mut d = doc.borrow_mut();
            let _ = d.take_dirty_nodes();
            d.resolve_layout(viewport_width, viewport_height);

            // Check if the inset fast path requested a full repaint
            // (absolute element moved — dirty region caching can't track the
            // old position reliably, so force full scene rebuild).
            if d.tree.full_repaint_needed {
                d.tree.full_repaint_needed = false;
                self.scene_dirty = true;
                #[cfg(not(feature = "gpu"))]
                {
                    self.has_previous_frame = false;
                }
            }
        }

        // Apply deferred scroll-into-view now that layout is fresh
        self.apply_ce_scroll_into_view();
        self.apply_scroll_into_view();

        // Post-layout: cache heights, then verify materialized range with
        // fresh positions. If the range changed (big scroll jump), re-layout.
        if let Some(ce_ops) = self.ce_ops.clone() {
            let mut ops = ce_ops.borrow_mut();
            if let Some(vw) = &mut ops.virtual_window {
                if vw.is_active() {
                    let mut protected = Vec::new();
                    if let Some(ce) = &self.focused_contenteditable {
                        let d = doc.borrow();
                        if let Some((block_id, _)) =
                            Self::find_block_and_parent(&d.tree, ce.cursor.node_id, ce.ce_node_id)
                        {
                            protected.push(block_id);
                        }
                    }
                    let mut d = doc.borrow_mut();
                    vw.post_layout_cache(&mut d);
                    // Re-check with fresh positions — if pre_layout_update used
                    // stale positions, the range may be wrong after layout.
                    if vw.pre_layout_update(&mut d, &protected) {
                        let _ = d.take_dirty_nodes();
                        d.resolve_layout(viewport_width, viewport_height);
                        vw.post_layout_cache(&mut d);
                    }
                }
            }
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

    /// Apply deferred scroll-into-view requests.
    ///
    /// Must be called AFTER `resolve_layout` so node positions are valid.
    /// Walks ancestors to find the nearest scroll container and adjusts its
    /// scroll offset to make the target element visible.
    fn apply_scroll_into_view(&mut self) {
        let Some(doc) = &self.doc else { return };
        let requests = doc.borrow_mut().drain_scroll_into_view_requests();
        if requests.is_empty() {
            return;
        }

        for target_nid in requests {
            let mut d = doc.borrow_mut();

            // Walk ancestors to find nearest scroll container
            let target_id = target_nid.0;
            let scroll_container = {
                let mut current = d.tree.nodes.get(target_id).and_then(|n| n.parent);
                let mut found = None;
                while let Some(ancestor_id) = current {
                    if let Some(ancestor) = d.tree.nodes.get(ancestor_id) {
                        use rinch_dom::computed_style::OverflowValue;
                        if matches!(
                            ancestor.computed_style.overflow_y,
                            OverflowValue::Auto | OverflowValue::Scroll
                        ) {
                            found = Some(ancestor_id);
                            break;
                        }
                        current = ancestor.parent;
                    } else {
                        break;
                    }
                }
                found
            };

            let Some(container_id) = scroll_container else {
                continue;
            };

            // Compute element's Y position relative to the scroll container
            let mut rel_y = 0.0_f32;
            let mut current = target_id;
            while current != container_id {
                if let Some(node) = d.tree.nodes.get(current) {
                    rel_y += node.layout.y;
                    if let Some(parent_id) = node.parent {
                        if parent_id != container_id {
                            if let Some(parent) = d.tree.nodes.get(parent_id) {
                                rel_y -= parent.scroll_offset.1 as f32;
                            }
                        }
                    }
                    current = node.parent.unwrap_or(container_id);
                } else {
                    break;
                }
            }

            let target_height = d
                .tree
                .nodes
                .get(target_id)
                .map(|n| n.layout.height)
                .unwrap_or(0.0);

            let container_nid = rinch_core::dom::NodeId(container_id);
            let visible_height = d.client_height(container_nid);
            let current_scroll = d.scroll_top(container_nid);
            let content_height = d.scroll_height(container_nid);
            let max_scroll = (content_height - visible_height).max(0.0);

            // Determine if element is outside the visible area
            let elem_top = rel_y as f64;
            let elem_bottom = elem_top + target_height as f64;

            let new_scroll = if elem_top < current_scroll {
                // Element is above visible area — scroll up
                elem_top
            } else if elem_bottom > current_scroll + visible_height {
                // Element is below visible area — scroll down
                elem_bottom - visible_height
            } else {
                continue; // already visible
            };

            let clamped = new_scroll.clamp(0.0, max_scroll);
            if let Some(node) = d.tree.nodes.get_mut(container_id) {
                node.scroll_offset.1 = clamped;
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
            }
            d.tree.dirty_nodes.insert(container_id);
            self.scene_dirty = true;
        }
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
    /// The scene is painted via the `Painter` trait and a reference to the
    /// underlying `vello::Scene` is returned for the GPU renderer.
    #[cfg(feature = "gpu")]
    pub fn build_scene(&mut self, scale: f64, size: (u32, u32)) -> &Scene {
        if !self.scene_dirty {
            return self.painter.scene();
        }

        self.painter.reset();
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            let d = &mut *d;
            rinch_dom::paint::paint_document(
                &d.tree,
                &mut self.painter,
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
            self.painter
                .scene_mut()
                .append(drag.snapshot.scene(), Some(Affine::translate((tx, ty))));
        }

        // Paint inspect mode highlight overlay
        if let Some((x, y, w, h)) = self.inspect_highlight {
            Self::paint_inspect_overlay(&mut self.painter, scale, x, y, w, h);
        }

        self.scene_dirty = false;
        self.painter.scene()
    }

    /// Build pixels via TinySkiaPainter for software rendering.
    ///
    /// Surface pixels are painted inline during paint traversal (like `<img>`),
    /// set via `rinch_dom::paint::set_surface_pixels()` before calling this.
    ///
    /// Uses dirty region caching: when only a few nodes changed, clears and
    /// repaints only the affected rectangular area, preserving unchanged pixels.
    ///
    /// Returns (pixels, width, height) in RGBA8 format. The painter is lazily
    /// created on first call and resized as needed.
    #[cfg(not(feature = "gpu"))]
    pub fn build_pixels(
        &mut self,
        scale: f64,
        size: (u32, u32),
        transparent: bool,
    ) -> (&[u8], u32, u32) {
        let w = (size.0 as f64 * scale).round() as u32;
        let h = (size.1 as f64 * scale).round() as u32;

        // Lazily create or resize the painter
        let resized = match &mut self.skia_painter {
            None => {
                self.skia_painter = Some(TinySkiaPainter::new(w, h));
                self.has_previous_frame = false;
                true
            }
            Some(painter) => {
                let r = painter.width() != w || painter.height() != h;
                if r {
                    painter.resize(w, h);
                    self.has_previous_frame = false;
                }
                r
            }
        };
        let painter = self.skia_painter.as_mut().unwrap();

        if self.scene_dirty {
            let paint_start = Instant::now();

            // Mark surface DOM nodes as paint-dirty when new pixels arrive,
            // so dirty region caching correctly includes surface rects.
            if let Some(doc) = &self.doc {
                let mut d = doc.borrow_mut();
                let dirty_surface_nodes: Vec<_> = d
                    .tree
                    .nodes
                    .iter()
                    .filter_map(|(node_id, node)| {
                        let id_str = node.attributes.get("data-render-surface")?;
                        let sid = id_str.parse::<usize>().ok()?;
                        if crate::render_surface::is_surface_dirty_by_id(sid) {
                            Some(node_id)
                        } else {
                            None
                        }
                    })
                    .collect();
                d.tree.paint_dirty_nodes.extend(dirty_surface_nodes);
            }

            // Compute dirty region before clearing paint_dirty_nodes
            let dirty_region = if self.has_previous_frame && !resized {
                self.doc.as_ref().and_then(|doc| {
                    let d = doc.borrow();
                    rinch_dom::paint::compute_dirty_region(&d.tree, scale, w as f64, h as f64)
                })
            } else {
                None // Full repaint: first frame or resize
            };

            // Drain paint_dirty_nodes now that we've computed the region
            if let Some(doc) = &self.doc {
                doc.borrow_mut().tree.paint_dirty_nodes.clear();
            }

            // Check if dirty region is small enough to benefit from caching
            // (less than 50% of viewport area)
            // Disable dirty region when inspect highlight or drag overlay is active
            // (overlays are painted after document and dirty tracking doesn't cover them).
            let use_dirty_region = self.inspect_highlight.is_none()
                && self.active_dnd.is_none()
                && dirty_region.is_some_and(|r| {
                    let region_area = r.width() * r.height();
                    let viewport_area = w as f64 * h as f64;
                    region_area < viewport_area * 0.5 && region_area > 0.0
                });

            if use_dirty_region {
                let region = dirty_region.unwrap();

                // Clear only the dirty region
                let rx = region.x0 as u32;
                let ry = region.y0 as u32;
                let rw = region.width().ceil() as u32;
                let rh = region.height().ceil() as u32;
                if transparent {
                    painter.clear_rect_transparent(rx, ry, rw, rh);
                } else {
                    painter.clear_rect_white(rx, ry, rw, rh);
                }

                // Set dirty region so paint_node can skip subtrees outside it
                use peniko::kurbo::Rect;
                rinch_dom::paint::set_dirty_region(Some(Rect::new(
                    region.x0, region.y0, region.x1, region.y1,
                )));

                // Push a clip rect to prevent drawing outside the dirty region
                let clip_shape = rinch_dom::paint::painter::PaintShape::Rect(Rect::new(
                    region.x0, region.y0, region.x1, region.y1,
                ));
                painter.push_clip(
                    peniko::Fill::NonZero,
                    peniko::kurbo::Affine::IDENTITY,
                    &clip_shape,
                );

                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
                    let d = &mut *d;
                    rinch_dom::paint::paint_document(
                        &d.tree,
                        painter,
                        scale,
                        (size.0 as f32, size.1 as f32),
                        &mut d.font_cx,
                        &mut d.layout_cx,
                    );
                }

                painter.pop_layer();
                rinch_dom::paint::set_dirty_region(None);
            } else {
                // Full repaint
                painter.reset();
                if transparent {
                    painter.fill_transparent();
                } else {
                    painter.fill_white();
                }

                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
                    let d = &mut *d;
                    rinch_dom::paint::paint_document(
                        &d.tree,
                        painter,
                        scale,
                        (size.0 as f32, size.1 as f32),
                        &mut d.font_cx,
                        &mut d.layout_cx,
                    );
                }
            }

            // Paint drag-and-drop snapshot overlay
            if let Some(ref drag) = self.active_dnd {
                let dx = (drag.cursor.0 - drag.anchor.0) as i32;
                let dy = (drag.cursor.1 - drag.anchor.1) as i32;
                Self::blit_drag_overlay(
                    painter.pixels_mut(),
                    w,
                    h,
                    &drag.snapshot_pixels,
                    drag.snapshot_width,
                    drag.snapshot_height,
                    dx,
                    dy,
                );
            }

            // Paint inspect mode highlight overlay (after all document painting)
            if let Some((x, y, w_r, h_r)) = self.inspect_highlight {
                Self::paint_inspect_overlay(painter, scale, x, y, w_r, h_r);
            }

            // Log paint timing if RINCH_PERF is set
            if std::env::var("RINCH_PERF").is_ok() {
                let elapsed = paint_start.elapsed();
                if use_dirty_region {
                    let region = dirty_region.unwrap();
                    let pct = (region.width() * region.height()) / (w as f64 * h as f64) * 100.0;
                    eprintln!(
                        "[PERF] paint (dirty region {:.0}x{:.0}, {:.1}%): {:.2}ms",
                        region.width(),
                        region.height(),
                        pct,
                        elapsed.as_secs_f64() * 1000.0,
                    );
                } else {
                    eprintln!(
                        "[PERF] paint (full): {:.2}ms",
                        elapsed.as_secs_f64() * 1000.0,
                    );
                }
            }

            self.scene_dirty = false;
            self.has_previous_frame = true;
        }

        (self.skia_painter.as_ref().unwrap().pixels(), w, h)
    }

    /// Blit premultiplied RGBA source pixels onto a destination buffer with
    /// alpha compositing (source-over). `dx`/`dy` can be negative for partially
    /// off-screen overlays.
    #[cfg(not(feature = "gpu"))]
    #[allow(clippy::too_many_arguments)]
    fn blit_drag_overlay(
        dst: &mut [u8],
        dst_w: u32,
        dst_h: u32,
        src: &[u8],
        src_w: u32,
        src_h: u32,
        dx: i32,
        dy: i32,
    ) {
        let dst_w = dst_w as i32;
        let dst_h = dst_h as i32;
        let src_w_i = src_w as i32;
        let src_h_i = src_h as i32;

        // Compute visible rectangle in source coordinates
        let sx0 = 0i32.max(-dx);
        let sy0 = 0i32.max(-dy);
        let sx1 = src_w_i.min(dst_w - dx);
        let sy1 = src_h_i.min(dst_h - dy);
        if sx0 >= sx1 || sy0 >= sy1 {
            return;
        }

        for sy in sy0..sy1 {
            let dest_y = (sy + dy) as u32;
            let src_row = (sy as u32 * src_w * 4) as usize;
            let dst_row = (dest_y * dst_w as u32 * 4) as usize;
            for sx in sx0..sx1 {
                let si = src_row + (sx as usize) * 4;
                let di = dst_row + ((sx + dx) as usize) * 4;
                let sa = src[si + 3] as u32;
                if sa == 0 {
                    continue;
                }
                if sa == 255 {
                    dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
                } else {
                    // Source-over compositing (premultiplied alpha)
                    let inv_a = 255 - sa;
                    dst[di] = (src[si] as u32 + (dst[di] as u32 * inv_a + 127) / 255) as u8;
                    dst[di + 1] =
                        (src[si + 1] as u32 + (dst[di + 1] as u32 * inv_a + 127) / 255) as u8;
                    dst[di + 2] =
                        (src[si + 2] as u32 + (dst[di + 2] as u32 * inv_a + 127) / 255) as u8;
                    dst[di + 3] = (sa + (dst[di + 3] as u32 * inv_a + 127) / 255) as u8;
                }
            }
        }
    }

    /// Mark the scene as needing a repaint on the next frame.
    pub fn mark_scene_dirty(&mut self) {
        self.scene_dirty = true;
    }

    /// Check if there are dirty nodes that need repaint.
    pub fn has_dirty_nodes(&self) -> bool {
        self.doc
            .as_ref()
            .map(|doc| {
                let d = doc.borrow();
                !d.tree.dirty_nodes.is_empty() || d.tree.styles_dirty
            })
            .unwrap_or(false)
    }

    /// Paint a semi-transparent inspect highlight overlay on the given painter.
    fn paint_inspect_overlay(
        painter: &mut dyn Painter,
        scale: f64,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        use peniko::color::AlphaColor;
        use peniko::kurbo::{Affine, Rect, Stroke};
        use rinch_dom::paint::painter::PaintShape;

        let s = scale;
        let rect = Rect::new(
            (x as f64) * s,
            (y as f64) * s,
            ((x + w) as f64) * s,
            ((y + h) as f64) * s,
        );
        let shape = PaintShape::Rect(rect);

        // Semi-transparent blue fill
        let fill_color = AlphaColor::new([66.0 / 255.0, 133.0 / 255.0, 244.0 / 255.0, 0.3]);
        painter.fill_color(peniko::Fill::NonZero, Affine::IDENTITY, fill_color, &shape);

        // 1px blue border
        let border_color = AlphaColor::new([66.0 / 255.0, 133.0 / 255.0, 244.0 / 255.0, 0.8]);
        let stroke = Stroke::new(1.0 * s);
        painter.stroke_color(&stroke, Affine::IDENTITY, border_color, &shape);
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
        // Check for read-only text selection first
        if self
            .text_selection
            .as_ref()
            .is_some_and(|s| s.anchor_offset != s.focus_offset)
        {
            self.copy_text_selection();
            return;
        }
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

    // ── Tab navigation ─────────────────────────────────────────────────

    /// Handle Tab/Shift+Tab key to navigate between focusable elements.
    fn handle_tab(&mut self, shift: bool) {
        let focusable = self.collect_focusable_nodes();
        if focusable.is_empty() {
            return;
        }

        // Find current focused element
        let current = self
            .focused_input_node_id
            .or(self.focused_contenteditable.as_ref().map(|c| c.ce_node_id));

        let current_idx = current.and_then(|id| focusable.iter().position(|&fid| fid == id));

        let target_idx = match (current_idx, shift) {
            (Some(idx), false) => (idx + 1) % focusable.len(),
            (Some(idx), true) => idx.checked_sub(1).unwrap_or(focusable.len() - 1),
            (None, false) => 0,
            (None, true) => focusable.len() - 1,
        };

        self.focus_element(focusable[target_idx]);
    }

    /// Collect all focusable node IDs in DOM pre-order (natural tab order).
    fn collect_focusable_nodes(&self) -> Vec<usize> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let d = doc.borrow();
        let mut result = Vec::new();

        // Walk DOM tree depth-first from root (node 0)
        let mut stack: Vec<usize> = vec![0];
        while let Some(nid) = stack.pop() {
            let Some(node) = d.tree.get(nid) else {
                continue;
            };

            // Skip disabled nodes
            if node
                .attributes
                .get("data-disabled")
                .is_some_and(|v| v == "true")
            {
                continue;
            }

            // Skip nodes with tabindex="-1"
            if node.attributes.get("tabindex").is_some_and(|v| v == "-1") {
                continue;
            }

            // Skip zero-size nodes (not visible)
            if node.layout.width <= 0.0 || node.layout.height <= 0.0 {
                // Still push children — a zero-size parent can have visible children
            } else {
                // Check if focusable
                let has_oninput = node.attributes.contains_key("data-oninput");
                let is_contenteditable = node
                    .attributes
                    .get("contenteditable")
                    .is_some_and(|v| matches!(v.as_str(), "true" | "plaintext-only" | ""));
                let has_tabindex = node
                    .attributes
                    .get("tabindex")
                    .and_then(|v| v.parse::<i32>().ok())
                    .is_some_and(|v| v >= 0);

                if has_oninput || is_contenteditable || has_tabindex {
                    result.push(nid);
                }
            }

            // Push children in reverse order so first child is processed first
            for &child_id in node.children.iter().rev() {
                stack.push(child_id);
            }
        }

        result
    }

    /// Focus a specific element by node ID (input or contenteditable).
    fn focus_element(&mut self, node_id: usize) {
        // Determine element type
        let (has_oninput, is_contenteditable) = {
            let Some(doc) = &self.doc else { return };
            let d = doc.borrow();
            let Some(node) = d.tree.get(node_id) else {
                return;
            };
            let has_oninput = node.attributes.contains_key("data-oninput");
            let is_ce = node
                .attributes
                .get("contenteditable")
                .is_some_and(|v| matches!(v.as_str(), "true" | "plaintext-only" | ""));
            (has_oninput, is_ce)
        };

        if has_oninput {
            // Clear any CE focus first
            if let Some(prev_ce) = self.focused_contenteditable.take() {
                ce::clear_active_ce_api();
                self.ce_ops = None;
                self.set_contenteditable_attributes(prev_ce.ce_node_id, false, 0, 0);
            }
            self.try_focus_input(node_id);
            // Update DOM focus state
            if let Some(doc) = &self.doc {
                let mut d = doc.borrow_mut();
                d.update_focus(Some(node_id));
            }
        } else if is_contenteditable {
            // Clear any input focus first
            self.clear_input_focus_attrs();
            self.focused_input_handler_id = None;
            self.focused_input_value.clear();
            self.focused_input_state = None;
            self.focused_input_node_id = None;

            // Set up CE focus with cursor at position 0
            let input_handler = InputHandler::new()
                .with_multiline(true)
                .with_macos(cfg!(target_os = "macos"));

            let dom_cursor = {
                let doc = self.doc.as_ref().unwrap();
                let d = doc.borrow();
                Self::first_text_cursor(&d.tree, node_id)
                    .unwrap_or(DomCursor { node_id, offset: 0 })
            };

            self.focused_contenteditable = Some(ContentEditableFocus {
                ce_node_id: node_id,
                cursor: dom_cursor,
                anchor: dom_cursor,
                input_handler,
            });
            self.register_ce_ops(node_id, dom_cursor);
            self.set_contenteditable_attributes_dom(node_id, true, dom_cursor, dom_cursor);

            // Update DOM focus state
            if let Some(doc) = &self.doc {
                let mut d = doc.borrow_mut();
                d.update_focus(Some(node_id));
            }
        }

        self.scene_dirty = true;
    }

    /// Programmatically focus an input element by node ID.
    ///
    /// Looks up the node in the DOM, checks for `data-oninput`, and sets up
    /// the full input focus state (handler ID, editable state, cursor).
    /// This is the programmatic equivalent of clicking on an input element.
    pub(crate) fn try_focus_input(&mut self, node_id: usize) {
        let Some(doc) = &self.doc else { return };
        let d = doc.borrow();
        let Some(node) = d.tree.get(node_id) else {
            return;
        };

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
    pub fn viewport_rect(&self, name: &str) -> Option<ViewportRect> {
        self.viewport_rect_with_radius(name).map(|(r, _)| r)
    }

    /// Like `viewport_rect`, but also returns the effective border-radius
    /// from the nearest ancestor with overflow clipping + border-radius.
    /// Returns `(rect, [tl, tr, br, bl])` where radii are in logical pixels.
    pub fn viewport_rect_with_radius(&self, name: &str) -> Option<(ViewportRect, [f32; 4])> {
        use rinch_dom::computed_style::values::OverflowValue;

        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        for (node_id, node) in &d.tree.nodes {
            if node.attributes.get("data-viewport").map(|v| v.as_str()) == Some(name) {
                // Compute absolute position by walking up the parent chain.
                // Also detect orphaned subtrees: if the walk reaches a node with
                // parent=None that isn't the root, this node was removed from the
                // live tree (its ancestor was removed via remove_node).
                let mut abs_x = 0.0_f32;
                let mut abs_y = 0.0_f32;
                let mut clip_radii = [0.0_f32; 4];
                let mut connected = false;
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

                        // Check for overflow clipping ancestor with border-radius
                        if clip_radii == [0.0; 4] {
                            let cs = &n.computed_style;
                            let clips = matches!(
                                cs.overflow_y,
                                OverflowValue::Hidden
                                    | OverflowValue::Scroll
                                    | OverflowValue::Auto
                                    | OverflowValue::Clip
                            );
                            if clips {
                                let resolve_size = n.layout.width.min(n.layout.height);
                                let tl = cs.border_radius_top_left.resolve(resolve_size);
                                let tr = cs.border_radius_top_right.resolve(resolve_size);
                                let br = cs.border_radius_bottom_right.resolve(resolve_size);
                                let bl = cs.border_radius_bottom_left.resolve(resolve_size);
                                if tl > 0.0 || tr > 0.0 || br > 0.0 || bl > 0.0 {
                                    clip_radii = [tl, tr, br, bl];
                                }
                            }
                        }

                        if n.parent.is_none() {
                            // Only connected if this is the actual document root
                            connected = id == d.tree.root_id;
                        }
                        current = n.parent;
                    } else {
                        break;
                    }
                }
                if !connected {
                    continue; // orphaned subtree — skip
                }
                return Some((
                    (abs_x, abs_y, node.layout.width, node.layout.height),
                    clip_radii,
                ));
            }
        }
        None
    }

    /// Find the clip rect for a viewport by intersecting ALL overflow-clipping
    /// ancestors in the parent chain.
    ///
    /// Returns the absolute rect `(x, y, w, h)` that is the intersection of
    /// every ancestor with `overflow: hidden/scroll/auto/clip`. Returns `None`
    /// if there are no clipping ancestors.
    pub fn viewport_clip_rect(&self, name: &str) -> Option<ViewportRect> {
        use rinch_dom::computed_style::values::OverflowValue;

        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        for (_node_id, node) in &d.tree.nodes {
            // Skip orphaned nodes
            if node.parent.is_none() {
                continue;
            }
            if node.attributes.get("data-viewport").map(|v| v.as_str()) != Some(name) {
                continue;
            }

            // Helper: compute absolute position of a node
            let abs_pos = |id: usize| -> (f32, f32) {
                let mut ax = 0.0_f32;
                let mut ay = 0.0_f32;
                let mut walk = Some(id);
                while let Some(wid) = walk {
                    if let Some(wn) = d.tree.get(wid) {
                        ax += wn.layout.x;
                        ay += wn.layout.y;
                        if let Some(pid) = wn.parent
                            && let Some(pn) = d.tree.get(pid)
                        {
                            ax -= pn.scroll_offset.0 as f32;
                            ay -= pn.scroll_offset.1 as f32;
                        }
                        walk = wn.parent;
                    } else {
                        break;
                    }
                }
                (ax, ay)
            };

            // Walk ALL ancestors and intersect their clip rects
            let mut result: Option<(f32, f32, f32, f32)> = None; // x1, y1, x2, y2
            let mut current = node.parent;
            while let Some(id) = current {
                let Some(n) = d.tree.get(id) else { break };
                let cs = &n.computed_style;
                let clips = matches!(
                    cs.overflow_x,
                    OverflowValue::Hidden
                        | OverflowValue::Scroll
                        | OverflowValue::Auto
                        | OverflowValue::Clip
                ) || matches!(
                    cs.overflow_y,
                    OverflowValue::Hidden
                        | OverflowValue::Scroll
                        | OverflowValue::Auto
                        | OverflowValue::Clip
                );
                if clips {
                    let (ax, ay) = abs_pos(id);
                    let x1 = ax;
                    let y1 = ay;
                    let x2 = ax + n.layout.width;
                    let y2 = ay + n.layout.height;
                    result = Some(match result {
                        None => (x1, y1, x2, y2),
                        Some((rx1, ry1, rx2, ry2)) => {
                            (rx1.max(x1), ry1.max(y1), rx2.min(x2), ry2.min(y2))
                        }
                    });
                }
                current = n.parent;
            }

            return result.map(|(x1, y1, x2, y2)| {
                let w = x2 - x1;
                let h = y2 - y1;
                if w > 0.0 && h > 0.0 {
                    (x1, y1, w, h)
                } else {
                    // Fully clipped — return zero-area rect
                    (0.0, 0.0, 0.0, 0.0)
                }
            });
        }
        None
    }

    /// Find a render surface's layout rect by surface ID.
    ///
    /// Searches for a DOM element with `data-render-surface={id}` and returns
    /// its absolute position and size in logical pixels.
    pub fn surface_layout_rect(&self, surface_id: usize) -> Option<ViewportRect> {
        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        let id_str = surface_id.to_string();
        for (node_id, node) in &d.tree.nodes {
            // Skip orphaned nodes
            if node.parent.is_none() {
                continue;
            }
            if node
                .attributes
                .get("data-render-surface")
                .map(|v| v.as_str())
                == Some(&id_str)
            {
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
            let mut new_ops = CeOps::new(doc.clone(), ce_node_id, ce_cursor);

            // Transfer state from the old CeOps if it exists and belongs
            // to the same CE root (click re-focus shouldn't lose it).
            // Check both self.ce_ops AND the CE_API_REGISTRY — the factory
            // path creates CeOps in the registry but not in self.ce_ops.
            let old_ops_rc: Option<Rc<RefCell<dyn rinch_core::ContentEditableApi>>> = self
                .ce_ops
                .as_ref()
                .and_then(|ops| {
                    let borrow = ops.borrow();
                    if borrow.ce_node_id() == ce_node_id {
                        Some(ops.clone() as Rc<RefCell<dyn rinch_core::ContentEditableApi>>)
                    } else {
                        None
                    }
                })
                .or_else(|| ce::with_ce_api_for_node(ce_node_id, |api| api.clone()));

            if let Some(old_rc) = old_ops_rc {
                let mut old = old_rc.borrow_mut();
                if let Some(old_ce_ops) = old.as_any_mut().downcast_mut::<CeOps>() {
                    if new_ops.virtual_window.is_none() {
                        new_ops.virtual_window = old_ce_ops.virtual_window.take();
                    }
                    // Transfer editor_doc from old ops to preserve CRDT history.
                    // Use std::mem::swap to move the document efficiently.
                    {
                        let old_doc = std::mem::replace(
                            &mut old_ce_ops.editor_doc,
                            rinch_editor::EditorDocument::new(),
                        );
                        // Only transfer if the new ops were created from DOM content
                        // (not from a pre-registered pending doc).
                        if !new_ops.skip_next_sync {
                            new_ops.editor_doc = old_doc;
                            new_ops.skip_next_sync = true;
                        }
                    }
                }
            }
            let ops = Rc::new(RefCell::new(new_ops));
            ce::set_active_ce_api(ops.clone());
            ce::register_ce_api(ce_node_id, ops.clone());
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
