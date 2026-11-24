//! Editor switches: the placeholder, read-only and dark mode.

use super::*;

impl EditorHandle {
    /// Show `text` while the editor is empty (#2210) — the analogue of a
    /// `<textarea placeholder>`, and as live: a later call changes the shown
    /// text at once, with the document and its undo history untouched. Empty
    /// text shows none. Styled by the `.rinch-editor-placeholder` rule
    /// (`--rinch-editor-placeholder-color`), and hidden while read-only. The
    /// `Editor { placeholder: … }` prop calls this at mount.
    ///
    /// The first non-empty call installs a
    /// [`PlaceholderPlugin`] through [`add_plugin`](Self::add_plugin), which
    /// restarts the undo history, so make it before content is typed (the
    /// prop does); every later call only changes the text. Returns `false`,
    /// changing nothing, when a placeholder plugin the app added itself holds
    /// the slot.
    pub fn set_placeholder(&self, text: &str) -> bool {
        let installed = self.core().placeholder.clone();
        match installed {
            Some(plugin) => {
                if *plugin.text() == *text {
                    return true;
                }
                plugin.set_text(text);
                let mut core = self.core_mut();
                let core = &mut *core;
                if let Some(view) = core.view.as_mut() {
                    view.refresh_decorations(&core.state);
                }
                true
            }
            None if text.is_empty() => true,
            None => {
                let plugin = PlaceholderPlugin::new(text);
                if !self.add_plugin(Rc::new(plugin.clone())) {
                    return false;
                }
                self.core_mut().placeholder = Some(plugin);
                true
            }
        }
    }

    /// The text [`set_placeholder`](Self::set_placeholder) gave, or `""`.
    pub fn placeholder(&self) -> String {
        self.core()
            .placeholder
            .as_ref()
            .map(|p| p.text().to_string())
            .unwrap_or_default()
    }

    /// Make the editor **read-only** (`true`) or editable again (`false`, the
    /// default). Takes effect at once and may be flipped at any time, mounted or
    /// not — the switch lives on the handle, so it survives a re-mount.
    ///
    /// Read-only means what `readonly` means on an `<input>`: the document can be
    /// read, selected and copied, and the person in front of it cannot change it.
    ///
    /// **Refused**, each reporting "not applied" (`false`) and leaving the document,
    /// the undo history and [`on_change`](Self::on_change) untouched: typing and IME
    /// commits ([`insert_text`](Self::insert_text), [`ime_commit`](Self::ime_commit),
    /// [`ime_delete_surrounding`](Self::ime_delete_surrounding)), paste and cut
    /// ([`replace_selection_with_html`](Self::replace_selection_with_html),
    /// [`replace_selection_with_text`](Self::replace_selection_with_text),
    /// [`insert_image`](Self::insert_image)), every document-changing
    /// [`command`](Self::command) — formatting, lists, tables, `undo`/`redo` — and
    /// so every key bound to one ([`dispatch_key`](Self::dispatch_key)),
    /// [`toggle_link`](Self::toggle_link), a task checkbox click
    /// ([`toggle_task_checked_at`](Self::toggle_task_checked_at)) and any
    /// transaction handed to [`update`](Self::update) that changes the document or
    /// sets stored marks. It makes no difference who calls: a toolbar button and a
    /// keystroke go through the same gate. [`can_run`](Self::can_run) answers
    /// `false` for a refused command, and no IME preedit is shown.
    ///
    /// **Still works:** placing and moving the caret, selecting (pointer, keyboard,
    /// `selectAll`), copying ([`selection_clipboard`](Self::selection_clipboard)),
    /// every query, [`load_doc`](Self::load_doc) / [`load_html`](Self::load_html)
    /// (the app replacing the document is how a read-only editor gets one to show)
    /// — and, when collaborating, **everything inbound**:
    /// [`collab_receive`](Self::collab_receive) integrates a peer's delta and
    /// re-projects exactly as in an editable editor, because remote integration
    /// never passes through the gate local edits pass through. A read-only editor
    /// is a live view of a document other people are writing.
    ///
    /// **While collaborating, nothing goes out:** no local change is recorded onto
    /// the CRDT, so `outbound` does not fire. That includes `load_doc` /
    /// `load_html`, which with a session attached are writes to the shared
    /// document and are refused too. (Joining — `start_collaboration_guest` —
    /// adopts the shared document and is not a write.)
    ///
    /// The gate is one check in the single place local changes land (see
    /// `EditorCore::commit`), not a check per input handler, so an input path added
    /// later is read-only by default.
    ///
    /// Switching it **on** drops pending typing state: stored marks (a clicked
    /// "Bold" waiting for text) and any IME preedit. The mounted container carries
    /// `data-pm-readonly="true"` while it is on, for styling; the built-in
    /// stylesheet uses it to hide the empty-editor placeholder, which would
    /// otherwise invite typing.
    pub fn set_read_only(&self, read_only: bool) {
        let mut core = self.core_mut();
        if core.read_only == read_only {
            return;
        }
        if read_only && core.state.stored_marks.is_some() {
            // Still editable here, so this goes through the ordinary path.
            let prev = core.state.clone();
            let mut tr = prev.tr();
            tr.set_stored_marks(None);
            let mapping = tr.mapping().clone();
            let next = prev.apply(tr);
            // Clears stored marks only: nothing moves, so nothing to scroll to.
            core.commit(prev, next, Some(&mapping), Scroll::No, Vec::new());
        }
        core.read_only = read_only;
        core.node_views.read_only.set(read_only);
        if let Some(view) = core.view.as_mut() {
            if read_only {
                view.set_preedit("");
            }
            view.set_read_only(read_only);
            // Every node view's `read_only` memo follows, once the borrow goes.
            view.node_view_flags_changed();
        }
        drop(core);
        // A read-only editor shows no spelling marks; an editable one again
        // is checked again (#2862).
        self.refresh_spellcheck();
        // No input event brought this change; a runtime that keeps per-editor
        // input state in step from its input handlers (the web's capture field)
        // hears about it here. After the borrow is released: the refresh reads
        // this handle.
        crate::registry::request_overlay_refresh();
    }

    /// Whether the editor is [read-only](Self::set_read_only).
    ///
    /// Uses `try_borrow` — soft, like [`Self::collab_receive`] — so it may be asked
    /// from inside a callback the handle invokes. It answers `false` when the handle
    /// is already borrowed, i.e. **fails open**, which is the wrong direction for a
    /// permission question and is why the window matters: no such re-entrant caller
    /// exists today (`on_change` runs with the borrow released, and `outbound`
    /// cannot fire on a read-only editor, which records nothing), so the answer is
    /// reachable only by a future re-entrant one. Do not build an access decision on
    /// it from inside a handle callback — the gate in `EditorCore::commit`, which
    /// reads the flag directly, is what actually refuses an edit.
    pub fn is_read_only(&self) -> bool {
        self.inner.try_borrow().is_ok_and(|core| core.read_only)
    }

    /// Whether every local edit is refused right now: the editor is
    /// [read-only](Self::set_read_only), or frozen by a collaboration freeze
    /// ([`Self::collab_oversized_tables`]). What a platform asks to behave as it does
    /// for a `readonly` field (input method off, no Cut/Paste, the web capture field
    /// `readonly`, printable keys still owned). Soft like [`Self::is_read_only`].
    pub fn refuses_edits(&self) -> bool {
        self.inner.try_borrow().is_ok_and(|core| core.locked())
    }

    /// Switch the editor between the light (default) and dark color schemes of the
    /// built-in stylesheet. A no-op before mount. The app should trigger a repaint
    /// afterward (toolbar/keyboard handlers already do).
    pub fn set_dark_mode(&self, dark: bool) {
        // A DOM write, so program order: run what the caller queued first
        // (`core()` does not, being the query path).
        rinch_core::reactive::flush_pending_effects();
        if let Some(view) = self.core().view.as_ref() {
            view.set_dark_mode(dark);
        }
        // Every node view's `dark_mode` memo follows (when the mutable borrow
        // that queues it is released).
        let registry = self.node_view_registry();
        if registry.dark.replace(dark) != dark
            && let Some(view) = self.core_mut().view.as_mut()
        {
            view.node_view_flags_changed();
        }
    }
}
