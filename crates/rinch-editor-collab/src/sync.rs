//! The Automerge sync transport, re-homed onto [`CollabDoc`].
//!
//! Salvaged from the old `rinch-editor` `sync.rs` (design §8: "the Automerge transport
//! is salvaged"). Two transports are exposed:
//!
//! * **Full sync protocol** — [`generate_sync_message`](CollabDoc::generate_sync_message)
//!   / [`receive_sync_message`](CollabDoc::receive_sync_message), with per-peer
//!   [`SyncState`]. Deduplicating, state-negotiating; the right default for a live link.
//! * **Incremental broadcast** — [`save_incremental`](CollabDoc::save_incremental) /
//!   [`load_incremental`](CollabDoc::load_incremental). Simpler fan-out: save the delta
//!   after a local edit, send to all peers, each applies it. No per-peer state.
//!
//! Both leave the [`CollabDoc`] converged; turning the converged CRDT back into model
//! [`Step`](rinch_editor_core::Step)s is [`crate::remote`]'s job.

pub use automerge::ChangeHash;
pub use automerge::sync::Message as SyncMessage;
pub use automerge::sync::State as SyncState;

use automerge::Patch;

use crate::error::Result;
use crate::projection::CollabDoc;

impl CollabDoc {
    /// Generate a sync message for a peer, or `None` if nothing new is pending.
    pub fn generate_sync_message(&mut self, sync_state: &mut SyncState) -> Option<SyncMessage> {
        use automerge::sync::SyncDoc;
        self.doc.sync().generate_sync_message(sync_state)
    }

    /// Receive a peer's sync message, merging its changes into this CRDT.
    pub fn receive_sync_message(
        &mut self,
        sync_state: &mut SyncState,
        message: SyncMessage,
    ) -> Result<()> {
        use automerge::sync::SyncDoc;
        self.doc.sync().receive_sync_message(sync_state, message)?;
        Ok(())
    }

    /// The current heads (change frontier) of the CRDT.
    pub fn get_heads(&mut self) -> Vec<ChangeHash> {
        self.doc.get_heads()
    }

    /// Save only the changes since the last save (the broadcast primitive).
    pub fn save_incremental(&mut self) -> Vec<u8> {
        self.doc.save_incremental()
    }

    /// Apply a peer's broadcast delta, merging it into this CRDT.
    pub fn load_incremental(&mut self, changes: &[u8]) -> Result<()> {
        self.doc.load_incremental(changes)?;
        Ok(())
    }

    /// Merge another peer's whole document into this one.
    pub fn merge(&mut self, other: &mut CollabDoc) -> Result<()> {
        self.doc.merge(&mut other.doc)?;
        Ok(())
    }

    /// The diff (Automerge patches) between two head frontiers — the basis for
    /// translating a remote change back into model steps ([`crate::remote`]).
    pub(crate) fn diff(&mut self, before: &[ChangeHash], after: &[ChangeHash]) -> Vec<Patch> {
        self.doc.diff(before, after)
    }
}
