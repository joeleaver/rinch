use crate::{EditableDocument, Position, Range};

/// Simple String-based document for single-line inputs.
#[derive(Debug, Clone, Default)]
pub struct StringDocument {
    text: String,
}

impl StringDocument {
    pub fn new() -> Self {
        Self {
            text: String::new(),
        }
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl EditableDocument for StringDocument {
    fn text_length(&self) -> usize {
        self.text.len()
    }

    fn text_slice(&self, range: Range) -> String {
        let start = range.start.0.min(self.text.len());
        let end = range.end.0.min(self.text.len());
        self.text[start..end].to_string()
    }

    fn to_text(&self) -> String {
        self.text.clone()
    }

    fn insert(&mut self, pos: Position, text: &str) {
        let pos = pos.0.min(self.text.len());
        self.text.insert_str(pos, text);
    }

    fn delete(&mut self, range: Range) {
        let start = range.start.0.min(self.text.len());
        let end = range.end.0.min(self.text.len());
        if start < end {
            self.text.drain(start..end);
        }
    }

    fn line_count(&self) -> usize {
        1
    }

    fn line_at(&self, _pos: Position) -> usize {
        0
    }

    fn line_start(&self, _line: usize) -> Position {
        Position(0)
    }

    fn line_end(&self, _line: usize) -> Position {
        Position(self.text.len())
    }
}
