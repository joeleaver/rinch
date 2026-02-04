use crate::{EditableDocument, Position, Range};

/// Multi-line document for textarea-style inputs.
#[derive(Debug, Clone)]
pub struct MultilineDocument {
    text: String,
    /// Cached line start byte positions for O(1) lookups.
    line_starts: Vec<usize>,
}

impl Default for MultilineDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl MultilineDocument {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            line_starts: vec![0],
        }
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let line_starts = Self::compute_line_starts(&text);
        Self { text, line_starts }
    }

    fn compute_line_starts(text: &str) -> Vec<usize> {
        let mut starts = vec![0];
        for (i, c) in text.char_indices() {
            if c == '\n' {
                starts.push(i + 1);
            }
        }
        starts
    }

    fn recompute_lines(&mut self) {
        self.line_starts = Self::compute_line_starts(&self.text);
    }
}

impl EditableDocument for MultilineDocument {
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
        self.recompute_lines();
    }

    fn delete(&mut self, range: Range) {
        let start = range.start.0.min(self.text.len());
        let end = range.end.0.min(self.text.len());
        if start < end {
            self.text.drain(start..end);
            self.recompute_lines();
        }
    }

    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    fn line_at(&self, pos: Position) -> usize {
        match self.line_starts.binary_search(&pos.0) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    fn line_start(&self, line: usize) -> Position {
        Position(
            self.line_starts
                .get(line)
                .copied()
                .unwrap_or(self.text.len()),
        )
    }

    fn line_end(&self, line: usize) -> Position {
        let start = self.line_start(line).0;
        let next_start = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        // End is before the newline (or at text end)
        let end = if next_start > 0
            && next_start <= self.text.len()
            && self.text.as_bytes().get(next_start - 1) == Some(&b'\n')
        {
            next_start - 1
        } else {
            next_start
        };
        Position(end.max(start))
    }
}
