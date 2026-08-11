//! The yrs bytes transport, re-homed onto [`CollabDoc`].
//!
//! Everything a peer exchanges is one opaque `Vec<u8>` in the lib0 v1 encoding, and
//! there is only **one** way to apply an inbound blob — [`CollabDoc::apply_update`].
//! That is what collapses automerge's two separate transports (an incremental
//! broadcast and a stateful sync protocol) into a single path:
//!
//! * **Broadcast** — a locally-projected transaction's own update is parked in the
//!   outbox by the observer set up in [`CollabDoc`], and
//!   [`CollabSession::save_incremental`](crate::CollabSession::save_incremental) drains
//!   it. Constant-size per edit, and empty when there is nothing to send.
//! * **Reconciliation** (a reconnect, an HTTP poll, a peer that was offline) — a peer
//!   sends its [`state vector`](CollabDoc::state_vector), the other side answers with
//!   [`diff_since`](CollabDoc::diff_since) that vector, and the answer is applied through
//!   the *same* entry point. No per-peer protocol state is kept: the state vector is
//!   recomputed from the document, which is exactly what a stateless server wants.
//!
//! A state vector summarises *insertions* only, so it can say what a peer is missing but
//! never that two peers agree — deletions are invisible to it. That asymmetry is safe
//! because the *reply* to a state vector carries the whole delete set, so requesting a
//! diff and applying it converges regardless; short-circuiting on state-vector equality
//! does not.
//!
//! Updates are **idempotent** and order-insensitive, so re-sending an overlapping range
//! is harmless — losing one is not.
//!
//! Both leave the [`CollabDoc`] converged; turning the converged CRDT back into model
//! [`Step`](rinch_editor_core::Step)s is [`crate::remote`]'s job.

use yrs::updates::decoder::Decode;
use yrs::{Origin, ReadTxn, StateVector, Transact, Update, merge_updates_v1};

use crate::error::Result;
use crate::projection::{CollabDoc, ENGINE_APPLY_ORIGIN, lock_outbox};

impl CollabDoc {
    /// The CRDT's current state vector — the insertions this replica has seen. What to
    /// send a peer so it can answer with [`Self::diff_since`]; **not** a convergence test
    /// (see the module docs).
    pub fn state_vector(&self) -> StateVector {
        self.doc.transact().state_vector()
    }

    /// Everything this replica has that a peer at `sv` does not — the reconciliation
    /// answer. Carries the complete delete set, so it repairs a missed deletion even
    /// though `sv` could not describe one.
    pub fn diff_since(&self, sv: &StateVector) -> Vec<u8> {
        self.doc.transact().encode_diff_v1(sv)
    }

    /// Apply a peer's update bytes, merging them into this CRDT. The single inbound entry
    /// point: a broadcast delta, a reconciliation diff and a whole-document snapshot are
    /// all the same encoding.
    ///
    /// The transaction is tagged `ENGINE_APPLY_ORIGIN` so the update observer leaves it out of
    /// the broadcast outbox — these bytes are already shared, and re-broadcasting them
    /// would echo. Forwarding them to *other* peers is the transport's job.
    pub fn apply_update(&mut self, bytes: &[u8]) -> Result<()> {
        let update = Update::decode_v1(bytes)?;
        self.apply_decoded(update)
    }

    /// Apply an already-decoded update. Split from [`Self::apply_update`] for the
    /// session's integrate path, which must tell a *decode* failure (the CRDT is
    /// untouched — transient) from an *apply* failure (the update may be partially
    /// integrated — yrs commits on drop and has no rollback), because only the latter
    /// can leave the document unprojectable (issue #196).
    pub(crate) fn apply_decoded(&mut self, update: Update) -> Result<()> {
        let mut txn = self
            .doc
            .transact_mut_with(Origin::from(ENGINE_APPLY_ORIGIN));
        txn.apply_update(update)?;
        Ok(())
    }

    /// Take everything the outbox has collected since the last drain, as one update.
    ///
    /// Empty when no locally-projected transaction has run since — unlike a
    /// state-vector diff, which stays non-empty forever once anything has been deleted.
    /// Several parked updates are combined with [`merge_updates_v1`] rather than
    /// concatenated: concatenated updates are not a valid single update.
    pub(crate) fn take_outbox(&mut self) -> Result<Vec<u8>> {
        let mut parked = lock_outbox(&self.outbox);
        match parked.len() {
            0 => Ok(Vec::new()),
            1 => Ok(parked.drain(..).next().unwrap_or_default()),
            _ => {
                let merged = merge_updates_v1(parked.iter())?;
                parked.clear();
                Ok(merged)
            }
        }
    }

    /// Merge another peer's whole document into this one.
    pub fn merge_from(&mut self, other: &CollabDoc) -> Result<()> {
        let missing = other.diff_since(&self.state_vector());
        self.apply_update(&missing)
    }
}
