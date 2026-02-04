//! Visual overlay layer for cursor and selection rendering.
//!
//! This module provides an absolutely-positioned overlay that renders cursor
//! and selection at correct screen positions using DOM layout queries.

use crate::editor::Editor;
use rinch_core::dom::{NodeHandle, RenderScope};
use std::collections::HashMap;

/// Screen-space bounding box for a glyph.
#[derive(Debug, Clone, Copy)]
pub struct GlyphBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Screen-space rectangle for selection highlighting.
#[derive(Debug, Clone, Copy)]
pub struct SelectionRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// State for the visual overlay layer.
#[derive(Debug)]
pub struct VisualLayerState {
    /// The cursor element (blinking caret)
    pub cursor_node: Option<NodeHandle>,
    /// Container for selection rectangles
    pub selection_container: Option<NodeHandle>,
    /// Map from block index to rendered DOM node (for position queries)
    pub block_nodes: HashMap<usize, NodeHandle>,
}

impl VisualLayerState {
    /// Create a new empty visual layer state.
    pub fn new() -> Self {
        Self {
            cursor_node: None,
            selection_container: None,
            block_nodes: HashMap::new(),
        }
    }
}

impl Default for VisualLayerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the visual overlay container with cursor and selection layers.
///
/// Returns `(overlay_container, state)` where:
/// - `overlay_container` should be positioned absolutely over the editor content
/// - `state` is used to update cursor/selection positions
pub fn create_visual_layer(scope: &mut RenderScope) -> (NodeHandle, VisualLayerState) {
    let container = scope.create_element("div");
    container.set_attribute("class", "editor-visual-layer");
    container.set_attribute(
        "style",
        "position: absolute; \
         top: 0; \
         left: 0; \
         width: 100%; \
         height: 100%; \
         pointer-events: none; \
         z-index: 1;",
    );

    // Create cursor element
    let cursor = scope.create_element("div");
    cursor.set_attribute("class", "editor-cursor");
    cursor.set_attribute("style", &cursor_style(0.0, 0.0, 20.0));
    container.append_child(&cursor);

    // Create selection container
    let selection_container = scope.create_element("div");
    selection_container.set_attribute("class", "editor-selection-layer");
    selection_container.set_attribute(
        "style",
        "position: absolute; \
         top: 0; \
         left: 0; \
         width: 100%; \
         height: 100%; \
         pointer-events: none; \
         z-index: 0;",
    );
    container.append_child(&selection_container);

    let state = VisualLayerState {
        cursor_node: Some(cursor),
        selection_container: Some(selection_container),
        block_nodes: HashMap::new(),
    };

    (container, state)
}

/// Generate CSS for the cursor element.
fn cursor_style(x: f32, y: f32, height: f32) -> String {
    format!(
        "position: absolute; \
         left: {}px; \
         top: {}px; \
         width: 2px; \
         height: {}px; \
         background-color: var(--rinch-primary-color, #228be6); \
         pointer-events: none; \
         animation: editor-cursor-blink 1s step-end infinite;",
        x, y, height
    )
}

/// Generate CSS for a selection rectangle.
fn selection_style(rect: &SelectionRect) -> String {
    format!(
        "position: absolute; \
         left: {}px; \
         top: {}px; \
         width: {}px; \
         height: {}px; \
         background-color: rgba(34, 139, 230, 0.3); \
         pointer-events: none;",
        rect.x, rect.y, rect.width, rect.height
    )
}

/// Update cursor position based on editor selection.
///
/// This queries the DOM for the glyph bounds at the cursor position and
/// updates the cursor element's position accordingly.
pub fn update_cursor_position(
    state: &mut VisualLayerState,
    editor: &Editor,
    _scope: &mut RenderScope,
) {
    let cursor_node = match &state.cursor_node {
        Some(node) => node,
        None => return,
    };

    let selection = editor.get_selection();
    if !selection.is_cursor() {
        // Hide cursor when there's a range selection
        cursor_node.set_attribute("style", "display: none;");
        return;
    }

    // Resolve cursor position to (block_index, text_offset)
    let resolved = match editor.doc.resolve_position(selection.head) {
        Ok(rp) => rp,
        Err(_) => {
            // Fallback: hide cursor if position can't be resolved
            cursor_node.set_attribute("style", "display: none;");
            return;
        }
    };

    // Get the NodeHandle for this block
    let block_node = match state.block_nodes.get(&resolved.block_index) {
        Some(node) => node,
        None => {
            // Block not found - hide cursor
            cursor_node.set_attribute("style", "display: none;");
            return;
        }
    };

    // Query caret position from the DOM
    let (x, y) = block_node
        .query_caret_position(resolved.text_offset)
        .unwrap_or((0.0, 0.0));

    // Get line height from actual text layout
    let height = block_node
        .query_glyph_bounds(resolved.text_offset)
        .map(|bounds| bounds.height)
        .unwrap_or(20.0);

    cursor_node.set_attribute("style", &cursor_style(x, y, height));
}

/// Update selection rectangles based on editor selection range.
///
/// This queries the DOM for glyph bounds across the selection range and
/// generates selection rectangles that highlight the selected text.
pub fn update_selection_rects(
    state: &mut VisualLayerState,
    editor: &Editor,
    scope: &mut RenderScope,
) {
    let container = match &state.selection_container {
        Some(node) => node,
        None => return,
    };

    // Clear existing selection rectangles
    for child in container.children() {
        child.remove();
    }

    let selection = editor.get_selection();
    if selection.is_cursor() {
        // No selection to render
        return;
    }

    // Resolve selection start/end to (block_index, text_offset)
    let start = match editor.doc.resolve_position(selection.start()) {
        Ok(pos) => pos,
        Err(_) => return,
    };
    let end = match editor.doc.resolve_position(selection.end()) {
        Ok(pos) => pos,
        Err(_) => return,
    };

    let mut rects = Vec::new();

    // For each block in the selection range
    for block_idx in start.block_index..=end.block_index {
        let node = match state.block_nodes.get(&block_idx) {
            Some(n) => n,
            None => continue,
        };

        // Calculate text range within this block
        let block_start = if block_idx == start.block_index {
            start.text_offset
        } else {
            0
        };

        let block_end = if block_idx == end.block_index {
            end.text_offset
        } else {
            // End of block - get block text length
            editor
                .doc
                .block_text(block_idx)
                .map(|t| t.len())
                .unwrap_or(0)
        };

        // Skip empty ranges
        if block_start >= block_end {
            continue;
        }

        // Query glyph bounds for start and end
        let start_bounds = match node.query_glyph_bounds(block_start) {
            Some(b) => b,
            None => continue,
        };

        let end_bounds = match node.query_glyph_bounds(block_end) {
            Some(b) => b,
            None => continue,
        };

        // Check if selection spans multiple lines (text wrapping)
        // If y-coordinates differ, we have line wrapping
        if (start_bounds.y - end_bounds.y).abs() > 1.0 {
            // Multi-line within block - need multiple rects
            // For now, create rects for each line
            let mut current_offset = block_start;
            let mut current_y = start_bounds.y;

            // Walk through the text to find line breaks
            while current_offset < block_end {
                let current_bounds = match node.query_glyph_bounds(current_offset) {
                    Some(b) => b,
                    None => break,
                };

                // Find end of this line (where y changes or we reach block_end)
                let mut line_end = current_offset + 1;
                while line_end < block_end {
                    if let Some(next_bounds) = node.query_glyph_bounds(line_end) {
                        if (next_bounds.y - current_y).abs() > 1.0 {
                            // Line changed
                            break;
                        }
                        line_end += 1;
                    } else {
                        break;
                    }
                }

                // Create rect for this line
                if let Some(line_end_bounds) = node.query_glyph_bounds(line_end.min(block_end)) {
                    rects.push(SelectionRect {
                        x: current_bounds.x,
                        y: current_bounds.y,
                        width: (line_end_bounds.x - current_bounds.x).max(2.0), // Minimum 2px width
                        height: current_bounds.height,
                    });
                }

                // Move to next line
                current_offset = line_end;
                if let Some(next_bounds) = node.query_glyph_bounds(current_offset) {
                    current_y = next_bounds.y;
                }
            }
        } else {
            // Single line - simple case
            rects.push(SelectionRect {
                x: start_bounds.x,
                y: start_bounds.y,
                width: (end_bounds.x - start_bounds.x).max(2.0), // Minimum 2px width
                height: start_bounds.height,
            });
        }
    }

    // Render selection rects as divs
    for rect in &rects {
        let selection_rect = scope.create_element("div");
        selection_rect.set_attribute("class", "editor-selection-rect");
        selection_rect.set_attribute("style", &selection_style(rect));
        container.append_child(&selection_rect);
    }
}

/// Register block nodes for position queries.
///
/// This should be called after rendering the document, passing a map from
/// block index to the rendered DOM node. The visual layer uses these nodes
/// to query glyph positions.
pub fn register_block_nodes(state: &mut VisualLayerState, block_nodes: HashMap<usize, NodeHandle>) {
    state.block_nodes = block_nodes;
}

/// CSS keyframes for cursor blinking animation.
///
/// This should be injected into the document's <style> element.
pub fn cursor_blink_css() -> &'static str {
    r#"
@keyframes editor-cursor-blink {
    0%, 49% { opacity: 1; }
    50%, 100% { opacity: 0; }
}
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visual_layer_state_creation() {
        let state = VisualLayerState::new();
        assert!(state.cursor_node.is_none());
        assert!(state.selection_container.is_none());
        assert!(state.block_nodes.is_empty());
    }

    #[test]
    fn test_cursor_style_generation() {
        let style = cursor_style(10.0, 20.0, 24.0);
        assert!(style.contains("left: 10px"));
        assert!(style.contains("top: 20px"));
        assert!(style.contains("height: 24px"));
        assert!(style.contains("width: 2px"));
        assert!(style.contains("background-color: var(--rinch-primary-color, #228be6)"));
        assert!(style.contains("animation: editor-cursor-blink"));
    }

    #[test]
    fn test_selection_style_generation() {
        let rect = SelectionRect {
            x: 5.0,
            y: 10.0,
            width: 50.0,
            height: 20.0,
        };
        let style = selection_style(&rect);
        assert!(style.contains("left: 5px"));
        assert!(style.contains("top: 10px"));
        assert!(style.contains("width: 50px"));
        assert!(style.contains("height: 20px"));
        assert!(style.contains("background-color: rgba(34, 139, 230, 0.3)"));
    }
}
