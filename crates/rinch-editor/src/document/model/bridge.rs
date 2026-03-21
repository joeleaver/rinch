//! Bridge between the ContentEditable DOM and the Automerge-backed EditorDocument.
//!
//! With the CRDT-native dual-write architecture, the [`EditorDocument`] is
//! maintained directly by CeOps (in the `rinch` crate) — every DOM mutation
//! simultaneously writes to both the DOM and the CRDT. This bridge is now a
//! thin sync wrapper:
//!
//! - **Outbound (local edits → sync):** The EditorDocument is already up-to-date
//!   after each CeOps mutation. Call [`generate_sync_message()`] to produce a
//!   message for remote peers.
//!
//! - **Inbound (remote edits → DOM):** Call [`receive_sync_message()`] with a
//!   message from a remote peer. The bridge applies it to the EditorDocument,
//!   then pushes updated content into the CE DOM via [`load_content()`].
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
//! // After local edits — no flush needed, EditorDocument is already current:
//! if let Some(msg) = bridge.generate_sync_message(&mut sync_state) {
//!     send_to_peer(msg);
//! }
//!
//! // When receiving from a remote peer:
//! bridge.receive_sync_message(&mut sync_state, remote_msg).unwrap();
//! ```

use std::cell::Cell;
use std::rc::Rc;

use rinch_core::ce::{BlockData, InlineMarkData, subscribe_ce_events, with_ce_api_for_node};

use super::sync::{SyncMessage, SyncState};
use super::{EditorDocument, MarkData};
use crate::document::{Position, Range};
use crate::error::EditorError;

/// Bridge between a contenteditable element and an Automerge-backed document.
///
/// With CRDT-native dual-write, CeOps maintains the EditorDocument directly.
/// This bridge handles only sync message generation and inbound remote changes.
pub struct CeDocBridge {
    doc: EditorDocument,
    ce_node_id: usize,
    applying_remote: Rc<Cell<bool>>,
}

impl CeDocBridge {
    /// Create a bridge for a contenteditable element.
    ///
    /// Reads the element's current content and initializes the
    /// [`EditorDocument`] from it.
    pub fn new(ce_node_id: usize) -> Self {
        let initial_blocks = with_ce_api_for_node(ce_node_id, |api| api.borrow().extract_content())
            .unwrap_or_default();
        let applying_remote = Rc::new(Cell::new(false));

        // Subscribe to CE events to suppress outbound sync during inbound apply.
        let remote_flag = applying_remote.clone();
        subscribe_ce_events(Rc::new(move |_event| {
            // With dual-write, we don't need a dirty flag — the EditorDocument
            // is updated in real-time by CeOps. We only subscribe to detect
            // remote-triggered events that we should ignore.
            let _ = remote_flag.get();
        }));

        Self {
            doc: EditorDocument::from_block_data(&initial_blocks),
            ce_node_id,
            applying_remote,
        }
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

        let applying_remote = Rc::new(Cell::new(false));
        Self {
            doc,
            ce_node_id,
            applying_remote,
        }
    }

    /// Sync local CE DOM changes into the [`EditorDocument`].
    ///
    /// With the CRDT-native dual-write architecture, CeOps keeps the
    /// EditorDocument in sync automatically. This method rebuilds content
    /// in-place as a safety net — preserving the Automerge document identity
    /// and change history (unlike `from_block_data` which creates a fresh doc).
    pub fn flush(&mut self) {
        let new_blocks =
            match with_ce_api_for_node(self.ce_node_id, |api| api.borrow().extract_content()) {
                Some(blocks) => blocks,
                None => return,
            };

        let old_blocks = self.doc.to_block_data();
        if new_blocks == old_blocks {
            return;
        }

        rebuild_doc_from_blocks(&mut self.doc, &new_blocks);
    }

    /// Generate a sync message to send to a remote peer.
    ///
    /// Returns `None` if there is nothing new to send.
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
        // Flush to ensure we have the latest local state
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
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Clear an EditorDocument and repopulate from blocks, preserving identity.
///
/// Unlike `EditorDocument::from_block_data()` which creates a fresh Automerge
/// document, this mutates the existing one — preserving actor ID and change
/// history for collaboration.
fn rebuild_doc_from_blocks(doc: &mut EditorDocument, new_blocks: &[BlockData]) {
    // Clear all existing content
    let total_len = doc.text_length();
    if total_len > 0 {
        let _ = doc.delete_range(Range::new(0usize, total_len));
    }
    while doc.block_count() > 1 {
        let _ = doc.delete_block(doc.block_count() - 1);
    }

    if new_blocks.is_empty() {
        return;
    }

    // First block
    let first = &new_blocks[0];
    let attrs = if first.attrs.is_empty() {
        None
    } else {
        Some(first.attrs.clone())
    };
    let _ = doc.set_block_type(0, &first.block_type, attrs);

    let first_text = flatten_block_text(first);
    if !first_text.is_empty() {
        let _ = doc.insert_text(Position(0), &first_text);
    }
    apply_block_marks(doc, 0, first);

    let mut pos = first_text.len();

    // Remaining blocks
    for (i, block) in new_blocks.iter().enumerate().skip(1) {
        let _ = doc.split_block(Position(pos));
        pos += 1;

        let attrs = if block.attrs.is_empty() {
            None
        } else {
            Some(block.attrs.clone())
        };
        let _ = doc.set_block_type(i, &block.block_type, attrs);

        let text = flatten_block_text(block);
        if !text.is_empty() {
            let _ = doc.insert_text(Position(pos), &text);
        }
        apply_block_marks(doc, pos, block);

        pos += text.len();
    }
}

/// Concatenate all inline run text in a block.
fn flatten_block_text(block: &BlockData) -> String {
    block.content.iter().map(|r| r.text.as_str()).collect()
}

/// Apply marks for a single block.
fn apply_block_marks(doc: &mut EditorDocument, block_start: usize, block: &BlockData) {
    let mut offset = 0;
    for run in &block.content {
        if !run.marks.is_empty() && !run.text.is_empty() {
            let run_start = block_start + offset;
            let run_end = run_start + run.text.len();
            let run_range = Range::new(run_start, run_end);
            for mark in &run.marks {
                let mark_data = MarkData::with_attrs(&mark.mark_type, mark.attrs.clone());
                let _ = doc.add_mark(run_range, mark_data);
            }
        }
        offset += run.text.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::ce::{BlockData, InlineMarkData, InlineRunData};
    use std::collections::HashMap;

    use crate::document::Position;

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

    #[test]
    fn from_block_data_roundtrip() {
        let blocks = vec![
            plain_block("paragraph", "Hello"),
            plain_block("paragraph", "World"),
        ];
        let doc = EditorDocument::from_block_data(&blocks);
        assert_eq!(doc.to_text(), "Hello\nWorld");
        assert_eq!(doc.block_count(), 2);
    }

    #[test]
    fn from_block_data_with_marks() {
        let blocks = vec![marked_block(
            "paragraph",
            vec![("Hello ", vec![]), ("world", vec!["bold"])],
        )];
        let doc = EditorDocument::from_block_data(&blocks);
        assert_eq!(doc.to_text(), "Hello world");
        let marks = doc.marks_at(Position(6));
        assert!(
            marks.iter().any(|m| m.mark_type == "bold"),
            "Expected bold mark at pos 6, got: {:?}",
            marks
        );
    }

    #[test]
    fn from_block_data_heading() {
        let blocks = vec![BlockData {
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
        let doc = EditorDocument::from_block_data(&blocks);
        assert_eq!(doc.block_type(0), Some("heading".into()));
    }
}
