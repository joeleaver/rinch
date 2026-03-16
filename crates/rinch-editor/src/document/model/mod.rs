//! Core document model using Automerge CRDT for collaboration.

mod mutations;
mod queries;
pub(crate) mod serialization;
#[cfg(test)]
mod roundtrip_tests;

use std::collections::HashMap;

use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, transaction::Transactable};

/// Mark data (type + optional attributes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkData {
    /// The mark type (e.g., "bold", "italic", "link")
    pub mark_type: String,
    /// Optional attributes (e.g., href for links)
    pub attrs: HashMap<String, String>,
}

impl MarkData {
    /// Create a new mark with no attributes.
    pub fn new(mark_type: impl Into<String>) -> Self {
        Self {
            mark_type: mark_type.into(),
            attrs: HashMap::new(),
        }
    }

    /// Create a new mark with attributes.
    pub fn with_attrs(mark_type: impl Into<String>, attrs: HashMap<String, String>) -> Self {
        Self {
            mark_type: mark_type.into(),
            attrs,
        }
    }
}

/// A run of text with its formatting marks, for rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineRun {
    /// The text content (or empty for hard_break)
    pub text: String,
    /// The type of inline node: "text" or "hard_break"
    pub inline_type: String,
    /// Marks applied to this run
    pub marks: Vec<MarkData>,
}

/// The main document structure backed by Automerge CRDT.
///
/// The document stores content as a list of blocks, where each block contains
/// a list of inline nodes (text, hard breaks, etc.). This structure maps
/// directly to an Automerge document for real-time collaboration.
///
/// # Automerge Structure
///
/// ```text
/// Root Map:
///   "content" -> List of Block Maps:
///     Each Block Map:
///       "type" -> String (e.g., "paragraph", "heading")
///       "attrs" -> Map of attributes
///       "content" -> List of Inline Maps:
///         Each Inline Map:
///           "type" -> "text" | "hard_break" | "image"
///           "text" -> String (for text nodes)
///           "marks" -> List of Mark Maps
/// ```
#[derive(Debug)]
pub struct EditorDocument {
    /// Automerge document handle
    pub(crate) doc: AutoCommit,
    /// ObjId for the root "content" array
    pub(crate) content_id: ObjId,
}

impl Default for EditorDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorDocument {
    /// Create a new empty document with a single empty paragraph.
    pub fn new() -> Self {
        let mut doc = AutoCommit::new();
        let content_id = doc
            .put_object(&automerge::ROOT, "content", ObjType::List)
            .unwrap();
        // Add initial empty paragraph
        let block = doc.insert_object(&content_id, 0, ObjType::Map).unwrap();
        doc.put(&block, "type", "paragraph").unwrap();
        doc.put_object(&block, "attrs", ObjType::Map).unwrap();
        let inline_content = doc.put_object(&block, "content", ObjType::List).unwrap();
        let text_node = doc.insert_object(&inline_content, 0, ObjType::Map).unwrap();
        doc.put(&text_node, "type", "text").unwrap();
        doc.put(&text_node, "text", "").unwrap();
        doc.put_object(&text_node, "marks", ObjType::List).unwrap();

        Self { doc, content_id }
    }

    /// Get the ObjId of a block at the given index.
    pub(crate) fn block_obj(&self, index: usize) -> Option<ObjId> {
        self.doc
            .get(&self.content_id, index)
            .ok()
            .flatten()
            .and_then(|(val, id)| {
                if matches!(val, automerge::Value::Object(ObjType::Map)) {
                    Some(id)
                } else {
                    None
                }
            })
    }

    /// Get the "content" list ObjId of a block.
    pub(crate) fn block_content_obj(&self, block_id: &ObjId) -> Option<ObjId> {
        self.doc
            .get(block_id, "content")
            .ok()
            .flatten()
            .and_then(|(val, id)| {
                if matches!(val, automerge::Value::Object(ObjType::List)) {
                    Some(id)
                } else {
                    None
                }
            })
    }

    /// Read marks from an inline node.
    pub(crate) fn read_marks(&self, inline_id: &ObjId) -> Vec<MarkData> {
        let marks_id =
            match self
                .doc
                .get(inline_id, "marks")
                .ok()
                .flatten()
                .and_then(|(val, id)| {
                    if matches!(val, automerge::Value::Object(ObjType::List)) {
                        Some(id)
                    } else {
                        None
                    }
                }) {
                Some(id) => id,
                None => return Vec::new(),
            };

        let mut marks = Vec::new();
        let len = self.doc.length(&marks_id);
        for i in 0..len {
            if let Some((_, mark_id)) = self.doc.get(&marks_id, i).ok().flatten() {
                let mark_type = self.get_str(&mark_id, "type").unwrap_or_default();
                let mut attrs = HashMap::new();
                if let Some((_, attrs_id)) = self.doc.get(&mark_id, "attrs").ok().flatten() {
                    for key in self.doc.keys(&attrs_id) {
                        if let Some(val) = self.get_str(&attrs_id, key.as_str()) {
                            attrs.insert(key, val);
                        }
                    }
                }
                marks.push(MarkData { mark_type, attrs });
            }
        }
        marks
    }

    /// Helper: get a string value from an automerge object.
    pub(crate) fn get_str(&self, obj: &ObjId, key: &str) -> Option<String> {
        self.doc.get(obj, key).ok().flatten().and_then(|(val, _)| {
            if let automerge::Value::Scalar(s) = val
                && let automerge::ScalarValue::Str(smol) = s.as_ref()
            {
                return Some(smol.to_string());
            }
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Position, Range};

    #[test]
    fn new_creates_single_empty_paragraph() {
        let doc = EditorDocument::new();
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block_type(0), Some("paragraph".into()));
        assert_eq!(doc.block_text(0), Some(String::new()));
    }

    #[test]
    fn text_length_empty_doc() {
        let doc = EditorDocument::new();
        assert_eq!(doc.text_length(), 0);
    }

    #[test]
    fn to_text_empty_doc() {
        let doc = EditorDocument::new();
        assert_eq!(doc.to_text(), "");
    }

    #[test]
    fn insert_text_at_start() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        assert_eq!(doc.block_text(0), Some("Hello".into()));
        assert_eq!(doc.to_text(), "Hello");
        assert_eq!(doc.text_length(), 5);
    }

    #[test]
    fn insert_text_at_middle() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Heo").unwrap();
        doc.insert_text(Position(2), "ll").unwrap();
        assert_eq!(doc.block_text(0), Some("Hello".into()));
    }

    #[test]
    fn insert_text_at_end() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.insert_text(Position(5), " World").unwrap();
        assert_eq!(doc.to_text(), "Hello World");
    }

    #[test]
    fn delete_range_within_block() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello World").unwrap();
        doc.delete_range(Range::new(5usize, 11usize)).unwrap();
        assert_eq!(doc.to_text(), "Hello");
    }

    #[test]
    fn delete_range_entire_text() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.delete_range(Range::new(0usize, 5usize)).unwrap();
        assert_eq!(doc.to_text(), "");
    }

    #[test]
    fn delete_range_empty_is_noop() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.delete_range(Range::collapsed(2usize)).unwrap();
        assert_eq!(doc.to_text(), "Hello");
    }

    #[test]
    fn split_block_at_middle() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "HelloWorld").unwrap();
        doc.split_block(Position(5)).unwrap();
        assert_eq!(doc.block_count(), 2);
        assert_eq!(doc.block_text(0), Some("Hello".into()));
        assert_eq!(doc.block_text(1), Some("World".into()));
        assert_eq!(doc.to_text(), "Hello\nWorld");
    }

    #[test]
    fn split_block_at_start() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.split_block(Position(0)).unwrap();
        assert_eq!(doc.block_count(), 2);
        assert_eq!(doc.block_text(0), Some("".into()));
        assert_eq!(doc.block_text(1), Some("Hello".into()));
    }

    #[test]
    fn split_block_at_end() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.split_block(Position(5)).unwrap();
        assert_eq!(doc.block_count(), 2);
        assert_eq!(doc.block_text(0), Some("Hello".into()));
        assert_eq!(doc.block_text(1), Some("".into()));
    }

    #[test]
    fn set_block_type_to_heading() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Title").unwrap();
        let mut attrs = HashMap::new();
        attrs.insert("level".into(), "1".into());
        doc.set_block_type(0, "heading", Some(attrs)).unwrap();
        assert_eq!(doc.block_type(0), Some("heading".into()));
        let block_attrs = doc.block_attrs(0).unwrap();
        assert_eq!(block_attrs.get("level"), Some(&"1".to_string()));
    }

    #[test]
    fn to_html_simple() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        assert_eq!(doc.to_html(), "<p>Hello</p>");
    }

    #[test]
    fn to_html_heading() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Title").unwrap();
        let mut attrs = HashMap::new();
        attrs.insert("level".into(), "2".into());
        doc.set_block_type(0, "heading", Some(attrs)).unwrap();
        assert_eq!(doc.to_html(), "<h2>Title</h2>");
    }

    #[test]
    fn to_html_escapes_content() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "<script>alert('xss')</script>")
            .unwrap();
        let html = doc.to_html();
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn bytes_roundtrip() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello World").unwrap();
        let bytes = doc.to_bytes();
        let doc2 = EditorDocument::from_bytes(&bytes).unwrap();
        assert_eq!(doc2.to_text(), "Hello World");
        assert_eq!(doc2.block_count(), 1);
    }

    #[test]
    fn add_mark_whole_node() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.add_mark(Range::new(0usize, 5usize), MarkData::new("bold"))
            .unwrap();
        let marks = doc.marks_at(Position(0));
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].mark_type, "bold");
    }

    #[test]
    fn add_mark_partial_node() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello World").unwrap();
        doc.add_mark(Range::new(0usize, 5usize), MarkData::new("bold"))
            .unwrap();
        // First inline node should have bold
        let marks = doc.marks_at(Position(0));
        assert!(marks.iter().any(|m| m.mark_type == "bold"));
        // Text after the bold range should not have bold
        let marks_after = doc.marks_at(Position(6));
        assert!(!marks_after.iter().any(|m| m.mark_type == "bold"));
    }

    #[test]
    fn add_mark_idempotent() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.add_mark(Range::new(0usize, 5usize), MarkData::new("bold"))
            .unwrap();
        doc.add_mark(Range::new(0usize, 5usize), MarkData::new("bold"))
            .unwrap();
        let marks = doc.marks_at(Position(0));
        assert_eq!(marks.len(), 1); // Not duplicated
    }

    #[test]
    fn remove_mark() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.add_mark(Range::new(0usize, 5usize), MarkData::new("bold"))
            .unwrap();
        doc.remove_mark(Range::new(0usize, 5usize), "bold").unwrap();
        let marks = doc.marks_at(Position(0));
        assert!(marks.is_empty());
    }

    #[test]
    fn to_html_with_marks() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.add_mark(Range::new(0usize, 5usize), MarkData::new("bold"))
            .unwrap();
        let html = doc.to_html();
        assert_eq!(html, "<p><strong>Hello</strong></p>");
    }

    #[test]
    fn marks_at_returns_empty_for_no_marks() {
        let doc = EditorDocument::new();
        let marks = doc.marks_at(Position(0));
        assert!(marks.is_empty());
    }

    #[test]
    fn delete_range_cross_block() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.split_block(Position(5)).unwrap();
        // "Hello\nWorld" -> block 0: "Hello", block 1: ""
        // Insert into block 1
        doc.insert_text(Position(6), "World").unwrap();
        assert_eq!(doc.to_text(), "Hello\nWorld");

        // Delete from position 3 to 8 -> "Hel" + "rld" = "Helrld"
        doc.delete_range(Range::new(3usize, 8usize)).unwrap();
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.to_text(), "Helrld");
    }

    #[test]
    fn multiple_blocks_text_length() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.split_block(Position(5)).unwrap();
        doc.insert_text(Position(6), "World").unwrap();
        // "Hello" (5) + "\n" (1) + "World" (5) = 11
        assert_eq!(doc.text_length(), 11);
    }

    #[test]
    fn resolve_position_first_block() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        let rp = doc.resolve_position(Position(3)).unwrap();
        assert_eq!(rp.block_index, 0);
        assert_eq!(rp.text_offset, 3);
    }

    #[test]
    fn resolve_position_second_block() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.split_block(Position(5)).unwrap();
        doc.insert_text(Position(6), "World").unwrap();
        // Position 6 is start of second block
        let rp = doc.resolve_position(Position(6)).unwrap();
        assert_eq!(rp.block_index, 1);
        assert_eq!(rp.text_offset, 0);
    }

    #[test]
    fn resolve_position_invalid() {
        let doc = EditorDocument::new();
        let result = doc.resolve_position(Position(100));
        assert!(result.is_err());
    }

    #[test]
    fn mark_data_with_attrs() {
        let mut attrs = HashMap::new();
        attrs.insert("href".into(), "https://example.com".into());
        let mark = MarkData::with_attrs("link", attrs);
        assert_eq!(mark.mark_type, "link");
        assert_eq!(
            mark.attrs.get("href"),
            Some(&"https://example.com".to_string())
        );
    }

    #[test]
    fn to_html_link_mark() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "click here").unwrap();
        let mut attrs = HashMap::new();
        attrs.insert("href".into(), "https://example.com".into());
        doc.add_mark(
            Range::new(0usize, 10usize),
            MarkData::with_attrs("link", attrs),
        )
        .unwrap();
        let html = doc.to_html();
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains("click here"));
    }

    #[test]
    fn delete_preserves_marks_within_block() {
        // "Hello World" with "World" bolded, delete "lo W" -> "Hel" + "orld"(bold)
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello World").unwrap();
        doc.add_mark(Range::new(6usize, 11usize), MarkData::new("bold"))
            .unwrap();
        // Now we have: ["Hello "] ["World"(bold)]
        // Delete "lo W" = positions 3..7
        doc.delete_range(Range::new(3usize, 7usize)).unwrap();
        assert_eq!(doc.to_text(), "Helorld");
        // "orld" should still be bold
        let marks = doc.marks_at(Position(3));
        assert!(
            marks.iter().any(|m| m.mark_type == "bold"),
            "marks at pos 3 should include bold, got: {:?}",
            marks
        );
    }

    #[test]
    fn split_block_preserves_marks() {
        // "Hello World" with "World" bolded, split at "Wor|ld"
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello World").unwrap();
        doc.add_mark(Range::new(6usize, 11usize), MarkData::new("bold"))
            .unwrap();
        // Split at position 9 (inside "World" -> "Wor" | "ld")
        doc.split_block(Position(9)).unwrap();
        assert_eq!(doc.block_count(), 2);
        assert_eq!(doc.block_text(0), Some("Hello Wor".into()));
        assert_eq!(doc.block_text(1), Some("ld".into()));
        // Both "Wor" and "ld" should be bold
        let marks_before = doc.marks_at(Position(6)); // "W" in block 0
        assert!(
            marks_before.iter().any(|m| m.mark_type == "bold"),
            "Wor should be bold"
        );
        let marks_after = doc.marks_at(Position(10)); // "l" in block 1 (pos 10 = after newline)
        assert!(
            marks_after.iter().any(|m| m.mark_type == "bold"),
            "ld should be bold"
        );
    }

    #[test]
    fn cross_block_delete_preserves_marks() {
        // Two blocks: "Hello" and "World"(bold), delete across to merge
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.split_block(Position(5)).unwrap();
        doc.insert_text(Position(6), "World").unwrap();
        doc.add_mark(Range::new(6usize, 11usize), MarkData::new("bold"))
            .unwrap();
        // Delete from pos 3 to pos 8: "lo\nWo" deleted -> "Hel" + "rld"(bold)
        doc.delete_range(Range::new(3usize, 8usize)).unwrap();
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.to_text(), "Helrld");
        // "rld" should be bold (positions 3,4,5)
        let marks = doc.marks_at(Position(3));
        assert!(
            marks.iter().any(|m| m.mark_type == "bold"),
            "rld should be bold after cross-block delete"
        );
    }

    #[test]
    fn delete_within_block_multiple_marks() {
        // "AB CD EF" with "AB" bold, "EF" italic, delete "B CD E"
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "AB CD EF").unwrap();
        doc.add_mark(Range::new(0usize, 2usize), MarkData::new("bold"))
            .unwrap();
        doc.add_mark(Range::new(6usize, 8usize), MarkData::new("italic"))
            .unwrap();
        // Delete positions 1..7: "B CD E"
        doc.delete_range(Range::new(1usize, 7usize)).unwrap();
        assert_eq!(doc.to_text(), "AF");
        // "A" should be bold
        let marks_a = doc.marks_at(Position(0));
        assert!(marks_a.iter().any(|m| m.mark_type == "bold"));
        // "F" should be italic
        let marks_f = doc.marks_at(Position(1));
        assert!(marks_f.iter().any(|m| m.mark_type == "italic"));
    }

    // TODO: extract_fragment tests removed — method not yet implemented

    #[test]
    fn to_markdown_simple() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        assert_eq!(doc.to_markdown(), "Hello");
    }

    #[test]
    fn to_markdown_with_bold() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello World").unwrap();
        doc.add_mark(Range::new(0usize, 5usize), MarkData::new("bold"))
            .unwrap();
        let md = doc.to_markdown();
        assert!(md.contains("**Hello**"));
        assert!(md.contains("World"));
    }

    #[test]
    fn insert_fragment_text_then_marks() {
        // Simulate the insert_fragment approach: insert all text first, then apply marks.
        // This verifies that add_mark correctly splits inline nodes.
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "First line").unwrap();
        doc.split_block(Position(10)).unwrap();
        // Now: Block 0 = "First line", Block 1 = ""

        // Simulate pasting "Hello World" with "Hello" bold into block 1
        let insert_pos = Position(11); // start of block 1
        doc.insert_text(insert_pos, "Hello World").unwrap();

        // Apply bold to just "Hello" (first 5 chars)
        doc.add_mark(Range::new(11usize, 16usize), MarkData::new("bold"))
            .unwrap();

        // Verify text
        assert_eq!(doc.block_text(1), Some("Hello World".into()));
        assert_eq!(doc.text_length(), 10 + 1 + 11); // 22

        // Verify marks: "Hello" should be bold, " World" should not
        let marks_h = doc.marks_at(Position(11));
        assert!(
            marks_h.iter().any(|m| m.mark_type == "bold"),
            "H should be bold"
        );
        let marks_space = doc.marks_at(Position(16));
        assert!(
            !marks_space.iter().any(|m| m.mark_type == "bold"),
            "space after Hello should NOT be bold"
        );
    }

    #[test]
    fn add_mark_subrange_no_fragmentation_when_already_marked() {
        // Regression test: typing with stored marks active should NOT fragment
        // the inline into one node per character. When add_mark is called on a
        // sub-range of an inline that already has the mark, it should be a no-op.
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "B").unwrap();
        // Mark the entire inline as bold
        doc.add_mark(Range::new(0usize, 1usize), MarkData::new("bold"))
            .unwrap();
        assert_eq!(doc.block_inline_runs(0).len(), 1);

        // Simulate typing "o" after "B" — insert_text appends to the bold inline
        doc.insert_text(Position(1), "o").unwrap();
        // Now add_mark on just the new char (sub-range of the already-bold inline)
        doc.add_mark(Range::new(1usize, 2usize), MarkData::new("bold"))
            .unwrap();

        // Should still be ONE inline, not two
        let runs = doc.block_inline_runs(0);
        assert_eq!(
            runs.len(),
            1,
            "Should be 1 inline, got {}: {:?}",
            runs.len(),
            runs
        );
        assert_eq!(runs[0].text, "Bo");
        assert!(runs[0].marks.iter().any(|m| m.mark_type == "bold"));

        // Continue: type "ld" one char at a time
        doc.insert_text(Position(2), "l").unwrap();
        doc.add_mark(Range::new(2usize, 3usize), MarkData::new("bold"))
            .unwrap();
        doc.insert_text(Position(3), "d").unwrap();
        doc.add_mark(Range::new(3usize, 4usize), MarkData::new("bold"))
            .unwrap();

        let runs = doc.block_inline_runs(0);
        assert_eq!(
            runs.len(),
            1,
            "Should still be 1 inline after 4 chars, got {}: {:?}",
            runs.len(),
            runs
        );
        assert_eq!(runs[0].text, "Bold");
    }

    #[test]
    fn from_block_data_plain_after_bold_not_inherited() {
        // Regression test: from_block_data must not let plain text inherit
        // marks from a preceding marked inline. Previously, plain-text runs
        // used insert_text (which appends to whatever inline the position
        // resolves to, inheriting its marks) instead of insert_text_with_marks.
        use rinch_core::ce::{BlockData, InlineMarkData, InlineRunData};

        let blocks = vec![BlockData {
            block_type: "paragraph".to_string(),
            attrs: HashMap::new(),
            content: vec![
                InlineRunData {
                    text: "normal ".to_string(),
                    marks: vec![],
                },
                InlineRunData {
                    text: "bold".to_string(),
                    marks: vec![InlineMarkData {
                        mark_type: "bold".to_string(),
                        attrs: HashMap::new(),
                    }],
                },
                InlineRunData {
                    text: " normal".to_string(),
                    marks: vec![],
                },
            ],
        }];

        let doc = EditorDocument::from_block_data(&blocks);
        let runs = doc.block_inline_runs(0);

        // Should have 3 inline runs preserving mark boundaries
        assert_eq!(
            runs.len(),
            3,
            "Expected 3 inline runs, got {}: {:?}",
            runs.len(),
            runs
        );
        // First run: plain
        assert_eq!(runs[0].text, "normal ");
        assert!(runs[0].marks.is_empty(), "first run should have no marks");
        // Second run: bold
        assert_eq!(runs[1].text, "bold");
        assert!(
            runs[1].marks.iter().any(|m| m.mark_type == "bold"),
            "second run should be bold"
        );
        // Third run: plain (NOT bold)
        assert_eq!(runs[2].text, " normal");
        assert!(
            runs[2].marks.is_empty(),
            "third run should have no marks, got: {:?}",
            runs[2].marks
        );
    }

    #[test]
    fn from_block_data_roundtrip_preserves_marks() {
        // Verify that from_block_data -> to_block_data is lossless
        use rinch_core::ce::{BlockData, InlineMarkData, InlineRunData};

        let original = vec![BlockData {
            block_type: "paragraph".to_string(),
            attrs: HashMap::new(),
            content: vec![
                InlineRunData {
                    text: "hello ".to_string(),
                    marks: vec![],
                },
                InlineRunData {
                    text: "world".to_string(),
                    marks: vec![InlineMarkData {
                        mark_type: "bold".to_string(),
                        attrs: HashMap::new(),
                    }],
                },
                InlineRunData {
                    text: "!".to_string(),
                    marks: vec![],
                },
            ],
        }];

        let doc = EditorDocument::from_block_data(&original);

        // Verify intermediate state
        assert_eq!(doc.block_count(), 1, "should have 1 block");
        let runs = doc.block_inline_runs(0);
        assert_eq!(runs.len(), 3, "should have 3 inline runs, got: {:?}", runs);

        let roundtripped = doc.to_block_data();

        assert_eq!(
            roundtripped.len(),
            1,
            "roundtrip blocks: {:?}",
            roundtripped
        );
        assert_eq!(roundtripped[0].content.len(), 3);
        assert_eq!(roundtripped[0].content[0].text, "hello ");
        assert!(roundtripped[0].content[0].marks.is_empty());
        assert_eq!(roundtripped[0].content[1].text, "world");
        assert_eq!(roundtripped[0].content[1].marks[0].mark_type, "bold");
        assert_eq!(roundtripped[0].content[2].text, "!");
        assert!(roundtripped[0].content[2].marks.is_empty());
    }
}
