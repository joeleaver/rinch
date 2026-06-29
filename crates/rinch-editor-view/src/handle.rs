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
use rinch_editor_core::commands::{current_block_type, in_node_type, is_mark_active};
use rinch_editor_core::model::{Fragment, Slice};
use rinch_editor_core::serialize::{
    slice_from_html, slice_from_text, slice_to_html, slice_to_text,
};
use rinch_editor_core::{
    EditorState, EditorView, Node, Plugin, Pos, Schema, Selection, Transaction,
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
    fn commit(&mut self, prev: EditorState, next: EditorState) {
        self.state = next.clone();
        if let Some(view) = self.view.as_mut() {
            view.update_dom(&prev, &next);
        }
        #[cfg(feature = "collaboration")]
        self.record_local(&prev, &next);
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
        super::registry::register_editor(container.node_id().0, self.clone());
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
        core.commit(prev, next);
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
        core.commit(prev, next);
        true
    }

    /// Whether the named command currently applies (toolbar enablement).
    pub fn can_run(&self, name: &str) -> bool {
        self.inner.borrow().state.can_run(name)
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

    /// Move the selection (and re-project, so the caret follows once geometry lands).
    pub fn set_selection(&self, selection: Selection) {
        self.update(|state| {
            let mut tr = state.tr();
            tr.set_selection(selection.clone());
            Some(tr)
        });
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
        core.commit(prev, next);
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
    pub(crate) fn set_caret_blink(&self, visible: bool) -> Option<bool> {
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
    /// transport between existing peers — the full Automerge sync protocol (for
    /// lossy / out-of-order reconciliation) is not yet exposed through the handle.
    pub fn collab_snapshot(&self) -> Option<Vec<u8>> {
        self.inner
            .borrow_mut()
            .collab
            .as_mut()
            .map(|b| b.session.snapshot())
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

            // A bullet list is a nested block — outside the staged flat-text scope.
            assert!(host.load_html("<ul><li><p>item</p></li></ul>"));

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
