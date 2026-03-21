//! Sync protocol support for collaborative editing.
//!
//! This module exposes automerge's built-in sync protocol through
//! [`EditorDocument`] methods, enabling real-time collaboration between peers
//! without exposing the raw `AutoCommit`.
//!
//! # Usage
//!
//! ```rust
//! use rinch_editor::prelude::*;
//! use rinch_editor::sync::{SyncState, SyncMessage};
//!
//! // Create a document and edit it
//! let mut peer1 = EditorDocument::new();
//! peer1.insert_text(Position(0), "Hello from peer1").unwrap();
//!
//! // Fork to create a second peer (shared origin)
//! let mut peer2 = EditorDocument::from_bytes(&peer1.to_bytes()).unwrap();
//!
//! // Each peer maintains a SyncState for the other
//! let mut state1 = SyncState::new();
//! let mut state2 = SyncState::new();
//!
//! // peer1 makes another edit
//! peer1.insert_text(Position(peer1.text_length()), " - updated").unwrap();
//!
//! // Sync loop: exchange messages until both converge
//! for _ in 0..100 {
//!     let msg1 = peer1.generate_sync_message(&mut state1);
//!     let msg2 = peer2.generate_sync_message(&mut state2);
//!     let done = msg1.is_none() && msg2.is_none();
//!     if let Some(msg) = msg1 {
//!         peer2.receive_sync_message(&mut state2, msg).unwrap();
//!     }
//!     if let Some(msg) = msg2 {
//!         peer1.receive_sync_message(&mut state1, msg).unwrap();
//!     }
//!     if done { break; }
//! }
//!
//! assert_eq!(peer1.to_text(), peer2.to_text());
//! ```

pub use automerge::ChangeHash;
pub use automerge::sync::Message as SyncMessage;
pub use automerge::sync::State as SyncState;

use super::EditorDocument;

impl EditorDocument {
    /// Generate a sync message to send to a peer.
    ///
    /// Returns `None` if there is nothing new to send (either because we are
    /// waiting for an acknowledgement of an in-flight message, or because the
    /// remote peer is already up to date).
    pub fn generate_sync_message(&mut self, sync_state: &mut SyncState) -> Option<SyncMessage> {
        use automerge::sync::SyncDoc;
        self.doc.sync().generate_sync_message(sync_state)
    }

    /// Receive and apply a sync message from a peer.
    pub fn receive_sync_message(
        &mut self,
        sync_state: &mut SyncState,
        message: SyncMessage,
    ) -> Result<(), crate::error::EditorError> {
        use automerge::sync::SyncDoc;
        self.doc
            .sync()
            .receive_sync_message(sync_state, message)
            .map_err(|e| crate::error::EditorError::Automerge(e.to_string()))
    }

    /// Get the current heads of this document (for change tracking).
    pub fn get_heads(&mut self) -> Vec<ChangeHash> {
        self.doc.get_heads()
    }

    /// Merge all changes from another document into this one.
    pub fn merge(&mut self, other: &mut EditorDocument) -> Result<(), crate::error::EditorError> {
        self.doc
            .merge(&mut other.doc)
            .map(|_| ())
            .map_err(|e| crate::error::EditorError::Automerge(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Position;

    /// Create a second peer from the same document (shared origin).
    /// This is the realistic collaboration pattern — both peers start
    /// from the same document state.
    fn fork(doc: &mut EditorDocument) -> EditorDocument {
        EditorDocument::from_bytes(&doc.to_bytes()).unwrap()
    }

    /// Helper: run the sync protocol between two peers until convergence.
    fn sync_peers(
        peer1: &mut EditorDocument,
        state1: &mut SyncState,
        peer2: &mut EditorDocument,
        state2: &mut SyncState,
    ) {
        for _ in 0..100 {
            let msg1 = peer1.generate_sync_message(state1);
            let msg2 = peer2.generate_sync_message(state2);
            let done = msg1.is_none() && msg2.is_none();
            if let Some(msg) = msg1 {
                peer2.receive_sync_message(state2, msg).unwrap();
            }
            if let Some(msg) = msg2 {
                peer1.receive_sync_message(state1, msg).unwrap();
            }
            if done {
                break;
            }
        }
    }

    #[test]
    fn two_peer_sync_converges() {
        let mut peer1 = EditorDocument::new();
        peer1.insert_text(Position(0), "Hello").unwrap();

        // Fork from peer1 so both share the same document origin
        let mut peer2 = fork(&mut peer1);
        let mut state1 = SyncState::new();
        let mut state2 = SyncState::new();

        // peer1 makes an additional edit
        peer1
            .insert_text(Position(peer1.text_length()), " World")
            .unwrap();

        sync_peers(&mut peer1, &mut state1, &mut peer2, &mut state2);

        assert_eq!(peer1.to_text(), peer2.to_text());
        assert_eq!(peer2.to_text(), "Hello World");
    }

    #[test]
    fn incremental_sync_sends_only_new_changes() {
        let mut peer1 = EditorDocument::new();
        let mut peer2 = fork(&mut peer1);
        let mut state1 = SyncState::new();
        let mut state2 = SyncState::new();

        // Initial edit + sync
        peer1.insert_text(Position(0), "Hello").unwrap();
        sync_peers(&mut peer1, &mut state1, &mut peer2, &mut state2);
        assert_eq!(peer1.to_text(), peer2.to_text());

        // After full sync, no messages pending
        assert!(
            peer1.generate_sync_message(&mut state1).is_none(),
            "should have nothing to send after full sync"
        );

        // Make a new edit
        peer1
            .insert_text(Position(peer1.text_length()), " World")
            .unwrap();

        // Now there should be a message (new changes since last sync)
        let msg = peer1.generate_sync_message(&mut state1);
        assert!(msg.is_some(), "should have a message after new edit");

        // Complete the sync
        if let Some(msg) = msg {
            peer2.receive_sync_message(&mut state2, msg).unwrap();
        }
        sync_peers(&mut peer1, &mut state1, &mut peer2, &mut state2);

        assert_eq!(peer2.to_text(), peer1.to_text());
    }

    #[test]
    fn merge_combines_two_documents() {
        let mut doc1 = EditorDocument::new();
        doc1.insert_text(Position(0), "Hello").unwrap();

        let mut doc2 = fork(&mut doc1);
        doc2.insert_text(Position(doc2.text_length()), " World")
            .unwrap();

        doc1.merge(&mut doc2).unwrap();

        assert_eq!(doc1.to_text(), "Hello World");
    }

    #[test]
    fn get_heads_changes_after_edit() {
        let mut doc = EditorDocument::new();
        let heads_before = doc.get_heads();

        doc.insert_text(Position(0), "text").unwrap();
        let heads_after = doc.get_heads();

        assert_ne!(heads_before, heads_after);
    }
}
