//! [`EditorHandle`] — the imperative editor API for app/component code (design
//! A7), the successor to the deleted `with_active_ce_api`/`NodeHandle::with_ce_api`
//! surface.
//!
//! A handle owns the authoritative [`EditorState`] **and** its desktop projection
//! ([`RinchDomEditorView`]). Every mutation runs through `EditorState::apply` and
//! then re-projects the host via the view's phase-1 `update_dom` — there is one
//! mutation path and the host is always derived from the model (design §6). The
//! handle is cheap to [`Clone`] (an `Rc`), so a component can hand it to toolbar
//! buttons and the runtime alike. It works **before focus** — `load_doc`/`command`
//! operate on the owned state whether or not the editor is focused.
//!
//! Caret geometry (phase-2 `update_caret`) is driven by the runtime *after* layout
//! via [`EditorHandle::update_caret`]; in a headless context it is a no-op.

use std::cell::RefCell;
use std::fmt;
use std::rc::{Rc, Weak};

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_editor_core::commands::{current_block_type, in_node_type, is_mark_active, marks_at};
use rinch_editor_core::model::{Fragment, Slice};
use rinch_editor_core::serialize::{
    slice_from_html, slice_from_text, slice_to_html, slice_to_text,
};
use rinch_editor_core::{
    CursorMotion, EditorState, EditorView, KeyBinding, Node, Plugin, Pos, Schema, Selection,
    Transaction, apply_input_rules,
};

#[cfg(feature = "collaboration")]
use rinch_editor_collab::{CollabError, CollabSession};

#[cfg(feature = "collaboration")]
use super::collab::CollabBridge;
use super::view::RinchDomEditorView;

/// The owned editor: its state, its desktop projection, and the schema/plugins
/// needed to rebuild a fresh state on `load_doc`.
///
/// `view` is `None` until the editor is mounted into a host element (design A7:
/// a handle is created with [`create_editor`](super::create_editor) *before* its
/// container exists, then projected when the [`Editor`](super::Editor) component
/// renders). State edits (`load_doc`/`command`/`set_selection`) work before mount
/// — they mutate the owned state, and the view renders the current state when it
/// attaches.
struct EditorCore {
    state: EditorState,
    view: Option<RinchDomEditorView>,
    schema: Rc<Schema>,
    plugins: Vec<Rc<dyn Plugin>>,
    /// Invoked after a local edit changes the document — see
    /// [`EditorHandle::on_change`]. Held behind an `Rc` so the notifier can clone
    /// it out and invoke it with **no** borrow held: the callback commonly
    /// re-enters the handle (an autosave reads `doc()`), which would otherwise
    /// panic with a `RefCell` double-borrow.
    on_change: Option<Rc<dyn Fn()>>,
    /// The collaboration session + outbound delta sink, when this editor is
    /// collaborating (design M9). `None` for a non-collaborative editor — the
    /// common case — so the mutation path's collab hook is a cheap early return.
    #[cfg(feature = "collaboration")]
    collab: Option<CollabBridge>,
}

impl EditorCore {
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
    /// Returns whether the **document** changed (a selection-only edit leaves the
    /// same doc `Rc`), which is what drives [`EditorHandle::on_change`].
    fn commit(&mut self, prev: EditorState, next: EditorState) -> bool {
        let doc_changed = !prev.doc.same_ref(&next.doc);
        self.state = next.clone();
        if let Some(view) = self.view.as_mut() {
            view.update_dom(&prev, &next);
        }
        #[cfg(feature = "collaboration")]
        self.record_local(&prev, &next);
        doc_changed
    }

    /// Project a just-applied local change onto the CRDT and broadcast the delta to
    /// peers. A no-op when not collaborating, or for a selection-only edit (the
    /// document is the same `Rc`, so there is nothing to project).
    #[cfg(feature = "collaboration")]
    fn record_local(&mut self, prev: &EditorState, next: &EditorState) {
        let Some(bridge) = self.collab.as_mut() else {
            return;
        };
        if prev.doc.same_ref(&next.doc) {
            return;
        }
        match bridge.session.record_local(&prev.doc, &next.doc) {
            Ok(()) => {
                let delta = bridge.session.save_incremental();
                if !delta.is_empty() {
                    (bridge.outbound)(delta);
                }
            }
            // Design A22 fail-loud: an edit outside the staged flat-text scope
            // (a table, a nested block) cannot be projected. Surface it rather than
            // silently diverging; apps should keep such edits out of a collab
            // session for now.
            Err(e) => bridge.last_error = Some(e),
        }
    }
}

/// A minimal valid document — `doc(paragraph())` — used to keep the editor in a
/// renderable, editable state when a load would otherwise yield a block-less doc.
/// `None` only if the schema lacks `paragraph`/`doc` (not the case for the starter
/// kit), in which case the caller keeps the original doc.
fn empty_paragraph_doc(schema: &Schema) -> Option<Node> {
    let para = schema.branch("paragraph", Fragment::empty()).ok()?;
    schema.branch("doc", Fragment::from_node(para)).ok()
}

/// A cloneable handle to an editor (design A7). Cheap to [`Clone`] (an `Rc`), so a
/// component can hand it to toolbar buttons and the runtime alike.
#[derive(Clone)]
pub struct EditorHandle {
    inner: Rc<RefCell<EditorCore>>,
}

impl fmt::Debug for EditorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The state/view aren't `Debug`; the host id is the useful identity.
        let mounted = self.inner.borrow().view.is_some();
        f.debug_struct("EditorHandle")
            .field("mounted", &mounted)
            .finish_non_exhaustive()
    }
}

impl EditorHandle {
    /// Build a handle over a fresh [`EditorState`] **without** a host projection —
    /// the deferred-mount path. The view is attached later by [`Self::mount`] (or
    /// [`Self::attach`]) when the host container exists. Used by
    /// [`create_editor`](super::create_editor).
    pub(crate) fn unmounted(
        schema: Rc<Schema>,
        doc: Node,
        plugins: Vec<Rc<dyn Plugin>>,
    ) -> EditorHandle {
        let state = EditorState::create(schema.clone(), doc, plugins.clone());
        EditorHandle {
            inner: Rc::new(RefCell::new(EditorCore {
                state,
                view: None,
                schema,
                plugins,
                on_change: None,
                #[cfg(feature = "collaboration")]
                collab: None,
            })),
        }
    }

    /// Build a handle and project it into `container` in one step (the eager path,
    /// used by tests and by [`Self::mount`]). `doc_ref` is a weak handle to the
    /// host document the view patches. Does **not** register the editor with the
    /// runtime — that is [`Self::mount`]'s job.
    pub fn new(
        container: NodeHandle,
        doc_ref: Weak<RefCell<dyn DomDocument>>,
        schema: Rc<Schema>,
        doc: Node,
        plugins: Vec<Rc<dyn Plugin>>,
    ) -> EditorHandle {
        let state = EditorState::create(schema.clone(), doc, plugins.clone());
        let view = RinchDomEditorView::new(container, doc_ref, &state);
        EditorHandle {
            inner: Rc::new(RefCell::new(EditorCore {
                state,
                view: Some(view),
                schema,
                plugins,
                on_change: None,
                #[cfg(feature = "collaboration")]
                collab: None,
            })),
        }
    }

    /// Project this (unmounted) handle into `container`, building the view from the
    /// current state — so any content loaded before mount renders immediately. The
    /// caller owns `container` (an empty host element); `doc_ref` is a weak handle
    /// to the host document. Re-attaching a handle that is already mounted replaces
    /// its view (the old projection is abandoned).
    pub(crate) fn attach(&self, container: NodeHandle, doc_ref: Weak<RefCell<dyn DomDocument>>) {
        let mut core = self.inner.borrow_mut();
        let view = RinchDomEditorView::new(container, doc_ref, &core.state);
        core.view = Some(view);
    }

    /// Mount this handle into `scope`: create its host container (`data-pm-editor`,
    /// deliberately **not** `contenteditable` so the legacy CE engine doesn't also
    /// activate), project the current state into it, and register the editor so the
    /// runtime drives its caret and routes input. Returns the container to place in
    /// the tree. The [`Editor`](super::Editor) component calls this on render.
    pub fn mount(&self, scope: &mut RenderScope) -> NodeHandle {
        let container = scope.create_element("div");
        container.set_attribute("data-pm-editor", "true");
        self.attach(container.clone(), scope.doc_weak());
        // Register scoped to this document — container ids collide across
        // documents on one thread (issue #134).
        let doc_key = scope
            .doc_weak()
            .upgrade()
            .map(|d| d.borrow().doc_key())
            .unwrap_or(0);
        super::registry::register_editor(doc_key, container.node_id().0, self.clone());
        container
    }

    /// Apply the transaction built by `build` (given the current state), then
    /// re-project the host. `build` returns `None` to dispatch nothing. Returns
    /// whether a transaction was applied. The single dispatch path — `command`,
    /// keyboard insert, paste, and IME commit all funnel through here.
    pub fn update(&self, build: impl FnOnce(&EditorState) -> Option<Transaction>) -> bool {
        let mut core = self.inner.borrow_mut();
        let Some(tr) = build(&core.state) else {
            return false;
        };
        let prev = core.state.clone();
        let next = core.state.apply(tr);
        let doc_changed = core.commit(prev, next);
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }

    /// Run the named command (applying + re-projecting if it applies). Returns
    /// whether it applied. The toolbar/keymap entry point.
    pub fn command(&self, name: &str) -> bool {
        let mut core = self.inner.borrow_mut();
        let Some(next) = core.state.run(name) else {
            return false;
        };
        let prev = core.state.clone();
        let doc_changed = core.commit(prev, next);
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }

    /// Register a callback invoked after a **local edit changes the document**.
    /// Replaces any previously registered callback.
    ///
    /// This is the cross-platform way to know the user edited — for autosave,
    /// dirty-marking or word counts. There is no DOM `input` event to listen for:
    /// the editor is model-first and deliberately never `contenteditable`.
    ///
    /// Fires for every edit path — [`update`](Self::update) (which typing, paste
    /// and IME commit all funnel through), [`command`](Self::command) and
    /// [`insert_image`](Self::insert_image). It does **not** fire for:
    /// - selection-only changes (the document is unchanged),
    /// - [`load_doc`](Self::load_doc) / [`load_html`](Self::load_html) — a
    ///   programmatic load is not a user edit, and firing would make an autosave
    ///   consumer immediately re-save freshly loaded content,
    /// - remote collaboration integration ([`collab_receive`](Self::collab_receive)),
    ///   which is already in the shared CRDT.
    ///
    /// The callback runs with no internal borrow held, so it may freely re-enter
    /// the handle (e.g. call [`doc`](Self::doc) to serialize for a save).
    ///
    /// ```ignore
    /// let editor = create_editor();
    /// editor.on_change({
    ///     let editor = editor.clone();
    ///     move || schedule_autosave(editor.doc())
    /// });
    /// ```
    pub fn on_change(&self, cb: impl Fn() + 'static) {
        self.inner.borrow_mut().on_change = Some(Rc::new(cb));
    }

    /// Invoke the change callback, if any, with **no borrow held** — the callback
    /// commonly re-enters the handle, which would otherwise double-borrow.
    fn notify_change(&self) {
        let cb = self.inner.borrow().on_change.clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    /// Whether the named command currently applies (toolbar enablement).
    pub fn can_run(&self, name: &str) -> bool {
        self.inner.borrow().state.can_run(name)
    }

    /// Look up `binding` in the editor's aggregated keymap and run the bound command.
    /// Returns `Some(applied)` when a binding matched (the key is **consumed** either
    /// way — the caller must key "consumed" on `is_some()`, not on the bool, so a no-op
    /// binding like `Tab` at top level never falls through to text insertion), or `None`
    /// when nothing is bound. This is the single keymap entry point: both the desktop
    /// and web key dispatchers translate their native event into a platform-agnostic
    /// [`KeyBinding`] and route through here, so a binding added in editor-core works on
    /// every platform (design §5).
    pub fn dispatch_key(&self, binding: KeyBinding) -> Option<bool> {
        let name = self
            .inner
            .borrow()
            .state
            .keymap()
            .command_for(&binding)
            .map(str::to_string);
        name.map(|n| self.command(&n))
    }

    /// Whether the mark named `mark` is active for the current selection (toolbar
    /// "on" state) — reads **state**, never the host.
    pub fn is_mark_active(&self, mark: &str) -> bool {
        let core = self.inner.borrow();
        match core.state.schema().mark_type(mark) {
            Some(mt) => is_mark_active(&core.state, mt),
            None => false,
        }
    }

    /// The `href` of the `link` mark active at the selection head, or `None` when
    /// the selection isn't inside a link — for pre-filling an "edit link" dialog
    /// (`is_mark_active("link")` only reports presence, not the target). Reads
    /// **state**, never the host.
    pub fn active_link_href(&self) -> Option<String> {
        let core = self.inner.borrow();
        let state = &core.state;
        let mt = state.schema().mark_type("link")?;
        marks_at(state, state.selection.head().0)
            .iter()
            .find(|m| &m.typ == mt)
            .and_then(|m| m.attrs.get_str("href"))
            .map(str::to_string)
    }

    /// The schema type name of the block the cursor is in (e.g. `"heading"`), or
    /// `None` across a multi-block selection.
    pub fn current_block_type(&self) -> Option<String> {
        current_block_type(&self.inner.borrow().state).map(|nt| nt.name().to_string())
    }

    /// Whether the selection is inside a node of the given type (e.g. `"blockquote"`,
    /// `"bullet_list"`) — drives the List/Blockquote toolbar active states (A6).
    pub fn in_node_type(&self, type_name: &str) -> bool {
        in_node_type(&self.inner.borrow().state, type_name)
    }

    /// The current document (the save shape; serialize with `to_doc()` under the
    /// `serde` feature).
    pub fn doc(&self) -> Node {
        self.inner.borrow().state.doc.clone()
    }

    /// A snapshot of the whole editor state.
    pub fn state(&self) -> EditorState {
        self.inner.borrow().state.clone()
    }

    /// The current selection.
    pub fn selection(&self) -> Selection {
        self.inner.borrow().state.selection.clone()
    }

    /// The host id of the editor container element, or `0` if not yet mounted.
    pub fn container_id(&self) -> usize {
        self.inner
            .borrow()
            .view
            .as_ref()
            .map_or(0, |v| v.container_id())
    }

    /// The host caret address `(textblock element id, flat UTF-8 byte offset)` for a
    /// model `pos` (used by app-side geometry: caret point, vertical movement).
    pub fn caret_address(&self, pos: Pos) -> Option<(usize, usize)> {
        let core = self.inner.borrow();
        core.view.as_ref()?.caret_address(&core.state.doc, pos)
    }

    /// Map a host caret address `(textblock element id, flat UTF-8 byte offset)` —
    /// e.g. from a pointer hit-test — to a model [`Pos`] (without moving the cursor).
    pub fn pos_at(&self, textblock_dom_id: usize, ifc_byte: usize) -> Option<Pos> {
        self.inner
            .borrow()
            .view
            .as_ref()?
            .pos_at(textblock_dom_id, ifc_byte)
    }

    /// Model-based vertical fallback for the geometry-driven Up/Down caret step: the
    /// caret position in the **adjacent textblock** — just before (`down = false`) or
    /// after (`down = true`) the current textblock. Used when geometry can't resolve
    /// the next visual line: the target line is an *empty* block (no text to hit-test)
    /// or a block atom is in the way, so the screen-space probe snaps back to the
    /// current line. Stepping in the model lets the caret still land on a blank line /
    /// past an atom. `None` at the document edge. Mirrors the desktop `vertical_step`
    /// stuck-path so both platforms behave the same.
    pub fn vertical_block_fallback(&self, down: bool) -> Option<Selection> {
        let core = self.inner.borrow();
        let doc = &core.state.doc;
        let head = core.state.selection.head();
        let r = doc.resolve(head).ok()?;
        let probe = if r.parent().is_textblock() {
            let content_start = head.0 - r.parent_offset();
            if down {
                content_start + r.parent().content().size() + 1
            } else {
                content_start.checked_sub(1)?
            }
        } else {
            head.0
        };
        Selection::near_text(
            doc,
            Pos(probe.min(doc.content_size())),
            if down { 1 } else { -1 },
        )
    }

    /// A [`Selection::Node`] for the leaf node (image / horizontal rule) whose host
    /// element is `host_id` — the pointer hit-test path for node-selecting a leaf the
    /// user clicks. `None` if `host_id` isn't a placed node, or its node isn't
    /// selectable (design §6 node-views).
    ///
    /// [`Selection::Node`]: rinch_editor_core::Selection::Node
    pub fn node_selection_at_host(&self, host_id: usize) -> Option<Selection> {
        let core = self.inner.borrow();
        let (pos, node) = core.view.as_ref()?.node_pos_for_host(host_id)?;
        // Node-views are *leaf* atoms (image / horizontal rule). A block container
        // (paragraph, list, blockquote) is `selectable` in the schema but is never
        // node-selected by a click, so restrict to leaves here.
        if !node.node_type().is_leaf() {
            return None;
        }
        Selection::node_at(&core.state.doc, Pos(pos))
    }

    /// Place the cursor at a host caret address `(textblock element id, flat UTF-8
    /// byte offset)` produced by a pointer hit-test (the click→`Pos` path). Returns
    /// whether the address resolved to a model position.
    pub fn set_cursor_from_ifc(&self, textblock_dom_id: usize, ifc_byte: usize) -> bool {
        match self.pos_at(textblock_dom_id, ifc_byte) {
            Some(pos) => {
                self.set_selection(Selection::cursor(pos));
                true
            }
            None => false,
        }
    }

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
        self.update(|state| {
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
        })
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

    /// Move the selection (and re-project, so the caret follows once geometry lands).
    pub fn set_selection(&self, selection: Selection) {
        self.update(|state| {
            let mut tr = state.tr();
            tr.set_selection(selection.clone());
            Some(tr)
        });
    }

    /// Select the word around model `pos` (the double-click gesture). Returns whether
    /// it resolved to a non-empty range.
    pub fn select_word_at(&self, pos: Pos) -> bool {
        let doc = self.doc();
        let (from, to) = rinch_editor_core::word_range_at(&doc, pos);
        self.set_selection(Selection::text(from, to));
        from != to
    }

    /// Select the whole textblock around model `pos` (the triple-click gesture).
    /// Returns whether it resolved to a non-empty range.
    pub fn select_block_at(&self, pos: Pos) -> bool {
        let doc = self.doc();
        let (from, to) = rinch_editor_core::block_range_at(&doc, pos);
        self.set_selection(Selection::text(from, to));
        from != to
    }

    /// Apply a model caret `motion` (the horizontal / word / model-line / document
    /// motions — *not* vertical, which needs laid-out geometry the platform supplies).
    /// `extend` keeps the selection anchor (shift+arrow). Returns whether the cursor
    /// moved. Shared by every platform's keyboard glue; the desktop runtime layers
    /// visual-line and vertical geometry on top.
    pub fn move_cursor(&self, motion: CursorMotion, extend: bool) -> bool {
        let st = self.state();
        match rinch_editor_core::resolve_cursor_motion(
            &st.doc,
            st.selection.head(),
            st.selection.anchor(),
            motion,
            extend,
        ) {
            Some(sel) => {
                self.set_selection(sel);
                true
            }
            None => false,
        }
    }

    /// Move the cursor to the next (`shift=false`) or previous (`shift=true`) table
    /// cell — the Tab/Shift-Tab gesture inside a table. Forward-Tab in the last cell
    /// appends a row first. A no-op (returns `false`) outside a table. Shared by every
    /// platform's keyboard glue.
    pub fn tab_cell(&self, shift: bool) -> bool {
        use rinch_editor_core::tables;
        let state = self.state();
        let head = state.selection.head();
        let dir = if shift { -1 } else { 1 };
        if let Some(target) = tables::next_cell_in_table(&state.doc, head, dir) {
            self.set_selection(Selection::near(&state.doc, Pos(target + 1), 1));
            return true;
        }
        if dir > 0 && tables::cell_at_pos(&state.doc, head).is_some() && self.command("addRowAfter")
        {
            let state = self.state();
            if let Some(target) = tables::next_cell_in_table(&state.doc, state.selection.head(), 1)
            {
                self.set_selection(Selection::near(&state.doc, Pos(target + 1), 1));
            }
            return true;
        }
        false
    }

    /// Switch the editor between the light (default) and dark color schemes of the
    /// built-in stylesheet. A no-op before mount. The app should trigger a repaint
    /// afterward (toolbar/keyboard handlers already do).
    pub fn set_dark_mode(&self, dark: bool) {
        if let Some(view) = self.inner.borrow().view.as_ref() {
            view.set_dark_mode(dark);
        }
    }

    /// Replace the document with `doc`, resetting selection and history (a fresh
    /// load, not an undoable edit). The host diffs from the old content to the new,
    /// so unchanged leading blocks are reused. Works before focus.
    ///
    /// A **block-less** `doc` (zero children — e.g. from parsing empty/whitespace
    /// HTML, since `Schema::branch` does not fill required content) is repaired to a
    /// single empty paragraph, so the editor is never left with no textblock to
    /// render or place a caret in.
    pub fn load_doc(&self, doc: Node) {
        let mut core = self.inner.borrow_mut();
        let doc = if doc.child_count() == 0 {
            empty_paragraph_doc(&core.schema).unwrap_or(doc)
        } else {
            doc
        };
        let prev = core.state.clone();
        let next = EditorState::create(core.schema.clone(), doc, core.plugins.clone());
        // Deliberately does not fire `on_change`: loading a document is a
        // programmatic replace, not a user edit. Firing here would make an
        // autosave consumer immediately re-save freshly loaded content.
        core.commit(prev, next);
    }

    /// Parse `html` (schema-whitelisted) and load it as the document. Empty or
    /// whitespace-only `html` loads a single empty paragraph (via [`Self::load_doc`]),
    /// not a block-less doc. Returns `false` only if `html` fails to parse at all.
    pub fn load_html(&self, html: &str) -> bool {
        let schema = self.inner.borrow().schema.clone();
        let Ok(slice) = slice_from_html(&schema, html) else {
            return false;
        };
        let Ok(doc) = schema.branch("doc", slice.content.clone()) else {
            return false;
        };
        self.load_doc(doc);
        true
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
    pub fn selection_clipboard(&self) -> Option<(String, String)> {
        let core = self.inner.borrow();
        let sel = &core.state.selection;
        if sel.is_empty() {
            return None;
        }
        let slice = core.state.doc.slice(sel.from().0, sel.to().0).ok()?;
        Some((slice_to_html(&slice), slice_to_text(&slice)))
    }

    /// Replace the current selection with a parsed (schema-whitelisted) HTML
    /// payload — the rich paste path. Returns whether anything was inserted.
    pub fn replace_selection_with_html(&self, html: &str) -> bool {
        let schema = self.inner.borrow().schema.clone();
        match slice_from_html(&schema, html) {
            Ok(slice) if slice.content.child_count() > 0 => self.replace_selection_slice(slice),
            _ => false,
        }
    }

    /// Insert an image node with `src` (e.g. a `data:` URL) and `alt`, replacing
    /// the current selection — the image-paste path. Returns whether the document
    /// changed (the schema rejects an image where inline content isn't allowed).
    pub fn insert_image(&self, src: &str, alt: &str) -> bool {
        let cmd = rinch_editor_core::commands::insert_image(src.to_string(), alt.to_string());
        let mut core = self.inner.borrow_mut();
        let Some(next) = core.state.run_command(&cmd) else {
            return false;
        };
        let prev = core.state.clone();
        let doc_changed = core.commit(prev, next);
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
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
        let mut core = self.inner.borrow_mut();
        let Some(next) = core.state.run_command(&cmd) else {
            return false;
        };
        let prev = core.state.clone();
        let doc_changed = core.commit(prev, next);
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }

    /// Replace the current selection with plain text (one paragraph per line) —
    /// the plain-text paste path. Returns whether anything was inserted.
    pub fn replace_selection_with_text(&self, text: &str) -> bool {
        let schema = self.inner.borrow().schema.clone();
        match slice_from_text(&schema, text) {
            Ok(slice) if slice.content.child_count() > 0 => self.replace_selection_slice(slice),
            _ => false,
        }
    }

    /// Replace the selection range with `slice` (the shared paste mechanism). The
    /// open slice merges into the surrounding block via the transform's `replace`;
    /// the cursor lands after the inserted content via the transaction's selection
    /// mapping.
    fn replace_selection_slice(&self, slice: Slice) -> bool {
        self.update(move |state| {
            let (from, to) = (state.selection.from().0, state.selection.to().0);
            let mut tr = state.tr();
            tr.replace(from, to, slice).ok()?;
            // Collapse the cursor just after the inserted content (PM
            // `replaceSelection` semantics). Without this, the default per-endpoint
            // selection mapping leaves a range selection spanning the paste. Map the
            // range's right edge with assoc +1 so a pure-insert cursor lands *after*
            // the inserted content (assoc -1 would keep it before).
            let end = tr.mapping().map(to, 1);
            let cursor = Selection::near(tr.doc(), Pos(end), -1);
            tr.set_selection(cursor);
            Some(tr)
        })
    }

    /// Phase-2 projection: render caret/selection geometry from the current state.
    /// The runtime calls this **after** layout (design A3); headless it is a no-op.
    /// Returns whether an overlay actually moved (so the runtime can force a full
    /// repaint — the overlays are absolutely positioned and the software renderer's
    /// dirty-region cache can't clear their old rect).
    pub fn update_caret(&self) -> bool {
        let mut core = self.inner.borrow_mut();
        let state = core.state.clone();
        match core.view.as_mut() {
            Some(view) => {
                view.update_caret(&state);
                view.take_overlay_dirty()
            }
            None => false,
        }
    }

    /// Hide this editor's overlays (caret + selection highlight) because it isn't
    /// focused. The runtime's focus-aware caret pass calls this for every editor
    /// that isn't the focused one. A no-op before mount. Returns whether an overlay
    /// was actually cleared (so the runtime can force a full repaint).
    pub fn hide_overlays(&self) -> bool {
        match self.inner.borrow_mut().view.as_mut() {
            Some(view) => {
                view.hide_overlays();
                view.take_overlay_dirty()
            }
            None => false,
        }
    }

    /// Apply a caret blink phase (the runtime's blink driver calls this each
    /// half-period). Returns `None` if there is no caret to blink (no collapsed
    /// cursor), `Some(true)` if the caret's visibility actually toggled (repaint
    /// needed), or `Some(false)` if the phase was already applied.
    pub fn set_caret_blink(&self, visible: bool) -> Option<bool> {
        self.inner
            .borrow_mut()
            .view
            .as_mut()?
            .set_caret_blink_visible(visible)
    }

    // ── IME (input method editor) ────────────────────────────────────────────
    //
    // The composition (preedit) is a **view-local overlay** that is never part of
    // the document (design A5): it is shown at the caret while composing and
    // discarded on commit or clear. Commit inserts the final text as one ordinary
    // edit, so undo/history treat it exactly like typing.

    /// Show the IME composition string `text` as a transient overlay at the caret
    /// (never inserted into the document). An empty `text` clears the overlay.
    /// `cursor` is the candidate cursor within `text`; the overlay ignores it for
    /// now (the platform candidate box is placed from the model caret instead). A
    /// no-op before mount.
    pub fn ime_set_preedit(&self, text: &str, _cursor: Option<(usize, usize)>) {
        if let Some(view) = self.inner.borrow_mut().view.as_mut() {
            view.set_preedit(text);
        }
    }

    /// Clear the IME composition overlay without inserting anything (composition
    /// cancelled / disabled). A no-op before mount.
    pub fn ime_clear_preedit(&self) {
        if let Some(view) = self.inner.borrow_mut().view.as_mut() {
            view.set_preedit("");
        }
    }

    /// Commit composed `text`: clear the preedit overlay, then insert the text at
    /// the selection as one ordinary edit (so it joins the undo history like
    /// typing). An empty commit just clears the overlay.
    pub fn ime_commit(&self, text: &str) {
        if let Some(view) = self.inner.borrow_mut().view.as_mut() {
            view.set_preedit("");
        }
        if !text.is_empty() {
            self.insert_text(text);
        }
    }

    /// Delete `before` characters before the caret and `after` after it — the
    /// surrounding-text edit some IMEs use to recompose. Clears any preedit first,
    /// then deletes the clamped `[head - before, head + after)` range in one edit.
    /// A defensive no-op if the range is empty or the delete is invalid (e.g. it
    /// would cross a block boundary the schema rejects). Only reached once a backend
    /// advertises surrounding-text support.
    pub fn ime_delete_surrounding(&self, before: usize, after: usize) {
        if let Some(view) = self.inner.borrow_mut().view.as_mut() {
            view.set_preedit("");
        }
        if before == 0 && after == 0 {
            return;
        }
        self.update(|state| {
            let head = state.selection.head().0;
            let from = head.saturating_sub(before);
            let to = (head + after).min(state.doc.content().size());
            if from >= to {
                return None;
            }
            let mut tr = state.tr();
            tr.delete(from, to).ok()?;
            Some(tr)
        });
    }
}

// ── Collaboration (design M9, the `collaboration` feature) ───────────────────
//
// One [`CollabSession`] per editor. A local edit is projected onto the CRDT and
// broadcast (the `commit` → `record_local` path above); a peer's delta arrives via
// `collab_receive`, which integrates it and re-projects without re-broadcasting. The
// transport is the caller's concern: `outbound` carries bytes out, `collab_receive`
// (or the platform runtime's thread-safe `post_remote_delta`) carries them
// back in.
#[cfg(feature = "collaboration")]
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
    pub fn start_collaboration_host(
        &self,
        outbound: impl Fn(Vec<u8>) + 'static,
    ) -> Result<Vec<u8>, CollabError> {
        let mut core = self.inner.borrow_mut();
        let mut session = CollabSession::new(&core.state)?;
        let snapshot = session.snapshot();
        core.collab = Some(CollabBridge::new(session, Box::new(outbound)));
        Ok(snapshot)
    }

    /// Join an existing collaboration as a **guest** from a host's `snapshot` (from
    /// [`Self::start_collaboration_host`]): adopt the host's converged document and
    /// attach a session whose CRDT already matches it, so both peers start from the
    /// same content. `outbound` carries this guest's local deltas back to peers.
    /// Returns a [`CollabError`] if the snapshot is unreadable or its content is
    /// outside the staged scope.
    pub fn start_collaboration_guest(
        &self,
        snapshot: &[u8],
        outbound: impl Fn(Vec<u8>) + 'static,
    ) -> Result<(), CollabError> {
        let session = CollabSession::from_bytes(snapshot)?;
        // Adopt the host's document first (no session attached yet, so this load is
        // not recorded back onto the CRDT), then attach the matching session.
        let schema = self.inner.borrow().schema.clone();
        let doc = session.projected_doc(&schema)?;
        self.load_doc(doc);
        self.inner.borrow_mut().collab = Some(CollabBridge::new(session, Box::new(outbound)));
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
        // `outbound` runs while a local edit holds this handle's borrow; a self-
        // wired sink that calls back in here would otherwise hit an already-borrowed
        // panic. Fail soft instead.
        let Ok(mut core) = self.inner.try_borrow_mut() else {
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
            .integrate_incremental(&prev, delta);
        match result {
            Ok(Some(next)) => {
                core.state = next.clone();
                if let Some(view) = core.view.as_mut() {
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
        self.inner.borrow().collab.is_some()
    }

    /// A snapshot of the shared document **as it stands now**, for a *late-joining*
    /// peer: hand it to a new guest's [`Self::start_collaboration_guest`] so they
    /// adopt the current content (not just the host's original document). `None`
    /// when this editor isn't collaborating. Assumes a reliable, ordered delta
    /// transport between existing peers; for lossy / out-of-order reconciliation use
    /// the sync protocol ([`Self::collab_generate_sync_message`] /
    /// [`Self::collab_receive_sync_message`]) instead.
    pub fn collab_snapshot(&self) -> Option<Vec<u8>> {
        self.inner
            .borrow_mut()
            .collab
            .as_mut()
            .map(|b| b.session.snapshot())
    }

    /// Generate the next Automerge **sync-protocol** message for the peer tracked by
    /// `sync_state`, or `None` when that peer is up to date (or this editor isn't
    /// collaborating).
    ///
    /// This is the state-negotiating alternative to the incremental-delta broadcast
    /// (`outbound` + [`Self::collab_receive`]). Prefer it when the transport is not a
    /// reliable ordered stream — an HTTP poll, a reconnecting socket, a peer that was
    /// offline — because it reconciles from each side's actual heads instead of
    /// assuming every delta arrived exactly once.
    ///
    /// The caller owns the [`SyncState`], one per peer. Generating a message mutates
    /// protocol bookkeeping only; the document is untouched, so nothing needs saving
    /// as a result of this call.
    ///
    /// Uses `try_borrow_mut` and yields `None` if the handle is already borrowed, for
    /// the same reason [`Self::collab_receive`] does.
    pub fn collab_generate_sync_message(
        &self,
        sync_state: &mut rinch_editor_collab::SyncState,
    ) -> Option<rinch_editor_collab::SyncMessage> {
        let mut core = self.inner.try_borrow_mut().ok()?;
        core.collab
            .as_mut()?
            .session
            .generate_sync_message(sync_state)
    }

    /// Integrate a peer's Automerge **sync-protocol** message: merge it into the CRDT,
    /// rebuild the model from the *converged* CRDT, and re-project the view. Like
    /// [`Self::collab_receive`], the change lands as a non-undoable, remote-origin
    /// transaction and is **not** re-broadcast. Returns whether the document changed
    /// (a message that only advances protocol state returns `false`).
    ///
    /// Must run on the main thread. A projection failure is recorded and readable via
    /// [`Self::collab_take_error`].
    pub fn collab_receive_sync_message(
        &self,
        sync_state: &mut rinch_editor_collab::SyncState,
        message: rinch_editor_collab::SyncMessage,
    ) -> bool {
        let Ok(mut core) = self.inner.try_borrow_mut() else {
            return false;
        };
        if core.collab.is_none() {
            return false;
        }
        let prev = core.state.clone();
        let result = core
            .collab
            .as_mut()
            .unwrap()
            .session
            .integrate_sync_message(&prev, sync_state, message);
        match result {
            Ok(Some(next)) => {
                core.state = next.clone();
                if let Some(view) = core.view.as_mut() {
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

    /// The CRDT's current heads (its change frontier), or `None` when this editor
    /// isn't collaborating. Useful for persisting "what this peer had" alongside a
    /// snapshot, and for diffing against a later frontier.
    pub fn collab_heads(&self) -> Option<Vec<rinch_editor_collab::ChangeHash>> {
        self.inner
            .borrow_mut()
            .collab
            .as_mut()
            .map(|b| b.session.heads())
    }

    /// Detach the collaboration session (stop projecting and broadcasting). The
    /// document is unchanged; subsequent edits are local-only again.
    pub fn stop_collaboration(&self) {
        self.inner.borrow_mut().collab = None;
    }

    /// Take (and clear) the most recent collaboration error — e.g. an edit outside
    /// the staged flat-text scope that could not be projected (design A22). `None`
    /// when not collaborating or no error is pending.
    pub fn collab_take_error(&self) -> Option<CollabError> {
        self.inner
            .borrow_mut()
            .collab
            .as_mut()
            .and_then(|b| b.last_error.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::dom::NodeId;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_editor_core::model::Fragment;
    use rinch_editor_core::{Pos, default_plugins};

    struct Harness {
        doc: Rc<RefCell<dyn DomDocument>>,
        container_id: NodeId,
        handle: EditorHandle,
    }

    fn mount(html_blocks: Node) -> Harness {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let container_id = doc.borrow_mut().create_element("div");
        let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
        let schema = Rc::new(Schema::starter_kit());
        let handle = EditorHandle::new(
            container,
            Rc::downgrade(&doc),
            schema,
            html_blocks,
            default_plugins(),
        );
        Harness {
            doc,
            container_id,
            handle,
        }
    }

    fn schema() -> Schema {
        Schema::starter_kit()
    }
    fn para(s: &Schema, t: &str) -> Node {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    }
    fn doc_node(s: &Schema, blocks: Vec<Node>) -> Node {
        s.branch("doc", Fragment::from_children(blocks)).unwrap()
    }
    fn children(h: &Harness, id: NodeId) -> Vec<NodeId> {
        h.doc.borrow().get_children(id)
    }
    fn tag(h: &Harness, id: NodeId) -> Option<String> {
        h.doc.borrow().tag_name(id)
    }
    fn text(h: &Harness, id: NodeId) -> Option<String> {
        h.doc.borrow().text_content(id)
    }

    #[test]
    fn command_toggles_mark_and_reprojects() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
        // Select the whole word, then bold it via the command.
        h.handle.set_selection(Selection::text(Pos(1), Pos(5)));
        assert!(!h.handle.is_mark_active("bold"));
        assert!(h.handle.command("toggleBold"), "toggleBold applies");
        assert!(h.handle.is_mark_active("bold"), "state reports bold active");

        // The host re-projected: the run is now wrapped in <strong>.
        let p = children(&h, h.container_id)[0];
        let strong = children(&h, p)[0];
        assert_eq!(tag(&h, strong).as_deref(), Some("strong"));
        assert_eq!(text(&h, strong).as_deref(), Some("abcd"));
    }

    #[test]
    fn block_type_command_and_query() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "title")]));
        h.handle.set_selection(Selection::cursor(Pos(2)));
        assert_eq!(h.handle.current_block_type().as_deref(), Some("paragraph"));
        assert!(h.handle.command("setHeading2"));
        assert_eq!(h.handle.current_block_type().as_deref(), Some("heading"));
        let block = children(&h, h.container_id)[0];
        assert_eq!(tag(&h, block).as_deref(), Some("h2"));
    }

    #[test]
    fn update_inserts_text_through_one_path() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab")]));
        h.handle.set_selection(Selection::cursor(Pos(3))); // end of "ab"
        let applied = h.handle.update(|state| {
            let mut tr = state.tr();
            tr.insert_text("c").ok()?;
            Some(tr)
        });
        assert!(applied);
        let p = children(&h, h.container_id)[0];
        assert_eq!(text(&h, p).as_deref(), Some("abc"));
    }

    fn empty_para(s: &Schema) -> Node {
        s.branch("paragraph", Fragment::empty()).unwrap()
    }

    #[test]
    fn insert_text_fires_block_input_rule() {
        // Typing "# " at the block start applies the heading input rule. This proves the
        // markdown rules are wired into the live text-input path — not merely
        // unit-tested in editor-core (they were unreachable before this).
        let s = schema();
        let h = mount(doc_node(&s, vec![empty_para(&s)]));
        h.handle.set_selection(Selection::cursor(Pos(1)));
        h.handle.insert_text("#");
        h.handle.insert_text(" ");
        assert_eq!(h.handle.current_block_type().as_deref(), Some("heading"));
        let block = children(&h, h.container_id)[0];
        assert_eq!(tag(&h, block).as_deref(), Some("h1"));
    }

    #[test]
    fn insert_text_fires_mark_input_rule() {
        // "**bold**" typed at a cursor wraps "bold" in a bold mark and drops the markup —
        // verified on the projected DOM (a <strong> wrapping the text), not just the model.
        let s = schema();
        let h = mount(doc_node(&s, vec![empty_para(&s)]));
        h.handle.set_selection(Selection::cursor(Pos(1)));
        h.handle.insert_text("**bold**");
        let p = children(&h, h.container_id)[0];
        let strong = children(&h, p)[0];
        assert_eq!(tag(&h, strong).as_deref(), Some("strong"));
        assert_eq!(text(&h, strong).as_deref(), Some("bold"));
    }

    #[test]
    fn insert_text_fires_task_list_input_rule() {
        // "[ ] " at the block start turns the block into a task list.
        let s = schema();
        let h = mount(doc_node(&s, vec![empty_para(&s)]));
        h.handle.set_selection(Selection::cursor(Pos(1)));
        h.handle.insert_text("[ ] ");
        assert_eq!(h.handle.doc().child(0).type_name(), "task_list");
    }

    #[test]
    fn toggle_task_checked_flips_the_enclosing_item() {
        let s = schema();
        let item = s
            .branch("task_item", Fragment::from_node(para(&s, "x")))
            .unwrap();
        let list = s.branch("task_list", Fragment::from_node(item)).unwrap();
        let h = mount(doc_node(&s, vec![list]));
        // Cursor inside the item's paragraph (pos 3 = before "x").
        h.handle.set_selection(Selection::cursor(Pos(3)));

        let checked = |h: &Harness| h.handle.doc().child(0).child(0).attrs().get_bool("checked");
        assert_ne!(checked(&h), Some(true), "starts unchecked");
        assert!(
            h.handle.toggle_task_checked_at(3),
            "toggles the enclosing task item"
        );
        assert_eq!(checked(&h), Some(true), "now checked");
        assert!(h.handle.toggle_task_checked_at(3));
        assert_ne!(checked(&h), Some(true), "toggled back off");
    }

    #[test]
    fn toggle_task_checked_is_a_noop_outside_a_task_item() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "plain")]));
        assert!(
            !h.handle.toggle_task_checked_at(2),
            "no task item at the cursor"
        );
    }

    #[test]
    fn dispatch_key_runs_a_bound_command() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
        h.handle.set_selection(Selection::text(Pos(1), Pos(5)));
        // Mod-b → toggleBold, from the aggregated keymap.
        let b = KeyBinding::parse("Mod-b").unwrap();
        assert_eq!(
            h.handle.dispatch_key(b),
            Some(true),
            "a bound key runs its command"
        );
        assert!(h.handle.is_mark_active("bold"));
    }

    #[test]
    fn dispatch_key_returns_none_when_unbound() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "x")]));
        // A bare 'q' has no binding — the caller falls through to text insertion.
        assert_eq!(h.handle.dispatch_key(KeyBinding::parse("q").unwrap()), None);
        // selectAll is bound via Mod-a and always applies (consumes the key).
        assert_eq!(
            h.handle.dispatch_key(KeyBinding::parse("Mod-a").unwrap()),
            Some(true)
        );
    }

    #[test]
    fn insert_text_with_a_selection_skips_input_rules() {
        // Input rules only fire at a collapsed cursor: typing "**" over a selection just
        // replaces it (no half-applied rule against a non-empty range).
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
        h.handle.set_selection(Selection::text(Pos(1), Pos(5)));
        h.handle.insert_text("x");
        let p = children(&h, h.container_id)[0];
        assert_eq!(text(&h, p).as_deref(), Some("x"));
    }

    #[test]
    fn load_doc_replaces_content() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "old")]));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("old")
        );

        h.handle
            .load_doc(doc_node(&s, vec![para(&s, "fresh"), para(&s, "lines")]));
        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2);
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("fresh"));
        assert_eq!(text(&h, blocks[1]).as_deref(), Some("lines"));
    }

    #[test]
    fn load_html_parses_and_loads() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "x")]));
        assert!(
            h.handle
                .load_html("<h1>Title</h1><p>Body <strong>bold</strong></p>")
        );
        let blocks = children(&h, h.container_id);
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("h1"));
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("Title"));
        assert_eq!(tag(&h, blocks[1]).as_deref(), Some("p"));
        // The bold run is wrapped.
        let p_children = children(&h, blocks[1]);
        let has_strong = p_children
            .iter()
            .any(|&c| tag(&h, c).as_deref() == Some("strong"));
        assert!(has_strong, "bold inline survived the load");
    }

    #[test]
    fn load_html_empty_keeps_one_empty_paragraph() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "old")]));
        // Empty HTML must NOT leave a block-less doc (no textblock to render or
        // place a caret in) — it loads a single empty paragraph instead.
        assert!(h.handle.load_html(""));
        let doc = h.handle.doc();
        assert_eq!(doc.child_count(), 1, "one block, not zero");
        assert_eq!(doc.child(0).type_name(), "paragraph");
        assert_eq!(doc.child(0).child_count(), 0, "the paragraph is empty");

        // The host re-projected to exactly one (empty) <p>.
        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 1);
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("p"));

        // And the cursor can be placed in it (a block-less doc would have no valid
        // position here).
        h.handle.set_selection(Selection::cursor(Pos(1)));
        assert!(h.handle.insert_text("x"));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("x")
        );
    }

    #[test]
    fn load_doc_blockless_is_repaired() {
        // A directly-constructed block-less doc (Schema::branch does not fill
        // required content) is repaired to one empty paragraph by load_doc.
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "old")]));
        let blockless = s.branch("doc", Fragment::empty()).unwrap();
        assert_eq!(blockless.child_count(), 0);
        h.handle.load_doc(blockless);
        assert_eq!(h.handle.doc().child_count(), 1);
        assert_eq!(h.handle.doc().child(0).type_name(), "paragraph");
    }

    #[test]
    fn handle_clones_share_one_editor() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab")]));
        let clone = h.handle.clone();
        clone.set_selection(Selection::cursor(Pos(3)));
        clone.update(|state| {
            let mut tr = state.tr();
            tr.insert_text("Z").ok()?;
            Some(tr)
        });
        // The original handle sees the mutation (shared Rc).
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("abZ")
        );
    }

    // ── Clipboard (copy / cut / paste) ───────────────────────────────────────

    #[test]
    fn selection_clipboard_serializes_marked_run() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
        // Empty selection → nothing to copy.
        h.handle.set_selection(Selection::cursor(Pos(2)));
        assert!(h.handle.selection_clipboard().is_none());

        // Bold the whole word, select it, copy.
        h.handle.set_selection(Selection::text(Pos(1), Pos(5)));
        assert!(h.handle.command("toggleBold"));
        let (html, plain) = h
            .handle
            .selection_clipboard()
            .expect("non-empty selection copies");
        assert_eq!(html, "<strong>abcd</strong>", "rich HTML carries the mark");
        assert_eq!(plain, "abcd", "plain text drops the mark");
    }

    #[test]
    fn paste_html_inserts_rich_content_at_cursor() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab")]));
        h.handle.set_selection(Selection::cursor(Pos(2))); // between a and b
        assert!(h.handle.replace_selection_with_html("<strong>X</strong>"));

        // "a" + bold "X" + "b" inside the one paragraph.
        let p = children(&h, h.container_id)[0];
        assert_eq!(text(&h, p).as_deref(), Some("aXb"));
        let strong = children(&h, p)
            .into_iter()
            .find(|&c| tag(&h, c).as_deref() == Some("strong"));
        assert!(strong.is_some(), "pasted bold run wrapped in <strong>");
        // Cursor lands after the inserted run (model pos 3: 0[p 1 a X b]).
        assert!(
            h.handle.selection().is_empty(),
            "paste collapses the cursor"
        );
        assert_eq!(h.handle.selection().head(), Pos(3));
    }

    #[test]
    fn insert_image_places_an_inline_image_node() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab")]));
        h.handle.set_selection(Selection::cursor(Pos(2))); // between "a" and "b"
        assert!(h.handle.insert_image("data:image/png;base64,AAAA", "shot"));
        // The image lands inline in the paragraph, between the text runs.
        let p = children(&h, h.container_id)[0];
        let img = children(&h, p)
            .into_iter()
            .find(|&c| tag(&h, c).as_deref() == Some("img"));
        assert!(img.is_some(), "image node placed inline in the paragraph");
        let img = img.unwrap();
        assert_eq!(
            h.doc.borrow().get_attribute(img, "src").as_deref(),
            Some("data:image/png;base64,AAAA")
        );
    }

    #[test]
    fn toggle_link_sets_and_removes_a_link_mark() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "click here")]));
        // Select "click", then link it.
        h.handle.set_selection(Selection::text(Pos(1), Pos(6)));
        assert!(!h.handle.is_mark_active("link"));
        assert!(h.handle.toggle_link("https://example.com"), "link applies");
        assert!(h.handle.is_mark_active("link"), "state reports link active");
        assert_eq!(
            h.handle.active_link_href().as_deref(),
            Some("https://example.com")
        );

        // The host re-projected: the run is now wrapped in <a href=...>.
        let p = children(&h, h.container_id)[0];
        let anchor = children(&h, p)
            .into_iter()
            .find(|&c| tag(&h, c).as_deref() == Some("a"));
        assert!(anchor.is_some(), "anchor projected into the host");
        assert_eq!(
            h.doc
                .borrow()
                .get_attribute(anchor.unwrap(), "href")
                .as_deref(),
            Some("https://example.com")
        );

        // Toggling again over the same range removes the link.
        assert!(h.handle.toggle_link("https://ignored.example"));
        assert!(
            !h.handle.is_mark_active("link"),
            "link removed on re-toggle"
        );
        assert_eq!(h.handle.active_link_href(), None);
    }

    #[test]
    fn toggle_link_is_a_noop_on_a_collapsed_cursor() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "abc")]));
        h.handle.set_selection(Selection::cursor(Pos(2)));
        // A link needs a range — the collapsed toggle changes nothing.
        assert!(!h.handle.toggle_link("https://example.com"));
        assert!(!h.handle.is_mark_active("link"));
        assert_eq!(h.handle.active_link_href(), None);
    }

    #[test]
    fn active_link_href_reads_the_target_from_inside_a_link() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "linked text")]));
        h.handle.set_selection(Selection::text(Pos(1), Pos(12)));
        assert!(h.handle.toggle_link("https://rust-lang.org"));
        // Collapse the cursor inside the linked run — the href is still readable.
        h.handle.set_selection(Selection::cursor(Pos(3)));
        assert_eq!(
            h.handle.active_link_href().as_deref(),
            Some("https://rust-lang.org")
        );
    }

    #[test]
    fn paste_text_over_selection_replaces_it() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "hello")]));
        h.handle.set_selection(Selection::text(Pos(1), Pos(6))); // whole word
        assert!(h.handle.replace_selection_with_text("bye"));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("bye")
        );
        assert!(h.handle.selection().is_empty());
    }

    #[test]
    fn paste_multiline_text_splits_blocks() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "")]));
        h.handle.set_selection(Selection::cursor(Pos(1)));
        assert!(h.handle.replace_selection_with_text("one\ntwo"));
        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2, "two lines → two paragraphs");
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("one"));
        assert_eq!(text(&h, blocks[1]).as_deref(), Some("two"));
    }

    #[test]
    fn cut_then_paste_round_trips() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
        // "Cut" cd: copy then delete the selection.
        h.handle.set_selection(Selection::text(Pos(3), Pos(5)));
        let (html, _plain) = h.handle.selection_clipboard().expect("copies");
        assert!(h.handle.command("deleteSelection"));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("ab")
        );

        // Paste it back at the end.
        h.handle.set_selection(Selection::cursor(Pos(3)));
        assert!(h.handle.replace_selection_with_html(&html));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("abcd")
        );
    }

    // ── Node-views (NodeSelection of an image / horizontal rule) ─────────────

    fn hr(s: &Schema) -> Node {
        s.branch("horizontal_rule", Fragment::empty()).unwrap()
    }

    fn img(s: &Schema) -> Node {
        s.create_node(
            "image",
            rinch_editor_core::Attrs::from_iter([(
                "src",
                rinch_editor_core::AttrValue::from("a.png"),
            )]),
            Fragment::empty(),
        )
        .unwrap()
    }

    #[test]
    fn node_selection_at_host_resolves_a_leaf_and_rejects_a_textblock() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab"), hr(&s)]));
        let blocks = children(&h, h.container_id);
        assert_eq!(tag(&h, blocks[1]).as_deref(), Some("hr"));

        // The hr's host id → a NodeSelection of the hr (model pos 4..5).
        let sel = h
            .handle
            .node_selection_at_host(blocks[1].0)
            .expect("hr host resolves to a node selection");
        assert_eq!(sel.from(), Pos(4));
        assert_eq!(sel.to(), Pos(5));

        // The paragraph host is a node but not a selectable leaf → None.
        assert!(
            h.handle.node_selection_at_host(blocks[0].0).is_none(),
            "a textblock is not node-selectable"
        );
    }

    #[test]
    fn backspace_deletes_a_node_selection() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab"), hr(&s)]));
        let sel = h
            .handle
            .node_selection_at_host(children(&h, h.container_id)[1].0)
            .unwrap();
        h.handle.set_selection(sel);

        // Backspace over a (never-empty) node selection deletes the node.
        assert!(h.handle.command("deleteCharBackward"));
        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 1, "the hr was removed");
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("p"));
    }

    #[test]
    fn typing_replaces_an_inline_image_node_selection() {
        let s = schema();
        // doc(paragraph(text "a", image)) — positions 0[p 1 a 2 (img) 3]4.
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![s.text("a").unwrap(), img(&s)]),
            )
            .unwrap();
        let h = mount(doc_node(&s, vec![p]));
        // The image is the paragraph's 2nd inline host child; node-select it.
        let para_host = children(&h, h.container_id)[0];
        let img_host = children(&h, para_host)[1];
        let sel = h
            .handle
            .node_selection_at_host(img_host.0)
            .expect("inline image node-selects");
        h.handle.set_selection(sel);

        // A flat text insert *can* replace an inline image (text is valid inline
        // content) — the image becomes the typed text within the paragraph.
        assert!(h.handle.update(|state| {
            let mut tr = state.tr();
            tr.insert_text("X").ok()?;
            Some(tr)
        }));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("aX")
        );
        assert!(
            h.handle.selection().is_empty(),
            "cursor collapses after insert"
        );
    }

    #[test]
    fn insert_text_over_a_block_node_selection_deletes_then_inserts() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab"), hr(&s)]));
        let sel = h
            .handle
            .node_selection_at_host(children(&h, h.container_id)[1].0)
            .unwrap();
        h.handle.set_selection(sel);

        // Typing over a *block* node selection (a selected hr, where a bare text
        // node isn't valid `doc` content) deletes the node, then inserts the text
        // at the resulting cursor — landing at the end of the previous paragraph.
        assert!(h.handle.insert_text("X"));
        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 1, "the hr was removed");
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("abX"));
        assert!(
            h.handle.selection().is_empty(),
            "cursor collapses after insert"
        );
    }

    // ── IME (input method editor) ────────────────────────────────────────────

    #[test]
    fn ime_preedit_is_view_local_and_commit_inserts_one_edit() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab")]));
        h.handle.set_selection(Selection::cursor(Pos(3))); // end of "ab"

        // Composing shows a preedit overlay but never touches the document.
        h.handle.ime_set_preedit("ne", None);
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("ab"),
            "preedit is a view overlay, not part of the document"
        );

        // Commit inserts the final text as one ordinary edit.
        h.handle.ime_commit("ね");
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("abね")
        );
        // ...and it's a single undo step, exactly like typing.
        assert!(h.handle.command("undo"));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("ab")
        );
    }

    #[test]
    fn ime_clear_and_empty_commit_leave_doc_unchanged() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "ab")]));
        h.handle.set_selection(Selection::cursor(Pos(3)));

        h.handle.ime_set_preedit("xy", None);
        h.handle.ime_clear_preedit(); // composition cancelled
        h.handle.ime_commit(""); // an empty commit clears, inserts nothing
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("ab")
        );
    }

    #[test]
    fn ime_delete_surrounding_deletes_around_caret() {
        let s = schema();
        // doc(paragraph "abcd") — positions 0[p 1 a 2 b 3 c 4 d 5]6.
        let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
        h.handle.set_selection(Selection::cursor(Pos(3))); // between "b" and "c"
        h.handle.ime_delete_surrounding(1, 1); // delete "b" and "c"
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("ad")
        );
    }

    #[test]
    fn ime_methods_are_safe_before_mount() {
        let s = Rc::new(schema());
        let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());
        h.set_selection(Selection::cursor(Pos(1))); // inside the empty paragraph
        // No view yet → preedit overlay ops are safe no-ops, but commit still edits
        // the owned state.
        h.ime_set_preedit("ab", None);
        h.ime_clear_preedit();
        h.ime_commit("hi");
        // "hi" landed in the document even though the editor isn't mounted.
        let doc = h.doc();
        assert_eq!(doc.child_count(), 1);
        assert_eq!(doc.child(0).child(0).text(), Some("hi"));
    }

    // ── Deferred mount (create_editor → load → attach) ───────────────────────

    fn empty_doc(s: &Schema) -> Node {
        s.branch(
            "doc",
            Fragment::from_node(s.branch("paragraph", Fragment::empty()).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn on_change_fires_for_edits_but_not_loads_or_selection() {
        use std::cell::Cell;

        let s = Rc::new(schema());
        let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());

        let hits = Rc::new(Cell::new(0u32));
        h.on_change({
            let hits = hits.clone();
            move || hits.set(hits.get() + 1)
        });

        // A programmatic load is not a user edit — must not fire (otherwise an
        // autosave consumer would re-save every freshly opened document).
        assert!(h.load_html("<p>hello world</p>"));
        assert_eq!(hits.get(), 0, "load_doc/load_html must not fire on_change");

        // Typing (funnels through `update`) is an edit → fires.
        assert!(h.insert_text("!"));
        assert_eq!(hits.get(), 1, "an edit must fire on_change");

        // A command that changes the doc → fires.
        h.set_selection(Selection::text(Pos(1), Pos(3)));
        let before = hits.get();
        assert!(h.command("toggleBold"));
        assert_eq!(hits.get(), before + 1, "a doc-changing command must fire");

        // Selection-only movement leaves the doc identical → must not fire.
        let before = hits.get();
        h.set_selection(Selection::cursor(Pos(2)));
        assert_eq!(hits.get(), before, "selection-only change must not fire");
    }

    #[test]
    fn on_change_callback_may_reenter_the_handle() {
        use std::cell::Cell;

        // The realistic autosave shape: the callback reads the document back out.
        // This double-borrows the inner RefCell unless the notifier drops its
        // borrow first, so this test is the regression guard for that.
        let s = Rc::new(schema());
        let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());

        let seen = Rc::new(Cell::new(0usize));
        h.on_change({
            let h = h.clone();
            let seen = seen.clone();
            move || seen.set(h.doc().child_count())
        });

        assert!(h.insert_text("abc"));
        assert!(
            seen.get() > 0,
            "callback should have read the doc without panicking"
        );
    }

    #[test]
    fn unmounted_handle_is_safe_and_edits_state() {
        let s = Rc::new(schema());
        let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());

        // No host projection yet → view ops are safe no-ops, not panics.
        assert_eq!(h.container_id(), 0);
        assert_eq!(h.caret_address(Pos(0)), None);
        assert_eq!(h.pos_at(1, 0), None);
        assert_eq!(h.set_caret_blink(true), None);
        h.update_caret();

        // State edits still apply before mount (they render when the view attaches).
        assert!(h.load_html("<p>hi there</p>"));
        assert_eq!(h.doc().child_count(), 1);
    }

    #[test]
    fn attach_projects_state_loaded_before_mount() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let container_id = doc.borrow_mut().create_element("div");
        let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
        let s = Rc::new(schema());
        let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());

        // Load while unmounted (state only), then attach: the first view build
        // renders the loaded content directly.
        assert!(h.load_html("<p>before mount</p>"));
        h.attach(container, Rc::downgrade(&doc));

        assert_eq!(h.container_id(), container_id.0);
        let blocks = doc.borrow().get_children(container_id);
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            doc.borrow().text_content(blocks[0]).as_deref(),
            Some("before mount")
        );
    }

    // ── Collaboration (design M9, the `collaboration` feature) ───────────────
    //
    // Two real `EditorHandle`s (each over its own mock host) wired into a single
    // in-process loopback — the exact seam the two-pane demo uses. The convergence
    // assertions exercise the whole wiring: a local edit's `record_local` →
    // `save_incremental` → `outbound`, and the peer's `collab_receive` →
    // `integrate_incremental` → re-projection.
    #[cfg(feature = "collaboration")]
    mod collab {
        use super::*;
        use std::cell::Cell;

        /// The concatenated text of every block in a handle's document, blocks
        /// joined by `\n` — a cheap, layout-free convergence probe.
        fn doc_text(h: &EditorHandle) -> String {
            fn collect(n: &Node, out: &mut String) {
                if let Some(t) = n.text() {
                    out.push_str(t);
                    return;
                }
                for i in 0..n.child_count() {
                    collect(n.child(i), out);
                }
            }
            let doc = h.doc();
            let mut s = String::new();
            for i in 0..doc.child_count() {
                if i > 0 {
                    s.push('\n');
                }
                collect(doc.child(i), &mut s);
            }
            s
        }

        /// Wire `host` and `guest` into a synchronous in-process loopback: each
        /// side's outbound delta is delivered straight to the other's
        /// `collab_receive`. Returns after the guest has adopted the host's
        /// document.
        fn loopback(host: &EditorHandle, guest: &EditorHandle) {
            let guest_in = guest.clone();
            let snapshot = host
                .start_collaboration_host(move |delta| {
                    guest_in.collab_receive(&delta);
                })
                .expect("host projects its document");
            let host_in = host.clone();
            guest
                .start_collaboration_guest(&snapshot, move |delta| {
                    host_in.collab_receive(&delta);
                })
                .expect("guest joins from the snapshot");
        }

        #[test]
        fn guest_adopts_host_document_on_join() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "shared title")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "stale local")])).handle;
            loopback(&host, &guest);
            assert_eq!(
                doc_text(&guest),
                "shared title",
                "the guest adopts the host's converged document"
            );
            assert!(host.is_collaborating() && guest.is_collaborating());
        }

        #[test]
        fn local_edits_converge_both_directions() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);

            // Type in the host → the guest converges.
            host.set_selection(Selection::cursor(Pos(6))); // end of "hello"
            assert!(host.insert_text(" world"));
            assert_eq!(doc_text(&host), "hello world");
            assert_eq!(
                doc_text(&guest),
                "hello world",
                "host edit reached the guest"
            );

            // Type in the guest → the host converges.
            guest.set_selection(Selection::cursor(Pos(1))); // start of the block
            assert!(guest.insert_text("X"));
            assert_eq!(doc_text(&guest), "Xhello world");
            assert_eq!(
                doc_text(&host),
                "Xhello world",
                "guest edit reached the host"
            );
        }

        #[test]
        fn marks_converge() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "abcd")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);

            host.set_selection(Selection::text(Pos(1), Pos(5)));
            assert!(host.command("toggleBold"));
            // The guest's projected document carries the bold mark on the run.
            let guest_doc = guest.doc();
            let run = guest_doc.child(0).child(0);
            assert_eq!(run.text(), Some("abcd"));
            assert!(
                !run.marks().is_empty(),
                "the bold mark projected through the CRDT to the guest"
            );
        }

        #[test]
        fn concurrent_edits_converge() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

            // Buffer deltas instead of delivering them, so both peers edit against
            // the same base — a genuine concurrent edit.
            let to_guest: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let tg = to_guest.clone();
            let snapshot = host
                .start_collaboration_host(move |d| tg.borrow_mut().push(d))
                .unwrap();
            let th = to_host.clone();
            guest
                .start_collaboration_guest(&snapshot, move |d| th.borrow_mut().push(d))
                .unwrap();

            // Concurrent: host appends, guest prepends — neither has seen the other.
            host.set_selection(Selection::cursor(Pos(6)));
            assert!(host.insert_text("H"));
            guest.set_selection(Selection::cursor(Pos(1)));
            assert!(guest.insert_text("G"));

            // Exchange both deltas.
            for d in to_guest.borrow_mut().drain(..) {
                guest.collab_receive(&d);
            }
            for d in to_host.borrow_mut().drain(..) {
                host.collab_receive(&d);
            }

            // Automerge convergence: identical documents on both peers.
            let h = doc_text(&host);
            let g = doc_text(&guest);
            assert_eq!(h, g, "concurrent edits converge to one document");
            assert!(
                h.contains('H') && h.contains('G'),
                "both edits survived: {h}"
            );
        }

        /// Run the Automerge sync protocol between two handles until both go quiet,
        /// each side keeping its own [`SyncState`] — the shape a networked caller uses.
        fn sync_until_quiet(a: &EditorHandle, b: &EditorHandle) {
            use rinch_editor_collab::SyncState;
            let mut a_state = SyncState::new();
            let mut b_state = SyncState::new();
            for _ in 0..20 {
                let a_to_b = a.collab_generate_sync_message(&mut a_state);
                if let Some(m) = a_to_b.clone() {
                    b.collab_receive_sync_message(&mut b_state, m);
                }
                let b_to_a = b.collab_generate_sync_message(&mut b_state);
                if let Some(m) = b_to_a.clone() {
                    a.collab_receive_sync_message(&mut a_state, m);
                }
                if a_to_b.is_none() && b_to_a.is_none() {
                    return;
                }
            }
            panic!("sync protocol did not settle");
        }

        #[test]
        fn the_sync_protocol_converges_two_peers_whose_deltas_never_arrived() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

            // Deltas go nowhere: this models the transport the sync protocol exists
            // for — an HTTP poll, a dropped socket, a peer that was offline. The
            // broadcast path alone would leave these two permanently diverged.
            let snapshot = host.start_collaboration_host(|_d| {}).unwrap();
            guest.start_collaboration_guest(&snapshot, |_d| {}).unwrap();

            host.set_selection(Selection::cursor(Pos(6)));
            assert!(host.insert_text(" world"));
            guest.set_selection(Selection::cursor(Pos(1)));
            assert!(guest.insert_text("G"));
            assert_ne!(
                doc_text(&host),
                doc_text(&guest),
                "precondition: no delta was delivered, so the peers diverged"
            );

            sync_until_quiet(&host, &guest);

            let h = doc_text(&host);
            assert_eq!(h, doc_text(&guest), "the sync protocol converged the peers");
            assert!(
                h.contains("world") && h.contains('G'),
                "both offline edits survived: {h}"
            );
            assert_eq!(
                host.collab_heads(),
                guest.collab_heads(),
                "converged peers share one change frontier"
            );
        }

        #[test]
        fn sync_methods_are_inert_without_a_session() {
            use rinch_editor_collab::SyncState;
            let s = schema();
            let solo = mount(doc_node(&s, vec![para(&s, "local only")])).handle;
            let mut state = SyncState::new();
            assert!(solo.collab_generate_sync_message(&mut state).is_none());
            assert!(solo.collab_heads().is_none());
            assert_eq!(doc_text(&solo), "local only", "the document is untouched");
        }

        #[test]
        fn integrating_a_sync_message_does_not_echo() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

            let snapshot = host.start_collaboration_host(|_d| {}).unwrap();
            // A sync-integrated change must not fire the broadcast sink either — a
            // caller running both transports would otherwise loop.
            let guest_emits = Rc::new(Cell::new(0usize));
            let ge = guest_emits.clone();
            guest
                .start_collaboration_guest(&snapshot, move |_d| ge.set(ge.get() + 1))
                .unwrap();

            host.set_selection(Selection::cursor(Pos(3)));
            assert!(host.insert_text("c"));
            sync_until_quiet(&host, &guest);

            assert_eq!(doc_text(&guest), "abc", "host edit reached the guest");
            assert_eq!(
                guest_emits.get(),
                0,
                "integrating a sync message must not broadcast a delta back"
            );
        }

        #[test]
        fn integrating_a_remote_delta_does_not_echo() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

            let guest_in = guest.clone();
            let snapshot = host
                .start_collaboration_host(move |d| {
                    guest_in.collab_receive(&d);
                })
                .unwrap();
            // Count the guest's outbound emissions: integrating the host's delta
            // must NOT produce one (no echo / infinite loop).
            let guest_emits = Rc::new(Cell::new(0usize));
            let ge = guest_emits.clone();
            guest
                .start_collaboration_guest(&snapshot, move |_d| ge.set(ge.get() + 1))
                .unwrap();

            host.set_selection(Selection::cursor(Pos(3)));
            assert!(host.insert_text("c"));
            assert_eq!(doc_text(&guest), "abc", "host edit applied on the guest");
            assert_eq!(
                guest_emits.get(),
                0,
                "integrating a remote delta must not broadcast one back"
            );
        }

        #[test]
        fn selection_only_change_broadcasts_nothing() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "abc")])).handle;
            let emits = Rc::new(Cell::new(0usize));
            let e = emits.clone();
            host.start_collaboration_host(move |_d| e.set(e.get() + 1))
                .unwrap();

            host.set_selection(Selection::cursor(Pos(2)));
            assert_eq!(emits.get(), 0, "moving the cursor broadcasts nothing");
            assert!(host.insert_text("X"));
            assert_eq!(emits.get(), 1, "a text edit broadcasts exactly one delta");
        }

        #[test]
        fn stop_collaboration_silences_broadcasts() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
            let emits = Rc::new(Cell::new(0usize));
            let e = emits.clone();
            host.start_collaboration_host(move |_d| e.set(e.get() + 1))
                .unwrap();
            assert!(host.is_collaborating());

            host.stop_collaboration();
            assert!(!host.is_collaborating());
            host.set_selection(Selection::cursor(Pos(3)));
            assert!(host.insert_text("c"));
            assert_eq!(emits.get(), 0, "a detached editor broadcasts nothing");
        }

        #[test]
        fn late_joiner_adopts_current_content_via_snapshot() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);

            // Edit AFTER the original join snapshot.
            host.set_selection(Selection::cursor(Pos(6)));
            assert!(host.insert_text(" world"));
            assert_eq!(doc_text(&guest), "hello world");

            // A third peer joins late from the host's CURRENT snapshot — it must
            // adopt the edited content, not the host's original document.
            let late = mount(doc_node(&s, vec![para(&s, "stale")])).handle;
            let snapshot = host.collab_snapshot().expect("host is collaborating");
            late.start_collaboration_guest(&snapshot, |_d| {}).unwrap();
            assert_eq!(
                doc_text(&late),
                "hello world",
                "a late joiner adopts the current shared document"
            );
        }

        #[test]
        fn unsupported_local_edit_fails_loud_without_touching_the_peer() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "ok")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);
            assert_eq!(doc_text(&guest), "ok");

            // A blockquote is still outside the projected scope (lists are supported
            // now; blockquote / tables / task lists / inline atoms are not).
            assert!(host.load_html("<blockquote><p>quoted</p></blockquote>"));

            // The host's model changed locally, but the projection failed loud (the
            // CRDT was left untouched, all-or-nothing) so the peer received nothing.
            assert!(
                host.collab_take_error().is_some(),
                "an unsupported edit surfaces a fail-loud error"
            );
            assert_eq!(
                doc_text(&guest),
                "ok",
                "the peer is untouched by an unsupported local edit (no partial sync)"
            );
        }

        #[test]
        fn list_edits_sync_to_the_peer() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "ok")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);
            assert_eq!(doc_text(&guest), "ok");

            // Lists are inside the projected scope, so this must sync rather than
            // fail loud (the counterpart to the blockquote case above).
            assert!(host.load_html("<ul><li><p>item</p></li></ul>"));
            assert!(
                host.collab_take_error().is_none(),
                "a bullet list is supported and must not fail loud"
            );
            assert_eq!(
                doc_text(&guest),
                "item",
                "the peer receives the list content"
            );

            // A nested list survives the round-trip too.
            assert!(host.load_html("<ol><li><p>a</p><ul><li><p>b</p></li></ul></li></ol>"));
            assert!(host.collab_take_error().is_none());
            assert_eq!(
                doc_text(&guest),
                "ab",
                "nested list content reaches the peer"
            );
        }

        /// Seeded fuzz over the real `EditorHandle` wiring: two handles relay random
        /// edits (via the `outbound` sink) and integrate them (`collab_receive`) in
        /// interleaved order, then must converge to the *identical* document
        /// (structural — marks included, which is what the mark-order fix guarantees).
        #[test]
        fn fuzz_handle_wiring_converges() {
            struct Rng(u64);
            impl Rng {
                fn new(s: u64) -> Rng {
                    Rng(s ^ 0x9E37_79B9_7F4A_7C15)
                }
                fn next(&mut self) -> u64 {
                    let mut x = self.0;
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    self.0 = x;
                    x
                }
                fn below(&mut self, n: usize) -> usize {
                    if n == 0 {
                        0
                    } else {
                        (self.next() % n as u64) as usize
                    }
                }
            }

            for seed in 0..6u64 {
                let s = schema();
                let host = mount(doc_node(&s, vec![para(&s, "start")])).handle;
                let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
                // Each peer's outbound pushes into the OTHER peer's inbox.
                let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
                let to_guest: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
                let tg = to_guest.clone();
                let snap = host
                    .start_collaboration_host(move |d| tg.borrow_mut().push(d))
                    .unwrap();
                let th = to_host.clone();
                guest
                    .start_collaboration_guest(&snap, move |d| th.borrow_mut().push(d))
                    .unwrap();

                let peers = [&host, &guest];
                let inbox = [&to_host, &to_guest]; // inbox[p] = deltas destined for peer p
                let mut seen = [0usize, 0usize];
                let mut rng = Rng::new(seed);

                let deliver = |p: usize, seen: &mut [usize; 2]| {
                    let delta = {
                        let q = inbox[p].borrow();
                        (seen[p] < q.len()).then(|| q[seen[p]].clone())
                    };
                    if let Some(d) = delta {
                        seen[p] += 1;
                        peers[p].collab_receive(&d);
                    }
                };

                for _ in 0..140 {
                    if rng.below(100) < 65 {
                        let p = rng.below(2);
                        let h = peers[p];
                        let doc = h.doc();
                        let size = doc.content().size();
                        let pos = 1 + rng.below(size.max(1));
                        h.set_selection(Selection::near(&doc, Pos(pos.min(size)), 1));
                        match rng.below(5) {
                            0..=2 => {
                                h.insert_text(["a", "b", " ", "猫", "Z"][rng.below(5)]);
                            }
                            3 => {
                                h.command(
                                    ["toggleBold", "toggleItalic", "toggleStrike", "toggleCode"]
                                        [rng.below(4)],
                                );
                            }
                            _ => {
                                h.command(
                                    ["setHeading1", "setParagraph", "splitBlock"][rng.below(3)],
                                );
                            }
                        }
                    } else {
                        deliver(rng.below(2), &mut seen);
                    }
                }
                // Flush both inboxes.
                for p in 0..2 {
                    while seen[p] < inbox[p].borrow().len() {
                        deliver(p, &mut seen);
                    }
                }

                // Compare via HTML, not `Node` equality: each handle has its OWN
                // `Schema` instance (`create_editor`/`mount` make one per editor), and
                // `Node` equality compares `NodeType` by interned-pointer identity, so
                // structurally-identical docs from different schemas compare unequal.
                // HTML is schema-independent and captures text + marks + block type, so
                // it's the faithful cross-handle convergence check. (Full structural
                // convergence under one schema is proven by the adapter fuzz.)
                use rinch_editor_core::serialize::node_to_html;
                assert_eq!(
                    node_to_html(&host.doc()),
                    node_to_html(&guest.doc()),
                    "handles diverged (seed={seed})"
                );
            }
        }
    }
}
