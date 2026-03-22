//! Read-only query methods for EditorDocument.

use std::collections::HashMap;

use automerge::{ObjType, ReadDoc};

use super::{EditorDocument, InlineRun, MarkData};
use crate::document::position::{Position, ResolvedPosition};
use crate::error::EditorError;

impl EditorDocument {
    /// Get number of blocks in the document.
    pub fn block_count(&self) -> usize {
        self.doc.length(&self.content_id)
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
                if let automerge::Value::Scalar(s) = val
                    && let automerge::ScalarValue::Str(smol) = s.as_ref()
                {
                    return Some(smol.to_string());
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
            if let Some((val, _)) = self.doc.get(&attrs_id, key.as_str()).ok().flatten()
                && let automerge::Value::Scalar(s) = val
                && let automerge::ScalarValue::Str(smol) = s.as_ref()
            {
                result.insert(key, smol.to_string());
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
                        let text = self.inline_text(&inline_id);
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
                        let t = self.inline_text(&inline_id);
                        if !t.is_empty() {
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

    /// Calculate absolute position for the start of a block.
    /// This is the canonical formula: sum of (block_text_len + 1) for all preceding blocks.
    ///
    /// Examples:
    /// - block_start_position(0) = 0 (first block starts at 0)
    /// - block_start_position(1) = len(block_0) + 1 (after block 0 text + newline)
    /// - block_start_position(2) = len(block_0) + 1 + len(block_1) + 1
    pub fn block_start_position(&self, block_index: usize) -> usize {
        let mut abs = 0;
        for i in 0..block_index {
            abs += self.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
        }
        abs
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
                    "text" => self.inline_text(&inline_id).len(),
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
}
