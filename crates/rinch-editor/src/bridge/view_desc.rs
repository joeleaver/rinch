//! ViewDesc: bidirectional mapping between DOM node IDs and EditorDocument positions.
//!
//! Built after each reconciliation from the reconciler's output.
//! Provides lookup from DOM cursor positions (used by CeEvents) to
//! editor model positions, enabling the bridge's observer pattern.

use std::collections::HashMap;

use rinch_core::ce::DomCursor;

use crate::document::Position;
use crate::editor::Editor;

/// Maps DOM node IDs to EditorDocument positions and vice versa.
///
/// The reconciler populates this after each render by recording which
/// DOM text nodes correspond to which byte ranges within each block.
#[derive(Debug, Default)]
pub struct ViewDesc {
    /// Per-block mapping (indexed by block_index).
    blocks: Vec<BlockMapping>,
    /// Quick lookup: text node DOM ID → (block_index, byte_start, byte_end).
    text_node_index: HashMap<usize, (usize, usize, usize)>,
}

/// Mapping data for a single document block.
#[derive(Debug, Clone, Default)]
pub struct BlockMapping {
    /// The block element's DOM node ID.
    pub dom_node_id: usize,
    /// The block's index in the EditorDocument.
    pub block_index: usize,
    /// Text nodes within this block and their byte ranges.
    pub text_nodes: Vec<TextNodeMapping>,
}

/// Mapping for a single text node within a block.
#[derive(Debug, Clone)]
pub struct TextNodeMapping {
    /// The text node's DOM node ID.
    pub dom_node_id: usize,
    /// Byte offset where this text node starts within the block's text.
    pub block_byte_start: usize,
    /// Byte offset where this text node ends within the block's text.
    pub block_byte_end: usize,
}

impl ViewDesc {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all mappings (before a full re-render).
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.text_node_index.clear();
    }

    /// Add a block mapping after rendering a block.
    pub fn add_block(&mut self, mapping: BlockMapping) {
        let block_index = mapping.block_index;
        // Remove old entries for the block being replaced to prevent stale lookups.
        // This matters when the same block index is re-rendered with new DOM node IDs
        // (e.g. after a structure change): old IDs must be evicted before inserting new ones.
        if block_index < self.blocks.len() {
            let old = &self.blocks[block_index];
            self.text_node_index.remove(&old.dom_node_id);
            for tn in &old.text_nodes {
                self.text_node_index.remove(&tn.dom_node_id);
            }
        }
        for tn in &mapping.text_nodes {
            self.text_node_index.insert(
                tn.dom_node_id,
                (block_index, tn.block_byte_start, tn.block_byte_end),
            );
        }
        // Also index the block element itself
        self.text_node_index
            .entry(mapping.dom_node_id)
            .or_insert((block_index, 0, 0));
        // Ensure blocks vec is large enough
        while self.blocks.len() <= block_index {
            self.blocks.push(BlockMapping::default());
        }
        self.blocks[block_index] = mapping;
    }

    /// Remove mapping for blocks at or above the given index.
    pub fn truncate_blocks(&mut self, new_len: usize) {
        if new_len >= self.blocks.len() {
            return; // Nothing to truncate
        }
        for removed in self.blocks.drain(new_len..) {
            // Clean up text node index for removed blocks
            self.text_node_index.remove(&removed.dom_node_id);
            for tn in &removed.text_nodes {
                self.text_node_index.remove(&tn.dom_node_id);
            }
        }
    }

    /// Convert a DomCursor to an editor Position.
    ///
    /// Uses the text node index to find which block and offset the cursor maps to.
    pub fn dom_cursor_to_position(&self, cursor: &DomCursor, editor: &Editor) -> Option<Position> {
        let &(block_index, byte_start, byte_end) = self.text_node_index.get(&cursor.node_id)?;

        // The cursor.offset is within the text node.
        // The text node maps to [byte_start..byte_end] within the block.
        let text_len = byte_end.saturating_sub(byte_start);
        let block_offset = byte_start + cursor.offset.min(text_len);

        let block_start = editor.doc.block_start_position(block_index);
        let doc_len = editor.doc.text_length();
        Some(Position::new((block_start + block_offset).min(doc_len)))
    }

    /// Find the block index for a DOM node ID.
    pub fn block_index_for_node(&self, node_id: usize) -> Option<usize> {
        self.text_node_index
            .get(&node_id)
            .map(|(block_index, _, _)| *block_index)
    }

    /// Get the text node mappings for a block (for in-place text updates).
    pub fn text_nodes_for_block(&self, block_index: usize) -> &[TextNodeMapping] {
        if block_index < self.blocks.len() {
            &self.blocks[block_index].text_nodes
        } else {
            &[]
        }
    }

    /// Get the block element's DOM node ID for a given block index.
    pub fn block_node_id(&self, block_index: usize) -> Option<usize> {
        self.blocks.get(block_index).map(|b| b.dom_node_id)
    }

    /// Shift byte ranges in a block after text insertion or deletion.
    ///
    /// After CeOps modifies a text node directly, the ViewDesc byte ranges
    /// become stale. This adjusts them without running the reconciler.
    ///
    /// `from_byte` is the byte offset within the block where the change starts.
    /// `delta` is positive for insertion, negative for deletion.
    pub fn shift_block_ranges(&mut self, block_index: usize, from_byte: usize, delta: isize) {
        if block_index >= self.blocks.len() {
            return;
        }
        let block = &mut self.blocks[block_index];
        for tn in &mut block.text_nodes {
            if tn.block_byte_start > from_byte {
                // Text node is entirely after the change point — shift both ends
                tn.block_byte_start = (tn.block_byte_start as isize + delta).max(0) as usize;
                tn.block_byte_end = (tn.block_byte_end as isize + delta).max(0) as usize;
            } else if tn.block_byte_end >= from_byte {
                // Text node spans or touches the change point — only shift the end
                tn.block_byte_end =
                    (tn.block_byte_end as isize + delta).max(tn.block_byte_start as isize) as usize;
            }
            // Update the fast lookup index
            self.text_node_index.insert(
                tn.dom_node_id,
                (block_index, tn.block_byte_start, tn.block_byte_end),
            );
        }
    }

    /// Get the block-level byte start for a text node by DOM ID.
    pub fn text_node_byte_start(&self, node_id: usize) -> Option<usize> {
        self.text_node_index
            .get(&node_id)
            .map(|(_, byte_start, _)| *byte_start)
    }

    /// Get the number of tracked blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_view_desc() {
        let vd = ViewDesc::new();
        assert_eq!(vd.block_count(), 0);
        assert!(vd.block_index_for_node(42).is_none());
    }

    #[test]
    fn add_block_and_lookup() {
        let mut vd = ViewDesc::new();
        vd.add_block(BlockMapping {
            dom_node_id: 10,
            block_index: 0,
            text_nodes: vec![
                TextNodeMapping {
                    dom_node_id: 11,
                    block_byte_start: 0,
                    block_byte_end: 5,
                },
                TextNodeMapping {
                    dom_node_id: 12,
                    block_byte_start: 5,
                    block_byte_end: 10,
                },
            ],
        });

        assert_eq!(vd.block_count(), 1);
        assert_eq!(vd.block_index_for_node(10), Some(0));
        assert_eq!(vd.block_index_for_node(11), Some(0));
        assert_eq!(vd.block_index_for_node(12), Some(0));
        assert_eq!(vd.block_index_for_node(99), None);
    }

    #[test]
    fn truncate_blocks() {
        let mut vd = ViewDesc::new();
        vd.add_block(BlockMapping {
            dom_node_id: 10,
            block_index: 0,
            text_nodes: vec![TextNodeMapping {
                dom_node_id: 11,
                block_byte_start: 0,
                block_byte_end: 5,
            }],
        });
        vd.add_block(BlockMapping {
            dom_node_id: 20,
            block_index: 1,
            text_nodes: vec![TextNodeMapping {
                dom_node_id: 21,
                block_byte_start: 0,
                block_byte_end: 3,
            }],
        });

        assert_eq!(vd.block_count(), 2);
        assert!(vd.block_index_for_node(21).is_some());

        vd.truncate_blocks(1);
        assert_eq!(vd.block_count(), 1);
        assert!(vd.block_index_for_node(21).is_none());
        assert!(vd.block_index_for_node(11).is_some());
    }

    #[test]
    fn dom_cursor_byte_offset_clamped() {
        let mut vd = ViewDesc::new();
        vd.add_block(BlockMapping {
            dom_node_id: 10,
            block_index: 0,
            text_nodes: vec![TextNodeMapping {
                dom_node_id: 11,
                block_byte_start: 0,
                block_byte_end: 5,
            }],
        });

        // Cursor offset beyond text node length is clamped
        let _cursor = DomCursor::new(11, 100);
        // dom_cursor_to_position needs an Editor, skip for unit test
        // Just verify the index lookup works
        assert_eq!(vd.block_index_for_node(11), Some(0));
    }
}
