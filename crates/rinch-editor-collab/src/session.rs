//! [`CollabSession`] — the imperative adapter that ties one editor's model to its CRDT.
//!
//! A session owns the [`CollabDoc`] for one editor and exposes the collab lifecycle:
//!
//! * **`new` / `from_bytes`** — start a session from a fresh model, or join a peer's
//!   saved CRDT.
//! * **`record_local(schema, before, after)`** — after the editor applies a local
//!   transaction, project the `before → after` change onto the CRDT (immediate
//!   projection keeps the `model ≡ project(model)` invariant and means there is never an
//!   unconfirmed backlog to rebase). A change the staged scope cannot express is refused
//!   loud, and the next projectable edit re-bases on the CRDT and catches up — see
//!   *Outbound stalls* below.
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
//! ## Outbound stalls (issue #220)
//!
//! A local edit outside the staged A22 scope — pasting a table, wrapping in a
//! `blockquote` — is refused by `record_local` with the CRDT untouched. But the *model*
//! has already applied it, so from that moment the caller's `before` is a false
//! description of the CRDT.
//!
//! Diffing against it was what turned one refused edit into a wedge. Every later call
//! failed the block-count gate — **including the undo that was supposed to be the
//! cure** — until the counts happened to realign by coincidence. And when they did, the
//! diff skipped every block it believed unchanged, so an edit made during the wedge sat
//! in the model and never reached the CRDT: `record_local` returned `Ok` on a document
//! that no longer matched its own projection, with nothing reporting it. Silent
//! divergence.
//!
//! `record_local` therefore treats the caller's `before` as a *hint*. If diffing against
//! it fails, the change is re-projected against [`CollabDoc::to_doc`] — the CRDT's own
//! read-back, which by construction describes what is really there. That heals every
//! accumulated difference in one write, and it makes the failure honest: if the content
//! really is out of scope the error names it (`Unsupported: blockquote`) instead of a
//! block-count symptom.
//!
//! Once stalled, the hint is skipped entirely rather than tried first. The fast diff
//! verifies only the block *count* and then skips every block the transaction did not
//! touch, so an out-of-scope block that keeps its index — a `blockquote` wrapped around
//! a paragraph in place — lets a later edit elsewhere pass, answer `Ok`, and clear the
//! stall while the model and the CRDT still differ. `before` is known to be false from
//! the moment of the first refusal; there is nothing left to trust in it.
//!
//! [`CollabSession::outbound_stall`] reports the state in between, so an app can say
//! "not syncing — remove the table" rather than leaving the user to wonder. It is
//! **not** poison: local-outbound-only, the shared CRDT is healthy throughout, inbound
//! keeps working, and it clears itself on the next projectable edit.
//!
//! ## Poisoning (issue #196)
//!
//! yrs has no rollback: once inbound bytes are applied, they cannot be un-applied. If
//! an integrate leaves the shared document **unprojectable** — foreign bytes whose
//! `content` root was built as the wrong yrs type, or a peer delta that parked
//! out-of-scope content (an embed) inside a block — the rebuild keeps failing until
//! some future inbound bytes change the content, so the session cannot receive.
//! Detecting that costs nothing extra: the converged rebuild ([`CollabDoc::to_doc`])
//! already runs on every integrate, and its failure *after* bytes touched the CRDT
//! **is** the diagnosis — with one carve-out. A rebuild failure while yrs holds
//! updates parked on **missing dependencies** ([`CollabDoc::has_pending_updates`]) is
//! *transient*, not poison: a misrouted reconciliation diff can delete a map item
//! whose replacement waits on an insertion this replica has not seen, and the missing
//! delta alone cures the read-back — poisoning there would kill a self-healing state
//! and refuse the very bytes that heal it.
//!
//! The carve-out has a window: while dependencies are pending, even an
//! interior-damage shape classifies transient and outbound stays open — exactly the
//! pre-#196 baseline, no worse — and the latch fires on the first integrate after the
//! pending set drains without curing the read-back.
//!
//! A poisoned session turns **sticky-loud in both directions**
//! ([`CollabError::SessionPoisoned`](crate::CollabError::SessionPoisoned)): inbound
//! kept failing anyway, and outbound (`record_local`/`save_incremental`/`sync_diff`)
//! now refuses too — a replica that cannot converge must not keep broadcasting, and a
//! poisoned document's diff must not be handed to healthy peers. Inbound integration
//! is still *attempted*, though: an update that leaves the document rebuildable again
//! (say, the damaged lineage deleting its own offending content) **clears** the
//! poison; until then every attempt reports the sticky error. A heal re-syncs the
//! model to the converged shared document, discarding any local edits made during the
//! poison window — each was already refused loudly at commit time — the same
//! semantics as stopping and rejoining. A *decode* failure leaves the CRDT untouched
//! and stays transient. Recovery in practice is a fresh session from a healthy peer's
//! snapshot.

#[cfg(feature = "test-util")]
use yrs::ClientID;
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
    /// Why outbound is currently refusing, if it is — see
    /// [`CollabSession::outbound_stall`] (issue #220). Cleared by the next successful
    /// [`CollabSession::record_local`]; unrelated to `poisoned`.
    stalled: Option<CollabError>,
}

impl CollabSession {
    /// Start a session from a fresh editor state, projecting its document onto a new
    /// CRDT. Fails loud on content outside the staged scope (design A22).
    pub fn new(state: &EditorState) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::from_doc(&state.doc)?,
            poisoned: None,
            stalled: None,
        })
    }

    /// Join an existing collaboration from a peer's saved CRDT bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::load(bytes)?,
            poisoned: None,
            stalled: None,
        })
    }

    /// [`CollabSession::new`] with this replica's yrs client id pinned — the
    /// [`crate::testing`] seam's implementation. Never a production path; see that
    /// module for why.
    #[cfg(feature = "test-util")]
    pub(crate) fn new_with_client_id(state: &EditorState, client_id: u64) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::from_doc_with_client_id(&state.doc, Some(ClientID::new(client_id)))?,
            poisoned: None,
            stalled: None,
        })
    }

    /// [`CollabSession::from_bytes`] with this replica's yrs client id pinned — the
    /// [`crate::testing`] seam's implementation. Never a production path.
    #[cfg(feature = "test-util")]
    pub(crate) fn from_bytes_with_client_id(bytes: &[u8], client_id: u64) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::load_with_client_id(bytes, Some(ClientID::new(client_id)))?,
            poisoned: None,
            stalled: None,
        })
    }

    /// Whether this session is **poisoned** — an integrate left the shared CRDT
    /// unprojectable with nothing pending that could cure it, so every
    /// convergence-relevant operation now fails with the sticky
    /// [`CollabError::SessionPoisoned`] (see the module docs). Cleared by an inbound
    /// integration that leaves the document rebuildable again; recovery in practice
    /// is a fresh session from a healthy peer's snapshot.
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

    /// The sticky poison error if this session is poisoned, otherwise `err`
    /// unchanged — inbound failures on a poisoned session keep reporting the poison
    /// rather than a shifting mix of underlying errors. The masked failure's own
    /// message is appended for diagnostics (an app logging the error still sees why
    /// *this* delta failed); the stored poison keeps the original cause, so repeated
    /// failures do not accrete.
    fn sticky_or(&self, err: CollabError) -> CollabError {
        match &self.poisoned {
            Some(CollabError::SessionPoisoned(cause)) => {
                CollabError::SessionPoisoned(format!("{cause}; latest inbound failure: {err}"))
            }
            Some(sticky) => sticky.clone(),
            None => err,
        }
    }

    /// Classify an inbound failure that left the shared document failing its
    /// converged rebuild, and return the error to surface.
    ///
    /// * Already poisoned → the sticky error, unchanged.
    /// * Updates parked on missing dependencies → **transient** (`cause` as-is): the
    ///   missing delta alone can cure the read-back, so this must neither poison nor
    ///   be reported as poison (#196 review, F1 — a misrouted reconciliation diff is
    ///   the reachable shape).
    /// * Otherwise → poison: mark the session and return the sticky error every
    ///   affected operation will keep failing with until an inbound integration
    ///   heals it.
    fn classify_unprojectable(&mut self, cause: CollabError) -> CollabError {
        if let Some(sticky) = &self.poisoned {
            return sticky.clone();
        }
        if self.cdoc.has_pending_updates() {
            return cause;
        }
        let err = CollabError::SessionPoisoned(cause.to_string());
        self.poisoned = Some(err.clone());
        err
    }

    /// Save the whole CRDT (hand to a peer so they can `from_bytes` and join).
    ///
    /// Not poison-guarded (the signature is infallible), so a poisoned session still
    /// yields its bytes. Don't hand them to a joiner: [`CollabDoc::load`]'s content
    /// gate validates *shape*, not schema, so some poisoned states (e.g. a block
    /// `type` rewritten to a name the schema lacks) load fine and only fail loud one
    /// step later, at the joiner's first [`Self::projected_doc`].
    pub fn snapshot(&self) -> Vec<u8> {
        self.cdoc.save()
    }

    /// Project a just-applied local change (`before` = old `state.doc`, `after` = new
    /// `state.doc`) onto the CRDT. Fails loud on out-of-scope content.
    ///
    /// The CRDT is untouched by a failure (the projection is all-or-nothing, issue
    /// #194), and a failure is **not** sticky: edit the offending local content away and
    /// the very next call resumes — carrying everything that accumulated meanwhile. On a
    /// [poisoned](Self::is_poisoned) session, though, this refuses with the sticky error
    /// before reading anything: a replica that cannot receive must not keep
    /// projecting-and-broadcasting as though it were converging (issue #196).
    ///
    /// # Recovering from a refused edit (issue #220)
    ///
    /// `before` is the caller's claim about what the CRDT holds, and after a refusal it
    /// is a **false** claim: the model applied the edit, the CRDT did not. Diffing
    /// against it was what wedged the session — every later call failed the block-count
    /// gate, *including the undo that was supposed to be the cure*, until the counts
    /// happened to realign. And when they did, the diff skipped every block it believed
    /// unchanged, so an edit made during the wedge stayed in the model and never reached
    /// the CRDT: `record_local` returned `Ok` on a document that no longer matched its
    /// projection, with nothing reporting it. Silent divergence, which is the one thing
    /// this crate exists to prevent.
    ///
    /// So the fast diff against `before` is only the *first* attempt. If it fails, the
    /// change is re-projected against the CRDT's own read-back — the authoritative base,
    /// which by construction describes what is really there — and that heals every
    /// accumulated difference at once, not just the blocks this edit touched. If *that*
    /// also fails the change really is out of scope, and the error names the actual
    /// offending content (`Unsupported: blockquote`) rather than the block-count symptom
    /// the old path reported.
    ///
    /// `schema` is a parameter rather than session state because
    /// [`CollabSession`](Self) must stay `Send` and `Rc<Schema>` is not.
    ///
    /// The re-base reads the whole document, so a session left stalled by content the
    /// caller never removes pays that on every edit. That is the intended trade: the
    /// cheap path is unchanged for every session that is actually converging, and a
    /// stalled one is a state the app is being told about via
    /// [`Self::outbound_stall`].
    pub fn record_local(&mut self, schema: &Schema, before: &Node, after: &Node) -> Result<()> {
        self.guard()?;
        let fallback = match self.stalled.clone() {
            // Already stalled: `before` is a *known*-false description of the CRDT, so
            // the fast diff must not be tried at all. It verifies only the block count
            // (`project_change` gate 1) and then skips every block the transaction did
            // not touch — an `Rc`-identity prefix/suffix that is never compared against
            // the CRDT. A stalled session whose out-of-scope block keeps its index and
            // is left alone by the next edit therefore passes the fast path, answers
            // `Ok`, and clears this very flag while the model and the CRDT still differ:
            // `<paragraph>Zone<blockquote>two` against `<paragraph>Zone<paragraph>two`,
            // silent, with `outbound_stall()` reporting healthy. Go straight to the
            // authoritative base instead — which is also what makes the "a stalled
            // session pays the read-back per edit" note below true.
            Some(prev) => prev,
            None => match self.cdoc.project_change(before, after) {
                Ok(()) => return Ok(()),
                Err(e) => e,
            },
        };
        // The caller's `before` did not describe the CRDT. Re-base on what does.
        // `to_doc` failing means the shared document is unprojectable — report it, but
        // do not poison: poison is for *inbound* damage (issue #196), and this path has
        // written nothing.
        let base = match self.cdoc.to_doc(schema) {
            Ok(base) => base,
            Err(_) => {
                self.stalled = Some(fallback.clone());
                return Err(fallback);
            }
        };
        match self.cdoc.project_change(&base, after) {
            Ok(()) => {
                self.stalled = None;
                Ok(())
            }
            Err(e) => {
                self.stalled = Some(e.clone());
                Err(e)
            }
        }
    }

    /// Why this replica's **outbound** is currently refusing, if it is (issue #220).
    ///
    /// `Some` from the moment a local edit cannot be projected — content outside the
    /// staged A22 scope (a table paste, a `blockquote` wrap) — until a later
    /// [`Self::record_local`] succeeds. While it is `Some`, local edits are **not**
    /// reaching peers; inbound integration is unaffected and the shared document is
    /// healthy throughout.
    ///
    /// The cure is to remove the offending content, which the error names. This is
    /// deliberately *not* [poison](Self::is_poisoned): it is local-outbound-only, the
    /// shared CRDT is untouched, and it clears itself on the next projectable edit.
    ///
    /// An app can render this directly — "not syncing: {error}" — which is the whole
    /// point: before, the app saw a block-count mismatch that named neither the cause
    /// nor the cure.
    pub fn outbound_stall(&self) -> Option<&CollabError> {
        self.stalled.as_ref()
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
    /// [poisoned](Self::is_poisoned) session, so no delta leaves a replica that
    /// cannot converge (issue #196).
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
    ///   rebuild ([`CollabDoc::to_doc`]) cannot be un-applied (yrs has no rollback).
    ///   If yrs holds updates parked on **missing dependencies**, the failure is
    ///   transient — the missing delta cures it. Otherwise the session **poisons
    ///   itself**: this call — and every subsequent convergence-relevant call,
    ///   outbound included — fails with the sticky
    ///   [`SessionPoisoned`](CollabError::SessionPoisoned). The read that decides
    ///   this is the rebuild every integrate already performs, so a healthy delta
    ///   pays nothing extra.
    /// * On a poisoned session, integration is still **attempted**: an update that
    ///   leaves the document rebuildable again clears the poison and applies
    ///   normally; any other inbound failure re-reports the sticky error.
    pub fn integrate_incremental(
        &mut self,
        state: &EditorState,
        changes: &[u8],
    ) -> Result<Option<EditorState>> {
        if changes.is_empty() || changes == EMPTY_UPDATE {
            // Nothing to apply — and therefore nothing that could heal a poisoned
            // session, so the sticky error still reports.
            self.guard()?;
            return Ok(None);
        }
        // Decode failure: the CRDT is untouched, so this can neither poison nor heal.
        let update = match Update::decode_v1(changes) {
            Ok(update) => update,
            Err(e) => return Err(self.sticky_or(e.into())),
        };
        if let Err(apply_err) = self.cdoc.apply_decoded(update) {
            // yrs commits on drop with no rollback, so a failed apply may have
            // partially integrated. Projectability decides how loud to be: if the
            // document still rebuilds, the session is still convergent (transient);
            // if not, classify (pending-aware) like any unprojectable state. The
            // model was not advanced, so a still-projectable outcome does not clear
            // an existing poison — only a full, successful integration does.
            return match self.cdoc.to_doc(state.schema()) {
                Ok(_) => Err(self.sticky_or(apply_err)),
                Err(_) => Err(self.classify_unprojectable(apply_err)),
            };
        }
        // The converged rebuild — the projectability verdict for these bytes.
        let target = match self.cdoc.to_doc(state.schema()) {
            Ok(target) => target,
            Err(cause) => return Err(self.classify_unprojectable(cause)),
        };
        // The document is projectable: a previously-poisoned session is healed by
        // exactly this — a full inbound integration whose rebuild succeeds (the
        // returned state below is what brings the model back in step).
        self.poisoned = None;
        // Past this point a failure is not the unprojectable class: building/applying
        // the model transaction reads only the validated rebuild (and is unreachable
        // in practice for a doc `to_doc` admitted). Defense in depth, not a supported
        // failure path.
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
