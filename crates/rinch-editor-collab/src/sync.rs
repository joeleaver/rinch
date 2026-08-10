//! The yrs bytes transport, re-homed onto [`CollabDoc`].
//!
//! Everything a peer exchanges is one opaque `Vec<u8>` in the lib0 v1 encoding, and
//! there is only **one** way to apply an inbound blob — [`CollabDoc::apply_update`].
//! That is what collapses automerge's two separate transports (an incremental
//! broadcast and a stateful sync protocol) into a single path:
//!
//! * **Broadcast** — [`CollabSession::save_incremental`](crate::CollabSession::save_incremental)
//!   encodes everything produced since the last broadcast
//!   ([`CollabDoc::diff_since`]) and every peer applies it.
//! * **Reconciliation** (a reconnect, an HTTP poll, a peer that was offline) — a peer
//!   sends its [`state vector`](CollabDoc::state_vector), the other side answers with
//!   `diff_since` that vector, and it is applied through the *same* entry point. No
//!   per-peer protocol state is kept: the state vector is recomputed from the document,
//!   which is exactly what a stateless server wants.
//!
//! Updates are **idempotent** and order-insensitive, so re-sending an overlapping range
//! is harmless — losing one is not, which is why reconciliation compares state vectors
//! rather than trusting delivery.
//!
//! Both leave the [`CollabDoc`] converged; turning the converged CRDT back into model
//! [`Step`](rinch_editor_core::Step)s is [`crate::remote`]'s job.

use yrs::updates::decoder::Decode;
use yrs::{ReadTxn, StateVector, Transact, Update};

use crate::error::Result;
use crate::projection::CollabDoc;

impl CollabDoc {
    /// The CRDT's current state vector — "everything this replica has seen". Comparing
    /// two of these is the convergence check (automerge's head frontier).
    pub fn state_vector(&self) -> StateVector {
        self.doc.transact().state_vector()
    }

    /// Everything this replica has that a peer at `sv` does not — the delta to send,
    /// whether that is a live broadcast or an offline reconciliation.
    pub fn diff_since(&self, sv: &StateVector) -> Vec<u8> {
        self.doc.transact().encode_diff_v1(sv)
    }

    /// Apply a peer's update bytes, merging them into this CRDT. The single inbound
    /// entry point: a broadcast delta, a reconciliation diff and a whole-document
    /// snapshot are all the same encoding.
    pub fn apply_update(&mut self, bytes: &[u8]) -> Result<()> {
        let update = Update::decode_v1(bytes)?;
        self.doc.transact_mut().apply_update(update)?;
        Ok(())
    }

    /// Merge another peer's whole document into this one.
    pub fn merge_from(&mut self, other: &CollabDoc) -> Result<()> {
        let missing = other.diff_since(&self.state_vector());
        self.apply_update(&missing)
    }
}
