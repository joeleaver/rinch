//! Write/mutation methods for EditorDocument.

use std::collections::HashMap;

use automerge::{ObjId, ObjType, ReadDoc, transaction::Transactable};

use super::{EditorDocument, MarkData};
use crate::document::position::{Position, Range};
use crate::error::EditorError;

impl EditorDocument {
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

        let text_id = self
            .inline_text_obj(&inline_id)
            .ok_or_else(|| EditorError::Automerge("inline text is not a Text object".into()))?;
        let existing = self.doc.text(&text_id).unwrap_or_default();
        let byte_offset = resolved.text_offset.min(existing.len());
        // Automerge splice_text uses character (Unicode scalar) positions, not bytes.
        let char_offset = existing[..byte_offset].chars().count();
        self.doc
            .splice_text(&text_id, char_offset, 0, text)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;

        Ok(())
    }

    /// Insert text at a position with explicit marks, creating a new inline if needed.
    ///
    /// Unlike `insert_text` which appends to the existing inline (inheriting its marks),
    /// this method ensures the inserted text has exactly the specified marks.
    /// If the marks match the current inline, inserts in-place. Otherwise, creates
    /// a new inline node with the correct marks.
    pub fn insert_text_with_marks(
        &mut self,
        pos: Position,
        text: &str,
        marks: &[MarkData],
    ) -> Result<(), EditorError> {
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

        let existing_marks = self.read_marks(&inline_id);
        let existing_text = self.inline_text(&inline_id);
        let offset = resolved.text_offset.min(existing_text.len());

        // Check if marks match — if so, insert in-place (fast path)
        let marks_match = marks.len() == existing_marks.len()
            && marks
                .iter()
                .all(|m| existing_marks.iter().any(|em| em.mark_type == m.mark_type));

        if marks_match {
            // Same marks: splice into existing inline's Text object
            let text_id = self
                .inline_text_obj(&inline_id)
                .ok_or_else(|| EditorError::Automerge("inline text is not a Text object".into()))?;
            let char_offset = Self::byte_offset_to_char_offset(&existing_text, offset);
            self.doc
                .splice_text(&text_id, char_offset, 0, text)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
        } else if offset == existing_text.len() {
            // At end of inline: insert new inline after
            self.insert_text_inline(&content_id, resolved.inline_index + 1, text, marks)?;
        } else if offset == 0 {
            // At start of inline: insert new inline before
            self.insert_text_inline(&content_id, resolved.inline_index, text, marks)?;
        } else {
            // In middle: split current inline, insert new inline between
            let after = &existing_text[offset..];

            // Truncate current inline's Text object to "before" text
            let text_id = self
                .inline_text_obj(&inline_id)
                .ok_or_else(|| EditorError::Automerge("inline text is not a Text object".into()))?;
            let char_offset = Self::byte_offset_to_char_offset(&existing_text, offset);
            let char_del = Self::byte_offset_to_char_offset(
                &existing_text[offset..],
                existing_text.len() - offset,
            );
            self.doc
                .splice_text(&text_id, char_offset, char_del as isize, "")
                .map_err(|e| EditorError::Automerge(e.to_string()))?;

            // Insert "after" part as new inline (preserves original marks)
            self.insert_text_inline(
                &content_id,
                resolved.inline_index + 1,
                after,
                &existing_marks,
            )?;

            // Insert our new text between them
            self.insert_text_inline(&content_id, resolved.inline_index + 1, text, marks)?;
        }

        Ok(())
    }

    /// Read inline nodes from a content list starting at `from_index` as (text, marks) pairs.
    pub(crate) fn collect_inlines(
        &self,
        content_id: &ObjId,
        from_index: usize,
    ) -> Vec<(String, Vec<MarkData>)> {
        let len = self.doc.length(content_id);
        let mut result = Vec::new();
        for i in from_index..len {
            if let Some((_, inline_id)) = self.doc.get(content_id, i).ok().flatten() {
                let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();
                if inline_type == "text" {
                    let text = self.inline_text(&inline_id);
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

            let existing = self.inline_text(&inline_id);
            let start_off = start_resolved.text_offset.min(existing.len());
            let end_off = end_resolved.text_offset.min(existing.len());
            let del_len = end_off - start_off;

            if start_off == 0 && end_off >= existing.len() && self.doc.length(&content_id) > 1 {
                // Remove empty inline node (but keep at least one)
                self.doc
                    .delete(&content_id, start_resolved.inline_index)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            } else if del_len > 0 {
                let text_id = self.inline_text_obj(&inline_id).ok_or_else(|| {
                    EditorError::Automerge("inline text is not a Text object".into())
                })?;
                let char_start = Self::byte_offset_to_char_offset(&existing, start_off);
                let char_del = Self::byte_offset_to_char_offset(&existing[start_off..], del_len);
                self.doc
                    .splice_text(&text_id, char_start, char_del as isize, "")
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
            if let Some((_, start_inline_id)) = self
                .doc
                .get(&start_content_id, start_resolved.inline_index)
                .ok()
                .flatten()
            {
                let text = self.inline_text(&start_inline_id);
                let keep_len = start_resolved.text_offset.min(text.len());
                let del = text.len() - keep_len;
                if del > 0
                    && let Some(text_id) = self.inline_text_obj(&start_inline_id)
                {
                    let char_keep = Self::byte_offset_to_char_offset(&text, keep_len);
                    let char_del = Self::byte_offset_to_char_offset(&text[keep_len..], del);
                    self.doc
                        .splice_text(&text_id, char_keep, char_del as isize, "")
                        .map_err(|e| EditorError::Automerge(e.to_string()))?;
                }
            }

            // Remove all inlines in start block after start_inline_index
            let start_inline_count = self.doc.length(&start_content_id);
            for _ in (start_resolved.inline_index + 1..start_inline_count).rev() {
                self.doc
                    .delete(&start_content_id, start_resolved.inline_index + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }

            // Collect inlines to keep from end block: truncate end inline, collect from there
            // First, read the end inline's remaining text and marks
            let mut end_inlines_to_append: Vec<(String, Vec<MarkData>)> = Vec::new();

            if let Some((_, end_inline_id)) = self
                .doc
                .get(&end_content_id, end_resolved.inline_index)
                .ok()
                .flatten()
            {
                let text = self.inline_text(&end_inline_id);
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
            let start_inline_text = if let Some((_, sid)) = self
                .doc
                .get(&start_content_id, start_resolved.inline_index)
                .ok()
                .flatten()
            {
                self.inline_text(&sid)
            } else {
                String::new()
            };

            let mut insert_idx =
                if start_inline_text.is_empty() && !end_inlines_to_append.is_empty() {
                    // Remove empty start inline, we'll replace with end inlines
                    if self.doc.length(&start_content_id) > 0 {
                        self.doc
                            .delete(&start_content_id, start_resolved.inline_index)
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

            // Merge adjacent inlines with matching marks in the surviving block.
            // Cross-block delete appends the end block's inlines, which may be
            // mergeable with the start block's last inline.
            self.merge_adjacent_inlines(start_resolved.block_index)?;
        }

        Ok(())
    }

    /// Merge adjacent text inline nodes that have the same marks.
    ///
    /// After cross-block delete, the surviving block may have two adjacent
    /// text inlines with identical marks (e.g., "Hello" + "World" both unmarked).
    /// The DOM merges these into one text node, so the CRDT must do the same.
    fn merge_adjacent_inlines(&mut self, block_index: usize) -> Result<(), EditorError> {
        let block_id = self
            .block_obj(block_index)
            .ok_or_else(|| EditorError::CommandFailed("Block not found for merge".into()))?;
        let content_id = self.block_content_obj(&block_id).ok_or_else(|| {
            EditorError::CommandFailed("Block content not found for merge".into())
        })?;

        let mut i = 0;
        while i + 1 < self.doc.length(&content_id) {
            let (_, id_a) = self
                .doc
                .get(&content_id, i)
                .ok()
                .flatten()
                .ok_or_else(|| EditorError::CommandFailed("Inline not found".into()))?;
            let (_, id_b) = self
                .doc
                .get(&content_id, i + 1)
                .ok()
                .flatten()
                .ok_or_else(|| EditorError::CommandFailed("Inline not found".into()))?;

            let type_a = self.get_str(&id_a, "type").unwrap_or_default();
            let type_b = self.get_str(&id_b, "type").unwrap_or_default();

            if type_a != "text" || type_b != "text" {
                i += 1;
                continue;
            }

            let marks_a = self.read_marks(&id_a);
            let marks_b = self.read_marks(&id_b);

            if marks_a == marks_b {
                // Merge: append b's text to a, then remove b
                let text_b = self.inline_text(&id_b);
                if !text_b.is_empty()
                    && let Some(text_id_a) = self.inline_text_obj(&id_a)
                {
                    let char_len_a = self.doc.length(&text_id_a);
                    self.doc
                        .splice_text(&text_id_a, char_len_a, 0, &text_b)
                        .map_err(|e| EditorError::Automerge(e.to_string()))?;
                }
                self.doc
                    .delete(&content_id, i + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
                // Don't increment i — check if merged node can merge with next
            } else {
                i += 1;
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
        let block_id = self
            .block_obj(block_index)
            .ok_or_else(|| EditorError::CommandFailed("Block not found".into()))?;
        let content_id = self
            .block_content_obj(&block_id)
            .ok_or_else(|| EditorError::CommandFailed("Block content not found".into()))?;

        // Truncate start inline node
        if let Some((_, start_id)) = self.doc.get(&content_id, start_inline).ok().flatten() {
            let text = self.inline_text(&start_id);
            let keep_len = start_offset.min(text.len());
            let del = text.len() - keep_len;
            if del > 0
                && let Some(text_id) = self.inline_text_obj(&start_id)
            {
                let char_keep = Self::byte_offset_to_char_offset(&text, keep_len);
                let char_del = Self::byte_offset_to_char_offset(&text[keep_len..], del);
                self.doc
                    .splice_text(&text_id, char_keep, char_del as isize, "")
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }

        // Truncate end inline node
        if let Some((_, end_id)) = self.doc.get(&content_id, end_inline).ok().flatten() {
            let text = self.inline_text(&end_id);
            let del = end_offset.min(text.len());
            if del > 0
                && let Some(text_id) = self.inline_text_obj(&end_id)
            {
                let char_del = Self::byte_offset_to_char_offset(&text, del);
                self.doc
                    .splice_text(&text_id, 0, char_del as isize, "")
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }

        // Remove intermediate inline nodes (between start+1 and end-1)
        // Remove in reverse to keep indices stable
        if end_inline > start_inline + 1 {
            for _ in (start_inline + 1..end_inline).rev() {
                self.doc
                    .delete(&content_id, start_inline + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }

        // Now clean up: remove empty start/end nodes
        // After removing intermediates, end node is now at start_inline + 1
        let end_new_idx = start_inline + 1;

        // Check end node (check it first since removing start would shift it)
        let end_empty =
            if let Some((_, eid)) = self.doc.get(&content_id, end_new_idx).ok().flatten() {
                self.inline_text(&eid).is_empty()
            } else {
                false
            };

        let start_empty =
            if let Some((_, sid)) = self.doc.get(&content_id, start_inline).ok().flatten() {
                self.inline_text(&sid).is_empty()
            } else {
                false
            };

        let total = self.doc.length(&content_id);
        // Don't remove if it would leave the block with no inlines
        if end_empty && total > 1 {
            self.doc
                .delete(&content_id, end_new_idx)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
        }
        let total = self.doc.length(&content_id);
        if start_empty && total > 1 {
            self.doc
                .delete(&content_id, start_inline)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
        }

        Ok(())
    }

    /// Set the text of a block (clears all inlines, creates single plain text node).
    /// Only used in tests; production code uses mark-preserving operations.
    #[cfg(test)]
    #[allow(dead_code)]
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
        let text_obj = self
            .doc
            .put_object(&text_node, "text", ObjType::Text)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        if !text.is_empty() {
            self.doc
                .splice_text(&text_obj, 0, 0, text)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
        }
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

            let text = self.inline_text(&inline_id);

            if start.text_offset == 0 && end.text_offset >= text.len() {
                // Covers entire node - add mark directly
                self.append_mark_to_inline(&inline_id, &mark)?;
            } else {
                // Check if the inline already has this mark — if so, no split needed.
                // This prevents fragmentation when typing with stored marks active:
                // insert_text appends to the existing marked inline, then add_mark
                // would needlessly split it into per-character nodes.
                let existing_marks = self.read_marks(&inline_id);
                if !existing_marks.iter().any(|m| m.mark_type == mark.mark_type) {
                    // Need to split the inline node into up to 3 parts
                    self.split_and_mark_inline(
                        &content_id,
                        start.inline_index,
                        start.text_offset,
                        end.text_offset,
                        &mark,
                    )?;
                }
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
            if let Some((_, mid)) = self.doc.get(&marks_id, i).ok().flatten()
                && self.get_str(&mid, "type").as_deref() == Some(&mark.mark_type)
            {
                return Ok(()); // Already has this mark
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

        let text = self.inline_text(&inline_id);
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
    pub(crate) fn insert_text_inline(
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
        let text_obj = self
            .doc
            .put_object(&node, "text", ObjType::Text)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        if !text.is_empty() {
            self.doc
                .splice_text(&text_obj, 0, 0, text)
                .map_err(|e| EditorError::Automerge(e.to_string()))?;
        }
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
                    let marks_id = match self.doc.get(&inline_id, "marks").ok().flatten().and_then(
                        |(val, id)| {
                            if matches!(val, automerge::Value::Object(ObjType::List)) {
                                Some(id)
                            } else {
                                None
                            }
                        },
                    ) {
                        Some(id) => id,
                        None => continue,
                    };
                    // Find and remove the mark
                    let len = self.doc.length(&marks_id);
                    for i in (0..len).rev() {
                        if let Some((_, mid)) = self.doc.get(&marks_id, i).ok().flatten()
                            && self.get_str(&mid, "type").as_deref() == Some(mark_type)
                        {
                            self.doc
                                .delete(&marks_id, i)
                                .map_err(|e| EditorError::Automerge(e.to_string()))?;
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
        let block_type = self
            .block_type(block_idx)
            .unwrap_or_else(|| "paragraph".into());

        let block_id = self
            .block_obj(block_idx)
            .ok_or_else(|| EditorError::CommandFailed("Block not found".into()))?;
        let content_id = self
            .block_content_obj(&block_id)
            .ok_or_else(|| EditorError::CommandFailed("Block content not found".into()))?;

        let split_inline = resolved.inline_index;
        let split_offset = resolved.text_offset;

        // Determine which inlines go to the new block.
        // If split_offset is in the middle of an inline, we need to split it.
        let mut new_block_inlines: Vec<(String, Vec<MarkData>)> = Vec::new();

        // Read the split inline node's text and marks
        let (split_text, split_marks) =
            if let Some((_, inline_id)) = self.doc.get(&content_id, split_inline).ok().flatten() {
                let text = self.inline_text(&inline_id);
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
                let del = split_text.len() - split_offset;
                if del > 0
                    && let Some(text_id) = self.inline_text_obj(&inline_id)
                {
                    let char_split = Self::byte_offset_to_char_offset(&split_text, split_offset);
                    let char_del =
                        Self::byte_offset_to_char_offset(&split_text[split_offset..], del);
                    self.doc
                        .splice_text(&text_id, char_split, char_del as isize, "")
                        .map_err(|e| EditorError::Automerge(e.to_string()))?;
                }
            }
            // Remove all inlines after split_inline
            let total = self.doc.length(&content_id);
            for _ in (split_inline + 1..total).rev() {
                self.doc
                    .delete(&content_id, split_inline + 1)
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        } else if split_at_start_of_inline {
            // Remove all inlines from split_inline onward
            let total = self.doc.length(&content_id);
            for _ in (split_inline..total).rev() {
                self.doc
                    .delete(&content_id, split_inline)
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
                self.doc
                    .delete(&content_id, split_inline + 1)
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

    /// Insert a new block at the given index with a specific type and attributes.
    ///
    /// The block is created with a single empty text inline node.
    /// If `at` is beyond the current block count, it's clamped to the end.
    pub fn insert_block_at(
        &mut self,
        at: usize,
        block_type: &str,
        attrs: Option<HashMap<String, String>>,
    ) -> Result<(), EditorError> {
        let at = at.min(self.block_count());
        let block = self
            .doc
            .insert_object(&self.content_id, at, ObjType::Map)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.doc
            .put(&block, "type", block_type)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        let attrs_id = self
            .doc
            .put_object(&block, "attrs", ObjType::Map)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        if let Some(attrs) = attrs {
            for (k, v) in &attrs {
                self.doc
                    .put(&attrs_id, k.as_str(), v.as_str())
                    .map_err(|e| EditorError::Automerge(e.to_string()))?;
            }
        }
        let inline_content = self
            .doc
            .put_object(&block, "content", ObjType::List)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        self.insert_text_inline(&inline_content, 0, "", &[])?;
        Ok(())
    }

    /// Delete a block at the given index.
    ///
    /// Returns error if this is the last block (document must have at least one).
    pub fn delete_block(&mut self, block_index: usize) -> Result<(), EditorError> {
        if self.block_count() <= 1 {
            return Err(EditorError::CommandFailed(
                "Cannot delete the last block".into(),
            ));
        }
        if block_index >= self.block_count() {
            return Err(EditorError::CommandFailed(format!(
                "Block index {} out of bounds ({})",
                block_index,
                self.block_count()
            )));
        }
        self.doc
            .delete(&self.content_id, block_index)
            .map_err(|e| EditorError::Automerge(e.to_string()))?;
        Ok(())
    }
}
