//! Document fragment for representing parts of documents.

use super::model::MarkData;
use std::collections::HashMap;

/// A fragment of document content (for clipboard, drag-drop, etc.).
#[derive(Clone, Debug)]
pub struct Fragment {
    /// The blocks in this fragment.
    pub blocks: Vec<FragmentBlock>,
}

impl Fragment {
    /// Create a new empty fragment.
    pub fn empty() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Create a fragment from plain text. Each line becomes a paragraph block.
    pub fn from_text(text: &str) -> Self {
        let blocks = text
            .split('\n')
            .map(|line| FragmentBlock {
                block_type: "paragraph".to_string(),
                attrs: HashMap::new(),
                content: vec![FragmentInline::Text {
                    text: line.to_string(),
                    marks: Vec::new(),
                }],
            })
            .collect();
        Self { blocks }
    }

    /// Get the total character count of this fragment.
    pub fn size(&self) -> usize {
        let text_len: usize = self.blocks.iter().map(|b| b.text_length()).sum();
        let separators = self.blocks.len().saturating_sub(1);
        text_len + separators
    }

    /// Convert fragment to plain text.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Check if the fragment is empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty() || self.blocks.iter().all(|b| b.content.is_empty())
    }
}

/// A block within a fragment.
#[derive(Clone, Debug)]
pub struct FragmentBlock {
    /// Block type (e.g., "paragraph", "heading")
    pub block_type: String,
    /// Block attributes
    pub attrs: HashMap<String, String>,
    /// Inline content
    pub content: Vec<FragmentInline>,
}

impl FragmentBlock {
    /// Get text length of this block.
    pub fn text_length(&self) -> usize {
        self.content.iter().map(|i| i.text_length()).sum()
    }

    /// Get plain text of this block.
    pub fn text(&self) -> String {
        self.content.iter().map(|i| i.text()).collect()
    }
}

/// Inline content within a fragment block.
#[derive(Clone, Debug)]
pub enum FragmentInline {
    /// A text run with optional marks.
    Text { text: String, marks: Vec<MarkData> },
    /// A hard line break.
    HardBreak,
}

impl FragmentInline {
    /// Get the text length of this inline.
    pub fn text_length(&self) -> usize {
        match self {
            FragmentInline::Text { text, .. } => text.len(),
            FragmentInline::HardBreak => 1,
        }
    }

    /// Get the text of this inline.
    pub fn text(&self) -> String {
        match self {
            FragmentInline::Text { text, .. } => text.clone(),
            FragmentInline::HardBreak => "\n".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_fragment() {
        let f = Fragment::empty();
        assert!(f.is_empty());
        assert_eq!(f.size(), 0);
        assert_eq!(f.text(), "");
    }

    #[test]
    fn from_text_single_line() {
        let f = Fragment::from_text("Hello");
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.text(), "Hello");
        assert_eq!(f.size(), 5);
    }

    #[test]
    fn from_text_multiple_lines() {
        let f = Fragment::from_text("Hello\nWorld");
        assert_eq!(f.blocks.len(), 2);
        assert_eq!(f.text(), "Hello\nWorld");
        assert_eq!(f.size(), 11); // 5 + 1 (separator) + 5
    }

    #[test]
    fn fragment_block_type() {
        let f = Fragment::from_text("Hello");
        assert_eq!(f.blocks[0].block_type, "paragraph");
    }
}
