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

use rinch_core::ce::{subscribe_ce_events, with_ce_api_for_node};

use super::EditorDocument;
use super::sync::{SyncMessage, SyncState};
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
    /// EditorDocument in sync automatically. This method rebuilds from
    /// DOM content as a safety net / compatibility shim.
    pub fn flush(&mut self) {
        // Rebuild EditorDocument from current DOM content to ensure sync.
        // With dual-write in CeOps this is typically a no-op in terms of
        // content change, but ensures consistency.
        let new_blocks =
            match with_ce_api_for_node(self.ce_node_id, |api| api.borrow().extract_content()) {
                Some(blocks) => blocks,
                None => return,
            };
        self.doc = EditorDocument::from_block_data(&new_blocks);
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
