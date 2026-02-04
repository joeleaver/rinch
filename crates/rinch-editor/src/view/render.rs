//! DOM rendering using Rinch's NodeHandle API.
//!
//! Fine-grained DOM updates: each block is tracked individually, and only
//! changed blocks are re-rendered. This aligns with rinch's philosophy of
//! surgical DOM mutations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::rc::Weak;

use crate::document::{EditorDocument, InlineRun, MarkData};
use crate::editor::Editor;
use crate::view::visual_layer::{
    VisualLayerState, create_visual_layer, register_block_nodes, update_cursor_position,
    update_selection_rects,
};
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::{Effect, Signal};

/// Reactive state for the editor view.
///
/// Tracks DOM nodes for each block and provides fine-grained update capabilities.
#[allow(clippy::type_complexity)]
pub struct BlockSignals {
    /// Global version signal, bumped on any change.
    pub render_version: Signal<u64>,
    /// Map from block index to DOM node handle.
    pub block_nodes: Rc<RefCell<HashMap<usize, NodeHandle>>>,
    /// Visual layer state for cursor and selection rendering.
    pub visual_layer: Rc<RefCell<VisualLayerState>>,
    /// The container element holding all blocks.
    pub container: Rc<RefCell<Option<NodeHandle>>>,
    /// Weak reference to the document for creating child scopes.
    pub doc_weak: Rc<RefCell<Option<Weak<RefCell<dyn DomDocument>>>>>,
}

impl BlockSignals {
    pub fn new(visual_layer: VisualLayerState) -> Self {
        Self {
            render_version: Signal::new(0),
            block_nodes: Rc::new(RefCell::new(HashMap::new())),
            visual_layer: Rc::new(RefCell::new(visual_layer)),
            container: Rc::new(RefCell::new(None)),
            doc_weak: Rc::new(RefCell::new(None)),
        }
    }

    /// Bump the render version to trigger a re-render.
    pub fn bump(&self) {
        self.render_version.update(|v| *v += 1);
    }

    /// Get the DOM node for a block index.
    pub fn get_block_node(&self, block_index: usize) -> Option<NodeHandle> {
        self.block_nodes.borrow().get(&block_index).cloned()
    }

    /// Update the visual layer (cursor and selection) based on editor state.
    pub fn update_visual_layer(&self, editor: &Editor, scope: &mut RenderScope) {
        let mut visual_layer = self.visual_layer.borrow_mut();

        // Update block nodes mapping
        let block_map = self.block_nodes.borrow().clone();
        register_block_nodes(&mut visual_layer, block_map);

        // Update cursor position
        update_cursor_position(&mut visual_layer, editor, scope);

        // Update selection rectangles
        update_selection_rects(&mut visual_layer, editor, scope);
    }
}

/// Apply editor changes with fine-grained DOM updates.
///
/// Call this after any editor command. Only changed blocks are re-rendered.
pub fn apply_changes(editor: &Rc<RefCell<Editor>>, signals: &Rc<BlockSignals>) {
    let changes = if let Ok(mut ed) = editor.try_borrow_mut() {
        ed.take_changes()
    } else {
        tracing::warn!("apply_changes: editor borrow_mut FAILED");
        return;
    };

    tracing::info!(
        "apply_changes: changed_blocks={:?}, structure_changed={}",
        changes.changed_blocks,
        changes.structure_changed
    );

    // Get document reference for creating scopes
    let doc_weak = signals.doc_weak.borrow().clone();
    let Some(doc_weak) = doc_weak else {
        tracing::warn!("apply_changes: no doc_weak available");
        signals.bump();
        return;
    };

    let Some(doc): Option<Rc<RefCell<dyn DomDocument>>> = doc_weak.upgrade() else {
        tracing::warn!("apply_changes: doc_weak upgrade failed");
        signals.bump();
        return;
    };

    let container = signals.container.borrow().clone();
    let Some(container) = container else {
        tracing::warn!("apply_changes: no container available");
        signals.bump();
        return;
    };

    if let Ok(ed) = editor.try_borrow() {
        if changes.structure_changed {
            // Block count changed - need to reconcile structure
            tracing::info!("apply_changes: structure changed, reconciling blocks");
            reconcile_blocks(&doc, &container, &ed, signals);
        } else {
            // Only content changed - update specific blocks
            for &block_idx in &changes.changed_blocks {
                tracing::info!("apply_changes: updating block {}", block_idx);
                update_block_content(&doc, &ed, block_idx, signals);
            }
        }

        // Update visual layer with current block nodes
        {
            let block_map = signals.block_nodes.borrow().clone();
            let mut visual_layer = signals.visual_layer.borrow_mut();
            register_block_nodes(&mut visual_layer, block_map);
        }
    }

    // Force layout resolution before querying cursor position
    // This ensures text_layout is populated so caret queries work
    {
        let mut doc_mut = doc.borrow_mut();
        // Use a reasonable default viewport size - the actual size will be
        // corrected by the runtime's resolve_layout call before paint
        doc_mut.resolve_layout(1200.0, 800.0);
    }

    // Update cursor position and selection rectangles (needs editor borrow)
    if let Ok(ed) = editor.try_borrow() {
        update_cursor_after_changes(&ed, signals, &doc);
    }

    // Still bump to trigger any dependent effects
    signals.bump();
}

/// Reconcile block structure when blocks are added/removed.
fn reconcile_blocks(
    doc: &Rc<RefCell<dyn DomDocument>>,
    container: &NodeHandle,
    editor: &Editor,
    signals: &BlockSignals,
) {
    let block_count = editor.doc.block_count();
    let mut block_nodes = signals.block_nodes.borrow_mut();
    let old_count = block_nodes.len();

    // Simple approach: if structure changed, rebuild all blocks
    // More sophisticated diffing could be added later

    // Remove all existing block nodes
    for (_idx, node) in block_nodes.drain() {
        node.remove();
    }

    // Re-render all blocks
    let container_id = container.node_id();
    let mut scope = RenderScope::new(doc.clone(), container_id);

    for i in 0..block_count {
        let block_node = Renderer::render_block(&mut scope, &editor.doc, i);
        container.append_child(&block_node);
        block_nodes.insert(i, block_node);
    }

    tracing::info!("reconcile_blocks: {} -> {} blocks", old_count, block_count);
}

/// Update content of a single block without recreating the element.
fn update_block_content(
    doc: &Rc<RefCell<dyn DomDocument>>,
    editor: &Editor,
    block_idx: usize,
    signals: &BlockSignals,
) {
    let block_nodes = signals.block_nodes.borrow_mut();

    let Some(block_node) = block_nodes.get(&block_idx) else {
        tracing::warn!("update_block_content: block {} not found", block_idx);
        return;
    };

    // Clear existing children of the block
    for child in block_node.children() {
        child.remove();
    }

    // Re-render inline content into the block
    let container_id = block_node.node_id();
    let mut scope = RenderScope::new(doc.clone(), container_id);

    let block_type = editor
        .doc
        .block_type(block_idx)
        .unwrap_or_else(|| "paragraph".into());

    // For hr, no content needed
    if block_type == "horizontal_rule" {
        return;
    }

    // For code_block, wrap in <code>
    if block_type == "code_block" {
        let code = scope.create_element("code");
        let runs = editor.doc.block_inline_runs(block_idx);
        Renderer::render_inline_runs(&mut scope, &code, &runs);
        block_node.append_child(&code);
        return;
    }

    // Render inline content
    let runs = editor.doc.block_inline_runs(block_idx);
    Renderer::render_inline_runs(&mut scope, block_node, &runs);
}

/// Update cursor position and selection rectangles after changes.
fn update_cursor_after_changes(
    editor: &Editor,
    signals: &BlockSignals,
    doc: &Rc<RefCell<dyn DomDocument>>,
) {
    let visual_layer = signals.visual_layer.borrow_mut();
    let block_nodes = signals.block_nodes.borrow();

    let selection = editor.get_selection();

    // Update cursor position
    let resolved = editor.doc.resolve_position(selection.head).ok();
    if let Some(rp) = resolved
        && let Some(block_node) = block_nodes.get(&rp.block_index)
        && let Some((x, y)) = block_node.query_caret_position(rp.text_offset)
    {
        // Add block's layout position to get coordinates relative to wrapper
        let (block_x, block_y) = block_node
            .get_layout_bounds()
            .map(|(bx, by, _, _)| (bx, by))
            .unwrap_or((0.0, 0.0));
        let cursor_x = block_x + x;
        let cursor_y = block_y + y;

        if let Some(cursor_node) = &visual_layer.cursor_node {
            // Hide cursor when there's a selection range
            if selection.is_cursor() {
                let height = block_node
                    .query_glyph_bounds(rp.text_offset)
                    .map(|bounds| bounds.height)
                    .unwrap_or(20.0);
                let style = format!(
                    "position: absolute; \
                             left: {}px; \
                             top: {}px; \
                             width: 2px; \
                             height: {}px; \
                             background-color: var(--rinch-primary-color, #228be6); \
                             pointer-events: none; \
                             animation: editor-cursor-blink 1s step-end infinite;",
                    cursor_x, cursor_y, height
                );
                cursor_node.set_attribute("style", &style);
            } else {
                // Hide cursor when there's a selection
                cursor_node.set_attribute("style", "display: none;");
            }
        }
    }

    // Update selection rectangles
    // Need to drop block_nodes borrow before creating scope
    drop(block_nodes);

    if let Some(selection_container) = &visual_layer.selection_container {
        // Clear existing selection rectangles
        for child in selection_container.children() {
            child.remove();
        }

        if !selection.is_cursor() {
            // Render selection rectangles
            let container_id = selection_container.node_id();
            let mut scope = RenderScope::new(doc.clone(), container_id);

            // Get selection range
            let start = editor.doc.resolve_position(selection.start());
            let end = editor.doc.resolve_position(selection.end());

            if let (Ok(start), Ok(end)) = (start, end) {
                let block_nodes = signals.block_nodes.borrow();

                for block_idx in start.block_index..=end.block_index {
                    if let Some(block_node) = block_nodes.get(&block_idx) {
                        let block_start = if block_idx == start.block_index {
                            start.text_offset
                        } else {
                            0
                        };

                        let block_end = if block_idx == end.block_index {
                            end.text_offset
                        } else {
                            editor
                                .doc
                                .block_text(block_idx)
                                .map(|t| t.len())
                                .unwrap_or(0)
                        };

                        if block_start < block_end {
                            // Get block position
                            let (block_x, block_y) = block_node
                                .get_layout_bounds()
                                .map(|(bx, by, _, _)| (bx, by))
                                .unwrap_or((0.0, 0.0));

                            // Query glyph bounds for start and end
                            if let (Some(start_bounds), Some(end_bounds)) = (
                                block_node.query_glyph_bounds(block_start),
                                block_node.query_glyph_bounds(block_end),
                            ) {
                                // Check if selection spans multiple lines
                                if (start_bounds.y - end_bounds.y).abs() > 1.0 {
                                    // Multi-line selection - walk through to find line breaks
                                    let mut current_offset = block_start;
                                    let mut current_y = start_bounds.y;

                                    while current_offset < block_end {
                                        let current_bounds =
                                            match block_node.query_glyph_bounds(current_offset) {
                                                Some(b) => b,
                                                None => break,
                                            };

                                        // Find end of this line
                                        let mut line_end = current_offset + 1;
                                        while line_end < block_end {
                                            if let Some(next_bounds) =
                                                block_node.query_glyph_bounds(line_end)
                                            {
                                                if (next_bounds.y - current_y).abs() > 1.0 {
                                                    break;
                                                }
                                                line_end += 1;
                                            } else {
                                                break;
                                            }
                                        }

                                        // Create rect for this line
                                        if let Some(line_end_bounds) =
                                            block_node.query_glyph_bounds(line_end.min(block_end))
                                        {
                                            let selection_rect = scope.create_element("div");
                                            selection_rect
                                                .set_attribute("class", "editor-selection-rect");
                                            let style = format!(
                                                "position: absolute; \
                                                 left: {}px; \
                                                 top: {}px; \
                                                 width: {}px; \
                                                 height: {}px; \
                                                 background-color: rgba(34, 139, 230, 0.3); \
                                                 pointer-events: none;",
                                                block_x + current_bounds.x,
                                                block_y + current_bounds.y,
                                                (line_end_bounds.x - current_bounds.x).max(2.0),
                                                current_bounds.height.max(20.0)
                                            );
                                            selection_rect.set_attribute("style", &style);
                                            selection_container.append_child(&selection_rect);
                                        }

                                        current_offset = line_end;
                                        if let Some(next_bounds) =
                                            block_node.query_glyph_bounds(current_offset)
                                        {
                                            current_y = next_bounds.y;
                                        }
                                    }
                                } else {
                                    // Single line selection
                                    let selection_rect = scope.create_element("div");
                                    selection_rect.set_attribute("class", "editor-selection-rect");
                                    let style = format!(
                                        "position: absolute; \
                                         left: {}px; \
                                         top: {}px; \
                                         width: {}px; \
                                         height: {}px; \
                                         background-color: rgba(34, 139, 230, 0.3); \
                                         pointer-events: none;",
                                        block_x + start_bounds.x,
                                        block_y + start_bounds.y,
                                        (end_bounds.x - start_bounds.x).max(2.0),
                                        start_bounds.height.max(20.0)
                                    );
                                    selection_rect.set_attribute("style", &style);
                                    selection_container.append_child(&selection_rect);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Render the document reactively with fine-grained updates.
///
/// Returns `(container, block_signals)`. The container is the DOM node.
/// After each editor command, call `apply_changes(editor, &signals)` to
/// trigger surgical DOM updates.
pub fn render_document_reactive(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
) -> (NodeHandle, Rc<BlockSignals>) {
    // Create visual layer overlay
    let (visual_layer_container, visual_layer_state) = create_visual_layer(scope);

    let signals = Rc::new(BlockSignals::new(visual_layer_state));

    // Create wrapper container that holds both content and visual layer
    let wrapper = scope.create_element("div");
    wrapper.set_attribute("style", "position: relative;");

    let container = scope.create_element("div");
    container.set_attribute("class", "editor-document");

    // Store container and doc reference for apply_changes
    *signals.container.borrow_mut() = Some(container.clone());
    *signals.doc_weak.borrow_mut() = Some(scope.doc_weak());

    // Initial render using direct DOM manipulation
    if let Ok(ed) = editor.try_borrow() {
        let block_count = ed.doc.block_count();
        let mut block_nodes = signals.block_nodes.borrow_mut();

        for i in 0..block_count {
            let block_node = Renderer::render_block(scope, &ed.doc, i);
            container.append_child(&block_node);
            block_nodes.insert(i, block_node);
        }

        tracing::info!(
            "render_document_reactive: initial render of {} blocks",
            block_count
        );
    }

    // Append content and visual layer to wrapper
    wrapper.append_child(&container);
    wrapper.append_child(&visual_layer_container);

    // Effect for cursor updates (lightweight, doesn't re-render content)
    let version = signals.render_version;
    let editor_for_cursor = editor.clone();
    let visual_layer_clone = signals.visual_layer.clone();
    let block_nodes_clone = signals.block_nodes.clone();
    let initial_v = version.get();
    let prev_v: Rc<RefCell<u64>> = Rc::new(RefCell::new(initial_v));

    let effect = Effect::new(move || {
        let v = version.get();
        let prev = *prev_v.borrow();
        if v == prev {
            return;
        }
        *prev_v.borrow_mut() = v;

        // Update cursor position
        if let Ok(ed) = editor_for_cursor.try_borrow() {
            let visual_layer = visual_layer_clone.borrow();
            let block_nodes = block_nodes_clone.borrow();

            let resolved = ed.doc.resolve_position(ed.get_selection().head).ok();
            if let Some(rp) = resolved
                && let Some(block_node) = block_nodes.get(&rp.block_index)
                && let Some((x, y)) = block_node.query_caret_position(rp.text_offset)
            {
                // Add block's layout position to get coordinates relative to wrapper
                let (block_x, block_y) = block_node
                    .get_layout_bounds()
                    .map(|(bx, by, _, _)| (bx, by))
                    .unwrap_or((0.0, 0.0));
                let cursor_x = block_x + x;
                let cursor_y = block_y + y;

                if let Some(cursor_node) = &visual_layer.cursor_node {
                    let height = block_node
                        .query_glyph_bounds(rp.text_offset)
                        .map(|bounds| bounds.height)
                        .unwrap_or(20.0);
                    let style = format!(
                        "position: absolute; \
                                 left: {}px; \
                                 top: {}px; \
                                 width: 2px; \
                                 height: {}px; \
                                 background-color: var(--rinch-primary-color, #228be6); \
                                 pointer-events: none; \
                                 animation: editor-cursor-blink 1s step-end infinite;",
                        cursor_x, cursor_y, height
                    );
                    cursor_node.set_attribute("style", &style);
                }
            }
        }
    });
    scope.create_effect_from(effect);

    (wrapper, signals)
}

// ============================================================================
// Renderer - Direct DOM manipulation
// ============================================================================

/// Renderer for converting document model to DOM using direct node creation.
#[derive(Debug)]
pub struct Renderer;

impl Renderer {
    /// Render a single block to the appropriate HTML element.
    pub fn render_block(
        scope: &mut RenderScope,
        doc: &EditorDocument,
        block_index: usize,
    ) -> NodeHandle {
        let block_type = doc
            .block_type(block_index)
            .unwrap_or_else(|| "paragraph".into());
        let attrs = doc.block_attrs(block_index).unwrap_or_default();

        let tag = match block_type.as_str() {
            "paragraph" => "p",
            "heading" => match attrs.get("level").map(|s| s.as_str()) {
                Some("1") => "h1",
                Some("2") => "h2",
                Some("3") => "h3",
                Some("4") => "h4",
                Some("5") => "h5",
                Some("6") => "h6",
                _ => "h1",
            },
            "blockquote" => "blockquote",
            "code_block" => "pre",
            "bullet_list" => "li",
            "ordered_list" => "li",
            "list_item" => "li",
            "horizontal_rule" => "hr",
            _ => "p",
        };

        let element = scope.create_element(tag);
        element.set_attribute("data-block-index", &block_index.to_string());

        // Apply alignment if present
        if let Some(align) = attrs.get("align") {
            element.set_attribute("style", &format!("text-align: {};", align));
        }

        // For hr, no inline content needed
        if block_type == "horizontal_rule" {
            return element;
        }

        // For code_block, wrap content in <code>
        if block_type == "code_block" {
            let code = scope.create_element("code");
            let runs = doc.block_inline_runs(block_index);
            Self::render_inline_runs(scope, &code, &runs);
            element.append_child(&code);
            return element;
        }

        // Render inline content
        let runs = doc.block_inline_runs(block_index);
        Self::render_inline_runs(scope, &element, &runs);

        element
    }

    /// Render inline runs into a parent element.
    pub fn render_inline_runs(scope: &mut RenderScope, parent: &NodeHandle, runs: &[InlineRun]) {
        for run in runs {
            let node = Self::render_inline_run(scope, run);
            parent.append_child(&node);
        }
    }

    /// Render a single inline run (text with marks or hard break).
    pub fn render_inline_run(scope: &mut RenderScope, run: &InlineRun) -> NodeHandle {
        match run.inline_type.as_str() {
            "hard_break" => scope.create_element("br"),
            "text" => {
                if run.marks.is_empty() {
                    // Plain text node
                    scope.create_text(&run.text)
                } else {
                    // Wrap text in nested mark elements (outermost first)
                    Self::wrap_in_marks(scope, &run.text, &run.marks)
                }
            }
            _ => scope.create_text(""),
        }
    }

    /// Wrap text content in nested mark elements.
    fn wrap_in_marks(scope: &mut RenderScope, text: &str, marks: &[MarkData]) -> NodeHandle {
        if marks.is_empty() {
            return scope.create_text(text);
        }

        // Build from innermost to outermost
        let text_node = scope.create_text(text);
        let mut current = text_node;

        for mark in marks.iter().rev() {
            let wrapper = Self::mark_element(scope, mark);
            wrapper.append_child(&current);
            current = wrapper;
        }

        current
    }

    /// Create the HTML element for a mark.
    fn mark_element(scope: &mut RenderScope, mark: &MarkData) -> NodeHandle {
        match mark.mark_type.as_str() {
            "bold" => scope.create_element("strong"),
            "italic" => scope.create_element("em"),
            "underline" => scope.create_element("u"),
            "strike" | "strikethrough" => scope.create_element("s"),
            "code" => scope.create_element("code"),
            "highlight" => scope.create_element("mark"),
            "subscript" => scope.create_element("sub"),
            "superscript" => scope.create_element("sup"),
            "link" => {
                let a = scope.create_element("a");
                if let Some(href) = mark.attrs.get("href") {
                    a.set_attribute("href", href);
                }
                a
            }
            "textColor" => {
                let span = scope.create_element("span");
                if let Some(color) = mark.attrs.get("color") {
                    span.set_attribute("style", &format!("color: {};", color));
                }
                span
            }
            _ => {
                // Unknown mark: wrap in span
                let span = scope.create_element("span");
                span.set_attribute("data-mark", &mark.mark_type);
                span
            }
        }
    }
}

/// CSS for cursor blink animation.
#[allow(dead_code)]
pub fn cursor_blink_css() -> &'static str {
    r#"
    @keyframes editor-cursor-blink {
        0%, 100% { opacity: 1; }
        50% { opacity: 0; }
    }
    "#
}
