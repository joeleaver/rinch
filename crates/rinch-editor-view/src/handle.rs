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
use std::collections::HashMap;
use std::fmt;
use std::rc::{Rc, Weak};

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_editor_core::commands::{current_block_type, in_node_type, is_mark_active, marks_at};
use rinch_editor_core::model::{Fragment, Slice};
use rinch_editor_core::serialize::{
    slice_from_html, slice_from_text, slice_to_html, slice_to_text,
};
use rinch_editor_core::transform::Mapping;
use rinch_editor_core::{
    CursorMotion, EditorState, EditorView, KeyBinding, Node, Plugin, Pos, Schema, Selection,
    Transaction, ViewRequest, apply_input_rules,
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
    /// Selections captured by asynchronous work still in flight — see
    /// [`SelectionAnchor`]. Empty for every editor that has none, so the
    /// mutation path's carry step is a cheap early return.
    anchors: Rc<RefCell<AnchorMap>>,
    /// Whether local edits are refused — see [`EditorHandle::set_read_only`].
    /// Enforced in exactly one place: [`Self::refuses`], which [`Self::commit`]
    /// asks before it stores anything. Every local change lands in `commit`, so
    /// an input path added later is read-only without knowing this flag exists.
    read_only: bool,
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
    fn commit(
        &mut self,
        prev: EditorState,
        next: EditorState,
        mapping: Option<&Mapping>,
    ) -> Option<bool> {
        if self.refuses(&prev, &next, mapping.is_none()) {
            return None;
        }
        let doc_changed = !prev.doc.same_ref(&next.doc);
        if doc_changed {
            self.carry_anchors(&next.doc, mapping);
        }
        self.state = next.clone();
        if let Some(view) = self.view.as_mut() {
            view.update_dom(&prev, &next);
        }
        #[cfg(feature = "collaboration")]
        self.record_local(&prev, &next);
        Some(doc_changed)
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
    fn refuses(&self, prev: &EditorState, next: &EditorState, is_load: bool) -> bool {
        if !self.read_only {
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

    /// Carry every live [`SelectionAnchor`] across a document change, so an
    /// asynchronous operation still inserts where the user asked for it.
    ///
    /// With a `mapping`, each anchor is re-mapped and re-resolved against the new
    /// document (`Selection::map` falls back to the nearest valid selection if its
    /// textblock went away). Without one — a `load_doc`, or a remote
    /// re-projection, where the new document has no positional relationship to the
    /// old — the anchor is invalidated instead of being silently pointed at
    /// unrelated content.
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
        match bridge
            .session
            .record_local(next.schema(), &prev.doc, &next.doc)
        {
            // An empty delta means the projection produced no CRDT change at all, so
            // there is nothing for peers to apply.
            Ok(()) => match bridge.session.save_incremental() {
                Ok(delta) if !delta.is_empty() => (bridge.outbound)(delta),
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

/// Selections captured by in-flight asynchronous work, each carried forward
/// through every document change until its operation completes.
///
/// Lives behind its **own** `RefCell`, not inside [`EditorCore`], on purpose: a
/// [`SelectionAnchor`] releases itself when dropped, and that drop can land while
/// the core is borrowed (a callback dropping its anchor from inside an `update`
/// closure). Borrowing only this map keeps the release from being a double-borrow
/// panic.
#[derive(Default)]
struct AnchorMap {
    next_id: u64,
    /// `None` marks an anchor whose document is gone — see
    /// [`EditorCore::carry_anchors`].
    live: HashMap<u64, Option<Selection>>,
}

/// A selection captured for an operation that finishes *later*, kept pointing at
/// the same content as the user keeps editing.
///
/// The problem it solves: an asynchronous paste is dispatched when the user hits
/// Ctrl+V but lands once the clipboard answers — and because the UI no longer
/// freezes while that happens (issue #149), the user can type in between. A raw
/// position captured at dispatch would be stale by then; the live caret would be
/// wherever they wandered to. An anchor is neither: it is the captured selection
/// **mapped through the intervening steps**, which is what a transactional editor
/// can offer and a `contenteditable` one cannot.
///
/// Obtain one from [`EditorHandle::anchor_selection`]. Dropping it releases the
/// capture, so an operation that is abandoned leaves nothing behind.
///
/// ```ignore
/// let anchor = handle.anchor_selection();
/// paste_text_async(move |text| {
///     // ... marshalled back onto the UI thread ...
///     if let Some(sel) = anchor.selection() {
///         handle.set_selection(sel);
///         handle.replace_selection_with_text(&text);
///     }
/// });
/// ```
pub struct SelectionAnchor {
    anchors: Weak<RefCell<AnchorMap>>,
    id: u64,
}

impl SelectionAnchor {
    /// Where the captured selection sits **now**, after every change applied
    /// since it was taken.
    ///
    /// `None` once the anchor can no longer mean anything: the editor was
    /// dropped, or the document it pointed into was replaced wholesale
    /// (`load_doc`/`load_html`, or a collaborative re-projection). A caller
    /// should then abandon the operation rather than guess — the content the
    /// user aimed at is gone.
    pub fn selection(&self) -> Option<Selection> {
        let anchors = self.anchors.upgrade()?;
        let anchors = anchors.borrow();
        anchors.live.get(&self.id).cloned().flatten()
    }
}

impl Drop for SelectionAnchor {
    fn drop(&mut self) {
        if let Some(anchors) = self.anchors.upgrade() {
            // A `Selection` drop runs no user code, so this cannot re-enter.
            if let Ok(mut anchors) = anchors.try_borrow_mut() {
                anchors.live.remove(&self.id);
            }
        }
    }
}

impl fmt::Debug for SelectionAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectionAnchor")
            .field("id", &self.id)
            .field("selection", &self.selection())
            .finish()
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
                anchors: Rc::new(RefCell::new(AnchorMap::default())),
                read_only: false,
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
                anchors: Rc::new(RefCell::new(AnchorMap::default())),
                read_only: false,
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
        // The switch lives on the handle, so one set before mount (or across a
        // re-mount) is on the new container from its first frame.
        view.set_read_only(core.read_only);
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
    /// whether a transaction was applied — `false`, too, when the editor is
    /// [read-only](Self::set_read_only) and the transaction would have changed the
    /// document. The single dispatch path — `command`, keyboard insert, paste, and
    /// IME commit all funnel through here.
    pub fn update(&self, build: impl FnOnce(&EditorState) -> Option<Transaction>) -> bool {
        let mut core = self.inner.borrow_mut();
        let Some(tr) = build(&core.state) else {
            return false;
        };
        let prev = core.state.clone();
        // The mapping has to be taken before `apply` consumes the transaction.
        let mapping = tr.mapping().clone();
        let next = core.state.apply(tr);
        let Some(doc_changed) = core.commit(prev, next, Some(&mapping)) else {
            return false;
        };
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }

    /// Add `plugin` to this editor, rebuilding its state over the **current**
    /// document and selection.
    ///
    /// The plugin list is otherwise fixed at construction
    /// ([`create_editor`](super::create_editor) installs
    /// [`default_plugins`](rinch_editor_core::default_plugins)), which leaves an app
    /// no way to contribute one of its own — a spellchecker's decorations, say. This
    /// is that seam.
    ///
    /// Rebuilding the state **discards every plugin's folded state**, undo history
    /// included, so this is a construction-time call: add the plugin to a freshly
    /// created handle, before any content is loaded or typed. Adding a key that is
    /// already installed is a no-op, so calling it twice is harmless.
    ///
    /// Returns whether the plugin was added.
    pub fn add_plugin(&self, plugin: Rc<dyn Plugin>) -> bool {
        let mut core = self.inner.borrow_mut();
        if core.plugins.iter().any(|p| p.key() == plugin.key()) {
            return false;
        }
        core.plugins.push(plugin);
        let prev = core.state.clone();
        let mut next =
            EditorState::create(core.schema.clone(), prev.doc.clone(), core.plugins.clone());
        next.selection = prev.selection.clone();
        // No mapping: the document is unchanged, so nothing needs remapping, and
        // `commit` treats a `None` mapping as a load — which this is, in the sense
        // that matters (state replaced wholesale rather than stepped forward).
        core.commit(prev, next, None).is_some()
    }

    /// Run the named command (applying + re-projecting if it applies). Returns
    /// whether it applied — a [read-only](Self::set_read_only) editor refuses every
    /// command that would change the document. The toolbar/keymap entry point.
    pub fn command(&self, name: &str) -> bool {
        let mut core = self.inner.borrow_mut();
        let Some((next, mapping)) = core.state.run_mapped(name) else {
            return false;
        };
        let prev = core.state.clone();
        let Some(doc_changed) = core.commit(prev, next, Some(&mapping)) else {
            return false;
        };
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }

    /// Capture the current selection for an operation that will finish later,
    /// returning an anchor that stays pointed at the same content as the user
    /// keeps editing.
    ///
    /// This is what makes an asynchronous insertion land where the user asked for
    /// it. The built-in paste uses it: Ctrl+V anchors the selection, the clipboard
    /// read runs off the UI thread (so the user can keep typing — issue #149), and
    /// the content is inserted at the anchor, mapped through everything typed in
    /// the meantime.
    ///
    /// The anchor releases itself when dropped, and reports `None` from
    /// [`SelectionAnchor::selection`] once its document has been replaced.
    pub fn anchor_selection(&self) -> SelectionAnchor {
        let core = self.inner.borrow();
        let anchors = core.anchors.clone();
        let id = {
            let mut map = anchors.borrow_mut();
            map.next_id += 1;
            let id = map.next_id;
            map.live.insert(id, Some(core.state.selection.clone()));
            id
        };
        SelectionAnchor {
            anchors: Rc::downgrade(&anchors),
            id,
        }
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
    ///
    /// In a [read-only](Self::set_read_only) editor that means "applies **and**
    /// would not be refused": `can_run("toggleBold")` is `false`, `can_run(
    /// "selectAll")` still `true`, so a toolbar that greys its buttons from here
    /// goes inert with the switch.
    ///
    /// Answered by running the command against the state and asking the same rule
    /// the gate asks, because whether a command changes the document is not knowable
    /// without computing what it would do. Nothing is committed — the resulting
    /// state is discarded — but this is **not** the cheap `Command`-with-`None`
    /// applicability query the editable path uses: the command's dispatch branch
    /// runs, building the transaction and folding every plugin (the history plugin
    /// inverts every step). So a read-only toolbar that re-queries twenty buttons on
    /// every caret move pays twenty transaction builds; ask
    /// [`is_read_only`](Self::is_read_only) once instead and grey them all. It also
    /// means an app-supplied command whose dispatch branch has a side effect of its
    /// own would see it fire from a query — no command in `rinch-editor-core` has
    /// one. Tracked for a cheaper answer.
    pub fn can_run(&self, name: &str) -> bool {
        let core = self.inner.borrow();
        if !core.read_only {
            return core.state.can_run(name);
        }
        core.state
            .run_mapped(name)
            .is_some_and(|(next, _)| !core.refuses(&core.state, &next, false))
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
        let mut core = self.inner.borrow_mut();
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
            core.commit(prev, next, Some(&mapping));
        }
        core.read_only = read_only;
        if let Some(view) = core.view.as_mut() {
            if read_only {
                view.set_preedit("");
            }
            view.set_read_only(read_only);
        }
        drop(core);
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
        //
        // No mapping: the new document is unrelated to the old one, so every
        // outstanding `SelectionAnchor` is invalidated rather than remapped onto
        // whatever now happens to sit at those offsets.
        core.commit(prev, next, None).is_some()
    }

    /// Parse `html` (schema-whitelisted) and load it as the document. Empty or
    /// whitespace-only `html` loads a single empty paragraph (via [`Self::load_doc`]),
    /// not a block-less doc. Returns `false` if `html` fails to parse at all, or if
    /// the editor is read-only and collaborating (see [`Self::load_doc`]).
    pub fn load_html(&self, html: &str) -> bool {
        let schema = self.inner.borrow().schema.clone();
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
        let Some((next, mapping)) = core.state.run_command_mapped(&cmd) else {
            return false;
        };
        let prev = core.state.clone();
        let Some(doc_changed) = core.commit(prev, next, Some(&mapping)) else {
            return false;
        };
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
        let Some((next, mapping)) = core.state.run_command_mapped(&cmd) else {
            return false;
        };
        let prev = core.state.clone();
        let Some(doc_changed) = core.commit(prev, next, Some(&mapping)) else {
            return false;
        };
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
    ///
    /// This is also where the view's [`ViewRequest`]s are *fulfilled*. The view owns
    /// no window and no scroll container, so it hands back what it needs as data
    /// (design §6); the handle is the nearest thing to a runtime that both platforms
    /// share, so it turns `ScrollSelectionIntoView` into a
    /// [`scroll_into_view`](rinch_core::dom::NodeHandle::scroll_into_view) on the
    /// view's *scroll anchor* — which element that is stays the view's knowledge.
    /// The backend then does the minimal "nearest" scroll of the caret's closest
    /// scroll container (immediately on web, after the next layout on desktop). The
    /// view emits the request only when the selection overlay actually moved, so a
    /// pass that re-renders the caret where it already was never drags a user who
    /// has scrolled away back to it.
    pub fn update_caret(&self) -> bool {
        let mut core = self.inner.borrow_mut();
        let state = core.state.clone();
        match core.view.as_mut() {
            Some(view) => {
                for request in view.update_caret(&state) {
                    match request {
                        ViewRequest::ScrollSelectionIntoView => {
                            if let Some(anchor) = view.scroll_anchor() {
                                anchor.scroll_into_view();
                            }
                        }
                    }
                }
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
    ///
    /// A [read-only](Self::set_read_only) editor shows no composition: the commit
    /// it would lead to is refused, so the overlay would be text that can never
    /// land.
    pub fn ime_set_preedit(&self, text: &str, _cursor: Option<(usize, usize)>) {
        let mut core = self.inner.borrow_mut();
        let text = if core.read_only { "" } else { text };
        if let Some(view) = core.view.as_mut() {
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
        let mut core = self.inner.borrow_mut();
        let session = CollabSession::new(&core.state)?;
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
        // Adopt the host's document first, with **no** session attached, so the load
        // is not recorded onto a CRDT — the new session already holds this content,
        // and a session left attached from an earlier document must not have this
        // one written over what its peers share. A read-only editor depends on the
        // same thing: with no session attached, a load is never refused. Then
        // attach the matching session.
        let schema = self.inner.borrow().schema.clone();
        let doc = session.projected_doc(&schema)?;
        self.inner.borrow_mut().collab = None;
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
                // A remote integration rebuilds the document rather than applying
                // mapped local steps, so there is no mapping to carry an anchor
                // across; invalidate instead of guessing (see `carry_anchors`).
                core.carry_anchors(&next.doc, None);
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
        self.inner
            .borrow()
            .collab
            .as_ref()
            .map(|b| b.session.snapshot())
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
        let mut core = self.inner.try_borrow_mut().ok()?;
        let bridge = core.collab.as_mut()?;
        match bridge.session.sync_diff(remote_state_vector) {
            Ok(diff) => Some(diff),
            Err(e) => {
                bridge.last_error = Some(e);
                None
            }
        }
    }

    /// Detach the collaboration session (stop projecting and broadcasting). The
    /// document is unchanged; subsequent edits are local-only again.
    pub fn stop_collaboration(&self) {
        self.inner.borrow_mut().collab = None;
    }

    /// Why this editor's **outbound** collaboration is currently refusing, if it is
    /// (issue #220): a local edit outside the staged A22 scope — a pasted table, a
    /// `blockquote` wrap — cannot be projected onto the CRDT, so this edit and every
    /// one after it stays local until that content is removed.
    ///
    /// Unlike [`Self::collab_take_error`] this does **not** clear: it stays `Some` for
    /// as long as the condition holds, so it is what an app should drive a persistent
    /// "not syncing — remove the table to resume" indicator from. It clears itself the
    /// moment a local edit projects again, and that same edit broadcasts everything
    /// that accumulated meanwhile.
    ///
    /// This is deliberately **not** [poison](Self::is_collaboration_poisoned): the
    /// shared document is healthy throughout, inbound integration keeps working, and
    /// recovery needs no rejoin. `None` when not collaborating.
    ///
    /// Uses `try_borrow` — soft, like [`Self::collab_receive`] — so an `outbound`
    /// callback may call it re-entrantly.
    pub fn collab_outbound_stall(&self) -> Option<CollabError> {
        let core = self.inner.try_borrow().ok()?;
        core.collab
            .as_ref()
            .and_then(|b| b.session.outbound_stall().cloned())
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
        /// The *same* allocation as `doc`, typed: the mock's test-only helpers are
        /// inherent methods and unreachable through `dyn DomDocument`.
        mock: Rc<RefCell<MockDomDocument>>,
        container_id: NodeId,
        handle: EditorHandle,
    }

    fn mount(html_blocks: Node) -> Harness {
        let mock = Rc::new(RefCell::new(MockDomDocument::new()));
        let doc: Rc<RefCell<dyn DomDocument>> = mock.clone();
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
            mock,
            container_id,
            handle,
        }
    }

    /// The handle→request plumbing: `update_caret` must *fulfil* the view's
    /// `ScrollSelectionIntoView` by calling `scroll_into_view()` on the caret
    /// element, and must do it only when the caret moved. The mock queues those
    /// calls the way the desktop backend does, so the queue is what we read.
    ///
    /// Before this, the returned `Vec<ViewRequest>` was dropped on the floor and
    /// `ScrollSelectionIntoView` had no consumer anywhere in the tree.
    #[test]
    fn update_caret_scrolls_the_caret_into_view_only_when_it_moved() {
        let s = schema();
        let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
        let h = mount(doc_node(&s, vec![empty(), empty()]));

        // Measure the two paragraphs so the caret has somewhere to land (the mock
        // lays nothing out; `__set_node_layout` is the injection point).
        let blocks = children(&h, h.container_id);
        {
            let mut m = h.mock.borrow_mut();
            for (i, b) in blocks.iter().enumerate() {
                m.__set_node_layout(*b, 0.0, i as f32 * 20.0, 200.0, 20.0);
            }
        }
        let drained = |h: &Harness| h.doc.borrow_mut().drain_scroll_into_view_requests();

        // doc(p(), p()) → 0[p 1]2[p 3]4.
        h.handle.set_selection(Selection::cursor(Pos(1)));
        h.handle.update_caret();
        let first = drained(&h);
        assert_eq!(first.len(), 1, "the caret's first placement scrolls to it");
        assert_eq!(
            h.doc.borrow().get_attribute(first[0], "data-pm-caret"),
            Some("true".to_string()),
            "and it is the caret element that was scrolled to"
        );

        // The runtime's post-layout pass runs on every frame; an unchanged caret
        // must not keep re-scrolling, or a user who scrolled away is dragged back.
        h.handle.update_caret();
        h.handle.update_caret();
        assert!(drained(&h).is_empty(), "a repeat pass scrolls nothing");

        h.handle.set_selection(Selection::cursor(Pos(3)));
        h.handle.update_caret();
        assert_eq!(drained(&h).len(), 1, "a moved caret scrolls again");
    }

    /// Issue #217 where a user actually meets it. `create_editor` mints a **new**
    /// `Rc<Schema>` per handle and `NodeType`/`MarkType` equality is `Rc::ptr_eq`, so
    /// the documented `doc()` → `load_doc()` pair hands one editor a document whose
    /// marks belong to another editor's schema.
    ///
    /// On `main` the next `toggleBold` over that text answered `true` and left the run
    /// carrying **two** `bold` marks — the document's real one plus a freshly added
    /// foreign twin. That is silent corruption on a first-party path, and it is what
    /// `Transform::add_mark`'s guard now refuses.
    ///
    /// What the guard does **not** do is make the pair work: `is_mark_active` still
    /// answers `false` for text that is bold, and the command now simply fails. Fixing
    /// that means re-interning the adopted document through the receiving schema, which
    /// belongs to `load_doc` rather than to the transform.
    #[test]
    fn a_document_adopted_from_another_handle_never_grows_a_duplicate_mark() {
        let s = Schema::starter_kit();
        let a = mount(doc_node(&s, vec![para(&s, "hello")]));
        a.handle.set_selection(Selection::text(Pos(1), Pos(6)));
        assert!(a.handle.command("toggleBold"), "A bolds its own text");
        assert_eq!(a.handle.doc().child(0).child(0).marks().len(), 1);

        let b = mount(doc_node(&s, vec![para(&s, "x")]));
        b.handle.load_doc(a.handle.doc());
        b.handle.set_selection(Selection::text(Pos(1), Pos(6)));
        // The command is refused rather than corrupting the run.
        assert!(
            !b.handle.command("toggleBold"),
            "a foreign-schema document must refuse the mark, not accept it"
        );
        assert_eq!(
            b.handle.doc().child(0).child(0).marks().len(),
            1,
            "exactly one bold: the document's own, with no foreign twin added beside it"
        );
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

    // ── SelectionAnchor (the asynchronous-insertion point, #149) ─────────────

    /// The property the whole anchor exists for: text typed **before** the
    /// anchored position pushes it along, so a late insertion still lands where
    /// the user asked rather than where the offset happened to point.
    #[test]
    fn an_anchor_is_carried_by_an_edit_in_front_of_it() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
        // Anchor at the space, i.e. "insert here" -> "hello| world".
        h.handle.set_selection(Selection::cursor(Pos(6)));
        let anchor = h.handle.anchor_selection();

        // The user keeps typing at the *start* of the line while we wait.
        h.handle.set_selection(Selection::cursor(Pos(1)));
        assert!(h.handle.insert_text("XY"));

        let carried = anchor.selection().expect("still valid");
        assert_eq!(
            carried.head(),
            Pos(8),
            "two characters inserted ahead of the anchor move it by two"
        );

        // Inserting there splits the original text at the same *content* point.
        h.handle.set_selection(carried);
        assert!(h.handle.replace_selection_with_text("!"));
        let p = children(&h, h.container_id)[0];
        assert_eq!(text(&h, p).as_deref(), Some("XYhello! world"));
    }

    /// An edit *after* the anchor leaves it alone — the paste does not drift
    /// toward text the user typed somewhere else.
    #[test]
    fn an_anchor_ignores_an_edit_behind_it() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
        h.handle.set_selection(Selection::cursor(Pos(6)));
        let anchor = h.handle.anchor_selection();

        h.handle.set_selection(Selection::cursor(Pos(12))); // end of line
        assert!(h.handle.insert_text("!!!"));

        assert_eq!(anchor.selection().expect("still valid").head(), Pos(6));
    }

    /// An anchored *range* (paste over a selection) survives as a range, so the
    /// late paste still replaces what the user had selected.
    #[test]
    fn an_anchored_range_stays_a_range() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
        h.handle.set_selection(Selection::text(Pos(7), Pos(12))); // "world"
        let anchor = h.handle.anchor_selection();

        // Plain characters, not "* " — that would fire the bullet-list input
        // rule and restructure the block, which is the `load_doc` case, not this
        // one.
        h.handle.set_selection(Selection::cursor(Pos(1)));
        assert!(h.handle.insert_text("AB"));

        let carried = anchor.selection().expect("still valid");
        assert_eq!((carried.from(), carried.to()), (Pos(9), Pos(14)));
        h.handle.set_selection(carried);
        assert!(h.handle.replace_selection_with_text("there"));
        let p = children(&h, h.container_id)[0];
        assert_eq!(text(&h, p).as_deref(), Some("ABhello there"));
    }

    /// A command's edits carry anchors too — `command` goes through
    /// `run_mapped`, not just `update`, so a keymap-driven edit while a paste is
    /// in flight is mapped like any other.
    #[test]
    fn a_command_carries_an_anchor() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "one"), para(&s, "two")]));
        // Anchor in the *second* paragraph.
        h.handle.set_selection(Selection::cursor(Pos(8)));
        let anchor = h.handle.anchor_selection();
        let before = anchor.selection().unwrap().head();

        // Wrapping the first paragraph in a blockquote adds an opening token in
        // front of the anchor, so the anchor must shift.
        h.handle.set_selection(Selection::cursor(Pos(2)));
        assert!(h.handle.command("wrapInBlockquote"));
        h.handle.set_selection(Selection::cursor(Pos(2)));

        let after = anchor.selection().expect("still valid").head();
        assert!(
            after.0 > before.0,
            "the anchor moved with the content the command pushed along \
             (before {before:?}, after {after:?})"
        );
    }

    /// Loading a new document invalidates outstanding anchors: the content the
    /// user aimed at no longer exists, and quietly reusing the offset would drop
    /// the paste into unrelated text.
    #[test]
    fn a_document_load_invalidates_an_anchor() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
        h.handle.set_selection(Selection::cursor(Pos(6)));
        let anchor = h.handle.anchor_selection();
        assert!(anchor.selection().is_some());

        h.handle
            .load_doc(doc_node(&s, vec![para(&s, "something else")]));
        assert!(
            anchor.selection().is_none(),
            "an anchor into a replaced document must not resolve"
        );
    }

    /// A selection-only change is not a document change, so it neither moves nor
    /// invalidates an anchor: the caret wandering is exactly the case the anchor
    /// is meant to be immune to.
    #[test]
    fn moving_the_caret_does_not_disturb_an_anchor() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
        h.handle.set_selection(Selection::cursor(Pos(6)));
        let anchor = h.handle.anchor_selection();

        h.handle.set_selection(Selection::cursor(Pos(1)));
        h.handle.set_selection(Selection::text(Pos(2), Pos(4)));

        assert_eq!(anchor.selection().expect("still valid").head(), Pos(6));
    }

    /// Dropping an anchor releases it — an abandoned asynchronous operation must
    /// not leave the editor mapping a position for the rest of its life.
    #[test]
    fn dropping_an_anchor_releases_it() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "hello")]));
        let live = || h.handle.inner.borrow().anchors.borrow().live.len();

        let a = h.handle.anchor_selection();
        let b = h.handle.anchor_selection();
        assert_eq!(live(), 2);
        drop(a);
        assert_eq!(live(), 1);
        drop(b);
        assert_eq!(live(), 0, "no anchor outlives the value that owns it");
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

    // ── Read-only (`set_read_only`) ──────────────────────────────────────────
    //
    // The gate is one check in `EditorCore::commit`, so these go through the public
    // entry points the platform glue calls — the keyboard's `insert_text` and
    // `dispatch_key`, the clipboard's `replace_selection_with_*`, the IME's
    // `ime_commit`, a toolbar's `command` — and assert on the document `Rc` itself:
    // refused means *the same document object*, not an equal one.

    /// A paragraph, a bullet list with one item and a task list with one item —
    /// enough structure for every edit below to have something it would change.
    ///
    /// Built **by the handle**: parsed by its own schema, wrapped by its own
    /// commands, then reloaded so the history starts empty. Node types compare by
    /// identity (#217), so a list assembled from a second `Schema::starter_kit()`
    /// would make the list commands no-ops and their refusals prove nothing.
    fn read_only_fixture() -> Harness {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "")]));
        assert!(
            h.handle
                .load_html("<p>hello world</p><p>item</p><p>todo</p>")
        );
        // "hello world" is 1..12; the second paragraph opens at 13.
        h.handle.set_selection(Selection::cursor(Pos(15)));
        assert!(h.handle.command("toggleBulletList"));
        // list 13, item 14, paragraph 15, "item" 16..20; the third opens at 23.
        h.handle.set_selection(Selection::cursor(Pos(25)));
        assert!(h.handle.command("toggleTaskList"));
        // task list 23, task item 24, paragraph 25, "todo" 26..30.
        h.handle.load_doc(h.handle.doc());
        let doc = h.handle.doc();
        assert_eq!(doc.child(1).type_name(), "bullet_list");
        assert_eq!(doc.child(2).type_name(), "task_list");
        h
    }

    /// A local edit by name: where the selection sits for it, and the call the
    /// platform glue (or a toolbar) makes, answering whether it applied.
    type LocalEdit = (&'static str, Selection, fn(&EditorHandle) -> bool);

    /// Every kind of local mutation there is an entry point for. In
    /// [`read_only_fixture`] "hello world" is 1..12 ("world" 7..12), the list
    /// item's "item" starts at 16 and the task item's "todo" at 26.
    fn local_edits() -> Vec<LocalEdit> {
        /// For the entry points that answer `()`: did the document change?
        fn changed(h: &EditorHandle, edit: impl FnOnce(&EditorHandle)) -> bool {
            let before = h.doc();
            edit(h);
            !h.doc().same_ref(&before)
        }
        let world = || Selection::text(Pos(7), Pos(12));
        let end = || Selection::cursor(Pos(12));
        /// A key the keymap binds: consumed whether or not its command applies.
        fn bound_key(h: &EditorHandle, k: &str) -> bool {
            let applied = h.dispatch_key(KeyBinding::parse(k).unwrap());
            assert!(applied.is_some(), "{k} is bound, so consumed either way");
            applied == Some(true)
        }
        vec![
            ("typing over a selection", world(), |h| h.insert_text("X")),
            ("typing at a caret", end(), |h| h.insert_text("!")),
            ("a markdown input rule", end(), |h| h.insert_text(" **b** ")),
            ("an IME commit", end(), |h| {
                changed(h, |h| h.ime_commit("ね"))
            }),
            ("an IME surrounding-text delete", end(), |h| {
                changed(h, |h| h.ime_delete_surrounding(2, 0))
            }),
            ("cut / delete selection", world(), |h| {
                h.command("deleteSelection")
            }),
            ("backspace", end(), |h| h.command("deleteCharBackward")),
            ("forward delete", Selection::cursor(Pos(3)), |h| {
                h.command("deleteCharForward")
            }),
            ("the Backspace key", end(), |h| bound_key(h, "Backspace")),
            ("enter", end(), |h| h.command("enter")),
            ("the Enter key", end(), |h| bound_key(h, "Enter")),
            ("a hard break", end(), |h| h.command("insertHardBreak")),
            ("a horizontal rule", end(), |h| {
                h.command("insertHorizontalRule")
            }),
            ("a table", end(), |h| h.command("insertTable")),
            ("a plain-text paste", world(), |h| {
                h.replace_selection_with_text("pasted\nlines")
            }),
            ("a rich paste", world(), |h| {
                h.replace_selection_with_html("<p><b>pasted</b></p>")
            }),
            ("an image paste", end(), |h| {
                h.insert_image("data:image/png;base64,AAAA", "")
            }),
            ("bold over a range", world(), |h| h.command("toggleBold")),
            ("Mod-b over a range", world(), |h| bound_key(h, "Mod-b")),
            ("italic over a range", world(), |h| {
                h.command("toggleItalic")
            }),
            ("a link", world(), |h| h.toggle_link("https://example.com")),
            ("a heading", end(), |h| h.command("setHeading1")),
            ("alignment", end(), |h| h.command("setTextAlignCenter")),
            ("a blockquote wrap", end(), |h| {
                h.command("wrapInBlockquote")
            }),
            ("a list wrap", end(), |h| h.command("toggleBulletList")),
            ("outdent", Selection::cursor(Pos(17)), |h| {
                h.command("liftListItem")
            }),
            ("Shift-Tab in a list", Selection::cursor(Pos(17)), |h| {
                bound_key(h, "Shift-Tab")
            }),
            ("a task checkbox click", end(), |h| {
                h.toggle_task_checked_at(26)
            }),
            ("a hand-built transaction", end(), |h| {
                h.update(|state| {
                    let mut tr = state.tr();
                    tr.delete(1, 6).ok()?;
                    Some(tr)
                })
            }),
        ]
    }

    /// Every local edit, refused: it answers "not applied", the document is the
    /// very same object afterwards, the host still shows it, and `on_change` stayed
    /// silent. Each is first shown to **apply** on an editable editor from the same
    /// position — a command that would have done nothing anyway proves nothing.
    #[test]
    fn a_read_only_editor_refuses_every_local_edit() {
        use std::cell::Cell;
        for (what, selection, edit) in local_edits() {
            let control = read_only_fixture();
            control.handle.set_selection(selection.clone());
            let before = control.handle.doc();
            assert!(
                edit(&control.handle),
                "control: {what} applies when editable"
            );
            assert!(
                !control.handle.doc().same_ref(&before),
                "control: {what} changes the document when editable"
            );

            let h = read_only_fixture();
            let hits = Rc::new(Cell::new(0u32));
            h.handle.on_change({
                let hits = hits.clone();
                move || hits.set(hits.get() + 1)
            });
            assert!(!h.handle.is_read_only(), "editable by default");
            h.handle.set_read_only(true);
            assert!(h.handle.is_read_only());
            h.handle.set_selection(selection.clone());
            let before = h.handle.doc();
            let host_before: Vec<_> = children(&h, h.container_id)
                .into_iter()
                .map(|id| text(&h, id))
                .collect();

            assert!(!edit(&h.handle), "{what} must report that it did not apply");
            assert!(
                h.handle.doc().same_ref(&before),
                "{what} must leave the very same document"
            );
            assert_eq!(
                h.handle.selection(),
                selection,
                "{what} must not move the selection either"
            );
            let host_after: Vec<_> = children(&h, h.container_id)
                .into_iter()
                .map(|id| text(&h, id))
                .collect();
            assert_eq!(host_after, host_before, "{what}: the host is untouched");
            assert_eq!(hits.get(), 0, "{what}: on_change stayed silent");
        }
    }

    /// Undo and redo replay document changes, so they are refused like any other —
    /// and the history they would have replayed is still there when the editor is
    /// writable again. Flipping the switch back restores editing in full.
    #[test]
    fn undo_and_redo_are_refused_while_read_only_and_flipping_back_restores_editing() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "abc")]));
        h.handle.set_selection(Selection::cursor(Pos(4)));
        assert!(h.handle.insert_text("d"));
        let para_text = |h: &Harness| text(h, children(h, h.container_id)[0]);
        assert_eq!(para_text(&h).as_deref(), Some("abcd"));

        h.handle.set_read_only(true);
        let locked = h.handle.doc();
        assert!(!h.handle.command("undo"), "undo is refused");
        assert!(h.handle.doc().same_ref(&locked));
        assert!(!h.handle.can_run("undo"), "and reported as unavailable");

        h.handle.set_read_only(false);
        assert!(!h.handle.is_read_only());
        assert!(h.handle.can_run("undo"));
        assert!(h.handle.command("undo"), "the history survived the lock");
        assert_eq!(para_text(&h).as_deref(), Some("abc"));

        h.handle.set_read_only(true);
        let locked = h.handle.doc();
        assert!(!h.handle.command("redo"), "redo is refused");
        assert!(h.handle.doc().same_ref(&locked));

        h.handle.set_read_only(false);
        assert!(h.handle.command("redo"));
        assert_eq!(para_text(&h).as_deref(), Some("abcd"));
        h.handle.set_selection(Selection::cursor(Pos(5)));
        assert!(h.handle.insert_text("e"), "typing works again");
        assert_eq!(para_text(&h).as_deref(), Some("abcde"));
    }

    /// What a reader does: place the caret, move it, extend a selection, select a
    /// word, a block, everything — and copy.
    #[test]
    fn a_read_only_editor_still_selects_moves_the_caret_and_copies() {
        let s = schema();
        let h = mount(doc_node(
            &s,
            vec![para(&s, "hello world"), para(&s, "second")],
        ));
        h.handle.set_read_only(true);

        h.handle.set_selection(Selection::cursor(Pos(3)));
        assert_eq!(h.handle.selection(), Selection::cursor(Pos(3)));
        assert!(h.handle.move_cursor(CursorMotion::CharRight, false));
        assert_eq!(h.handle.selection().head(), Pos(4));
        assert!(h.handle.move_cursor(CursorMotion::WordRight, true));
        assert!(!h.handle.selection().is_empty(), "shift+motion extends");
        assert!(h.handle.move_cursor(CursorMotion::DocEnd, false));

        assert!(h.handle.select_word_at(Pos(8)));
        let (html, plain) = h.handle.selection_clipboard().expect("a word to copy");
        assert_eq!(plain, "world");
        assert!(html.contains("world"));
        assert!(h.handle.select_block_at(Pos(15)));
        assert_eq!(h.handle.selection_clipboard().unwrap().1, "second");

        assert!(h.handle.can_run("selectAll"));
        assert!(
            h.handle.command("selectAll"),
            "select-all changes no document"
        );
        assert_eq!(
            h.handle.selection_clipboard().unwrap().1,
            "hello world\nsecond"
        );
        assert_eq!(
            h.handle.dispatch_key(KeyBinding::parse("Mod-a").unwrap()),
            Some(true)
        );
    }

    /// "Click Bold, then type" is an edit in waiting: refused on a read-only editor
    /// (or the toolbar would light Bold for text nobody can type), dropped when the
    /// switch goes on, and never in the way of a caret move.
    #[test]
    fn stored_marks_are_refused_and_dropped_by_a_read_only_editor() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "abc")]));
        h.handle.set_selection(Selection::cursor(Pos(2)));
        assert!(h.handle.command("toggleBold"), "editable: bold is stored");
        assert!(h.handle.is_mark_active("bold"));

        h.handle.set_read_only(true);
        assert!(
            !h.handle.is_mark_active("bold"),
            "going read-only drops the pending mark"
        );
        assert!(!h.handle.command("toggleBold"), "and none can be stored");
        assert!(!h.handle.is_mark_active("bold"));
        assert!(!h.handle.can_run("toggleBold"));

        // Stored marks left over in the state must not make a caret move look like
        // an edit: clearing them is what every caret move does.
        h.handle.set_read_only(false);
        assert!(h.handle.command("toggleItalic"));
        {
            // Lock without the tidy-up `set_read_only` does, to pin the rule itself.
            h.handle.inner.borrow_mut().read_only = true;
        }
        assert!(h.handle.state().stored_marks.is_some());
        assert!(h.handle.move_cursor(CursorMotion::CharRight, false));
        assert_eq!(h.handle.selection().head(), Pos(3), "the caret moved");
        assert!(h.handle.state().stored_marks.is_none());
    }

    /// `can_run` is what a toolbar greys its buttons from, so it has to follow the
    /// switch — without taking the reader's own commands down with it.
    #[test]
    fn can_run_follows_the_switch() {
        let h = read_only_fixture();
        h.handle.set_selection(Selection::text(Pos(7), Pos(12)));
        let editing = ["toggleBold", "setHeading1", "toggleBulletList", "enter"];
        for name in editing {
            assert!(h.handle.can_run(name), "{name} applies while editable");
        }
        h.handle.set_read_only(true);
        let (doc, selection) = (h.handle.doc(), h.handle.selection());
        for name in editing {
            assert!(!h.handle.can_run(name), "{name} is unavailable read-only");
        }
        assert!(
            h.handle.can_run("selectAll"),
            "a reader can still select all"
        );
        assert!(
            h.handle.doc().same_ref(&doc) && h.handle.selection() == selection,
            "asking ran nothing: the answer comes from a dry run"
        );
        h.handle.set_read_only(false);
        for name in editing {
            assert!(h.handle.can_run(name), "{name} is back");
        }
    }

    /// The app loading a document is not the user editing one: it is how a
    /// read-only editor gets something to show, flag on, no toggling round it.
    #[test]
    fn a_read_only_editor_still_loads_a_document() {
        let s = schema();
        let h = mount(doc_node(&s, vec![para(&s, "old")]));
        h.handle.set_read_only(true);
        assert!(h.handle.load_html("<p>new content</p>"));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("new content")
        );
        h.handle.load_doc(doc_node(&s, vec![para(&s, "and again")]));
        assert_eq!(
            text(&h, children(&h, h.container_id)[0]).as_deref(),
            Some("and again")
        );
        assert!(h.handle.is_read_only(), "a load does not unlock it");
        assert!(!h.handle.insert_text("x"));
    }

    /// The switch lives on the handle, not the view: set before mount it is on the
    /// container from the first build, a re-mount keeps it, and switching it off
    /// takes the attribute away again.
    #[test]
    fn the_container_carries_the_switch_across_mounts() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let mount_into = |h: &EditorHandle| {
            let id = doc.borrow_mut().create_element("div");
            h.attach(
                NodeHandle::new(id, Rc::downgrade(&doc)),
                Rc::downgrade(&doc),
            );
            id
        };
        let readonly_attr = |id: NodeId| doc.borrow().get_attribute(id, "data-pm-readonly");

        let s = Rc::new(schema());
        let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());
        h.set_read_only(true); // before any view exists
        assert!(!h.insert_text("x"), "refused before mount too");

        let first = mount_into(&h);
        assert_eq!(readonly_attr(first).as_deref(), Some("true"));
        let second = mount_into(&h); // a reactive re-mount
        assert_eq!(readonly_attr(second).as_deref(), Some("true"));

        h.set_read_only(false);
        assert_eq!(readonly_attr(second), None, "absent, not \"false\"");
        h.set_read_only(true);
        assert_eq!(readonly_attr(second).as_deref(), Some("true"));
    }

    /// The `Editor { read_only: true }` prop switches the handle on at mount; the
    /// default `false` leaves a handle the app already locked alone.
    #[test]
    fn the_editor_components_read_only_prop_only_ever_locks() {
        use rinch_core::Component;
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let render = |editor: crate::Editor| {
            let root = doc.borrow_mut().create_element("div");
            let mut scope = RenderScope::new(doc.clone(), root);
            let container = editor.render(&mut scope, &[]);
            (scope, container)
        };

        let prop = crate::create_editor();
        let (_scope, container) = render(crate::Editor {
            editor: Some(prop.clone()),
            content: "<p>shown</p>".into(),
            read_only: true,
        });
        assert!(prop.is_read_only());
        assert_eq!(
            container.get_attribute("data-pm-readonly").as_deref(),
            Some("true")
        );
        assert_eq!(
            prop.doc().child(0).child(0).text(),
            Some("shown"),
            "the content prop still loads"
        );
        assert!(!prop.insert_text("x"));

        let locked = crate::create_editor();
        locked.set_read_only(true);
        let (_scope, _container) = render(crate::Editor {
            editor: Some(locked.clone()),
            ..Default::default()
        });
        assert!(
            locked.is_read_only(),
            "`read_only: false` is the default, not an instruction to unlock"
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

        /// A peer's keystroke in the paragraph the caret is in leaves the caret in
        /// that paragraph, where it was in the text, and asks the platform to
        /// repaint the overlays. Both used to go wrong: the caret was carried to
        /// the next paragraph, and on a runtime that refreshes the caret from its
        /// input handlers (web) it stayed painted where the line used to end.
        #[test]
        fn a_remote_edit_in_the_carets_paragraph_keeps_the_caret_and_asks_for_a_repaint() {
            thread_local! {
                static REFRESHES: Cell<u32> = const { Cell::new(0) };
            }
            fn count_refresh() {
                REFRESHES.with(|n| n.set(n.get() + 1));
            }

            let s = schema();
            let host = mount(doc_node(
                &s,
                vec![para(&s, "Once in a while"), para(&s, "next")],
            ))
            .handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);

            // The guest's caret sits at the end of the first line; the host
            // backspaces that line's last letter.
            guest.set_selection(Selection::cursor(Pos(16)));
            crate::registry::set_overlay_refresher(count_refresh);
            REFRESHES.with(|n| n.set(0));
            host.set_selection(Selection::cursor(Pos(16)));
            host.ime_delete_surrounding(1, 0);
            assert_eq!(doc_text(&guest), "Once in a whil\nnext");

            assert_eq!(
                guest.state().selection.head(),
                Pos(15),
                "the caret stays at the end of its own line"
            );
            assert!(
                REFRESHES.with(|n| n.get()) >= 1,
                "the platform was asked to repaint the overlays"
            );
        }

        // ── Read-only and collaboration ──────────────────────────────────────
        //
        // The pair of properties a read-only collaborator is for: what peers write
        // still arrives, and nothing it does ever leaves.

        /// A host and a **read-only** guest on one loopback. The guest was locked
        /// before it joined, so the join itself is part of what is proven. Returns
        /// the guest's harness (for its mock host) and a counter of every delta the
        /// guest tried to send.
        fn read_only_guest(host: &EditorHandle) -> (Harness, Rc<Cell<usize>>) {
            let s = schema();
            let guest = mount(doc_node(&s, vec![para(&s, "stale local")]));
            guest.handle.set_read_only(true);
            let guest_in = guest.handle.clone();
            let snapshot = host
                .start_collaboration_host(move |delta| {
                    guest_in.collab_receive(&delta);
                })
                .expect("host projects its document");
            let sent = Rc::new(Cell::new(0usize));
            let (host_in, counter) = (host.clone(), sent.clone());
            guest
                .handle
                .start_collaboration_guest(&snapshot, move |delta| {
                    counter.set(counter.get() + 1);
                    host_in.collab_receive(&delta);
                })
                .expect("a read-only guest joins");
            (guest, sent)
        }

        /// The first block of a harness's **host** — what is on screen.
        fn shown(h: &Harness) -> Option<String> {
            text(h, children(h, h.container_id)[0])
        }

        /// Remote deltas are applied and shown exactly as in an editable editor:
        /// integration never passes through the gate local edits pass through.
        #[test]
        fn a_read_only_guest_joins_and_keeps_receiving_remote_edits() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "shared")])).handle;
            let (guest, sent) = read_only_guest(&host);
            assert_eq!(doc_text(&guest.handle), "shared", "adopted on join");
            assert_eq!(shown(&guest).as_deref(), Some("shared"), "and shown");
            assert!(guest.handle.is_read_only() && guest.handle.is_collaborating());

            // Typing, a mark, a new block and a deletion, all from the peer.
            host.set_selection(Selection::cursor(Pos(7)));
            assert!(host.insert_text(" text"));
            assert_eq!(doc_text(&guest.handle), "shared text");
            assert_eq!(
                shown(&guest).as_deref(),
                Some("shared text"),
                "the view follows"
            );

            host.set_selection(Selection::text(Pos(1), Pos(7)));
            assert!(host.command("toggleBold"));
            assert_eq!(
                guest.handle.doc().child(0).child(0).marks().len(),
                1,
                "a remote mark arrives"
            );

            host.set_selection(Selection::cursor(Pos(12)));
            assert!(host.command("enter"));
            assert!(host.insert_text("second"));
            assert_eq!(doc_text(&guest.handle), "shared text\nsecond");
            assert_eq!(children(&guest, guest.container_id).len(), 2);

            host.set_selection(Selection::text(Pos(1), Pos(8)));
            assert!(host.command("deleteSelection"));
            assert_eq!(doc_text(&guest.handle), "text\nsecond");
            assert_eq!(shown(&guest).as_deref(), Some("text"));

            // A reconciliation diff is a remote update like any other.
            let sv = guest.handle.collab_state_vector().unwrap();
            let diff = host.collab_sync_diff(&sv).unwrap();
            guest.handle.collab_receive(&diff);
            assert_eq!(doc_text(&guest.handle), doc_text(&host));
            assert_eq!(
                sent.get(),
                0,
                "and through all of it the guest sent nothing"
            );
        }

        /// Local insert, delete, paste, format and undo on a read-only collaborator:
        /// refused, and the CRDT is byte-for-byte what it was — same snapshot, same
        /// state vector, nothing handed to `outbound`, nothing at the peer.
        #[test]
        fn a_read_only_guest_changes_nothing_shared() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello world")])).handle;
            let (guest, sent) = read_only_guest(&host);
            let guest = guest.handle;
            // Something to undo, had the guest been allowed: the host's edit is in
            // the guest's document, and a local history entry is made below.
            host.set_selection(Selection::cursor(Pos(12)));
            assert!(host.insert_text("!"));
            assert_eq!(doc_text(&guest), "hello world!");

            let (snapshot, sv) = (guest.collab_snapshot(), guest.collab_state_vector());
            let (doc, host_doc) = (guest.doc(), host.doc());

            guest.set_selection(Selection::text(Pos(7), Pos(12)));
            assert!(!guest.insert_text("X"), "insert");
            assert!(!guest.command("deleteSelection"), "delete");
            assert!(!guest.command("deleteCharBackward"), "backspace");
            assert!(!guest.replace_selection_with_text("pasted"), "paste");
            assert!(!guest.replace_selection_with_html("<p><i>rich</i></p>"));
            assert!(!guest.command("toggleBold"), "format");
            assert!(!guest.command("setHeading2"), "block format");
            assert!(!guest.command("undo"), "undo");
            assert!(!guest.command("redo"), "redo");
            guest.ime_commit("ね");

            assert!(
                guest.doc().same_ref(&doc),
                "the guest's document is untouched"
            );
            assert_eq!(
                guest.collab_snapshot(),
                snapshot,
                "the CRDT bytes are unchanged"
            );
            assert_eq!(
                guest.collab_state_vector(),
                sv,
                "and so is the state vector"
            );
            assert_eq!(sent.get(), 0, "outbound never fired");
            assert!(host.doc().same_ref(&host_doc), "the peer saw nothing");
            assert!(guest.collab_take_error().is_none());
        }

        /// The switch is a runtime one (a role changes): off, the same session
        /// edits and broadcasts again; on again, it stops — with the peer's edits
        /// arriving throughout.
        #[test]
        fn flipping_read_only_mid_session_stops_and_restores_editing() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
            let (guest, sent) = read_only_guest(&host);
            let guest = guest.handle;

            guest.set_selection(Selection::cursor(Pos(3)));
            assert!(!guest.insert_text("c"));

            guest.set_read_only(false);
            assert!(guest.insert_text("c"), "editable again");
            assert_eq!(doc_text(&host), "abc", "and the edit reached the peer");
            assert_eq!(sent.get(), 1);
            // Its own history works: undo of its own edit is an edit, and travels.
            assert!(guest.command("undo"));
            assert_eq!(doc_text(&host), "ab");
            assert!(guest.command("redo"));
            assert_eq!(doc_text(&host), "abc");
            let sent_while_editable = sent.get();

            guest.set_read_only(true);
            assert!(!guest.insert_text("d"));
            assert!(
                !guest.command("undo"),
                "its own history is locked again too"
            );
            host.set_selection(Selection::cursor(Pos(1)));
            assert!(host.insert_text(">"));
            assert_eq!(doc_text(&guest), ">abc", "inbound never stopped");
            assert_eq!(doc_text(&host), ">abc");
            assert_eq!(
                sent.get(),
                sent_while_editable,
                "locked: nothing more went out"
            );
        }

        /// With a session attached a load is a write to the shared document — it
        /// would be recorded and broadcast — so a read-only collaborator refuses it.
        /// Detached, the same load is the app showing a document, and works.
        #[test]
        fn a_read_only_collaborator_refuses_a_load_until_it_detaches() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "shared")])).handle;
            let (guest, sent) = read_only_guest(&host);
            let snapshot = guest.handle.collab_snapshot();

            assert!(
                !guest.handle.load_html("<p>replaced</p>"),
                "reported as refused"
            );
            guest
                .handle
                .load_doc(doc_node(&s, vec![para(&s, "replaced")]));
            assert_eq!(doc_text(&guest.handle), "shared");
            assert_eq!(doc_text(&host), "shared", "the peer's document survives");
            assert_eq!(guest.handle.collab_snapshot(), snapshot);
            assert_eq!(sent.get(), 0);

            guest.handle.stop_collaboration();
            assert!(guest.handle.load_html("<p>next document</p>"));
            assert_eq!(shown(&guest).as_deref(), Some("next document"));
            assert_eq!(doc_text(&host), "shared");
        }

        /// `Editor { content, read_only: true }` mounted onto a handle that is
        /// **already collaborating**: the component locks before it loads, so its
        /// `content` is refused like any other write to a shared document rather
        /// than being sent to the peers of a document this user was just told they
        /// may not change. The order is observable nowhere else — with no session a
        /// load is never refused, so an ordinary mount still fills the editor
        /// (`the_editor_components_read_only_prop_only_ever_locks`).
        #[test]
        fn the_content_prop_does_not_write_a_collaborating_read_only_editors_shared_doc() {
            use rinch_core::Component;
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "shared")])).handle;
            let snapshot = host.start_collaboration_host(|_| {}).unwrap();

            let pane = crate::create_editor();
            let sent = Rc::new(Cell::new(0usize));
            let counter = sent.clone();
            pane.start_collaboration_guest(&snapshot, move |_| counter.set(counter.get() + 1))
                .unwrap();

            let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
            let root = doc.borrow_mut().create_element("div");
            let mut scope = RenderScope::new(doc.clone(), root);
            let container = crate::Editor {
                editor: Some(pane.clone()),
                content: "<p>mine</p>".into(),
                read_only: true,
            }
            .render(&mut scope, &[]);

            assert_eq!(
                doc_text(&pane),
                "shared",
                "the shared document is what shows"
            );
            assert_eq!(doc_text(&host), "shared", "and the peer's is untouched");
            assert_eq!(sent.get(), 0, "nothing was broadcast");
            assert_eq!(
                container.get_attribute("data-pm-readonly").as_deref(),
                Some("true")
            );
        }

        /// One editor pane moving from document to document (Pimble's shape), and
        /// the app forgot `stop_collaboration` in between. The join adopts the new
        /// document — read-only or not — and the *old* session's peers are not sent
        /// the new document as an edit to theirs.
        #[test]
        fn joining_with_a_stale_session_attached_adopts_the_new_document_and_spares_the_old() {
            let s = schema();
            for read_only in [true, false] {
                let first = mount(doc_node(&s, vec![para(&s, "first document")])).handle;
                let second = mount(doc_node(&s, vec![para(&s, "second document")])).handle;
                let pane = mount(doc_node(&s, vec![para(&s, "")]));
                pane.handle.set_read_only(read_only);

                let to_first = Rc::new(Cell::new(0usize));
                let snapshot = first.start_collaboration_host(|_| {}).unwrap();
                let (first_in, counter) = (first.clone(), to_first.clone());
                pane.handle
                    .start_collaboration_guest(&snapshot, move |delta| {
                        counter.set(counter.get() + 1);
                        first_in.collab_receive(&delta);
                    })
                    .unwrap();
                assert_eq!(doc_text(&pane.handle), "first document");

                // No `stop_collaboration` here.
                let snapshot = second.start_collaboration_host(|_| {}).unwrap();
                pane.handle
                    .start_collaboration_guest(&snapshot, |_| {})
                    .unwrap();
                assert_eq!(
                    doc_text(&pane.handle),
                    "second document",
                    "read_only={read_only}: the join adopted the new document"
                );
                assert_eq!(shown(&pane).as_deref(), Some("second document"));
                assert_eq!(
                    (to_first.get(), doc_text(&first).as_str()),
                    (0, "first document"),
                    "read_only={read_only}: the first document's peers were sent nothing"
                );
            }
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

            // CRDT convergence: identical documents on both peers.
            let h = doc_text(&host);
            let g = doc_text(&guest);
            assert_eq!(h, g, "concurrent edits converge to one document");
            assert!(
                h.contains('H') && h.contains('G'),
                "both edits survived: {h}"
            );
        }

        /// Reconcile two handles by exchanging state vectors and the diffs they imply —
        /// the shape a networked caller uses when it cannot trust delta delivery.
        ///
        /// The exchange is unconditional in both directions: equal state vectors would
        /// not prove convergence (they summarise insertions only), so settling is
        /// detected by both sides integrating without a document change.
        fn sync_until_quiet(a: &EditorHandle, b: &EditorHandle) {
            for _ in 0..20 {
                let a_sv = a.collab_state_vector().expect("a is collaborating");
                let b_sv = b.collab_state_vector().expect("b is collaborating");
                let to_b = a.collab_sync_diff(&b_sv).expect("diff for b");
                let to_a = b.collab_sync_diff(&a_sv).expect("diff for a");
                let b_changed = b.collab_receive(&to_b);
                let a_changed = a.collab_receive(&to_a);
                if !a_changed && !b_changed {
                    return;
                }
            }
            panic!("the state-vector exchange did not settle");
        }

        #[test]
        fn the_sync_protocol_converges_two_peers_whose_deltas_never_arrived() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

            // Deltas go nowhere: this models the transport reconciliation exists for —
            // an HTTP poll, a dropped socket, a peer that was offline. The broadcast
            // path alone would leave these two permanently diverged.
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
            assert_eq!(h, doc_text(&guest), "reconciliation converged the peers");
            assert!(
                h.contains("world") && h.contains('G'),
                "both offline edits survived: {h}"
            );
            assert_eq!(
                host.collab_state_vector(),
                guest.collab_state_vector(),
                "converged peers have seen the same changes"
            );
        }

        /// Issue #220, end to end through the handle. A horizontal rule is outside the
        /// staged A22 scope, so inserting one while collaborating cannot be projected.
        /// The editor still applies it — the model is the source of truth — so from that
        /// point the local document holds a block the CRDT does not, and outbound
        /// stalls.
        ///
        /// What used to happen: every later edit failed a block-count check whose
        /// message named neither the cause nor the cure ("the model document holds 2
        /// block(s) but the CRDT holds 1"), *including the deletion that was supposed
        /// to be the cure*; and when the counts finally realigned, the diff skipped
        /// every block it believed unchanged, so text typed during the stall stayed
        /// local forever while `record_local` answered `Ok`.
        ///
        /// What must happen now: the stall is visible and self-describing, nothing
        /// reaches the guest while it holds, and deleting the rule ships the whole
        /// backlog in one delta.
        #[test]
        fn an_out_of_scope_edit_stalls_outbound_and_removing_it_ships_the_backlog() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);
            assert_eq!(doc_text(&guest), "hello");
            assert!(host.collab_outbound_stall().is_none(), "healthy to start");

            // Append a blockquote — applied locally, refused by the projection. (A
            // `horizontal_rule` used to stand here; leaf block atoms are inside the
            // projected scope now, so the stall needs content that is still outside it.)
            assert!(
                host.update(|state| {
                    let s = state.schema().clone();
                    let inner = s
                        .branch("paragraph", Fragment::from_node(s.text("q").ok()?))
                        .ok()?;
                    let bq = s.branch("blockquote", Fragment::from_node(inner)).ok()?;
                    let at = state.doc.content_size();
                    let mut tr = state.tr();
                    tr.replace(at, at, Slice::new(Fragment::from_node(bq), 0, 0))
                        .ok()?;
                    Some(tr)
                }),
                "the editor applies it"
            );
            let stall = host
                .collab_outbound_stall()
                .expect("outbound must report itself stalled");
            assert!(
                stall.to_string().contains("blockquote"),
                "the stall must name the content to remove, got: {stall}"
            );
            assert!(
                !host.is_collaboration_poisoned(),
                "an out-of-scope LOCAL edit is not poison — the shared doc is healthy"
            );

            // Typing while stalled stays local — this is the text that used to be lost.
            host.set_selection(Selection::cursor(Pos(6)));
            assert!(host.insert_text("!!"));
            assert!(
                host.collab_outbound_stall().is_some(),
                "still stalled while the blockquote is there"
            );
            assert_eq!(
                doc_text(&guest),
                "hello",
                "nothing reached the guest during the stall"
            );

            // Delete the blockquote — selecting it and pressing Delete, as an app would. Note
            // this is NOT `undo`: the text typed during the stall stays, which is the
            // half that must survive.
            assert!(
                host.update(|state| {
                    let doc = &state.doc;
                    let last = doc.child_count() - 1;
                    let from: usize = (0..last).map(|i| doc.child(i).node_size()).sum();
                    let to = from + doc.child(last).node_size();
                    let mut tr = state.tr();
                    tr.delete(from, to).ok()?;
                    Some(tr)
                }),
                "the blockquote is deleted"
            );
            assert!(
                host.collab_outbound_stall().is_none(),
                "removing the offending content resumes outbound: {:?}",
                host.collab_outbound_stall().map(|e| e.to_string())
            );
            assert_eq!(
                doc_text(&host),
                "hello!!",
                "the local document kept what was typed during the stall"
            );
            assert_eq!(
                doc_text(&guest),
                doc_text(&host),
                "and the guest caught up on all of it in one delta"
            );
        }

        #[test]
        fn sync_methods_are_inert_without_a_session() {
            let s = schema();
            let solo = mount(doc_node(&s, vec![para(&s, "local only")])).handle;
            assert!(solo.collab_state_vector().is_none());
            assert!(solo.collab_sync_diff(&[0]).is_none());
            assert_eq!(doc_text(&solo), "local only", "the document is untouched");
        }

        #[test]
        fn integrating_a_reconciliation_diff_does_not_echo() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

            let snapshot = host.start_collaboration_host(|_d| {}).unwrap();
            // A change that arrives by reconciliation must not fire the broadcast sink
            // either — a caller running both paths would otherwise loop.
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
                "integrating a reconciliation diff must not broadcast a delta back"
            );
        }

        #[test]
        fn reconciliation_carries_a_deletion_that_the_state_vector_cannot_express() {
            // A state vector counts insertions, so a peer that only *deleted* leaves it
            // unchanged. If any part of the path short-circuits on state-vector equality
            // the deletion is silently lost — which is exactly the class of bug the fuzz
            // suites caught during the engine swap, so it is pinned here at the handle
            // level too.
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "abcdef")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

            let snapshot = host.start_collaboration_host(|_d| {}).unwrap();
            guest.start_collaboration_guest(&snapshot, |_d| {}).unwrap();
            assert_eq!(doc_text(&guest), "abcdef", "guest joined on the host's doc");

            let sv_before = host.collab_state_vector().unwrap();
            host.update(|st| {
                let mut tr = st.tr();
                tr.delete(3, 5).ok()?; // drop "cd"
                Some(tr)
            });
            assert_eq!(doc_text(&host), "abef", "the host really deleted");
            assert_eq!(
                host.collab_state_vector().unwrap(),
                sv_before,
                "precondition: a delete-only change leaves the state vector untouched"
            );

            sync_until_quiet(&host, &guest);
            assert_eq!(doc_text(&guest), "abef", "the deletion reached the guest");
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

        // ── The poisoned session (issue #196) ────────────────────────────────
        //
        // Foreign bytes whose `content` root was created as the wrong yrs type leave
        // the shared CRDT unprojectable once integrated, with nothing pending that
        // could cure it (yrs has no rollback). The session must then go loud in BOTH
        // directions — sticky `SessionPoisoned` on inbound and outbound — instead of
        // one-way partitioning: keeping `record_local` Ok and broadcasting while
        // every receive fails was the dangerous silent half. (Inbound stays
        // *attempted*: an update that makes the document rebuildable again clears
        // the poison — pinned at the session level in tests/poison.rs.)

        /// Whole-document bytes of a foreign yrs doc whose `content` root is a
        /// **Text** type — the issue's headline poison shape. Decodes and applies
        /// fine; the read-back is what can never succeed.
        fn foreign_text_root_bytes() -> Vec<u8> {
            use yrs::{ReadTxn, Text, Transact};
            let doc = yrs::Doc::new();
            let t = doc.get_or_insert_text("content");
            t.insert(&mut doc.transact_mut(), 0, "foreign");
            doc.transact()
                .encode_state_as_update_v1(&yrs::StateVector::default())
        }

        #[test]
        fn a_poisoning_delta_turns_the_session_loud_in_both_directions() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);

            // Foreign bytes reach the HOST mid-session.
            assert!(!host.collab_receive(&foreign_text_root_bytes()));
            assert!(
                host.is_collaboration_poisoned(),
                "the host session is poisoned"
            );
            assert!(
                !guest.is_collaboration_poisoned(),
                "the guest never saw the bytes and stays healthy"
            );
            assert!(
                matches!(
                    host.collab_take_error(),
                    Some(CollabError::SessionPoisoned(_))
                ),
                "the parked error is the sticky kind, distinguishable from a transient one"
            );

            // The host keeps editing locally, but nothing may leave the poisoned
            // replica — on unfixed code this edit was projected AND broadcast.
            host.set_selection(Selection::cursor(Pos(6)));
            assert!(host.insert_text("X"));
            assert_eq!(doc_text(&host), "helloX", "the local model still edits");
            assert_eq!(
                doc_text(&guest),
                "hello",
                "no delta left the poisoned replica"
            );
            assert!(
                matches!(
                    host.collab_take_error(),
                    Some(CollabError::SessionPoisoned(_))
                ),
                "each refused edit re-reports the sticky error"
            );

            // Inbound stays refused for perfectly valid peer deltas too.
            guest.set_selection(Selection::cursor(Pos(1)));
            assert!(guest.insert_text("G"));
            assert_eq!(doc_text(&guest), "Ghello");
            assert_eq!(
                doc_text(&host),
                "helloX",
                "the poisoned host can no longer receive"
            );

            // Recovery: stop, rejoin from the healthy peer's snapshot.
            host.stop_collaboration();
            assert!(
                !host.is_collaboration_poisoned(),
                "no session, no poison to report"
            );
            let snapshot = guest.collab_snapshot().expect("guest is collaborating");
            let g_in = guest.clone();
            host.start_collaboration_guest(&snapshot, move |d| {
                g_in.collab_receive(&d);
            })
            .expect("rejoining from a healthy snapshot works");
            assert_eq!(
                doc_text(&host),
                "Ghello",
                "the rejoined host adopts the healthy shared document"
            );
            host.set_selection(Selection::cursor(Pos(1)));
            assert!(host.insert_text("R"));
            assert_eq!(
                doc_text(&guest),
                "RGhello",
                "collaboration flows again after the rejoin"
            );
        }

        #[test]
        fn an_undecodable_blob_is_transient_not_poison() {
            // A garbage blob never touches the CRDT, so it must NOT trip the sticky
            // state — the session keeps collaborating in both directions.
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);

            assert!(!host.collab_receive(&[0xff, 0xff, 0xff, 0xff]));
            assert!(
                matches!(host.collab_take_error(), Some(CollabError::Engine(_))),
                "a decode failure surfaces as a transient engine error"
            );
            assert!(!host.is_collaboration_poisoned());

            guest.set_selection(Selection::cursor(Pos(1)));
            assert!(guest.insert_text("G"));
            assert_eq!(doc_text(&host), "Ghello", "inbound still works");
            host.set_selection(Selection::cursor(Pos(7)));
            assert!(host.insert_text("X"));
            assert_eq!(doc_text(&guest), "GhelloX", "outbound still works");
        }

        // ── The zero-block state (issue #192) ────────────────────────────────
        //
        // Two peers deleting *different* blocks concurrently deletes every block, so
        // the shared CRDT converges to an empty content list. No model can mirror that
        // (the schema requires a block), so both peers show the starter paragraph while
        // the CRDT holds nothing. That state used to wedge the session permanently: the
        // next local edit tried to reconcile a block the CRDT did not have, failed with
        // `Schema("reconcile_node: missing node")`, broadcast nothing, and never
        // recovered.

        /// Delete a whole block from `h` by its index, tokens included.
        fn delete_block(h: &EditorHandle, index: usize) {
            let doc = h.doc();
            let start: usize = (0..index).map(|i| doc.child(i).node_size()).sum();
            let end = start + doc.child(index).node_size();
            h.update(|st| {
                let mut tr = st.tr();
                tr.delete(start, end).ok()?;
                Some(tr)
            });
        }

        /// Wire `host` and `guest` through a **buffered** loopback: deltas pile up
        /// instead of being delivered, so edits made between exchanges are genuinely
        /// concurrent. The returned closure drains both directions until quiet.
        fn buffered_loopback(host: &EditorHandle, guest: &EditorHandle) -> impl Fn() {
            let to_guest: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let tg = to_guest.clone();
            let snapshot = host
                .start_collaboration_host(move |d| tg.borrow_mut().push(d))
                .expect("host projects its document");
            let th = to_host.clone();
            guest
                .start_collaboration_guest(&snapshot, move |d| th.borrow_mut().push(d))
                .expect("guest joins from the snapshot");
            let h = host.clone();
            let g = guest.clone();
            move || {
                // An integrated delta is never re-broadcast, so this settles in one
                // pass; the loop keeps that from being an assumption.
                for _ in 0..4 {
                    let out_g: Vec<Vec<u8>> = to_guest.borrow_mut().drain(..).collect();
                    let out_h: Vec<Vec<u8>> = to_host.borrow_mut().drain(..).collect();
                    if out_g.is_empty() && out_h.is_empty() {
                        return;
                    }
                    for d in out_g {
                        g.collab_receive(&d);
                    }
                    for d in out_h {
                        h.collab_receive(&d);
                    }
                }
                panic!("the buffered loopback did not settle");
            }
        }

        /// Drive two peers to the converged zero-block state: each deletes a different
        /// block of a two-block document, then the deltas are exchanged.
        fn empty_the_shared_document(
            host: &EditorHandle,
            guest: &EditorHandle,
            exchange: &impl Fn(),
        ) {
            assert_eq!(doc_text(host), "one\ntwo");
            assert_eq!(
                doc_text(guest),
                "one\ntwo",
                "the guest joined on the host's doc"
            );
            delete_block(host, 1); // host drops "two"
            delete_block(guest, 0); // guest drops "one"
            assert_eq!(doc_text(host), "one");
            assert_eq!(doc_text(guest), "two");
            exchange();
            assert_eq!(doc_text(host), "", "every block was deleted");
            assert_eq!(doc_text(guest), "");
            assert_eq!(
                host.doc().child_count(),
                1,
                "the starter paragraph stands in"
            );
            assert_eq!(guest.doc().child_count(), 1);
        }

        #[test]
        fn concurrent_block_deletions_do_not_wedge_the_session() {
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "one"), para(&s, "two")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            let exchange = buffered_loopback(&host, &guest);
            empty_the_shared_document(&host, &guest, &exchange);
            assert!(
                host.collab_take_error().is_none() && guest.collab_take_error().is_none(),
                "converging to an empty document is not an error"
            );

            // The host types: the edit must project, broadcast, and reach the guest.
            host.set_selection(Selection::cursor(Pos(1)));
            assert!(host.insert_text("Z"));
            assert_eq!(doc_text(&host), "Z");
            assert!(
                host.collab_take_error().is_none(),
                "the first edit after the empty state must project cleanly"
            );
            exchange();
            assert_eq!(
                doc_text(&guest),
                "Z",
                "the recovered edit reached the guest"
            );

            // And so must the guest's, in the other direction.
            guest.set_selection(Selection::cursor(Pos(1)));
            assert!(guest.insert_text("Y"));
            assert_eq!(doc_text(&guest), "YZ");
            assert!(guest.collab_take_error().is_none());
            exchange();
            assert_eq!(doc_text(&host), "YZ", "the guest's edit reached the host");
            assert!(
                host.collab_take_error().is_none() && guest.collab_take_error().is_none(),
                "no collaboration error surfaced anywhere in the recovery"
            );
        }

        #[test]
        fn concurrent_first_edits_after_the_empty_state_converge() {
            // Both peers type before seeing the other, so both project their starter
            // paragraph into the CRDT. Two concurrent inserts into an empty array is
            // ordinary CRDT behaviour: two blocks, identical on both peers.
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "one"), para(&s, "two")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            let exchange = buffered_loopback(&host, &guest);
            empty_the_shared_document(&host, &guest, &exchange);

            host.set_selection(Selection::cursor(Pos(1)));
            assert!(host.insert_text("H"));
            guest.set_selection(Selection::cursor(Pos(1)));
            assert!(guest.insert_text("G"));
            exchange();

            use rinch_editor_core::serialize::node_to_html;
            assert_eq!(
                node_to_html(&host.doc()),
                node_to_html(&guest.doc()),
                "concurrent first edits out of the empty state converge"
            );
            assert_eq!(host.doc().child_count(), 2, "one block per peer");
            let t = doc_text(&host);
            assert!(
                t.contains('H') && t.contains('G'),
                "both edits survived: {t}"
            );
            assert!(host.collab_take_error().is_none() && guest.collab_take_error().is_none());
        }

        #[test]
        fn a_late_joiner_can_join_a_session_with_no_blocks_left() {
            // The second symptom of #192: `load` used to reject a zero-block projection
            // as "not a rinch editor projection", locking a late joiner out of exactly
            // the state above.
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "one"), para(&s, "two")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

            // Manual buffers rather than the helper: the host's deltas have to fan out to
            // two peers once the late joiner arrives.
            let to_peers: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let tp = to_peers.clone();
            let snapshot = host
                .start_collaboration_host(move |d| tp.borrow_mut().push(d))
                .unwrap();
            let th = to_host.clone();
            guest
                .start_collaboration_guest(&snapshot, move |d| th.borrow_mut().push(d))
                .unwrap();

            delete_block(&host, 1);
            delete_block(&guest, 0);
            for d in to_peers.borrow_mut().drain(..) {
                guest.collab_receive(&d);
            }
            for d in to_host.borrow_mut().drain(..) {
                host.collab_receive(&d);
            }
            assert_eq!(
                doc_text(&host),
                "",
                "precondition: the CRDT holds no blocks"
            );
            assert_eq!(doc_text(&guest), "");

            // A third peer joins from the host's CURRENT (zero-block) snapshot.
            let late = mount(doc_node(&s, vec![para(&s, "stale")])).handle;
            let late_out: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let lo = late_out.clone();
            let snapshot = host.collab_snapshot().expect("host is collaborating");
            late.start_collaboration_guest(&snapshot, move |d| lo.borrow_mut().push(d))
                .expect("a zero-block snapshot is joinable");
            assert_eq!(
                doc_text(&late),
                "",
                "the late joiner adopts the starter paragraph"
            );
            assert_eq!(late.doc().child_count(), 1);

            // The joiner types: the host converges.
            late.set_selection(Selection::cursor(Pos(1)));
            assert!(late.insert_text("L"));
            for d in late_out.borrow_mut().drain(..) {
                host.collab_receive(&d);
                guest.collab_receive(&d);
            }
            assert_eq!(doc_text(&host), "L", "the joiner's edit reached the host");
            assert!(late.collab_take_error().is_none());

            // The host types: the joiner converges.
            host.set_selection(Selection::cursor(Pos(2)));
            assert!(host.insert_text("H"));
            for d in to_peers.borrow_mut().drain(..) {
                late.collab_receive(&d);
                guest.collab_receive(&d);
            }
            assert_eq!(doc_text(&late), "LH", "the host's edit reached the joiner");
            assert!(
                host.collab_take_error().is_none() && late.collab_take_error().is_none(),
                "no collaboration error surfaced in either direction"
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
            // now, and so are inline atoms; blockquote / tables / task lists are not).
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
        fn inline_atom_edits_sync_to_the_peer() {
            // The handle-level pin on PlotWeb's symptom: a hard break or an image used
            // to stop the body syncing the moment it appeared, while the editor still
            // reported a clean save. Both are inside the projected scope now, so
            // neither may fail loud, and the guest must actually receive them.
            let s = schema();
            let host = mount(doc_node(&s, vec![para(&s, "one two")])).handle;
            let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
            loopback(&host, &guest);
            assert_eq!(doc_text(&guest), "one two");

            // Shift+Enter between the words.
            host.set_selection(Selection::cursor(Pos(5)));
            assert!(host.command("insertHardBreak"));
            assert!(
                host.collab_take_error().is_none(),
                "a hard break is supported and must not fail loud"
            );

            // An image paste, which is an inline atom carrying attrs.
            assert!(host.insert_image("data:image/png;base64,AAAA", "shot"));
            assert!(
                host.collab_take_error().is_none(),
                "an image is supported and must not fail loud"
            );

            // The guest holds the same document, atoms and all. Compared by *shape*,
            // not by `==`: the two handles are mounted with their own `Schema`
            // instances, and node/mark type equality is `Rc::ptr_eq` (issue #217).
            let line = guest.doc().child(0).clone();
            let kinds: Vec<String> = (0..line.child_count())
                .map(|i| line.child(i).type_name().to_string())
                .collect();
            assert_eq!(
                kinds,
                vec!["text", "hard_break", "image", "text"],
                "the peer received the inline atoms, not just the text around them"
            );
            assert_eq!(
                line.child(2).attrs().get_str("src"),
                Some("data:image/png;base64,AAAA"),
                "with the image's attrs intact"
            );
            assert!(
                guest.collab_take_error().is_none(),
                "and integrated them without an error"
            );

            // Ordinary typing keeps flowing afterwards — the "it went quiet" symptom.
            host.set_selection(Selection::cursor(Pos(1)));
            assert!(host.insert_text("Z"));
            assert!(
                doc_text(&guest).starts_with('Z'),
                "typing still reaches the peer"
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
