use crate::{Position, Range};

/// Trait for documents that support basic text editing.
pub trait EditableDocument {
    /// Get total text length in bytes.
    fn text_length(&self) -> usize;

    /// Get text in a range.
    fn text_slice(&self, range: Range) -> String;

    /// Get all text as a string.
    fn to_text(&self) -> String {
        self.text_slice(Range::new(0, self.text_length()))
    }

    /// Insert text at position.
    fn insert(&mut self, pos: Position, text: &str);

    /// Delete text in range.
    fn delete(&mut self, range: Range);

    /// Replace text in range with new text.
    fn replace(&mut self, range: Range, text: &str) {
        self.delete(range);
        self.insert(range.start, text);
    }

    // Line operations

    /// Get number of lines.
    fn line_count(&self) -> usize;

    /// Get the line index containing a position.
    fn line_at(&self, pos: Position) -> usize;

    /// Get start position of a line.
    fn line_start(&self, line: usize) -> Position;

    /// Get end position of a line (before newline).
    fn line_end(&self, line: usize) -> Position;

    /// Get column offset within a line.
    fn column_at(&self, pos: Position) -> usize {
        let line = self.line_at(pos);
        pos.0.saturating_sub(self.line_start(line).0)
    }
}
