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
#[cfg(test)]
mod focus_lifecycle_tests;
pub(crate) mod hit_testing;
#[cfg(test)]
mod input_commit_tests;
#[cfg(test)]
mod input_ime_tests;
#[cfg(test)]
mod node_ime_tests;
mod select_widget;
mod text_selection;
#[cfg(test)]
mod textarea_newline_tests;

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

// ── Scrollbars ───────────────────────────────────────────────────────────────

/// Which of a scroll container's two scrollbars an interaction is about.
///
/// One enum rather than a vertical and a horizontal copy of everything: the
/// thumb arithmetic is identical once you name the axis, and a single pointer
/// can only ever be dragging one bar, which a pair of parallel `Option` fields
/// would let the type system forget (#178).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ScrollAxis {
    /// The bar down the right-hand edge, moving `scroll_offset.1`.
    Vertical,
    /// The bar along the bottom edge, moving `scroll_offset.0`.
    Horizontal,
}

impl ScrollAxis {
    /// The component of a pointer position that moves this bar's thumb.
    pub(crate) fn along(self, x: f32, y: f32) -> f32 {
        match self {
            ScrollAxis::Vertical => y,
            ScrollAxis::Horizontal => x,
        }
    }
}

/// State for an active scrollbar drag operation.
///
/// Axis-generic: `start_pos`, `content_size` and `container_size` are all read
/// along [`ScrollbarDrag::axis`].
pub(crate) struct ScrollbarDrag {
    /// The node ID of the scroll container being scrolled.
    pub node_id: usize,
    /// Which bar is being dragged.
    pub axis: ScrollAxis,
    /// The pointer coordinate along `axis` where the drag started (screen
    /// pixels — see the coordinate-space note on [`ScrollbarHit`]).
    pub start_pos: f32,
    /// The scroll offset along `axis` when the drag started.
    pub start_scroll: f64,
    /// Content extent along `axis` (for ratio calculation).
    pub content_size: f64,
    /// Container extent along `axis`.
    pub container_size: f64,
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
    /// Parley layout context for hit testing, paired with `hit_test_font_cx`
    /// and reused across clicks. Caret placement rebuilds a field's text layout
    /// to ask which character a click landed on; a `LayoutContext::new()` per
    /// click would throw away the shaping caches every time. Not the document's
    /// own `layout_cx` — reaching that would mean holding the document's
    /// `RefCell` mutably right in the middle of click dispatch.
    pub(crate) hit_test_layout_cx: parley::LayoutContext<peniko::Brush>,
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
    /// The framebuffer rect the drag ghost occupied in the frame currently on
    /// screen, in device pixels — `None` when the last frame drew no ghost.
    ///
    /// The ghost is blitted straight into the framebuffer after the document
    /// paint, so no DOM node owns those pixels and `compute_dirty_region` is
    /// blind to them. Without carrying the rect forward, the frame that stops
    /// drawing the ghost never clears where it used to be and it stays on
    /// screen until something else happens to dirty that area (#173).
    #[cfg(software_shell)]
    pub(crate) last_ghost_rect: Option<peniko::kurbo::Rect>,
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
    /// Whether the window currently has **OS** focus (issue #147).
    ///
    /// Separate from [`Self::focus_target`], which is *kept* across a window
    /// blur: the focused widget is notified and re-notified, not released
    /// (releasing would fire `data-onchange` on every alt-tab — a #226
    /// regression). This flag is what makes the difference observable — while
    /// it is `false` the runtime reports IME disabled, so the OS candidate
    /// window follows the window that actually has the keyboard.
    ///
    /// Starts `true`: a shell that never sends
    /// [`PlatformEvent::WindowFocus`] behaves exactly as it did before.
    pub(crate) window_focused: bool,
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
            hit_test_font_cx: rinch_dom::fonts::new_font_context(),
            hit_test_layout_cx: parley::LayoutContext::new(),
            window_props: None,
            modifiers: Modifiers::default(),
            scene_dirty: true,
            text_scale: 1.0,
            #[cfg(software_shell)]
            has_previous_frame: false,
            #[cfg(software_shell)]
            last_ghost_rect: None,
            focused_input_handler_id: None,
            focused_input_value: String::new(),
            focused_input_baseline: String::new(),
            focused_input_state: None,
            focused_input_node_id: None,
            focused_input_preedit: None,
            focused_input_deferred_value: None,
            focus_target: FocusTarget::None,
            window_focused: true,
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
        //
        // A finished image decode is the one thing that needs resolving without
        // any node being dirty: the decoding thread cannot know which nodes
        // carry that `src`, so it queues the result and leaves the matching to
        // `resolve_layout`'s `drain_pending_images`. Returning early here means
        // that drain never happens, and an `<img>` loaded into an otherwise
        // idle screen keeps the 0x0 intrinsic size it was created with — laid
        // out as nothing and painted as nothing. Found while showing a
        // rasterised PDF page from local storage, where the app is completely
        // still between the tap that starts the import and the picture that is
        // supposed to appear.
        {
            let d = doc.borrow();
            if d.tree.dirty_nodes.is_empty()
                && !d.tree.styles_dirty
                && !theme_changed
                && !rinch_dom::image_cache::has_pending(d.doc_key())
            {
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
        let mut to_fire: Vec<(usize, rinch_core::events::ScrollEvent)> = Vec::new();
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
                    // The clamp is vertical-only (`clamp_scroll_offsets` writes
                    // `.scroll_offset.1`), but the payload carries both axes, so
                    // the horizontal offset comes off the node unchanged rather
                    // than being reported as zero.
                    let scroll_left = d.tree.nodes.get(node.0).map_or(0.0, |n| n.scroll_offset.0);
                    to_fire.push((
                        handler_id,
                        rinch_core::events::ScrollEvent::new(scroll_top, scroll_left),
                    ));
                }
            }
        }
        for (handler_id, event) in to_fire {
            use rinch_core::events::{EventHandlerId, dispatch_scroll_event};
            dispatch_scroll_event(EventHandlerId(handler_id), event);
        }
    }

    /// The [`ScrollEvent`](rinch_core::events::ScrollEvent) describing where a
    /// container currently sits.
    ///
    /// Read off the node *after* every axis of a gesture has been applied, so a
    /// diagonal wheel's single event carries both new offsets rather than one
    /// new and one stale (#177).
    pub(crate) fn scroll_event_for(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
    ) -> rinch_core::events::ScrollEvent {
        let (left, top) = tree.get(node_id).map_or((0.0, 0.0), |n| n.scroll_offset);
        rinch_core::events::ScrollEvent::new(top, left)
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

            // Surface and viewport nodes are marked paint-dirty by the shell,
            // at collect time — `mark_surface_nodes_paint_dirty` /
            // `mark_viewport_nodes_paint_dirty`. It cannot be done here: the
            // frame collectors clear `needs_redraw` before this runs, so a
            // `is_surface_dirty_by_id` scan from inside `build_pixels` answers
            // "no" for every surface that just delivered a frame — and worse,
            // on the paths that call `build_pixels` with no frame map set at
            // all (the debug screenshot), marking the node only guarantees it
            // repaints *without* its pixels.

            // Compute dirty region before clearing paint_dirty_nodes
            let dirty_region = if self.has_previous_frame && !resized {
                let from_nodes = self.doc.as_ref().and_then(|doc| {
                    let d = doc.borrow();
                    rinch_dom::paint::compute_dirty_region(&d.tree, scale, w as f64, h as f64)
                });
                // Where the ghost sat in the frame on screen belongs to no DOM
                // node, so it has to be added by hand or the frame that stops
                // drawing it leaves it painted there (#173).
                Self::union_ghost_rect(from_nodes, self.last_ghost_rect)
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
            let mut ghost_rect = None;
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
                    ghost_rect = Self::ghost_overlay_rect(
                        dx,
                        dy,
                        drag.snapshot_width,
                        drag.snapshot_height,
                        w,
                        h,
                    );
                }
            }
            // Carried into the next frame's dirty region so those pixels get
            // cleared when the ghost moves on or disappears (#173).
            self.last_ghost_rect = ghost_rect;

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

    /// The framebuffer rect `blit_drag_overlay` touches for the same arguments,
    /// clipped to the surface. `None` when the ghost lands entirely off-screen.
    ///
    /// Device (physical) pixels: `dx`/`dy` are the blit's destination offsets
    /// and `src_w`/`src_h` the snapshot pixmap's size, so this is literally the
    /// span of framebuffer the blit writes — the same space `compute_dirty_region`
    /// returns and `clear_rect_*` takes. Deliberately derived from the blit
    /// arguments rather than re-projected from the cursor: the ghost's position
    /// then cannot drift from where it was actually drawn, whatever space the
    /// shell's pointer coordinates turn out to be in (#299).
    #[cfg(software_shell)]
    fn ghost_overlay_rect(
        dx: i32,
        dy: i32,
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
    ) -> Option<peniko::kurbo::Rect> {
        let x0 = dx.max(0).min(dst_w as i32);
        let y0 = dy.max(0).min(dst_h as i32);
        let x1 = (dx + src_w as i32).clamp(0, dst_w as i32);
        let y1 = (dy + src_h as i32).clamp(0, dst_h as i32);
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        Some(peniko::kurbo::Rect::new(
            x0 as f64, y0 as f64, x1 as f64, y1 as f64,
        ))
    }

    /// Fold the previous frame's ghost rect into the dirty region.
    ///
    /// `None` in stays `None` out even with a ghost pending: no dirty node
    /// means the caller takes the full-repaint path, which clears the whole
    /// framebuffer and so erases the ghost anyway. Answering the ghost's rect
    /// there would shrink an existing full repaint down to it — a different,
    /// unrelated change.
    #[cfg(software_shell)]
    fn union_ghost_rect(
        from_nodes: Option<peniko::kurbo::Rect>,
        ghost: Option<peniko::kurbo::Rect>,
    ) -> Option<peniko::kurbo::Rect> {
        match (from_nodes, ghost) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, _) => a,
        }
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
    ///
    /// A finished image decode counts, even though it dirties no node — see
    /// [`Self::has_pending_images`]. Every "is there anything to do?" gate in
    /// the workspace is one of these two predicates, so answering it here is
    /// what makes the drain reachable from *every* host (the desktop paint
    /// preamble and wake, the `AboutToWait` frame clock, the Android loop, an
    /// embedded `RinchContext::update`) instead of only from the ones that
    /// remembered to ask separately.
    pub fn has_dirty_nodes(&self) -> bool {
        self.doc
            .as_ref()
            .map(|doc| {
                let d = doc.borrow();
                !d.tree.dirty_nodes.is_empty() || d.tree.styles_dirty
            })
            .unwrap_or(false)
            || self.has_pending_images()
    }

    /// Check if there are pending layout changes that need resolving
    /// before the next paint. Covers DOM mutations, style changes,
    /// structural changes that set layout_dirty directly, and a decoded
    /// image waiting to be taken into the cache
    /// ([`Self::has_pending_images`]).
    pub fn has_pending_layout(&self) -> bool {
        self.doc
            .as_ref()
            .map(|doc| {
                let d = doc.borrow();
                !d.tree.dirty_nodes.is_empty() || d.tree.styles_dirty || d.tree.layout_dirty
            })
            .unwrap_or(false)
            || self.has_pending_images()
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
    /// Enter in a focused text control.
    ///
    /// `shift` is what separates a `<textarea>`'s two meanings for the key —
    /// see the newline block below. An `<input>` ignores it.
    fn handle_enter(&mut self, shift: bool) {
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
                    Self::node_is_textarea(&d.tree, nid),
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
        // Post-change submit resolution — see the comment above. Resolved by
        // walking **up**, like `data-onchange`: the web backend's keydown
        // delegation fires the nearest ancestor's `data-onsubmit`, so a field
        // wrapped in a submitting container has to mean the same thing here —
        // all the more now that the fallback is an insert rather than a
        // no-op. A freed id is no handler at all (issue #141): dispatching it
        // would swallow Enter, and in a `<textarea>` that means no line break
        // can be typed until the field re-renders.
        let submit_handler_id = node_id
            .and_then(|nid| {
                let doc = self.doc.as_ref()?;
                let d = doc.borrow();
                Self::input_attr_handler_up(&d.tree, nid, "data-onsubmit")
            })
            .filter(|&hid| events::has_click_handler(events::EventHandlerId(hid)));

        // A `<textarea>` is the one control where Enter has a second meaning:
        // insert a line break. Which of the two it takes follows the web
        // backend's keydown delegation (`rinch-web`'s `event_delegation`), so
        // one rsx! tree reads the same in a browser and on a phone:
        //
        //   * Shift+Enter always inserts. It is the escape hatch out of a
        //     submit, and the idiom every chat composer has taught; the web
        //     backend leaves it to the browser, which inserts.
        //   * A plain Enter submits when the author put `data-onsubmit` on the
        //     field — declaring that Enter means send — and inserts when they
        //     did not. Without this second half a `<textarea>` with no submit
        //     handler swallows the key and no line break can be typed at all.
        //
        // An `<input>` is untouched by any of it: a line break is not
        // representable in a single-line value, so Enter there stays a commit
        // (and Shift+Enter with it — leaving a modifier that does nothing at
        // all would be a regression bought for nothing).
        //
        // A composition in flight is nobody's line break: the preedit is not in
        // the document yet, and the painter splices it into `value` at
        // `data-cursor-pos`, so inserting here would move the caret out from
        // under the composition and drop the `\n` in the middle of it. The web
        // backend skips its Enter path on `isComposing` for the same reason —
        // that Enter belongs to the IME, confirming the candidate.
        let insert_newline = is_textarea
            && self.focused_input_preedit.is_none()
            && (shift || submit_handler_id.is_none());

        if insert_newline {
            // Through the ordinary edit path, so the break is one undo step,
            // carries the caret and fires `oninput` exactly as a typed
            // character does — on the web a line break *is* an input event
            // (`inputType: insertLineBreak`), not a separate kind of edit.
            self.handle_input_edit_command(EditCommand::InsertNewline);
        } else if let Some(handler_id) = submit_handler_id {
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

    /// Whether a node is disabled, and so takes no focus — not by Tab, not by
    /// a mousedown claim.
    ///
    /// `data-disabled` is a **boolean attribute**, spelled the way HTML spells
    /// one: present means disabled, whatever the value, and only the explicit
    /// `"false"` opts out. The probe this replaces demanded the literal value
    /// `"true"`, which no in-tree writer of `data-disabled` produces — the one
    /// that exists (`select_widget.rs`, for a disabled `<option>`) writes `""`,
    /// and callers following the HTML idiom write `""` too — so it matched
    /// nothing but its own test and every `data-disabled` control stayed
    /// tabbable.
    ///
    /// **Only `data-disabled`.** The plain HTML `disabled` attribute that the
    /// component library writes (`Button`, `ActionIcon`, `Checkbox`, `Radio`,
    /// `TextInput`, `Textarea`, …) is *not* consulted, so a disabled
    /// `<input>`/`<textarea>` is still a Tab stop and still typable. Teaching
    /// this probe that spelling is only half the fix — the pointer path
    /// (`found_input_focus` in `click_handling.rs`) and the editable engine
    /// would have to honour it too — so it is deliberately left alone here.
    pub(crate) fn node_is_disabled(node: &rinch_dom::Node) -> bool {
        node.attributes
            .get("data-disabled")
            .is_some_and(|v| !v.eq_ignore_ascii_case("false"))
    }

    /// A node's `tabindex` as an integer, if it carries a parseable one.
    ///
    /// The single spelling of "does this node opt into focus", shared by the
    /// Tab collector, the mousedown claim, the programmatic-focus path and the
    /// arbiter's liveness probe — they must agree, and four hand-rolled copies
    /// of `get("tabindex").and_then(parse)` could not be relied on to.
    pub(crate) fn node_tabindex(node: &rinch_dom::Node) -> Option<i32> {
        node.attributes
            .get("tabindex")
            .and_then(|v| v.parse::<i32>().ok())
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
            let skip_self = Self::node_is_disabled(node)
                || Self::node_tabindex(node).is_some_and(|v| v < 0)
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
                let has_tabindex = Self::node_tabindex(node).is_some_and(|v| v >= 0);

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

        // A blurred owner's user code (a `data-onchange` commit, a registered
        // target's `on_focus_lost`), held back until this node's DOM focus
        // state is installed below — the handler may mutate the DOM, and the
        // install must not race it (issue #147, the #244 review's rule).
        let mut pending_work = None;
        // Whether the arbiter actually *moved* onto this node. A re-focus of
        // the node that already holds the claim (Tab in a document with a
        // single focusable, a `request_focus` re-run) is not a new gain, and
        // announcing one would give a registered target a second
        // `on_focus_gained` with no `on_focus_lost` between them.
        let mut target_changed = false;
        if has_oninput {
            // `try_focus_input` takes focus through the arbiter (tears down any
            // prior surface / editor / input).
            self.try_focus_input(node_id);
        } else {
            // A generic focusable node: take focus through the arbiter too, so
            // the previous owner (an input's keys and IME included) is torn
            // down instead of lingering alongside a focus that went nowhere.
            let (changed, work) = self.set_focus_target_deferred(FocusTarget::Node(node_id));
            target_changed = changed;
            pending_work = work;
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
        // Installation complete: the blurred owner's callbacks, then the new
        // owner's. `notify_node_focus_gained` re-checks the arbiter, so an
        // `on_focus_lost` that moved focus again cannot produce a phantom gain.
        Self::fire_focus_work(pending_work);
        if claimed && target_changed {
            self.notify_node_focus_gained(node_id);
        }
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
    /// anchors Tab, or activates whatever unrelated node reused the slot.
    ///
    /// A **registered** focus target (issue #147) is excused only the
    /// *attribute* half of the probe: its unmount deregisters through the scope
    /// cleanup, which is a push notification rather than a guess, so a live
    /// registration stands in for the `tabindex` that a mid-flight re-render may
    /// momentarily not have written yet. It is **not** excused the attachment
    /// check below — a registration whose node was detached (registered outside
    /// a render scope, or removed with `remove_child` while its scope lives on)
    /// would otherwise own the keyboard forever and let Enter activate a
    /// `data-rid` in a subtree that is no longer in the document.
    ///
    /// Neither path closes the recycled-slot window (issue #304): a slot reused
    /// by another focusable node — or re-registered by a *different* widget —
    /// still passes, and that widget starts receiving `on_key` without ever
    /// having been announced an `on_focus_gained`. Closing it needs a
    /// registration identity (a generation counter), not a sharper probe.
    fn node_target_is_live(&self, node_id: usize) -> bool {
        let registered = crate::focus_registry::is_registered(self.doc_key(), node_id);
        let Some(doc) = &self.doc else { return false };
        let d = doc.borrow();
        let focusable = registered || d.tree.get(node_id).and_then(Self::node_tabindex).is_some();
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

    /// Whether a background image decode is waiting to be taken into this
    /// document's cache.
    ///
    /// A shell whose frame loop only resolves when something is dirty has to
    /// ask this: a finished decode dirties no node — see
    /// `rinch_dom::image_cache::has_pending` — so an `<img>` on an otherwise
    /// idle screen is never given its pixels, and paints as nothing.
    ///
    /// Shells do not call this directly; [`Self::has_dirty_nodes`] and
    /// [`Self::has_pending_layout`] fold it in, so every existing "is there
    /// anything to do?" gate already covers it. It stays public because a shell
    /// that wants the reason on its own can ask.
    pub fn has_pending_images(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|d| rinch_dom::image_cache::has_pending(d.borrow().doc_key()))
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
            // just-opened dialog), and only the Tab collector excludes it. A
            // disabled node takes no focus at all, programmatic included —
            // the rule `collect_focusable_nodes` and the mousedown claim
            // already apply, and what `docs/src/guide/focus.md` promises.
            let focusable = !Self::node_is_disabled(node) && Self::node_tabindex(node).is_some();
            drop(d);
            if focusable {
                // Deferred for the same reason the input path below defers its
                // commit: the blurred owner's callback is user code and must
                // not run before this node's DOM focus is installed.
                let (changed, work) = self.set_focus_target_deferred(FocusTarget::Node(node_id));
                if let Some(doc) = &self.doc {
                    doc.borrow_mut().update_focus(Some(node_id));
                }
                self.scene_dirty = true;
                Self::fire_focus_work(work);
                // Only a real transition is a gain: re-focusing the node that
                // already holds the claim must not announce a second
                // `on_focus_gained` with no `on_focus_lost` between them.
                if changed {
                    self.notify_node_focus_gained(node_id);
                }
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
        let (focus_changed, work) = self.set_focus_target_deferred(FocusTarget::Input(node_id));

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
        let commit_fired = Self::fire_focus_work(work);
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

    /// Mark every `data-viewport` node whose name is in `names` paint-dirty.
    ///
    /// The software backend paints video frames **inline** now (issue #358),
    /// which puts them under the dirty-region cache — and a node that is not in
    /// `paint_dirty_nodes` is skipped when a region is in force. A video would
    /// therefore freeze on screen the moment any *other* node dirtied a small
    /// region, and one always does: the video controls' own timestamp ticks
    /// once a second, dirtying a text run outside the video's box.
    ///
    /// The marking has to happen at *collect* time, in the shell, not inside
    /// `build_pixels`: the frame collectors clear `needs_redraw` on their way
    /// past, so by the time `build_pixels` runs there is nothing left to ask.
    /// See [`Self::mark_surface_nodes_paint_dirty`], the `RenderSurface` half
    /// of the same job.
    pub fn mark_viewport_nodes_paint_dirty(&mut self, names: &[&str]) {
        if names.is_empty() {
            return;
        }
        self.mark_attribute_nodes_paint_dirty("data-viewport", |value| names.contains(&value));
    }

    /// The `RenderSurface` half of the same problem, by `data-render-surface`
    /// id.
    ///
    /// An inline `RenderSurface` is subject to the dirty-region cache for
    /// exactly the reason an inline video is, and `build_pixels` never covered
    /// it: the block that asked `is_surface_dirty_by_id` ran *after* the frame
    /// collectors had already cleared `needs_redraw`, so it marked nothing.
    /// Surfaces only kept moving because nothing else marked anything either
    /// and `compute_dirty_region` fell back to a full repaint — the moment any
    /// other node dirties a small region (a blinking caret, a ticking clock)
    /// the surface's subtree is pruned and its last frame freezes on screen.
    /// Marking at collect time, before the flags are cleared, is what fixes it.
    pub fn mark_surface_nodes_paint_dirty(&mut self, ids: &[usize]) {
        if ids.is_empty() {
            return;
        }
        self.mark_attribute_nodes_paint_dirty("data-render-surface", |value| {
            value.parse::<usize>().is_ok_and(|id| ids.contains(&id))
        });
    }

    /// Mark every **connected** node whose `attr` value satisfies `matches`.
    ///
    /// The connectivity walk is the same one `viewport_rect_with_radius` does
    /// and for the same reason: a removed subtree can outlive its removal in
    /// the node arena, and an orphan's summed layout offsets describe a rect
    /// that is nowhere in particular. Marking one dirty would union that
    /// phantom rect into every frame's dirty region — cheap to avoid, and
    /// impossible to reason about once it happens.
    fn mark_attribute_nodes_paint_dirty(&mut self, attr: &str, matches: impl Fn(&str) -> bool) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let mut d = doc.borrow_mut();
        let dirty: Vec<_> = {
            let tree = &d.tree;
            let root = tree.root_id;
            tree.nodes
                .iter()
                .filter_map(|(node_id, node)| {
                    if !matches(node.attributes.get(attr)?.as_str()) {
                        return None;
                    }
                    let mut connected = false;
                    let mut current = Some(node_id);
                    while let Some(id) = current {
                        let n = tree.get(id)?;
                        if n.parent.is_none() {
                            connected = id == root;
                            break;
                        }
                        current = n.parent;
                    }
                    connected.then_some(node_id)
                })
                .collect()
        };
        d.tree.paint_dirty_nodes.extend(dirty);
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

/// The drag ghost's contribution to the software renderer's dirty region (#173).
///
/// The ghost is blitted into the framebuffer after the document paint, so no
/// DOM node owns its pixels and `compute_dirty_region` cannot see them. The
/// frame that stops drawing the ghost therefore has to be told, by hand, to
/// clear where it used to be.
///
/// Gated on `software_shell`, so these run under CI's `cargo test -p rinch
/// --features embed,theme,clipboard` step — NOT under `cargo test --workspace`,
/// which unifies `rinch/gpu` on from the GPU examples and turns the cfg off.
#[cfg(all(test, software_shell))]
mod drag_ghost_dirty_region_tests {
    use super::*;
    use peniko::kurbo::Rect;

    /// Every number here is a device (physical) pixel: the arguments are the
    /// blit's own destination offsets and pixmap size, so the rect is the span
    /// of framebuffer that was written, not a re-projection of the cursor.
    #[test]
    fn the_rect_is_the_span_the_blit_wrote() {
        let r = RinchApp::ghost_overlay_rect(297, 501, 239, 40, 900, 700).unwrap();
        assert_eq!(r, Rect::new(297.0, 501.0, 536.0, 541.0));
    }

    #[test]
    fn a_ghost_hanging_off_the_top_left_is_clipped_to_the_surface() {
        let r = RinchApp::ghost_overlay_rect(-30, -12, 239, 40, 900, 700).unwrap();
        assert_eq!(r, Rect::new(0.0, 0.0, 209.0, 28.0));
    }

    #[test]
    fn a_ghost_hanging_off_the_bottom_right_is_clipped_to_the_surface() {
        let r = RinchApp::ghost_overlay_rect(800, 680, 239, 40, 900, 700).unwrap();
        assert_eq!(r, Rect::new(800.0, 680.0, 900.0, 700.0));
    }

    #[test]
    fn a_ghost_entirely_off_screen_contributes_nothing() {
        assert!(RinchApp::ghost_overlay_rect(-400, 10, 239, 40, 900, 700).is_none());
        assert!(RinchApp::ghost_overlay_rect(10, 900, 239, 40, 900, 700).is_none());
        assert!(RinchApp::ghost_overlay_rect(10, 10, 0, 40, 900, 700).is_none());
    }

    /// At scale 2 the snapshot pixmap and the blit offsets are already doubled,
    /// and the framebuffer is 1800x1400 — so a ghost past the 900px logical
    /// width is nowhere near the edge and must not be clipped. Reading the rect
    /// in logical pixels would truncate it here and leave half the ghost behind.
    #[test]
    fn the_rect_is_device_pixels_not_logical() {
        let r = RinchApp::ghost_overlay_rect(1000, 800, 478, 80, 1800, 1400).unwrap();
        assert_eq!(r, Rect::new(1000.0, 800.0, 1478.0, 880.0));
    }

    /// The bug: releasing outside a drop target still dirties something small
    /// (the placeholder clearing, the source card losing its dragging style),
    /// so the frame takes the dirty-region path — with a region that misses the
    /// ghost entirely and leaves it painted.
    #[test]
    fn the_end_of_drag_region_covers_where_the_ghost_was() {
        let from_nodes = Rect::new(47.0, 295.0, 290.0, 345.0); // the To Do column
        let ghost = RinchApp::ghost_overlay_rect(297, 501, 239, 40, 900, 700).unwrap();
        assert!(
            from_nodes.intersect(ghost).is_zero_area(),
            "the fixture must reproduce the bug: a dirty region disjoint from the ghost"
        );

        let region = RinchApp::union_ghost_rect(Some(from_nodes), Some(ghost)).unwrap();
        assert!(region.contains((ghost.x0 + 0.5, ghost.y0 + 0.5)));
        assert!(region.contains((ghost.x1 - 0.5, ghost.y1 - 0.5)));
        assert!(region.contains((from_nodes.x0 + 0.5, from_nodes.y0 + 0.5)));
    }

    /// An `ondragend` that changes nothing leaves no dirty node at all, and
    /// `None` is the caller's signal to repaint everything — which clears the
    /// ghost by itself. The ghost must not turn that into a partial repaint.
    #[test]
    fn no_dirty_node_still_means_a_full_repaint() {
        let ghost = Rect::new(297.0, 501.0, 536.0, 541.0);
        assert_eq!(RinchApp::union_ghost_rect(None, Some(ghost)), None);
    }

    #[test]
    fn a_frame_with_no_ghost_leaves_the_region_alone() {
        let from_nodes = Rect::new(47.0, 295.0, 290.0, 345.0);
        assert_eq!(
            RinchApp::union_ghost_rect(Some(from_nodes), None),
            Some(from_nodes)
        );
        assert_eq!(RinchApp::union_ghost_rect(None, None), None);
    }
}

// ── #358: an inline video is subject to the dirty-region cache ───────────────

#[cfg(all(test, software_shell))]
mod video_inline_dirty_region_tests {
    use super::*;
    use rinch_dom::paint::SurfacePixelData;
    use std::cell::Cell;
    use std::collections::{HashMap, HashSet};

    const SIZE: (u32, u32) = (800, 600);

    /// A 40×10 solid frame — 4:1 against the 2:1 video box, so `contain` fits
    /// the width and the centre pixel is unambiguously frame, not letterbox.
    fn frame(rgb: [u8; 3]) -> HashMap<String, SurfacePixelData> {
        HashMap::from([(
            "v".to_string(),
            SurfacePixelData {
                data: [rgb[0], rgb[1], rgb[2], 255].repeat(40 * 10),
                width: 40,
                height: 10,
            },
        )])
    }

    const MAGENTA: [u8; 3] = [255, 0, 255];
    const CYAN: [u8; 3] = [0, 255, 255];

    fn pixel_at(app: &RinchApp, x: u32, y: u32) -> [u8; 4] {
        let p = app.skia_painter.as_ref().expect("a software painter");
        let idx = ((y * p.width() + x) * 4) as usize;
        let d = p.pixels();
        [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
    }

    fn is(p: [u8; 4], rgb: [u8; 3]) -> bool {
        p[3] == 255
            && p[0].abs_diff(rgb[0]) < 6
            && p[1].abs_diff(rgb[1]) < 6
            && p[2].abs_diff(rgb[2]) < 6
    }

    /// A 400×200 video card at the origin, plus a small box far below it
    /// standing in for the video controls' ticking timestamp — the node that
    /// produces a *small* dirty region with the video outside it.
    ///
    /// Returns the app and the label's node id.
    fn mount() -> (RinchApp, usize) {
        let label_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let captured = label_id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 800px; height: 600px;");

            let card = scope.create_element("div");
            card.set_attribute(
                "style",
                "width: 400px; height: 200px; overflow: hidden; background-color: white;",
            );
            let video = scope.create_element("div");
            video.set_attribute(
                "style",
                "width: 100%; height: 100%; background: transparent;",
            );
            video.set_attribute("data-viewport", "v");
            video.set_attribute("data-viewport-ready", "true");
            card.append_child(&video);
            root.append_child(&card);

            let label = scope.create_element("div");
            label.set_attribute(
                "style",
                "position: absolute; left: 0px; top: 400px; width: 60px; height: 20px; \
                 background-color: rgb(0, 128, 0);",
            );
            captured.set(Some(label.node_id().0));
            root.append_child(&label);

            root
        });
        app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
        let id = label_id.get().expect("label id captured at mount");
        (app, id)
    }

    /// Paint one frame with `pixels` installed, exactly the way
    /// `paint_software` does it.
    fn paint(app: &mut RinchApp, pixels: HashMap<String, SurfacePixelData>, mark_video: bool) {
        rinch_dom::paint::set_active_viewports(Some(HashSet::new()));
        app.mark_scene_dirty();
        if mark_video {
            app.mark_viewport_nodes_paint_dirty(&["v"]);
        }
        rinch_dom::paint::set_viewport_pixels(Some(pixels));
        app.build_pixels(1.0, SIZE, false);
        rinch_dom::paint::set_viewport_pixels(None);
        rinch_dom::paint::set_active_viewports(None);
    }

    /// Dirty only the label, the way a one-second timestamp tick does.
    fn dirty_the_label(app: &mut RinchApp, label: usize) {
        let doc = app.doc.as_ref().expect("document");
        let mut d = doc.borrow_mut();
        d.tree.paint_dirty_nodes.push(label);
    }

    /// Baseline: the frame reaches the pixel buffer through `build_pixels` at
    /// all — inline, with no blit anywhere in sight.
    #[test]
    fn the_first_frame_paints_inline_through_build_pixels() {
        let (mut app, _) = mount();
        paint(&mut app, frame(MAGENTA), true);
        assert!(
            is(pixel_at(&app, 200, 100), MAGENTA),
            "the video frame is painted by build_pixels, got {:?}",
            pixel_at(&app, 200, 100)
        );
    }

    /// **The trap.** Inline painting puts the video under the dirty-region
    /// cache, and the video controls tick a timestamp once a second — a small
    /// dirty region with the video's box entirely outside it. Marking the
    /// viewport node at collect time is what keeps the video moving.
    #[test]
    fn a_small_dirty_region_elsewhere_does_not_freeze_the_video() {
        let (mut app, label) = mount();
        paint(&mut app, frame(MAGENTA), true);
        assert!(is(pixel_at(&app, 200, 100), MAGENTA), "first frame landed");

        dirty_the_label(&mut app, label);
        paint(&mut app, frame(CYAN), true);

        assert!(
            is(pixel_at(&app, 200, 100), CYAN),
            "the next video frame is painted even though the only other dirty \
             node is far outside the video's box (#358), got {:?}",
            pixel_at(&app, 200, 100)
        );
    }

    /// Why that marking is load-bearing rather than belt-and-braces: without
    /// it the region is the label's alone, `paint_node` prunes the video's
    /// subtree, and the previous frame stays on screen.
    ///
    /// This pins the *mechanism* the marking exists to defeat. If dirty-region
    /// caching itself ever changes shape, this is the test that should be
    /// re-read — not silently relaxed.
    #[test]
    fn without_the_marking_the_region_excludes_the_video() {
        let (mut app, label) = mount();
        paint(&mut app, frame(MAGENTA), true);

        dirty_the_label(&mut app, label);
        paint(&mut app, frame(CYAN), false);

        assert!(
            is(pixel_at(&app, 200, 100), MAGENTA),
            "unmarked, the video's box is outside the dirty region and keeps \
             the previous frame, got {:?}",
            pixel_at(&app, 200, 100)
        );
    }

    /// The marking itself: the viewport node lands in `paint_dirty_nodes`, and
    /// a name that matches nothing marks nothing.
    #[test]
    fn marking_targets_the_named_viewport_node_only() {
        let (mut app, label) = mount();
        {
            let doc = app.doc.as_ref().expect("document");
            doc.borrow_mut().tree.paint_dirty_nodes.clear();
        }

        app.mark_viewport_nodes_paint_dirty(&["nobody"]);
        assert!(
            app.doc
                .as_ref()
                .unwrap()
                .borrow()
                .tree
                .paint_dirty_nodes
                .is_empty(),
            "a name no viewport carries marks nothing"
        );

        app.mark_viewport_nodes_paint_dirty(&["v"]);
        let d = app.doc.as_ref().unwrap().borrow();
        assert_eq!(
            d.tree.paint_dirty_nodes.len(),
            1,
            "exactly the one viewport node named"
        );
        assert_ne!(
            d.tree.paint_dirty_nodes[0], label,
            "and it is not the label"
        );
        let region = rinch_dom::paint::compute_dirty_region(&d.tree, 1.0, 800.0, 600.0)
            .expect("a dirty region");
        assert!(
            region.x0 <= 0.0 && region.x1 >= 400.0 && region.y0 <= 0.0 && region.y1 >= 200.0,
            "the region covers the whole video box: {region:?}"
        );
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
mod oncontextmenu_viewport_tests {
    use super::*;
    use std::cell::Cell;

    /// #300: `dispatch_oncontextmenu`'s caller used to divide `window_size` by
    /// `scale_factor` inline — unrounded and unguarded — instead of reusing
    /// [`RinchApp::layout_viewport`], so a right-click's `ClickContext`
    /// viewport could disagree with every other event's. Drive a real
    /// right-click through `handle_event` at a scale that does not divide
    /// evenly and check the `ClickContext` left behind agrees exactly with
    /// `layout_viewport`.
    #[test]
    fn oncontextmenu_click_context_viewport_matches_layout_viewport() {
        // 500 physical / 3.0 = 166.667 — a scale that does not divide evenly,
        // so the old unrounded `vw`/`vh` would visibly disagree with the
        // rounded, guarded `layout_viewport`.
        let physical = (500u32, 500u32);
        let scale = 3.0_f64;
        let (expected_w, expected_h) = RinchApp::layout_viewport(physical, scale);

        let handler_ran = Rc::new(Cell::new(false));
        let handler_ran_in = handler_ran.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 100%; height: 100%");
            let handler_id = events::register_handler(Rc::new(move || {
                handler_ran_in.set(true);
            }));
            root.set_attribute("data-oncontextmenu", &handler_id.0.to_string());
            root
        });
        let (lw, lh) = rinch_platform::to_logical(physical, scale);
        app.mount_component(lw as f32, lh as f32);

        app.handle_event(
            PlatformEvent::MouseDown {
                x: 10.0,
                y: 10.0,
                button: MouseButton::Right,
            },
            physical,
            scale,
        );

        assert!(
            handler_ran.get(),
            "oncontextmenu handler must fire on right-click"
        );
        let ctx = events::get_click_context();
        assert_eq!(
            (ctx.viewport_width, ctx.viewport_height),
            (expected_w, expected_h),
            "ClickContext viewport must match RinchApp::layout_viewport, not an \
             inline, unrounded window_size/scale_factor division"
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
        use rinch_core::events::{ScrollCallback, ScrollEvent, register_scroll_handler};

        let fired: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let fired_in = fired.clone();
        let handler_id = register_scroll_handler(ScrollCallback::from(move |ev: ScrollEvent| {
            let top = ev.scroll_top;
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
    ///
    /// RE-RATIFIED for #147 (maintainer decision 2, 2026-08-27). The
    /// assertions are unchanged but their reason is not: this row was the
    /// other half of #239's "a pointer press only ever *releases* a Node
    /// claim" split — a press inside the focused node was the one case that
    /// kept it. Now a press claims the nearest focusable ancestor outright, so
    /// "keeps focus" here is a re-press onto the node that already owns the
    /// claim rather than an exemption from a release. `:focus-visible` still
    /// drops, because that is about the input modality, not the claim.
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

    /// The behaviour change #147 decision 2 buys, pinned: a click-focused div
    /// **consumes** Enter/Space.
    ///
    /// Before the mousedown claim a pointer click left the arbiter at `None`,
    /// so Enter missed the `FocusTarget::Node` activation guard and fell
    /// through to `handle_enter` (the input commit / submit path). Now the
    /// click owns the keyboard and Enter activates the node — the same key,
    /// routed somewhere else.
    #[test]
    fn a_click_focused_node_consumes_enter_instead_of_falling_through() {
        let (mut app, _input_id, div_id, clicks) = mount_input_and_div();

        let (cx, cy) = abs_center(&app, div_id);
        click(&mut app, cx, cy);
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(div_id),
            "the press claims the keyboard"
        );
        assert_eq!(clicks.get(), 1, "the click itself dispatched");

        key(&mut app, KeyCode::Enter, None, false);
        assert_eq!(
            clicks.get(),
            2,
            "Enter now activates the click-focused node (pre-#147 it fell \
             through to handle_enter and the node never saw it)"
        );
    }

    /// A press on a **disabled** focusable claims nothing — the same rule that
    /// keeps it out of the Tab order, applied to the pointer.
    #[test]
    fn a_press_on_a_disabled_node_claims_nothing() {
        let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let d = scope.create_element("div");
            d.set_attribute("style", "width: 200px; height: 40px");
            d.set_attribute("tabindex", "0");
            // The spelling every in-tree writer actually uses (issue #147's
            // free row): a bare boolean attribute, not the literal "true".
            d.set_attribute("data-disabled", "");
            root.append_child(&d);
            id_in.set(Some(d.node_id().0));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let d_id = id.get().unwrap();

        let (cx, cy) = abs_center(&app, d_id);
        click(&mut app, cx, cy);
        assert_eq!(
            app.focus_target,
            FocusTarget::None,
            "a disabled node takes no focus from a pointer press"
        );
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
        // RE-RATIFIED for #147 (maintainer decision 2, 2026-08-27). This row
        // asserted `FocusTarget::None` — "a pointer click does not claim
        // arbiter Node focus" — as the intended #228/#239 split. That decision
        // was deliberately **reversed**: a mousedown now claims the nearest
        // focusable ancestor, matching the web, because without it a
        // click-focused custom control has dead Enter/Space and a registered
        // focus target hears nothing until it is reached by Tab — the reported
        // defect. Tab anchoring, what this test is actually about, is unchanged
        // either way.
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(mid_id),
            "a pointer click claims arbiter Node focus (#147 decision 2)"
        );

        key(&mut app, KeyCode::Tab, None, false);
        assert_eq!(
            app.focus_target,
            FocusTarget::Node(last_id),
            "Tab continues from the clicked node, not from the top"
        );
    }

    /// `data-disabled` / `tabindex="-1"` remove only the node from the Tab
    /// order, not its subtree (web semantics).
    ///
    /// Both spellings of the boolean attribute count (issue #147's free row):
    /// the probe used to demand the literal `"true"`, which **nothing in the
    /// tree ever writes** — the one in-tree writer (`select_widget.rs`, for a
    /// disabled `<option>`) writes `""` — so every disabled control stayed
    /// tabbable and only this test's own fixture was ever caught.
    #[test]
    fn disabled_and_negative_tabindex_skip_only_the_node() {
        let ids: Rc<Cell<Option<[usize; 5]>>> = Rc::new(Cell::new(None));
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
            // The spelling in-tree writers actually produce.
            let bare_disabled = scope.create_element("div");
            bare_disabled.set_attribute("style", "width: 200px; height: 40px");
            bare_disabled.set_attribute("tabindex", "0");
            bare_disabled.set_attribute("data-disabled", "");
            root.append_child(&disabled_wrap);
            root.append_child(&neg_wrap);
            root.append_child(&bare_disabled);
            ids_in.set(Some([
                disabled_wrap.node_id().0,
                child_a.node_id().0,
                neg_wrap.node_id().0,
                child_b.node_id().0,
                bare_disabled.node_id().0,
            ]));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let [disabled_wrap, child_a, neg_wrap, child_b, bare_disabled] = ids.get().unwrap();

        let focusable = app.collect_focusable_nodes();
        assert!(
            !focusable.contains(&disabled_wrap),
            "a disabled node is not tabbable"
        );
        assert!(
            !focusable.contains(&bare_disabled),
            "`data-disabled=\"\"` is a boolean attribute, and disables too \
             (the probe used to demand the literal \"true\" and matched nothing)"
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

#[cfg(test)]
mod popup_backdrop_hit_tests {
    //! A dropdown menu's items answer the tap that lands on them.
    //!
    //! `DropdownMenu` is two boxes over the page: the panel, and an invisible
    //! backdrop that catches the clicks that miss it. Which of the two a tap
    //! resolves to is decided by [`rinch_dom::stacking`], and the panel is only
    //! above the backdrop while the two are in the *same* stacking context —
    //! a `position: fixed` backdrop is not. Rinch hoists a fixed box to the
    //! body so it escapes every ancestor clip, and because an overflow clip
    //! *is* a stacking context here, it escapes every ancestor stacking
    //! context with it: it then outranks every non-fixed box on the page
    //! whatever the z-indexes say, panel included.
    //!
    //! So these tests mount the real component under its real stylesheet,
    //! behind an `overflow: hidden` root — the shape of every app that has a
    //! scroll container or a fixed-height shell — and tap an item.

    use super::*;
    use std::cell::Cell;

    use rinch_components::{
        DropdownMenu, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuTarget,
    };
    use rinch_core::{Callback, Component};

    const VIEWPORT: (f32, f32) = (800.0, 600.0);

    struct Menu {
        app: RinchApp,
        /// The first item's node id, so a tap can be aimed at where it is.
        item: usize,
        /// How many times the item's own `onclick` ran.
        item_clicks: Rc<Cell<usize>>,
        /// How many times the menu asked to be closed — by the backdrop, or by
        /// the item through `close_on_item_click`.
        closes: Rc<Cell<usize>>,
    }

    /// Mount an open `DropdownMenu` inside an `overflow: hidden` root.
    ///
    /// The construction order is the one `rsx!` uses and the one the component
    /// documents: the props are built first (their `Default` publishes the
    /// close signal on a thread-local), then the item children render and pick
    /// it up, then the menu itself renders. Building it by hand rather than
    /// through `rsx!` is what lets the test hold the item's node id.
    ///
    /// `backdrop_override` is an inline `style` written onto the backdrop after
    /// the component has built it, so one test can put the old
    /// `position: fixed` spelling back and show what it did.
    fn mount(backdrop_override: Option<&'static str>) -> Menu {
        let item_clicks: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let closes: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let item_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));

        let clicks_in = item_clicks.clone();
        let closes_in = closes.clone();
        let item_id_in = item_id.clone();

        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            // The app shell: a stacking context between the menu and the body,
            // which is what every scroll container and most app roots are.
            let shell = scope.create_element("div");
            shell.set_attribute(
                "style",
                "position: relative; overflow: hidden; width: 800px; height: 600px",
            );

            let closes_cb = closes_in.clone();
            let props = DropdownMenu {
                opened: true,
                on_close: Some(Callback::new(move || closes_cb.set(closes_cb.get() + 1))),
                ..Default::default()
            };

            let trigger = scope.create_element("div");
            trigger.set_attribute("style", "width: 120px; height: 32px");
            let target = DropdownMenuTarget.render(scope, &[trigger]);

            let clicks_cb = clicks_in.clone();
            let label = scope.create_text("Edit lyrics / chords…");
            let item = DropdownMenuItem {
                onclick: Some(Callback::new(move || clicks_cb.set(clicks_cb.get() + 1))),
                ..Default::default()
            }
            .render(scope, &[label]);
            item_id_in.set(Some(item.node_id().0));

            let dropdown = DropdownMenuDropdown.render(scope, &[item]);
            let menu = props.render(scope, &[target, dropdown]);

            shell.append_child(&menu);
            shell
        });

        app.mount_component(VIEWPORT.0, VIEWPORT.1);

        // The component's own stylesheet, not a copy of it: this is what makes
        // the test bite when the backdrop's `position` changes.
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.load_css(&rinch_components::generate_component_css());
            d.recompute_all_styles_full();
        }
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

        if let Some(style) = backdrop_override {
            let backdrop = find_backdrop(&app);
            {
                let doc = app.doc.as_ref().unwrap();
                let mut d = doc.borrow_mut();
                d.set_attribute(rinch_core::dom::NodeId(backdrop), "style", style);
                d.recompute_all_styles_full();
            }
            app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        }

        Menu {
            item: item_id
                .get()
                .expect("the item's node id, captured at mount"),
            app,
            item_clicks,
            closes,
        }
    }

    /// The one node carrying the backdrop's class. The component appends it
    /// last, but finding it by class says what is meant rather than relying on
    /// that.
    ///
    /// Iterated with `Slab::iter`, not `0..nodes.len()`: a slab's `len()` is
    /// its *occupied* count, not one past its highest index, so the moment
    /// anything in the tree is freed the backdrop can live past the end of
    /// that range and this would fail blaming the component.
    fn find_backdrop(app: &RinchApp) -> usize {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| {
                n.attributes
                    .get("class")
                    .is_some_and(|c| c.contains("rinch-dropdown-menu__backdrop"))
            })
            .map(|(id, _)| id)
            .expect("the menu renders a backdrop when close_on_click_outside is on")
    }

    fn centre(app: &RinchApp, node_id: usize) -> (f32, f32) {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let n = d.tree.get(node_id).expect("node still in the tree");
        let (x, y) = RinchApp::compute_absolute_position(&d.tree, node_id);
        assert!(
            n.layout.width > 0.0 && n.layout.height > 0.0,
            "node {node_id} has no box to aim at: {:?}",
            n.layout
        );
        (x + n.layout.width / 2.0, y + n.layout.height / 2.0)
    }

    /// One tap, spelled the way Android spells it: `TouchGesture` resolves a
    /// still finger's lift into a `MouseDown` and a `MouseUp` pushed into the
    /// *same* event batch, with nothing in between. On the desktop the two are
    /// separated by however long the button was held; here they are not, and a
    /// dismissal armed on one of them would beat an activation armed on the
    /// other.
    fn tap(app: &mut RinchApp, x: f32, y: f32) {
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

    /// The fault this module exists for: a tap on a menu item runs the item.
    ///
    /// It used to close the menu and do nothing else, on Android and on the
    /// desktop alike — the backdrop was above the panel, so the tap never
    /// reached the item at all and what fired was the dismissal.
    #[test]
    fn a_tap_on_a_menu_item_runs_the_item_and_not_the_dismissal() {
        let mut menu = mount(None);
        let (x, y) = centre(&menu.app, menu.item);

        tap(&mut menu.app, x, y);

        assert_eq!(
            menu.item_clicks.get(),
            1,
            "the tap landed inside the item's box and must have run its handler"
        );
        assert_eq!(
            menu.closes.get(),
            1,
            "and closed the menu once, through close_on_item_click — not twice, \
             and not by the backdrop instead"
        );
    }

    /// The other half of the backdrop's job, which the fix must not cost: a tap
    /// that misses the panel still dismisses.
    #[test]
    fn a_tap_outside_the_panel_still_dismisses_and_runs_no_item() {
        let mut menu = mount(None);
        let (ix, iy) = centre(&menu.app, menu.item);

        // Well below the panel, still inside the shell.
        tap(&mut menu.app, ix, iy + 400.0);

        assert_eq!(menu.item_clicks.get(), 0, "nothing was aimed at");
        assert_eq!(menu.closes.get(), 1, "the backdrop caught it");
    }

    /// Why the backdrop is `position: absolute`, kept as an executable
    /// statement rather than a comment: put the fixed spelling back and the
    /// menu is dead again.
    ///
    /// A fixed box is viewport-level content in Rinch — hoisted to the body out
    /// of every ancestor clip, and out of every ancestor stacking context with
    /// it. Its `z-index: 99` is then being compared against the *shell's*
    /// place at the body, never against the panel's `100`, and it wins over
    /// every non-fixed box on the page.
    #[test]
    fn a_fixed_backdrop_swallows_the_item_it_was_meant_to_sit_under() {
        let mut menu = mount(Some(
            "display: block; position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 99",
        ));
        let (x, y) = centre(&menu.app, menu.item);

        tap(&mut menu.app, x, y);

        assert_eq!(
            menu.item_clicks.get(),
            0,
            "a fixed backdrop is above the panel, so the tap never reaches the item"
        );
        assert_eq!(menu.closes.get(), 1, "what it reaches is the dismissal");
    }
}

#[cfg(test)]
mod pointer_cancel_tests {
    use super::*;
    use std::cell::Cell;

    /// A `draggable` div carrying a live click handler, which is the shape that
    /// makes the trap visible: `MouseDown` on a draggable arms a *pending* drag
    /// rather than clicking, and it is `MouseUp` that later decides between
    /// "the threshold was never crossed, so this was a click" and "this was a
    /// drag". A cancel has to reach in between those two.
    fn mount_draggable() -> (RinchApp, (f32, f32), Rc<Cell<usize>>) {
        let clicks: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let clicks_in = clicks.clone();
        let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let row = scope.create_element("div");
            row.set_attribute("style", "width: 200px; height: 40px");
            row.set_attribute("draggable", "true");
            let rid = scope.register_handler({
                let clicks = clicks_in.clone();
                move || clicks.set(clicks.get() + 1)
            });
            row.set_attribute("data-rid", &rid.0.to_string());
            root.append_child(&row);
            id_in.set(Some(row.node_id().0));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let row_id = id.get().expect("node id captured at mount");
        let centre = {
            let d = app.doc.as_ref().unwrap().borrow();
            let n = d.tree.get(row_id).unwrap();
            let (ax, ay) = RinchApp::compute_absolute_position(&d.tree, row_id);
            (ax + n.layout.width / 2.0, ay + n.layout.height / 2.0)
        };
        (app, centre, clicks)
    }

    fn send(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
        app.handle_event(event, (800, 600), 1.0)
    }

    fn press(x: f32, y: f32) -> PlatformEvent {
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        }
    }

    fn release(x: f32, y: f32) -> PlatformEvent {
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        }
    }

    /// The whole point of the event, stated as the difference between two
    /// otherwise identical gestures. Press and release is a click; press,
    /// cancel, release is nothing at all.
    ///
    /// This is the trap stage 3 walks into without it: once a moving finger
    /// emits a real `MouseDown`/`MouseUp` pair, every flick through a list ends
    /// in a click on whichever row it started on, because `MouseUp` is where
    /// `handle_click` fires. The cancel is what takes the press off the table
    /// while the finger is still moving.
    #[test]
    fn a_cancel_between_press_and_release_leaves_no_click_behind() {
        let (mut app, (x, y), clicks) = mount_draggable();

        // Control: the same two events, uninterrupted, are a click.
        send(&mut app, press(x, y));
        send(&mut app, release(x, y));
        assert_eq!(clicks.get(), 1, "precondition: press+release clicks");

        send(&mut app, press(x, y));
        assert!(
            app.pending_drag.is_some(),
            "precondition: a press on a draggable arms a pending drag"
        );

        send(&mut app, PlatformEvent::PointerCancel);
        assert!(
            app.pending_drag.is_none(),
            "the cancel drops the pending drag rather than deferring it"
        );
        assert!(
            app.doc
                .as_ref()
                .unwrap()
                .borrow()
                .tree
                .active_node
                .is_none(),
            ":active must not stick to an element that is no longer pressed"
        );

        send(&mut app, release(x, y));
        assert_eq!(
            clicks.get(),
            1,
            "the release after a cancel completes nothing"
        );
    }

    /// The gesture after a cancelled one is an ordinary gesture. A cancel is a
    /// teardown, not a mode.
    #[test]
    fn a_press_after_a_cancelled_one_clicks_normally() {
        let (mut app, (x, y), clicks) = mount_draggable();

        send(&mut app, press(x, y));
        send(&mut app, PlatformEvent::PointerCancel);
        send(&mut app, release(x, y));
        assert_eq!(clicks.get(), 0);

        send(&mut app, press(x, y));
        send(&mut app, release(x, y));
        assert_eq!(clicks.get(), 1, "the next press is just a press");
    }

    /// Most gestures start over something that is not draggable and holds
    /// nothing, and on Android every scroll now sends one of these. A cancel
    /// with nothing in flight must therefore be free — no repaint asked for, no
    /// state disturbed.
    #[test]
    fn a_cancel_with_nothing_in_flight_asks_for_no_repaint() {
        let (mut app, (x, y), clicks) = mount_draggable();

        let actions = send(&mut app, PlatformEvent::PointerCancel);
        assert!(
            actions.is_empty(),
            "nothing was released, so nothing needs redrawing, got {actions:?}"
        );

        send(&mut app, press(x, y));
        send(&mut app, release(x, y));
        assert_eq!(clicks.get(), 1, "and the document is untouched");
    }
}

#[cfg(test)]
mod wheel_scroll_dispatch_tests {
    use super::*;
    use rinch_core::events::{ScrollCallback, ScrollEvent, register_scroll_handler};

    /// A mounted scroll container and everything a test needs to poke it: its
    /// node id, a point inside it, and every `scrollTop` its `onscroll` handler
    /// has been handed.
    struct Scroller {
        app: RinchApp,
        id: usize,
        centre: (f32, f32),
        fired: Rc<RefCell<Vec<ScrollEvent>>>,
    }

    /// A scroll container with `data-onscroll` wired to a recorder, sized by the
    /// caller so one fixture covers "scrolls sideways", "scrolls down" and
    /// "scrolls both ways".
    fn mount_scroller(container_style: &str, content_style: &str) -> Scroller {
        let fired: Rc<RefCell<Vec<ScrollEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let fired_in = fired.clone();
        let handler_id = register_scroll_handler(ScrollCallback::from(move |ev: ScrollEvent| {
            fired_in.borrow_mut().push(ev);
        }));
        let container_style = container_style.to_string();
        let content_style = content_style.to_string();
        let id: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let container = scope.create_element("div");
            container.set_attribute("style", &container_style);
            container.set_attribute("data-onscroll", &handler_id.0.to_string());
            let content = scope.create_element("div");
            content.set_attribute("style", &content_style);
            container.append_child(&content);
            *id_in.borrow_mut() = Some(container.node_id().0);
            container
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let container_id = id.borrow().expect("node id captured at mount");
        let centre = {
            let d = app.doc.as_ref().unwrap().borrow();
            let n = d.tree.get(container_id).unwrap();
            let (ax, ay) = RinchApp::compute_absolute_position(&d.tree, container_id);
            (ax + n.layout.width / 2.0, ay + n.layout.height / 2.0)
        };
        Scroller {
            app,
            id: container_id,
            centre,
            fired,
        }
    }

    /// Wheel deltas are the *content's* movement, so a negative delta_x scrolls
    /// right — the same sign convention the touch recogniser emits with.
    fn wheel(app: &mut RinchApp, (x, y): (f32, f32), delta_x: f64, delta_y: f64) {
        app.handle_event(
            PlatformEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            },
            (800, 600),
            1.0,
        );
    }

    fn offsets(app: &RinchApp, id: usize) -> (f64, f64) {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.get(id).unwrap().scroll_offset
    }

    /// The gap this fixes: a container scrolled sideways moved its content and
    /// told nobody, while the identical vertical gesture fired `onscroll`. On a
    /// phone that is half of every scroll going unreported.
    #[test]
    fn a_horizontal_scroll_dispatches_its_onscroll_handler() {
        let Scroller {
            mut app,
            id,
            centre,
            fired,
        } = mount_scroller(
            "width: 100px; height: 50px; overflow-x: auto",
            "width: 500px; height: 20px",
        );

        wheel(&mut app, centre, -30.0, 0.0);

        assert_eq!(offsets(&app, id).0, 30.0, "precondition: the strip moved");
        assert_eq!(
            fired.borrow().len(),
            1,
            "the handler fires for the axis that moved"
        );
        // #177: the payload carries the axis that actually moved. Before
        // `ScrollEvent` this was a bare `scroll_top` and a horizontal-only
        // scroller fired an event that said nothing — the listener learned that
        // *something* happened and had to go back to the DOM to find out what.
        assert_eq!(
            fired.borrow()[0],
            ScrollEvent::new(0.0, 30.0),
            "scroll_left is the news; scroll_top is unchanged because this \
             container only scrolls sideways"
        );
    }

    /// A scroll that moves nothing — already hard against the edge — is not a
    /// scroll, and must not fire. The vertical half has always worked this way.
    #[test]
    fn a_horizontal_scroll_that_moves_nothing_does_not_dispatch() {
        let Scroller {
            mut app,
            id,
            centre,
            fired,
        } = mount_scroller(
            "width: 100px; height: 50px; overflow-x: auto",
            "width: 500px; height: 20px",
        );

        wheel(&mut app, centre, 30.0, 0.0);

        assert_eq!(offsets(&app, id).0, 0.0, "already at the left edge");
        assert!(fired.borrow().is_empty());
    }

    /// The behaviour that already worked, asserted so the rewrite of the
    /// dispatch bookkeeping cannot quietly drop it.
    #[test]
    fn a_vertical_scroll_still_dispatches_with_its_new_top() {
        let Scroller {
            mut app,
            id,
            centre,
            fired,
        } = mount_scroller(
            "width: 100px; height: 50px; overflow-y: auto",
            "width: 20px; height: 500px",
        );

        wheel(&mut app, centre, 0.0, -40.0);

        assert_eq!(offsets(&app, id).1, 40.0);
        // The number `virtual_list` keys off, in the field it now reads.
        assert_eq!(*fired.borrow(), vec![ScrollEvent::new(40.0, 0.0)]);
        assert_eq!(fired.borrow()[0].scroll_top, 40.0);
    }

    /// A diagonal flick moves one container on both axes. `onscroll` means "this
    /// element scrolled", not "this element scrolled vertically", so it fires
    /// once — twice would make a handler that counts, or that starts an
    /// animation, do it double on the diagonal frames of every flick.
    #[test]
    fn a_diagonal_scroll_of_one_container_dispatches_once() {
        let Scroller {
            mut app,
            id,
            centre,
            fired,
        } = mount_scroller(
            "width: 100px; height: 50px; overflow: auto",
            "width: 500px; height: 500px",
        );

        wheel(&mut app, centre, -30.0, -40.0);

        assert_eq!(
            offsets(&app, id),
            (30.0, 40.0),
            "precondition: both axes moved"
        );
        assert_eq!(
            *fired.borrow(),
            vec![ScrollEvent::new(40.0, 30.0)],
            "one event, carrying BOTH new offsets — the payload is read back \
             after both axes have been applied, so neither is stale"
        );
    }

    /// The two axes need not resolve to the same element: a sideways strip
    /// inside a vertically scrolling page is the ordinary case. Each container
    /// scrolled, so each is owed its own event, with its own top.
    #[test]
    fn a_diagonal_scroll_across_two_containers_dispatches_to_each() {
        let outer_fired: Rc<RefCell<Vec<ScrollEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let inner_fired: Rc<RefCell<Vec<ScrollEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let outer_in = outer_fired.clone();
        let inner_in = inner_fired.clone();
        let outer_handler =
            register_scroll_handler(ScrollCallback::from(move |ev: ScrollEvent| {
                outer_in.borrow_mut().push(ev);
            }));
        let inner_handler =
            register_scroll_handler(ScrollCallback::from(move |ev: ScrollEvent| {
                inner_in.borrow_mut().push(ev);
            }));
        let ids: Rc<RefCell<Option<(usize, usize)>>> = Rc::new(RefCell::new(None));
        let ids_in = ids.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let page = scope.create_element("div");
            page.set_attribute("style", "width: 300px; height: 100px; overflow-y: auto");
            page.set_attribute("data-onscroll", &outer_handler.0.to_string());
            let strip = scope.create_element("div");
            strip.set_attribute("style", "width: 100px; height: 400px; overflow-x: auto");
            strip.set_attribute("data-onscroll", &inner_handler.0.to_string());
            let wide = scope.create_element("div");
            wide.set_attribute("style", "width: 500px; height: 400px");
            strip.append_child(&wide);
            page.append_child(&strip);
            *ids_in.borrow_mut() = Some((page.node_id().0, strip.node_id().0));
            page
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let (page_id, strip_id) = ids.borrow().expect("node ids captured at mount");
        // A point inside both boxes. Not the strip's centre: the strip is 400
        // tall inside a 100-tall page, so its middle is clipped away and the
        // wheel would land on the page instead.
        let point = {
            let d = app.doc.as_ref().unwrap().borrow();
            let (ax, ay) = RinchApp::compute_absolute_position(&d.tree, strip_id);
            (ax + 50.0, ay + 25.0)
        };

        wheel(&mut app, point, -30.0, -40.0);

        assert_eq!(offsets(&app, page_id).1, 40.0);
        assert_eq!(offsets(&app, strip_id).0, 30.0);
        assert_eq!(
            *outer_fired.borrow(),
            vec![ScrollEvent::new(40.0, 0.0)],
            "the page scrolled down"
        );
        assert_eq!(
            *inner_fired.borrow(),
            vec![ScrollEvent::new(0.0, 30.0)],
            "the strip scrolled sideways, and reports its own offsets"
        );
    }

    /// `virtual_list` is the in-tree consumer that *keys off* the payload: it
    /// mounts the row range covering `scroll_top`. The `ScrollEvent` change
    /// moved that number out of the callback's only argument and into a field,
    /// so this drives the whole wire — wheel, dispatch, `ev.scroll_top`, window
    /// recompute — rather than asserting on the payload alone.
    #[test]
    fn virtual_list_still_windows_off_the_vertical_offset() {
        let list_id: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
        let list_in = list_id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let outer = scope.create_element("div");
            outer.set_attribute("style", "width: 200px; height: 100px");
            let list = rinch_core::virtual_list(
                scope,
                20.0,
                || (0..1000).collect::<Vec<i32>>(),
                |i: &i32| *i,
                0,
                |i: i32, scope: &mut RenderScope| {
                    let row = scope.create_element("div");
                    row.set_attribute("style", "height: 20px");
                    row.set_attribute("data-row", &i.to_string());
                    row
                },
            );
            *list_in.borrow_mut() = Some(list.node_id().0);
            outer.append_child(&list);
            outer
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let list = list_id.borrow().expect("list captured at mount");
        // The window div is the container's second child (spacer first).
        let win = {
            let d = app.doc.as_ref().unwrap().borrow();
            let kids = &d.tree.get(list).unwrap().children;
            assert_eq!(
                d.tree.get(kids[1]).unwrap().attributes["class"],
                "rinch-vlist__window"
            );
            kids[1]
        };

        let first_row = |app: &RinchApp| -> i32 {
            let d = app.doc.as_ref().unwrap().borrow();
            let kids = &d.tree.get(win).unwrap().children;
            assert!(!kids.is_empty(), "the window renders rows");
            d.tree.get(kids[0]).unwrap().attributes["data-row"]
                .parse()
                .unwrap()
        };
        assert_eq!(first_row(&app), 0, "precondition: windowed at the top");

        let centre = {
            let d = app.doc.as_ref().unwrap().borrow();
            let n = d.tree.get(list).unwrap();
            let (ax, ay) = RinchApp::compute_absolute_position(&d.tree, list);
            (ax + n.layout.width / 2.0, ay + n.layout.height / 2.0)
        };
        wheel(&mut app, centre, 0.0, -400.0);

        assert_eq!(offsets(&app, list).1, 400.0, "the list scrolled down");
        assert_eq!(
            first_row(&app),
            20,
            "the window re-mounted from row 400/20 — `scroll_top` reached the \
             list unchanged"
        );
    }
}

/// The desktop horizontal scrollbar (#178): hit-tested, dragged, and — with
/// both bars up — the corner that belongs to neither.
#[cfg(test)]
mod horizontal_scrollbar_tests {
    use super::hit_testing::{SCROLLBAR_HIT_THICKNESS, find_scrollbar_hit};
    use super::*;
    use rinch_core::events::{ScrollCallback, ScrollEvent, register_scroll_handler};

    struct Bars {
        app: RinchApp,
        id: usize,
        /// Absolute origin and size of the scroll container.
        rect: (f32, f32, f32, f32),
        fired: Rc<RefCell<Vec<ScrollEvent>>>,
    }

    /// A scroll container with `data-onscroll` wired to a recorder and a single
    /// child sized by the caller, so one fixture covers "overflows sideways",
    /// "overflows down" and "overflows both ways".
    fn mount(container_style: &str, content_style: &str) -> Bars {
        let fired: Rc<RefCell<Vec<ScrollEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let fired_in = fired.clone();
        let handler_id = register_scroll_handler(ScrollCallback::from(move |ev: ScrollEvent| {
            fired_in.borrow_mut().push(ev);
        }));
        let container_style = container_style.to_string();
        let content_style = content_style.to_string();
        let id: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let container = scope.create_element("div");
            container.set_attribute("style", &container_style);
            container.set_attribute("data-onscroll", &handler_id.0.to_string());
            let content = scope.create_element("div");
            content.set_attribute("style", &content_style);
            container.append_child(&content);
            *id_in.borrow_mut() = Some(container.node_id().0);
            container
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let container_id = id.borrow().expect("node id captured at mount");
        let rect = {
            let d = app.doc.as_ref().unwrap().borrow();
            let n = d.tree.get(container_id).unwrap();
            let (ax, ay) = RinchApp::compute_absolute_position(&d.tree, container_id);
            (ax, ay, n.layout.width, n.layout.height)
        };
        Bars {
            app,
            id: container_id,
            rect,
            fired,
        }
    }

    fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
        app.handle_event(
            PlatformEvent::MouseDown {
                x,
                y,
                button: rinch_platform::MouseButton::Left,
            },
            (800, 600),
            1.0,
        );
    }

    fn drag_to(app: &mut RinchApp, (x, y): (f32, f32)) {
        app.handle_event(PlatformEvent::MouseMove { x, y }, (800, 600), 1.0);
    }

    fn offsets(app: &RinchApp, id: usize) -> (f64, f64) {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.get(id).unwrap().scroll_offset
    }

    const WIDE: &str = "width: 200px; height: 100px; overflow-x: auto";
    const TALL: &str = "width: 200px; height: 100px; overflow-y: auto";
    const BOTH: &str = "width: 200px; height: 100px; overflow: auto";

    /// The gap: a container that overflows sideways had no bar to hit, so a
    /// user with an ordinary mouse could not pan it at all.
    #[test]
    fn the_bottom_edge_of_a_horizontal_scroller_hits_its_scrollbar() {
        let Bars { app, id, rect, .. } = mount(WIDE, "width: 800px; height: 40px");
        let (x, y, w, h) = rect;
        let d = app.doc.as_ref().unwrap().borrow();

        let hit = find_scrollbar_hit(&d.tree, x + w / 2.0, y + h - 2.0)
            .expect("the bottom strip is the horizontal scrollbar");
        assert_eq!(hit.node_id, id);
        assert_eq!(hit.axis, ScrollAxis::Horizontal);
        assert_eq!(hit.content_size, 800.0, "content extent along the axis");
        assert_eq!(hit.container_size, 200.0, "visible extent along the axis");

        // And nothing on the right-hand edge: this container does not scroll
        // vertically, so there is no vertical bar to grab.
        assert!(
            find_scrollbar_hit(&d.tree, x + w - 2.0, y + h / 2.0).is_none(),
            "no vertical bar on a horizontal-only scroller"
        );
    }

    /// The behaviour that already worked, pinned so the axis generalisation
    /// cannot quietly drop it.
    #[test]
    fn the_right_edge_of_a_vertical_scroller_still_hits_its_scrollbar() {
        let Bars { app, id, rect, .. } = mount(TALL, "width: 40px; height: 800px");
        let (x, y, w, h) = rect;
        let d = app.doc.as_ref().unwrap().borrow();

        let hit = find_scrollbar_hit(&d.tree, x + w - 2.0, y + h / 2.0)
            .expect("the right strip is the vertical scrollbar");
        assert_eq!(hit.node_id, id);
        assert_eq!(hit.axis, ScrollAxis::Vertical);
        assert_eq!(hit.content_size, 800.0);
        assert_eq!(hit.container_size, 100.0);
        assert!(
            find_scrollbar_hit(&d.tree, x + w / 2.0, y + h - 2.0).is_none(),
            "no horizontal bar on a vertical-only scroller"
        );
    }

    /// The corner. With both bars up their strips would overlap in a square at
    /// the bottom-right and one would silently win every click there. Neither
    /// claims it — which also matches the paint pass, where both tracks stop
    /// short so no thumb is ever drawn in a square that cannot be grabbed.
    #[test]
    fn the_corner_between_two_scrollbars_belongs_to_neither() {
        let Bars { app, rect, .. } = mount(BOTH, "width: 800px; height: 800px");
        let (x, y, w, h) = rect;
        let d = app.doc.as_ref().unwrap().borrow();
        let t = SCROLLBAR_HIT_THICKNESS;

        assert!(
            find_scrollbar_hit(&d.tree, x + w - 2.0, y + h - 2.0).is_none(),
            "the corner square hits no scrollbar"
        );

        // Both bars still exist — they have merely given up the corner. Just
        // clear of it, each strip answers for its own axis.
        let above_corner = find_scrollbar_hit(&d.tree, x + w - 2.0, y + h - t - 2.0)
            .expect("the vertical bar runs up to the corner");
        assert_eq!(above_corner.axis, ScrollAxis::Vertical);
        let left_of_corner = find_scrollbar_hit(&d.tree, x + w - t - 2.0, y + h - 2.0)
            .expect("the horizontal bar runs left of the corner");
        assert_eq!(left_of_corner.axis, ScrollAxis::Horizontal);
    }

    /// Press then drag the horizontal thumb: `scroll_offset.0` follows the
    /// pointer's *x*, and `.1` is untouched.
    #[test]
    fn dragging_the_horizontal_thumb_scrolls_sideways() {
        let Bars {
            mut app,
            id,
            rect,
            fired,
        } = mount(WIDE, "width: 800px; height: 40px");
        let (x, y, w, h) = rect;

        // Press at the left end of the track: jump-to-click lands at 0.
        press(&mut app, (x + 2.0, y + h - 2.0));
        assert_eq!(offsets(&app, id), (0.0, 0.0));

        // Drag half the track. `scroll_delta = (moved / (container - 4)) *
        // content` — the vertical bar's arithmetic, read along x.
        let moved = (w - 4.0) / 2.0;
        drag_to(&mut app, (x + 2.0 + moved, y + h - 2.0));

        let (left, top) = offsets(&app, id);
        assert!(left > 0.0, "the container scrolled right, got {left}");
        assert_eq!(top, 0.0, "the vertical offset is untouched");
        assert_eq!(
            left,
            ((w as f64 - 4.0) / 2.0 / (200.0 - 4.0)) * 800.0,
            "half the track is half the content, clamped by max_scroll"
        );

        // And the app was told, on the axis that moved (#177).
        let last = *fired.borrow().last().expect("onscroll fired");
        assert_eq!(last, ScrollEvent::new(0.0, left));
    }

    /// The vertical drag, unchanged by the generalisation.
    #[test]
    fn dragging_the_vertical_thumb_still_scrolls_down() {
        let Bars {
            mut app,
            id,
            rect,
            fired,
        } = mount(TALL, "width: 40px; height: 800px");
        let (x, y, w, h) = rect;

        press(&mut app, (x + w - 2.0, y + 2.0));
        assert_eq!(offsets(&app, id), (0.0, 0.0));

        let moved = (h - 4.0) / 2.0;
        drag_to(&mut app, (x + w - 2.0, y + 2.0 + moved));

        let (left, top) = offsets(&app, id);
        assert_eq!(left, 0.0, "the horizontal offset is untouched");
        assert_eq!(top, ((h as f64 - 4.0) / 2.0 / (100.0 - 4.0)) * 800.0);
        assert_eq!(
            *fired.borrow().last().expect("onscroll fired"),
            ScrollEvent::new(top, 0.0)
        );
    }

    /// A thumb that moves must dirty where it *was* as well as where it now is,
    /// or the software renderer's dirty-region caching leaves the old thumb
    /// painted — #173's failure mode. It does, because the thumb lives inside
    /// the container's own layout rect and every scroll marks that whole rect
    /// paint-dirty; this pins the property rather than the mechanism.
    #[test]
    fn a_moving_horizontal_thumb_dirties_its_old_rect_as_well_as_its_new_one() {
        let Bars {
            mut app, id, rect, ..
        } = mount(WIDE, "width: 800px; height: 40px");
        let (x, y, w, h) = rect;

        press(&mut app, (x + 2.0, y + h - 2.0));
        // Clear the frame's dirty bookkeeping, then move the thumb.
        app.resolve_and_repaint(800.0, 600.0);
        {
            let mut d = app.doc.as_ref().unwrap().borrow_mut();
            d.tree.paint_dirty_nodes.clear();
        }
        drag_to(&mut app, (x + 2.0 + (w - 4.0) / 2.0, y + h - 2.0));

        let d = app.doc.as_ref().unwrap().borrow();
        assert!(
            d.tree.paint_dirty_nodes.contains(&id),
            "the scroll container is paint-dirty after the thumb moved"
        );
        let region = rinch_dom::paint::compute_dirty_region(&d.tree, 1.0, 800.0, 600.0)
            .expect("a dirty region");
        // The thumb travels along the bottom edge inside the container's box,
        // so the whole strip — old position at the left end, new position mid
        // track — is inside the region.
        assert!(
            region.x0 <= x as f64 && region.x1 >= (x + w) as f64,
            "the region spans the whole track, not just the new thumb: {region:?}"
        );
        assert!(
            region.y0 <= (y + h - 8.0) as f64 && region.y1 >= (y + h) as f64,
            "and covers the bottom edge the thumb sits on: {region:?}"
        );
    }
}

/// Caret placement from a click inside a multi-line text field.
///
/// A `<textarea>` between two blocks is laid out by an inline formatting
/// context, and the anonymous block box the IFC wraps it in clones the
/// *containing block's* computed style, padding included. Paint and hit testing
/// both add that content-box offset; the caret
/// arithmetic summed the parent chain and did not, so it measured the click
/// against a box one padding higher than the one on screen and put the caret a
/// line below the finger. Invisible in an `<input>` — there is only one line to
/// land on — which is why it survived every single-line text-input test.
#[cfg(test)]
mod input_caret_hit_tests {
    use super::*;
    use rinch_core::events::{InputCallback, register_input_handler};
    use std::cell::Cell;
    use std::ops::Range;

    const VIEWPORT: (f32, f32) = (393.0, 852.0);

    /// Six numbered lines. Distinct, and each one its own line in the layout.
    const VALUE: &str = "line zero\nline one\nline two\nline three\nline four\nline five";

    /// The app screen this was found on: a padded scrolling column with a note,
    /// the field, and room under it. The padding is what the field's painted
    /// origin picks up and a plain parent-chain sum does not.
    fn mount_field() -> (RinchApp, usize) {
        let oninput_id = register_input_handler(InputCallback::new(|_| {}));
        let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let column = scope.create_element("div");
            column.set_attribute("style", "padding: 14px 22px 0; overflow-y: auto;");
            let note = scope.create_element("div");
            note.set_attribute("style", "font-size: 12px;");
            note.append_child(&scope.create_text("Chords over the words."));
            let field = scope.create_element("textarea");
            field.set_attribute(
                "style",
                "width: 100%; padding: 14px 15px; border: 1px solid #ccc; \
                 font-family: monospace; font-size: 13.5px; line-height: 1.5;",
            );
            field.set_attribute("rows", "12");
            field.set_attribute("value", VALUE);
            field.set_attribute("data-oninput", &oninput_id.0.to_string());
            let room = scope.create_element("div");
            room.set_attribute("style", "height: 320px;");
            column.append_child(&note);
            column.append_child(&field);
            column.append_child(&room);
            root.append_child(&column);
            id_in.set(Some(field.node_id().0));
            root
        });
        app.mount_component(VIEWPORT.0, VIEWPORT.1);
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        (
            app,
            id.get().expect("the field's node id, captured at mount"),
        )
    }

    fn click(app: &mut RinchApp, x: f32, y: f32) {
        let window = (VIEWPORT.0 as u32, VIEWPORT.1 as u32);
        app.handle_event(
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            },
            window,
            1.0,
        );
        app.handle_event(
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            },
            window,
            1.0,
        );
    }

    /// Where the painter puts the field's text: the origin of its first line box
    /// and, per visual line, `(top, height, byte range)` relative to that origin.
    ///
    /// Built the way `paint_input_value` builds it, against the *document's* font
    /// context, so this is the geometry actually on screen on whatever machine
    /// runs the test rather than a second guess at it.
    #[allow(clippy::type_complexity)]
    fn painted_lines(app: &mut RinchApp, id: usize) -> ((f32, f32), Vec<(f32, f32, Range<usize>)>) {
        let doc = app.doc.clone().expect("mounted");
        let mut d = doc.borrow_mut();
        let (style, width) = {
            let node = d.tree.get(id).expect("the field");
            (node.computed_style.clone(), node.layout.width)
        };
        let (sum_x, sum_y) = RinchApp::compute_absolute_position(&d.tree, id);
        let (ifc_dx, ifc_dy) = {
            let node = d.tree.get(id).expect("the field");
            rinch_dom::paint::ifc_content_box_offset(&d.tree, node)
        };
        let pad_l = style.padding_left.to_px();
        let pad_t = style.padding_top.to_px();

        let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        let mut builder = layout_cx.ranged_builder(&mut d.font_cx, VALUE, 1.0, true);
        builder.push_default(parley::style::StyleProperty::FontSize(style.font_size));
        builder.push_default(parley::style::StyleProperty::FontStack(
            parley::style::FontStack::Source(std::borrow::Cow::Owned(style.font_family.clone())),
        ));
        let mut layout = builder.build(VALUE);
        layout.break_all_lines(Some(width - pad_l * 2.0));

        let lines = layout
            .lines()
            .map(|line| {
                let m = line.metrics();
                (m.baseline - m.ascent, m.line_height, line.text_range())
            })
            .collect();
        ((sum_x + ifc_dx + pad_l, sum_y + ifc_dy + pad_t), lines)
    }

    /// The fault: a click on the middle of a painted line must put the caret on
    /// *that* line. Before the fix every one of them landed a line low.
    #[test]
    fn a_click_lands_on_the_line_it_was_aimed_at() {
        let (mut app, field) = mount_field();
        let ((text_x, text_y), lines) = painted_lines(&mut app, field);
        assert_eq!(lines.len(), 6, "one visual line per source line");

        for (n, (top, height, range)) in lines.iter().enumerate() {
            click(&mut app, text_x + 2.0, text_y + top + height / 2.0);
            let caret = app
                .focused_input_state
                .as_ref()
                .expect("the click focused the field")
                .selection
                .head
                .0;
            assert!(
                range.contains(&caret) || (n + 1 == lines.len() && caret == range.end),
                "a click on line {n} ({range:?}) put the caret at {caret}, on line {:?}",
                lines.iter().position(|(_, _, r)| r.contains(&caret)),
            );
        }
    }

    /// The same field with nothing beside it is laid out as a block, picks up no
    /// IFC offset, and must keep landing where it always did.
    #[test]
    fn a_field_outside_a_text_flow_still_lands_on_its_line() {
        let oninput_id = register_input_handler(InputCallback::new(|_| {}));
        let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let field = scope.create_element("textarea");
            field.set_attribute(
                "style",
                "width: 340px; padding: 14px 15px; font-family: monospace; font-size: 13.5px;",
            );
            field.set_attribute("rows", "12");
            field.set_attribute("value", VALUE);
            field.set_attribute("data-oninput", &oninput_id.0.to_string());
            root.append_child(&field);
            id_in.set(Some(field.node_id().0));
            root
        });
        app.mount_component(VIEWPORT.0, VIEWPORT.1);
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        let field = id.get().expect("the field's node id");

        let ((text_x, text_y), lines) = painted_lines(&mut app, field);
        for (n, (top, height, range)) in lines.iter().enumerate() {
            click(&mut app, text_x + 2.0, text_y + top + height / 2.0);
            let caret = app
                .focused_input_state
                .as_ref()
                .expect("the click focused the field")
                .selection
                .head
                .0;
            assert!(
                range.contains(&caret) || (n + 1 == lines.len() && caret == range.end),
                "a click on line {n} ({range:?}) put the caret at {caret}"
            );
        }
    }
}

#[cfg(test)]
mod android_frame_clock_tests {
    //! A bottom sheet opens on Android.
    //!
    //! The sheet idiom is three always-mounted boxes: a full-screen root whose
    //! `pointer-events` flips, a scrim whose opacity fades in, and a panel
    //! parked below the fold that slides up. Opening one is a style change and
    //! nothing else — which means two of the three properties are transitioned,
    //! and a transitioned property moves only when something turns the frame
    //! clock.
    //!
    //! [`PlatformEvent::AboutToWait`] is that clock, and the Android shell did
    //! not send it. So the tap flipped `pointer-events` — not animatable, so it
    //! applied at once — while the scrim stayed at opacity 0 and the panel
    //! stayed 700px below the window, for ever. The sheet was open, invisible,
    //! and covering the screen: the tap that opened it did nothing visible, and
    //! the next tap anywhere landed on its scrim and closed it again.
    //!
    //! The tap is spelled the way Android spells it — a `MouseDown` and a
    //! `MouseUp` in one batch with nothing in between — because that is the
    //! shape the fault was found in, on a phone, tapping a sort chip.

    // `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
    use super::*;
    use crate as rinch;
    use crate::shell::android_frame;
    use rinch_core::Signal;
    use rinch_macros::rsx;
    use std::time::{Duration, Instant};

    /// A portrait phone at 1x, so physical and layout pixels agree and the
    /// numbers below are the ones a reader can check against the styles.
    const PHYSICAL: (u32, u32) = (393, 852);
    const SCALE: f64 = 1.0;
    const VIEWPORT: (f32, f32) = (393.0, 852.0);

    /// Far enough below the window that the tallest sheet is clear of it. In
    /// pixels, not `100%`: the transition engine drops percentage translations
    /// when it interpolates a transform, and a percentage slide snaps.
    const PARKED_PX: f64 = 700.0;
    const EASE: &str = "220ms cubic-bezier(0, 0, 0.2, 1)";
    /// Long enough for the 220ms slide to finish, with room for a slow runner.
    const SETTLE: Duration = Duration::from_millis(1500);

    fn root_style(open: bool) -> String {
        let taps = if open { "auto" } else { "none" };
        format!(
            "position: absolute; left: 0; top: 0; right: 0; bottom: 0; \
             z-index: 40; pointer-events: {taps};"
        )
    }

    fn scrim_style(open: bool) -> String {
        let opacity = if open { "1" } else { "0" };
        format!(
            "position: absolute; left: 0; top: 0; right: 0; bottom: 0; \
             background: rgba(28, 25, 23, 0.38); opacity: {opacity}; \
             transition: opacity {EASE};"
        )
    }

    fn panel_style(open: bool) -> String {
        let y = if open { 0.0 } else { PARKED_PX };
        format!(
            "position: absolute; left: 0; right: 0; bottom: 0; height: 68%; \
             transform: translateY({y}px); transition: transform {EASE};"
        )
    }

    struct Sheet {
        app: RinchApp,
        trigger: usize,
        scrim: usize,
        panel: usize,
    }

    /// Mount the chip and the sheet it opens, inside the `overflow: hidden`
    /// root a fixed-height app shell is built from.
    fn mount() -> Sheet {
        let open = Signal::new(false);
        let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    style: "position: relative; overflow: hidden; \
                            width: 393px; height: 852px;",

                    div {
                        class: "trigger",
                        onclick: move || open.set(true),
                        style: "width: 120px; height: 32px;",
                    }

                    div {
                        style: {move || root_style(open.get())},
                        div {
                            class: "scrim",
                            style: {move || scrim_style(open.get())},
                        }
                        div {
                            class: "panel",
                            style: {move || panel_style(open.get())},
                        }
                    }
                }
            }
        });
        app.mount_component(VIEWPORT.0, VIEWPORT.1);
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

        let trigger = by_class(&app, "trigger");
        let scrim = by_class(&app, "scrim");
        let panel = by_class(&app, "panel");
        Sheet {
            app,
            trigger,
            scrim,
            panel,
        }
    }

    fn by_class(app: &RinchApp, class: &str) -> usize {
        let doc = app.doc.as_ref().expect("document");
        let d = doc.borrow();
        (0..d.tree.nodes.len())
            .find(|&id| {
                d.tree
                    .get(id)
                    .and_then(|n| n.attributes.get("class"))
                    .is_some_and(|c| c.split_whitespace().any(|c| c == class))
            })
            .unwrap_or_else(|| panic!("no node with class {class:?}"))
    }

    /// The panel's translateY, in pixels: `matrix[5]` is the `f` of the
    /// `[a, b, c, d, e, f]` affine, which is the vertical translation.
    fn panel_y(app: &RinchApp, panel: usize) -> f64 {
        let doc = app.doc.as_ref().expect("document");
        let d = doc.borrow();
        d.tree
            .get(panel)
            .expect("panel")
            .computed_style
            .transform
            .matrix[5]
    }

    /// The resolved `opacity` of a node, which for a transitioned or animated
    /// one is the value the last tick sampled.
    fn opacity_of(app: &RinchApp, node_id: usize) -> f32 {
        let doc = app.doc.as_ref().expect("document");
        let d = doc.borrow();
        d.tree.get(node_id).expect("node").computed_style.opacity
    }

    fn scrim_opacity(app: &RinchApp, scrim: usize) -> f32 {
        opacity_of(app, scrim)
    }

    fn centre(app: &RinchApp, node_id: usize) -> (f32, f32) {
        let doc = app.doc.as_ref().expect("document");
        let d = doc.borrow();
        let n = d.tree.get(node_id).expect("node still in the tree");
        let (x, y) = RinchApp::compute_absolute_position(&d.tree, node_id);
        assert!(
            n.layout.width > 0.0 && n.layout.height > 0.0,
            "node {node_id} has no box to aim at: {:?}",
            n.layout
        );
        (x + n.layout.width / 2.0, y + n.layout.height / 2.0)
    }

    /// One tap, spelled the way `TouchGesture` spells a still finger's lift:
    /// a `MouseDown` and a `MouseUp` pushed into the *same* event batch.
    fn tap(app: &mut RinchApp, x: f32, y: f32) {
        for event in [
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            },
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            },
        ] {
            app.handle_event(event, PHYSICAL, SCALE);
        }
    }

    /// What the Android loop does between polls, run until `done` or `SETTLE`
    /// elapses. `resolve_and_repaint` is the half the loop always had; the
    /// frame pump is the half it was missing.
    ///
    /// Returns how many frames the shell would have presented. Presenting is
    /// what clears `scene_dirty`, so the count is only meaningful if the test
    /// clears it exactly where a present would — which is what the
    /// `scene_dirty = false` below stands in for.
    fn run_frames(app: &mut RinchApp, pump: bool, done: impl Fn(&RinchApp) -> bool) -> usize {
        let deadline = Instant::now() + SETTLE;
        let mut presented = 0;
        while Instant::now() < deadline {
            let (pending, paint) = if pump {
                let frame = android_frame::pump_frame(app, PHYSICAL, SCALE);
                (frame.pending_layout, frame.needs_paint)
            } else {
                (app.has_pending_layout(), false)
            };
            if pending {
                app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
            }
            if pending || paint {
                app.scene_dirty = false;
                presented += 1;
            }
            if done(app) {
                return presented;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        presented
    }

    /// The fault this module exists for: a tap on the chip opens the sheet, and
    /// the sheet arrives on screen.
    #[test]
    fn a_tap_opens_the_sheet_and_the_frame_clock_slides_it_into_place() {
        let mut sheet = mount();
        assert_eq!(
            panel_y(&sheet.app, sheet.panel),
            PARKED_PX,
            "the sheet starts parked below the fold"
        );

        let (x, y) = centre(&sheet.app, sheet.trigger);
        tap(&mut sheet.app, x, y);

        // The tap alone does not move it: the style change *starts* the
        // transition, and a transition's first sample is its old value.
        assert_eq!(
            panel_y(&sheet.app, sheet.panel),
            PARKED_PX,
            "the tap starts the slide; it does not finish it"
        );

        let panel = sheet.panel;
        run_frames(&mut sheet.app, true, move |app| panel_y(app, panel) == 0.0);

        assert_eq!(
            panel_y(&sheet.app, sheet.panel),
            0.0,
            "the sheet must reach its open position — a sheet that never leaves \
             the fold is an invisible, pointer-active screen cover"
        );
        assert!(
            (scrim_opacity(&sheet.app, sheet.scrim) - 1.0).abs() < 0.01,
            "and the scrim must reach full opacity, got {}",
            scrim_opacity(&sheet.app, sheet.scrim)
        );
    }

    /// Why the loop turns the frame clock, kept as an executable statement
    /// rather than a comment: take the pump away and the sheet never moves,
    /// however many frames are painted and however long is waited.
    #[test]
    fn without_the_frame_clock_the_sheet_stays_parked_for_ever() {
        let mut sheet = mount();
        let (x, y) = centre(&sheet.app, sheet.trigger);
        tap(&mut sheet.app, x, y);

        run_frames(&mut sheet.app, false, |_| false);

        assert_eq!(
            panel_y(&sheet.app, sheet.panel),
            PARKED_PX,
            "with no clock the transition is sampled once, at its old value, \
             and stays there"
        );
        assert_eq!(
            scrim_opacity(&sheet.app, sheet.scrim),
            0.0,
            "and so does the scrim's opacity"
        );
    }

    /// The tick that *finishes* the slide has to be presented too — and on a
    /// phone it is usually the only tick there is.
    ///
    /// The first paint after the tap is not free: on the handset this was found
    /// on it takes around 300ms, against a 220ms slide. So by the time the loop
    /// comes back round, one tick completes the whole transition — and
    /// `tick_transitions` answers "is anything *still* running", which on that
    /// tick is no. Gate the repaint on that answer and the one frame that ever
    /// had the sheet in it is the one frame that is dropped: the surface keeps
    /// showing the parked sheet until something unrelated forces a present.
    #[test]
    fn the_tick_that_finishes_the_slide_asks_to_be_presented() {
        let mut sheet = mount();
        let (x, y) = centre(&sheet.app, sheet.trigger);
        tap(&mut sheet.app, x, y);

        // The frame the tap dirtied, presented — and the present is slow.
        let frame = android_frame::pump_frame(&mut sheet.app, PHYSICAL, SCALE);
        if frame.pending_layout {
            sheet.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        }
        sheet.app.scene_dirty = false;
        std::thread::sleep(Duration::from_millis(320));

        // One tick, and it is the one that completes the slide.
        let frame = android_frame::pump_frame(&mut sheet.app, PHYSICAL, SCALE);
        assert_eq!(
            panel_y(&sheet.app, sheet.panel),
            0.0,
            "the tick applied the transition's end value"
        );
        assert!(
            frame.needs_paint,
            "and the frame it applied it in has to reach the glass — this is \
             the only frame the sheet was ever in"
        );
    }

    /// The other half of turning the clock: the frames it moves the sheet in
    /// have to reach the screen.
    ///
    /// A transition on a paint-only property marks its node `PAINT`-dirty and
    /// nothing else, so the loop's three standing reasons to present — pending
    /// layout, a requested redraw, scroll momentum — are all false while a
    /// sheet slides. The sheet then moves in the tree and not on the glass:
    /// the surface keeps the frame from before the tap until something
    /// unrelated forces a present, which is the *next* tap, by which time it is
    /// landing on a sheet that is logically already open.
    #[test]
    fn every_frame_the_sheet_moves_in_asks_to_be_presented() {
        let mut sheet = mount();
        let (x, y) = centre(&sheet.app, sheet.trigger);
        tap(&mut sheet.app, x, y);

        let panel = sheet.panel;
        let presented = run_frames(&mut sheet.app, true, move |app| panel_y(app, panel) == 0.0);

        assert_eq!(panel_y(&sheet.app, sheet.panel), 0.0, "the slide finished");
        assert!(
            presented > 1,
            "a 220ms slide is many frames and every one of them has to be \
             presented; only {presented} asked to be"
        );
    }

    // ── The animation half of the same guard ─────────────────────────────────

    /// A `@keyframes` animation, started by the tap the way the sheet is.
    ///
    /// `fill-mode` is left at its default `none` on purpose. That is what makes
    /// the last tick a *completing* tick: `Animation::is_filling` is false, so
    /// the animation is dropped rather than kept, and `tick_animations` reports
    /// `any_active = false` for the very tick that ended it.
    const RISE_CSS: &str = r#"
        @keyframes k24-rise {
            from { opacity: 0.1; }
            to   { opacity: 0.9; }
        }
        .rising {
            animation: k24-rise 220ms linear;
        }
    "#;

    struct Riser {
        app: RinchApp,
        trigger: usize,
    }

    /// A chip that, when tapped, hands a box the class that animates it.
    fn mount_riser() -> Riser {
        let running = Signal::new(false);
        let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    style: "position: relative; overflow: hidden; \
                            width: 393px; height: 852px;",

                    style { {RISE_CSS} }

                    div {
                        class: "trigger",
                        onclick: move || running.set(true),
                        style: "width: 120px; height: 32px;",
                    }

                    div {
                        class: {move || if running.get() { "riser rising" } else { "riser" }},
                        style: "width: 120px; height: 120px; background: #444;",
                    }
                }
            }
        });
        app.mount_component(VIEWPORT.0, VIEWPORT.1);
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

        let trigger = by_class(&app, "trigger");
        Riser { app, trigger }
    }

    fn running_animations(app: &RinchApp) -> usize {
        let doc = app.doc.as_ref().expect("document");
        let d = doc.borrow();
        d.tree.active_animations.len()
    }

    /// The same fault as `the_tick_that_finishes_the_slide_asks_to_be_presented`,
    /// on the other half of the clock.
    ///
    /// `tick_animations` answers exactly the question `tick_transitions` did —
    /// "is anything *still* running" — and answers it `false` on the tick that
    /// ends an animation, after that tick has already changed what the element
    /// looks like. K23's `had_running` guard was written to cover both, because
    /// it asks "was there anything to tick" instead, but only the transition
    /// half had a test. This is the animation half.
    ///
    /// It matters more than it looks on Android, and card K24 is why: the first
    /// paint after a tap on the moto g stylus 5G took about 320ms against this
    /// 220ms animation, so the tick that ends it was the only tick it ever got.
    /// Even now that the same frame is nearer 60ms, an animation shorter than
    /// two frames still lands entirely on its completing tick, and dropping
    /// that frame means it never appears at all.
    #[test]
    fn the_tick_that_finishes_an_animation_asks_to_be_presented() {
        let mut riser = mount_riser();
        let (x, y) = centre(&riser.app, riser.trigger);
        tap(&mut riser.app, x, y);

        // The frame the tap dirtied, presented — and the present is slow.
        let frame = android_frame::pump_frame(&mut riser.app, PHYSICAL, SCALE);
        if frame.pending_layout {
            riser.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        }
        assert_eq!(
            running_animations(&riser.app),
            1,
            "the tap has to have started the animation, or the rest of this \
             test proves nothing about it"
        );
        riser.app.scene_dirty = false;
        std::thread::sleep(Duration::from_millis(320));

        // One tick, and it is the one that ends the animation.
        let frame = android_frame::pump_frame(&mut riser.app, PHYSICAL, SCALE);
        assert_eq!(
            running_animations(&riser.app),
            0,
            "this tick has to be the one that finished it — that is the whole \
             shape being tested"
        );
        assert!(
            frame.needs_paint,
            "and the frame that finished it has to reach the glass: the \
             element looks different now than in the frame before, and no \
             later tick will ever say so"
        );
    }

    /// The animation, driven through the pump the way the loop drives it, ends
    /// where the keyframes say — and asks to be presented more than once when
    /// the frames are fast enough for there to be more than one.
    #[test]
    fn the_frame_clock_runs_an_animation_to_its_end() {
        let mut riser = mount_riser();
        let (x, y) = centre(&riser.app, riser.trigger);
        tap(&mut riser.app, x, y);

        // "Nothing is running" is also the state *before* the tap's style
        // change has been resolved, so it cannot be the stop condition on its
        // own — the loop would return on its first pass having watched
        // nothing. Latch the start first; the transition half gets this for
        // free because its stop value (`panel_y == 0`) is not its start value.
        let seen = std::cell::Cell::new((false, false));
        let presented = run_frames(&mut riser.app, true, |app| {
            let (mut started, mut moved) = seen.get();
            if running_animations(app) > 0 {
                started = true;
                // The animation runs `opacity` from 0.1 to 0.9, so any sample
                // of it is below the 1.0 the class alone would give.
                moved |= opacity_of(app, by_class(app, "riser")) < 1.0;
            }
            seen.set((started, moved));
            started && running_animations(app) == 0
        });
        let (started, moved) = seen.get();

        assert!(started, "the tap has to have started the animation");
        assert!(
            moved,
            "and the animation has to have been sampled onto the element — an \
             animation that registers and expires without ever changing what \
             the element looks like is the fault this test exists for"
        );
        assert_eq!(
            running_animations(&riser.app),
            0,
            "the animation has to have finished inside {SETTLE:?}"
        );
        assert!(
            presented > 1,
            "a 220ms animation is many frames and every one of them has to be \
             presented; only {presented} asked to be"
        );
    }

    /// And the reason the loop turns the clock at all, stated for animations
    /// the way it already is for transitions: take the pump away and the
    /// animation never runs, however many frames are painted.
    #[test]
    fn without_the_frame_clock_an_animation_never_runs() {
        let mut riser = mount_riser();
        let (x, y) = centre(&riser.app, riser.trigger);
        tap(&mut riser.app, x, y);

        run_frames(&mut riser.app, false, |_| false);

        assert_eq!(
            running_animations(&riser.app),
            1,
            "with no clock the animation is registered and never advanced, so \
             it is still sitting there after {SETTLE:?}"
        );
    }
}

/// An image that finishes decoding while the app is idle has to reach the
/// screen it was idle on.
///
/// The decoding thread cannot know which nodes carry that `src` — working that
/// out is `drain_pending_images`'s job, and it runs inside layout. So every
/// gate of the form "nothing is dirty, skip the frame" also skips the one thing
/// that would have discovered there was work, and the picture never arrives:
/// not late, *never*, until something unrelated happens to redraw the screen.
/// `RinchApp` has exactly two such gates — [`RinchApp::has_dirty_nodes`] (the
/// `AboutToWait` frame clock, `RinchContext::update`, `needs_update`) and
/// [`RinchApp::has_pending_layout`] (the desktop paint preamble and wake, the
/// Android loop) — and both are checked here, along with the thing they exist
/// to buy: the box actually growing to the decoded bitmap.
#[cfg(test)]
mod pending_image_tests {
    use super::*;
    use rinch_core::image::{ImageLoadResult, ImageLoader};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    /// A 3x2 opaque-red PNG, inline so the test needs neither a fixture file
    /// nor an encoder dependency.
    const RED_3X2_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x9d,
        0x74, 0x66, 0x1a, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x86, 0x19, 0x90, 0x39, 0x00, 0x9b, 0x7e, 0x0b, 0xf5, 0x0f, 0x5f,
        0x26, 0x22, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// The same bitmap as a `data:` URI, which is decoded synchronously into
    /// the cache and never goes near the pending queue.
    const RED_3X2_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR42mP4z8DwH4YZkDkAm34L9Q9fJiIAAAAASUVORK5CYII=";

    /// A loader that parks its decode thread until the test lets it go. The
    /// window between "the app settled and went idle" and "the decode landed"
    /// is then the test's to control, instead of a race with a thread spawn.
    struct GatedLoader {
        gate: Arc<(Mutex<bool>, Condvar)>,
    }

    impl ImageLoader for GatedLoader {
        fn load(&self, _src: &str) -> ImageLoadResult {
            let (lock, cv) = &*self.gate;
            let mut open = lock.lock().unwrap_or_else(|e| e.into_inner());
            while !*open {
                open = cv.wait(open).unwrap_or_else(|e| e.into_inner());
            }
            ImageLoadResult::Loaded(RED_3X2_PNG.to_vec())
        }
    }

    fn img_box(app: &RinchApp, img: &NodeHandle) -> (f32, f32) {
        let doc = app.doc.as_ref().expect("mounted");
        let d = doc.borrow();
        let n = d
            .tree
            .nodes
            .get(img.node_id().0)
            .expect("img still in the tree");
        (n.layout.width, n.layout.height)
    }

    #[test]
    fn a_decode_landing_on_an_idle_app_reaches_the_box() {
        let captured: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let captured_in = captured.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let img = scope.create_element("img");
            root.append_child(&img);
            *captured_in.borrow_mut() = Some(img);
            root
        });
        app.mount_component(800.0, 600.0);

        // Swap in the gated loader before the `src` that triggers the load.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        {
            let doc = app.doc.as_ref().expect("mounted");
            doc.borrow_mut().tree.image_loader = Some(Arc::new(GatedLoader { gate: gate.clone() }));
        }

        let img = captured.borrow().clone().expect("img captured at mount");
        img.set_attribute("src", "gated://red.png");

        // Settle, the way an app that sets a `src` and then sits still settles:
        // the load is requested, the tree goes clean, the frame loop sleeps.
        for _ in 0..8 {
            if !app.has_pending_layout() {
                break;
            }
            app.resolve_and_repaint(800.0, 600.0);
        }
        assert!(
            !app.has_pending_layout() && !app.has_dirty_nodes(),
            "the app has to actually be idle for this test to mean anything"
        );
        assert_eq!(
            img_box(&app, &img),
            (0.0, 0.0),
            "no pixels yet, so no box yet"
        );

        // The decode lands. Nothing else happens — no input, no signal write.
        {
            let (lock, cv) = &*gate;
            *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
            cv.notify_all();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app.has_pending_images() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            app.has_pending_images(),
            "the decode never reached the queue"
        );

        assert!(
            app.has_pending_layout(),
            "the desktop paint preamble / wake and the Android loop gate on this \
             — without it the frame loop goes back to sleep for ever"
        );
        assert!(
            app.has_dirty_nodes(),
            "the AboutToWait frame clock and RinchContext::update gate on this \
             one instead — it is a different predicate and needs its own answer"
        );

        // And the resolve those gates ask for is the one that gives the `<img>`
        // the intrinsic size it was created without.
        app.resolve_and_repaint(800.0, 600.0);
        assert_eq!(
            img_box(&app, &img),
            (3.0, 2.0),
            "the decoded bitmap's size must reach the box"
        );

        // Drained: this costs one frame, not every frame from here on.
        assert!(!app.has_pending_images());
    }

    /// A `background-image` decode changes no box — `drain_pending_images`
    /// reports it as *not* needing a Taffy recompute — but its users' pixels did
    /// change, so they have to be in the dirty region the next paint clears, or
    /// the software renderer's dirty-region path repaints some other node's rect
    /// and skips the freshly decoded background entirely.
    #[test]
    fn a_background_image_decode_marks_its_users_paint_dirty() {
        let captured: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let captured_in = captured.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 50px; height: 50px");
            *captured_in.borrow_mut() = Some(root.clone());
            root
        });
        app.mount_component(800.0, 600.0);

        // Swap the loader in before the style that triggers the load, so the
        // default `FileImageLoader` never gets a chance to fail it first.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        {
            let doc = app.doc.as_ref().expect("mounted");
            doc.borrow_mut().tree.image_loader = Some(Arc::new(GatedLoader { gate: gate.clone() }));
        }
        captured
            .borrow()
            .clone()
            .expect("root captured at mount")
            .set_attribute(
                "style",
                "width: 50px; height: 50px; background-image: url(gated://bg.png)",
            );
        for _ in 0..8 {
            if !app.has_pending_layout() {
                break;
            }
            app.resolve_and_repaint(800.0, 600.0);
        }

        // A paint consumes the dirty region, so drop what the settle frames
        // left behind: what the decode adds has to stand on its own.
        {
            let doc = app.doc.as_ref().expect("mounted");
            doc.borrow_mut().tree.paint_dirty_nodes.clear();
        }

        {
            let (lock, cv) = &*gate;
            *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
            cv.notify_all();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app.has_pending_images() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            app.has_pending_images(),
            "the background-image decode never reached the queue"
        );

        app.resolve_and_repaint(800.0, 600.0);
        assert!(!app.has_pending_images(), "drained");

        let bg_id = captured
            .borrow()
            .clone()
            .expect("root captured at mount")
            .node_id()
            .0;
        let doc = app.doc.as_ref().expect("mounted");
        let d = doc.borrow();
        assert!(
            d.tree.paint_dirty_nodes.contains(&bg_id),
            "the background's own node must be in the region the next paint \
             clears, or the dirty-region path repaints around it"
        );
    }

    /// The synchronous path this is measured against: a `data:` URI is decoded
    /// straight into the cache, so the `<img>` is sized on its first layout.
    /// Both paths set the same Taffy node context — the asynchronous one just
    /// does it after the inline boxes have already been measured, which is the
    /// whole difficulty.
    #[test]
    fn a_data_uri_is_sized_on_the_first_layout() {
        let captured: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let captured_in = captured.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let img = scope.create_element("img");
            img.set_attribute("src", RED_3X2_DATA_URI);
            root.append_child(&img);
            *captured_in.borrow_mut() = Some(img);
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);

        let img = captured.borrow().clone().expect("img captured at mount");
        assert_eq!(img_box(&app, &img), (3.0, 2.0));
        assert!(
            !app.has_pending_images(),
            "a data: URI never goes near the pending queue"
        );
    }
}
