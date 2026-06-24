use crate::{EditCommand, EditableDocument, Position, Range, Selection, TextOperation, UndoStack};

/// Orchestrates document, selection, and undo for text editing.
#[derive(Debug)]
pub struct EditableState<D: EditableDocument> {
    pub document: D,
    pub selection: Selection,
    pub undo_stack: UndoStack<TextOperation>,
}

impl<D: EditableDocument + Default> Default for EditableState<D> {
    fn default() -> Self {
        Self::new(D::default())
    }
}

impl<D: EditableDocument> EditableState<D> {
    pub fn new(document: D) -> Self {
        Self {
            document,
            selection: Selection::cursor(0),
            undo_stack: UndoStack::default(),
        }
    }

    /// Execute an edit command and return the clipboard content if Copy/Cut.
    pub fn execute(&mut self, cmd: EditCommand) -> Option<String> {
        match cmd {
            // Text modification
            EditCommand::InsertText(text) => {
                self.insert_text(&text);
                None
            }
            EditCommand::DeleteBackward => {
                self.delete_backward();
                None
            }
            EditCommand::DeleteForward => {
                self.delete_forward();
                None
            }
            EditCommand::DeleteWordBackward => {
                self.delete_word_backward();
                None
            }
            EditCommand::DeleteWordForward => {
                self.delete_word_forward();
                None
            }
            EditCommand::DeleteToLineStart => {
                self.delete_to_line_start();
                None
            }
            EditCommand::DeleteToLineEnd => {
                self.delete_to_line_end();
                None
            }

            // Cursor movement
            EditCommand::MoveLeft => {
                self.move_left(false);
                None
            }
            EditCommand::MoveRight => {
                self.move_right(false);
                None
            }
            EditCommand::MoveUp => {
                self.move_up(false);
                None
            }
            EditCommand::MoveDown => {
                self.move_down(false);
                None
            }
            EditCommand::MoveWordLeft => {
                self.move_word_left(false);
                None
            }
            EditCommand::MoveWordRight => {
                self.move_word_right(false);
                None
            }
            EditCommand::MoveToLineStart => {
                self.move_to_line_start(false);
                None
            }
            EditCommand::MoveToLineEnd => {
                self.move_to_line_end(false);
                None
            }
            EditCommand::MoveToDocStart => {
                self.move_to_doc_start(false);
                None
            }
            EditCommand::MoveToDocEnd => {
                self.move_to_doc_end(false);
                None
            }

            // Selection
            EditCommand::SelectLeft => {
                self.move_left(true);
                None
            }
            EditCommand::SelectRight => {
                self.move_right(true);
                None
            }
            EditCommand::SelectUp => {
                self.move_up(true);
                None
            }
            EditCommand::SelectDown => {
                self.move_down(true);
                None
            }
            EditCommand::SelectWordLeft => {
                self.move_word_left(true);
                None
            }
            EditCommand::SelectWordRight => {
                self.move_word_right(true);
                None
            }
            EditCommand::SelectToLineStart => {
                self.move_to_line_start(true);
                None
            }
            EditCommand::SelectToLineEnd => {
                self.move_to_line_end(true);
                None
            }
            EditCommand::SelectToDocStart => {
                self.move_to_doc_start(true);
                None
            }
            EditCommand::SelectToDocEnd => {
                self.move_to_doc_end(true);
                None
            }
            EditCommand::SelectAll => {
                self.select_all();
                None
            }

            // Clipboard
            EditCommand::Cut => self.cut(),
            EditCommand::Copy => self.copy(),
            EditCommand::Paste(text) => {
                self.insert_text(&text);
                None
            }

            // Undo/Redo
            EditCommand::Undo => {
                self.undo();
                None
            }
            EditCommand::Redo => {
                self.redo();
                None
            }

            // Multi-line
            EditCommand::InsertNewline => {
                self.insert_text("\n");
                None
            }
        }
    }

    // === Text modification ===

    fn insert_text(&mut self, text: &str) {
        let range = self.selection.range();
        let deleted_text = if !range.is_empty() {
            let deleted = self.document.text_slice(range);
            self.document.delete(range);
            self.undo_stack.push(TextOperation::Delete {
                range,
                deleted_text: deleted.clone(),
            });
            Some(deleted)
        } else {
            None
        };

        let pos = range.start;
        self.document.insert(pos, text);
        self.undo_stack.push(TextOperation::Insert {
            pos,
            text: text.to_string(),
        });

        let new_pos = Position(pos.0 + text.len());
        self.selection = Selection::cursor(new_pos);

        // If we deleted text, we need to handle the composite operation
        let _ = deleted_text;
    }

    fn delete_backward(&mut self) {
        if !self.selection.is_cursor() {
            // Delete selection
            self.delete_selection();
        } else if self.selection.head.0 > 0 {
            // Find previous character boundary
            let text = self.document.to_text();
            let pos = self.selection.head.0;
            let prev_pos = text[..pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            let range = Range::new(prev_pos, pos);
            let deleted = self.document.text_slice(range);
            self.document.delete(range);
            self.undo_stack.push(TextOperation::Delete {
                range,
                deleted_text: deleted,
            });
            self.selection = Selection::cursor(prev_pos);
        }
    }

    fn delete_forward(&mut self) {
        if !self.selection.is_cursor() {
            self.delete_selection();
        } else if self.selection.head.0 < self.document.text_length() {
            let text = self.document.to_text();
            let pos = self.selection.head.0;
            let next_pos = text[pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| pos + i)
                .unwrap_or(text.len());
            let range = Range::new(pos, next_pos);
            let deleted = self.document.text_slice(range);
            self.document.delete(range);
            self.undo_stack.push(TextOperation::Delete {
                range,
                deleted_text: deleted,
            });
        }
    }

    fn delete_word_backward(&mut self) {
        if !self.selection.is_cursor() {
            self.delete_selection();
        } else {
            let pos = self.selection.head.0;
            let word_start = self.find_word_start(pos);
            if word_start < pos {
                let range = Range::new(word_start, pos);
                let deleted = self.document.text_slice(range);
                self.document.delete(range);
                self.undo_stack.push(TextOperation::Delete {
                    range,
                    deleted_text: deleted,
                });
                self.selection = Selection::cursor(word_start);
            }
        }
    }

    fn delete_word_forward(&mut self) {
        if !self.selection.is_cursor() {
            self.delete_selection();
        } else {
            let pos = self.selection.head.0;
            let word_end = self.find_word_end(pos);
            if word_end > pos {
                let range = Range::new(pos, word_end);
                let deleted = self.document.text_slice(range);
                self.document.delete(range);
                self.undo_stack.push(TextOperation::Delete {
                    range,
                    deleted_text: deleted,
                });
            }
        }
    }

    fn delete_to_line_start(&mut self) {
        if !self.selection.is_cursor() {
            self.delete_selection();
        } else {
            let pos = self.selection.head.0;
            let line = self.document.line_at(self.selection.head);
            let line_start = self.document.line_start(line).0;
            if line_start < pos {
                let range = Range::new(line_start, pos);
                let deleted = self.document.text_slice(range);
                self.document.delete(range);
                self.undo_stack.push(TextOperation::Delete {
                    range,
                    deleted_text: deleted,
                });
                self.selection = Selection::cursor(line_start);
            }
        }
    }

    fn delete_to_line_end(&mut self) {
        if !self.selection.is_cursor() {
            self.delete_selection();
        } else {
            let pos = self.selection.head.0;
            let line = self.document.line_at(self.selection.head);
            let line_end = self.document.line_end(line).0;
            if line_end > pos {
                let range = Range::new(pos, line_end);
                let deleted = self.document.text_slice(range);
                self.document.delete(range);
                self.undo_stack.push(TextOperation::Delete {
                    range,
                    deleted_text: deleted,
                });
            }
        }
    }

    fn delete_selection(&mut self) {
        let range = self.selection.range();
        if !range.is_empty() {
            let deleted = self.document.text_slice(range);
            self.document.delete(range);
            self.undo_stack.push(TextOperation::Delete {
                range,
                deleted_text: deleted,
            });
            self.selection = Selection::cursor(range.start);
        }
    }

    // === Cursor movement ===

    fn move_left(&mut self, extend: bool) {
        if !extend && !self.selection.is_cursor() {
            // Collapse to start
            self.selection = Selection::cursor(self.selection.start());
        } else if self.selection.head.0 > 0 {
            let text = self.document.to_text();
            let pos = self.selection.head.0;
            let prev_pos = text[..pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            if extend {
                self.selection.head = Position(prev_pos);
            } else {
                self.selection = Selection::cursor(prev_pos);
            }
        }
    }

    fn move_right(&mut self, extend: bool) {
        if !extend && !self.selection.is_cursor() {
            // Collapse to end
            self.selection = Selection::cursor(self.selection.end());
        } else if self.selection.head.0 < self.document.text_length() {
            let text = self.document.to_text();
            let pos = self.selection.head.0;
            let next_pos = text[pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| pos + i)
                .unwrap_or(text.len());
            if extend {
                self.selection.head = Position(next_pos);
            } else {
                self.selection = Selection::cursor(next_pos);
            }
        }
    }

    fn move_up(&mut self, extend: bool) {
        let line = self.document.line_at(self.selection.head);
        if line > 0 {
            let col = self.document.column_at(self.selection.head);
            let prev_line_start = self.document.line_start(line - 1).0;
            let prev_line_end = self.document.line_end(line - 1).0;
            let prev_line_len = prev_line_end - prev_line_start;
            let new_pos = Position(prev_line_start + col.min(prev_line_len));
            if extend {
                self.selection.head = new_pos;
            } else {
                self.selection = Selection::cursor(new_pos);
            }
        } else if !extend {
            self.selection = Selection::cursor(0);
        }
    }

    fn move_down(&mut self, extend: bool) {
        let line = self.document.line_at(self.selection.head);
        if line + 1 < self.document.line_count() {
            let col = self.document.column_at(self.selection.head);
            let next_line_start = self.document.line_start(line + 1).0;
            let next_line_end = self.document.line_end(line + 1).0;
            let next_line_len = next_line_end - next_line_start;
            let new_pos = Position(next_line_start + col.min(next_line_len));
            if extend {
                self.selection.head = new_pos;
            } else {
                self.selection = Selection::cursor(new_pos);
            }
        } else if !extend {
            self.selection = Selection::cursor(self.document.text_length());
        }
    }

    fn move_word_left(&mut self, extend: bool) {
        let pos = self.selection.head.0;
        let word_start = self.find_word_start(pos);
        if extend {
            self.selection.head = Position(word_start);
        } else {
            self.selection = Selection::cursor(word_start);
        }
    }

    fn move_word_right(&mut self, extend: bool) {
        let pos = self.selection.head.0;
        let word_end = self.find_word_end(pos);
        if extend {
            self.selection.head = Position(word_end);
        } else {
            self.selection = Selection::cursor(word_end);
        }
    }

    fn move_to_line_start(&mut self, extend: bool) {
        let line = self.document.line_at(self.selection.head);
        let line_start = self.document.line_start(line);
        if extend {
            self.selection.head = line_start;
        } else {
            self.selection = Selection::cursor(line_start);
        }
    }

    fn move_to_line_end(&mut self, extend: bool) {
        let line = self.document.line_at(self.selection.head);
        let line_end = self.document.line_end(line);
        if extend {
            self.selection.head = line_end;
        } else {
            self.selection = Selection::cursor(line_end);
        }
    }

    fn move_to_doc_start(&mut self, extend: bool) {
        if extend {
            self.selection.head = Position(0);
        } else {
            self.selection = Selection::cursor(0);
        }
    }

    fn move_to_doc_end(&mut self, extend: bool) {
        let end = Position(self.document.text_length());
        if extend {
            self.selection.head = end;
        } else {
            self.selection = Selection::cursor(end);
        }
    }

    fn select_all(&mut self) {
        self.selection = Selection::new(0, self.document.text_length());
    }

    // === Clipboard ===

    fn cut(&mut self) -> Option<String> {
        let text = self.copy();
        self.delete_selection();
        text
    }

    fn copy(&self) -> Option<String> {
        if self.selection.is_cursor() {
            None
        } else {
            Some(self.document.text_slice(self.selection.range()))
        }
    }

    // === Undo/Redo ===

    fn undo(&mut self) {
        if let Some(op) = self.undo_stack.undo() {
            self.apply_operation(&op);
        }
    }

    fn redo(&mut self) {
        if let Some(op) = self.undo_stack.redo() {
            self.apply_operation(&op);
        }
    }

    fn apply_operation(&mut self, op: &TextOperation) {
        match op {
            TextOperation::Insert { pos, text } => {
                self.document.insert(*pos, text);
                self.selection = Selection::cursor(Position(pos.0 + text.len()));
            }
            TextOperation::Delete {
                range,
                deleted_text: _,
            } => {
                self.document.delete(*range);
                self.selection = Selection::cursor(range.start);
            }
        }
    }

    // === Word boundary helpers ===

    fn find_word_start(&self, pos: usize) -> usize {
        let text = self.document.to_text();
        if pos == 0 {
            return 0;
        }

        let before = &text[..pos.min(text.len())];
        let mut chars = before.char_indices().rev();

        // Skip whitespace backwards, then skip non-whitespace to find word start
        for (_, ch) in &mut chars {
            if !ch.is_whitespace() {
                // Found a word char; now scan back past remaining word chars
                for (j, ch2) in chars {
                    if ch2.is_whitespace() {
                        return j + ch2.len_utf8();
                    }
                }
                return 0;
            }
        }

        0
    }

    fn find_word_end(&self, pos: usize) -> usize {
        let text = self.document.to_text();
        let len = text.len();
        if pos >= len {
            return len;
        }

        let after = &text[pos.min(len)..];
        let mut chars = after.char_indices();

        // Skip non-whitespace (word chars) forwards, then skip whitespace
        let mut found_word = false;
        for (i, ch) in &mut chars {
            if ch.is_whitespace() {
                if found_word {
                    return pos + i;
                }
            } else {
                found_word = true;
            }
        }

        len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StringDocument;

    #[test]
    fn test_insert_text() {
        let mut state = EditableState::new(StringDocument::new());
        state.execute(EditCommand::InsertText("hello".to_string()));
        assert_eq!(state.document.to_text(), "hello");
        assert_eq!(state.selection.head.0, 5);
    }

    #[test]
    fn test_delete_backward() {
        let mut state = EditableState::new(StringDocument::with_text("hello"));
        state.selection = Selection::cursor(5);
        state.execute(EditCommand::DeleteBackward);
        assert_eq!(state.document.to_text(), "hell");
    }

    #[test]
    fn test_delete_selection() {
        let mut state = EditableState::new(StringDocument::with_text("hello world"));
        state.selection = Selection::new(0, 6);
        state.execute(EditCommand::DeleteBackward);
        assert_eq!(state.document.to_text(), "world");
    }

    #[test]
    fn test_undo_redo() {
        let mut state = EditableState::new(StringDocument::new());
        state.execute(EditCommand::InsertText("hello".to_string()));
        assert_eq!(state.document.to_text(), "hello");

        state.execute(EditCommand::Undo);
        assert_eq!(state.document.to_text(), "");

        state.execute(EditCommand::Redo);
        assert_eq!(state.document.to_text(), "hello");
    }

    #[test]
    fn test_select_all() {
        let mut state = EditableState::new(StringDocument::with_text("hello"));
        state.execute(EditCommand::SelectAll);
        assert_eq!(state.selection.start().0, 0);
        assert_eq!(state.selection.end().0, 5);
    }
}
