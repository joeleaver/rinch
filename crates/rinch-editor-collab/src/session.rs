//! [`CollabSession`] — the imperative adapter that ties one editor's model to its CRDT.
//!
//! A session owns the [`CollabDoc`] for one editor and exposes the collab lifecycle:
//!
//! * **`new` / `from_bytes`** — start a session from a fresh model, or join a peer's
//!   saved CRDT.
//! * **`record_local(before, after)`** — after the editor applies a local transaction,
//!   project the `before → after` change onto the CRDT (immediate projection keeps the
//!   `model ≡ project(model)` invariant and means there is never an unconfirmed backlog
//!   to rebase).
//! * **`save_incremental` / `generate_sync_message`** — produce something to send.
//! * **`integrate_incremental` / `integrate_sync_message`** — apply a peer's change:
//!   merge it into the CRDT, rebuild the model from the *converged* CRDT, and return the
//!   next [`EditorState`] (the change applied as a remote, non-undoable transaction).
//!
//! Because both peers rebuild from the same converged CRDT, their models converge. The
//! session never consults the host DOM — it is pure model ↔ CRDT.

use rinch_editor_core::{EditorState, Node, Schema};

use crate::error::Result;
use crate::projection::CollabDoc;
use crate::remote::build_remote_transaction;
use crate::sync::{ChangeHash, SyncMessage, SyncState};

/// One editor's collaboration session: its CRDT projection plus the lifecycle to drive
/// it.
#[derive(Debug)]
pub struct CollabSession {
    cdoc: CollabDoc,
}

impl CollabSession {
    /// Start a session from a fresh editor state, projecting its document onto a new
    /// CRDT. Fails loud on any non-flat content (design A22).
    pub fn new(state: &EditorState) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::from_doc(&state.doc)?,
        })
    }

    /// Join an existing collaboration from a peer's saved CRDT bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::load(bytes)?,
        })
    }

    /// Save the whole CRDT (hand to a peer so they can `from_bytes` and join).
    pub fn snapshot(&mut self) -> Vec<u8> {
        self.cdoc.save()
    }

    /// Project a just-applied local change (`before` = old `state.doc`, `after` = new
    /// `state.doc`) onto the CRDT. Fails loud on non-flat content.
    pub fn record_local(&mut self, before: &Node, after: &Node) -> Result<()> {
        self.cdoc.project_change(before, after)
    }

    /// The CRDT's current head frontier.
    pub fn heads(&mut self) -> Vec<ChangeHash> {
        self.cdoc.get_heads()
    }

    /// Save only the changes since the last save (broadcast delta).
    pub fn save_incremental(&mut self) -> Vec<u8> {
        self.cdoc.save_incremental()
    }

    /// Generate a sync message for a peer (full sync protocol).
    pub fn generate_sync_message(&mut self, sync_state: &mut SyncState) -> Option<SyncMessage> {
        self.cdoc.generate_sync_message(sync_state)
    }

    /// Integrate a peer's broadcast delta: merge into the CRDT, rebuild the model from
    /// the converged CRDT, and return the next state (or `None` if nothing changed).
    pub fn integrate_incremental(
        &mut self,
        state: &EditorState,
        changes: &[u8],
    ) -> Result<Option<EditorState>> {
        let before = self.cdoc.get_heads();
        self.cdoc.load_incremental(changes)?;
        let after = self.cdoc.get_heads();
        if before == after {
            return Ok(None);
        }
        self.apply_converged(state)
    }

    /// Integrate a peer's sync message (full sync protocol).
    pub fn integrate_sync_message(
        &mut self,
        state: &EditorState,
        sync_state: &mut SyncState,
        message: SyncMessage,
    ) -> Result<Option<EditorState>> {
        let before = self.cdoc.get_heads();
        self.cdoc.receive_sync_message(sync_state, message)?;
        let after = self.cdoc.get_heads();
        if before == after {
            return Ok(None);
        }
        self.apply_converged(state)
    }

    /// Merge another peer's CRDT wholesale into this one (test/utility convenience).
    pub fn merge(&mut self, other: &mut CollabSession) -> Result<()> {
        self.cdoc.merge(&mut other.cdoc)
    }

    /// The high-level [`RemoteOp`](crate::RemoteOp)s describing how the CRDT changed
    /// since the `before` heads (the surgical, cursor-preserving view of a remote
    /// change). The session itself integrates via converged rebuild; this exposes the
    /// op translation for callers that want to drive a finer-grained update.
    pub fn remote_ops_since(&mut self, before: &[ChangeHash]) -> Result<Vec<crate::RemoteOp>> {
        self.cdoc.remote_ops_since(before)
    }

    /// The model document the CRDT currently projects to — the canonical converged
    /// read-back (both peers reconstruct identically). The rock-solid convergence
    /// assertion in tests.
    pub fn projected_doc(&self, schema: &Schema) -> Result<Node> {
        self.cdoc.to_doc(schema)
    }

    /// Rebuild the model from the converged CRDT and apply it as a remote transaction.
    fn apply_converged(&mut self, state: &EditorState) -> Result<Option<EditorState>> {
        let target = self.cdoc.to_doc(state.schema())?;
        match build_remote_transaction(state, &target)? {
            Some(tr) => Ok(Some(state.apply(tr))),
            None => Ok(None),
        }
    }
}
