//! Core document model using Automerge CRDT for collaboration.

use std::collections::HashMap;

use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, transaction::Transactable};

use super::position::{Position, Range, ResolvedPosition};
use crate::error::EditorError;

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
    content_id: ObjId,
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
        let text_node = doc
            .insert_object(&inline_content, 0, ObjType::Map)
            .unwrap();
        doc.put(&text_node, "type", "text").unwrap();
        doc.put(&text_node, "text", "").unwrap();
        doc.put_object(&text_node, "marks", ObjType::List).unwrap();

        Self { doc, content_id }
    }

    /// Create from Automerge bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EditorError> {
        let doc = AutoCommit::load(bytes).map_err(|e| EditorError::Automerge(e.to_string()))?;
        // Find the content_id
        let content_id = doc
            .get(&automerge::ROOT, "content")
            .map_err(|e| EditorError::Automerge(e.to_string()))?
            .and_then(|(val, id)| {
                if matches!(val, automerge::Value::Object(ObjType::List)) {
                    Some(id)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                EditorError::Automerge("Missing or invalid 'content' field".to_string())
            })?;

        Ok(Self { doc, content_id })
    }

    /// Export to Automerge bytes.
    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    /// Get number of blocks in the document.
    pub fn block_count(&self) -> usize {
        self.doc.length(&self.content_id)
    }

    /// Get the ObjId of a block at the given index.
    fn block_obj(&self, index: usize) -> Option<ObjId> {
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
    fn block_content_obj(&self, block_id: &ObjId) -> Option<ObjId> {
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

    /// Get the Automerge ObjId for a block at the given index.
    /// This provides stable block identity across insertions/deletions.
    pub fn block_obj_id(&self, index: usize) -> Option<automerge::ObjId> {
        self.block_obj(index)
    }

    /// Get all block ObjIds in order.
    pub fn block_obj_ids(&self) -> Vec<automerge::ObjId> {
        let count = self.block_count();
        (0..count).filter_map(|i| self.block_obj_id(i)).collect()
    }

    /// Get block type at index.
    pub fn block_type(&self, index: usize) -> Option<String> {
        let block_id = self.block_obj(index)?;
        self.doc
            .get(&block_id, "type")
            .ok()
            .flatten()
            .and_then(|(val, _)| {
                if let automerge::Value::Scalar(s) = val {
                    if let automerge::ScalarValue::Str(smol) = s.as_ref() {
                        return Some(smol.to_string());
                    }
                }
                None
            })
    }

    /// Get block attributes at index.
    pub fn block_attrs(&self, index: usize) -> Option<HashMap<String, String>> {
        let block_id = self.block_obj(index)?;
        let attrs_id = self
            .doc
            .get(&block_id, "attrs")
            .ok()
            .flatten()
            .and_then(|(val, id)| {
                if matches!(val, automerge::Value::Object(ObjType::Map)) {
                    Some(id)
                } else {
                    None
                }
            })?;
        let mut result = HashMap::new();
        for key in self.doc.keys(&attrs_id) {
            if let Some((val, _)) = self.doc.get(&attrs_id, key.as_str()).ok().flatten() {
                if let automerge::Value::Scalar(s) = val {
                    if let automerge::ScalarValue::Str(smol) = s.as_ref() {
                        result.insert(key, smol.to_string());
                    }
                }
            }
        }
        Some(result)
    }

    /// Get structured inline runs for a block (for rendering).
    pub fn block_inline_runs(&self, block_index: usize) -> Vec<InlineRun> {
        let block_id = match self.block_obj(block_index) {
            Some(id) => id,
            None => return Vec::new(),
        };
        let content_id = match self.block_content_obj(&block_id) {
            Some(id) => id,
            None => return Vec::new(),
        };
        let mut runs = Vec::new();
        let len = self.doc.length(&content_id);
        for i in 0..len {
            if let Some((_, inline_id)) = self.doc.get(&content_id, i).ok().flatten() {
                let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();
                match inline_type.as_str() {
                    "text" => {
                        let text = self.get_str(&inline_id, "text").unwrap_or_default();
                        let marks = self.read_marks(&inline_id);
                        runs.push(InlineRun {
                            text,
                            inline_type: "text".into(),
                            marks,
                        });
                    }
                    "hard_break" => {
                        runs.push(InlineRun {
                            text: String::new(),
                            inline_type: "hard_break".into(),
                            marks: Vec::new(),
                        });
                    }
                    _ => {}
                }
            }
        }
        runs
    }

    /// Get text content of a block (concatenated text of all inline nodes).
    pub fn block_text(&self, index: usize) -> Option<String> {
        let block_id = self.block_obj(index)?;
        let content_id = self.block_content_obj(&block_id)?;
        let mut text = String::new();
        let len = self.doc.length(&content_id);
        for i in 0..len {
            if let Some((_, inline_id)) = self.doc.get(&content_id, i).ok().flatten() {
                let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();
                match inline_type.as_str() {
                    "text" => {
                        if let Some(t) = self.get_str(&inline_id, "text") {
                            text.push_str(&t);
                        }
                    }
                    "hard_break" => {
                        text.push('\n');
                    }
                    _ => {}
                }
            }
        }
        Some(text)
    }

    /// Get total text length (character count across all blocks).
    /// Each block boundary contributes 1 character (newline between blocks).
    pub fn text_length(&self) -> usize {
        let count = self.block_count();
        if count == 0 {
            return 0;
        }
        let mut total = 0;
        for i in 0..count {
            total += self.block_text(i).map(|t| t.len()).unwrap_or(0);
        }
        // Add newlines between blocks
        total += count.saturating_sub(1);
        total
    }

    /// Get all text as a single string (blocks separated by newlines).
    pub fn to_text(&self) -> String {
        let count = self.block_count();
        let mut parts = Vec::with_capacity(count);
        for i in 0..count {
            parts.push(self.block_text(i).unwrap_or_default());
        }
        parts.join("\n")
    }

    /// Get simple HTML representation.
    pub fn to_html(&self) -> String {
        let count = self.block_count();
        let mut html = String::new();
        for i in 0..count {
            let block_type = self.block_type(i).unwrap_or_else(|| "p".to_string());
            let tag = match block_type.as_str() {
                "paragraph" => "p",
                "heading" => {
                    let attrs = self.block_attrs(i).unwrap_or_default();
                    match attrs.get("level").map(|s| s.as_str()) {
                        Some("1") => "h1",
                        Some("2") => "h2",
                        Some("3") => "h3",
                        Some("4") => "h4",
                        Some("5") => "h5",
                        Some("6") => "h6",
                        _ => "h1",
                    }
                }
                "blockquote" => "blockquote",
                "code_block" => "pre",
                other => other,
            };
            html.push_str(&format!("<{}>", tag));
            html.push_str(&self.block_inline_html(i));
            html.push_str(&format!("</{}>", tag));
        }
        html
    }

    /// Get HTML for inline content of a block.
    fn block_inline_html(&self, block_index: usize) -> String {
        let block_id = match self.block_obj(block_index) {
            Some(id) => id,
            None => return String::new(),
        };
        let content_id = match self.block_content_obj(&block_id) {
            Some(id) => id,
            None => return String::new(),
        };
        let mut html = String::new();
        let len = self.doc.length(&content_id);
        for i in 0..len {
            if let Some((_, inline_id)) = self.doc.get(&content_id, i).ok().flatten() {
                let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();
                match inline_type.as_str() {
                    "text" => {
                        let text = self.get_str(&inline_id, "text").unwrap_or_default();
                        let marks = self.read_marks(&inline_id);
                        let mut result = html_escape(&text);
                        // Wrap in mark tags (innermost first)
                        for mark in &marks {
                            result = wrap_mark(&result, mark);
                        }
                        html.push_str(&result);
                    }
                    "hard_break" => {
                        html.push_str("<br>");
                    }
                    _ => {}
                }
            }
        }
        html
    }

    /// Read marks from an inline node.
    fn read_marks(&self, inline_id: &ObjId) -> Vec<MarkData> {
        let marks_id = match self
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

    /// Resolve a document position to block/inline/offset coordinates.
    pub fn resolve_position(&self, pos: Position) -> Result<ResolvedPosition, EditorError> {
        let mut remaining = pos.0;
        let count = self.block_count();
        for block_idx in 0..count {
            let block_text_len = self.block_text(block_idx).map(|t| t.len()).unwrap_or(0);
            if remaining <= block_text_len {
                // Position is within this block - find inline index
                let (inline_index, text_offset) =
                    self.resolve_inline_position(block_idx, remaining);
                return Ok(ResolvedPosition {
                    block_index: block_idx,
                    inline_index,
                    text_offset,
                });
            }
            // Account for block text + newline separator
            remaining = remaining.saturating_sub(block_text_len + 1);
        }
        Err(EditorError::InvalidPosition(pos.0, self.text_length()))
    }

    /// Find inline_index and text_offset within a block for a given character offset.
    fn resolve_inline_position(&self, block_index: usize, offset: usize) -> (usize, usize) {
        let block_id = match self.block_obj(block_index) {
            Some(id) => id,
            None => return (0, 0),
        };
        let content_id = match self.block_content_obj(&block_id) {
            Some(id) => id,
            None => return (0, 0),
        };
        let mut remaining = offset;
        let len = self.doc.length(&content_id);
        for i in 0..len {
            if let Some((_, inline_id)) = self.doc.get(&content_id, i).ok().flatten() {
                let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();
                let inline_len = match inline_type.as_str() {
                    "text" => self.get_str(&inline_id, "text").map(|t| t.len()).unwrap_or(0),
                    "hard_break" => 1,
                    _ => 0,
                };
                let is_last = i == len - 1;
                // For non-last inlines, position at end of one inline = start of next.
                // Only the last inline can hold offset == inline_len (cursor at end).
                if is_last {
                    if remaining <= inline_len {
                        return (i, remaining);
                    }
                } else if remaining < inline_len {
                    return (i, remaining);
                }
                remaining -= inline_len;
            }
        }
        // Past end: return last position
        (len.saturating_sub(1), remaining)
    }

    /// Get marks at a position.
    pub fn marks_at(&self, pos: Position) -> Vec<MarkData> {
        let resolved = match self.resolve_position(pos) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let block_id = match self.block_obj(resolved.block_index) {
            Some(id) => id,
            None => return Vec::new(),
        };
        let content_id = match self.block_content_obj(&block_id) {
            Some(id) => id,
            None => return Vec::new(),
        };
        if let Some((_, inline_id)) = self
            .doc
            .get(&content_id, resolved.inline_index)
            .ok()
            .flatten()
        {
            self.read_marks(&inline_id)
        } else {
            Vec::new()
        }
    }

    /// Insert text at a position.
    pub fn insert_text(&mut self, pos: Position, text: &str) -> Result<(), EditorError> {
        if text.is_empty() {
            return Ok(());
        }
        let resolved = self.resolve_position(pos)?;
        let block_id = self
            .block_obj(resolved.block_index)
            .ok_or_else(|| EditorError::InvalidPosition(pos.0, self.text_length()))?;
        let content_id = self
            .block_content_obj(&block_id)
            .ok_or_else(|| EditorError::InvalidPosition(pos.0, self.text_length()))?;

        let inline_id = self
            .doc
            .get(&content_id, resolved.inline_index)
            .ok()
            .flatten()
            .map(|(_, id)| id)
            .ok_or_else(|| EditorError::InvalidPosition(pos.0, self.text_length()))?;

        let existing = self.get_str(&inline_id, "text").unwrap_or_default();
        let offset = resolved.text_offset.min(existing.len());
        let mut new_text = String::with_capacity(existing.len() + text.len());
        new_text.push_str(&existing[..offset]);
        new_text.push_str(text);
        new_text.push_str(&existing[offset..]);
        self.doc
            .put(&inline_id, "text", new_text.as_str())
            .map_err(|e| EditorError::Automerge(e.to_string()))?;

        Ok(())
    }

    /// Read inline nodes from a content list starting at `from_index` as (text, marks) pairs.
    fn collect_inlines(&self, content_id: &ObjId, from_index: usize) -> Vec<(String, Vec<MarkData>)> {
        let len = self.doc.length(content_id);
        let mut result = Vec::new();
        for i in from_index..len {
            if let Some((_, inline_id)) = self.doc.get(content_id, i).ok().flatten() {
                let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();
                if inline_type == "text" {
                    let text = self.get_str(&inline_id, "text").unwrap_or_default();
                    let marks = self.read_marks(&inline_id);
                    result.push((text, marks));
                }
            }
        }
        result
    }

    /// Delete text in a range.
    pub fn delete_range(&mut self, range: Range) -> Result<(), EditorError> {
        if range.is_empty() {
            return Ok(());
        }

        let start_resolved = self.resolve_position(range.start)?;
        let end_resolved = self.resolve_position(range.end)?;

        // Simple case: same block, same inline node
        if start_resolved.block_index == end_resolved.block_index
            && start_resolved.inline_index == end_resolved.inline_index
        {
            let block_id = self.block_obj(start_resolved.block_index).unwrap();
            let content_id = self.block_content_obj(&block_id).unwrap();
            let inline_id = self
                .doc
                .get(&content_id, start_resolved.inline_index)
                .ok()
                .flatten()
                .map(|(_, id)| id)
                .unwrap();

            let existing = self.get_str(&inline_id, "text").unwrap_or_default();
            let start_off = start_resolved.text_offset.min(existing.len());
            let end_off = end_resolved.text_offset.min(existing.len());
            let mut new_text = String::new();
            new_text.push_str(&existing[..start_off]);
            new_text.push_str(&existing[end_off..]);

            if new_text.is_empty() && self.doc.length(&content_id) > 1 {
                // Remove empty inline node (but keep at least one)
                self.doc
                    .delete(&content_id, start_resolved.inline_index)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            } else {
                self.doc
                    .put(&inline_id, "text", new_text.as_str())
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
            return Ok(());
        }

        if start_resolved.block_index == end_resolved.block_index {
            // Same block, different inline nodes
            self.delete_within_block_inlines(
                start_resolved.block_index,
                start_resolved.inline_index,
                start_resolved.text_offset,
                end_resolved.inline_index,
                end_resolved.text_offset,
            )?;
        } else {
            // Cross-block delete
            let start_block_id = self.block_obj(start_resolved.block_index).unwrap();
            let start_content_id = self.block_content_obj(&start_block_id).unwrap();

            let end_block_id = self.block_obj(end_resolved.block_index).unwrap();
            let end_content_id = self.block_content_obj(&end_block_id).unwrap();

            // Truncate start inline node text to [..start_text_offset]
            if let Some((_, start_inline_id)) = self.doc.get(&start_content_id, start_resolved.inline_index).ok().flatten() {
                let text = self.get_str(&start_inline_id, "text").unwrap_or_default();
                let keep = &text[..start_resolved.text_offset.min(text.len())];
                self.doc.put(&start_inline_id, "text", keep).map_err(|e| EditorError::Automerge(e.to_string()))?;
            }

            // Remove all inlines in start block after start_inline_index
            let start_inline_count = self.doc.length(&start_content_id);
            for _ in (start_resolved.inline_index + 1..start_inline_count).rev() {
                self.doc.delete(&start_content_id, start_resolved.inline_index + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }

            // Collect inlines to keep from end block: truncate end inline, collect from there
            // First, read the end inline's remaining text and marks
            let mut end_inlines_to_append: Vec<(String, Vec<MarkData>)> = Vec::new();

            if let Some((_, end_inline_id)) = self.doc.get(&end_content_id, end_resolved.inline_index).ok().flatten() {
                let text = self.get_str(&end_inline_id, "text").unwrap_or_default();
                let keep = text[end_resolved.text_offset.min(text.len())..].to_string();
                let marks = self.read_marks(&end_inline_id);
                if !keep.is_empty() {
                    end_inlines_to_append.push((keep, marks));
                }
            }

            // Collect remaining inlines after end_inline_index in end block
            let remaining = self.collect_inlines(&end_content_id, end_resolved.inline_index + 1);
            end_inlines_to_append.extend(remaining);

            // Append end block's remaining inlines to start block
            // Check if start inline became empty
            let start_inline_text = if let Some((_, sid)) = self.doc.get(&start_content_id, start_resolved.inline_index).ok().flatten() {
                self.get_str(&sid, "text").unwrap_or_default()
            } else {
                String::new()
            };

            let mut insert_idx = if start_inline_text.is_empty() && !end_inlines_to_append.is_empty() {
                // Remove empty start inline, we'll replace with end inlines
                if self.doc.length(&start_content_id) > 0 {
                    self.doc.delete(&start_content_id, start_resolved.inline_index)
                        .map_err(|e| EditorError::Automerge(e.to_string()))?;
                }
                start_resolved.inline_index
            } else {
                start_resolved.inline_index + 1
            };

            for (text, marks) in &end_inlines_to_append {
                self.insert_text_inline(&start_content_id, insert_idx, text, marks)?;
                insert_idx += 1;
            }

            // Ensure block has at least one inline
            if self.doc.length(&start_content_id) == 0 {
                self.insert_text_inline(&start_content_id, 0, "", &[])?;
            }

            // Remove intermediate + end blocks
            for _ in (start_resolved.block_index + 1..=end_resolved.block_index).rev() {
                self.doc
                    .delete(&self.content_id, start_resolved.block_index + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }

        Ok(())
    }

    /// Delete within a single block across multiple inline nodes, preserving marks.
    fn delete_within_block_inlines(
        &mut self,
        block_index: usize,
        start_inline: usize,
        start_offset: usize,
        end_inline: usize,
        end_offset: usize,
    ) -> Result<(), EditorError> {
        let block_id = self.block_obj(block_index)
            .ok_or_else(|| EditorError::CommandFailed("Block not found".into()))?;
        let content_id = self.block_content_obj(&block_id)
            .ok_or_else(|| EditorError::CommandFailed("Block content not found".into()))?;

        // Truncate start inline node
        if let Some((_, start_id)) = self.doc.get(&content_id, start_inline).ok().flatten() {
            let text = self.get_str(&start_id, "text").unwrap_or_default();
            let keep = &text[..start_offset.min(text.len())];
            self.doc.put(&start_id, "text", keep).map_err(|e| EditorError::Automerge(e.to_string()))?;
        }

        // Truncate end inline node
        if let Some((_, end_id)) = self.doc.get(&content_id, end_inline).ok().flatten() {
            let text = self.get_str(&end_id, "text").unwrap_or_default();
            let keep = text[end_offset.min(text.len())..].to_string();
            self.doc.put(&end_id, "text", &keep).map_err(|e| EditorError::Automerge(e.to_string()))?;
        }

        // Remove intermediate inline nodes (between start+1 and end-1)
        // Remove in reverse to keep indices stable
        if end_inline > start_inline + 1 {
            for _ in (start_inline + 1..end_inline).rev() {
                self.doc.delete(&content_id, start_inline + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }

        // Now clean up: remove empty start/end nodes
        // After removing intermediates, end node is now at start_inline + 1
        let end_new_idx = start_inline + 1;

        // Check end node (check it first since removing start would shift it)
        let end_empty = if let Some((_, eid)) = self.doc.get(&content_id, end_new_idx).ok().flatten() {
            self.get_str(&eid, "text").unwrap_or_default().is_empty()
        } else {
            false
        };

        let start_empty = if let Some((_, sid)) = self.doc.get(&content_id, start_inline).ok().flatten() {
            self.get_str(&sid, "text").unwrap_or_default().is_empty()
        } else {
            false
        };

        let total = self.doc.length(&content_id);
        // Don't remove if it would leave the block with no inlines
        if end_empty && total > 1 {
            self.doc.delete(&content_id, end_new_idx)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
        }
        let total = self.doc.length(&content_id);
        if start_empty && total > 1 {
            self.doc.delete(&content_id, start_inline)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
        }

        Ok(())
    }

    /// Set the text of a block (clears all inlines, creates single plain text node).
    /// Only used in tests; production code uses mark-preserving operations.
    #[cfg(test)]
    fn set_block_text(&mut self, block_index: usize, text: &str) -> Result<(), EditorError> {
        let block_id = self
            .block_obj(block_index)
            .ok_or_else(|| EditorError::CommandFailed("Block not found".into()))?;
        let content_id = self
            .block_content_obj(&block_id)
            .ok_or_else(|| EditorError::CommandFailed("Block content not found".into()))?;

        // Clear all inline nodes and create a single text node
        let len = self.doc.length(&content_id);
        for _ in (0..len).rev() {
            self.doc
                .delete(&content_id, 0)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
        }
        let text_node = self
            .doc
            .insert_object(&content_id, 0, ObjType::Map)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put(&text_node, "type", "text")
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put(&text_node, "text", text)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put_object(&text_node, "marks", ObjType::List)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;

        Ok(())
    }

    /// Add a mark to a range of text.
    pub fn add_mark(&mut self, range: Range, mark: MarkData) -> Result<(), EditorError> {
        if range.is_empty() {
            return Ok(());
        }

        let start = self.resolve_position(range.start)?;
        let end = self.resolve_position(range.end)?;

        // For simplicity, handle the case where the range is within one block + one inline node
        // For cross-node ranges, we'd need to split inline nodes - simplified for now
        if start.block_index == end.block_index && start.inline_index == end.inline_index {
            // If the mark covers the entire inline node's text, just add the mark
            let block_id = self.block_obj(start.block_index).unwrap();
            let content_id = self.block_content_obj(&block_id).unwrap();
            let (_, inline_id) = self
                .doc
                .get(&content_id, start.inline_index)
                .ok()
                .flatten()
                .unwrap();

            let text = self.get_str(&inline_id, "text").unwrap_or_default();

            if start.text_offset == 0 && end.text_offset >= text.len() {
                // Covers entire node - add mark directly
                self.append_mark_to_inline(&inline_id, &mark)?;
            } else {
                // Need to split the inline node into up to 3 parts
                self.split_and_mark_inline(
                    &content_id,
                    start.inline_index,
                    start.text_offset,
                    end.text_offset,
                    &mark,
                )?;
            }
        } else {
            // Multi-inline or multi-block: for now, iterate blocks
            for block_idx in start.block_index..=end.block_index {
                let block_id = match self.block_obj(block_idx) {
                    Some(id) => id,
                    None => continue,
                };
                let content_id = match self.block_content_obj(&block_id) {
                    Some(id) => id,
                    None => continue,
                };
                let inline_count = self.doc.length(&content_id);
                for inline_idx in 0..inline_count {
                    if let Some((_, inline_id)) =
                        self.doc.get(&content_id, inline_idx).ok().flatten()
                    {
                        let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();
                        if inline_type == "text" {
                            // Determine if this inline is in range
                            let is_in_range = if block_idx == start.block_index
                                && block_idx == end.block_index
                            {
                                inline_idx >= start.inline_index && inline_idx <= end.inline_index
                            } else if block_idx == start.block_index {
                                inline_idx >= start.inline_index
                            } else if block_idx == end.block_index {
                                inline_idx <= end.inline_index
                            } else {
                                true
                            };
                            if is_in_range {
                                self.append_mark_to_inline(&inline_id, &mark)?;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Append a mark to an inline node's marks list.
    fn append_mark_to_inline(
        &mut self,
        inline_id: &ObjId,
        mark: &MarkData,
    ) -> Result<(), EditorError> {
        let marks_id = self
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
            });
        let marks_id = match marks_id {
            Some(id) => id,
            None => self
                .doc
                .put_object(inline_id, "marks", ObjType::List)
                .map_err(|e| EditorError::Automerge(e.to_string()))?,
        };

        // Check if mark already exists
        let len = self.doc.length(&marks_id);
        for i in 0..len {
            if let Some((_, mid)) = self.doc.get(&marks_id, i).ok().flatten() {
                if self.get_str(&mid, "type").as_deref() == Some(&mark.mark_type) {
                    return Ok(()); // Already has this mark
                }
            }
        }

        let mark_obj = self
            .doc
            .insert_object(&marks_id, len, ObjType::Map)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put(&mark_obj, "type", mark.mark_type.as_str())
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        if !mark.attrs.is_empty() {
            let attrs_obj = self
                .doc
                .put_object(&mark_obj, "attrs", ObjType::Map)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
            for (k, v) in &mark.attrs {
                self.doc
                    .put(&attrs_obj, k.as_str(), v.as_str())
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Split an inline text node and apply a mark to the middle portion.
    fn split_and_mark_inline(
        &mut self,
        content_id: &ObjId,
        inline_index: usize,
        start_offset: usize,
        end_offset: usize,
        mark: &MarkData,
    ) -> Result<(), EditorError> {
        let (_, inline_id) = self
            .doc
            .get(content_id, inline_index)
            .ok()
            .flatten()
            .ok_or_else(|| EditorError::CommandFailed("Inline node not found".into()))?;

        let text = self.get_str(&inline_id, "text").unwrap_or_default();
        let existing_marks = self.read_marks(&inline_id);

        let before = &text[..start_offset];
        let marked = &text[start_offset..end_offset];
        let after = &text[end_offset..];

        // Remove original inline node
        self.doc
            .delete(content_id, inline_index)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;

        let mut insert_idx = inline_index;

        // Insert "before" part (if non-empty)
        if !before.is_empty() {
            self.insert_text_inline(content_id, insert_idx, before, &existing_marks)?;
            insert_idx += 1;
        }

        // Insert "marked" part with the new mark added
        if !marked.is_empty() {
            let mut new_marks = existing_marks.clone();
            if !new_marks.iter().any(|m| m.mark_type == mark.mark_type) {
                new_marks.push(mark.clone());
            }
            self.insert_text_inline(content_id, insert_idx, marked, &new_marks)?;
            insert_idx += 1;
        }

        // Insert "after" part (if non-empty)
        if !after.is_empty() {
            self.insert_text_inline(content_id, insert_idx, after, &existing_marks)?;
        }

        Ok(())
    }

    /// Insert a new text inline node at a position in a content list.
    fn insert_text_inline(
        &mut self,
        content_id: &ObjId,
        index: usize,
        text: &str,
        marks: &[MarkData],
    ) -> Result<ObjId, EditorError> {
        let node = self
            .doc
            .insert_object(content_id, index, ObjType::Map)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put(&node, "type", "text")
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put(&node, "text", text)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        let marks_list = self
            .doc
            .put_object(&node, "marks", ObjType::List)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        for (i, m) in marks.iter().enumerate() {
            let mark_obj = self
                .doc
                .insert_object(&marks_list, i, ObjType::Map)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
            self.doc
                .put(&mark_obj, "type", m.mark_type.as_str())
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
            if !m.attrs.is_empty() {
                let attrs_obj = self
                    .doc
                    .put_object(&mark_obj, "attrs", ObjType::Map)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
                for (k, v) in &m.attrs {
                    self.doc
                        .put(&attrs_obj, k.as_str(), v.as_str())
                        .map_err(|e| EditorError::Automerge(e.to_string()))?;
                }
            }
        }
        Ok(node)
    }

    /// Remove a mark from a range of text.
    pub fn remove_mark(&mut self, range: Range, mark_type: &str) -> Result<(), EditorError> {
        if range.is_empty() {
            return Ok(());
        }

        let start = self.resolve_position(range.start)?;
        let end = self.resolve_position(range.end)?;

        for block_idx in start.block_index..=end.block_index {
            let block_id = match self.block_obj(block_idx) {
                Some(id) => id,
                None => continue,
            };
            let content_id = match self.block_content_obj(&block_id) {
                Some(id) => id,
                None => continue,
            };
            let inline_count = self.doc.length(&content_id);
            for inline_idx in 0..inline_count {
                if let Some((_, inline_id)) = self.doc.get(&content_id, inline_idx).ok().flatten() {
                    let marks_id = match self
                        .doc
                        .get(&inline_id, "marks")
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
                        None => continue,
                    };
                    // Find and remove the mark
                    let len = self.doc.length(&marks_id);
                    for i in (0..len).rev() {
                        if let Some((_, mid)) = self.doc.get(&marks_id, i).ok().flatten() {
                            if self.get_str(&mid, "type").as_deref() == Some(mark_type) {
                                self.doc
                                    .delete(&marks_id, i)
                                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Split a block at position (Enter key behavior). Preserves marks on both halves.
    pub fn split_block(&mut self, pos: Position) -> Result<(), EditorError> {
        let resolved = self.resolve_position(pos)?;
        let block_idx = resolved.block_index;
        let block_type = self.block_type(block_idx).unwrap_or_else(|| "paragraph".into());

        let block_id = self.block_obj(block_idx)
            .ok_or_else(|| EditorError::CommandFailed("Block not found".into()))?;
        let content_id = self.block_content_obj(&block_id)
            .ok_or_else(|| EditorError::CommandFailed("Block content not found".into()))?;

        let split_inline = resolved.inline_index;
        let split_offset = resolved.text_offset;

        // Determine which inlines go to the new block.
        // If split_offset is in the middle of an inline, we need to split it.
        let mut new_block_inlines: Vec<(String, Vec<MarkData>)> = Vec::new();

        // Read the split inline node's text and marks
        let (split_text, split_marks) = if let Some((_, inline_id)) = self.doc.get(&content_id, split_inline).ok().flatten() {
            let text = self.get_str(&inline_id, "text").unwrap_or_default();
            let marks = self.read_marks(&inline_id);
            (text, marks)
        } else {
            (String::new(), Vec::new())
        };

        let split_at_start_of_inline = split_offset == 0;
        let split_at_end_of_inline = split_offset >= split_text.len();

        // Collect inlines that move to new block
        if split_at_start_of_inline {
            // All inlines from split_inline onward go to new block
            let moving = self.collect_inlines(&content_id, split_inline);
            new_block_inlines.extend(moving);
        } else if split_at_end_of_inline {
            // All inlines after split_inline go to new block
            let moving = self.collect_inlines(&content_id, split_inline + 1);
            new_block_inlines.extend(moving);
        } else {
            // Split the inline node: remainder goes to new block
            let after_text = split_text[split_offset..].to_string();
            if !after_text.is_empty() {
                new_block_inlines.push((after_text, split_marks.clone()));
            }
            // All inlines after split_inline also go
            let moving = self.collect_inlines(&content_id, split_inline + 1);
            new_block_inlines.extend(moving);
        }

        // Now modify the original block: remove inlines that moved
        // First, truncate the split inline if needed
        if !split_at_start_of_inline && !split_at_end_of_inline {
            // Truncate to before text
            if let Some((_, inline_id)) = self.doc.get(&content_id, split_inline).ok().flatten() {
                let before_text = &split_text[..split_offset];
                self.doc.put(&inline_id, "text", before_text)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
            // Remove all inlines after split_inline
            let total = self.doc.length(&content_id);
            for _ in (split_inline + 1..total).rev() {
                self.doc.delete(&content_id, split_inline + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        } else if split_at_start_of_inline {
            // Remove all inlines from split_inline onward
            let total = self.doc.length(&content_id);
            for _ in (split_inline..total).rev() {
                self.doc.delete(&content_id, split_inline)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
            // Ensure original block has at least one empty inline
            if self.doc.length(&content_id) == 0 {
                self.insert_text_inline(&content_id, 0, "", &[])?;
            }
        } else {
            // split_at_end_of_inline: remove all inlines after split_inline
            let total = self.doc.length(&content_id);
            for _ in (split_inline + 1..total).rev() {
                self.doc.delete(&content_id, split_inline + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }

        // Create new block
        let new_block = self
            .doc
            .insert_object(&self.content_id, block_idx + 1, ObjType::Map)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put(&new_block, "type", block_type.as_str())
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put_object(&new_block, "attrs", ObjType::Map)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        let new_content = self
            .doc
            .put_object(&new_block, "content", ObjType::List)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;

        if new_block_inlines.is_empty() {
            // Empty new block
            self.insert_text_inline(&new_content, 0, "", &[])?;
        } else {
            for (i, (text, marks)) in new_block_inlines.iter().enumerate() {
                self.insert_text_inline(&new_content, i, text, marks)?;
            }
        }

        Ok(())
    }

    /// Set block type (e.g., paragraph -> heading).
    pub fn set_block_type(
        &mut self,
        block_index: usize,
        node_type: &str,
        attrs: Option<HashMap<String, String>>,
    ) -> Result<(), EditorError> {
        let block_id = self
            .block_obj(block_index)
            .ok_or_else(|| EditorError::CommandFailed("Block not found".into()))?;
        self.doc
            .put(&block_id, "type", node_type)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;

        if let Some(attrs) = attrs {
            let attrs_id = self
                .doc
                .put_object(&block_id, "attrs", ObjType::Map)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
            for (k, v) in &attrs {
                self.doc
                    .put(&attrs_id, k.as_str(), v.as_str())
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Helper: get a string value from an automerge object.
    fn get_str(&self, obj: &ObjId, key: &str) -> Option<String> {
        self.doc.get(obj, key).ok().flatten().and_then(|(val, _)| {
            if let automerge::Value::Scalar(s) = val {
                if let automerge::ScalarValue::Str(smol) = s.as_ref() {
                    return Some(smol.to_string());
                }
            }
            None
        })
    }
}

/// Simple HTML escaping.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Wrap text in an HTML tag for a mark.
fn wrap_mark(content: &str, mark: &MarkData) -> String {
    match mark.mark_type.as_str() {
        "bold" => format!("<strong>{}</strong>", content),
        "italic" => format!("<em>{}</em>", content),
        "underline" => format!("<u>{}</u>", content),
        "strikethrough" | "strike" => format!("<s>{}</s>", content),
        "code" => format!("<code>{}</code>", content),
        "link" => {
            let href = mark.attrs.get("href").map(|s| s.as_str()).unwrap_or("#");
            format!("<a href=\"{}\">{}</a>", html_escape(href), content)
        }
        _ => content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        doc.add_mark(Range::new(0usize, 10usize), MarkData::with_attrs("link", attrs))
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
        doc.add_mark(Range::new(6usize, 11usize), MarkData::new("bold")).unwrap();
        // Now we have: ["Hello "] ["World"(bold)]
        // Delete "lo W" = positions 3..7
        doc.delete_range(Range::new(3usize, 7usize)).unwrap();
        assert_eq!(doc.to_text(), "Helorld");
        // "orld" should still be bold
        let marks = doc.marks_at(Position(3));
        assert!(marks.iter().any(|m| m.mark_type == "bold"), "marks at pos 3 should include bold, got: {:?}", marks);
    }

    #[test]
    fn split_block_preserves_marks() {
        // "Hello World" with "World" bolded, split at "Wor|ld"
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello World").unwrap();
        doc.add_mark(Range::new(6usize, 11usize), MarkData::new("bold")).unwrap();
        // Split at position 9 (inside "World" -> "Wor" | "ld")
        doc.split_block(Position(9)).unwrap();
        assert_eq!(doc.block_count(), 2);
        assert_eq!(doc.block_text(0), Some("Hello Wor".into()));
        assert_eq!(doc.block_text(1), Some("ld".into()));
        // Both "Wor" and "ld" should be bold
        let marks_before = doc.marks_at(Position(6)); // "W" in block 0
        assert!(marks_before.iter().any(|m| m.mark_type == "bold"), "Wor should be bold");
        let marks_after = doc.marks_at(Position(10)); // "l" in block 1 (pos 10 = after newline)
        assert!(marks_after.iter().any(|m| m.mark_type == "bold"), "ld should be bold");
    }

    #[test]
    fn cross_block_delete_preserves_marks() {
        // Two blocks: "Hello" and "World"(bold), delete across to merge
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.split_block(Position(5)).unwrap();
        doc.insert_text(Position(6), "World").unwrap();
        doc.add_mark(Range::new(6usize, 11usize), MarkData::new("bold")).unwrap();
        // Delete from pos 3 to pos 8: "lo\nWo" deleted -> "Hel" + "rld"(bold)
        doc.delete_range(Range::new(3usize, 8usize)).unwrap();
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.to_text(), "Helrld");
        // "rld" should be bold (positions 3,4,5)
        let marks = doc.marks_at(Position(3));
        assert!(marks.iter().any(|m| m.mark_type == "bold"), "rld should be bold after cross-block delete");
    }

    #[test]
    fn delete_within_block_multiple_marks() {
        // "AB CD EF" with "AB" bold, "EF" italic, delete "B CD E"
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "AB CD EF").unwrap();
        doc.add_mark(Range::new(0usize, 2usize), MarkData::new("bold")).unwrap();
        doc.add_mark(Range::new(6usize, 8usize), MarkData::new("italic")).unwrap();
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
}
