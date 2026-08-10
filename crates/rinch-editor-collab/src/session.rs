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
//! * **`save_incremental` / `state_vector` / `sync_diff`** — produce something to send:
//!   the next broadcast delta, this replica's state vector, or the diff a peer at a
//!   given state vector is missing.
//! * **`integrate_incremental`** — apply a peer's bytes, whatever transport produced
//!   them: merge into the CRDT, rebuild the model from the *converged* CRDT, and return
//!   the next [`EditorState`] (the change applied as a remote, non-undoable transaction).
//!
//! Because both peers rebuild from the same converged CRDT, their models converge. The
//! session never consults the host DOM — it is pure model ↔ CRDT.

use yrs::StateVector;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;

use rinch_editor_core::{EditorState, Node, Schema};

use crate::error::Result;
use crate::projection::CollabDoc;
use crate::remote::build_remote_transaction;

/// An update that carries nothing, in the lib0 v1 encoding: zero blocks followed by a
/// zero-length delete set.
///
/// Worth naming, because it is the **only** sound "nothing happened" test on this
/// wire. A state vector counts the *inserts* a replica has seen and says nothing about
/// deletions, so two docs can differ by an arbitrary number of deletes (and, since yrs
/// implements un-formatting by deleting format markers, by arbitrary *mark removals*)
/// while their state vectors are identical. Comparing state vectors to decide whether
/// anything changed silently drops exactly those changes.
const EMPTY_UPDATE: &[u8] = &[0, 0];

/// One editor's collaboration session: its CRDT projection plus the lifecycle to drive
/// it.
#[derive(Debug)]
pub struct CollabSession {
    cdoc: CollabDoc,
    /// The state vector as of the last [`Self::save_incremental`] — the watermark that
    /// bounds how far back a broadcast delta has to reach for *inserts*. Deletions ride
    /// along regardless: yrs writes the whole delete set into every diff, so a deletion
    /// this watermark cannot represent still reaches every peer (and keeps reaching
    /// them, which is why a missed delete self-heals).
    ///
    /// Deliberately *not* advanced by [`Self::snapshot`] or
    /// [`Self::integrate_incremental`]: a snapshot handed to a late joiner must not
    /// make the existing peers miss the next delta, and re-broadcasting a change that
    /// arrived from a peer is harmless (updates are idempotent) where dropping one is
    /// not.
    last_saved: StateVector,
}

impl CollabSession {
    /// Start a session from a fresh editor state, projecting its document onto a new
    /// CRDT. Fails loud on content outside the staged scope (design A22).
    pub fn new(state: &EditorState) -> Result<CollabSession> {
        let cdoc = CollabDoc::from_doc(&state.doc)?;
        let last_saved = cdoc.state_vector();
        Ok(CollabSession { cdoc, last_saved })
    }

    /// Join an existing collaboration from a peer's saved CRDT bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<CollabSession> {
        let cdoc = CollabDoc::load(bytes)?;
        let last_saved = cdoc.state_vector();
        Ok(CollabSession { cdoc, last_saved })
    }

    /// Save the whole CRDT (hand to a peer so they can `from_bytes` and join).
    pub fn snapshot(&self) -> Vec<u8> {
        self.cdoc.save()
    }

    /// Project a just-applied local change (`before` = old `state.doc`, `after` = new
    /// `state.doc`) onto the CRDT. Fails loud on out-of-scope content.
    pub fn record_local(&mut self, before: &Node, after: &Node) -> Result<()> {
        self.cdoc.project_change(before, after)
    }

    /// The next broadcast delta: everything produced since the previous call. Empty when
    /// there is genuinely nothing to send, so a caller can skip the transmission.
    ///
    /// "Nothing to send" is decided from the *encoded delta* ([`EMPTY_UPDATE`]), never
    /// from a state-vector comparison — a local deletion or mark removal does not move
    /// the state vector, and skipping on that basis would drop the change permanently.
    pub fn save_incremental(&mut self) -> Vec<u8> {
        let delta = self.cdoc.diff_since(&self.last_saved);
        self.last_saved = self.cdoc.state_vector();
        if delta == EMPTY_UPDATE {
            return Vec::new();
        }
        delta
    }

    /// This replica's state vector, encoded for the wire — the inserts it has seen. Send
    /// it to a peer, which answers with [`Self::sync_diff`].
    ///
    /// **Not a convergence test.** Equal state vectors mean two replicas have seen the
    /// same insertions, not that they hold the same document: deletions (and mark
    /// removals, which yrs implements by deleting format markers) are absent from a
    /// state vector. That is exactly why the reply to it — a diff — always carries the
    /// complete delete set, and why reconciliation must apply that reply rather than
    /// short-circuit on state-vector equality.
    pub fn state_vector(&self) -> Vec<u8> {
        self.cdoc.state_vector().encode_v1()
    }

    /// The update a peer whose state vector is `remote_state_vector` is missing. Feed
    /// the result to that peer's [`Self::integrate_incremental`]; one exchange in each
    /// direction reconciles two replicas that have drifted apart.
    pub fn sync_diff(&self, remote_state_vector: &[u8]) -> Result<Vec<u8>> {
        let sv = StateVector::decode_v1(remote_state_vector)?;
        Ok(self.cdoc.diff_since(&sv))
    }

    /// Integrate a peer's update bytes (a broadcast delta or a reconciliation diff —
    /// same encoding, same path): merge into the CRDT, rebuild the model from the
    /// converged CRDT, and return the next state (or `None` if the document did not
    /// change).
    ///
    /// Whether anything changed is decided by rebuilding and *comparing documents*
    /// ([`build_remote_transaction`] returns `None` for an identical rebuild), not by
    /// comparing state vectors before and after: a delta that only deletes content — or
    /// only removes a mark — leaves the state vector untouched while changing the
    /// document, so a state-vector guard here would apply the change to the CRDT and
    /// then never tell the model about it, breaking `model ≡ project(model)` in a way
    /// that only shows up rounds later as divergence.
    pub fn integrate_incremental(
        &mut self,
        state: &EditorState,
        changes: &[u8],
    ) -> Result<Option<EditorState>> {
        if changes == EMPTY_UPDATE {
            return Ok(None);
        }
        self.cdoc.apply_update(changes)?;
        self.apply_converged(state)
    }

    /// Merge another peer's CRDT wholesale into this one (test/utility convenience).
    pub fn merge(&mut self, other: &CollabSession) -> Result<()> {
        self.cdoc.merge_from(&other.cdoc)
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
