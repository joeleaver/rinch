//! Cursor synchronization between editor selection and contentEditable DOM.
//!
//! ContentEditable handles cursor rendering and selection display natively.
//! This module provides utilities for syncing editor model positions to/from
//! DOM state when needed (e.g., after programmatic cursor moves).

use crate::document::Position;
use crate::editor::Editor;

/// Convert a click's text hit info (block_index, byte_offset) to an absolute
/// editor Position.
pub fn hit_to_position(editor: &Editor, block_index: usize, byte_offset: usize) -> Position {
    let block_count = editor.doc.block_count();
    if block_count == 0 {
        return Position::new(0);
    }

    let bi = block_index.min(block_count - 1);
    let block_start = editor.doc.block_start_position(bi);
    let block_len = editor.doc.block_text(bi).map(|t| t.len()).unwrap_or(0);
    let offset = byte_offset.min(block_len);

    Position::new((block_start + offset).min(editor.doc.text_length()))
}

/// Convert an absolute editor Position to (block_index, byte_offset).
/// Returns None if the position cannot be resolved.
pub fn position_to_block_offset(editor: &Editor, pos: Position) -> Option<(usize, usize)> {
    editor
        .doc
        .resolve_position(pos)
        .ok()
        .map(|rp| (rp.block_index, rp.text_offset))
}

/// Get the active marks at the current cursor position.
/// Returns mark type names that are active (from document marks + stored marks).
pub fn active_marks(editor: &Editor) -> Vec<String> {
    // If stored marks are explicitly set, use those instead of document context
    if let Some(ref stored) = editor.stored_marks {
        return stored.clone();
    }

    let sel = editor.get_selection();
    editor
        .doc
        .marks_at(sel.head)
        .iter()
        .map(|m| m.mark_type.clone())
        .collect()
}

/// Get the block type at the current cursor position.
pub fn active_block_type(editor: &Editor) -> Option<String> {
    let sel = editor.get_selection();
    let rp = editor.doc.resolve_position(sel.head).ok()?;
    editor.doc.block_type(rp.block_index)
}

/// Get block attributes at the current cursor position.
pub fn active_block_attrs(editor: &Editor) -> Option<std::collections::HashMap<String, String>> {
    let sel = editor.get_selection();
    let rp = editor.doc.resolve_position(sel.head).ok()?;
    editor.doc.block_attrs(rp.block_index)
}
