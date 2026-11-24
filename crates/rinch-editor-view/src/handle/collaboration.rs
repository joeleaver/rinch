//! The `collaboration` feature's `impl EditorHandle`: hosting or joining a
//! session, applying a peer's delta, and the sync protocol.

use super::*;

// ── Collaboration (design M9, the `collaboration` feature) ───────────────────
//
// One [`CollabSession`] per editor. A local edit is projected onto the CRDT and
// broadcast (the `commit` → `record_local` path in `editor_core.rs`); a peer's delta arrives via
// `collab_receive`, which integrates it and re-projects without re-broadcasting. The
// transport is the caller's concern: `outbound` carries bytes out, `collab_receive`
// (or the platform runtime's thread-safe `post_remote_delta`) carries them
// back in.
impl EditorHandle {
    /// Start collaborating as the **host** of a fresh session: project this
    /// editor's current document onto a new CRDT and return a snapshot peers join
    /// from (via [`Self::start_collaboration_guest`]). `outbound` carries each delta
    /// produced by a *local* edit to peers; it is invoked on the main thread right
    /// after the edit is projected. Returns the join snapshot, or a [`CollabError`]
    /// if the current document is outside the staged flat-text scope (design A22).
    ///
    /// `outbound` runs while this handle is borrowed, so it must **not** synchronously
    /// re-enter the *same* handle — forward the bytes to a transport/channel or to a
    /// *peer* handle ([`Self::collab_receive`] is borrow-soft, but the mutation
    /// methods are not).
    ///
    /// # The transport owns relaying
    ///
    /// `outbound` fires for this editor's **own** edits only. A delta that arrives through
    /// [`Self::collab_receive`] is deliberately *not* re-broadcast — it is already in the
    /// shared CRDT, and echoing it would loop between peers. So the adapter never relays
    /// transitively, and the transport must be one of:
    ///
    /// * a **full mesh**, where every peer's `outbound` reaches every other peer; or
    /// * a **hub**, where the server fans each received delta out to the other peers
    ///   itself, forwarding the raw bytes (they need no re-encoding).
    ///
    /// A chain — A wired to B, B wired to C, with nothing joining A and C — silently
    /// partitions: C never sees A's edits. It will not error; the two ends simply drift.
    /// If a peer may fall behind for any reason, the repair is the reconciliation pair
    /// ([`Self::collab_state_vector`] / [`Self::collab_sync_diff`]), which catches a peer
    /// up from whatever it is missing.
    pub fn start_collaboration_host(
        &self,
        outbound: impl Fn(Vec<u8>) + 'static,
    ) -> Result<Vec<u8>, CollabError> {
        let session = CollabSession::new(&self.core().state)?;
        Ok(self.start_collaboration_host_with(session, outbound))
    }

    /// [`Self::start_collaboration_host`] with a session the caller built over this
    /// editor's current document (`CollabSession::new(&handle.state())`). **Test
    /// seam**: a test builds the session with a pinned yrs client id
    /// (`rinch_editor_collab::testing`, behind that crate's `test-util` feature, which
    /// no release build enables), so a multi-peer fuzz replays a seed bit for bit. A
    /// session over another document makes the editor and its CRDT disagree.
    #[doc(hidden)]
    pub fn start_collaboration_host_with(
        &self,
        session: CollabSession,
        outbound: impl Fn(Vec<u8>) + 'static,
    ) -> Vec<u8> {
        let snapshot = session.snapshot();
        self.core_mut().collab = Some(CollabBridge::new(session, Box::new(outbound)));
        snapshot
    }

    /// Join an existing collaboration as a **guest** from a host's `snapshot` (from
    /// [`Self::start_collaboration_host`]): adopt the host's converged document and
    /// attach a session whose CRDT already matches it, so both peers start from the
    /// same content. `outbound` carries this guest's local deltas back to peers.
    /// Returns a [`CollabError`] if the snapshot is unreadable or its content is
    /// outside the staged scope.
    ///
    /// As on the host, `outbound` fires for this editor's own edits only — an integrated
    /// remote delta is never re-broadcast, so the transport must mesh the peers or fan out
    /// received deltas itself. See "The transport owns relaying" on
    /// [`Self::start_collaboration_host`].
    pub fn start_collaboration_guest(
        &self,
        snapshot: &[u8],
        outbound: impl Fn(Vec<u8>) + 'static,
    ) -> Result<(), CollabError> {
        let session = CollabSession::from_bytes(snapshot)?;
        self.start_collaboration_guest_with(session, outbound)
    }

    /// [`Self::start_collaboration_guest`] with a session the caller joined from the
    /// host's snapshot (`CollabSession::from_bytes`). **Test seam**, like
    /// [`Self::start_collaboration_host_with`]: a test joins with a pinned yrs client id.
    #[doc(hidden)]
    pub fn start_collaboration_guest_with(
        &self,
        session: CollabSession,
        outbound: impl Fn(Vec<u8>) + 'static,
    ) -> Result<(), CollabError> {
        // Adopt the host's document first, with **no** session attached, so the load
        // is not recorded onto a CRDT — the new session already holds this content,
        // and a session left attached from an earlier document must not have this
        // one written over what its peers share. A read-only editor depends on the
        // same thing: with no session attached, a load is never refused. Then
        // attach the matching session.
        let schema = self.core().schema.clone();
        let doc = session.projected_doc(&schema)?;
        self.core_mut().collab = None;
        // Not capped (#1214): the shared document is peers' edits, not a load.
        self.load_doc_inner(doc, false);
        self.core_mut().collab = Some(CollabBridge::new(session, Box::new(outbound)));
        Ok(())
    }

    /// Integrate a remote `delta` from a peer: merge it into the CRDT, rebuild the
    /// model from the *converged* CRDT, and re-project the host. The change is
    /// applied as a non-undoable, remote-origin transaction and is **not**
    /// re-broadcast (it is already in the shared CRDT). Returns whether the document
    /// changed. A no-op (returns `false`) if this editor isn't collaborating.
    ///
    /// Must run on the main thread — a network transport should marshal received
    /// bytes through the platform runtime's `post_remote_delta` rather than
    /// calling this directly off-thread.
    ///
    /// Uses `try_borrow_mut`, so the degenerate case of an `outbound` sink wired to
    /// re-enter the *same* handle (instead of the peer / a channel) degrades to a
    /// no-op `false` rather than panicking.
    pub fn collab_receive(&self, delta: &[u8]) -> bool {
        let changed = self.integrate_remote(delta);
        if changed {
            // No input event brought this change, so a runtime that refreshes the
            // caret from its input handlers has to be told (see
            // `registry::set_overlay_refresher`). After `integrate_remote`
            // returned: the refresh borrows this handle.
            crate::registry::request_overlay_refresh();
        }
        changed
    }

    #[cfg(feature = "collaboration")]
    fn integrate_remote(&self, delta: &[u8]) -> bool {
        // `outbound` runs while a local edit holds this handle's borrow; a self-
        // wired sink that calls back in here would otherwise hit an already-borrowed
        // panic. Fail soft instead.
        let Ok(mut core) = self.try_core_mut() else {
            return false;
        };
        if core.collab.is_none() {
            return false;
        }
        let prev = core.state.clone();
        // `prev` is an owned clone, so borrowing the bridge mutably here doesn't
        // conflict with reading/writing `core.state`/`core.view` afterwards.
        let result = core
            .collab
            .as_mut()
            .unwrap()
            .session
            .integrate_incremental_transaction(&prev, delta);
        match result {
            Ok(Some(tr)) => {
                // A peer's change to a node view's attributes is a
                // `SetNodeAttrStep` (the collab adapter proves it is the same
                // atom), so the view stays mounted.
                let kept = kept_nodes(&tr);
                let mapping = tr.mapping().clone();
                let next = prev.apply(tr);
                core.carry_anchors_remote(&prev, &next.doc);
                if !prev.doc.same_ref(&next.doc) {
                    core.carry_caret_hint(&prev, &next);
                }
                core.note_selection(&prev.selection, &next.selection);
                core.state = next.clone();
                if let Some(view) = core.view.as_mut() {
                    view.set_kept(kept);
                    view.set_remote_mapping(mapping);
                    view.update_dom(&prev, &next);
                }
                true
            }
            Ok(None) => false,
            Err(e) => {
                if let Some(bridge) = core.collab.as_mut() {
                    bridge.last_error = Some(e);
                }
                false
            }
        }
    }

    /// Whether this editor currently has a collaboration session attached.
    pub fn is_collaborating(&self) -> bool {
        self.core().collab.is_some()
    }

    /// Whether the attached collaboration session is **poisoned** (issue #196): an
    /// inbound delta left the shared CRDT unprojectable with nothing pending that
    /// could cure it, so the session cannot receive — and, to avoid the silent
    /// one-way partition of a replica that keeps broadcasting while receiving
    /// nothing, it refuses to send as well. Every affected call keeps failing with
    /// the sticky [`CollabError::SessionPoisoned`] (also visible via
    /// [`Self::collab_take_error`], which distinguishes it from a transient error
    /// such as an undecodable blob or a rebuild still waiting on a missing
    /// dependency). Inbound deltas are still *attempted*: one that makes the shared
    /// document rebuildable again clears the poison. `false` when not collaborating.
    ///
    /// Recovery in practice: [`Self::stop_collaboration`], then rejoin from a
    /// healthy peer's snapshot ([`Self::start_collaboration_guest`]).
    ///
    /// Uses `try_borrow` — soft, like [`Self::collab_receive`] — so an `outbound`
    /// sink may call it without panicking. While the handle is borrowed a local edit
    /// is being committed, which a poisoned session refuses, so `false` is the
    /// consistent answer for that window.
    pub fn is_collaboration_poisoned(&self) -> bool {
        let Ok(core) = self.inner.try_borrow() else {
            return false;
        };
        core.collab
            .as_ref()
            .is_some_and(|b| b.session.is_poisoned())
    }

    /// A snapshot of the shared document **as it stands now**, for a *late-joining*
    /// peer: hand it to a new guest's [`Self::start_collaboration_guest`] so they
    /// adopt the current content (not just the host's original document). `None`
    /// when this editor isn't collaborating. Assumes a reliable, ordered delta
    /// transport between existing peers; for lossy / out-of-order reconciliation
    /// exchange state vectors instead ([`Self::collab_state_vector`] /
    /// [`Self::collab_sync_diff`]).
    pub fn collab_snapshot(&self) -> Option<Vec<u8>> {
        self.core().collab.as_ref().map(|b| b.session.snapshot())
    }

    /// This editor's CRDT **state vector**: the opaque summary of what it has seen, to
    /// hand a peer so the peer can answer with [`Self::collab_sync_diff`]. `None` when
    /// this editor isn't collaborating.
    ///
    /// Together with `collab_sync_diff` this is the reconciliation path, for transports
    /// where the delta broadcast (`outbound` + [`Self::collab_receive`]) cannot be
    /// trusted to deliver every message exactly once: an HTTP poll, a reconnecting
    /// socket, a peer that was offline. A client sends its state vector, the server
    /// replies with the diff plus its own state vector, the client applies that and
    /// answers with the diff the server is missing — one round trip per direction, with
    /// no per-peer protocol state to keep on either side.
    ///
    /// **Not a convergence test.** A state vector summarises *insertions*; deletions
    /// (and mark removals, which the engine implements by deleting format markers) do
    /// not appear in it, so two editors can hold different documents behind equal state
    /// vectors. Its job is to *request* a diff — and the diff it gets back always
    /// carries the deletions in full.
    ///
    /// Uses `try_borrow` and yields `None` if the handle is already borrowed, for the
    /// same reason [`Self::collab_receive`] does.
    pub fn collab_state_vector(&self) -> Option<Vec<u8>> {
        let core = self.inner.try_borrow().ok()?;
        core.collab.as_ref().map(|b| b.session.state_vector())
    }

    /// The update a peer whose state vector is `remote_state_vector` is missing — feed
    /// it to that peer's [`Self::collab_receive`]. `None` when this editor isn't
    /// collaborating, or when the bytes are not a decodable state vector (the error is
    /// recorded and readable via [`Self::collab_take_error`]).
    ///
    /// This is a pure read of the CRDT: it broadcasts nothing and leaves both the
    /// document and the broadcast watermark untouched, so a caller may answer any number
    /// of peers without disturbing the live delta stream.
    pub fn collab_sync_diff(&self, remote_state_vector: &[u8]) -> Option<Vec<u8>> {
        let mut core = self.try_core_mut().ok()?;
        let bridge = core.collab.as_mut()?;
        match bridge.session.sync_diff(remote_state_vector) {
            Ok(diff) => Some(diff),
            Err(e) => {
                bridge.last_error = Some(e);
                None
            }
        }
    }

    /// A **sticky index** for position `pos`: an address that follows the text through
    /// every later edit, local or a peer's, on this replica or any other. For deep
    /// links into a collaborating document. Resolve it with
    /// [`Self::collab_resolve_sticky`].
    ///
    /// The bytes are a plain yrs `StickyIndex` (v1 encoding) on the `text` of the
    /// textblock holding `pos`, so an app can also resolve them with yrs alone, against
    /// any replica of the shared document, without an editor
    /// ([`rinch_editor_collab::CollabDoc::sticky_index`] documents the encoding).
    ///
    /// `None` when not collaborating; when `pos` is not inside a textblock (between
    /// blocks); for the starter paragraph of a shared document with no blocks, which
    /// has nothing in the CRDT behind it; and while the session is
    /// [poisoned](Self::is_collaboration_poisoned) or
    /// [stalled](Self::collab_outbound_stall). Uses `try_borrow`, like
    /// [`Self::collab_state_vector`].
    pub fn collab_sticky_index(&self, pos: Pos) -> Option<Vec<u8>> {
        let core = self.inner.try_borrow().ok()?;
        let bridge = core.collab.as_ref()?;
        bridge.session.sticky_index(&core.state.doc, pos)
    }

    /// Where a sticky index from [`Self::collab_sticky_index`] (made by this editor or
    /// by any peer's) points now, as a position in this editor's document.
    ///
    /// When its character was deleted it resolves to where that character was. `None`
    /// when not collaborating; when the bytes do not decode or name an item this
    /// replica has not seen (a peer's edit not merged yet); when its **block** was
    /// deleted (a block joined into the one before it counts: its text moves as a new
    /// insert); and while the session is poisoned or stalled.
    pub fn collab_resolve_sticky(&self, bytes: &[u8]) -> Option<Pos> {
        let core = self.inner.try_borrow().ok()?;
        let bridge = core.collab.as_ref()?;
        bridge.session.resolve_sticky(&core.state.doc, bytes)
    }

    /// A sticky index for position `pos` that holds the character `assoc` names and
    /// **survives the deletion of its block**: [`Self::collab_locate_sticky`] then
    /// answers the place the block stood, marked
    /// [`displaced`](StickyLocation::displaced). The bytes start with the plain yrs
    /// `StickyIndex` [`Self::collab_sticky_index`] describes (with `assoc`'s
    /// association), followed by a checked trailer; treat them as opaque (store,
    /// compare, hand back)
    /// ([`rinch_editor_collab::CollabDoc::sticky_index_with`] documents them).
    ///
    /// For a range (a comment's span), make the start with [`StickyAssoc::After`] and
    /// the end with [`StickyAssoc::Before`]. `None` in the same cases as
    /// [`Self::collab_sticky_index`].
    pub fn collab_sticky_index_with(&self, pos: Pos, assoc: StickyAssoc) -> Option<Vec<u8>> {
        let core = self.inner.try_borrow().ok()?;
        let bridge = core.collab.as_ref()?;
        bridge
            .session
            .sticky_index_with(&core.state.doc, pos, assoc)
    }

    /// Where a sticky index (from [`Self::collab_sticky_index_with`] or
    /// [`Self::collab_sticky_index`], made by this editor or a peer's) points now in
    /// this editor's document, with the path of its textblock (`path[0]` is the
    /// top-level block) and whether its block is gone and the position is only the
    /// nearest place to where it stood
    /// ([`rinch_editor_collab::CollabDoc::locate_sticky`]). `None` in the same cases as
    /// [`Self::collab_resolve_sticky`], except that a deleted block's index from
    /// [`Self::collab_sticky_index_with`] is answered.
    pub fn collab_locate_sticky(&self, bytes: &[u8]) -> Option<StickyLocation> {
        let core = self.inner.try_borrow().ok()?;
        let bridge = core.collab.as_ref()?;
        bridge.session.locate_sticky(&core.state.doc, bytes)
    }

    /// [`Self::collab_locate_sticky`] for every index of `anchors`, in order, in one
    /// walk of the shared document: for resolving many anchors at once (every comment
    /// of a document on each layout). One answer per index; every answer is `None`
    /// when not collaborating, while the editor is borrowed, and while the session is
    /// poisoned or stalled.
    pub fn collab_locate_sticky_many(&self, anchors: &[&[u8]]) -> Vec<Option<StickyLocation>> {
        let Ok(core) = self.inner.try_borrow() else {
            return vec![None; anchors.len()];
        };
        let Some(bridge) = core.collab.as_ref() else {
            return vec![None; anchors.len()];
        };
        bridge.session.locate_sticky_many(&core.state.doc, anchors)
    }

    /// Detach the collaboration session (stop projecting and broadcasting). The
    /// document is unchanged; subsequent edits are local-only again.
    pub fn stop_collaboration(&self) {
        self.core_mut().collab = None;
    }

    /// Why this editor's **outbound** collaboration is currently refusing, if it is
    /// (issue #220): a local edit outside the staged A22 scope — a pasted ragged
    /// table, a task list — cannot be projected onto the CRDT, so this edit and every
    /// one after it stays local until that content is removed.
    ///
    /// Unlike [`Self::collab_take_error`] this does **not** clear: it stays `Some` for
    /// as long as the condition holds, so it is what an app should drive a persistent
    /// "not syncing — remove the task list to resume" indicator from. It clears itself the
    /// moment a local edit projects again, and that same edit broadcasts everything
    /// that accumulated meanwhile.
    ///
    /// This is deliberately **not** [poison](Self::is_collaboration_poisoned): the
    /// shared document is healthy throughout, inbound integration keeps working, and
    /// recovery needs no rejoin. `None` when not collaborating.
    ///
    /// Uses `try_borrow` — soft, like [`Self::collab_receive`] — so an `outbound`
    /// callback may call it re-entrantly.
    ///
    /// [`CollabError::OversizedTable`] is the **freeze** (see
    /// [`Self::collab_oversized_tables`]): reported as soon as such a table arrives,
    /// and cleared only by [`Self::collab_delete_oversized_table`] or a peer removing
    /// or shrinking the table, not by an edit.
    pub fn collab_outbound_stall(&self) -> Option<CollabError> {
        let core = self.inner.try_borrow().ok()?;
        core.collab
            .as_ref()
            .and_then(|b| b.session.outbound_stall().cloned())
    }

    /// The tables in the shared document too large to read: a peer grew each past
    /// the read's budget, and the model shows it as a one-cell placeholder. While
    /// any exists, the editor is **frozen**: every local edit is refused, as a
    /// [read-only](Self::set_read_only) editor refuses it (typing, commands, paste,
    /// undo and loads answer `false`; the caret, selection and copy still work), and
    /// [`Self::collab_outbound_stall`] reports [`CollabError::OversizedTable`] naming
    /// the tables — because no diff against a placeholder can be trusted not to delete
    /// real content, and an edit kept to ship later is lost to the next inbound change.
    /// An app should say so ("a collaborator added a table too large to load — delete
    /// it to keep editing"). Cure it with [`Self::collab_delete_oversized_table`], or wait for a
    /// peer to delete or shrink the table. Empty when not collaborating.
    pub fn collab_oversized_tables(&self) -> Vec<OversizedTable> {
        let Ok(core) = self.inner.try_borrow() else {
            return Vec::new();
        };
        core.collab
            .as_ref()
            .map(|b| b.session.oversized_tables())
            .unwrap_or_default()
    }

    /// Delete the table too large to read named `id` (an [`OversizedTable::id`] from
    /// [`Self::collab_oversized_tables`]) from the shared document, for every peer:
    /// the one change that may touch it, and it works while frozen (it is not a model
    /// edit). Its placeholder leaves the model, and once no such table is left editing
    /// works again. Not undoable. `Ok(false)` when `id` is not in
    /// [`Self::collab_oversized_tables`] **now** (a peer deleted the table, or shrank
    /// it back into a readable one) or this editor is not collaborating.
    #[cfg(feature = "collaboration")]
    pub fn collab_delete_oversized_table(&self, id: &str) -> Result<bool, CollabError> {
        let mut core = self.core_mut();
        // A read-only editor writes nothing to the shared document (as its loads).
        if core.collab.is_none() || core.read_only {
            return Ok(false);
        }
        let prev = core.state.clone();
        let bridge = core.collab.as_mut().unwrap();
        let Some(next) = bridge.session.delete_oversized_table(&prev, id)? else {
            return Ok(false);
        };
        match bridge.session.save_incremental() {
            Ok(delta) if !delta.is_empty() => untracked_handler(|| (bridge.outbound)(delta)),
            Ok(_) => {}
            Err(e) => bridge.last_error = Some(e),
        }
        if !prev.doc.same_ref(&next.doc) {
            core.carry_anchors_remote(&prev, &next.doc);
            core.carry_caret_hint(&prev, &next);
            core.note_selection(&prev.selection, &next.selection);
            core.state = next.clone();
            if let Some(view) = core.view.as_mut() {
                view.update_dom(&prev, &next);
            }
        }
        drop(core);
        // The freeze may have lifted: a runtime keeping per-editor input state (the
        // web capture field's `readonly`) hears it here, as from `set_read_only`.
        crate::registry::request_overlay_refresh();
        Ok(true)
    }

    /// Take (and clear) the most recent collaboration error — e.g. an edit outside
    /// the staged flat-text scope that could not be projected (design A22). `None`
    /// when not collaborating or no error is pending.
    ///
    /// A [`CollabError::SessionPoisoned`] here is **not** a one-off: the session is
    /// dead in both directions and every affected call re-fails with it (taking it
    /// does not un-poison — see [`Self::is_collaboration_poisoned`]). Anything else
    /// (an undecodable blob, an out-of-scope local edit) is transient: the session
    /// keeps collaborating.
    ///
    /// For an out-of-scope local edit specifically, taking the error does not end the
    /// condition — outbound stays stalled until the content is removed. Use
    /// [`Self::collab_outbound_stall`] for the *state*; this is the one-shot event.
    pub fn collab_take_error(&self) -> Option<CollabError> {
        self.core_mut()
            .collab
            .as_mut()
            .and_then(|b| b.last_error.take())
    }
}

// ─── Node views ──────────────────────────────────────────────────────────────
