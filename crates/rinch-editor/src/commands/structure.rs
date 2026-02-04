//! Structural commands (nodes and block transformations).

use std::collections::HashMap;

use crate::error::EditorError;
use crate::editor::Editor;
use crate::document::Position;
use crate::history::UndoOperation;
use crate::selection::Selection;

/// Commands for manipulating document structure.
#[derive(Debug)]
pub struct StructureCommands;

impl StructureCommands {
    /// Set the block type at the current cursor position.
    pub fn set_block_type(editor: &mut Editor, node_type: &str) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        let resolved = editor.doc.resolve_position(sel.head)?;
        let old_type = editor.doc.block_type(resolved.block_index).unwrap_or_default();
        let old_attrs = editor.doc.block_attrs(resolved.block_index).unwrap_or_default();
        editor.doc.set_block_type(resolved.block_index, node_type, None)?;
        editor.record_undo(UndoOperation::SetBlockType {
            block_index: resolved.block_index,
            old_type,
            new_type: node_type.to_string(),
            old_attrs,
            new_attrs: HashMap::new(),
        });
        // Use mark_structure_changed because changing block type requires replacing
        // the DOM element (e.g., <p> to <h1>), not just updating content
        editor.mark_structure_changed();
        Ok(())
    }

    /// Set the block type with custom attributes at the current cursor position.
    pub fn set_block_type_with_attrs(
        editor: &mut Editor,
        node_type: &str,
        attrs: HashMap<String, String>,
    ) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        let resolved = editor.doc.resolve_position(sel.head)?;
        let old_type = editor.doc.block_type(resolved.block_index).unwrap_or_default();
        let old_attrs = editor.doc.block_attrs(resolved.block_index).unwrap_or_default();
        editor.doc.set_block_type(resolved.block_index, node_type, Some(attrs.clone()))?;
        editor.record_undo(UndoOperation::SetBlockType {
            block_index: resolved.block_index,
            old_type,
            new_type: node_type.to_string(),
            old_attrs,
            new_attrs: attrs,
        });
        // Use mark_structure_changed because changing block type requires replacing
        // the DOM element (e.g., <p> to <h1>), not just updating content
        editor.mark_structure_changed();
        Ok(())
    }

    /// Wrap the selection's block in a new parent node type.
    /// For now, this changes the block type (true wrapping requires nested blocks).
    pub fn wrap_in(editor: &mut Editor, node_type: &str) -> Result<(), EditorError> {
        // Simplified: set block type. Real wrapping would create a parent container.
        Self::set_block_type(editor, node_type)
    }

    /// Lift the selection out of its parent node (reset to paragraph).
    pub fn lift(editor: &mut Editor) -> Result<(), EditorError> {
        Self::set_block_type(editor, "paragraph")
    }

    /// Join the current block with the previous block.
    /// This merges text from the current block into the previous one and removes the current block.
    pub fn join_backward(editor: &mut Editor) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        let resolved = editor.doc.resolve_position(sel.head)?;
        let block_idx = resolved.block_index;

        if block_idx == 0 {
            return Ok(()); // First block, nothing to join with
        }

        let curr_text = editor.doc.block_text(block_idx).unwrap_or_default();

        // Calculate position at end of previous block using canonical helper
        let prev_block_len = editor.doc.block_text(block_idx - 1).map(|t| t.len()).unwrap_or(0);
        let abs_prev_end = editor.doc.block_start_position(block_idx - 1) + prev_block_len;
        // Delete the newline separator and merge
        let delete_start = abs_prev_end;
        let delete_end = abs_prev_end + 1; // the newline
        if editor.doc.block_count() > 1 {
            // Delete from end of prev block text through start of current block text
            editor.doc.delete_range(crate::document::Range::new(delete_start, delete_end))?;
        }

        editor.record_undo(UndoOperation::JoinBlocks {
            position: Position::new(delete_start),
            second_block_text: curr_text,
        });

        // Set cursor at join point
        editor.set_selection(Selection::cursor(Position::new(delete_start)));
        editor.mark_structure_changed();
        Ok(())
    }

    /// Split the current block at the cursor position.
    pub fn split_block(editor: &mut Editor) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();

        // If there's a selection, delete it first
        if !sel.is_cursor() {
            editor.doc.delete_range(sel.range())?;
        }

        let pos = sel.start();

        // Capture moved text for undo (text after split point in current block)
        let resolved = editor.doc.resolve_position(pos)?;
        let block_text = editor.doc.block_text(resolved.block_index).unwrap_or_default();
        let moved_text = block_text[resolved.text_offset..].to_string();

        editor.doc.split_block(pos)?;

        editor.record_undo(UndoOperation::SplitBlock {
            position: pos,
            moved_text,
        });

        // Move cursor to start of new block
        let new_pos = pos + 1; // +1 for the newline separator
        editor.set_selection(Selection::cursor(new_pos));
        editor.mark_structure_changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schema;
    use crate::editor::EditorConfig;

    fn editor_with_text(text: &str) -> Editor {
        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        editor.doc.insert_text(Position::new(0), text).unwrap();
        editor
    }

    #[test]
    fn set_block_type_changes_type() {
        let mut editor = editor_with_text("Hello");
        editor.set_selection(Selection::cursor(Position::new(0)));
        StructureCommands::set_block_type(&mut editor, "heading").unwrap();
        assert_eq!(editor.doc.block_type(0), Some("heading".into()));
    }

    #[test]
    fn lift_resets_to_paragraph() {
        let mut editor = editor_with_text("Hello");
        editor.set_selection(Selection::cursor(Position::new(0)));
        StructureCommands::set_block_type(&mut editor, "heading").unwrap();
        StructureCommands::lift(&mut editor).unwrap();
        assert_eq!(editor.doc.block_type(0), Some("paragraph".into()));
    }

    #[test]
    fn split_block_creates_two_blocks() {
        let mut editor = editor_with_text("HelloWorld");
        editor.set_selection(Selection::cursor(Position::new(5)));
        StructureCommands::split_block(&mut editor).unwrap();
        assert_eq!(editor.doc.block_count(), 2);
        assert_eq!(editor.doc.block_text(0), Some("Hello".into()));
        assert_eq!(editor.doc.block_text(1), Some("World".into()));
    }

    #[test]
    fn split_block_moves_cursor() {
        let mut editor = editor_with_text("HelloWorld");
        editor.set_selection(Selection::cursor(Position::new(5)));
        StructureCommands::split_block(&mut editor).unwrap();
        // Cursor should be at start of new block
        assert_eq!(editor.get_selection().head, Position::new(6));
    }

    #[test]
    fn join_backward_at_first_block_is_noop() {
        let mut editor = editor_with_text("Hello");
        editor.set_selection(Selection::cursor(Position::new(0)));
        StructureCommands::join_backward(&mut editor).unwrap();
        assert_eq!(editor.doc.block_count(), 1);
    }

    #[test]
    fn join_backward_merges_blocks() {
        let mut editor = editor_with_text("Hello");
        editor.doc.split_block(Position::new(5)).unwrap();
        editor.doc.insert_text(Position::new(6), "World").unwrap();
        assert_eq!(editor.doc.block_count(), 2);
        // Cursor at start of second block
        editor.set_selection(Selection::cursor(Position::new(6)));
        StructureCommands::join_backward(&mut editor).unwrap();
        assert_eq!(editor.doc.block_count(), 1);
        assert_eq!(editor.doc.to_text(), "HelloWorld");
    }
}
