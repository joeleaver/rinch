//! Bridge between the ContentEditable DOM and the Automerge-backed EditorDocument.
//!
//! `CeDocBridge` keeps an [`EditorDocument`] in sync with a contenteditable
//! element's DOM, enabling real-time collaboration via the Automerge sync
//! protocol.
//!
//! # Outbound (local edits → sync)
//!
//! The bridge subscribes to [`CeEvent`]s. On each [`flush()`](CeDocBridge::flush),
//! it reads the current CE content via [`extract_content()`], diffs it against
//! the last known state, and applies the delta to the [`EditorDocument`].
//! Call [`generate_sync_message()`](CeDocBridge::generate_sync_message) to
//! produce a message for remote peers.
//!
//! # Inbound (remote edits → DOM)
//!
//! Call [`receive_sync_message()`](CeDocBridge::receive_sync_message) with a
//! message from a remote peer. The bridge applies it to the [`EditorDocument`],
//! then pushes the updated content into the CE DOM via [`load_content()`].
//!
//! # Usage
//!
//! ```rust,ignore
//! use rinch_editor::bridge::CeDocBridge;
//! use rinch_editor::sync::{SyncState, SyncMessage};
//!
//! // Create bridge for a contenteditable element
//! let mut bridge = CeDocBridge::new(ce_node_id);
//! let mut sync_state = SyncState::new();
//!
//! // After local edits (call once per frame):
//! bridge.flush();
//! if let Some(msg) = bridge.generate_sync_message(&mut sync_state) {
//!     send_to_peer(msg);
//! }
//!
//! // When receiving from a remote peer:
//! bridge.receive_sync_message(&mut sync_state, remote_msg).unwrap();
//! ```

use std::cell::Cell;
use std::rc::Rc;

use rinch_core::ce::{BlockData, CeEvent, subscribe_ce_events, with_ce_api_for_node};

use super::sync::{SyncMessage, SyncState};
use super::{EditorDocument, MarkData};
use crate::document::{Position, Range};
use crate::error::EditorError;

/// Bridge between a contenteditable element and an Automerge-backed document.
///
/// Keeps the two in sync so that local DOM edits flow into the CRDT and
/// remote CRDT changes flow back into the DOM.
pub struct CeDocBridge {
    doc: EditorDocument,
    ce_node_id: usize,
    last_blocks: Vec<BlockData>,
    dirty: Rc<Cell<bool>>,
    applying_remote: Rc<Cell<bool>>,
}

impl CeDocBridge {
    /// Create a bridge for a contenteditable element.
    ///
    /// Reads the element's current content and initializes the
    /// [`EditorDocument`] from it. Subscribes to [`CeEvent`]s so that
    /// [`flush()`](Self::flush) knows when to re-sync.
    pub fn new(ce_node_id: usize) -> Self {
        let initial_blocks = with_ce_api_for_node(ce_node_id, |api| api.borrow().extract_content())
            .unwrap_or_default();
        Self::new_inner(
            ce_node_id,
            EditorDocument::from_block_data(&initial_blocks),
            initial_blocks,
        )
    }

    /// Create a bridge with an existing [`EditorDocument`].
    ///
    /// The document's content is pushed into the CE element immediately.
    pub fn new_with_doc(ce_node_id: usize, doc: EditorDocument) -> Self {
        let blocks = doc.to_block_data();
        // Push document content into the CE DOM
        with_ce_api_for_node(ce_node_id, |api| {
            api.borrow_mut().load_content(&blocks);
        });
        Self::new_inner(ce_node_id, doc, blocks)
    }

    fn new_inner(ce_node_id: usize, doc: EditorDocument, initial_blocks: Vec<BlockData>) -> Self {
        let dirty = Rc::new(Cell::new(false));
        let applying_remote = Rc::new(Cell::new(false));

        // Subscribe to CE events — just set the dirty flag.
        // We can't process events synchronously because CeOps is borrowed
        // during dispatch. Processing happens in flush().
        let dirty_flag = dirty.clone();
        let remote_flag = applying_remote.clone();
        subscribe_ce_events(Rc::new(move |event| {
            if remote_flag.get() {
                return;
            }
            if matches!(
                event,
                CeEvent::TextInserted { .. }
                    | CeEvent::TextDeleted { .. }
                    | CeEvent::TextNodeCreated { .. }
                    | CeEvent::NodeRemoved { .. }
                    | CeEvent::BlockSplit { .. }
                    | CeEvent::BlockJoined { .. }
                    | CeEvent::BlockTypeChanged { .. }
                    | CeEvent::SelectionWrapped { .. }
                    | CeEvent::SelectionUnwrapped { .. }
                    | CeEvent::ListItemOutdented { .. }
                    | CeEvent::BlockIndented { .. }
                    | CeEvent::UndoApplied
                    | CeEvent::RedoApplied
                    | CeEvent::HtmlPasted { .. }
            ) {
                dirty_flag.set(true);
            }
        }));

        Self {
            doc,
            ce_node_id,
            last_blocks: initial_blocks,
            dirty,
            applying_remote,
        }
    }

    /// Sync local CE DOM changes into the [`EditorDocument`].
    ///
    /// Reads the current CE content, diffs against the last known state,
    /// and applies incremental mutations to the Automerge document.
    /// Call this once per frame (or before [`generate_sync_message`]).
    pub fn flush(&mut self) {
        if !self.dirty.get() {
            return;
        }
        self.dirty.set(false);

        let new_blocks =
            match with_ce_api_for_node(self.ce_node_id, |api| api.borrow().extract_content()) {
                Some(blocks) => blocks,
                None => return,
            };

        if new_blocks == self.last_blocks {
            return;
        }

        let old_blocks = std::mem::replace(&mut self.last_blocks, new_blocks.clone());
        self.apply_diff(&old_blocks, &new_blocks);
    }

    /// Generate a sync message to send to a remote peer.
    ///
    /// Calls [`flush()`](Self::flush) first to ensure all local edits are
    /// captured. Returns `None` if there is nothing new to send.
    pub fn generate_sync_message(&mut self, sync_state: &mut SyncState) -> Option<SyncMessage> {
        self.flush();
        self.doc.generate_sync_message(sync_state)
    }

    /// Receive a sync message from a remote peer and apply changes to the DOM.
    ///
    /// Updates the [`EditorDocument`], then pushes any content changes into
    /// the CE element via [`load_content()`]. The bridge suppresses outbound
    /// sync during this operation to avoid feedback loops.
    pub fn receive_sync_message(
        &mut self,
        sync_state: &mut SyncState,
        message: SyncMessage,
    ) -> Result<(), EditorError> {
        // Flush any pending local changes first so we don't lose them
        self.flush();

        let old_blocks = self.doc.to_block_data();
        self.doc.receive_sync_message(sync_state, message)?;
        let new_blocks = self.doc.to_block_data();

        if old_blocks != new_blocks {
            self.applying_remote.set(true);
            with_ce_api_for_node(self.ce_node_id, |api| {
                api.borrow_mut().load_content(&new_blocks);
            });
            self.applying_remote.set(false);
            self.last_blocks = new_blocks;
        }

        Ok(())
    }

    /// Access the underlying [`EditorDocument`].
    pub fn document(&self) -> &EditorDocument {
        &self.doc
    }

    /// Mutably access the underlying [`EditorDocument`].
    pub fn document_mut(&mut self) -> &mut EditorDocument {
        &mut self.doc
    }

    /// The CE element node ID this bridge is attached to.
    pub fn ce_node_id(&self) -> usize {
        self.ce_node_id
    }

    // ── Diff Engine ──────────────────────────────────────────────────────

    /// Apply the diff between old and new block data to the EditorDocument.
    fn apply_diff(&mut self, old_blocks: &[BlockData], new_blocks: &[BlockData]) {
        // Common case: same number of blocks — incremental text/mark diff
        if old_blocks.len() == new_blocks.len() {
            self.apply_same_block_count_diff(old_blocks, new_blocks);
            return;
        }

        // Structural change (split, join, paste, undo, etc.) — full rebuild
        self.rebuild_from_blocks(new_blocks);
    }

    /// Incremental diff when block count hasn't changed.
    ///
    /// For each block, computes a character-level text diff and mark diff,
    /// then applies the minimal set of mutations to the EditorDocument.
    fn apply_same_block_count_diff(&mut self, old_blocks: &[BlockData], new_blocks: &[BlockData]) {
        let mut block_start = 0usize;

        for (i, (old_b, new_b)) in old_blocks.iter().zip(new_blocks.iter()).enumerate() {
            if i > 0 {
                block_start += 1; // block separator (virtual \n)
            }

            let old_text = flatten_block_text(old_b);
            let new_text = flatten_block_text(new_b);

            // Text diff
            if old_text != new_text
                && let Some(diff) = diff_text(&old_text, &new_text)
            {
                if diff.delete_len > 0 {
                    let _ = self.doc.delete_range(Range::new(
                        block_start + diff.offset,
                        block_start + diff.offset + diff.delete_len,
                    ));
                }
                if !diff.insert_text.is_empty() {
                    let _ = self
                        .doc
                        .insert_text(Position(block_start + diff.offset), &diff.insert_text);
                }
            }

            // Block type change
            if old_b.block_type != new_b.block_type || old_b.attrs != new_b.attrs {
                let attrs = if new_b.attrs.is_empty() {
                    None
                } else {
                    Some(new_b.attrs.clone())
                };
                let _ = self.doc.set_block_type(i, &new_b.block_type, attrs);
            }

            // Mark diff — only if runs changed
            if old_b.content != new_b.content {
                let block_text_len = new_text.len();
                self.sync_block_marks(i, block_start, block_text_len, old_b, new_b);
            }

            block_start += new_text.len();
        }
    }

    /// Sync marks for a single block by removing stale marks and adding new ones.
    fn sync_block_marks(
        &mut self,
        _block_index: usize,
        block_start: usize,
        block_text_len: usize,
        old_block: &BlockData,
        new_block: &BlockData,
    ) {
        if block_text_len == 0 {
            return;
        }
        let block_range = Range::new(block_start, block_start + block_text_len);

        // Collect all mark types present in old content
        let old_mark_types: std::collections::HashSet<&str> = old_block
            .content
            .iter()
            .flat_map(|r| r.marks.iter().map(|m| m.mark_type.as_str()))
            .collect();

        // Remove all old marks from the block range
        for mark_type in &old_mark_types {
            let _ = self.doc.remove_mark(block_range, mark_type);
        }

        // Apply new marks
        let mut offset = 0;
        for run in &new_block.content {
            if !run.marks.is_empty() && !run.text.is_empty() {
                let run_start = block_start + offset;
                let run_end = run_start + run.text.len();
                let run_range = Range::new(run_start, run_end);
                for mark in &run.marks {
                    let mark_data = MarkData::with_attrs(&mark.mark_type, mark.attrs.clone());
                    let _ = self.doc.add_mark(run_range, mark_data);
                }
            }
            offset += run.text.len();
        }
    }

    /// Full rebuild: clear the EditorDocument and repopulate from blocks.
    ///
    /// Used when the block structure changed (splits, joins, paste, undo).
    /// Produces character-level Automerge operations, preserving CRDT history.
    fn rebuild_from_blocks(&mut self, new_blocks: &[BlockData]) {
        // Step 1: Delete all existing content
        let total_len = self.doc.text_length();
        if total_len > 0 {
            let _ = self.doc.delete_range(Range::new(0usize, total_len));
        }
        // Now: 1 empty block

        if new_blocks.is_empty() {
            return;
        }

        // Step 2: First block — set type and insert text
        let first = &new_blocks[0];
        let attrs = if first.attrs.is_empty() {
            None
        } else {
            Some(first.attrs.clone())
        };
        let _ = self.doc.set_block_type(0, &first.block_type, attrs);

        let first_text = flatten_block_text(first);
        if !first_text.is_empty() {
            let _ = self.doc.insert_text(Position(0), &first_text);
        }
        self.apply_block_marks(0, first);

        let mut pos = first_text.len();

        // Step 3: Remaining blocks — split, set type, insert text, marks
        for (i, block) in new_blocks.iter().enumerate().skip(1) {
            let _ = self.doc.split_block(Position(pos));
            pos += 1; // block separator

            let attrs = if block.attrs.is_empty() {
                None
            } else {
                Some(block.attrs.clone())
            };
            let _ = self.doc.set_block_type(i, &block.block_type, attrs);

            let text = flatten_block_text(block);
            if !text.is_empty() {
                let _ = self.doc.insert_text(Position(pos), &text);
            }
            self.apply_block_marks(pos, block);

            pos += text.len();
        }
    }

    /// Apply marks for a single block during rebuild.
    fn apply_block_marks(&mut self, block_start: usize, block: &BlockData) {
        let mut offset = 0;
        for run in &block.content {
            if !run.marks.is_empty() && !run.text.is_empty() {
                let run_start = block_start + offset;
                let run_end = run_start + run.text.len();
                let run_range = Range::new(run_start, run_end);
                for mark in &run.marks {
                    let mark_data = MarkData::with_attrs(&mark.mark_type, mark.attrs.clone());
                    let _ = self.doc.add_mark(run_range, mark_data);
                }
            }
            offset += run.text.len();
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Concatenate all inline run text in a block.
fn flatten_block_text(block: &BlockData) -> String {
    block.content.iter().map(|r| r.text.as_str()).collect()
}

/// Result of diffing two text strings.
struct TextDiff {
    /// Byte offset where the change starts.
    offset: usize,
    /// Number of bytes deleted from old text.
    delete_len: usize,
    /// Text inserted at offset (replacing the deleted bytes).
    insert_text: String,
}

/// Compute the minimal single-span diff between two strings.
///
/// Finds the longest common prefix and suffix, then treats the middle
/// as a single delete + insert. Returns `None` if the strings are equal.
fn diff_text(old: &str, new: &str) -> Option<TextDiff> {
    if old == new {
        return None;
    }

    // Common prefix (char-aligned)
    let prefix_len: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();

    // Common suffix (char-aligned, not overlapping prefix)
    let old_rest = &old[prefix_len..];
    let new_rest = &new[prefix_len..];
    let suffix_len: usize = old_rest
        .chars()
        .rev()
        .zip(new_rest.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();

    let delete_len = old.len() - prefix_len - suffix_len;
    let insert_text = &new[prefix_len..new.len() - suffix_len];

    Some(TextDiff {
        offset: prefix_len,
        delete_len,
        insert_text: insert_text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::ce::{BlockData, InlineMarkData, InlineRunData};
    use std::collections::HashMap;

    fn plain_block(block_type: &str, text: &str) -> BlockData {
        BlockData {
            block_type: block_type.to_string(),
            attrs: HashMap::new(),
            content: vec![InlineRunData {
                text: text.to_string(),
                marks: vec![],
            }],
        }
    }

    fn marked_block(block_type: &str, runs: Vec<(&str, Vec<&str>)>) -> BlockData {
        BlockData {
            block_type: block_type.to_string(),
            attrs: HashMap::new(),
            content: runs
                .into_iter()
                .map(|(text, marks)| InlineRunData {
                    text: text.to_string(),
                    marks: marks
                        .into_iter()
                        .map(|m| InlineMarkData {
                            mark_type: m.to_string(),
                            attrs: HashMap::new(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    // ── diff_text tests ──────────────────────────────────────────────

    #[test]
    fn diff_text_equal() {
        assert!(diff_text("hello", "hello").is_none());
    }

    #[test]
    fn diff_text_insert_at_end() {
        let d = diff_text("hello", "hello world").unwrap();
        assert_eq!(d.offset, 5);
        assert_eq!(d.delete_len, 0);
        assert_eq!(d.insert_text, " world");
    }

    #[test]
    fn diff_text_insert_at_start() {
        let d = diff_text("world", "hello world").unwrap();
        assert_eq!(d.offset, 0);
        assert_eq!(d.delete_len, 0);
        assert_eq!(d.insert_text, "hello ");
    }

    #[test]
    fn diff_text_insert_in_middle() {
        let d = diff_text("helo", "hello").unwrap();
        assert_eq!(d.offset, 3);
        assert_eq!(d.delete_len, 0);
        assert_eq!(d.insert_text, "l");
    }

    #[test]
    fn diff_text_delete_at_end() {
        let d = diff_text("hello world", "hello").unwrap();
        assert_eq!(d.offset, 5);
        assert_eq!(d.delete_len, 6);
        assert_eq!(d.insert_text, "");
    }

    #[test]
    fn diff_text_replace_middle() {
        let d = diff_text("hello world", "hello earth").unwrap();
        assert_eq!(d.offset, 6);
        assert_eq!(d.delete_len, 5);
        assert_eq!(d.insert_text, "earth");
    }

    #[test]
    fn diff_text_unicode() {
        let d = diff_text("café", "cafés").unwrap();
        assert_eq!(d.offset, "café".len());
        assert_eq!(d.delete_len, 0);
        assert_eq!(d.insert_text, "s");
    }

    // ── Incremental sync tests (same block count) ────────────────────

    #[test]
    fn sync_text_insert_within_block() {
        let old = vec![plain_block("paragraph", "Hello")];
        let new = vec![plain_block("paragraph", "Hello World")];

        let mut doc = EditorDocument::from_block_data(&old);
        assert_eq!(doc.to_text(), "Hello");

        let mut bridge = CeDocBridge {
            doc,
            ce_node_id: 0,
            last_blocks: old.clone(),
            dirty: Rc::new(Cell::new(false)),
            applying_remote: Rc::new(Cell::new(false)),
        };

        bridge.apply_diff(&old, &new);
        assert_eq!(bridge.doc.to_text(), "Hello World");
    }

    #[test]
    fn sync_text_delete_within_block() {
        let old = vec![plain_block("paragraph", "Hello World")];
        let new = vec![plain_block("paragraph", "Hello")];

        let mut doc = EditorDocument::from_block_data(&old);
        let mut bridge = CeDocBridge {
            doc,
            ce_node_id: 0,
            last_blocks: old.clone(),
            dirty: Rc::new(Cell::new(false)),
            applying_remote: Rc::new(Cell::new(false)),
        };

        bridge.apply_diff(&old, &new);
        assert_eq!(bridge.doc.to_text(), "Hello");
    }

    #[test]
    fn sync_text_change_in_second_block() {
        let old = vec![
            plain_block("paragraph", "First"),
            plain_block("paragraph", "Second"),
        ];
        let new = vec![
            plain_block("paragraph", "First"),
            plain_block("paragraph", "Second paragraph"),
        ];

        let mut doc = EditorDocument::from_block_data(&old);
        assert_eq!(doc.to_text(), "First\nSecond");

        let mut bridge = CeDocBridge {
            doc,
            ce_node_id: 0,
            last_blocks: old.clone(),
            dirty: Rc::new(Cell::new(false)),
            applying_remote: Rc::new(Cell::new(false)),
        };

        bridge.apply_diff(&old, &new);
        assert_eq!(bridge.doc.to_text(), "First\nSecond paragraph");
    }

    #[test]
    fn sync_block_type_change() {
        let old = vec![plain_block("paragraph", "Title")];
        let new = vec![BlockData {
            block_type: "heading".to_string(),
            attrs: {
                let mut m = HashMap::new();
                m.insert("level".into(), "1".into());
                m
            },
            content: vec![InlineRunData {
                text: "Title".to_string(),
                marks: vec![],
            }],
        }];

        let mut doc = EditorDocument::from_block_data(&old);
        let mut bridge = CeDocBridge {
            doc,
            ce_node_id: 0,
            last_blocks: old.clone(),
            dirty: Rc::new(Cell::new(false)),
            applying_remote: Rc::new(Cell::new(false)),
        };

        bridge.apply_diff(&old, &new);
        assert_eq!(bridge.doc.block_type(0), Some("heading".into()));
    }

    #[test]
    fn sync_mark_change() {
        let old = vec![plain_block("paragraph", "Hello")];
        let new = vec![marked_block("paragraph", vec![("Hello", vec!["bold"])])];

        let mut doc = EditorDocument::from_block_data(&old);
        let mut bridge = CeDocBridge {
            doc,
            ce_node_id: 0,
            last_blocks: old.clone(),
            dirty: Rc::new(Cell::new(false)),
            applying_remote: Rc::new(Cell::new(false)),
        };

        bridge.apply_diff(&old, &new);
        let marks = bridge.doc.marks_at(Position(0));
        assert!(
            marks.iter().any(|m| m.mark_type == "bold"),
            "Expected bold mark, got: {:?}",
            marks
        );
    }

    // ── Rebuild tests (block structure changes) ──────────────────────

    #[test]
    fn rebuild_after_block_split() {
        let old = vec![plain_block("paragraph", "HelloWorld")];
        let new = vec![
            plain_block("paragraph", "Hello"),
            plain_block("paragraph", "World"),
        ];

        let mut doc = EditorDocument::from_block_data(&old);
        let mut bridge = CeDocBridge {
            doc,
            ce_node_id: 0,
            last_blocks: old.clone(),
            dirty: Rc::new(Cell::new(false)),
            applying_remote: Rc::new(Cell::new(false)),
        };

        bridge.apply_diff(&old, &new);
        assert_eq!(bridge.doc.block_count(), 2);
        assert_eq!(bridge.doc.to_text(), "Hello\nWorld");
    }

    #[test]
    fn rebuild_after_block_join() {
        let old = vec![
            plain_block("paragraph", "Hello"),
            plain_block("paragraph", "World"),
        ];
        let new = vec![plain_block("paragraph", "HelloWorld")];

        let mut doc = EditorDocument::from_block_data(&old);
        let mut bridge = CeDocBridge {
            doc,
            ce_node_id: 0,
            last_blocks: old.clone(),
            dirty: Rc::new(Cell::new(false)),
            applying_remote: Rc::new(Cell::new(false)),
        };

        bridge.apply_diff(&old, &new);
        assert_eq!(bridge.doc.block_count(), 1);
        assert_eq!(bridge.doc.to_text(), "HelloWorld");
    }

    #[test]
    fn rebuild_preserves_marks() {
        let old = vec![plain_block("paragraph", "Hello")];
        let new = vec![marked_block(
            "paragraph",
            vec![("Hel", vec![]), ("lo", vec!["bold"])],
        )];

        // Use rebuild path (different block count trick: go through rebuild manually)
        let mut doc = EditorDocument::from_block_data(&old);
        let mut bridge = CeDocBridge {
            doc,
            ce_node_id: 0,
            last_blocks: old.clone(),
            dirty: Rc::new(Cell::new(false)),
            applying_remote: Rc::new(Cell::new(false)),
        };

        bridge.rebuild_from_blocks(&new);
        assert_eq!(bridge.doc.to_text(), "Hello");
        let marks = bridge.doc.marks_at(Position(3));
        assert!(
            marks.iter().any(|m| m.mark_type == "bold"),
            "Expected bold at pos 3, got: {:?}",
            marks
        );
        let marks_start = bridge.doc.marks_at(Position(0));
        assert!(
            !marks_start.iter().any(|m| m.mark_type == "bold"),
            "Should not be bold at pos 0"
        );
    }
}
