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
use rinch_editor_core::model::Slice;
use rinch_editor_core::serialize::{
    slice_from_html, slice_from_text, slice_to_html, slice_to_text,
};
use rinch_editor_core::{
    EditorState, EditorView, Node, Plugin, Pos, Schema, Selection, Transaction,
};

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
        core.state = next.clone();
        if let Some(view) = core.view.as_mut() {
            view.update_dom(&prev, &next);
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
        core.state = next.clone();
        if let Some(view) = core.view.as_mut() {
            view.update_dom(&prev, &next);
        }
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

    /// Replace the document with `doc`, resetting selection and history (a fresh
    /// load, not an undoable edit). The host diffs from the old content to the new,
    /// so unchanged leading blocks are reused. Works before focus.
    pub fn load_doc(&self, doc: Node) {
        let mut core = self.inner.borrow_mut();
        let prev = core.state.clone();
        let next = EditorState::create(core.schema.clone(), doc, core.plugins.clone());
        core.state = next.clone();
        if let Some(view) = core.view.as_mut() {
            view.update_dom(&prev, &next);
        }
    }

    /// Parse `html` (schema-whitelisted) and load it as the document. Returns
    /// `false` if it does not parse into valid top-level document content.
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
    pub(crate) fn hide_overlays(&self) -> bool {
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
}
