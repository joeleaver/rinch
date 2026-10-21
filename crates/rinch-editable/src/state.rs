use crate::{
    EditCommand, EditableDocument, Position, Range, Selection, TextOperation, UndoMark, UndoStack,
};

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
    ///
    /// A command is **one** undo step however many operations it made — typing
    /// over a selection deletes and then inserts, and one undo brings the
    /// selected text back (issue #288).
    pub fn execute(&mut self, cmd: EditCommand) -> Option<String> {
        if matches!(cmd, EditCommand::Undo | EditCommand::Redo) {
            return self.execute_ungrouped(cmd);
        }
        let mark = self.undo_stack.mark();
        let out = self.execute_ungrouped(cmd);
        self.group_undo_since(mark);
        out
    }

    /// Fold every undo entry pushed since `mark` into one [`TextOperation::Group`],
    /// so they undo and redo as a single step. Nothing to fold, or one entry,
    /// leaves the stack as it is.
    ///
    /// A host uses it to make a command and whatever it caused *outside* the
    /// engine one step — rinch's desktop runtime folds the rewrite a controlled
    /// field's `oninput` makes in answer to a keystroke into that keystroke, so
    /// undo never replays the raw text a normalizer is about to rewrite again.
    pub fn group_undo_since(&mut self, mark: UndoMark) {
        let mut ops = self.undo_stack.take_since(mark);
        match ops.len() {
            0 => {}
            1 => self.undo_stack.push(ops.pop().expect("one entry")),
            _ => self.undo_stack.push(TextOperation::Group(ops)),
        }
    }

    fn execute_ungrouped(&mut self, cmd: EditCommand) -> Option<String> {
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
            // An emoji sequence goes whole, any other cluster one code point
            // at a time, as in Chrome (#1684).
            let text = self.document.to_text();
            let pos = self.selection.head.0;
            let prev_pos = rinch_words::backspace_start(&text, pos);
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
            // The whole grapheme cluster after the caret (#1684).
            let text = self.document.to_text();
            let pos = self.selection.head.0;
            let next_pos = rinch_words::next_grapheme_boundary(&text, pos);
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
            let word_start = self.word_delete_start(pos);
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
            let word_end = self.word_delete_end(pos);
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
            // One grapheme cluster (#1684).
            let text = self.document.to_text();
            let pos = self.selection.head.0;
            let prev_pos = rinch_words::prev_grapheme_boundary(&text, pos);
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
            // One grapheme cluster (#1684).
            let text = self.document.to_text();
            let pos = self.selection.head.0;
            let next_pos = rinch_words::next_grapheme_boundary(&text, pos);
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
            TextOperation::Group(ops) => {
                for op in ops {
                    self.apply_operation(op);
                }
            }
        }
    }

    // === Word boundary helpers ===

    /// Where a backward word delete from `pos` stops: the word start, but
    /// never across a line break. Right after one it deletes just the break
    /// (Chrome 155: `"ab \n|"` → `"ab "`, `"a\n|b"` → `"ab"`).
    fn word_delete_start(&self, pos: usize) -> usize {
        let text = self.document.to_text();
        let before = &text[..pos.min(text.len())];
        match before.rfind('\n') {
            Some(nl) if nl + 1 == before.len() => nl,
            Some(nl) => self.find_word_start(pos).max(nl + 1),
            None => self.find_word_start(pos),
        }
    }

    /// Where a forward word delete from `pos` stops: the word end, but never
    /// across a line break. Right before one it deletes just the break.
    fn word_delete_end(&self, pos: usize) -> usize {
        let text = self.document.to_text();
        let pos = pos.min(text.len());
        match text[pos..].find('\n') {
            Some(0) => pos + 1,
            Some(nl) => self.find_word_end(pos).min(pos + nl),
            None => self.find_word_end(pos),
        }
    }

    /// Where Ctrl+Left lands from `pos`: the start of the word before it, a
    /// word being what the rich editor's word motion steps over
    /// ([`rinch_words::text_word_boundary`], #1619) — so `foo-bar|` stops at
    /// `foo-|bar`. A line break is one stop: at a line's start it lands just
    /// before the break (the word *delete* still never crosses one, above).
    fn find_word_start(&self, pos: usize) -> usize {
        rinch_words::text_word_boundary(&self.document.to_text(), pos, false)
    }

    /// Where Ctrl+Right lands from `pos`: the end of the word after it (a
    /// line break is a word of its own).
    fn find_word_end(&self, pos: usize) -> usize {
        rinch_words::text_word_boundary(&self.document.to_text(), pos, true)
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

    /// Typing over a selection deletes and inserts; one undo brings the
    /// selected text back and one redo retypes (#288).
    #[test]
    fn replacing_a_selection_is_one_undo_step() {
        let mut state = EditableState::new(StringDocument::with_text("hello world"));
        state.selection = Selection::new(6, 11);
        state.execute(EditCommand::InsertText("there".into()));
        assert_eq!(state.document.to_text(), "hello there");
        state.execute(EditCommand::Undo);
        assert_eq!(state.document.to_text(), "hello world");
        state.execute(EditCommand::Redo);
        assert_eq!(state.document.to_text(), "hello there");
        assert_eq!(state.selection, Selection::cursor(11));
    }

    /// A host folds a command and what followed it into one step; the group
    /// undoes last-first, so an insert followed by a rewrite of it comes back
    /// out in the right order (#288).
    #[test]
    fn a_host_group_undoes_last_first() {
        let mut state = EditableState::new(StringDocument::new());
        state.execute(EditCommand::InsertText("x".into()));
        let mark = state.undo_stack.mark();
        state.execute(EditCommand::InsertText("a".into()));
        state.adopt_text("xA");
        state.group_undo_since(mark);
        assert_eq!(state.document.to_text(), "xA");

        state.execute(EditCommand::Undo);
        assert_eq!(
            state.document.to_text(),
            "x",
            "the keystroke and its rewrite"
        );
        state.execute(EditCommand::Undo);
        assert_eq!(state.document.to_text(), "");
        state.execute(EditCommand::Redo);
        state.execute(EditCommand::Redo);
        assert_eq!(state.document.to_text(), "xA");
        assert_eq!(state.selection, Selection::cursor(2));
    }

    /// Undo and redo are not pushes: a mark taken before one groups nothing it
    /// did.
    #[test]
    fn undo_and_redo_do_not_move_the_mark() {
        let mut state = EditableState::new(StringDocument::new());
        state.execute(EditCommand::InsertText("a".into()));
        state.execute(EditCommand::InsertText("b".into()));
        let mark = state.undo_stack.mark();
        state.execute(EditCommand::Undo);
        state.execute(EditCommand::Redo);
        assert_eq!(state.undo_stack.mark(), mark);
        state.group_undo_since(mark);
        state.execute(EditCommand::Undo);
        assert_eq!(state.document.to_text(), "a", "still two separate steps");
    }

    #[test]
    fn test_select_all() {
        let mut state = EditableState::new(StringDocument::with_text("hello"));
        state.execute(EditCommand::SelectAll);
        assert_eq!(state.selection.start().0, 0);
        assert_eq!(state.selection.end().0, 5);
    }

    /// A word delete never crosses a line break (#951's review): right beside
    /// one it takes the break alone, and further along it stops at it.
    #[test]
    fn a_word_delete_stops_at_a_line_break() {
        let run = |text: &str, at: usize, cmd: EditCommand| {
            let mut state = EditableState::new(StringDocument::with_text(text));
            state.selection = Selection::cursor(at);
            state.execute(cmd);
            state.document.to_text()
        };
        assert_eq!(run("ab \n", 4, EditCommand::DeleteWordBackward), "ab ");
        assert_eq!(run("a\nb", 2, EditCommand::DeleteWordBackward), "ab");
        assert_eq!(
            run("ab\n  cd", 7, EditCommand::DeleteWordBackward),
            "ab\n  "
        );
        assert_eq!(run("ab\n   ", 6, EditCommand::DeleteWordBackward), "ab\n");
        assert_eq!(run("ab\ncd", 2, EditCommand::DeleteWordForward), "abcd");
        assert_eq!(
            run("ab   \ncd", 2, EditCommand::DeleteWordForward),
            "ab\ncd"
        );
        assert_eq!(run("one two", 7, EditCommand::DeleteWordBackward), "one ");
    }
}
