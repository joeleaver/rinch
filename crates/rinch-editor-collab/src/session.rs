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
//!   the next broadcast delta (the updates of the local transactions since the last
//!   call), this replica's state vector, or the diff a peer at a given state vector is
//!   missing.
//! * **`integrate_incremental`** — apply a peer's bytes, whatever transport produced
//!   them: merge into the CRDT, rebuild the model from the *converged* CRDT, and return
//!   the next [`EditorState`] (the change applied as a remote, non-undoable transaction).
//!
//! Because both peers rebuild from the same converged CRDT, their models converge. The
//! session never consults the host DOM — it is pure model ↔ CRDT.
//!
//! ## Poisoning (issue #196)
//!
//! yrs has no rollback: once inbound bytes are applied, they cannot be un-applied. If
//! an integrate leaves the shared document **unprojectable** — foreign bytes whose
//! `content` root was built as the wrong yrs type, or a peer delta that parked
//! out-of-scope content (an embed) inside a block — every future rebuild fails the same
//! way, so the session can never receive again. Detecting that costs nothing extra:
//! the converged rebuild ([`CollabDoc::to_doc`]) already runs on every integrate, and
//! its failure *after* bytes touched the CRDT **is** the diagnosis. The session then
//! turns **sticky-loud in both directions**
//! ([`CollabError::SessionPoisoned`](crate::CollabError::SessionPoisoned)): inbound
//! kept failing anyway, and outbound (`record_local`/`save_incremental`/`sync_diff`)
//! now refuses too — a replica that can never converge must not keep broadcasting,
//! and a poisoned document's diff must not be handed to healthy peers. A *decode*
//! failure, by contrast, leaves the CRDT untouched and stays transient. Recovery is a
//! fresh session from a healthy peer's snapshot.

use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{StateVector, Update};

use rinch_editor_core::{EditorState, Node, Schema};

use crate::error::{CollabError, Result};
use crate::projection::CollabDoc;
use crate::remote::build_remote_transaction;

/// An update that carries nothing, in the lib0 v1 encoding: zero blocks followed by a
/// zero-length delete set. A peer can legitimately send one (a reconciliation diff for a
/// peer that is already up to date), so the inbound path recognises it instead of
/// decoding it into a no-op transaction.
const EMPTY_UPDATE: &[u8] = &[0, 0];

/// One editor's collaboration session: its CRDT projection plus the lifecycle to drive
/// it.
#[derive(Debug)]
pub struct CollabSession {
    cdoc: CollabDoc,
    /// The sticky poison error, set the moment an integrate leaves the CRDT
    /// unprojectable (see the module docs). `None` for a healthy session.
    poisoned: Option<CollabError>,
}

impl CollabSession {
    /// Start a session from a fresh editor state, projecting its document onto a new
    /// CRDT. Fails loud on content outside the staged scope (design A22).
    pub fn new(state: &EditorState) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::from_doc(&state.doc)?,
            poisoned: None,
        })
    }

    /// Join an existing collaboration from a peer's saved CRDT bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::load(bytes)?,
            poisoned: None,
        })
    }

    /// Whether this session is **poisoned** — an integrate left the shared CRDT
    /// permanently unprojectable, so every convergence-relevant operation now fails
    /// with the sticky [`CollabError::SessionPoisoned`] (see the module docs). The
    /// only recovery is a fresh session from a healthy peer's snapshot.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.is_some()
    }

    /// Fail with the sticky poison error if this session is poisoned.
    fn guard(&self) -> Result<()> {
        match &self.poisoned {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    /// Mark this session poisoned by `cause` and return the sticky error every
    /// subsequent operation will keep failing with.
    fn poison(&mut self, cause: &CollabError) -> CollabError {
        let err = CollabError::SessionPoisoned(cause.to_string());
        self.poisoned = Some(err.clone());
        err
    }

    /// Save the whole CRDT (hand to a peer so they can `from_bytes` and join).
    pub fn snapshot(&self) -> Vec<u8> {
        self.cdoc.save()
    }

    /// Project a just-applied local change (`before` = old `state.doc`, `after` = new
    /// `state.doc`) onto the CRDT. Fails loud on out-of-scope content.
    ///
    /// An ordinary failure here is **not** sticky: the CRDT is untouched (the
    /// projection is all-or-nothing, issue #194) and the offending *local* content can
    /// be edited away (undo the table paste), after which projection resumes. On a
    /// [poisoned](Self::is_poisoned) session, though, this refuses with the sticky
    /// error before reading anything: a replica that can never receive must not keep
    /// projecting-and-broadcasting as though it were converging (issue #196).
    pub fn record_local(&mut self, before: &Node, after: &Node) -> Result<()> {
        self.guard()?;
        self.cdoc.project_change(before, after)
    }

    /// The next broadcast delta: the updates of every locally-projected transaction since
    /// the previous call, as one update. **Empty** when nothing has been projected since,
    /// so a caller can skip the transmission.
    ///
    /// This drains an outbox fed by an update observer rather than diffing against the
    /// state vector of the last broadcast. The difference is not cosmetic:
    /// `encode_diff_v1` writes the *complete* delete set whatever state vector it is
    /// given, so a diff-based delta re-carries the document's entire deletion history on
    /// every keystroke — growing without bound — and a "nothing new" diff never becomes
    /// empty once anything has been deleted. A transaction's own update is constant-size
    /// and genuinely empty when there was no transaction.
    ///
    /// The **first** delta of a session started with [`Self::new`] also carries the
    /// initial projection of the document, by design: the outbox is armed before that
    /// projection is written, so no content can ever escape the broadcast stream. A peer
    /// that joined from [`Self::snapshot`] already has it and applies that part as a
    /// no-op, since updates are idempotent. Draining the outbox in `snapshot` instead
    /// would trade this one redundant delta for the risk of an *earlier* joiner missing a
    /// real one.
    ///
    /// Fails only if a parked update cannot be re-decoded to be merged with its
    /// neighbours, which would mean this replica produced a malformed update; the parked
    /// updates are left in place, so the error is not a silent loss — or, sticky, on a
    /// [poisoned](Self::is_poisoned) session, so no delta leaves a replica that can
    /// never converge (issue #196).
    pub fn save_incremental(&mut self) -> Result<Vec<u8>> {
        self.guard()?;
        self.cdoc.take_outbox()
    }

    /// This replica's state vector, encoded for the wire — the insertions it has seen.
    /// Send it to a peer, which answers with [`Self::sync_diff`].
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
    ///
    /// Sticky-refused on a [poisoned](Self::is_poisoned) session: this diff carries
    /// document content, and a poisoned document's content must not be handed to a
    /// healthy peer (issue #196).
    pub fn sync_diff(&self, remote_state_vector: &[u8]) -> Result<Vec<u8>> {
        self.guard()?;
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
    ///
    /// Both spellings of "nothing arrived" are a no-op: the two-byte empty update, which is what a
    /// reconciliation diff for an up-to-date peer encodes as, and an empty slice, which is
    /// what [`Self::save_incremental`] returns when it has nothing to send. Neither is
    /// decodable as an update, so recognising them here is what keeps a caller that
    /// forwards its own empty delta from parking a spurious decode error.
    ///
    /// # Failure semantics (issue #196)
    ///
    /// * Bytes that don't **decode** never touch the CRDT — a transient
    ///   [`Engine`](CollabError::Engine) error; the next valid delta integrates fine.
    /// * Bytes that were **applied** but leave the document failing its converged
    ///   rebuild ([`CollabDoc::to_doc`]) can never be un-applied (yrs has no rollback),
    ///   so the session **poisons itself**: this call — and every subsequent
    ///   convergence-relevant call, outbound included — fails with the sticky
    ///   [`SessionPoisoned`](CollabError::SessionPoisoned). The read that decides this
    ///   is the rebuild every integrate already performs, so a healthy delta pays
    ///   nothing extra.
    pub fn integrate_incremental(
        &mut self,
        state: &EditorState,
        changes: &[u8],
    ) -> Result<Option<EditorState>> {
        self.guard()?;
        if changes.is_empty() || changes == EMPTY_UPDATE {
            return Ok(None);
        }
        // Decode failure: the CRDT is untouched, so the error is transient by
        // construction — never poison here.
        let update = Update::decode_v1(changes)?;
        if let Err(apply_err) = self.cdoc.apply_decoded(update) {
            // yrs commits on drop with no rollback, so a failed apply may have
            // partially integrated. Projectability decides transient vs permanent:
            // if the document still rebuilds, the session is still convergent.
            return match self.cdoc.to_doc(state.schema()) {
                Ok(_) => Err(apply_err),
                Err(_) => Err(self.poison(&apply_err)),
            };
        }
        // The converged rebuild. Its failure after a successful apply is the
        // permanently-unprojectable state: these bytes are in the CRDT for good, and
        // every future rebuild hits the same content.
        let target = match self.cdoc.to_doc(state.schema()) {
            Ok(target) => target,
            Err(cause) => return Err(self.poison(&cause)),
        };
        // Past this point the document is projectable, so a failure is not the
        // permanent class: building/applying the model transaction reads only the
        // validated rebuild (and is unreachable in practice for a doc `to_doc`
        // admitted). Defense in depth, not a supported failure path.
        match build_remote_transaction(state, &target)? {
            Some(tr) => Ok(Some(state.apply(tr))),
            None => Ok(None),
        }
    }

    /// Merge another peer's CRDT wholesale into this one (test/utility convenience).
    /// Refused — sticky — when either side is [poisoned](Self::is_poisoned), for the
    /// same reason as [`Self::sync_diff`]: poison must not spread.
    pub fn merge(&mut self, other: &CollabSession) -> Result<()> {
        self.guard()?;
        other.guard()?;
        self.cdoc.merge_from(&other.cdoc)
    }

    /// The model document the CRDT currently projects to — the canonical converged
    /// read-back (both peers reconstruct identically). The rock-solid convergence
    /// assertion in tests. On a [poisoned](Self::is_poisoned) session this returns the
    /// sticky error rather than re-deriving the underlying read failure.
    pub fn projected_doc(&self, schema: &Schema) -> Result<Node> {
        self.guard()?;
        self.cdoc.to_doc(schema)
    }
}
