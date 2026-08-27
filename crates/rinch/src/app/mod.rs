//! Platform-agnostic application logic.
//!
//! `RinchApp` holds the reactive document, scene graph, cursor state, devtools,
//! and all input-handling logic that is independent of the windowing backend.
//! The desktop shell (winit + wgpu) translates native events into
//! [`PlatformEvent`]s, feeds them to `RinchApp`, and processes the returned
//! [`AppAction`]s.

mod click_handling;
#[cfg(feature = "debug")]
mod debug_commands;
mod event_dispatch;
mod focus;
pub(crate) mod hit_testing;
#[cfg(test)]
mod input_commit_tests;
#[cfg(test)]
mod input_ime_tests;
mod select_widget;
mod text_selection;

pub(crate) use hit_testing::*;

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};
use rinch_core::events;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::Painter;
// Painter selection is ADDITIVE (issue #140): the software painter is compiled
// whenever a native shell presents via TinySkia (`software_shell`, emitted by
// build.rs = desktop/android without gpu/android-gpu), and the Vello painter
// whenever anything drives `build_scene` (gpu/android-gpu/embed). Under
// desktop(software) + embed BOTH are present: the winit shell uses
// `build_pixels` while embed `RinchContext`s use `build_scene`.
#[cfg(software_shell)]
use rinch_dom::paint::skia_painter::TinySkiaPainter;
#[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
use rinch_dom::paint::vello_painter::VelloPainter;
use rinch_dom::text_query::byte_offset_from_position;
#[cfg(feature = "debug")]
use rinch_dom::text_query::caret_position_for_offset_layout;
#[cfg(feature = "debug")]
use rinch_dom::text_query::glyph_bounds_for_offset_layout;
use rinch_editable::{EditCommand, EditableDocument, EditableState, Selection, StringDocument};
use rinch_platform::{
    AppAction, ImeEvent, Instant, KeyCode, Modifiers, MouseButton, PlatformEvent, UserEvent,
};
#[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
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
    #[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
    pub snapshot: VelloPainter,
    /// Captured RGBA pixels of the source element's subtree (software backend).
    #[cfg(software_shell)]
    pub snapshot_pixels: Vec<u8>,
    /// Width of the snapshot pixmap in physical pixels.
    #[cfg(software_shell)]
    pub snapshot_width: u32,
    /// Height of the snapshot pixmap in physical pixels.
    #[cfg(software_shell)]
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

// ── Focus arbiter ────────────────────────────────────────────────────────────

/// What currently owns keyboard / IME input. Exactly one target is focused at a
/// time — the [`RinchApp::set_focus_target`] choke-point tears the previous one
/// down before installing the next, so focus is **mutually exclusive by
/// construction** (design A10). The runtime routes `KeyDown`/`KeyUp` (and `Ime`)
/// by matching on this single field instead of the old order-dependent
/// interceptor-then-fallback chain that caused the dual-arm routing bugs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum FocusTarget {
    /// Nothing focused — keys fall through to the global handlers (DevTools,
    /// inspect mode, Tab navigation, read-only text-selection caret motion).
    #[default]
    None,
    /// A render surface (game viewport / custom renderer), by surface id.
    Surface(usize),
    /// An `<input>`/`<textarea>` (the rinch-editable engine), by DOM node id.
    Input(usize),
    /// A native `<select>` whose popup is open, by the select's DOM node id. The
    /// popup nodes and highlight live in [`RinchApp::open_select`] (issue #121).
    Select(usize),
    /// A rich-text editor (`rinch-editor-core`) instance, by container node id.
    #[cfg(feature = "desktop")]
    Editor(usize),
    /// A generic focusable DOM node (`tabindex >= 0`, no text engine), by DOM
    /// node id — a custom control reached via Tab or `request_focus`
    /// (issue #228). Enter/Space dispatch its click handler; it drives no IME.
    Node(usize),
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
    #[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
    pub(crate) painter: VelloPainter,
    /// Software painter (reused across frames). Uses tiny-skia for CPU rendering.
    #[cfg(software_shell)]
    pub(crate) skia_painter: Option<TinySkiaPainter>,
    /// Parley layout context for paint-time text layout (debug screenshots).
    #[cfg(feature = "debug")]
    pub(crate) paint_layout_cx: parley::LayoutContext<peniko::Brush>,
    /// Current cursor position.
    pub(crate) cursor_pos: Option<(f32, f32)>,
    /// Active scrollbar drag state.
    pub(crate) scrollbar_drag: Option<ScrollbarDrag>,
    /// Pending drag-and-drop: mousedown on draggable, awaiting threshold.
    pub(crate) pending_drag: Option<PendingDrag>,
    /// Active drag-and-drop: threshold crossed, snapshot captured.
    pub(crate) active_dnd: Option<ActiveDrag>,
    /// Surface currently being dragged over (for DragEnter/DragLeave dispatch).
    /// Stores (surface_id, dom_node_id).
    pub(crate) drag_over_surface: Option<(usize, usize)>,
    /// Last theme CSS loaded into the document (for change detection).
    pub(crate) last_theme_css: Option<String>,
    /// Per-document theme CSS owned by this app (issue #138). `None` = follow
    /// the thread-global theme slot (the single-root shell/web/android paths).
    /// Embed contexts set this so creating another context on the same thread
    /// never restyles this one.
    pub(crate) owned_theme_css: Option<String>,
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
    /// Text rendering scale for HiDPI/mobile (applied to Parley font sizes).
    pub(crate) text_scale: f32,
    /// Whether we have a previous frame's pixels for dirty region caching.
    #[cfg(software_shell)]
    pub(crate) has_previous_frame: bool,
    /// The data-oninput handler ID for the currently focused text input.
    pub(crate) focused_input_handler_id: Option<usize>,
    /// Current accumulated text value for the focused text input.
    pub(crate) focused_input_value: String,
    /// The focused input's value at the last commit point — focus-take, or an
    /// Enter commit — for the `data-onchange` commit boundary (issue #226).
    /// `focused_input_value` is the live per-keystroke buffer, so "did the
    /// gesture change anything" needs this separate snapshot: re-reading the
    /// `value` attribute at teardown would be circular for controlled inputs
    /// (the attribute tracks the live buffer). Seeded only when
    /// [`Self::set_focus_target`] reports an actual focus change — a re-click
    /// inside the already-focused input moves the caret, not the baseline.
    pub(crate) focused_input_baseline: String,
    /// Editable state for the focused text input (cursor, selection, undo).
    pub(crate) focused_input_state: Option<EditableState<StringDocument>>,
    /// DOM node ID of the currently focused text input.
    pub(crate) focused_input_node_id: Option<usize>,
    /// In-progress IME composition (preedit) for the focused `<input>`: the
    /// composing string and an optional `(begin, end)` byte cursor within it.
    /// Rendered inline at the input caret as an underlined overlay (via the
    /// `data-preedit` attribute) and never part of the input's committed value.
    pub(crate) focused_input_preedit: Option<(String, Option<(usize, usize)>)>,
    /// A programmatic `value` write to the focused `<input>` that arrived while
    /// an IME composition was in flight (issue #238). Adopting it then would
    /// move the caret under the composition, so it is held here and applied
    /// when the composition commits or is cancelled; a later write replaces it.
    pub(crate) focused_input_deferred_value: Option<String>,
    /// The single authority for which widget owns keyboard/IME input (design
    /// A10). Kept in lockstep with the per-engine focus state below
    /// (`focused_input_*`, the surface/editor registries) via
    /// [`Self::set_focus_target`].
    pub(crate) focus_target: FocusTarget,
    /// The Enter/Space key currently latched by a `FocusTarget::Node`
    /// activation (issue #228). OS auto-repeat delivers indistinguishable
    /// KeyDowns (`PlatformEvent::KeyDown` carries no repeat flag), so a held
    /// key must activate once per physical press; cleared on the matching
    /// KeyUp.
    pub(crate) node_activation_held: Option<KeyCode>,
    /// The open native-`<select>` popup, if any. Present exactly when
    /// `focus_target == FocusTarget::Select(_)`. Holds the app-created popup DOM
    /// node ids and the keyboard highlight state (issue #121).
    pub(crate) open_select: Option<select_widget::OpenSelect>,
    /// Whether the native-select popup stylesheet has been injected (once).
    pub(crate) select_css_injected: bool,
    /// The "goal column" (a window-space x) preserved across consecutive vertical
    /// cursor moves (Up/Down) in the focused new editor, so the caret keeps its
    /// horizontal position through short lines instead of drifting to line ends.
    /// Set on the first Up/Down, reset by any other key, click, or drag.
    #[cfg(feature = "desktop")]
    pub(crate) editor_goal_x: Option<f32>,
    /// The render surface currently under the mouse cursor (for enter/leave events).
    /// Stores (surface_id, dom_node_id) so we can compute local coords during drags.
    pub(crate) hovered_surface: Option<(usize, usize)>,
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
    /// When true, `mount_component` namespaces stores/contexts under the
    /// document's `doc_key` (set by embed `RinchContext`s, issue #136). Shell
    /// roots leave this false and keep writing to the thread-global root 0.
    pub(crate) scope_context_to_doc: bool,
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
            #[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
            painter: VelloPainter::new(),
            #[cfg(software_shell)]
            skia_painter: None,
            #[cfg(feature = "debug")]
            paint_layout_cx: parley::LayoutContext::new(),
            cursor_pos: None,
            scrollbar_drag: None,
            pending_drag: None,
            active_dnd: None,
            drag_over_surface: None,
            last_theme_css: None,
            owned_theme_css: None,
            last_click_time: Instant::now(),
            last_click_pos: (0.0, 0.0),
            click_count: 0,
            hit_test_font_cx: parley::FontContext::new(),
            window_props: None,
            modifiers: Modifiers::default(),
            scene_dirty: true,
            text_scale: 1.0,
            #[cfg(software_shell)]
            has_previous_frame: false,
            focused_input_handler_id: None,
            focused_input_value: String::new(),
            focused_input_baseline: String::new(),
            focused_input_state: None,
            focused_input_node_id: None,
            focused_input_preedit: None,
            focused_input_deferred_value: None,
            focus_target: FocusTarget::None,
            node_activation_held: None,
            open_select: None,
            select_css_injected: false,
            #[cfg(feature = "desktop")]
            editor_goal_x: None,
            hovered_surface: None,
            text_selection: None,
            text_selecting: false,
            file_hover_target: None,
            inspect_highlight: None,
            pending_fonts: Vec::new(),
            scope_context_to_doc: false,
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

    /// The theme CSS this document should be using: the per-document owned CSS
    /// when set (embed contexts, issue #138), otherwise the thread-global slot
    /// (the single-root shell/web/android default).
    #[cfg(feature = "theme")]
    pub(crate) fn effective_theme_css(&self) -> String {
        match &self.owned_theme_css {
            Some(css) => css.clone(),
            None => rinch_core::get_current_theme_css().unwrap_or_default(),
        }
    }

    /// Mount the component, building the initial DOM.
    ///
    /// Called once after the window and renderer are ready.
    pub fn mount_component(&mut self, viewport_width: f32, viewport_height: f32) {
        let doc = Rc::new(RefCell::new(RinchDocument::new()));
        doc.borrow_mut().tree.text_scale = self.text_scale;

        // Set up network image loader if feature enabled (replaces default FileImageLoader)
        #[cfg(all(feature = "image-network", not(target_arch = "wasm32")))]
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
                let theme_css = self.effective_theme_css();
                if !theme_css.is_empty() {
                    // Must go through the theme slot (not load_css) so a later
                    // theme regeneration replaces this sheet instead of stacking
                    // a second one after the app's CSS.
                    d.set_theme_css(&theme_css);
                }
            }
            // Set viewport size so vh/vw units resolve correctly during DOM construction
            d.set_viewport(viewport_width, viewport_height);
        }

        // Remember the initial theme CSS so we can detect changes later
        #[cfg(feature = "theme")]
        {
            self.last_theme_css = Some(self.effective_theme_css());
        }

        // An embed context namespaces its stores/contexts under the document's
        // doc_key for the whole mount — the component run plus every effect it
        // creates captures this root (issue #136). Shell roots don't push a
        // root and keep writing to the thread-global root 0.
        let _root_guard = self
            .scope_context_to_doc
            .then(|| rinch_core::push_context_root(doc.borrow().doc_key()));

        // Create RenderScope
        let doc_as_dom: Rc<RefCell<dyn DomDocument>> = doc.clone();
        let body_id = doc.borrow().body();
        let scope = Rc::new(RefCell::new(RenderScope::new(doc_as_dom, body_id)));

        // Set thread-local context
        set_render_scope(scope.clone());

        // Run the component
        let component = self.component.take().expect("component already consumed");
        // The root scope owns everything the component tree builds (issue
        // #141). Deliberately narrower than `_root_guard` above, which spans the
        // whole function: the owner must not cover initial layout, caret
        // updates or scroll-clamp dispatch, none of which belong to the tree.
        let root = {
            let _owner = scope.borrow().push_owner();
            let mut scope_ref = scope.borrow_mut();
            component(&mut scope_ref)
        };

        // Append root to body
        doc.borrow_mut().append_child(body_id, root.node_id());

        clear_render_scope();

        // Initial layout
        {
            #[cfg(feature = "desktop")]
            let focused = self.focused_editor_id();
            let mut d = doc.borrow_mut();
            // New-editor: virtualize each large scroll editor BEFORE the first
            // layout so its off-screen blocks are never Parley-measured at mount.
            // The scroll-container gate reads computed `overflow-y`, so resolve
            // styles first (the `resolve_layout` below re-resolves them); the
            // post-layout pass then settles the off-screen height estimates so they
            // don't jump on the first interaction.
            #[cfg(feature = "desktop")]
            {
                d.resolve_styles();
                crate::editor::virtualization_pre_layout(&mut d, focused);
            }
            d.resolve_layout(viewport_width, viewport_height);
            #[cfg(feature = "desktop")]
            crate::editor::virtualization_post_layout(
                &mut d,
                focused,
                viewport_width,
                viewport_height,
            );
            let _ = d.take_dirty_nodes();
        }

        // New-editor phase 2 (design A3): render mounted editors' carets against
        // the fresh initial layout (the steady-state pass in resolve_and_repaint
        // short-circuits when nothing is dirty, so the first caret renders here).
        // Key off the local `doc` — `self.doc` is only assigned below, so
        // `self.doc_key()` would still be 0 here and filter every editor out.
        #[cfg(feature = "desktop")]
        crate::editor::update_all_carets(Some(doc.borrow().doc_key()), self.focused_editor_id());

        self.scene_dirty = true;
        self.doc = Some(doc.clone());
        self._render_scope = Some(scope);

        // The initial layout can clamp a scroll offset a component set during
        // construction (via set_scroll_top) — drain + notify (#144).
        self.dispatch_scroll_clamp_events();
    }

    // ── Layout / repaint ─────────────────────────────────────────────────

    /// Run the post-layout overlay pass for mounted editors (from an input handler,
    /// where a selection-only change doesn't dirty the document) and, if any overlay
    /// moved, force a full repaint. The caret / selection / node-outline overlays are
    /// absolutely positioned, and the software renderer's dirty-region cache can't
    /// clear a moved absolute element's old rect — so without this they ghost.
    #[cfg(feature = "desktop")]
    pub(crate) fn refresh_editor_overlays(&mut self) {
        if crate::editor::update_all_carets(Some(self.doc_key()), self.focused_editor_id()) {
            self.scene_dirty = true;
            #[cfg(software_shell)]
            {
                self.has_previous_frame = false;
            }
        }
    }

    /// Re-resolve layout after signal changes. Returns `true` if a redraw
    /// is needed.
    pub fn resolve_and_repaint(&mut self, viewport_width: f32, viewport_height: f32) -> bool {
        let Some(doc) = self.doc.clone() else {
            return false;
        };
        let doc = &doc;

        // A `value` write to the focused input from outside any keystroke — a
        // click handler, a menu, a timer — is adopted here (issue #238); a
        // string compare when nothing was written.
        self.adopt_focused_input_value_from_dom();

        // Check if theme CSS has changed (e.g. primary color or dark mode toggled)
        #[allow(unused_assignments, unused_mut)]
        let mut theme_changed = false;
        #[cfg(feature = "theme")]
        {
            let current_theme = self.effective_theme_css();
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
                #[cfg(software_shell)]
                {
                    self.has_previous_frame = false;
                }
            }
        }

        // New-editor block virtualization (design A3, phase 1) — run BEFORE the
        // short-circuit. Creating a window (or moving the materialized range) sets
        // the document's style/layout dirty flags, so this frame won't short-circuit
        // and the resolve below applies the collapse. A selection-only first
        // interaction otherwise never triggers the initial collapse.
        #[cfg(feature = "desktop")]
        {
            let focused = self.focused_editor_id();
            let mut d = doc.borrow_mut();
            crate::editor::virtualization_pre_layout(&mut d, focused);
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

            // Check if the inset fast path requested a full repaint
            // (absolute element moved — dirty region caching can't track the
            // old position reliably, so force full scene rebuild).
            if d.tree.full_repaint_needed {
                d.tree.full_repaint_needed = false;
                self.scene_dirty = true;
                #[cfg(software_shell)]
                {
                    self.has_previous_frame = false;
                }
            }
        }

        // Apply deferred scroll-into-view now that layout is fresh
        self.apply_scroll_into_view();

        // New-editor block virtualization (design A3, phase 2): cache measured
        // heights and re-verify the materialized range with fresh positions, re
        // laying out once if a big scroll jump changed it. Before the caret pass so
        // the caret reads post-virtualization geometry.
        #[cfg(feature = "desktop")]
        {
            let focused = self.focused_editor_id();
            let mut d = doc.borrow_mut();
            crate::editor::virtualization_post_layout(
                &mut d,
                focused,
                viewport_width,
                viewport_height,
            );
        }

        // New-editor phase 2 (design A3): render each mounted editor's caret from
        // its selection now that layout geometry is fresh. If an overlay (caret /
        // selection / node-outline) actually moved, re-resolve so its new absolute
        // position is current, then force a full repaint — the software renderer's
        // dirty-region cache can't clear a moved absolute element's *old* rect, so
        // the overlay would otherwise ghost.
        #[cfg(feature = "desktop")]
        if crate::editor::update_all_carets(Some(self.doc_key()), self.focused_editor_id()) {
            {
                let mut d = doc.borrow_mut();
                let _ = d.take_dirty_nodes();
                d.resolve_layout(viewport_width, viewport_height);
            }
            self.scene_dirty = true;
            #[cfg(software_shell)]
            {
                self.has_previous_frame = false;
            }
        }

        // Dispatch deferred scroll events for offsets the layout clamped, now
        // that the last resolve of the frame has run (#144).
        self.dispatch_scroll_clamp_events();

        // Refresh element-bounds signals against the freshly-computed layout.
        self.refresh_bounds_signals();

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

    /// Refresh element-bounds signals against the freshly-computed layout.
    ///
    /// Measures every registered node **while holding the document borrow**,
    /// then releases it before publishing. Subscribers (e.g. timeline-style
    /// widgets reading a strip's measured width) re-run synchronously inside
    /// `update_bounds_signals`, and a reactive `style:` closure — the idiom
    /// [`NodeHandle::bounds_signal`]'s own docs recommend — takes
    /// `doc.borrow_mut()` to patch the attribute. Publishing under the read
    /// borrow made that a `BorrowMutError` (#141).
    ///
    /// Called from `resolve_and_repaint` and from `resize_layout` — the resize
    /// path drains the dirty set, so the subsequent paint skips
    /// `resolve_and_repaint` and this is the only refresh a pure resize gets
    /// (#145).
    fn refresh_bounds_signals(&self) {
        let Some(doc) = &self.doc else { return };

        // Phase 1: measure under the read borrow.
        let (doc_key, measured) = {
            let d = doc.borrow();
            let doc_key = d.doc_key();
            let measured: Vec<(u64, (f32, f32, f32, f32))> =
                rinch_core::reactive::registered_bounds_nodes(doc_key)
                    .into_iter()
                    .filter_map(|node_id| {
                        let n = d.tree.nodes.get(node_id as usize)?;
                        // Walk to root accumulating parent-relative offsets — same
                        // convention as `dispatch_oncontextmenu` / `click_handling`.
                        let mut ax = n.layout.x;
                        let mut ay = n.layout.y;
                        let mut pid = n.parent;
                        while let Some(p) = pid {
                            if let Some(pn) = d.tree.nodes.get(p) {
                                ax += pn.layout.x;
                                ay += pn.layout.y;
                                ax -= pn.scroll_offset.0 as f32;
                                ay -= pn.scroll_offset.1 as f32;
                                pid = pn.parent;
                            } else {
                                break;
                            }
                        }
                        Some((node_id, (ax, ay, n.layout.width, n.layout.height)))
                    })
                    .collect();
            (doc_key, measured)
        };

        // Phase 2: publish with no document borrow held, so a woken effect is
        // free to mutate the DOM.
        rinch_core::reactive::update_bounds_signals(doc_key, |node_id| {
            measured
                .iter()
                .find(|(id, _)| *id == node_id)
                .map(|(_, rect)| *rect)
        });
    }

    /// Dispatch deferred scroll events for offsets the layout engine clamped.
    ///
    /// `resolve_layout` clamps a scroll container's offset when its content
    /// shrinks or its viewport grows (#144). The clamp runs while this app
    /// holds the document borrow, so the engine queues the (node, offset)
    /// pairs; this drains them once the borrow is released and fires the same
    /// `data-onscroll` handlers input-driven scrolling does.
    fn dispatch_scroll_clamp_events(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let clamps = doc.borrow_mut().drain_scroll_clamps();
        if clamps.is_empty() {
            return;
        }
        // Resolve handler ids under a read borrow, then drop it before
        // dispatching — handlers are user code and may call back into the
        // document.
        let mut to_fire: Vec<(usize, f64)> = Vec::new();
        {
            let d = doc.borrow();
            for (node, scroll_top) in clamps {
                if let Some(handler_id) = d
                    .tree
                    .nodes
                    .get(node.0)
                    .and_then(|n| n.attributes.get("data-onscroll"))
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    to_fire.push((handler_id, scroll_top));
                }
            }
        }
        for (handler_id, scroll_top) in to_fire {
            use rinch_core::events::{EventHandlerId, dispatch_scroll_event};
            dispatch_scroll_event(EventHandlerId(handler_id), scroll_top);
        }
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
        let Some(doc) = self.doc.clone() else { return };
        {
            let mut d = doc.borrow_mut();
            d.resolve_layout(width as f32, height as f32);
            let _ = d.take_dirty_nodes();
        }
        self.scene_dirty = true;
        // Layout at the new size may have clamped scroll offsets — notify (#144).
        self.dispatch_scroll_clamp_events();
        // The drain above emptied the dirty set, so the subsequent paint skips
        // `resolve_and_repaint`; without this a pure resize leaves bounds
        // signals reporting the pre-resize geometry (#145).
        self.refresh_bounds_signals();
    }

    /// Build the Vello scene from the current document state.
    ///
    /// The scene is painted via the `Painter` trait and a reference to the
    /// underlying `vello::Scene` is returned for the GPU renderer.
    #[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
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

        // Render drag-and-drop snapshot overlay (if not suppressed by drop target)
        if let Some(ref drag) = self.active_dnd {
            if rinch_core::events::is_drag_ghost_visible() {
                use peniko::kurbo::Affine;
                let tx = (drag.cursor.0 - drag.anchor.0) as f64;
                let ty = (drag.cursor.1 - drag.anchor.1) as f64;
                self.painter
                    .scene_mut()
                    .append(drag.snapshot.scene(), Some(Affine::translate((tx, ty))));
            }
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
    #[cfg(software_shell)]
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
                let mut d = doc.borrow_mut();
                d.tree.paint_dirty_nodes.clear();
                d.tree.paint_dirty_removed_rects.clear();
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

            // Paint drag-and-drop snapshot overlay (if not suppressed by drop target)
            if let Some(ref drag) = self.active_dnd {
                if rinch_core::events::is_drag_ghost_visible() {
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
    #[cfg(software_shell)]
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

    /// Check if there are pending layout changes that need resolving
    /// before the next paint. Covers DOM mutations, style changes, and
    /// structural changes that set layout_dirty directly.
    pub fn has_pending_layout(&self) -> bool {
        self.doc
            .as_ref()
            .map(|doc| {
                let d = doc.borrow();
                !d.tree.dirty_nodes.is_empty() || d.tree.styles_dirty || d.tree.layout_dirty
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

    /// The focused input's handler id, **if its handler is still registered**.
    ///
    /// Disposing a scope frees the event handlers it owns (issue #141), so this
    /// cached id dangles the instant the focused `<input>`'s branch is torn down
    /// — a modal closing, an `if` flipping, a `for` item being removed. Every
    /// consumer below then fails silently: `dispatch_input_event` returns a
    /// `false` nobody reads, `update_focused_input_dom_value` matches no node so
    /// the DOM stops tracking `focused_input_value`, and the window keeps IME
    /// enabled for an element that no longer exists.
    ///
    /// So a miss self-heals by dropping focus entirely, mirroring what
    /// `FocusTarget::Editor` already does when its container is unmounted. That
    /// also clears `focused_input_node_id`, which matters: it is a slab index,
    /// and a recycled one would aim the caret/value attribute writes at an
    /// unrelated element.
    ///
    /// Must be called with no outstanding borrow of `self.doc` — `set_focus_target`
    /// writes DOM attributes.
    fn live_focused_input_handler(&mut self) -> Option<usize> {
        let handler_id = self.focused_input_handler_id?;
        if events::has_input_handler(events::EventHandlerId(handler_id)) {
            return Some(handler_id);
        }
        tracing::debug!(
            "focused input handler {handler_id} was freed with its scope; dropping focus"
        );
        self.set_focus_target(FocusTarget::None);
        None
    }

    /// Central dispatch: execute an EditCommand on the focused input's EditableState.
    fn handle_input_edit_command(&mut self, cmd: EditCommand) {
        let Some(handler_id) = self.live_focused_input_handler() else {
            return;
        };
        // The edit applies to what the field displays: adopt any `value` write
        // that landed since the last sync (issue #238).
        self.adopt_focused_input_value_from_dom();
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
            // Effects flush synchronously inside the dispatch: a controlled
            // input's `value_fn` (or a normalizing handler) may just have
            // rewritten the value. Adopt it now, so the rewrite is what the
            // next keystroke edits and what the caret is placed in — not the
            // stale text that would otherwise be painted back over it on the
            // next sync (issue #238).
            self.adopt_focused_input_value_from_dom();
        }
    }

    fn handle_text_input(&mut self, text: &str) {
        if self.focused_input_state.is_some() {
            self.handle_input_edit_command(EditCommand::InsertText(text.to_string()));
        } else if let Some(handler_id) = self.live_focused_input_handler() {
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
        } else if let Some(handler_id) = self.live_focused_input_handler() {
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
        // Check if a text input is focused and has onchange/onsubmit handlers.
        // Probe liveness first: with a freed id the block below still runs, the
        // node lookups match nothing, and Enter is swallowed rather than falling
        // through to the global handlers (issue #141).
        if self.live_focused_input_handler().is_none() {
            return;
        }
        // Enter is a commit boundary but not an edit command, so neither of
        // `handle_input_edit_command`'s adopts has run: pull in any pending
        // `value` write first, so the change gate below compares — and the
        // payload carries — the text the field actually displays (issue #238).
        self.adopt_focused_input_value_from_dom();
        // Resolve the focused input's node: the stored id, else a linear scan
        // for the node carrying the focused oninput handler.
        let node_id = self.focused_input_node_id.or_else(|| {
            let doc = self.doc.as_ref()?;
            let d = doc.borrow();
            d.tree.nodes.iter().find_map(|(id, node)| {
                node.attributes
                    .get("data-oninput")
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&h| Some(h) == self.focused_input_handler_id)
                    .map(|_| id)
            })
        });
        // Resolve the change handler (walking up: `change` bubbles on the web,
        // so a delegating ancestor's handler counts — the desktop matches),
        // the commit payload, and the control's tag. `data-onsubmit` is
        // deliberately resolved AFTER the change dispatch below: a change
        // handler may re-render the input, freeing and re-registering the
        // submit handler, and dispatching the stale id would silently eat
        // Enter (#244 review).
        let (change_handler_id, change_payload, is_textarea) = match (node_id, &self.doc) {
            (Some(nid), Some(doc)) => {
                let d = doc.borrow();
                (
                    Self::input_attr_handler_up(&d.tree, nid, "data-onchange"),
                    d.tree
                        .get(nid)
                        .and_then(|n| n.attributes.get("value").cloned()),
                    d.tree.get(nid).and_then(|n| n.tag()) == Some("textarea"),
                )
            }
            _ => (None, None, false),
        };

        // Enter is an explicit commit (issue #226) for single-line inputs: fire
        // `data-onchange` before `data-onsubmit` — HTML orders change before
        // submit — and only if the value changed since the gesture began. A
        // `<textarea>` is exempt: browsers never fire change on Enter there,
        // so its gesture (and baseline) runs until blur.
        if !is_textarea
            && self.focused_input_value != self.focused_input_baseline
            && let Some(hid) = change_handler_id
            && events::has_input_handler(events::EventHandlerId(hid))
        {
            // Payload: the live `value` attribute — what the field displays and
            // what the web backend delivers — falling back to the keystroke
            // buffer. The buffer-vs-baseline gate above stays authoritative for
            // "did the user change anything".
            let payload = change_payload.unwrap_or_else(|| self.focused_input_value.clone());
            events::dispatch_input_event(events::EventHandlerId(hid), payload);
            // A controlled change handler may have rewritten the value (e.g. a
            // normalize-on-commit); pull the rewrite into the editable state
            // (a no-op when nothing was rewritten, so the caret stays put).
            self.adopt_focused_input_value_from_dom();
        }
        // Post-change submit resolution — see the comment above.
        let submit_handler_id = node_id.and_then(|nid| {
            let doc = self.doc.as_ref()?;
            let d = doc.borrow();
            d.tree
                .get(nid)
                .and_then(|n| n.attributes.get("data-onsubmit"))
                .and_then(|s| s.parse::<usize>().ok())
        });
        if let Some(handler_id) = submit_handler_id {
            events::dispatch_event(events::EventHandlerId(handler_id));

            // After onsubmit, the handler may have changed the signal (e.g., cleared it).
            // Re-read the value and adopt it into the editable state.
            self.adopt_focused_input_value_from_dom();
        }
        if !is_textarea {
            // Enter committed: the gesture measures from here on, so the
            // eventual real blur doesn't re-fire an already-committed change.
            // (A textarea never committed — its gesture continues.)
            self.focused_input_baseline = self.focused_input_value.clone();
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
            d.tree.paint_dirty_nodes.push(node_id);
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
            node.attributes.remove("data-preedit");
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

        // Find current focused element: a focused input, a generic focusable
        // node held by the arbiter, or — falling back — the DOM's focused node
        // resolved upward to its nearest focusable ancestor (a pointer click
        // focuses the deepest hit element, so a click inside a focusable node
        // must still anchor Tab there).
        let current = match self.focus_target {
            // The arbiter is the source of truth; `focused_input_node_id` only
            // ever accompanies `Input` (kept as a fallback so a broken
            // invariant degrades to the same answer rather than anchoring Tab
            // on a stale input).
            FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
            _ => self.focused_input_node_id,
        };

        let current_idx = current
            .and_then(|id| focusable.iter().position(|&fid| fid == id))
            .or_else(|| {
                let doc = self.doc.as_ref()?;
                let d = doc.borrow();
                let mut cur = d.tree.focused_node;
                while let Some(id) = cur {
                    if let Some(idx) = focusable.iter().position(|&fid| fid == id) {
                        return Some(idx);
                    }
                    cur = d.tree.get(id).and_then(|n| n.parent);
                }
                None
            });

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

            // A disabled or negative-tabindex node is not itself focusable, but
            // its children still are (web semantics remove only the node from
            // the Tab order, not its subtree). Same for zero-size or
            // `visibility: hidden` (invisible, unclickable) nodes. The tabindex
            // test parses like the focusable test below so `-2`, `-01`, … are
            // negative too, not just the literal string "-1".
            let skip_self = node
                .attributes
                .get("data-disabled")
                .is_some_and(|v| v == "true")
                || node
                    .attributes
                    .get("tabindex")
                    .and_then(|v| v.parse::<i32>().ok())
                    .is_some_and(|v| v < 0)
                || node.layout.width <= 0.0
                || node.layout.height <= 0.0
                || matches!(
                    node.computed_style.visibility,
                    rinch_dom::computed_style::VisibilityValue::Hidden
                        | rinch_dom::computed_style::VisibilityValue::Collapse
                );

            if !skip_self {
                // Check if focusable
                let has_oninput = node.attributes.contains_key("data-oninput");
                let has_tabindex = node
                    .attributes
                    .get("tabindex")
                    .and_then(|v| v.parse::<i32>().ok())
                    .is_some_and(|v| v >= 0);

                if has_oninput || has_tabindex {
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

    /// Focus a specific element by node ID via Tab: an `<input>`/`<textarea>`,
    /// or a generic `tabindex >= 0` node (issue #228). Keyboard-driven, so the
    /// focused node gets the `:focus-visible` ring either way.
    fn focus_element(&mut self, node_id: usize) {
        let has_oninput = {
            let Some(doc) = &self.doc else { return };
            let d = doc.borrow();
            let Some(node) = d.tree.get(node_id) else {
                return;
            };
            node.attributes.contains_key("data-oninput")
        };

        if has_oninput {
            // `try_focus_input` takes focus through the arbiter (tears down any
            // prior surface / editor / input).
            self.try_focus_input(node_id);
        } else {
            // A generic focusable node: take focus through the arbiter too, so
            // the previous owner (an input's keys and IME included) is torn
            // down instead of lingering alongside a focus that went nowhere.
            self.set_focus_target(FocusTarget::Node(node_id));
        }

        // Update DOM focus state and mark the keyboard focus ring — but only
        // when the arbiter actually claimed this node. `try_focus_input` bails
        // on a malformed `data-oninput` (or a vanished node); painting `:focus`
        // + the ring on a node that owns no keys would split the visual focus
        // from the keyboard focus.
        let claimed = matches!(
            self.focus_target,
            FocusTarget::Input(id) | FocusTarget::Node(id) if id == node_id
        );
        if claimed && let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            d.update_focus(Some(node_id));
            d.set_focus_visible(node_id, true);
            // Sequential focus navigation scrolls the new focus into view
            // (applied by `apply_scroll_into_view` after the next layout pass),
            // like browsers — Tab must never land on a control the user can't
            // see because it sits below the fold of a scroll container.
            d.request_scroll_into_view(rinch_core::dom::NodeId(node_id));
        }

        self.scene_dirty = true;
    }

    /// Enter/Space on a keyboard-focused generic node (issue #228): dispatch
    /// the click handler of the nearest ancestor-or-self carrying a **live**
    /// `data-rid` (the same liveness probe as the pointer path — a freed
    /// handler must not swallow the key), with a `ClickContext` synthesized
    /// from the handler node's absolute rect, cursor at its center. A node with
    /// no live handler anywhere in its chain is a quiet no-op.
    fn activate_focused_node(&mut self, node_id: usize, vp_w: f32, vp_h: f32) {
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let d = doc.borrow();
        let mut current = Some(node_id);
        while let Some(nid) = current {
            let Some(node) = d.tree.get(nid) else { break };
            if let Some(rid_str) = node.attributes.get("data-rid")
                && let Ok(handler_id) = rid_str.parse::<usize>()
                && events::has_click_handler(events::EventHandlerId(handler_id))
            {
                // The same absolute-rect walk as the pointer path, via the
                // shared helper (it stops at `position: fixed`, which is
                // viewport-relative — the hand-rolled copy this replaces did
                // not).
                let (elem_x, elem_y) = Self::compute_absolute_position(&d.tree, nid);
                let (elem_w, elem_h) = (node.layout.width, node.layout.height);

                events::set_click_context(events::ClickContext {
                    mouse_x: elem_x + elem_w / 2.0,
                    mouse_y: elem_y + elem_h / 2.0,
                    element_x: elem_x,
                    element_y: elem_y,
                    element_width: elem_w,
                    element_height: elem_h,
                    text_hit: events::TextHitInfo::default(),
                    viewport_width: vp_w,
                    viewport_height: vp_h,
                    button: events::MouseButton::Left,
                    modifiers: self.modifier_state(),
                });
                events::set_click_ancestors(Self::collect_click_ancestors(&d.tree, nid));

                drop(d);
                events::dispatch_event(events::EventHandlerId(handler_id));
                // The handler may have requested focus (e.g. opening a dialog
                // that focuses an input) — honor it like the pointer path does.
                if let Some(focus_node_id) = rinch_core::take_pending_focus_request(self.doc_key())
                {
                    self.try_focus_input(focus_node_id);
                }
                self.scene_dirty = true;
                return;
            }
            current = node.parent;
        }
    }

    /// Whether a `FocusTarget::Node` claim still names a live, attached,
    /// focusable node. Node ids are recycled slab indices (the same hazard
    /// [`Self::live_focused_input_handler`] documents for inputs), so a claim
    /// that outlived its node must be dropped before it swallows Enter/Space,
    /// anchors Tab, or activates whatever unrelated node reused the slot. A
    /// recycled slot that happens to hold another focusable node is still
    /// accepted — full protection needs an unmount notification (like the
    /// editor registry gives the Editor target).
    fn node_target_is_live(&self, node_id: usize) -> bool {
        let Some(doc) = &self.doc else { return false };
        let d = doc.borrow();
        let focusable = d.tree.get(node_id).is_some_and(|n| {
            n.attributes
                .get("tabindex")
                .and_then(|v| v.parse::<i32>().ok())
                .is_some()
        });
        if !focusable {
            return false;
        }
        // Attached to the root? A detached node keeps its slab slot (and its
        // attributes) but must not stay focus-owner.
        let mut cur = Some(node_id);
        while let Some(nid) = cur {
            if nid == 0 {
                return true;
            }
            cur = d.tree.get(nid).and_then(|n| n.parent);
        }
        false
    }

    /// This app's document identity (see `DomDocument::doc_key`), or 0 before
    /// mount. Used to scope thread-local registries (bounds signals, pending
    /// focus requests) to this document (issue #134).
    pub(crate) fn doc_key(&self) -> u64 {
        self.doc.as_ref().map(|d| d.borrow().doc_key()).unwrap_or(0)
    }

    /// Programmatically focus an element by node ID (`request_focus` /
    /// `NodeHandle::focus()` land here).
    ///
    /// For an input (`data-oninput`), sets up the full input focus state
    /// (handler ID, editable state, cursor) — the programmatic equivalent of
    /// clicking it. For a generic `tabindex >= 0` node, takes Node focus
    /// through the arbiter (issue #228). Programmatic focus is not
    /// keyboard-driven, so it does not set the `:focus-visible` ring.
    pub(crate) fn try_focus_input(&mut self, node_id: usize) {
        let Some(doc) = &self.doc else { return };
        let d = doc.borrow();
        let Some(node) = d.tree.get(node_id) else {
            return;
        };

        let Some(oninput_str) = node.attributes.get("data-oninput") else {
            // Any parseable tabindex makes a node programmatically focusable —
            // including negative ones: `tabindex="-1"` is the standard
            // focusable-but-not-tabbable idiom (`element.focus()` into a
            // just-opened dialog), and only the Tab collector excludes it.
            let focusable = node
                .attributes
                .get("tabindex")
                .and_then(|v| v.parse::<i32>().ok())
                .is_some();
            drop(d);
            if focusable {
                self.set_focus_target(FocusTarget::Node(node_id));
                if let Some(doc) = &self.doc {
                    doc.borrow_mut().update_focus(Some(node_id));
                }
                self.scene_dirty = true;
            }
            return;
        };
        let Ok(handler_id) = oninput_str.parse::<usize>() else {
            return;
        };
        let value = node.attributes.get("value").cloned().unwrap_or_default();
        drop(d);

        // Take input focus through the arbiter (tears down a prior surface / CE /
        // editor / different input; re-focusing the same input is a no-op). The
        // blurred input's change commit is deferred until this input's state is
        // installed below — the handler is user code and may rewrite the very
        // input being focused (#244 review).
        let (focus_changed, commit) = self.set_focus_target_deferred(FocusTarget::Input(node_id));

        // Move DOM `:focus` too, like the generic-node branch above — the
        // programmatic path has no mousedown to do it, and the Node teardown
        // may just have blurred the previous holder, so skipping this would
        // leave the newly focused input matching no `:focus` CSS at all.
        if let Some(doc) = &self.doc {
            doc.borrow_mut().update_focus(Some(node_id));
        }

        // Same as the click path: re-focusing the already-focused input must not
        // let a pending programmatic write pass for a user edit (issue #238).
        self.adopt_focused_input_value_from_dom();
        self.focused_input_handler_id = Some(handler_id);
        self.focused_input_value = value.clone();
        self.focused_input_node_id = Some(node_id);
        if focus_changed {
            // A fresh gesture: snapshot the commit baseline (issue #226) —
            // re-focusing the already-focused input continues its gesture.
            self.focused_input_baseline = value.clone();
        }

        // Create EditableState with cursor at end
        let mut state = EditableState::new(StringDocument::with_text(&value));
        state.selection = Selection::cursor(value.len());
        self.focused_input_state = Some(state);
        self.sync_input_cursor_to_dom();
        self.scene_dirty = true;

        // Installation complete: fire the blurred input's commit, then adopt
        // any rewrite its handler made to this input (a no-op when the DOM
        // value already matches).
        let commit_fired = commit.is_some();
        Self::fire_input_commit(commit);
        if commit_fired {
            self.adopt_focused_input_value_from_dom();
            self.focused_input_baseline = self.focused_input_value.clone();
        }
    }

    /// Adopt a programmatic `value` write to the focused `<input>` into the
    /// editable state (issue #238).
    ///
    /// Every write that is not the runtime's own — a `value_fn` effect, a
    /// normalizing `oninput`, a swatch pick, a timer — goes through the
    /// `DomDocument` trait and lands only on the attribute paint reads. The
    /// keystrokes edit `focused_input_state`, so without this the next key
    /// edits stale text and `sync_input_cursor_to_dom` paints it back over
    /// the write. Called after every edit command's dispatch, at the IME
    /// commit/cancel, at the commit boundaries that used to rebuild the
    /// state, and once per frame resolve; a string compare when nothing
    /// changed.
    ///
    /// The text is spliced in place (`EditableState::adopt_text`) — never
    /// rebuilt — so the undo stack survives, and the selection is mapped
    /// through the rewrite so the caret keeps its logical place. While an IME
    /// composition is in flight the write is deferred (moving the caret under
    /// the composition would corrupt it) and applied when it ends.
    ///
    /// The `data-onchange` baseline (issue #226) follows the browser's dirty
    /// flag: a write that lands before any user edit in this gesture moves the
    /// baseline with the value, so a purely programmatic change never commits;
    /// a write after a user edit leaves the baseline, so the gesture still
    /// commits — with the rewritten text.
    pub(crate) fn adopt_focused_input_value_from_dom(&mut self) {
        let FocusTarget::Input(node_id) = self.focus_target else {
            return;
        };
        let Some(doc) = &self.doc else { return };
        // Compare before cloning: this runs once per frame for as long as any
        // field holds focus, and the common case is "nothing was written".
        let dom_value = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(node_id) else {
                return;
            };
            let dom = node
                .attributes
                .get("value")
                .map(String::as_str)
                .unwrap_or("");
            if dom == self.focused_input_value && self.focused_input_deferred_value.is_none() {
                return;
            }
            dom.to_string()
        };
        if self.focused_input_preedit.is_some() {
            // Composition in flight: hold the write; the latest one wins.
            if dom_value != self.focused_input_value {
                self.focused_input_deferred_value = Some(dom_value);
                // Hold the write back from the DOM as well, not just from the
                // engine. The painter splices the preedit into the `value`
                // attribute at the `data-cursor-pos` offset, and those caret
                // attributes still index the *engine* text — a `value` the
                // offsets don't index draws the composition in the wrong place
                // and, when the offset lands inside a multi-byte char, panics
                // the slice. Restoring the engine text keeps the node coherent
                // until the composition ends; this also matches the web
                // backend, where a deferred write never reaches `.value`.
                self.sync_input_cursor_to_dom();
            }
            // NOTE: an app write that happens to equal the engine text cannot be
            // told apart from the restore above, so it does not cancel an
            // outstanding deferral. Every write that actually differs does
            // replace it, so "the latest write wins" holds for all of them.
            return;
        }
        let value = self
            .focused_input_deferred_value
            .take()
            .unwrap_or(dom_value);
        if value == self.focused_input_value {
            return;
        }
        let Some(state) = self.focused_input_state.as_mut() else {
            return;
        };
        state.adopt_text(&value);
        if self.focused_input_value == self.focused_input_baseline {
            self.focused_input_baseline = value.clone();
        }
        self.focused_input_value = value;
        self.sync_input_cursor_to_dom();
        self.scene_dirty = true;
    }

    /// Set the text rendering scale factor (for HiDPI / mobile).
    /// Parley will rasterize glyphs at this scale so they're crisp at physical resolution.
    /// Call before `mount_component` so initial layout uses the correct scale.
    pub fn set_text_scale(&mut self, scale: f32) {
        self.text_scale = scale;
        if let Some(doc) = &self.doc {
            doc.borrow_mut().tree.text_scale = scale;
        }
    }

    // ── Embed API helpers ─────────────────────────────────────────────
    // `has_focused_input` / `has_focused_contenteditable` now live in `focus.rs`
    // (repointed at `FocusTarget`, design A9/A10).

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
    /// Find all DOM nodes with `data-render-surface` and return their surface
    /// IDs and layout rects. This is independent of the surface registry —
    /// surfaces that are still in the DOM get layout updates even if the
    /// registry was temporarily cleared by a reactive scope rebuild.
    pub fn all_surface_layout_rects(&self) -> Vec<(usize, ViewportRect)> {
        let doc = match self.doc.as_ref() {
            Some(d) => d,
            None => return Vec::new(),
        };
        let d = doc.borrow();
        let mut results = Vec::new();
        for (node_id, node) in &d.tree.nodes {
            if node.parent.is_none() {
                continue;
            }
            if let Some(id_str) = node.attributes.get("data-render-surface") {
                let surface_id: usize = match id_str.parse() {
                    Ok(id) => id,
                    Err(_) => continue,
                };
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
                results.push((
                    surface_id,
                    (abs_x, abs_y, node.layout.width, node.layout.height),
                ));
            }
        }
        results
    }

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
                d.tree.paint_dirty_nodes.push(node_id);
                d.tree.dirty_nodes.insert(node_id);
            }
        }
    }
}

#[cfg(test)]
mod viewport_relayout_tests {
    use super::*;
    use std::cell::Cell;

    /// A 1.25x display: winit reports a 575x780 physical surface, which is a
    /// 460x624 CSS-pixel viewport.
    const PHYSICAL: (u32, u32) = (575, 780);
    const SCALE: f64 = 1.25;
    const LOGICAL: (f32, f32) = (460.0, 624.0);

    /// Mount a full-viewport element and hand back the app plus its node id.
    /// A `width/height: 100%` box resolves against the viewport, so its layout
    /// box *is* the viewport the document currently believes in.
    fn mount_at_logical() -> (RinchApp, usize) {
        let captured: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let captured_in = captured.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 100%; height: 100%");
            captured_in.set(Some(root.node_id().0));
            root
        });
        // What the shell does at startup after #246: mount at the *logical* size.
        app.mount_component(LOGICAL.0, LOGICAL.1);
        let id = captured.get().expect("node id captured at mount");
        (app, id)
    }

    fn viewport_of(app: &RinchApp, id: usize) -> (f32, f32) {
        let doc = app.doc.as_ref().expect("document");
        let d = doc.borrow();
        let l = d.tree.get(id).expect("root node").layout;
        (l.width, l.height)
    }

    /// Mark the tree dirty the way an Effect writing to the DOM does, so the
    /// relayout branches below are actually reached (they all short-circuit on
    /// a clean tree).
    fn dirty(app: &RinchApp) {
        let doc = app.doc.as_ref().expect("document");
        let mut d = doc.borrow_mut();
        let body = d.tree.body_id;
        d.tree.dirty_nodes.insert(body);
    }

    /// #246 review finding: the shell mounted at the logical viewport, but every
    /// relayout driven from `handle_event` was handed the **physical** surface
    /// size instead — and `window_size` is physical by contract, because
    /// `handle_event` divides it itself for `ClickContext`.
    ///
    /// So the page was laid out correctly exactly once. The first `AboutToWait`
    /// (which the runtime fires on *every* event-loop iteration) or `ReRender`
    /// after any DOM change re-resolved it at 575 CSS px, and the paint preamble
    /// then skipped its own re-resolve because that relayout had already drained
    /// the dirty set — so the frame painted the oversized layout. The three
    /// `to_logical` unit tests could not see this: they test the conversion, not
    /// which viewport reaches `resolve_layout`.
    ///
    /// Every event here is driven with the physical size, exactly as the shell
    /// passes it.
    #[test]
    fn a_relayout_after_mount_keeps_the_logical_viewport() {
        let (mut app, id) = mount_at_logical();
        assert_eq!(
            viewport_of(&app, id),
            LOGICAL,
            "mount must lay out at the logical viewport"
        );

        for event in [
            PlatformEvent::AboutToWait,
            PlatformEvent::UserEvent(UserEvent::ReRender),
        ] {
            dirty(&app);
            app.handle_event(event.clone(), PHYSICAL, SCALE);
            assert_eq!(
                viewport_of(&app, id),
                LOGICAL,
                "{event:?} re-laid the document out at the physical surface size \
                 instead of the logical viewport"
            );
        }
    }

    /// The same guarantee for the resize path: `PlatformEvent::Resized` carries
    /// the logical viewport, while `window_size` alongside it stays physical.
    #[test]
    fn a_resize_event_lays_out_at_the_logical_viewport() {
        let (mut app, id) = mount_at_logical();
        // The window grew to a 720x900 physical surface at the same 1.25x.
        let grown_physical = (720u32, 900u32);
        let (lw, lh) = rinch_platform::to_logical(grown_physical, SCALE);
        app.handle_event(
            PlatformEvent::Resized {
                width: lw,
                height: lh,
            },
            grown_physical,
            SCALE,
        );
        assert_eq!(
            viewport_of(&app, id),
            (lw as f32, lh as f32),
            "a resize must land on the logical viewport, not the 720x900 surface"
        );

        // And it must still be logical after the next relayout — the regression
        // above, but from a resized starting point.
        dirty(&app);
        app.handle_event(PlatformEvent::AboutToWait, grown_physical, SCALE);
        assert_eq!(
            viewport_of(&app, id),
            (lw as f32, lh as f32),
            "the relayout after a resize reverted to the physical surface size"
        );
    }
}

#[cfg(test)]
mod layout_notification_tests {
    use super::*;
    use std::cell::Cell;

    /// #145: a pure window resize must refresh bounds signals. `resize_layout`
    /// drains the dirty set, so the subsequent paint never reaches
    /// `resolve_and_repaint` — the resize path has to refresh the signals
    /// itself or a full-size element keeps reporting pre-resize geometry.
    #[test]
    fn resize_layout_refreshes_bounds_signals() {
        use rinch_core::reactive::{ElementBounds, Signal};

        let captured: Rc<Cell<Option<Signal<ElementBounds>>>> = Rc::new(Cell::new(None));
        let captured_in = captured.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 100%; height: 100%");
            captured_in.set(Some(root.bounds_signal()));
            root
        });
        app.mount_component(800.0, 600.0);
        let bounds = captured.get().expect("bounds signal captured at mount");

        // Pure resize, no other interaction: the signal must report the new
        // viewport size.
        app.resize_layout(400, 300);
        let b = bounds.get();
        assert_eq!(
            (b.width, b.height),
            (400.0, 300.0),
            "bounds signal must track the resized viewport, got {b:?}"
        );

        // And again — a second resize keeps tracking.
        app.resize_layout(640, 480);
        let b = bounds.get();
        assert_eq!((b.width, b.height), (640.0, 480.0));
    }

    /// #144: a layout-time scroll clamp must fire the container's
    /// `data-onscroll` handler — exactly once, with the clamped offset —
    /// through the deferred queue drained after layout.
    #[test]
    fn layout_scroll_clamp_fires_onscroll_handler() {
        use rinch_core::events::{ScrollCallback, register_scroll_handler};

        let fired: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let fired_in = fired.clone();
        let handler_id = register_scroll_handler(ScrollCallback::from(move |top: f64| {
            fired_in.borrow_mut().push(top);
        }));

        let nodes: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::new(RefCell::new(None));
        let nodes_in = nodes.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let container = scope.create_element("div");
            container.set_attribute("style", "height: 100px; overflow-y: auto");
            container.set_attribute("data-onscroll", &handler_id.0.to_string());
            let content = scope.create_element("div");
            content.set_attribute("style", "height: 500px");
            container.append_child(&content);
            *nodes_in.borrow_mut() = Some((container.clone(), content));
            container
        });
        app.mount_component(800.0, 600.0);
        let (container, content) = nodes.borrow().clone().expect("nodes captured at mount");

        // Scroll to max (500 content - 100 visible = 400) — a valid offset,
        // so no clamp event fires.
        if let Some(doc) = &app.doc {
            doc.borrow_mut().set_scroll_top(container.node_id(), 400.0);
        }
        app.resolve_and_repaint(800.0, 600.0);
        assert!(
            fired.borrow().is_empty(),
            "no clamp event while the offset is valid"
        );

        // Shrink the content: layout clamps 400 → 150 and must notify once.
        content.set_attribute("style", "height: 250px");
        app.resolve_and_repaint(800.0, 600.0);
        assert_eq!(*fired.borrow(), vec![150.0]);

        // Settled — a further frame must not re-fire.
        app.resolve_and_repaint(800.0, 600.0);
        assert_eq!(
            fired.borrow().len(),
            1,
            "clamp event must fire exactly once"
        );
    }
}

#[cfg(test)]
mod tab_focus_tests {
    use super::*;
    use rinch_core::events::{InputCallback, register_input_handler};
    use std::cell::Cell;

    /// Mount an `<input>` followed by a `tabindex="0"` div carrying a live
    /// click handler. Returns the app, both node ids, and the click counter.
    fn mount_input_and_div() -> (RinchApp, usize, usize, Rc<Cell<usize>>) {
        let clicks: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let clicks_in = clicks.clone();
        let oninput_id = register_input_handler(InputCallback::new(|_| {}));
        let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let ids_in = ids.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let input = scope.create_element("input");
            input.set_attribute("style", "width: 200px; height: 30px");
            input.set_attribute("data-oninput", &oninput_id.0.to_string());
            let div = scope.create_element("div");
            div.set_attribute("style", "width: 200px; height: 40px");
            div.set_attribute("tabindex", "0");
            let rid = scope.register_handler({
                let clicks = clicks_in.clone();
                move || clicks.set(clicks.get() + 1)
            });
            div.set_attribute("data-rid", &rid.0.to_string());
            root.append_child(&input);
            root.append_child(&div);
            ids_in.set(Some((input.node_id().0, div.node_id().0)));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let (input_id, div_id) = ids.get().expect("node ids captured at mount");
        (app, input_id, div_id, clicks)
    }

    fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>, shift: bool) {
        app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: text.map(str::to_string),
                modifiers: Modifiers {
                    shift,
                    ..Default::default()
                },
            },
            (800, 600),
            1.0,
        );
    }

    fn click(app: &mut RinchApp, x: f32, y: f32) {
        app.handle_event(
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            },
            (800, 600),
            1.0,
        );
        app.handle_event(
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            },
            (800, 600),
            1.0,
        );
    }

    /// `(is_focused, is_focus_visible)` for a node.
    fn focus_bits(app: &RinchApp, id: usize) -> (bool, bool) {
        let d = app.doc.as_ref().unwrap().borrow();
        let n = d.tree.get(id).unwrap();
        (n.is_focused, n.is_focus_visible)
    }

    fn abs_center(app: &RinchApp, id: usize) -> (f32, f32) {
        let d = app.doc.as_ref().unwrap().borrow();
        let n = d.tree.get(id).unwrap();
        // The same walk the click/hit paths use (scroll offsets included), so
        // these clicks stay on target if a fixture ever gains a scroller.
        let (ax, ay) = RinchApp::compute_absolute_position(&d.tree, id);
        (ax + n.layout.width / 2.0, ay + n.layout.height / 2.0)
    }

    /// #228, the trap itself: Tab must advance past a `tabindex="0"` node in
    /// both directions, tearing each owner down as it goes. Before the fix the
    /// second Tab recomputed the same target forever while the input kept
    /// focus, keys, and IME.
    #[test]
    fn tab_advances_past_a_tabindex_node() {
        let (mut app, input_id, div_id, _clicks) = mount_input_and_div();

        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(
            app.focused_input_node_id,
            Some(input_id),
            "first Tab focuses the input"
        );

        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(div_id),
            "second Tab must move to the tabindex div (pre-#228 it stuck on the input)"
        );
        assert_eq!(
            app.focused_input_node_id, None,
            "the input's focus state is torn down"
        );
        assert_eq!(focus_bits(&app, input_id), (false, false));
        assert_eq!(
            focus_bits(&app, div_id),
            (true, true),
            "the div holds :focus and :focus-visible"
        );

        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(
            app.focused_input_node_id,
            Some(input_id),
            "third Tab wraps back to the input — the Node teardown ran"
        );
        assert_eq!(focus_bits(&app, div_id), (false, false));
        assert_eq!(focus_bits(&app, input_id), (true, true));

        key(&mut app, KeyCode::Tab, None, true);
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(div_id),
            "Shift+Tab moves backwards onto the div"
        );
    }

    /// Enter and Space on a keyboard-focused generic node each dispatch its
    /// click handler exactly once.
    #[test]
    fn enter_and_space_activate_the_focused_node() {
        let (mut app, _input_id, div_id, clicks) = mount_input_and_div();
        key(&mut app, KeyCode::Tab, None, false);
        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(app.focus_target, FocusTarget::Node(div_id));

        key(&mut app, KeyCode::Enter, None, false);
        assert_eq!(clicks.get(), 1, "Enter dispatches the click handler once");

        key(&mut app, KeyCode::Space, Some(" "), false);
        assert_eq!(clicks.get(), 2, "Space dispatches the click handler once");
    }

    /// A held key auto-repeats KeyDown with no repeat flag: activation must
    /// latch until the matching KeyUp — one physical press, one activation
    /// (a held Space on the web activates exactly once).
    #[test]
    fn held_key_activates_once_per_physical_press() {
        let (mut app, _input_id, div_id, clicks) = mount_input_and_div();
        key(&mut app, KeyCode::Tab, None, false);
        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(app.focus_target, FocusTarget::Node(div_id));

        key(&mut app, KeyCode::Enter, None, false);
        key(&mut app, KeyCode::Enter, None, false); // OS auto-repeat
        key(&mut app, KeyCode::Enter, None, false); // OS auto-repeat
        assert_eq!(clicks.get(), 1, "auto-repeat must not re-activate");

        app.handle_event(
            PlatformEvent::KeyUp {
                key: KeyCode::Enter,
                modifiers: Modifiers::default(),
            },
            (800, 600),
            1.0,
        );
        key(&mut app, KeyCode::Enter, None, false);
        assert_eq!(clicks.get(), 2, "a fresh press after KeyUp activates again");
    }

    /// A mousedown outside the focused node releases the arbiter claim right
    /// away — even for presses that never reach `handle_click` (empty space,
    /// draggables, scrollbars) — so an invisible claim can't keep swallowing
    /// Enter.
    #[test]
    fn mousedown_outside_releases_the_node_claim() {
        let (mut app, _input_id, div_id, clicks) = mount_input_and_div();
        key(&mut app, KeyCode::Tab, None, false);
        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(app.focus_target, FocusTarget::Node(div_id));

        // Far below the content: hits empty space (or at most the root), not
        // the div's subtree.
        click(&mut app, 700.0, 500.0);
        assert_eq!(
            app.focus_target,
            FocusTarget::None,
            "an outside press must release the Node claim"
        );
        key(&mut app, KeyCode::Enter, None, false);
        assert_eq!(clicks.get(), 0, "the blurred node must not activate");
    }

    /// A focused node with no live `data-rid` in its ancestor chain: Enter is
    /// a quiet no-op (no panic) and Tab keeps moving.
    #[test]
    fn node_without_handler_neither_panics_nor_swallows_tab() {
        let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let ids_in = ids.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let a = scope.create_element("div");
            a.set_attribute("style", "width: 100px; height: 40px");
            a.set_attribute("tabindex", "0");
            let b = scope.create_element("div");
            b.set_attribute("style", "width: 100px; height: 40px");
            b.set_attribute("tabindex", "0");
            root.append_child(&a);
            root.append_child(&b);
            ids_in.set(Some((a.node_id().0, b.node_id().0)));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let (a_id, b_id) = ids.get().unwrap();

        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(app.focus_target, FocusTarget::Node(a_id));
        key(&mut app, KeyCode::Enter, None, false);
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(a_id),
            "Enter is a no-op"
        );
        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(b_id),
            "Tab still advances"
        );
    }

    /// Pointer interaction drops the keyboard focus ring; a click inside the
    /// focused node keeps its focus (and still dispatches the click handler).
    #[test]
    fn pointer_click_clears_the_ring_but_keeps_focus() {
        let (mut app, _input_id, div_id, clicks) = mount_input_and_div();
        key(&mut app, KeyCode::Tab, None, false);
        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(focus_bits(&app, div_id), (true, true));

        let (cx, cy) = abs_center(&app, div_id);
        click(&mut app, cx, cy);
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(div_id),
            "a click inside the focused node keeps its focus"
        );
        assert_eq!(
            focus_bits(&app, div_id),
            (true, false),
            "pointer interaction clears :focus-visible but not :focus"
        );
        assert_eq!(clicks.get(), 1, "the click itself still dispatches");
    }

    /// `request_focus` / `NodeHandle::focus()` on a tabindex node takes Node
    /// focus (it was a silent no-op before #228). Programmatic focus is not
    /// keyboard-driven, so no focus ring.
    #[test]
    fn request_focus_takes_node_focus_programmatically() {
        let (mut app, _input_id, div_id, _clicks) = mount_input_and_div();
        app.try_focus_input(div_id);
        assert_eq!(app.focus_target, FocusTarget::Node(div_id));
        assert_eq!(focus_bits(&app, div_id), (true, false));
    }

    /// Tab after a pointer click anchors at the clicked position: the click
    /// focuses the deepest hit element, and Tab resolves upward to its nearest
    /// focusable ancestor rather than restarting from the top.
    #[test]
    fn tab_after_click_anchors_at_the_clicked_position() {
        let oninput_id = register_input_handler(InputCallback::new(|_| {}));
        let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let ids_in = ids.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let input = scope.create_element("input");
            input.set_attribute("style", "width: 200px; height: 30px");
            input.set_attribute("data-oninput", &oninput_id.0.to_string());
            let mid = scope.create_element("div");
            mid.set_attribute("style", "width: 200px; height: 40px");
            mid.set_attribute("tabindex", "0");
            let last = scope.create_element("div");
            last.set_attribute("style", "width: 200px; height: 40px");
            last.set_attribute("tabindex", "0");
            root.append_child(&input);
            root.append_child(&mid);
            root.append_child(&last);
            ids_in.set(Some((mid.node_id().0, last.node_id().0)));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let (mid_id, last_id) = ids.get().unwrap();

        let (cx, cy) = abs_center(&app, mid_id);
        click(&mut app, cx, cy);
        assert_eq!(
            app.focus_target,
            FocusTarget::None,
            "a pointer click does not claim arbiter Node focus"
        );

        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(last_id),
            "Tab continues from the clicked node, not from the top"
        );
    }

    /// `data-disabled="true"` / `tabindex="-1"` remove only the node from the
    /// Tab order, not its subtree (web semantics).
    #[test]
    fn disabled_and_negative_tabindex_skip_only_the_node() {
        let ids: Rc<Cell<Option<[usize; 4]>>> = Rc::new(Cell::new(None));
        let ids_in = ids.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let disabled_wrap = scope.create_element("div");
            disabled_wrap.set_attribute("style", "width: 200px; height: 60px");
            disabled_wrap.set_attribute("tabindex", "0");
            disabled_wrap.set_attribute("data-disabled", "true");
            let child_a = scope.create_element("div");
            child_a.set_attribute("style", "width: 100px; height: 40px");
            child_a.set_attribute("tabindex", "0");
            disabled_wrap.append_child(&child_a);
            let neg_wrap = scope.create_element("div");
            neg_wrap.set_attribute("style", "width: 200px; height: 60px");
            neg_wrap.set_attribute("tabindex", "-1");
            let child_b = scope.create_element("div");
            child_b.set_attribute("style", "width: 100px; height: 40px");
            child_b.set_attribute("tabindex", "0");
            neg_wrap.append_child(&child_b);
            root.append_child(&disabled_wrap);
            root.append_child(&neg_wrap);
            ids_in.set(Some([
                disabled_wrap.node_id().0,
                child_a.node_id().0,
                neg_wrap.node_id().0,
                child_b.node_id().0,
            ]));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let [disabled_wrap, child_a, neg_wrap, child_b] = ids.get().unwrap();

        let focusable = app.collect_focusable_nodes();
        assert!(
            !focusable.contains(&disabled_wrap),
            "a disabled node is not tabbable"
        );
        assert!(
            !focusable.contains(&neg_wrap),
            "a tabindex=\"-1\" node is not tabbable"
        );
        assert!(
            focusable.contains(&child_a),
            "children of a disabled node stay tabbable"
        );
        assert!(
            focusable.contains(&child_b),
            "children of a tabindex=\"-1\" node stay tabbable"
        );
    }
}
