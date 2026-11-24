//! Content edits: typed text, document loads, the clipboard, paste and
//! links.

use super::*;

impl EditorHandle {
    /// Insert `text`, replacing the current selection — the keyboard text-input
    /// path. A flat insert handles a text range or an *inline* node selection
    /// directly (text is valid inline content); for a *block* node selection (a
    /// selected horizontal rule, where a bare text node isn't valid `doc` content)
    /// it deletes the node first, then inserts the text at the resulting cursor
    /// (ProseMirror `replaceSelection` for text input). Returns whether the
    /// document changed.
    pub fn insert_text(&self, text: &str) -> bool {
        // Typing over a cell selection first clears the selected cells and collapses
        // the cursor into the top-left cell (PM `deleteCellSelection`); the text then
        // replaces the cells' content. Without this, the generic insert below would
        // splice the cell selection's coarse range and corrupt the table.
        if matches!(self.selection(), Selection::Cell(_)) {
            self.command("deleteCellSelection");
        }
        self.dispatch(
            |state| {
                // Markdown input rules (M3): at a collapsed cursor, a just-typed character
                // may complete a shortcut (`**bold**`, `# `, `[ ] ` → task list, …) and
                // rewrite the text instead of inserting it verbatim. Mirrors ProseMirror's
                // `inputRules` plugin, which runs before the plain text insert. Paste and IME
                // preedit don't reach here; an IME *commit* does (`ime_commit`), which is the
                // intended behaviour.
                if state.selection.is_empty() {
                    let pos = state.selection.from().0;
                    if let Some(tr) = apply_input_rules(state, state.input_rules(), pos, text) {
                        return Some(tr);
                    }
                }
                let mut tr = state.tr();
                if tr.insert_text(text).is_ok() {
                    return Some(tr);
                }
                // The flat insert couldn't replace the selection in place (a block node
                // selection). Delete it, then insert the text at the collapsed cursor.
                let mut tr = state.tr();
                tr.delete_selection().ok()?;
                tr.insert_text(text).ok()?;
                Some(tr)
            },
            true,
        )
    }

    /// Toggle the `checked` state of the task item enclosing document position `pos`
    /// — the checkbox-click path. Returns whether a task item was found and toggled
    /// (`false` if `pos` is not inside a task item, so the caller can fall back to a
    /// normal caret placement). Does not move the selection: ticking a box shouldn't
    /// jump the text cursor.
    pub fn toggle_task_checked_at(&self, pos: usize) -> bool {
        self.update(|state| {
            let r = state.doc.resolve(Pos(pos)).ok()?;
            // The nearest task_item ancestor (walk up from the resolved depth).
            let depth = (1..=r.depth())
                .rev()
                .find(|&d| r.node(d).type_name() == "task_item")?;
            let item_pos = r.before(depth)?;
            let checked = r.node(depth).attrs().get_bool("checked").unwrap_or(false);
            let mut tr = state.tr();
            tr.set_node_attr(
                item_pos,
                "checked",
                rinch_editor_core::AttrValue::Bool(!checked),
            )
            .ok()?;
            Some(tr)
        })
    }

    /// Replace the document with `doc`, resetting selection and history (a fresh
    /// load, not an undoable edit). The host diffs from the old content to the new,
    /// so unchanged leading blocks are reused. Works before focus.
    ///
    /// A **block-less** `doc` (zero children — e.g. from parsing empty/whitespace
    /// HTML, since `Schema::branch` does not fill required content) is repaired to a
    /// single empty paragraph, so the editor is never left with no textblock to
    /// render or place a caret in.
    ///
    /// A table cell's `colspan` past 1000 is capped at 1000 and its `rowspan`
    /// past 65534 at 65534, as the HTML import and Chrome read them (#1214,
    /// #1244, `rinch_editor_core::tables::cap_spans`). The same cap applies to
    /// the document [`Self::new`] is given. Edits (commands, [`Self::update`]) are not capped, and neither is
    /// the shared document a [collaboration guest](Self::start_collaboration_guest)
    /// adopts: that is its peers' edits.
    ///
    /// A [read-only](Self::set_read_only) editor still loads — that is how it gets
    /// a document to show — **except while collaborating**, where a load is a
    /// write to the shared document and is refused like any other (see
    /// [`Self::set_read_only`]); [`Self::load_html`] reports that as `false`.
    pub fn load_doc(&self, doc: Node) {
        self.load_doc_checked(doc);
    }

    /// [`Self::load_doc`], answering whether the document was loaded (`false`:
    /// refused by a read-only, collaborating editor).
    fn load_doc_checked(&self, doc: Node) -> bool {
        self.load_doc_inner(doc, true)
    }

    /// Install `doc` as a load does. `doc` is first re-homed onto this
    /// editor's own schema (`Node::rebind`, #440): `load_doc`'s documented use
    /// is moving a document between editors, each minting its own `Schema`
    /// (and therefore its own `NodeType`/`MarkType` handles, compared by
    /// `Rc::ptr_eq`), so installing a caller-supplied `Node` verbatim leaves
    /// its marks and node types unrecognizable to this editor — `toggleBold`
    /// over already-bold adopted text then takes the *add* branch instead of
    /// *remove*, duplicating the mark. A document naming a node or mark type
    /// this schema does not know by name refuses the load (returns `false`),
    /// same as a `load_html` parse failure. With `cap`, a table cell's
    /// `colspan` past 1000 and `rowspan` past 65534 are capped, as the HTML
    /// import reads them (#1214, #1244, `tables::cap_spans`). A collaboration guest adopts the shared
    /// document with `cap: false`: what the CRDT holds is its peers' edits,
    /// which are not capped, and a capped model would differ from its own
    /// projection and write the cap back to the peers as an edit nobody made.
    /// (That guest document is already projected through this editor's own
    /// schema — `CollabSession::projected_doc` — so its `rebind` here is the
    /// cheap already-this-schema fast path, not a rebuild.)
    pub(super) fn load_doc_inner(&self, doc: Node, cap: bool) -> bool {
        let mut core = self.core_mut();
        let Ok(doc) = doc.rebind(&core.schema) else {
            return false;
        };
        let doc = if doc.child_count() == 0 {
            empty_paragraph_doc(&core.schema).unwrap_or(doc)
        } else if cap {
            rinch_editor_core::tables::cap_spans(&doc)
        } else {
            doc
        };
        let prev = core.state.clone();
        let next = EditorState::create(core.schema.clone(), doc, core.plugins.clone());
        // Deliberately does not fire `on_change`: loading a document is a
        // programmatic replace, not a user edit. Firing here would make an
        // autosave consumer immediately re-save freshly loaded content.
        //
        // No mapping: the new document is unrelated to the old one, so every
        // outstanding `SelectionAnchor` is invalidated rather than remapped onto
        // whatever now happens to sit at those offsets.
        // A load is the app replacing the document, not the user moving the
        // caret: it scrolls nothing.
        core.commit(prev, next, None, Scroll::No, Vec::new())
            .is_some()
    }

    /// Parse `html` (schema-whitelisted) and load it as the document. Empty or
    /// whitespace-only `html` loads a single empty paragraph (via [`Self::load_doc`]),
    /// not a block-less doc. Returns `false` if `html` fails to parse at all, or if
    /// the editor is read-only and collaborating (see [`Self::load_doc`]).
    pub fn load_html(&self, html: &str) -> bool {
        let schema = self.core().schema.clone();
        let Ok(slice) = slice_from_html(&schema, html) else {
            return false;
        };
        let Ok(doc) = schema.branch("doc", slice.content.clone()) else {
            return false;
        };
        self.load_doc_checked(doc)
    }

    // ── Clipboard (copy / cut / paste) ──────────────────────────────────────
    //
    // The model side of the clipboard: serialize the current selection to the
    // `(text/html, text/plain)` pair to put on the clipboard, and replace the
    // selection with a parsed HTML or plain-text payload on paste. The actual
    // clipboard I/O (`copy_html`/`paste_html`) lives app-side behind the
    // `clipboard` feature; these methods keep all model/serialize knowledge here.

    /// The current selection serialized as `(html, plain_text)` for the clipboard,
    /// or `None` when the selection is empty (nothing to copy). The HTML is the
    /// rich payload (round-trips back via [`Self::replace_selection_with_html`]);
    /// the plain text is the `text/plain` alternative.
    ///
    /// A selection inside a list is copied with the list around its items
    /// (`<ol start="3">`, a task list with its checkboxes): content that is
    /// nothing without the node it was in keeps it ([`clipboard_slice`]). A
    /// selection across the cells of one table is a cell selection (#1847),
    /// which copies as a table of the selected cells, whole
    /// ([`cell_selection_slice`](rinch_editor_core::commands::table_ops::cell_selection_slice)),
    /// as ProseMirror copies one.
    pub fn selection_clipboard(&self) -> Option<(String, String)> {
        let core = self.core();
        let sel = &core.state.selection;
        if sel.is_empty() {
            return None;
        }
        // A cell selection's `from()..to()` is not a range of content: it
        // copies as a table of the selected cells.
        let slice = match sel {
            Selection::Cell(_) => {
                rinch_editor_core::commands::table_ops::cell_selection_slice(&core.state)?
            }
            _ => clipboard_slice(&core.state.doc, sel.from().0, sel.to().0).ok()?,
        };
        Some((slice_to_html(&slice), slice_to_text(&slice)))
    }

    /// Paste `paste` over the current selection: the entry point for every
    /// paste the platform reports as a paste event, which desktop (Ctrl+V,
    /// Ctrl+Shift+V, the context menu's Paste) and the web (the `paste` event)
    /// both call. A mobile keyboard's clipboard chip inserts text directly and
    /// does not come through here. Returns whether anything was applied.
    ///
    /// Every plugin is offered the paste first ([`Plugin::handle_paste`], in
    /// plugin order): the first to return a transaction claims it, and that
    /// transaction is applied as the paste. No claim leaves the default:
    /// `text/html` parsed into structure ([`Self::replace_selection_with_html`]),
    /// else, when the html is absent, parses to nothing or has no valid place,
    /// `text/plain` one paragraph per line
    /// ([`Self::replace_selection_with_text`]). In a code block the
    /// `text/plain` flavour is used whenever there is one, and goes in as it
    /// is. An empty `paste` does nothing and asks no plugin.
    ///
    /// Either way the paste is one transaction, so one undo step; it is the
    /// user's input, so it scrolls the caret into view, a collaborating editor
    /// records and broadcasts it, and [`on_change`](Self::on_change) fires if the
    /// document changed. A [read-only](Self::set_read_only) editor refuses it,
    /// a plugin's transaction included, and a refused claim does **not** fall
    /// through to the default.
    ///
    /// The plugins run with the editor borrowed, as a command does, so one must
    /// not call back into this handle.
    pub fn paste(&self, paste: &PasteContent) -> bool {
        if paste.is_empty() {
            return false;
        }
        let mut claimed = false;
        let applied = self.dispatch(
            |state| {
                let tr = state.handle_paste(paste);
                claimed = tr.is_some();
                tr
            },
            true,
        );
        if claimed {
            return applied;
        }
        // In a code block a paste is its text, as ProseMirror's is.
        if let Some(text) = &paste.text
            && self.selection_in_code()
        {
            return self.replace_selection_with_text(text);
        }
        if let Some(html) = &paste.html
            && self.replace_selection_with_html(html)
        {
            return true;
        }
        paste
            .text
            .as_deref()
            .is_some_and(|text| self.replace_selection_with_text(text))
    }

    /// Replace the current selection with a parsed (schema-whitelisted) HTML
    /// payload. The rich half of the default paste: a user's paste goes through
    /// [`Self::paste`], which offers it to the plugins first. Returns whether
    /// anything was inserted.
    ///
    /// The content is **fitted** to where it lands
    /// ([`Transaction::replace_selection`](rinch_editor_core::Transaction::replace_selection),
    /// ProseMirror's `replaceSelection`): a list, a table or a rule has no
    /// place inside the textblock the caret is in, so the textblock is closed
    /// or split around it rather than the paste being refused.
    ///
    /// It reads as a paste does ([`slice_from_pasted_html`]): an image keeps
    /// its `data-*` attributes only when rinch's own copy-out wrote them
    /// ([`selection_clipboard`](Self::selection_clipboard)), so another
    /// application's are dropped. [`Self::load_html`] keeps them all. An app
    /// inserting its own markup here keeps its data attributes by putting
    /// [`CLIPBOARD_MARK`](rinch_editor_core::serialize::CLIPBOARD_MARK)
    /// (`data-rinch-clip=""`) on the element that carries them, as the
    /// copy-out does; or it builds the node and inserts it with
    /// [`Self::update`].
    pub fn replace_selection_with_html(&self, html: &str) -> bool {
        let schema = self.core().schema.clone();
        match slice_from_pasted_html(&schema, html) {
            Ok(slice) if slice.content.child_count() > 0 => self.replace_selection_slice(slice),
            _ => false,
        }
    }

    /// Whether the selection starts in a node that holds code.
    fn selection_in_code(&self) -> bool {
        let core = self.core();
        core.state
            .doc
            .resolve(core.state.selection.from())
            .is_ok_and(|pos| pos.parent().node_type().spec().code)
    }

    /// Toggle a `link` mark with `href` across the current selection — the
    /// imperative "make link" / "unlink" path, mirroring [`insert_image`](Self::insert_image).
    ///
    /// Matches the `toggle_link` command semantics: if the selection already
    /// carries a link it is removed (`href` ignored), otherwise the link is added.
    /// A collapsed cursor is a no-op (a link needs a range) — returns `false`.
    /// Because a link takes a runtime `href`, it can't live in the arg-less string
    /// command registry, so — like `insert_image` — it gets a dedicated method
    /// rather than a `command("...")` name. To read an existing link's target for
    /// an edit dialog, use [`active_link_href`](Self::active_link_href).
    pub fn toggle_link(&self, href: &str) -> bool {
        let cmd = rinch_editor_core::commands::toggle_link(href.to_string());
        let mut core = self.core_mut();
        let Some((next, mapping)) = run_stamped(&core.state, &cmd) else {
            return false;
        };
        let prev = core.state.clone();
        let Some(doc_changed) =
            core.commit(prev, next, Some(&mapping), Scroll::IfChanged, Vec::new())
        else {
            return false;
        };
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }

    /// Replace the current selection with plain text (one paragraph per line).
    /// The plain half of the default paste: a user's paste goes through
    /// [`Self::paste`], which offers it to the plugins first. Returns whether
    /// anything was inserted.
    ///
    /// In a code block the text goes in as it is, line breaks included, and
    /// stays in the block.
    pub fn replace_selection_with_text(&self, text: &str) -> bool {
        let schema = self.core().schema.clone();
        if self.selection_in_code() {
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            return match schema.text(&text) {
                Ok(node) if !text.is_empty() => {
                    self.replace_selection_slice(Slice::from_fragment(Fragment::from_node(node)))
                }
                _ => false,
            };
        }
        match slice_from_text(&schema, text) {
            Ok(slice) if slice.content.child_count() > 0 => self.replace_selection_slice(slice),
            _ => false,
        }
    }

    /// Replace the selection with `slice` (the shared paste mechanism), fitted
    /// to the range, the caret left after the inserted content.
    ///
    /// Over a **cell selection** the selected cells are cleared and the
    /// content goes into the top-left one, in the same transaction: what
    /// typing over a cell selection does ([`insert_text`](Self::insert_text)),
    /// and what ProseMirror's `CellSelection.replace` does with content that
    /// is not table cells. (A cell selection's `from()..to()` is not a range
    /// of content: replacing it took cells out of their rows.)
    fn replace_selection_slice(&self, slice: Slice) -> bool {
        self.dispatch(
            move |state| {
                let mut tr = if matches!(state.selection, Selection::Cell(_)) {
                    rinch_editor_core::commands::table_ops::clear_cells(state)?
                } else {
                    state.tr()
                };
                tr.replace_selection(slice).ok()?;
                Some(tr)
            },
            true,
        )
    }
}
