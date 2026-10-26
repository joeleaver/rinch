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
//! ## Reading back what an update changed (#1241)
//!
//! The rebuild reads back only what the update changed when the session knows the
//! model it is handed to be the CRDT's read from just before it
//! (`atom_cache::synced`: the document the last integrate brought the model to, a
//! local edit projected from that document since, or the model a session was started
//! on or a guest built from [`CollabSession::projected_doc`]). yrs reports what an
//! update changed as the path to each shared type it touched
//! (`sync::AppliedChanges`); each top-level block a path
//! leads into is read again, a table whose cells' blocks alone changed only in those
//! cells, and every other block is the model's own. Anything a path does not describe
//! — a top-level block inserted or deleted, an atom's entry, a void container, a
//! table too large to read, a block that fails to read — makes it the whole read, as
//! before. A local edit that touched an atom forgets the document, since an atom's
//! identity is what the last read noted for its node. In this crate's debug tests
//! every partial read is compared with a whole one
//! (`testing::set_incremental_read_check`).
//!
//! ## Outbound stalls (issue #220)
//!
//! A local edit outside the staged A22 scope — pasting a ragged table, toggling a task
//! list — is refused by `record_local` with the CRDT untouched. But the *model*
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
//! really is out of scope the error names it (`Unsupported: task_list`) instead of a
//! block-count symptom.
//!
//! Once stalled, the hint is skipped entirely rather than tried first. The fast diff
//! verifies only the block *count* and then skips every block the transaction did not
//! touch, so an out-of-scope block that keeps its index — a `task_list` toggled on
//! a paragraph in place — lets a later edit elsewhere pass, answer `Ok`, and clear the
//! stall while the model and the CRDT still differ. `before` is known to be false from
//! the moment of the first refusal; there is nothing left to trust in it.
//!
//! [`CollabSession::outbound_stall`] reports the state in between, so an app can say
//! "not syncing — remove the task list" rather than leaving the user to wonder. It is
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
//! **is** the diagnosis (a partial read that fails to read a block it changed makes
//! the whole read, whose failure is the verdict; a block no update touched reads as it
//! did) — with one carve-out. A rebuild failure while yrs holds
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

use rinch_editor_core::{EditorState, Node, Pos, Schema};

use crate::atom_cache;
use crate::error::{CollabError, Result};
use crate::projection::{BlockId, CollabDoc};
use crate::remote::{
    AtomIds, InlineIds, atom_nodes, block_atoms, build_remote_transaction,
    build_remote_transaction_with, inline_atoms,
};
use crate::sticky::{StickyAssoc, StickyLocation};
use crate::table::OversizedTable;

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
    /// Whether the model holds local edits the CRDT does not: a [`Self::record_local`]
    /// was refused since the model last matched the CRDT. A freeze alone (a table too
    /// large to read arrived) leaves the model the CRDT's read, so sticky indexes keep
    /// working through it; an edit refused during it does not.
    model_ahead: bool,
    /// This session's key in [`atom_cache`].
    uid: u64,
    /// Bumped by every call that can change the CRDT, so an [`atom_cache`] entry is
    /// used only for the CRDT it was read from.
    generation: u64,
}

impl Drop for CollabSession {
    fn drop(&mut self) {
        atom_cache::forget(self.uid);
    }
}

impl CollabSession {
    /// Start a session from a fresh editor state, projecting its document onto a new
    /// CRDT. Fails loud on content outside the staged scope (design A22).
    pub fn new(state: &EditorState) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::from_doc(&state.doc)?,
            poisoned: None,
            stalled: None,
            model_ahead: false,
            uid: atom_cache::next_uid(),
            generation: 0,
        }
        .with_atom_ids(state))
    }

    /// Note which CRDT block each leaf block atom of `state`'s document became
    /// ([`atom_cache`]), so the first peer's update needs no read of its own to
    /// tell them apart. One read of the new CRDT, paid only when the document holds
    /// such an atom.
    fn with_atom_ids(self, state: &EditorState) -> CollabSession {
        let (blocks, inline) = (block_atoms(&state.doc) > 0, inline_atoms(&state.doc) > 0);
        // The new CRDT is the projection of `state`'s document, so it reads as that
        // document: the first peer's update is read back from what it changes.
        atom_cache::set_synced(
            self.uid,
            state.doc.clone(),
            self.cdoc.epoch(),
            !(blocks || inline),
        );
        if (blocks || inline)
            && let Ok((read, ids, inline_ids)) = self.cdoc.to_doc_with_all_atom_ids(state.schema())
            && read == state.doc
        {
            if blocks && let Some(ids) = ids {
                atom_cache::store(self.uid, self.generation, state.doc.clone(), ids);
            }
            if let Some(inline_ids) = &inline_ids {
                atom_cache::inline_store(self.uid, &state.doc, inline_ids);
            }
        }
        self
    }

    /// A local change the session recorded only set the attributes of these inline
    /// atoms (`(before, after, pos)`: the nodes, as an editor's `SetNodeAttrStep`s
    /// prove, and where `before` stood in the change's first document `doc`): each
    /// `after` is still the CRDT char its `before` was, so a peer's change of its
    /// attributes arriving next is told from another picture at its place, and a
    /// node view drawn for it stays mounted. Without it such a node is unknown
    /// until the next update arrives, and a peer's change of it remounts its view.
    ///
    /// The position says which copy changed when `doc` holds one `Rc` in two places
    /// (an in-editor copy): the carry follows that copy, or carries nothing when not
    /// every copy is known.
    pub fn note_kept_inline_atoms(&self, doc: &Node, pairs: &[(Node, Node, usize)]) {
        atom_cache::inline_carry(self.uid, doc, pairs);
    }

    /// Join an existing collaboration from a peer's saved CRDT bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::load(bytes)?,
            poisoned: None,
            stalled: None,
            model_ahead: false,
            uid: atom_cache::next_uid(),
            generation: 0,
        }
        .with_freeze())
    }

    /// Report the freeze as the outbound stall from the start, when the joined
    /// document holds a table too large to read.
    fn with_freeze(mut self) -> CollabSession {
        self.stalled = self.frozen();
        self
    }

    /// The CRDT may change: retire what [`atom_cache`] holds for the current
    /// generation. Answers that generation, to look the cache up by.
    fn touch(&mut self) -> u64 {
        let previous = self.generation;
        self.generation += 1;
        previous
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
            model_ahead: false,
            uid: atom_cache::next_uid(),
            generation: 0,
        }
        .with_atom_ids(state))
    }

    /// [`CollabSession::from_bytes`] with this replica's yrs client id pinned — the
    /// [`crate::testing`] seam's implementation. Never a production path.
    #[cfg(feature = "test-util")]
    pub(crate) fn from_bytes_with_client_id(bytes: &[u8], client_id: u64) -> Result<CollabSession> {
        Ok(CollabSession {
            cdoc: CollabDoc::load_with_client_id(bytes, Some(ClientID::new(client_id)))?,
            poisoned: None,
            stalled: None,
            model_ahead: false,
            uid: atom_cache::next_uid(),
            generation: 0,
        }
        .with_freeze())
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

    /// The tables in the shared document too large to read, as the last model read
    /// found them: each is shown in the model as a one-cell placeholder, and while any
    /// exists [`Self::record_local`] refuses every local change with
    /// [`CollabError::OversizedTable`]. Delete one with
    /// [`Self::delete_oversized_table`].
    pub fn oversized_tables(&self) -> Vec<OversizedTable> {
        self.cdoc.oversized_tables()
    }

    /// Whether the editor's model is the CRDT's projection, which sticky indexes rely
    /// on: not while poisoned, stalled by a refused edit, or holding edits refused
    /// during a freeze. A freeze on its own keeps it.
    fn model_matches_crdt(&self) -> bool {
        self.poisoned.is_none()
            && !self.model_ahead
            && matches!(self.stalled, None | Some(CollabError::OversizedTable(_)))
    }

    /// The freeze error, while the shared document holds a table too large to read.
    fn frozen(&self) -> Option<CollabError> {
        let tables = self.cdoc.oversized_tables();
        (!tables.is_empty()).then(|| {
            CollabError::OversizedTable(
                tables
                    .iter()
                    .map(|t| format!("table {}: {}", t.id, t.detail))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })
    }

    /// Delete the table too large to read named `id` ([`OversizedTable::id`]) from the
    /// shared document — the cure for the freeze, since no local edit may touch it.
    /// The CRDT node is removed without being read, and the model is brought to the
    /// CRDT's read (as an inbound change is), which removes the placeholder — with the
    /// container it was alone in, if any; one [`Self::save_incremental`] carries the
    /// deletion to peers. Returns the new state, or `None` when `id` is not a table
    /// too large to read **now** ([`Self::oversized_tables`]): a peer deleted it, or
    /// shrank it back into a real table nobody asked to delete.
    ///
    /// No backlog is shipped: an editor refuses every edit while frozen
    /// (`EditorCore::commit`), so its model never runs ahead of the CRDT. A caller
    /// driving the session directly that applied edits [`Self::record_local`] refused
    /// loses them here, as it would on any inbound change. The model change is not
    /// undoable.
    pub fn delete_oversized_table(
        &mut self,
        state: &EditorState,
        id: &str,
    ) -> Result<Option<EditorState>> {
        self.guard()?;
        self.touch();
        if !self.cdoc.oversized_tables().iter().any(|t| t.id == id) || !self.cdoc.delete_table(id) {
            return Ok(None);
        }
        let base = self.cdoc.to_doc(state.schema())?;
        let next = match build_remote_transaction(state, &base)? {
            Some(tr) => state.apply(tr),
            None => state.clone(),
        };
        self.stalled = self.frozen();
        self.model_ahead = false;
        Ok(Some(next))
    }

    /// Whether outbound is frozen: the shared document holds a table too large to
    /// read ([`Self::oversized_tables`]). An editor refuses every local edit while it
    /// is (`EditorCore::commit`), as a read-only one does.
    pub fn is_frozen(&self) -> bool {
        !self.cdoc.oversized_tables().is_empty()
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
    /// offending content (`Unsupported: task_list`) rather than the block-count symptom
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
        self.record_local_inner(schema, before, after)
    }

    /// [`Self::record_local`] for an editor that knows which atoms the change kept:
    /// `kept` holds every leaf block atom and inline atom (a picture) of `after` that
    /// is the same atom as the node at its place in `before` with other attributes
    /// (the editor proves it with the `SetNodeAttrStep`s of the transaction). Any
    /// **other** inline atom the change put where a different node stood is written as
    /// a new CRDT char — even one with the old one's `src` (a picture pasted over
    /// another; decision (f)) — and any other leaf block atom as a new CRDT block,
    /// rather than by rewriting the old one's attributes, so a peer can
    /// tell an embed whose attributes changed (one block, the same on both sides —
    /// [`Self::integrate_incremental_transaction`] makes that `SetNodeAttrStep`s)
    /// from an embed that came to stand where another stood (a move, an undo of a
    /// delete), which a node view keyed to the old one must not be handed.
    ///
    /// `kept` names the new nodes only, so each matches whatever node stood at its
    /// place: an editor's transaction should use [`Self::record_local_edit`], which
    /// also names the old ones. This form is for a load (`kept` empty) and for a
    /// caller that cannot name them.
    ///
    /// [`Self::record_local`] is the positional rule: a leaf atom put in another's
    /// place takes over its block, attributes and all, and a picture put in the place
    /// of one with the same `src` takes over its char.
    pub fn record_local_keeping(
        &mut self,
        schema: &Schema,
        before: &Node,
        after: &Node,
        kept: &[Node],
    ) -> Result<()> {
        let _kept = crate::projection::kept_atoms_scope(Some(kept));
        self.record_local_inner(schema, before, after)
    }

    /// [`Self::record_local_keeping`] for an editor's **transaction**: `kept` pairs each
    /// atom the transaction only set attributes of with the node it was before
    /// (`(old, new)`), and `before` and `after` are one document edited, never a load.
    ///
    /// The old half is what makes the proof sound: proved by its new node alone, an
    /// atom kept right after one the same transaction deleted (delete one picture or
    /// embed, renumber its neighbour, in one `update`) was paired with the deleted one's
    /// char or block, and a peer's concurrent change of the deleted one showed on it
    /// (review 2 of #1595, G1). And with it a proved pair keeps its char even where the
    /// model holds one of its `Rc`s twice (an in-editor copy, G2) — while the edit took
    /// no other copy away: a pair whose old `Rc` the change left with more than one copy
    /// missing names no one place and is dropped ([`one_place_pairs`]; the editor also
    /// checks every other copy stayed in place, which needs the transaction's mapping).
    /// A load, or a caller that cannot name the old nodes, uses
    /// [`Self::record_local_keeping`].
    pub fn record_local_edit(
        &mut self,
        schema: &Schema,
        before: &Node,
        after: &Node,
        kept: &[(Node, Node)],
    ) -> Result<()> {
        let kept = one_place_pairs(before, after, kept);
        let _kept = crate::projection::kept_pairs_scope(&kept);
        self.record_local_inner(schema, before, after)
    }

    fn record_local_inner(&mut self, schema: &Schema, before: &Node, after: &Node) -> Result<()> {
        self.guard()?;
        let was_matching = self.model_matches_crdt();
        let cached = atom_cache::lookup(self.uid, self.touch(), before);
        // Whether `before` is what the CRDT reads as now (`atom_cache::synced`): then
        // `after` is once the change is projected, which is the projection's own
        // invariant, and the next integrate reads back only what it changes (#1241).
        // A change refused below leaves the CRDT as it was, so this stays true of
        // `before`; one that writes moves the epoch past it.
        let synced = atom_cache::synced(self.uid, before, self.cdoc.epoch());
        // Frozen while the shared document holds a table too large to read: the model
        // shows a placeholder for it, and a diff that touches a placeholder can delete
        // real content (a value-matched pairing, an export loaded back, …), so no diff
        // runs at all. An editor never gets here with a real edit: it refuses the edit
        // itself while frozen (`EditorCore::commit`). A caller driving the session
        // directly that applies a refused edit anyway has a model ahead of the CRDT,
        // which the next inbound change or the cure discards.
        if let Some(frozen) = self.frozen() {
            self.stalled = Some(frozen.clone());
            self.model_ahead = true;
            self.carry_unprojected(before, after, cached);
            return Err(frozen);
        }
        let fallback = match self.stalled.clone() {
            // Already stalled: `before` is a *known*-false description of the CRDT, so
            // the fast diff must not be tried at all. It verifies only the block count
            // (`project_change` gate 1) and then skips every block the transaction did
            // not touch — an `Rc`-identity prefix/suffix that is never compared against
            // the CRDT. A stalled session whose out-of-scope block keeps its index and
            // is left alone by the next edit therefore passes the fast path, answers
            // `Ok`, and clears this very flag while the model and the CRDT still differ:
            // `<paragraph>Zone<task_list>two` against `<paragraph>Zone<paragraph>two`,
            // silent, with `outbound_stall()` reporting healthy. Go straight to the
            // authoritative base instead — which is also what makes the "a stalled
            // session pays the read-back per edit" note below true.
            Some(prev) => prev,
            None => match self.cdoc.project_change(before, after) {
                Ok(()) => {
                    // Only while the change touched no atom: an atom's identity is
                    // what the last read noted for its node (`atom_cache`), which a
                    // change that rewrites its char may leave behind.
                    if let Some(no_atoms) = synced
                        && !self.cdoc.touched_atoms
                    {
                        atom_cache::set_synced(
                            self.uid,
                            after.clone(),
                            self.cdoc.epoch(),
                            no_atoms,
                        );
                    }
                    // A document with no leaf block atom gains nothing from the
                    // cache: the next integrate's miss costs one walk of the blocks
                    // beside a whole read of the CRDT, where keeping it costs every
                    // local keystroke a second walk (review 2 of #1575, N3).
                    if let Some(ids) = cached
                        .filter(|ids| !ids.is_empty())
                        .and_then(|ids| atoms_after(before, after, ids))
                    {
                        atom_cache::store(self.uid, self.generation, after.clone(), ids);
                    }
                    return Ok(());
                }
                Err(e) => e,
            },
        };
        // The caller's `before` did not describe the CRDT. Re-base on what does — a
        // write that pairs the model's inline atoms with chars by value, so which
        // char each is cannot be carried through it.
        atom_cache::inline_forget(self.uid);
        // The caller's `before` did not describe the CRDT. Re-base on what does.
        // `to_doc` failing means the shared document is unprojectable — report it, but
        // do not poison: poison is for *inbound* damage (issue #196), and this path has
        // written nothing.
        // The read notes the CRDT block of each leaf block atom at no extra cost.
        let (base, base_ids) = match self.cdoc.to_doc_with_atom_ids(schema) {
            Ok(read) => read,
            Err(_) => {
                self.stalled = Some(fallback.clone());
                self.carry_unprojected(before, after, cached);
                return Err(fallback);
            }
        };
        // Which CRDT block each leaf block atom of `before` is: what the last
        // operation left in the cache, or — when the model was the CRDT's read until
        // this very edit (the stall starts here) — the read's, since a model that
        // matches the CRDT is its read index for index. Nothing else proves it.
        let before_ids = if block_atoms(after) == 0 {
            None
        } else {
            cached.or_else(|| {
                if was_matching && base == *before {
                    base_ids.clone()
                } else {
                    None
                }
            })
        };
        // The read may have found a table too large to read (one a local edit pushed
        // past the budget): freeze rather than diff against its placeholder.
        if let Some(frozen) = self.frozen() {
            self.stalled = Some(frozen.clone());
            self.model_ahead = true;
            self.carry_unprojected(before, after, before_ids);
            return Err(frozen);
        }
        // The re-base's `before` is the CRDT's read, which holds none of the caller's
        // old nodes and is not the document the caller edited: keep the proof's new
        // nodes only (`review2_1575::s11` otherwise remounts a view the stall did not
        // move).
        let _rebase_kept = crate::projection::rebase_proof_scope();
        match self.cdoc.project_change(&base, after) {
            Ok(()) => {
                self.stalled = None;
                self.model_ahead = false;
                self.settle_rebased_atoms(schema, before, after, before_ids, base_ids)
            }
            Err(e) => {
                self.stalled = Some(e.clone());
                self.carry_unprojected(before, after, before_ids);
                Err(e)
            }
        }
    }

    /// A local edit the CRDT refused (an outbound stall, #220; a freeze) moved the
    /// model from `before` to `after` and the CRDT not at all, so every leaf block
    /// atom of `after` that is one of `before`'s is still that atom's CRDT block.
    /// Note them for the next operation ([`carry_ids`]): a peer's change arriving
    /// during the stall then tells an atom whose attributes the peer changed from
    /// another that came to stand at its place, as it does when nothing is
    /// stalled. An atom nothing proves (one the refused edit wrote, or every one
    /// when `before`'s blocks are not known) is [unproven](atom_cache::unproven),
    /// so a view drawn for it is remounted, never re-pointed at a block it was not
    /// mounted for (review 2 of #1575, N1).
    fn carry_unprojected(&self, before: &Node, after: &Node, before_ids: Option<Vec<BlockId>>) {
        let n = block_atoms(after);
        if n == 0 {
            return;
        }
        let ids = match before_ids {
            Some(ids) if ids.len() == block_atoms(before) => carry_ids(before, after, &ids),
            _ => atom_cache::unproven_ids(n),
        };
        atom_cache::store(self.uid, self.generation, after.clone(), ids);
    }

    /// A re-base that ended a stall wrote `after` against the CRDT's own read,
    /// pairing blocks **by value**, so an atom of the model may now stand on
    /// another atom's CRDT block: two equal embeds the model reordered while
    /// stalled, or a copy inserted before its original. A view drawn for it would
    /// then write to the block a peer's view keys another node's state to. Read
    /// which block each atom of `after` became; where that is not the block the
    /// model proved it to be ([`carry_ids`]) — or an atom nothing proved took a
    /// block that was there before — write that atom again as a new block, so no
    /// view on any peer is handed another node (the peers' views of the old block
    /// are remounted). Paid only by the edit that ends a stall, and only while
    /// `after` holds a leaf block atom.
    fn settle_rebased_atoms(
        &mut self,
        schema: &Schema,
        before: &Node,
        after: &Node,
        before_ids: Option<Vec<BlockId>>,
        base_ids: Option<Vec<BlockId>>,
    ) -> Result<()> {
        let n = block_atoms(after);
        if n == 0 {
            return Ok(());
        }
        let carried = match before_ids {
            Some(ids) if ids.len() == block_atoms(before) => carry_ids(before, after, &ids),
            _ => atom_cache::unproven_ids(n),
        };
        let Ok((read, Some(now))) = self.cdoc.to_doc_with_atom_ids(schema) else {
            return Ok(());
        };
        if read != *after || now.len() != n {
            return Ok(());
        }
        let existed = |id: &BlockId| base_ids.as_ref().is_none_or(|ids| ids.contains(id));
        let wrong: Vec<usize> = (0..n)
            .filter(|&i| {
                carried[i] != now[i] && !(atom_cache::is_unproven(carried[i]) && !existed(&now[i]))
            })
            .collect();
        if wrong.is_empty() {
            atom_cache::store(self.uid, self.generation, after.clone(), now);
            return Ok(());
        }
        // `after` with those atoms copied (a new `Rc` each, equal by value): the
        // projection pairs each copy with its original as two different atoms and,
        // told none is kept, writes the original as a new block
        // (`changed_atom_identity`). Everything else is shared, so untouched.
        let refreshed = refresh_atoms(after, &wrong, &mut 0);
        let written = {
            let _none_kept = crate::projection::kept_atoms_scope(Some(&[]));
            self.cdoc.project_change(&refreshed, after)
        };
        if let Err(e) = written {
            self.stalled = Some(e.clone());
            return Err(e);
        }
        if let Ok((read, Some(now))) = self.cdoc.to_doc_with_atom_ids(schema)
            && read == *after
        {
            atom_cache::store(self.uid, self.generation, after.clone(), now);
        }
        Ok(())
    }

    /// Why this replica's **outbound** is currently refusing, if it is (issue #220).
    ///
    /// `Some` from the moment a local edit cannot be projected — content outside the
    /// staged A22 scope (a ragged table paste, a task list) — until a later
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
    ///
    /// [`CollabError::OversizedTable`] here is the **freeze**: the shared document
    /// holds a table too large to read, and it is reported from the moment the table
    /// arrives (or a guest joins), before any local edit. It does not clear on an
    /// edit — every local edit is refused while it lasts — but on
    /// [`Self::delete_oversized_table`] or an inbound change that removes or shrinks
    /// the table.
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
        Ok(self
            .integrate_incremental_transaction(state, changes)?
            .map(|tr| state.apply(tr)))
    }

    /// [`Self::integrate_incremental`], answering the **transaction** that brings
    /// `state` to the converged document rather than the state it leads to —
    /// for a caller that has to know how the document changed, not only what it
    /// became (an editor view telling a node whose attributes a peer changed from
    /// a node a peer replaced).
    ///
    /// A peer's change that only sets attributes of leaf block atoms (an
    /// `embed`'s `data-*` keys) — and only of atoms that are **the same CRDT
    /// block** before and after, so two embeds a peer swapped are not one —
    /// is one `SetNodeAttrStep` per key, and moves no position; anything else is
    /// the block-level replace [`build_remote_transaction`] describes. Telling the
    /// two apart needs the old document's block identities: the previous
    /// operation left them in [`atom_cache`]; on a miss they are read before the
    /// update is applied, a second full read of the CRDT, paid only while
    /// `state`'s document holds a leaf block atom. While outbound is stalled
    /// (#220) the model is not the CRDT's read and nothing is read: the blocks a
    /// refused local edit carried are used, and an atom nothing proves is
    /// replaced, so a view drawn for it is remounted.
    pub fn integrate_incremental_transaction(
        &mut self,
        state: &EditorState,
        changes: &[u8],
    ) -> Result<Option<rinch_editor_core::Transaction>> {
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
        // The CRDT blocks of the old document's leaf block atoms (`remote::AtomIds`):
        // only while the model is the CRDT's read (so its atoms are the CRDT's, one
        // for one). The last operation left them in `atom_cache` for exactly this
        // document; otherwise — the first update after the session started, or after
        // a local edit that wrote an atom — they are read before the update moves
        // anything, which costs a whole read of the CRDT, and only while the model
        // holds an atom at all.
        let generation = self.touch();
        // Whether the model is what the CRDT reads as right now, as the last
        // operation left it (`atom_cache::synced`): then only what the update changes
        // is read back (#1241), and `Some(true)` says the document holds no atom.
        let epoch = self.cdoc.epoch();
        let synced =
            (self.model_matches_crdt() && self.frozen().is_none() && incremental_read_enabled())
                .then(|| atom_cache::synced(self.uid, &state.doc, epoch))
                .flatten();
        let old_ids = if synced == Some(true) {
            Some(Vec::new())
        } else if self.model_matches_crdt() {
            atom_cache::lookup(self.uid, generation, &state.doc).or_else(|| {
                if block_atoms(&state.doc) == 0 {
                    return Some(Vec::new());
                }
                self.cdoc
                    .to_doc_with_atom_ids(state.schema())
                    .ok()
                    .filter(|(read, _)| *read == state.doc)
                    .and_then(|(_, ids)| ids)
                    // A read that differs from the model (a previous integrate
                    // failed after yrs applied part of an out-of-order update)
                    // proves nothing: every atom is remounted rather than paired
                    // by value with an equal one (review 3 of #1575, F1).
                    .or_else(|| Some(atom_cache::unproven_ids(block_atoms(&state.doc))))
            })
        } else {
            // The model is not the CRDT's read (outbound stalled, a refused edit
            // during a freeze, poisoned), so its atoms are not the read's index for
            // index. What a refused local edit carried (`carry_unprojected`) says
            // which are still their blocks; nothing proves the rest, and they are
            // remounted rather than paired with whatever stands at their place
            // (review 2 of #1575, N1).
            Some(
                atom_cache::lookup(self.uid, generation, &state.doc)
                    .unwrap_or_else(|| atom_cache::unproven_ids(block_atoms(&state.doc))),
            )
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
        // Which char each inline atom of the model is, from what the last read
        // left (`atom_cache::inline_lookup`): a picture the model made since is
        // unknown, and a peer's attribute change of it is a replace.
        let old_inline = if synced == Some(true) {
            Vec::new()
        } else {
            atom_cache::inline_lookup(self.uid, &state.doc)
        };
        // The converged document: read back from what the update changed when the
        // model is the CRDT's read from before it and every atom of it is known
        // (#1241), else the whole converged rebuild, which is also the
        // projectability verdict for these bytes. Noting the atoms' blocks costs no
        // read of its own either way.
        let changed = synced.and_then(|no_atoms| {
            let changes = self.cdoc.last_apply.as_ref().filter(|c| c.epoch == epoch)?;
            let old_ids = old_ids.as_deref()?;
            if old_inline.iter().any(Option::is_none) {
                return None;
            }
            self.cdoc.read_changed(
                state.schema(),
                &state.doc,
                changes,
                old_ids,
                &old_inline,
                no_atoms,
            )
        });
        note_read(changed.is_some());
        let (target, new_ids, new_inline, touched) = match changed {
            Some(read) => {
                if incremental_read_checked() {
                    check_against_whole_read(&self.cdoc, state.schema(), &read);
                }
                (
                    read.doc,
                    Some(read.block_ids),
                    Some(read.inline_ids),
                    Some(read.touched),
                )
            }
            None => match self.cdoc.to_doc_with_all_atom_ids(state.schema()) {
                Ok((doc, ids, inline)) => (doc, ids, inline, None),
                Err(cause) => return Err(self.classify_unprojectable(cause)),
            },
        };
        // The document is projectable: a previously-poisoned session is healed by
        // exactly this — a full inbound integration whose rebuild succeeds (the
        // returned state below is what brings the model back in step).
        self.poisoned = None;
        // The freeze follows the shared document: on while it holds a table too large
        // to read, off once a peer deleted or shrank the last one. The model is the
        // CRDT's read after this integration (the transaction below makes it so), so
        // a lifted freeze leaves nothing to re-base.
        match self.frozen() {
            Some(frozen) => self.stalled = Some(frozen),
            None if matches!(self.stalled, Some(CollabError::OversizedTable(_))) => {
                self.stalled = None
            }
            None => {}
        }
        if matches!(self.stalled, None | Some(CollabError::OversizedTable(_))) {
            self.model_ahead = false;
        }
        // Past this point a failure is not the unprojectable class: building/applying
        // the model transaction reads only the validated rebuild (and is unreachable
        // in practice for a doc `to_doc` admitted). Defense in depth, not a supported
        // failure path.
        let inline = new_inline.as_deref().map(|new| InlineIds {
            old: &old_inline,
            new,
        });
        // Neither document holds an atom: the model by what the session knows of it,
        // the converged one by its read.
        let next_no_atoms = new_ids.as_ref().is_some_and(Vec::is_empty)
            && new_inline.as_ref().is_some_and(Vec::is_empty);
        let no_atoms = synced == Some(true) && next_no_atoms;
        let ids = match (&old_ids, &new_ids) {
            (Some(old), Some(new)) => Some(AtomIds {
                old,
                new,
                inline,
                none: no_atoms,
            }),
            _ => None,
        };
        let tr = build_remote_transaction_with(state, &target, ids.as_ref())?;
        let next = tr
            .as_ref()
            .map_or_else(|| state.doc.clone(), |tr| tr.doc().clone());
        // The document the editor holds next is the read, atom for atom: note
        // which char each of its inline atoms is.
        match &new_inline {
            Some(_) if no_atoms => atom_cache::inline_store_none(self.uid),
            Some(new_inline) => atom_cache::inline_store(self.uid, &next, new_inline),
            None => atom_cache::inline_forget(self.uid),
        }
        // The document the editor holds next, and its atoms' blocks — for the next
        // operation (`atom_cache`). Only when this one knew which atom was which.
        if ids.is_some()
            && let Some(new_ids) = new_ids
        {
            atom_cache::store(self.uid, self.generation, next.clone(), new_ids);
        }
        // The tables' cell indexes (`table::locate_cell`) stay true through this
        // update where it left a table's grid alone (#1241): one it did not touch, and
        // one whose cells' blocks alone it changed, which is the table at the same
        // place in the next document. A table it read whole is indexed afresh. After a
        // whole read nothing is known, and every index goes stale.
        let written = self.cdoc.epoch();
        if let Some(touched) = touched {
            use crate::projection::TouchedRead;
            crate::table::carry_cell_indexes(self.cdoc.uid, epoch, written, |map, table| {
                match touched.iter().find(|t| crate::table::same_map(&t.map, map)) {
                    None => Some(table.clone()),
                    Some(t) if matches!(t.read, TouchedRead::Cells) => {
                        next.content().maybe_child(t.index).cloned()
                    }
                    Some(_) => None,
                }
            });
            for t in touched {
                if let (TouchedRead::Table(Some(cells)), Some(table)) =
                    (t.read, next.content().maybe_child(t.index))
                {
                    crate::table::install_cell_index(
                        self.cdoc.uid,
                        written,
                        table.clone(),
                        t.map,
                        cells,
                    );
                }
            }
        }
        // The model is the CRDT's read now: note it for the next operation.
        if self.model_matches_crdt() && self.frozen().is_none() {
            atom_cache::set_synced(self.uid, next, written, next_no_atoms);
        }
        Ok(tr)
    }

    /// Merge another peer's CRDT wholesale into this one (test/utility convenience).
    /// Refused — sticky — when either side is [poisoned](Self::is_poisoned), for the
    /// same reason as [`Self::sync_diff`]: poison must not spread.
    pub fn merge(&mut self, other: &CollabSession) -> Result<()> {
        self.guard()?;
        other.guard()?;
        self.touch();
        self.cdoc.merge_from(&other.cdoc)
    }

    /// The sticky index of model position `pos` of `doc` (this session's editor
    /// document), for a deep link that follows the text through every later edit, on
    /// this replica or any other. The bytes are a plain yrs `StickyIndex`, v1-encoded:
    /// see [`CollabDoc::sticky_index`] for the encoding, how to resolve it with yrs
    /// alone, and when it is `None`.
    ///
    /// Also `None` while the session is [poisoned](Self::is_poisoned) or outbound is
    /// [stalled](Self::outbound_stall): then the editor document holds content the CRDT
    /// does not, so its positions cannot be trusted to name the CRDT's blocks.
    pub fn sticky_index(&self, doc: &Node, pos: Pos) -> Option<Vec<u8>> {
        if !self.model_matches_crdt() {
            return None;
        }
        self.cdoc.sticky_index(doc, pos)
    }

    /// Where the sticky index `bytes` points now, as a position of `doc` (this
    /// session's editor document). See [`CollabDoc::resolve_sticky`] for when it is
    /// `None`; also `None` while the session is poisoned or outbound is stalled, as
    /// for [`Self::sticky_index`].
    pub fn resolve_sticky(&self, doc: &Node, bytes: &[u8]) -> Option<Pos> {
        if !self.model_matches_crdt() {
            return None;
        }
        self.cdoc.resolve_sticky(doc, bytes)
    }

    /// [`Self::sticky_index`] with the association `assoc`, in the form that survives
    /// the deletion of its block: see [`CollabDoc::sticky_index_with`]. `None` in the
    /// same cases as [`Self::sticky_index`].
    pub fn sticky_index_with(&self, doc: &Node, pos: Pos, assoc: StickyAssoc) -> Option<Vec<u8>> {
        if !self.model_matches_crdt() {
            return None;
        }
        self.cdoc.sticky_index_with(doc, pos, assoc)
    }

    /// Where the sticky index `bytes` points now in `doc` (this session's editor
    /// document), with its textblock's path and whether its block is gone: see
    /// [`CollabDoc::locate_sticky`]. Also `None` while the session is poisoned or
    /// outbound is stalled, as for [`Self::sticky_index`].
    pub fn locate_sticky(&self, doc: &Node, bytes: &[u8]) -> Option<StickyLocation> {
        if !self.model_matches_crdt() {
            return None;
        }
        self.cdoc.locate_sticky(doc, bytes)
    }

    /// [`Self::locate_sticky`] for every index of `anchors`, in order, in one walk of
    /// the CRDT ([`CollabDoc::locate_sticky_many`]). Every answer is `None` while the
    /// session is poisoned or outbound is stalled.
    pub fn locate_sticky_many(&self, doc: &Node, anchors: &[&[u8]]) -> Vec<Option<StickyLocation>> {
        if !self.model_matches_crdt() {
            return vec![None; anchors.len()];
        }
        self.cdoc.locate_sticky_many(doc, anchors)
    }

    /// The model document the CRDT currently projects to — the canonical converged
    /// read-back (both peers reconstruct identically). The rock-solid convergence
    /// assertion in tests. On a [poisoned](Self::is_poisoned) session this returns the
    /// sticky error rather than re-deriving the underlying read failure.
    pub fn projected_doc(&self, schema: &Schema) -> Result<Node> {
        self.guard()?;
        let (doc, ids, inline) = self.cdoc.to_doc_with_all_atom_ids(schema)?;
        // An editor built on this document (a guest joining) then knows its atoms'
        // blocks, and which char each of its inline atoms is, without another read;
        // and, unless the session already knows a document its CRDT reads as now
        // (an editor's own, which a test's look at the read must not displace), that
        // the CRDT reads as this one (#1241).
        let epoch = self.cdoc.epoch();
        if self.model_matches_crdt()
            && self.frozen().is_none()
            && !atom_cache::synced_at(self.uid, epoch)
        {
            let none = ids.as_ref().is_some_and(Vec::is_empty)
                && inline.as_ref().is_some_and(Vec::is_empty);
            atom_cache::set_synced(self.uid, doc.clone(), epoch, none);
        }
        if let Some(ids) = ids {
            atom_cache::store(self.uid, self.generation, doc.clone(), ids);
        }
        match &inline {
            Some(inline) => atom_cache::inline_store(self.uid, &doc, inline),
            None => atom_cache::inline_forget(self.uid),
        }
        Ok(doc)
    }
}

/// The CRDT blocks of `after`'s leaf block atoms, given `before`'s (`ids`), after
/// [`CollabDoc::project_change`] wrote `before → after` the fast way: the top-level
/// blocks the two share at either end (`Rc` identity, as the projection takes them)
/// were not written and keep their blocks, and the changed middle is written
/// afresh. `None` — read the CRDT next time — when the middle of `after` holds a
/// leaf block atom (its block is new, or was rewritten in place), or the two share
/// no block (the projection then matches by value).
fn atoms_after(
    before: &Node,
    after: &Node,
    ids: Vec<crate::projection::BlockId>,
) -> Option<Vec<crate::projection::BlockId>> {
    atom_cache::note_walk();
    let (prefix, suffix) = crate::projection::identity_runs(before, after);
    if (prefix, suffix) == (0, 0) {
        return None;
    }
    let (bn, an) = (before.child_count(), after.child_count());
    if (prefix..an - suffix).any(|i| block_atoms(after.child(i)) > 0) {
        return None;
    }
    let gone: usize = (prefix..bn - suffix)
        .map(|i| block_atoms(before.child(i)))
        .sum();
    if gone == 0 {
        return Some(ids);
    }
    let kept_before: usize = (0..prefix).map(|i| block_atoms(before.child(i))).sum();
    let mut ids = ids;
    ids.drain(kept_before..kept_before + gone);
    Some(ids)
}

/// The CRDT blocks of `after`'s leaf block atoms, given `before`'s (`ids`), for a
/// change the CRDT did **not** take ([`CollabSession::carry_unprojected`]): the
/// CRDT is as it was, so every atom of `after` that is one of `before`'s is still
/// that atom's block. That is an atom in a top-level block the two share at either
/// end ([`crate::projection::identity_runs`]), one of the changed middle that is
/// the same `Rc` as an atom of `before`'s middle (a local move; each of those
/// claimed once, so a copy pasted twice is one atom and one unknown), or one the
/// editor proved is the atom standing at its place before
/// ([`crate::projection::is_kept_atom`], an attribute change) when the middle
/// holds as many atoms on both sides. Every other atom is
/// [unproven](atom_cache::unproven).
fn carry_ids(
    before: &Node,
    after: &Node,
    ids: &[crate::projection::BlockId],
) -> Vec<crate::projection::BlockId> {
    atom_cache::note_walk();
    let (prefix, suffix) = crate::projection::identity_runs(before, after);
    let (bn, an) = (before.child_count(), after.child_count());
    let head: usize = (0..prefix).map(|i| block_atoms(before.child(i))).sum();
    let tail: usize = (bn - suffix..bn)
        .map(|i| block_atoms(before.child(i)))
        .sum();
    let mut old_mid = Vec::new();
    for i in prefix..bn - suffix {
        atom_nodes(before.child(i), &mut old_mid);
    }
    let mut new_mid = Vec::new();
    for i in prefix..an - suffix {
        atom_nodes(after.child(i), &mut new_mid);
    }
    let old_mid_ids = &ids[head..ids.len() - tail];
    let mut claimed = vec![false; old_mid.len()];
    let mut out = ids[..head].to_vec();
    for (j, atom) in new_mid.iter().enumerate() {
        let moved = (0..old_mid.len()).find(|&i| !claimed[i] && old_mid[i].same_ref(atom));
        let id = match moved {
            Some(i) => {
                claimed[i] = true;
                old_mid_ids[i]
            }
            None if old_mid.len() == new_mid.len()
                && !claimed[j]
                && crate::projection::is_kept_atom(atom) =>
            {
                claimed[j] = true;
                old_mid_ids[j]
            }
            None => atom_cache::unproven(),
        };
        out.push(id);
    }
    out.extend_from_slice(&ids[ids.len() - tail..]);
    out
}

/// `node` with its leaf block atoms at document-order indices `which` (sorted)
/// copied — a new `Rc` each, equal by value — and every ancestor of one rebuilt
/// around it; everything else is shared. `next` counts the atoms passed.
fn refresh_atoms(node: &Node, which: &[usize], next: &mut usize) -> Node {
    if crate::projection::is_leaf_block_atom(node.node_type()) {
        let i = *next;
        *next += 1;
        return if which.contains(&i) {
            node.copy_with_content(node.content().clone())
        } else {
            node.clone()
        };
    }
    if node.is_textblock() || node.is_leaf() {
        return node.clone();
    }
    let start = *next;
    let children: Vec<Node> = node
        .content()
        .children()
        .iter()
        .map(|c| refresh_atoms(c, which, next))
        .collect();
    if which.iter().any(|&i| (start..*next).contains(&i)) {
        node.copy_with_content(rinch_editor_core::Fragment::from_children(children))
    } else {
        node.clone()
    }
}

/// The pairs of `kept` (`(old, new)`, an editor's proof for `before` → `after`) that
/// name one place: `old` stands once in `before`, or one copy fewer of it is left in
/// `after` (the one that became `new`). After an in-editor copy one `Rc` stands in two
/// places, and when the edit also took another copy away, a pair naming only the `Rc`
/// could be matched with that copy's CRDT char or block, and a peer's concurrent change
/// of it would show on `new` (the round-3 review of #1595, F2). Such a pair proves
/// nothing: `new` is written as a new atom, its peer's change lost, never misattributed.
/// The editor checks more than the count — that every other copy is still in place
/// (`EditorHandle`'s own filter, which has the transaction's mapping) — so this is the
/// floor for a caller driving the session directly.
fn one_place_pairs(before: &Node, after: &Node, kept: &[(Node, Node)]) -> Vec<(Node, Node)> {
    let count = |doc: &Node, old: &Node| {
        let mut n = 0usize;
        doc.nodes_between(0, doc.content_size(), &mut |m: &Node, _, _| {
            n += usize::from(m.same_ref(old));
            true
        });
        n
    };
    kept.iter()
        .filter(|(old, _)| {
            let was = count(before, old);
            was <= 1 || count(after, old) + 1 == was
        })
        .cloned()
        .collect()
}

// ─── Reading back only what an update changed (#1241) ────────────────────────

thread_local! {
    /// Whether an integrate may read back only what an update changed
    /// ([`crate::testing::set_incremental_read`]).
    static INCREMENTAL_READ: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Whether every such read is compared with a whole read
    /// ([`crate::testing::set_incremental_read_check`]): on in this crate's own debug
    /// test builds, off everywhere else.
    static INCREMENTAL_CHECK: std::cell::Cell<bool> =
        const { std::cell::Cell::new(cfg!(all(feature = "test-util", debug_assertions))) };
    /// Integrates that read back only what changed, and ones that read the whole
    /// document ([`crate::testing::integrate_reads`]).
    static READS: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

fn incremental_read_enabled() -> bool {
    INCREMENTAL_READ.with(|c| c.get())
}

fn incremental_read_checked() -> bool {
    INCREMENTAL_CHECK.with(|c| c.get())
}

fn note_read(changed_only: bool) {
    READS.with(|c| {
        let (a, b) = c.get();
        c.set(if changed_only { (a + 1, b) } else { (a, b + 1) });
    });
}

#[cfg_attr(not(feature = "test-util"), allow(dead_code))]
pub(crate) fn set_incremental_read(on: bool) {
    INCREMENTAL_READ.with(|c| c.set(on));
}

#[cfg_attr(not(feature = "test-util"), allow(dead_code))]
pub(crate) fn set_incremental_read_check(on: bool) {
    INCREMENTAL_CHECK.with(|c| c.set(on));
}

#[cfg_attr(not(feature = "test-util"), allow(dead_code))]
pub(crate) fn integrate_reads() -> (u64, u64) {
    READS.with(|c| c.get())
}

/// Panic unless `read` is what a whole read of the CRDT gives: the document and
/// its atoms' identities. Only while a test asks for it.
fn check_against_whole_read(
    cdoc: &CollabDoc,
    schema: &Schema,
    read: &crate::projection::ChangedRead,
) {
    let (doc, ids, inline) = crate::projection::uncounted(|| cdoc.to_doc_with_all_atom_ids(schema))
        .expect("a document read back in part reads back whole");
    assert_eq!(read.doc, doc, "the document read back in part");
    assert_eq!(Some(&read.block_ids), ids.as_ref(), "its leaf block atoms");
    assert_eq!(Some(&read.inline_ids), inline.as_ref(), "its inline atoms");
}
