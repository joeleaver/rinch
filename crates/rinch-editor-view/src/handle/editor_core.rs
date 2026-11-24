//! `EditorCore`'s own methods: committing transactions, projecting the host,
//! firing the change hooks, and its teardown.

use super::*;

impl Drop for EditorCore {
    /// An editor with a hover callback leaves the count of editors that want
    /// link hover when its last handle goes.
    fn drop(&mut self) {
        if self.on_link_hover.is_some() {
            registry::link_hover_listener_removed();
        }
        if self.on_image_hover.is_some() {
            registry::image_hover_listener_removed();
        }
    }
}

impl EditorCore {
    /// The side of a soft wrap the caret at the selection's head belongs to:
    /// the hint while the selection is the one it came with, else the default.
    pub(super) fn caret_affinity(&self) -> CaretAffinity {
        self.affinity_at(self.state.selection.head())
    }

    /// The side of a soft wrap a caret at `pos` is drawn on: the hint at the
    /// head (while its selection holds), downstream elsewhere — and always
    /// **upstream right before a hard break**. A `hard_break` has no bytes in
    /// the host text, so the positions before and after one share a byte
    /// offset; which line a caret there belongs to is the model's to say, not
    /// the geometry's. Before the break is the end of its line (Chrome draws the
    /// DOM point there), after it the start of the next.
    pub(super) fn stored_marks_at(&self, pos: Pos) -> Option<&[Mark]> {
        // The marks the next typed character takes at the caret: its stored
        // marks, where the caret is (#1426: the caret is drawn where that
        // character lands).
        (self.state.selection.is_empty() && self.state.selection.head() == pos)
            .then_some(self.state.stored_marks.as_deref())
            .flatten()
    }

    pub(super) fn affinity_at(&self, pos: Pos) -> CaretAffinity {
        let before_hard_break = self
            .state
            .doc
            .resolve(pos)
            .ok()
            .and_then(|r| r.node_after())
            .is_some_and(|n| n.type_name() == "hard_break");
        if before_hard_break {
            return CaretAffinity::Upstream;
        }
        match &self.caret_hint {
            Some((sel, affinity))
                if *sel == self.state.selection && pos == self.state.selection.head() =>
            {
                *affinity
            }
            _ => CaretAffinity::Downstream,
        }
    }

    /// Commit a freshly-applied `next` state over `prev`: store it, re-project the
    /// host, and — when collaborating — record the local change onto the CRDT and
    /// broadcast the resulting delta. The single landing spot for every **local**
    /// edit (`update`/`command`/`load_doc`/`insert_image` all funnel through here).
    ///
    /// The remote integration path
    /// ([`EditorHandle::collab_receive`]) deliberately does **not** go through this
    /// helper: a remote change is already in the shared CRDT, so it must be stored
    /// and re-projected *without* recording it back onto the CRDT (which would echo
    /// it to peers and double-apply).
    ///
    /// `None` when the editor is [read-only](EditorHandle::set_read_only) and
    /// refuses the change ([`Self::refuses`]): nothing is stored, projected,
    /// recorded or broadcast, and the caller reports the edit as not applied.
    /// Otherwise whether the **document** changed (a selection-only edit leaves
    /// the same doc `Rc`), which is what drives [`EditorHandle::on_change`].
    ///
    /// `mapping` is the applied transaction's position mapping, which carries any
    /// [`SelectionAnchor`] across the edit. `None` means "there is no
    /// correspondence between the old and new positions" — a whole-document load
    /// — and invalidates every anchor.
    ///
    /// `scroll` says whether this change should bring the caret into view (see
    /// [`ScrollGate`]): [`Scroll::IfChanged`] arms the gate when the document or
    /// the selection changed. A refused change arms nothing.
    ///
    /// `kept` is the transaction's proof of which nodes are the same node with
    /// other attributes ([`kept_nodes`]) — passed in, never left behind: a refused
    /// change must not hand its proof to the next one (review of #1575, F2).
    pub(super) fn commit(
        &mut self,
        prev: EditorState,
        next: EditorState,
        mapping: Option<&Mapping>,
        scroll: Scroll,
        kept: Vec<Kept>,
    ) -> Option<bool> {
        if self.refuses(&prev, &next, mapping.is_none()) {
            return None;
        }
        let doc_changed = !prev.doc.same_ref(&next.doc);
        if scroll == Scroll::IfChanged && (doc_changed || prev.selection != next.selection) {
            self.scroll.arm();
        }
        if doc_changed {
            self.carry_anchors(&next.doc, mapping);
            if self.margins.has_pos_anchors() {
                self.margins.carry(mapping);
            }
            self.carry_reveal(mapping);
            self.carry_caret_hint(&prev, &next);
        }
        // A hint for any selection but the one landing is dead: keeping it would
        // revive it if the selection came back by another route.
        if self
            .caret_hint
            .as_ref()
            .is_some_and(|(sel, _)| *sel != next.selection)
        {
            self.caret_hint = None;
        }
        self.note_selection(&prev.selection, &next.selection);
        self.state = next.clone();
        // The atoms it only set attributes of, each with its node before and after
        // and where it stood: a proof for the collaboration projection. One whose old
        // `Rc` the document held at another place too is a proof only when every
        // other copy is still where it was (`names_one_place`).
        #[cfg(feature = "collaboration")]
        let kept_atoms: Vec<(Node, Node, usize)> = kept
            .iter()
            .filter(|k| names_one_place(k, &prev.doc, &next.doc, mapping))
            .filter_map(|k| Some((k.old.clone(), next.doc.node_at(k.pos)?, k.old_pos)))
            .filter(|(_, n, _)| n.is_leaf() && !n.is_text())
            .collect();
        #[cfg(feature = "collaboration")]
        let kept_nodes: Vec<(Node, Node)> = kept_atoms
            .iter()
            .map(|(o, n, _)| (o.clone(), n.clone()))
            .collect();
        // The inline atoms (a picture) among them: still their CRDT chars
        // (`CollabSession::note_kept_inline_atoms`).
        #[cfg(feature = "collaboration")]
        let kept_inline: Vec<(Node, Node, usize)> = kept_atoms
            .into_iter()
            .filter(|(_, n, _)| n.is_inline())
            .collect();
        let margins_moved = doc_changed && self.margins_wanted();
        if let Some(view) = self.view.as_mut() {
            view.set_kept(kept);
            view.update_dom(&prev, &next);
            // The caret and the highlight are drawn by the post-layout overlay
            // pass, and a selection-only change dirties no DOM, so a runtime
            // that skips a frame with nothing dirty would never run that pass
            // for it (#1001). Owe it; the input paths that run it themselves
            // clear this in the same turn. A document change owes it too while
            // margin anchors listen (#1677): their lines may have moved.
            // Stored marks too: they decide which side of a padded mark the
            // caret is drawn on (#1426).
            if prev.selection != next.selection
                || prev.stored_marks != next.stored_marks
                || margins_moved
            {
                crate::registry::owe_overlay_pass(view.doc_key());
            }
        }
        #[cfg(feature = "collaboration")]
        {
            self.record_local(&prev, &next, &kept_nodes, mapping);
            if !kept_inline.is_empty()
                && let Some(bridge) = self.collab.as_ref()
            {
                bridge
                    .session
                    .note_kept_inline_atoms(&prev.doc, &kept_inline);
            }
        }
        Some(doc_changed)
    }

    /// Carry a pending [`EditorHandle::scroll_into_view`] range across a local
    /// edit (the start maps forward, the end back, so an insertion at either
    /// edge stays outside), or drop it on a load (`mapping` is `None`): a range
    /// in the replaced document names nothing in the new one.
    fn carry_reveal(&mut self, mapping: Option<&Mapping>) {
        let Some((from, to, align)) = self.reveal else {
            return;
        };
        self.reveal = mapping.map(|m| {
            let from = m.map(from.0, 1);
            (Pos(from), Pos(m.map(to.0, -1).max(from)), align)
        });
    }

    /// Carry the caret-affinity hint (#301) across a document change `prev →
    /// next`, as CodeMirror maps a range's `assoc` through changes.
    ///
    /// The hint follows the selection when the edit only **shifted** it: both
    /// ends moved by the same amount, and the head at the same offset in its
    /// textblock, whose content **up to the head** is unchanged — an edit in
    /// another block, a peer typing above, or one typing after the caret in its
    /// own paragraph. The caret's line is decided by what comes before it, so it
    /// still belongs to the same side. (A change of the block's *width* — a
    /// re-indent, a resized table column — can re-wrap it; an `Upstream` caret
    /// that is no longer at a wrap draws where a `Downstream` one would, so
    /// keeping the hint then is harmless.) Any other change drops it: an edit
    /// before the caret in its own block re-wraps it, and typing moves the
    /// caret. Asked by position, not through the
    /// transaction's mapping, because a remote integration rebuilds the document
    /// and has none (see [`EditorHandle::collab_receive`]).
    ///
    /// Called only for a change of document. A selection-only transaction leaves
    /// the hint as it stands: [`EditorHandle::set_selection_with_affinity`]
    /// stores it for the selection the transaction is about to set, and it
    /// applies only if that is the selection that lands.
    pub(super) fn carry_caret_hint(&mut self, prev: &EditorState, next: &EditorState) {
        let Some((sel, affinity)) = self.caret_hint.take() else {
            return;
        };
        if sel == prev.selection && shifted_in_place(prev, next) {
            self.caret_hint = Some((next.selection.clone(), affinity));
        }
    }

    /// Owe [`EditorHandle::on_selection_change`] a call if the stored selection
    /// goes from `prev` to a different `next`. Every place that replaces
    /// `state` calls this: [`Self::commit`] (every local change) and the remote
    /// integration. The call itself is made by [`CoreMutGuard`]'s drop, after
    /// the borrow is released, so a mutation path added later needs nothing
    /// more than to go through `commit`.
    ///
    /// Free when no callback is registered: the comparison is skipped.
    ///
    /// [`CoreMutGuard`]: super::guards::CoreMutGuard
    pub(super) fn note_selection(&mut self, prev: &Selection, next: &Selection) {
        if self.on_selection_change.is_some() && prev != next {
            self.selection_owed = true;
        }
    }

    /// Take the callbacks owed, if any: the selection callback with the
    /// selection to report, and the caret-moved callback. See [`CoreMutGuard`].
    ///
    /// [`CoreMutGuard`]: super::guards::CoreMutGuard
    pub(super) fn take_owed(&mut self) -> Owed {
        let selection = std::mem::take(&mut self.selection_owed)
            .then(|| self.on_selection_change.clone())
            .flatten()
            .map(|cb| (cb, self.state.selection.clone()));
        let caret_moved = std::mem::take(&mut self.caret_moved_owed)
            .then(|| self.on_caret_moved.clone())
            .flatten();
        let node_views = self
            .view
            .as_mut()
            .map(RinchDomEditorView::take_node_view_work)
            .unwrap_or_default();
        // Every path that changes the state ends a mutable borrow here, so a
        // check owed by any of them (a command, an undo, a picture, a peer's
        // edit, a dispatch) is noticed in one place.
        let spell = match self
            .state
            .plugin_state::<crate::spellcheck::SpellState>(crate::spellcheck::SPELLCHECK_KEY)
        {
            Some(st) if st.generation != self.spell_seen => {
                self.spell_seen = st.generation;
                st.activated && !st.dirty.is_empty()
            }
            _ => false,
        };
        Owed {
            selection,
            caret_moved,
            node_views,
            spell,
        }
    }

    /// Whether the read-only switch refuses the local change `prev → next`.
    ///
    /// A **transaction** (typing, a command, paste, IME commit, undo) is refused
    /// if it changes the document, or sets stored marks — the "click Bold, then
    /// type" state, which is an edit in waiting and would light a toolbar button
    /// for text nobody can type. What is left is the selection: placing and
    /// moving the caret, selecting, select-all. Those still apply (clearing
    /// stored marks on the way, as a caret move always does), which is what keeps
    /// a read-only document selectable and copyable.
    ///
    /// A **load** (`is_load`: `load_doc` / `load_html`) is the app replacing the
    /// document rather than the user editing it, and is how a read-only editor
    /// gets something to show, so it applies — unless a collaboration session is
    /// attached. There a load is recorded onto the shared CRDT and broadcast like
    /// any other local edit, which is exactly the write read-only forbids.
    ///
    /// Judged on the states rather than on who is asking, so it holds for every
    /// caller of [`Self::commit`] there is or will be. Remote integration
    /// ([`EditorHandle::collab_receive`]) never reaches `commit` and so is never
    /// asked.
    ///
    /// A collaboration **freeze** ([`EditorHandle::collab_oversized_tables`]: the
    /// shared document holds a table too large to read) refuses exactly what the
    /// switch does, loads included, so the model never runs ahead of the CRDT while
    /// outbound cannot carry it. The freeze is not the switch: `is_read_only` does not
    /// report it, so the other places that read the switch (the OS input method, the
    /// context menu's Cut and Paste, the web capture textarea) do not follow it.
    fn refuses(&self, prev: &EditorState, next: &EditorState, is_load: bool) -> bool {
        if !self.locked() {
            return false;
        }
        if is_load {
            #[cfg(feature = "collaboration")]
            return self.collab.is_some();
            #[cfg(not(feature = "collaboration"))]
            return false;
        }
        !prev.doc.same_ref(&next.doc)
            || (next.stored_marks.is_some() && next.stored_marks != prev.stored_marks)
    }

    /// Whether local document changes are refused: the read-only switch, or a
    /// collaboration freeze (see [`Self::refuses`]).
    pub(super) fn locked(&self) -> bool {
        #[cfg(feature = "collaboration")]
        if self.collab.as_ref().is_some_and(|b| b.session.is_frozen()) {
            return true;
        }
        self.read_only
    }

    /// Carry every live [`SelectionAnchor`] across a document change, so an
    /// asynchronous operation still inserts where the user asked for it.
    ///
    /// With a `mapping`, each anchor is re-mapped and re-resolved against the new
    /// document (`Selection::map` falls back to the nearest valid selection if its
    /// textblock went away). Without one — a `load_doc`, where the new document
    /// has no positional relationship to the old — the anchor is invalidated
    /// instead of being silently pointed at unrelated content. A remote
    /// collaboration change has no mapping either, but is not a load:
    /// [`Self::carry_anchors_remote`] carries anchors across it.
    fn carry_anchors(&self, doc: &Node, mapping: Option<&Mapping>) {
        let mut anchors = self.anchors.borrow_mut();
        if anchors.live.is_empty() {
            return;
        }
        for slot in anchors.live.values_mut() {
            *slot = match (slot.take(), mapping) {
                (Some(sel), Some(mapping)) => Some(sel.map(doc, mapping)),
                (Some(_), None) | (None, _) => None,
            };
        }
    }

    /// Carry every live [`SelectionAnchor`] across a **remote** change, which
    /// brings `prev`'s document to `doc` (the converged one).
    ///
    /// A remote integration rebuilds the document rather than applying mapped
    /// local steps, but the two documents are related block by block, and the
    /// live selection is already carried across on that relation
    /// (`rinch_editor_collab::remote`). An anchor follows the same rule
    /// ([`RemoteCarry`](rinch_editor_collab::RemoteCarry)): outside the blocks
    /// the peer changed it shifts with them, inside a textblock the peer typed
    /// in it keeps its place in the text. It is invalidated only when the block
    /// it sat in is gone, or was split, joined or changed kind: there the place
    /// the person aimed at cannot be named, and guessing would put an insertion
    /// in unrelated content.
    ///
    /// It used to invalidate every anchor on every remote change, which was
    /// survivable while an anchor lived for a clipboard read; an anchor now
    /// lives for an upload (`on_image_input`), and a peer's keystroke anywhere
    /// in the document lost the picture.
    #[cfg(feature = "collaboration")]
    ///
    /// The margin anchors' positions ([`EditorHandle::set_margin_anchors`]) are
    /// carried by the same rule, and a document change owes the margin pass
    /// that reports where their lines moved.
    pub(super) fn carry_anchors_remote(&mut self, prev: &EditorState, doc: &Node) {
        if !prev.doc.same_ref(doc)
            && self.margins_wanted()
            && let Some(view) = self.view.as_ref()
        {
            crate::registry::owe_overlay_pass(view.doc_key());
        }
        let mut anchors = self.anchors.borrow_mut();
        let selections = anchors.live.values().any(Option::is_some);
        let margins = self.margins.has_pos_anchors();
        if !selections && !margins {
            return;
        }
        // `None`: the documents are the same, and every anchor stands.
        let Some(carry) = rinch_editor_collab::RemoteCarry::new(prev, doc) else {
            return;
        };
        for slot in anchors.live.values_mut() {
            *slot = slot.take().and_then(|sel| carry.selection(&sel));
        }
        if margins {
            self.margins.carry_remote(&carry);
        }
    }

    /// Whether a margin pass has an audience: margin anchors are set and a
    /// [`EditorHandle::on_margin_layout`] callback is registered.
    pub(super) fn margins_wanted(&self) -> bool {
        self.on_margin_layout.is_some() && !self.margins.anchors.is_empty()
    }

    /// Project a just-applied local change onto the CRDT and broadcast the delta to
    /// peers. A no-op when not collaborating, or for a selection-only edit (the
    /// document is the same `Rc`, so there is nothing to project).
    #[cfg(feature = "collaboration")]
    ///
    /// `kept` are the leaf block atoms of `next` the change only set attributes
    /// of, and the inline atoms (pictures) likewise ([`kept_nodes`]): any other
    /// leaf block atom put where a different node stood is written as a new CRDT
    /// block, and any other picture as a new CRDT char (same `src` or not —
    /// decision (f)), so a peer's node view for the old one is not handed it
    /// (`CollabSession::record_local_edit`). Each comes with the node it was
    /// before the change, so it matches only that node (review 2 of #1595, G1).
    /// A `load` (no mapping: two documents, not one edited) proves no atom and
    /// keeps the value rule for what the load did not change. A transaction hands
    /// over its mapping, so a block it changed at two places is written place by
    /// place (#1885, `CollabSession::record_local_edit_mapped`).
    fn record_local(
        &mut self,
        prev: &EditorState,
        next: &EditorState,
        kept: &[(Node, Node)],
        mapping: Option<&Mapping>,
    ) {
        let Some(bridge) = self.collab.as_mut() else {
            return;
        };
        if prev.doc.same_ref(&next.doc) {
            return;
        }
        let recorded = match mapping {
            None => bridge
                .session
                .record_local_keeping(next.schema(), &prev.doc, &next.doc, &[]),
            Some(mapping) => bridge.session.record_local_edit_mapped(
                next.schema(),
                &prev.doc,
                &next.doc,
                kept,
                mapping,
            ),
        };
        match recorded {
            // An empty delta means the projection produced no CRDT change at all, so
            // there is nothing for peers to apply.
            Ok(()) => match bridge.session.save_incremental() {
                // App code, run inside whatever effect made the edit: untracked
                // (#948). The core guard suspends tracking already; this keeps
                // the sink untracked should it ever be reached from elsewhere.
                Ok(delta) if !delta.is_empty() => untracked_handler(|| (bridge.outbound)(delta)),
                Ok(_) => {}
                Err(e) => bridge.last_error = Some(e),
            },
            // Design A22 fail-loud: an edit outside the staged flat-text scope
            // (a table, a nested block) cannot be projected. Surface it rather than
            // silently diverging.
            //
            // Outbound is now **stalled** (issue #220): this edit and every one after
            // it stays local until the offending content is removed, at which point
            // the session re-bases on the CRDT and the whole backlog broadcasts at
            // once. `EditorHandle::collab_outbound_stall` is the flag to render that
            // with — the error names the content to remove.
            Err(e) => bridge.last_error = Some(e),
        }
    }
}
