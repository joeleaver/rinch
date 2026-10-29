//! **Node views**: an app's own live component drawn inside an editor document —
//! a sheet, a canvas, a counter — in place of a leaf node of the document (the
//! starter kit's `embed`).
//!
//! An app registers a [`NodeViewSpec`] on an [`EditorHandle`]
//! ([`EditorHandle::register_node_view`]). For every node of its
//! [`node_type`](NodeViewSpec::node_type) whose attributes the spec
//! [`matches`](NodeViewSpec::matches) (the first registered spec that matches
//! wins), the view renders the spec's component into the node's host element,
//! with a [`NodeViewCtx`] that follows the node's attributes, whether it is
//! selected, and the editor's read-only and dark-mode switches.
//!
//! # Lifetime and identity
//!
//! A view is mounted when its node's host is built and disposed when the node
//! leaves the document: its [`RenderScope`] belongs to the node's descriptor, and
//! every signal, effect and handler the component made goes with it. Typing
//! elsewhere never touches it. A view is **never handed a different node**: its
//! node keeps one identity for the whole mount, and only that node's attributes
//! change (a `SetNodeAttrStep` — a [`NodeViewCtx::set_attrs`], its undo and redo,
//! or a collaborator's change, which arrives the same way). Anything else that
//! puts a node where the view's node stood — a replace, a move, an undo of a
//! delete, a node that moved to another parent — is a different node, and the
//! view is remounted for it: app state keyed to one node never moves to another.
//!
//! # When the app's code runs
//!
//! `matches`, `render`, `on_caret_enter` and `on_key` run with no editor borrow
//! held, after the transaction that made the work has been stored and projected
//! (never inside the diff), and untracked: they may read the editor, dispatch to
//! it and write signals. `render` runs with the view's own scope as the owner.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::{Memo, Owner, Signal, current_owner, unowned, untracked_handler};
use rinch_editor_core::{Attrs, Node, Pos, Selection, SetNodeAttrStep};

use crate::handle::{EditorHandle, WeakHandle};
use crate::keys::EditorKey;

/// The attribute the host element of a mounted node view carries (`""`), so a
/// platform can tell a press or a key inside an app's component from one on the
/// editor's own content.
pub const NODE_VIEW_ATTR: &str = "data-pm-node-view";

/// The side a keyboard caret arrived at a node view from — see
/// [`NodeViewSpec::on_caret_enter`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaretFrom {
    /// From above: ArrowDown onto the node.
    Above,
    /// From below: ArrowUp onto the node.
    Below,
    /// From the left: ArrowRight onto the node.
    Left,
    /// From the right: ArrowLeft onto the node.
    Right,
}

/// A node view's key callback — see [`NodeViewSpec::on_key`].
pub type NodeViewKey = Rc<dyn Fn(&EditorKey<'_>) -> bool>;

/// What renders an app's component for a node: given the view's own scope and its
/// [`NodeViewCtx`], the root node it built.
pub type NodeViewRender = Rc<dyn Fn(&mut RenderScope, NodeViewCtx) -> NodeHandle>;

/// A node view: which nodes it draws, and how — see the [module](self) docs and
/// [`EditorHandle::register_node_view`].
#[derive(Clone)]
pub struct NodeViewSpec {
    /// The node type it draws (`"embed"`). Only a **leaf** node type (one with no
    /// content of its own) can have a view; for any other the spec is never used.
    pub node_type: String,
    /// Whether it draws a node with these attributes. Asked when the node's host is
    /// built and whenever its attributes change; a node it stops matching is
    /// unmounted (and mounted by the next spec that matches, if any).
    pub matches: Rc<dyn Fn(&Attrs) -> bool>,
    /// Build the component. Its root is placed inside the node's host element.
    pub render: NodeViewRender,
    /// Offered when a keyboard caret moves onto the node (an arrow key), after the
    /// node has been node-selected: `true` says the view took the keyboard (it
    /// focused something of its own), `false` leaves the node selected.
    pub on_caret_enter: Option<Rc<dyn Fn(CaretFrom) -> bool>>,
    /// Offered every key pressed while the keyboard is **inside** the view (an
    /// input in it holds focus), before that element sees it: `true` consumes it.
    /// An Escape it does not consume gives the keyboard back to the editor, with
    /// the caret right after the node ([`NodeViewCtx::return_focus`]).
    pub on_key: Option<NodeViewKey>,
}

impl NodeViewSpec {
    /// A view of every node of `node_type`, drawn by `render`.
    pub fn new(
        node_type: impl Into<String>,
        render: impl Fn(&mut RenderScope, NodeViewCtx) -> NodeHandle + 'static,
    ) -> NodeViewSpec {
        NodeViewSpec {
            node_type: node_type.into(),
            matches: Rc::new(|_| true),
            render: Rc::new(render),
            on_caret_enter: None,
            on_key: None,
        }
    }

    /// Draw only the nodes whose attributes `matches` accepts.
    pub fn matches(mut self, matches: impl Fn(&Attrs) -> bool + 'static) -> NodeViewSpec {
        self.matches = Rc::new(matches);
        self
    }

    /// See [`NodeViewSpec::on_caret_enter`].
    pub fn on_caret_enter(mut self, f: impl Fn(CaretFrom) -> bool + 'static) -> NodeViewSpec {
        self.on_caret_enter = Some(Rc::new(f));
        self
    }

    /// See [`NodeViewSpec::on_key`].
    pub fn on_key(mut self, f: impl Fn(&EditorKey<'_>) -> bool + 'static) -> NodeViewSpec {
        self.on_key = Some(Rc::new(f));
        self
    }
}

impl fmt::Debug for NodeViewSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeViewSpec")
            .field("node_type", &self.node_type)
            .field("on_caret_enter", &self.on_caret_enter.is_some())
            .field("on_key", &self.on_key.is_some())
            .finish_non_exhaustive()
    }
}

/// What a node view's component is given: its node's attributes and state, and
/// the verbs that change its node.
///
/// The memos follow the node for the whole mount; read them in reactive closures
/// (`{|| ctx.attrs.get().get_str("data-sheet")…}`). They belong to the view's
/// scope and are freed with it.
#[derive(Clone)]
pub struct NodeViewCtx {
    /// The node's attributes (its `data-*` keys, for an `embed`).
    pub attrs: Memo<Attrs>,
    /// Whether the node is node-selected (a click on its frame, an arrow onto it).
    pub selected: Memo<bool>,
    /// Whether the editor is [read-only](EditorHandle::set_read_only).
    pub read_only: Memo<bool>,
    /// Whether the editor is in [dark mode](EditorHandle::set_dark_mode).
    pub dark_mode: Memo<bool>,
    editor: WeakHandle,
    slot: Weak<Slot>,
}

impl fmt::Debug for NodeViewCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeViewCtx").finish_non_exhaustive()
    }
}

impl NodeViewCtx {
    fn live(&self) -> Option<(EditorHandle, Rc<Slot>)> {
        let slot = self.slot.upgrade()?;
        if slot.state.borrow().removed {
            return None;
        }
        Some((self.editor.upgrade()?, slot))
    }

    /// The model position just before the node, or `None` once it has left the
    /// document (or the editor is gone).
    pub fn pos(&self) -> Option<Pos> {
        let (editor, slot) = self.live()?;
        editor.node_view_pos(&slot).map(Pos)
    }

    /// Change the node's attributes: `f` is handed a copy to edit, and every key it
    /// changed is written with a `SetNodeAttrStep` — one transaction, one undo
    /// step, sent to collaborators key by key. The view stays mounted. `false`
    /// when nothing changed, the node is gone, or the editor refuses edits
    /// ([read-only](EditorHandle::set_read_only)).
    pub fn set_attrs(&self, f: impl FnOnce(&mut Attrs)) -> bool {
        let Some((editor, _)) = self.live() else {
            return false;
        };
        let Some(pos) = self.pos() else {
            return false;
        };
        let Some(node) = editor.doc().node_at(pos.0) else {
            return false;
        };
        let old = node.attrs().clone();
        let mut new = old.clone();
        f(&mut new);
        if new == old {
            return false;
        }
        editor.update(move |state| {
            let mut tr = state.tr();
            for (key, value) in new.iter() {
                if old.get(key) != Some(value) {
                    tr.step(Box::new(SetNodeAttrStep::new(pos.0, key, value.clone())))
                        .ok()?;
                }
            }
            for (key, _) in old.iter() {
                if new.get(key).is_none() {
                    tr.step(Box::new(SetNodeAttrStep {
                        pos: pos.0,
                        attr: key.to_string(),
                        value: None,
                    }))
                    .ok()?;
                }
            }
            Some(tr)
        })
    }

    /// Delete the node (one undo step). `false` when it is gone already or the
    /// editor refuses edits.
    pub fn delete(&self) -> bool {
        let Some((editor, _)) = self.live() else {
            return false;
        };
        let Some(pos) = self.pos() else {
            return false;
        };
        editor.update(|state| {
            let size = state.doc.node_at(pos.0)?.node_size();
            let mut tr = state.tr();
            tr.delete(pos.0, pos.0 + size).ok()?;
            Some(tr)
        })
    }

    /// Node-select the node and give the editor the keyboard.
    pub fn select(&self) {
        let Some((editor, _)) = self.live() else {
            return;
        };
        if let Some(pos) = self.pos()
            && let Some(sel) = Selection::node_at(&editor.doc(), pos)
        {
            editor.set_selection(sel);
        }
        editor.focus();
    }

    /// Give the keyboard back to the editor with the caret right after the node:
    /// the start of the next textblock, or — when nothing follows the node — the
    /// node itself, node-selected. What an unconsumed Escape in the view does.
    pub fn return_focus(&self) {
        let Some((editor, _)) = self.live() else {
            return;
        };
        if let Some(pos) = self.pos() {
            editor.return_focus_after(pos);
        }
    }

    /// The editor the node is in.
    pub fn editor(&self) -> Option<EditorHandle> {
        self.editor.upgrade()
    }
}

/// Keeps a [`register_node_view`](EditorHandle::register_node_view)
/// registration: dropping it unregisters the spec, and every view it drew is
/// unmounted (and remounted by the next spec that matches its node, if any).
///
/// A registration made while a component renders is also released when that
/// component unmounts. To keep it for exactly that long — or, made outside any
/// render, for the editor's life — call [`keep`](Self::keep).
#[must_use = "dropping the registration unregisters the node view at once; call `.keep()` to tie it to the rendering component (or the editor)"]
pub struct NodeViewRegistration {
    editor: WeakHandle,
    token: u64,
}

impl NodeViewRegistration {
    pub(crate) fn new(editor: WeakHandle, token: u64) -> NodeViewRegistration {
        NodeViewRegistration { editor, token }
    }

    /// Keep the registration until its registering component unmounts — or, when
    /// it was made outside any render, for as long as the editor lives.
    pub fn keep(self) {
        std::mem::forget(self);
    }
}

impl fmt::Debug for NodeViewRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeViewRegistration")
            .field("token", &self.token)
            .finish()
    }
}

impl Drop for NodeViewRegistration {
    fn drop(&mut self) {
        if let Some(editor) = self.editor.upgrade() {
            editor.unregister_node_view(self.token);
        }
    }
}

// ─── The editor's side ────────────────────────────────────────────────────────

/// One registered spec, with the scope that registered it.
pub(crate) struct Entry {
    pub(crate) token: u64,
    pub(crate) spec: NodeViewSpec,
    owner: Option<Owner>,
}

impl Entry {
    fn is_live(&self) -> bool {
        self.owner.as_ref().is_none_or(Owner::is_alive)
    }

    /// Run app code of this spec: untracked, inside its registering owner, or not
    /// at all once that owner is gone.
    fn invoke<R>(&self, call: impl FnOnce() -> R) -> Option<R> {
        match &self.owner {
            Some(owner) if !owner.is_alive() => None,
            Some(owner) => Some(untracked_handler(|| owner.run(call))),
            None => Some(untracked_handler(|| unowned(call))),
        }
    }
}

/// An editor's registered specs and the switches every view follows. Shared by
/// the handle and its view (`Rc`); never borrowed across app code.
#[derive(Default)]
pub(crate) struct Registry {
    entries: RefCell<Vec<Rc<Entry>>>,
    next: Cell<u64>,
    pub(crate) read_only: Cell<bool>,
    pub(crate) dark: Cell<bool>,
}

impl Registry {
    /// Add `spec`, owned by the ambient owner; its token.
    pub(crate) fn add(&self, spec: NodeViewSpec) -> u64 {
        let token = self.next.get();
        self.next.set(token + 1);
        self.entries.borrow_mut().push(Rc::new(Entry {
            token,
            spec,
            owner: current_owner(),
        }));
        token
    }

    /// Remove the spec registered with `token`; whether it was there.
    pub(crate) fn remove(&self, token: u64) -> bool {
        let mut entries = self.entries.borrow_mut();
        let before = entries.len();
        entries.retain(|e| e.token != token);
        entries.len() != before
    }

    /// Whether a live spec draws nodes of `type_name` (the view gives such a node
    /// a [`Slot`]).
    pub(crate) fn claims(&self, type_name: &str) -> bool {
        self.entries
            .borrow()
            .iter()
            .any(|e| e.spec.node_type == type_name && e.is_live())
    }

    /// Whether any spec is registered at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }

    fn for_type(&self, type_name: &str) -> Vec<Rc<Entry>> {
        self.entries
            .borrow()
            .iter()
            .filter(|e| e.spec.node_type == type_name)
            .cloned()
            .collect()
    }

    /// The live spec registered with `token`.
    fn entry(&self, token: u64) -> Option<Rc<Entry>> {
        self.entries
            .borrow()
            .iter()
            .find(|e| e.token == token && e.is_live())
            .cloned()
    }
}

/// A node that a spec may draw: its host element, the model node it shows now,
/// and the mounted view, if any. Owned by the node's descriptor in the view.
pub(crate) struct Slot {
    /// The node's host element. An inline leaf's (an `image`'s) is swapped for an
    /// inline-block `<span>` while a view is mounted, and back when it unmounts
    /// ([`EditorHandle::swap_node_view_host`]): an `<img>` cannot hold children.
    host: RefCell<NodeHandle>,
    /// Whether the node is inline — its host is swapped while a view is mounted.
    inline: bool,
    /// Whether [`Self::host`] is the `<span>` a view is mounted in, not the
    /// node's own element.
    view_host: Cell<bool>,
    doc: Weak<RefCell<dyn DomDocument>>,
    state: RefCell<SlotState>,
}

struct SlotState {
    node: Node,
    removed: bool,
    selected: bool,
    mount: Option<Mount>,
    /// `render` is running for this slot. A sync it causes (the component
    /// setting its own node's attributes, a default) must not mount the slot a
    /// second time under it: it is noted in `resync`, and the slot is matched
    /// again once the mount is stored (review of #1575, F7; review 2, N5).
    mounting: bool,
    resync: bool,
}

/// A mounted view: its scope, root, and the signals behind its context's memos
/// (with the values they were last given, so an unchanged value is not written).
struct Mount {
    token: u64,
    scope: RenderScope,
    content: NodeHandle,
    attrs: (Signal<Attrs>, Attrs),
    selected: (Signal<bool>, bool),
    read_only: (Signal<bool>, bool),
    dark: (Signal<bool>, bool),
}

impl Slot {
    pub(crate) fn new(
        host: NodeHandle,
        doc: Weak<RefCell<dyn DomDocument>>,
        node: Node,
    ) -> Rc<Slot> {
        let inline = node.is_inline();
        Rc::new(Slot {
            host: RefCell::new(host),
            inline,
            view_host: Cell::new(false),
            doc,
            state: RefCell::new(SlotState {
                node,
                removed: false,
                selected: false,
                mount: None,
                mounting: false,
                resync: false,
            }),
        })
    }

    /// The node's host element now.
    pub(crate) fn host(&self) -> NodeHandle {
        self.host.borrow().clone()
    }

    /// The host is now `host` (an inline leaf's swap).
    pub(crate) fn set_host(&self, host: NodeHandle, view_host: bool) {
        *self.host.borrow_mut() = host;
        self.view_host.set(view_host);
    }

    /// Whether the host is the `<span>` a view of an inline leaf is mounted in.
    pub(crate) fn has_view_host(&self) -> bool {
        self.view_host.get()
    }

    /// The model node now shown (an attribute change of the same node).
    pub(crate) fn set_node(&self, node: Node) {
        self.state.borrow_mut().node = node;
    }

    /// The node left the document; the view is disposed by the next run.
    pub(crate) fn mark_removed(&self) {
        self.state.borrow_mut().removed = true;
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.state.borrow().removed
    }

    /// Whether the node is node-selected; `true` when this changed it.
    pub(crate) fn set_selected(&self, selected: bool) -> bool {
        let mut st = self.state.borrow_mut();
        let changed = st.selected != selected;
        st.selected = selected;
        changed
    }

    pub(crate) fn is_mounted(&self) -> bool {
        self.state.borrow().mount.is_some()
    }

    /// The registration token of the mounted view.
    fn mounted_token(&self) -> Option<u64> {
        self.state.borrow().mount.as_ref().map(|m| m.token)
    }
}

/// Work the view's diff queues for when no borrow is held — see [`run`].
/// A node a transaction only set attributes of ([`crate::handle::kept_nodes`]):
/// where it stands in the new document, and the node it was before. A node view's
/// descriptor holding `old` may be patched with the node at `pos` — the one way a
/// view's node changes without a remount.
pub(crate) struct Kept {
    pub(crate) pos: usize,
    pub(crate) old: Node,
    /// Where `old` stood in the transaction's first document: one `Rc` can stand in
    /// two places (an in-editor copy), and only this says which of them changed.
    pub(crate) old_pos: usize,
}

pub(crate) enum Work {
    /// Mount, re-match, update or (once removed) dispose this slot's view.
    Sync(Rc<Slot>),
    /// Discard a detached subtree that held views, after they are disposed.
    Discard(NodeHandle),
    /// Bring these views' read-only, dark-mode and selected memos up to date.
    Flags(Vec<Rc<Slot>>),
}

/// The editor's container and document key, for the host listener
/// ([`crate::registry::set_node_view_host_listener`]).
pub(crate) struct HostCx {
    pub(crate) doc_key: u64,
    pub(crate) container: u64,
}

/// Run queued node-view work for `editor`, with no editor borrow held: dispose
/// the views of removed nodes (their scopes first, so a cleanup still sees its own
/// nodes), discard the subtrees they were in, then mount, re-match or update the
/// rest, then the flags.
pub(crate) fn run(editor: &EditorHandle, work: Vec<Work>) {
    let registry = editor.node_view_registry();
    let host_cx = editor.node_view_host_cx();
    let mut syncs = Vec::new();
    let mut discards = Vec::new();
    let mut flags = Vec::new();
    for w in work {
        match w {
            Work::Sync(slot) if slot.is_removed() => unmount(&slot, false, host_cx.as_ref()),
            Work::Sync(slot) => syncs.push(slot),
            Work::Discard(node) => discards.push(node),
            Work::Flags(slots) => flags.extend(slots),
        }
    }
    for node in discards {
        node.discard();
    }
    for slot in syncs {
        sync(editor, &registry, &slot, host_cx.as_ref());
    }
    for slot in flags {
        refresh(&registry, &slot);
    }
}

/// Bring `slot`'s view in line with its node: mount the first matching spec,
/// remount when another spec matches now, unmount when none does, else update
/// the memos.
fn sync(editor: &EditorHandle, registry: &Registry, slot: &Rc<Slot>, host_cx: Option<&HostCx>) {
    if slot.is_removed() {
        unmount(slot, false, host_cx);
        return;
    }
    {
        let mut st = slot.state.borrow_mut();
        if st.mounting {
            st.resync = true;
            return;
        }
    }
    let node = slot.state.borrow().node.clone();
    let matched = registry
        .for_type(node.type_name())
        .into_iter()
        .find(|e| e.invoke(|| (e.spec.matches)(node.attrs())).unwrap_or(false));
    match (slot.mounted_token(), matched) {
        (Some(current), Some(entry)) if current == entry.token => refresh(registry, slot),
        (current, matched) => {
            if current.is_some() {
                unmount(slot, true, host_cx);
            }
            match matched {
                Some(entry) => mount(editor, registry, slot, &entry, host_cx),
                None => give_back_host(editor, slot),
            }
        }
    }
}

/// Mount `entry`'s view for `slot`.
fn mount(
    editor: &EditorHandle,
    registry: &Registry,
    slot: &Rc<Slot>,
    entry: &Rc<Entry>,
    host_cx: Option<&HostCx>,
) {
    let Some(doc) = slot.doc.upgrade() else {
        return;
    };
    let (node, selected) = {
        let st = slot.state.borrow();
        (st.node.clone(), st.selected)
    };
    let (read_only, dark) = (registry.read_only.get(), registry.dark.get());
    // An inline leaf's own element (an `<img>`) cannot hold the component: it is
    // swapped for an inline-block `<span>` first, and back if the mount fails.
    if slot.inline && !slot.has_view_host() && !editor.swap_node_view_host(slot, true) {
        return;
    }
    let mut scope = RenderScope::new(doc, slot.host().node_id());
    let (signals, ctx) = untracked_handler(|| {
        scope.build(|_| {
            let attrs = Signal::new(node.attrs().clone());
            let sel = Signal::new(selected);
            let ro = Signal::new(read_only);
            let dk = Signal::new(dark);
            let ctx = NodeViewCtx {
                attrs: Memo::new(move || attrs.get()),
                selected: Memo::new(move || sel.get()),
                read_only: Memo::new(move || ro.get()),
                dark_mode: Memo::new(move || dk.get()),
                editor: editor.weak(),
                slot: Rc::downgrade(slot),
            };
            ((attrs, sel, ro, dk), ctx)
        })
    });
    let render = entry.spec.render.clone();
    slot.state.borrow_mut().mounting = true;
    let content = entry.invoke(|| scope.build(|s| render(s, ctx)));
    let resync = {
        let mut st = slot.state.borrow_mut();
        st.mounting = false;
        std::mem::take(&mut st.resync)
    };
    let Some(content) = content else {
        scope.dispose();
        give_back_host(editor, slot);
        return;
    };
    // The render is app code: it may have removed its own node, or (re-entering
    // the editor) mounted this slot already.
    if slot.is_removed() || slot.is_mounted() {
        scope.dispose();
        content.discard();
        if !slot.is_mounted() {
            give_back_host(editor, slot);
        }
        return;
    }
    let host = slot.host();
    host.append_child(&content);
    host.set_attribute(NODE_VIEW_ATTR, "");
    let (attrs, sel, ro, dk) = signals;
    slot.state.borrow_mut().mount = Some(Mount {
        token: entry.token,
        scope,
        content,
        attrs: (attrs, node.attrs().clone()),
        selected: (sel, selected),
        read_only: (ro, read_only),
        dark: (dk, dark),
    });
    if let Some(cx) = host_cx {
        crate::registry::notify_node_view_host(cx.doc_key, cx.container, &slot.host(), true);
    }
    // What `render` itself changed (its node's attributes, say) is matched now,
    // as any other change of the node is: the same spec still matching is a
    // refresh of the memos (the token is equal, so no second render), another
    // spec or none remounts or unmounts. A render that keeps flipping which spec
    // matches would recurse here without end, so past `MAX_RESYNC_DEPTH` nested
    // re-matches the memos are refreshed instead (review 2 of #1575, N5).
    if resync {
        struct Depth;
        impl Drop for Depth {
            fn drop(&mut self) {
                RESYNC_DEPTH.with(|d| d.set(d.get() - 1));
            }
        }
        if RESYNC_DEPTH.with(|d| d.get()) < MAX_RESYNC_DEPTH {
            RESYNC_DEPTH.with(|d| d.set(d.get() + 1));
            let _depth = Depth;
            sync(editor, registry, slot, host_cx);
        } else {
            refresh(registry, slot);
        }
    }
}

/// An inline leaf's `<span>` host back to its own element, unless the node left
/// the document (its host goes with it).
fn give_back_host(editor: &EditorHandle, slot: &Rc<Slot>) {
    if slot.has_view_host() && !slot.is_removed() {
        editor.swap_node_view_host(slot, false);
    }
}

/// How many re-matches a render's own change may cause inside one another.
const MAX_RESYNC_DEPTH: u32 = 8;

thread_local! {
    /// The re-matches [`mount`] is running inside one another.
    static RESYNC_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Dispose `slot`'s mounted view, if any: its scope first, then (unless the host
/// subtree goes as a whole) its root.
fn unmount(slot: &Rc<Slot>, discard_content: bool, host_cx: Option<&HostCx>) {
    let mount = slot.state.borrow_mut().mount.take();
    let Some(mount) = mount else {
        return;
    };
    mount.scope.dispose();
    let host = slot.host();
    if discard_content {
        mount.content.discard();
        host.remove_attribute(NODE_VIEW_ATTR);
    }
    if let Some(cx) = host_cx {
        crate::registry::notify_node_view_host(cx.doc_key, cx.container, &host, false);
    }
}

/// Dispose every view in `slots` for good — the editor's unmount: scopes first,
/// then each view's root (the editor's own nodes are its owner's to discard).
pub(crate) fn dispose_all(slots: Vec<Rc<Slot>>, host_cx: Option<&HostCx>) {
    for slot in slots {
        slot.mark_removed();
        unmount(&slot, true, host_cx);
    }
}

/// Write the memos' signals that changed.
fn refresh(registry: &Registry, slot: &Rc<Slot>) {
    let (writes, attrs) = {
        let mut st = slot.state.borrow_mut();
        let node_attrs = st.node.attrs().clone();
        let selected = st.selected;
        let Some(m) = st.mount.as_mut() else {
            return;
        };
        let mut writes: Vec<(Signal<bool>, bool)> = Vec::new();
        for (cell, want) in [
            (&mut m.selected, selected),
            (&mut m.read_only, registry.read_only.get()),
            (&mut m.dark, registry.dark.get()),
        ] {
            if cell.1 != want {
                cell.1 = want;
                writes.push((cell.0, want));
            }
        }
        let attrs = (m.attrs.1 != node_attrs).then(|| {
            m.attrs.1 = node_attrs.clone();
            (m.attrs.0, node_attrs)
        });
        (writes, attrs)
    };
    // No borrow held: a write runs effects.
    if let Some((signal, value)) = attrs {
        signal.set(value);
    }
    for (signal, value) in writes {
        signal.set(value);
    }
}

/// Offer `key` to the view mounted on `slot`; see [`NodeViewSpec::on_key`].
pub(crate) fn offer_key(registry: &Registry, slot: &Rc<Slot>, key: &EditorKey<'_>) -> Option<bool> {
    let entry = registry.entry(slot.mounted_token()?)?;
    let on_key = entry.spec.on_key.clone()?;
    entry.invoke(|| on_key(key))
}

/// Offer a caret arriving from `from` to the view mounted on `slot`.
pub(crate) fn offer_caret_enter(registry: &Registry, slot: &Rc<Slot>, from: CaretFrom) -> bool {
    let Some(entry) = slot.mounted_token().and_then(|t| registry.entry(t)) else {
        return false;
    };
    let Some(f) = entry.spec.on_caret_enter.clone() else {
        return false;
    };
    entry.invoke(|| f(from)).unwrap_or(false)
}
