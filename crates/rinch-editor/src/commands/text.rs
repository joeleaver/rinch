//! Text manipulation commands.

use crate::document::{MarkData, Position, Range};
use crate::editor::Editor;
use crate::error::EditorError;
use crate::history::UndoOperation;
use crate::selection::Selection;

/// Commands for text insertion, deletion, and manipulation.
#[derive(Debug)]
pub struct TextCommands;

impl TextCommands {
    /// Insert text at the current selection.
    /// If there is a selection range, the selected text is replaced.
    pub fn insert_text(editor: &mut Editor, text: &str) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        // Delete selected text first if range selection
        if !sel.is_cursor() {
            let range = sel.range();
            let deleted_text = editor.text_in_range(range);
            editor.doc.delete_range(range)?;
            editor.record_undo(UndoOperation::DeleteText {
                range,
                deleted_text,
            });
        }
        let insert_pos = sel.start();

        // Insert text with mark-awareness:
        // - Some(marks): insert with exactly these marks (creates new inline if needed)
        // - None: inherit marks from cursor context (insert into existing inline)
        if let Some(ref desired_marks) = editor.stored_marks {
            let mark_data: Vec<MarkData> = desired_marks
                .iter()
                .map(MarkData::new)
                .collect();
            editor
                .doc
                .insert_text_with_marks(insert_pos, text, &mark_data)?;
        } else {
            editor.doc.insert_text(insert_pos, text)?;
        }

        // Record undo operation (merge single chars for typing)
        let op = UndoOperation::InsertText {
            position: insert_pos,
            text: text.to_string(),
        };
        if text.chars().count() == 1 && !text.contains('\n') {
            editor.record_typing(op);
        } else {
            editor.record_undo(op);
        }

        // Move cursor to end of inserted text
        let new_pos = insert_pos + text.len();
        editor.set_selection(Selection::cursor(new_pos));

        Ok(())
    }

    /// Delete text in the given range.
    pub fn delete_range(editor: &mut Editor, range: Range) -> Result<(), EditorError> {
        let deleted_text = editor.text_in_range(range);
        editor.doc.delete_range(range)?;
        editor.record_undo(UndoOperation::DeleteText {
            range,
            deleted_text,
        });
        editor.set_selection(Selection::cursor(range.start));
        Ok(())
    }

    /// Replace text in the given range with new text.
    pub fn replace_text(editor: &mut Editor, range: Range, text: &str) -> Result<(), EditorError> {
        let deleted_text = editor.text_in_range(range);
        editor.doc.delete_range(range)?;
        editor.record_undo(UndoOperation::DeleteText {
            range,
            deleted_text,
        });
        editor.doc.insert_text(range.start, text)?;
        editor.record_undo(UndoOperation::InsertText {
            position: range.start,
            text: text.to_string(),
        });
        let new_pos = range.start + text.len();
        editor.set_selection(Selection::cursor(new_pos));
        Ok(())
    }

    /// Delete backwards (backspace).
    pub fn delete_backward(editor: &mut Editor) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        if !sel.is_cursor() {
            // Delete selection
            return Self::delete_range(editor, sel.range());
        }
        let pos = sel.head;
        if pos.0 == 0 {
            return Ok(()); // At start of document
        }
        let range = Range::new(pos - 1, pos);
        Self::delete_range(editor, range)
    }

    /// Delete forwards (delete key).
    pub fn delete_forward(editor: &mut Editor) -> Result<(), EditorError> {
        let sel = editor.get_selection().clone();
        if !sel.is_cursor() {
            return Self::delete_range(editor, sel.range());
        }
        let pos = sel.head;
        let doc_len = editor.doc.text_length();
        if pos.0 >= doc_len {
            return Ok(()); // At end of document
        }
        let range = Range::new(pos, pos + 1);
        Self::delete_range(editor, range)
    }

    /// Move cursor to position.
    pub fn move_cursor(editor: &mut Editor, pos: Position) -> Result<(), EditorError> {
        // Validate position
        let doc_len = editor.doc.text_length();
        if pos.0 > doc_len {
            return Err(EditorError::InvalidPosition(pos.0, doc_len));
        }
        editor.set_selection(Selection::cursor(pos));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::EditorConfig;
    use crate::schema::Schema;

    fn test_editor() -> Editor {
        Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap()
    }

    fn editor_with_text(text: &str) -> Editor {
        let mut editor = test_editor();
        editor.doc.insert_text(Position::new(0), text).unwrap();
        editor.set_selection(Selection::cursor(Position::new(text.len())));
        editor
    }

    #[test]
    fn insert_text_at_cursor() {
        let mut editor = test_editor();
        TextCommands::insert_text(&mut editor, "Hello").unwrap();
        assert_eq!(editor.doc.to_text(), "Hello");
        assert_eq!(editor.get_selection().head, Position::new(5));
    }

    #[test]
    fn insert_text_replaces_selection() {
        let mut editor = editor_with_text("Hello World");
        editor.set_selection(Selection::new(Position::new(5), Position::new(11)));
        TextCommands::insert_text(&mut editor, " Rust").unwrap();
        assert_eq!(editor.doc.to_text(), "Hello Rust");
    }

    #[test]
    fn delete_range_moves_cursor() {
        let mut editor = editor_with_text("Hello World");
        TextCommands::delete_range(&mut editor, Range::new(5usize, 11usize)).unwrap();
        assert_eq!(editor.doc.to_text(), "Hello");
        assert_eq!(editor.get_selection().head, Position::new(5));
    }

    #[test]
    fn replace_text_works() {
        let mut editor = editor_with_text("Hello World");
        TextCommands::replace_text(&mut editor, Range::new(6usize, 11usize), "Rust").unwrap();
        assert_eq!(editor.doc.to_text(), "Hello Rust");
        assert_eq!(editor.get_selection().head, Position::new(10));
    }

    #[test]
    fn delete_backward_at_start_is_noop() {
        let mut editor = test_editor();
        editor.set_selection(Selection::cursor(Position::new(0)));
        TextCommands::delete_backward(&mut editor).unwrap();
        assert_eq!(editor.doc.to_text(), "");
    }

    #[test]
    fn delete_backward_removes_char() {
        let mut editor = editor_with_text("Hi");
        editor.set_selection(Selection::cursor(Position::new(2)));
        TextCommands::delete_backward(&mut editor).unwrap();
        assert_eq!(editor.doc.to_text(), "H");
    }

    #[test]
    fn delete_forward_at_end_is_noop() {
        let mut editor = editor_with_text("Hi");
        editor.set_selection(Selection::cursor(Position::new(2)));
        TextCommands::delete_forward(&mut editor).unwrap();
        assert_eq!(editor.doc.to_text(), "Hi");
    }

    #[test]
    fn delete_forward_removes_char() {
        let mut editor = editor_with_text("Hi");
        editor.set_selection(Selection::cursor(Position::new(0)));
        TextCommands::delete_forward(&mut editor).unwrap();
        assert_eq!(editor.doc.to_text(), "i");
    }

    #[test]
    fn move_cursor_valid() {
        let mut editor = editor_with_text("Hello");
        TextCommands::move_cursor(&mut editor, Position::new(3)).unwrap();
        assert_eq!(editor.get_selection().head, Position::new(3));
    }

    #[test]
    fn move_cursor_invalid() {
        let mut editor = editor_with_text("Hi");
        assert!(TextCommands::move_cursor(&mut editor, Position::new(100)).is_err());
    }
}
