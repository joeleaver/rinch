//! Translation of Automerge patches to CE-level remote operations.
//!
//! When a remote peer's changes arrive via `load_incremental`, Automerge can
//! produce patches (via `diff`) describing exactly what changed. This module
//! translates those low-level Automerge patches into CE-level operations that
//! can be applied directly to the ContentEditable DOM.
//!
//! This keeps the collaboration pipeline as "CRDT operations all the way
//! through" — no block-data diffing or snapshot comparison needed.

use super::EditorDocument;
use automerge::{ObjId, Patch, PatchAction, ReadDoc};

/// A CE-level operation produced by a remote peer's changes.
///
/// These map directly to `ContentEditableApi` methods. The consumer
/// (typically the CE bridge) applies these to the DOM in order.
#[derive(Debug, Clone, PartialEq)]
pub enum CeRemoteOp {
    /// Insert text at a flat document position.
    InsertText {
        /// Flat character offset in the document.
        pos: usize,
        /// The text to insert.
        text: String,
    },
    /// Delete a range of text/content.
    DeleteRange {
        /// Start of the range (flat offset).
        start: usize,
        /// End of the range (flat offset).
        end: usize,
    },
    /// Split a block at a position (Enter key equivalent).
    SplitBlock {
        /// The flat position where the split occurs.
        pos: usize,
    },
    /// Change a block's type.
    SetBlockType {
        /// Block index in the document.
        block_idx: usize,
        /// New block type (e.g., "paragraph", "heading").
        block_type: String,
        /// Block attributes (e.g., {"level": "2"} for h2).
        attrs: std::collections::HashMap<String, String>,
    },
    /// Add a mark (formatting) to a range.
    AddMark {
        /// Start of the range (flat offset).
        start: usize,
        /// End of the range (flat offset).
        end: usize,
        /// Mark type (e.g., "bold", "italic").
        mark_type: String,
    },
    /// Remove a mark from a range.
    RemoveMark {
        /// Start of the range (flat offset).
        start: usize,
        /// End of the range (flat offset).
        end: usize,
        /// Mark type to remove.
        mark_type: String,
    },
}

impl EditorDocument {
    /// Translate Automerge patches into CE-level remote operations.
    ///
    /// The patches come from `diff(before_heads, after_heads)` after
    /// `load_incremental`. Each patch targets a specific Automerge object;
    /// this method resolves that to a flat document position.
    pub fn patches_to_ce_ops(&self, patches: &[Patch]) -> Vec<CeRemoteOp> {
        let mut ops = Vec::new();

        for patch in patches {
            match &patch.action {
                PatchAction::SpliceText { index, value, .. } => {
                    // Text was inserted into a Text object.
                    // Find which block/inline this text object belongs to,
                    // then compute the flat position.
                    if let Some(flat_pos) = self.text_obj_offset_to_flat_pos(&patch.obj, *index) {
                        let text = value.make_string();
                        if !text.is_empty() {
                            ops.push(CeRemoteOp::InsertText {
                                pos: flat_pos,
                                text,
                            });
                        }
                    }
                }

                PatchAction::DeleteSeq { index, length } => {
                    // Elements deleted from a sequence.
                    // Could be text characters deleted from a Text object,
                    // or blocks deleted from the content list,
                    // or inlines deleted from a block's content.
                    if patch.obj == self.content_id {
                        // Block(s) deleted from the content list.
                        // Map block index to flat position for the block separator.
                        if let Some(flat_pos) = self.block_start_flat_pos(*index) {
                            // Deleting a block = deleting its text + the preceding separator
                            // For simplicity, compute the range.
                            let end = if *index + *length <= self.block_count() {
                                self.block_start_flat_pos(*index + *length)
                                    .unwrap_or(self.text_length())
                            } else {
                                self.text_length()
                            };
                            if end > flat_pos {
                                ops.push(CeRemoteOp::DeleteRange {
                                    start: flat_pos,
                                    end,
                                });
                            }
                        }
                    } else {
                        // Text characters deleted from a Text object
                        if let Some(flat_pos) = self.text_obj_offset_to_flat_pos(&patch.obj, *index)
                        {
                            ops.push(CeRemoteOp::DeleteRange {
                                start: flat_pos,
                                end: flat_pos + length,
                            });
                        }
                    }
                }

                PatchAction::Insert { index, values } => {
                    // New elements inserted into a list.
                    if patch.obj == self.content_id {
                        // New block(s) inserted — this is a split_block.
                        // Each inserted block = a split at the boundary.
                        if let Some(flat_pos) = self.block_start_flat_pos(*index) {
                            for _ in 0..values.len() {
                                ops.push(CeRemoteOp::SplitBlock { pos: flat_pos });
                            }
                        }
                    }
                    // Inserts into inline content lists are structural changes
                    // that will be reflected through their text children.
                }

                PatchAction::PutMap { key, value, .. } => {
                    // A map key was set. Could be block type change.
                    if key == "type"
                        && let Some(block_idx) = self.obj_to_block_index(&patch.obj)
                        && let automerge::Value::Scalar(s) = &value.0
                        && let automerge::ScalarValue::Str(smol) = s.as_ref()
                    {
                        ops.push(CeRemoteOp::SetBlockType {
                            block_idx,
                            block_type: smol.to_string(),
                            attrs: self.block_attrs(block_idx).unwrap_or_default(),
                        });
                    }
                }

                PatchAction::Mark { marks } => {
                    // Marks changed on a text object.
                    for mark in marks {
                        if let Some(base_pos) = self.text_obj_offset_to_flat_pos(&patch.obj, 0) {
                            let start = base_pos + mark.start;
                            let end = base_pos + mark.end;
                            let mark_type = mark.name().to_string();
                            let is_active = mark.value() != &automerge::ScalarValue::Null;

                            if is_active {
                                ops.push(CeRemoteOp::AddMark {
                                    start,
                                    end,
                                    mark_type,
                                });
                            } else {
                                ops.push(CeRemoteOp::RemoveMark {
                                    start,
                                    end,
                                    mark_type,
                                });
                            }
                        }
                    }
                }

                // PutSeq, Increment, Conflict — not used in our document model
                _ => {}
            }
        }

        ops
    }

    /// Map a Text object ObjId + character offset to a flat document position.
    ///
    /// Walks the document structure to find which block and inline node owns
    /// this text object, then computes the cumulative offset.
    fn text_obj_offset_to_flat_pos(&self, text_obj: &ObjId, offset: usize) -> Option<usize> {
        let mut flat_pos = 0usize;
        let block_count = self.block_count();

        for block_idx in 0..block_count {
            if block_idx > 0 {
                flat_pos += 1; // block separator
            }

            let block_id = self.block_obj(block_idx)?;
            let content_id = self.block_content_obj(&block_id)?;
            let inline_count = self.doc.length(&content_id);

            for inline_idx in 0..inline_count {
                let (_, inline_id) = self.doc.get(&content_id, inline_idx).ok()??;
                let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();

                match inline_type.as_str() {
                    "text" => {
                        // Check if this inline's text object matches
                        if let Some(tid) = self.inline_text_obj(&inline_id)
                            && tid == *text_obj
                        {
                            return Some(flat_pos + offset);
                        }
                        // Also check the inline_id itself (for legacy scalar text)
                        if inline_id == *text_obj {
                            return Some(flat_pos + offset);
                        }
                        let text_len = self.inline_text(&inline_id).len();
                        flat_pos += text_len;
                    }
                    "hard_break" => {
                        flat_pos += 1;
                    }
                    _ => {}
                }
            }
        }

        None
    }

    /// Map a block's Automerge ObjId to its block index.
    fn obj_to_block_index(&self, obj: &ObjId) -> Option<usize> {
        let block_count = self.block_count();
        for i in 0..block_count {
            if let Some(block_id) = self.block_obj(i)
                && block_id == *obj
            {
                return Some(i);
            }
        }
        None
    }

    /// Get the flat position of the start of a block.
    fn block_start_flat_pos(&self, block_idx: usize) -> Option<usize> {
        let mut flat_pos = 0usize;
        let block_count = self.block_count();
        if block_idx > block_count {
            return None;
        }

        for i in 0..block_idx {
            if i > 0 {
                flat_pos += 1; // block separator
            }
            let text_len = self.block_text(i).map(|t| t.len()).unwrap_or(0);
            flat_pos += text_len;
        }

        if block_idx > 0 {
            flat_pos += 1; // separator before target block
        }

        Some(flat_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Position;

    #[test]
    fn text_obj_offset_maps_correctly() {
        let mut doc = EditorDocument::new();
        doc.insert_text(Position(0), "Hello").unwrap();
        doc.split_block(Position(5)).unwrap();
        doc.insert_text(Position(6), "World").unwrap();

        // Block 0: "Hello" (pos 0-4), Block 1: "World" (pos 6-10)
        // Separator at pos 5
        assert_eq!(doc.block_start_flat_pos(0), Some(0));
        assert_eq!(doc.block_start_flat_pos(1), Some(6));
    }

    #[test]
    fn patches_from_insert() {
        let mut peer1 = EditorDocument::new();
        peer1.insert_text(Position(0), "Hello").unwrap();

        let mut peer2 = EditorDocument::from_bytes(&peer1.to_bytes()).unwrap();
        let heads_before = peer2.get_heads();

        // peer1 inserts " World"
        peer1.insert_text(Position(5), " World").unwrap();
        let delta = peer1.save_incremental();

        // peer2 loads and gets patches
        peer2.load_incremental(&delta).unwrap();
        let heads_after = peer2.get_heads();
        let patches = peer2.doc.diff(&heads_before, &heads_after);
        let ops = peer2.patches_to_ce_ops(&patches);

        assert!(!ops.is_empty());
        match &ops[0] {
            CeRemoteOp::InsertText { pos, text } => {
                assert_eq!(*pos, 5);
                assert_eq!(text, " World");
            }
            other => panic!("Expected InsertText, got {:?}", other),
        }
    }

    #[test]
    fn patches_from_delete() {
        let mut peer1 = EditorDocument::new();
        peer1.insert_text(Position(0), "Hello World").unwrap();

        let mut peer2 = EditorDocument::from_bytes(&peer1.to_bytes()).unwrap();
        let heads_before = peer2.get_heads();

        // peer1 deletes " World" (positions 5-11)
        peer1
            .delete_range(crate::document::Range::new(5usize, 11usize))
            .unwrap();
        let delta = peer1.save_incremental();

        peer2.load_incremental(&delta).unwrap();
        let heads_after = peer2.get_heads();
        let patches = peer2.doc.diff(&heads_before, &heads_after);
        let ops = peer2.patches_to_ce_ops(&patches);

        assert!(!ops.is_empty());
        match &ops[0] {
            CeRemoteOp::DeleteRange { start, end } => {
                assert_eq!(*start, 5);
                assert_eq!(*end, 11);
            }
            other => panic!("Expected DeleteRange, got {:?}", other),
        }
    }
}
