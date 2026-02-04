//! Formatting commands (marks and inline styles).

use std::collections::HashMap;

use crate::document::MarkData;
use crate::editor::Editor;
use crate::error::EditorError;
use crate::history::UndoOperation;

/// Commands for applying and removing formatting marks.
#[derive(Debug)]
pub struct FormattingCommands;

impl FormattingCommands {
    /// Toggle a mark on the current selection.
    /// If the selection already has the mark, remove it; otherwise add it.
    pub fn toggle_mark(editor: &mut Editor, mark_type: &str) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        if sel.is_empty() {
            // Cursor position: toggle stored mark for next insertion
            editor.toggle_stored_mark(mark_type);
            return Ok(());
        }
        let range = sel.range();

        // Check if mark exists at the start of the selection
        let marks = editor.doc.marks_at(range.start);
        let has_mark = marks.iter().any(|m| m.mark_type == mark_type);

        if has_mark {
            editor.doc.remove_mark(range, mark_type)?;
            editor.record_undo(UndoOperation::RemoveMark {
                range,
                mark_type: mark_type.to_string(),
            });
        } else {
            editor.doc.add_mark(range, MarkData::new(mark_type))?;
            editor.record_undo(UndoOperation::AddMark {
                range,
                mark_type: mark_type.to_string(),
                attrs: HashMap::new(),
            });
        }

        // Track affected blocks
        if let Ok(start_rp) = editor.doc.resolve_position(range.start)
            && let Ok(end_rp) = editor.doc.resolve_position(range.end)
        {
            for bi in start_rp.block_index..=end_rp.block_index {
                editor.mark_block_changed(bi);
            }
        }

        Ok(())
    }

    /// Add a mark to the current selection.
    pub fn add_mark(editor: &mut Editor, mark_type: &str) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        if sel.is_empty() {
            return Ok(());
        }
        let range = sel.range();
        editor.doc.add_mark(range, MarkData::new(mark_type))?;

        // Track affected blocks
        if let Ok(start_rp) = editor.doc.resolve_position(range.start)
            && let Ok(end_rp) = editor.doc.resolve_position(range.end)
        {
            for bi in start_rp.block_index..=end_rp.block_index {
                editor.mark_block_changed(bi);
            }
        }

        Ok(())
    }

    /// Remove a mark from the current selection.
    pub fn remove_mark(editor: &mut Editor, mark_type: &str) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        if sel.is_empty() {
            return Ok(());
        }
        let range = sel.range();
        editor.doc.remove_mark(range, mark_type)?;

        // Track affected blocks
        if let Ok(start_rp) = editor.doc.resolve_position(range.start)
            && let Ok(end_rp) = editor.doc.resolve_position(range.end)
        {
            for bi in start_rp.block_index..=end_rp.block_index {
                editor.mark_block_changed(bi);
            }
        }

        Ok(())
    }

    /// Clear all formatting from the current selection.
    pub fn clear_formatting(editor: &mut Editor) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        if sel.is_empty() {
            return Ok(());
        }
        let range = sel.range();

        // Get all known mark types from schema and remove each
        let mark_types: Vec<String> = editor.schema.marks.keys().cloned().collect();
        for mark_type in &mark_types {
            editor.doc.remove_mark(range, mark_type)?;
        }

        // Track affected blocks (conservative - could affect multiple blocks)
        editor.mark_structure_changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Position;
    use crate::editor::EditorConfig;
    use crate::schema::Schema;
    use crate::selection::Selection;

    fn editor_with_text(text: &str) -> Editor {
        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        editor.doc.insert_text(Position::new(0), text).unwrap();
        editor
    }

    #[test]
    fn toggle_mark_adds_when_absent() {
        let mut editor = editor_with_text("Hello");
        editor.set_selection(Selection::new(Position::new(0), Position::new(5)));
        FormattingCommands::toggle_mark(&mut editor, "bold").unwrap();
        let marks = editor.doc.marks_at(Position::new(0));
        assert!(marks.iter().any(|m| m.mark_type == "bold"));
    }

    #[test]
    fn toggle_mark_removes_when_present() {
        let mut editor = editor_with_text("Hello");
        editor.set_selection(Selection::new(Position::new(0), Position::new(5)));
        FormattingCommands::add_mark(&mut editor, "bold").unwrap();
        FormattingCommands::toggle_mark(&mut editor, "bold").unwrap();
        let marks = editor.doc.marks_at(Position::new(0));
        assert!(!marks.iter().any(|m| m.mark_type == "bold"));
    }

    #[test]
    fn add_mark_on_empty_selection_is_noop() {
        let mut editor = editor_with_text("Hello");
        editor.set_selection(Selection::cursor(Position::new(0)));
        FormattingCommands::add_mark(&mut editor, "bold").unwrap();
        // No crash, no mark added
    }

    #[test]
    fn remove_mark_works() {
        let mut editor = editor_with_text("Hello");
        editor.set_selection(Selection::new(Position::new(0), Position::new(5)));
        FormattingCommands::add_mark(&mut editor, "bold").unwrap();
        FormattingCommands::remove_mark(&mut editor, "bold").unwrap();
        let marks = editor.doc.marks_at(Position::new(0));
        assert!(marks.is_empty());
    }

    #[test]
    fn clear_formatting_removes_all_marks() {
        let mut editor = editor_with_text("Hello");
        editor.set_selection(Selection::new(Position::new(0), Position::new(5)));
        FormattingCommands::add_mark(&mut editor, "bold").unwrap();
        FormattingCommands::add_mark(&mut editor, "italic").unwrap();
        FormattingCommands::clear_formatting(&mut editor).unwrap();
        let marks = editor.doc.marks_at(Position::new(0));
        assert!(marks.is_empty());
    }
}
