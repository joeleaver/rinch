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

use rinch_core::dom::{CaretAffinity, DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::{Owner, current_owner, unowned, untracked_handler};
use rinch_editor_core::commands::{current_block_type, in_node_type, is_mark_active, marks_at};
use rinch_editor_core::model::{Fragment, Slice};
use rinch_editor_core::serialize::{
    clipboard_slice, slice_from_html, slice_from_pasted_html, slice_from_text, slice_to_html,
    slice_to_text,
};
use rinch_editor_core::transform::Mapping;
use rinch_editor_core::{
    CursorMotion, EditorState, EditorView, KeyBinding, Mark, Node, PasteContent, PlaceholderPlugin,
    Plugin, PluginKey, Pos, Schema, Selection, Transaction, ViewRequest, apply_input_rules,
};

#[cfg(feature = "collaboration")]
use rinch_editor_collab::{
    CollabError, CollabSession, OversizedTable, StickyAssoc, StickyLocation,
};

#[cfg(feature = "collaboration")]
use super::collab::CollabBridge;
use super::images::ImageHover;
use super::images::{ImageInput, ImageInputSource};
use super::keys::{EditorKey, TabBehavior};
use super::links::{LinkClick, LinkHover, LinkSpan};
use super::margins::{self, MarginAnchor, MarginLine, Margins};
use super::node_views::{self, CaretFrom, Kept, NodeViewRegistration, NodeViewSpec};
use super::registry;
use super::view::RinchDomEditorView;

mod anchors;
mod callbacks;
#[cfg(feature = "collaboration")]
mod collaboration;
mod content;
mod editor_core;
mod focus_scroll;
mod guards;
mod image_input;
mod ime;
mod lifecycle;
mod node_view_bridge;
mod queries;
mod selection;
mod settings;
#[cfg(test)]
mod tests;

use anchors::AnchorMap;
pub use anchors::SelectionAnchor;

/// An [`EditorHandle::on_link_click`] callback, with its owner (see [`Hook`]).
type LinkClickHook = Hook<dyn Fn(&LinkClick) -> bool>;
/// An [`EditorHandle::on_link_hover`] callback, with its owner (see [`Hook`]).
type LinkHoverHook = Hook<dyn Fn(Option<&LinkHover>)>;
/// An [`EditorHandle::on_image_hover`] callback, with its owner (see [`Hook`]).
type ImageHoverHook = Hook<dyn Fn(Option<&ImageHover>)>;
/// An [`EditorHandle::on_margin_layout`] callback, with its owner (see [`Hook`]).
type MarginHook = Hook<dyn Fn(&[MarginLine])>;

/// Whether `prev → next` only shifted the selection: the same kind, both ends
/// moved by one amount, and the head at the same offset in a textblock of the
/// same type and attrs whose content **up to the head** is unchanged — what
/// comes after it (a peer typing at the paragraph's end) cannot move the head
/// to another line. See [`EditorCore::carry_caret_hint`].
fn shifted_in_place(prev: &EditorState, next: &EditorState) -> bool {
    let (p, n) = (&prev.selection, &next.selection);
    if std::mem::discriminant(p) != std::mem::discriminant(n) {
        return false;
    }
    let delta = |a: Pos, b: Pos| b.0 as isize - a.0 as isize;
    if delta(p.head(), n.head()) != delta(p.anchor(), n.anchor()) {
        return false;
    }
    let (Ok(rp), Ok(rn)) = (prev.doc.resolve(p.head()), next.doc.resolve(n.head())) else {
        return false;
    };
    let off = rp.parent_offset();
    let (bp, bn) = (rp.parent(), rn.parent());
    bp.is_textblock()
        && off == rn.parent_offset()
        && (bp.same_ref(bn)
            || (bp.node_type() == bn.node_type()
                && bp.attrs() == bn.attrs()
                && bp.content().cut(0, off) == bn.content().cut(0, off)))
}

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
    on_change: Option<Hook<dyn Fn()>>,
    /// See [`EditorHandle::on_link_click`]. An `Rc` for the same reason as
    /// `on_change`: it is cloned out and called with no borrow held.
    on_link_click: Option<LinkClickHook>,
    /// See [`EditorHandle::on_link_hover`]. While it is `Some` this editor is
    /// counted in [`registry::link_hover_wanted`], which is what lets a
    /// runtime skip link hover entirely on a pointer move when no editor asked.
    on_link_hover: Option<LinkHoverHook>,
    /// See [`EditorHandle::on_image_hover`]; counted in
    /// [`registry::image_hover_wanted`] while `Some`, as `on_link_hover` is.
    /// Carries its owner, like `on_change`: a callback whose component has
    /// unmounted is not called.
    on_image_hover: Option<ImageHoverHook>,
    /// Offered every key press before the editor acts on it — see
    /// [`EditorHandle::on_key`]. Cloned out and called with no borrow held.
    on_key: Option<KeyHook>,
    /// Offered every picture pasted into or dropped on the editor — see
    /// [`EditorHandle::on_image_input`]. Cloned out and called with no borrow
    /// held.
    on_image_input: Option<ImageInputHook>,
    /// Told when the selection changes — see
    /// [`EditorHandle::on_selection_change`]. Cloned out and called with no
    /// borrow held, by [`CoreMutGuard`]'s drop.
    ///
    /// [`CoreMutGuard`]: guards::CoreMutGuard
    on_selection_change: Option<SelectionHook>,
    /// A change of selection is owed to [`Self::on_selection_change`]: set by
    /// [`Self::note_selection`] when the stored selection changed and a
    /// callback is registered, taken by [`CoreMutGuard`]'s drop once the
    /// borrow is released. Never set while no callback is registered.
    ///
    /// [`CoreMutGuard`]: guards::CoreMutGuard
    selection_owed: bool,
    /// Told when the caret pass moved the overlays — see
    /// [`EditorHandle::on_caret_moved`]. Called by [`CoreMutGuard`]'s drop.
    ///
    /// [`CoreMutGuard`]: guards::CoreMutGuard
    on_caret_moved: Option<Hook<dyn Fn()>>,
    /// A call is owed to [`Self::on_caret_moved`]; set by
    /// [`EditorHandle::update_caret`] only while a callback is registered.
    caret_moved_owed: bool,
    /// Selections captured by asynchronous work still in flight — see
    /// [`SelectionAnchor`]. Empty for every editor that has none, so the
    /// mutation path's carry step is a cheap early return.
    anchors: Rc<RefCell<AnchorMap>>,
    /// Whether local edits are refused — see [`EditorHandle::set_read_only`].
    /// Enforced in exactly one place: [`Self::refuses`], which [`Self::commit`]
    /// asks before it stores anything. Every local change lands in `commit`, so
    /// an input path added later is read-only without knowing this flag exists.
    read_only: bool,
    /// The placeholder this handle installed — see
    /// [`EditorHandle::set_placeholder`]. A clone of the installed plugin,
    /// sharing its text.
    placeholder: Option<PlaceholderPlugin>,
    /// What Tab does — see [`EditorHandle::set_tab_behavior`].
    tab_behavior: TabBehavior,
    /// Whether the caret owes a scroll into view — see [`ScrollGate`]. Armed by
    /// a local edit or selection move ([`Self::commit`] with
    /// [`Scroll::IfChanged`]); fulfilled and cleared by
    /// [`EditorHandle::update_caret`].
    scroll: ScrollGate,
    /// A range an app asked to see — see [`EditorHandle::scroll_into_view`].
    /// Fulfilled and cleared by [`EditorHandle::reveal_pass`] on the first pass
    /// with geometry for it; carried through local edits, dropped by a load.
    /// `None` for every editor that never asked, so the pass is one check.
    reveal: Option<(Pos, Pos, ScrollAlign)>,
    /// Overlay passes the pending `reveal` has found no geometry on; it is
    /// dropped at [`REVEAL_PATIENCE`]. Reset by each new request.
    reveal_misses: u8,
    /// The caret-affinity hint (#301) and the selection it was set with — see
    /// [`EditorHandle::caret_affinity`]. It applies only while the selection is
    /// still that one, so every other writer (an edit, undo, a load, a remote
    /// change that moves the caret) retires it without knowing it exists.
    caret_hint: Option<(Selection, CaretAffinity)>,
    /// The collaboration session + outbound delta sink, when this editor is
    /// collaborating (design M9). `None` for a non-collaborative editor — the
    /// common case — so the mutation path's collab hook is a cheap early return.
    #[cfg(feature = "collaboration")]
    collab: Option<CollabBridge>,
    /// The registered node views ([`EditorHandle::register_node_view`]), shared
    /// with the view.
    node_views: Rc<node_views::Registry>,
    /// The margin anchors ([`EditorHandle::set_margin_anchors`]) and what the last
    /// margin pass reported. Empty for an editor that has none, so the pass is
    /// one check.
    margins: Margins,
    /// See [`EditorHandle::on_margin_layout`]. Called by the margin pass with no
    /// borrow held.
    on_margin_layout: Option<MarginHook>,
    /// The spellchecker's timer and configuration listener (#2862).
    spell: crate::spellcheck::Driver,
    /// The spellchecker generation the last mutable borrow saw: a new one
    /// with dirty words owes a check (`Owed::spell`).
    spell_seen: u64,
}

/// The nodes `tr` only set attributes of: for each node a `SetNodeAttrStep`
/// changed, where it stands in the transaction's final document and the node it
/// was when the transaction started. A node an earlier step put in place (a
/// replace, then an attribute set on what replaced it) or a later step removed is
/// not one of them. What proves a node view's node is the same node with other
/// attributes (node views, decision (d)).
pub(crate) fn kept_nodes(tr: &Transaction) -> Vec<Kept> {
    let (steps, mapping) = (tr.steps(), tr.mapping());
    let mut out: Vec<Kept> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let Some(a) = step
            .as_any()
            .downcast_ref::<rinch_editor_core::SetNodeAttrStep>()
        else {
            continue;
        };
        let after = mapping.slice(i + 1, mapping.len()).map_result(a.pos, 1);
        if after.deleted_after() || out.iter().any(|k| k.pos == after.pos) {
            continue;
        }
        // Back to the transaction's start, step by step (`Mapping::invert` takes
        // every map, not a slice's): a node an earlier step inserted is not kept.
        let mut before = a.pos;
        let mut inserted = false;
        for map in mapping.maps()[..i].iter().rev() {
            let r = map.invert().map_result(before, 1);
            inserted |= r.deleted_after();
            before = r.pos;
        }
        if inserted {
            continue;
        }
        if let Some(old) = tr.docs().first().and_then(|d| d.node_at(before)) {
            out.push(Kept {
                pos: after.pos,
                old,
                old_pos: before,
            });
        }
    }
    out
}

/// Whether the kept atom `k` names one place of the transaction's first document
/// `prev`: its old `Rc` stands nowhere else there, or every other copy of it is
/// still in place in `next`, untouched by the transaction (`mapping`), and none
/// was added. After an in-editor copy one `Rc` stands in two places, and a proof
/// naming only the `Rc` would let the collaboration projection pair the kept atom
/// with the other copy's CRDT char or block when that copy was deleted or moved in
/// the same transaction (the round-3 review of #1595, F2): such a pair proves
/// nothing, and the atom is written as a new one (lost, never misattributed).
#[cfg(feature = "collaboration")]
fn names_one_place(k: &Kept, prev: &Node, next: &Node, mapping: Option<&Mapping>) -> bool {
    let at = |doc: &Node| {
        let mut at = Vec::new();
        doc.nodes_between(0, doc.content_size(), &mut |n: &Node, pos, _| {
            if n.same_ref(&k.old) {
                at.push(pos);
            }
            !n.is_leaf()
        });
        at
    };
    let others: Vec<usize> = at(prev).into_iter().filter(|&p| p != k.old_pos).collect();
    if others.is_empty() {
        return true;
    }
    let Some(m) = mapping else {
        return false;
    };
    at(next).len() == others.len()
        && others.iter().all(|&p| {
            let r = m.map_result(p, 1);
            !r.deleted_after() && next.node_at(r.pos).is_some_and(|n| n.same_ref(&k.old))
        })
}

/// A weak [`EditorHandle`]: what a node view's context holds, so a view never
/// keeps its editor alive.
#[derive(Clone)]
pub(crate) struct WeakHandle(Weak<RefCell<EditorCore>>);

impl WeakHandle {
    pub(crate) fn upgrade(&self) -> Option<EditorHandle> {
        self.0.upgrade().map(|inner| EditorHandle { inner })
    }
}

/// The callbacks a mutable borrow owes on its way out — see [`CoreMutGuard`].
///
/// [`CoreMutGuard`]: guards::CoreMutGuard
struct Owed {
    selection: Option<(SelectionHook, Selection)>,
    caret_moved: Option<Hook<dyn Fn()>>,
    node_views: Vec<node_views::Work>,
    /// The spellchecker has newly dirty words to check after the pause.
    spell: bool,
}

/// An app callback and the component that registered it (#147/#183).
///
/// The owner is the ambient scope at registration — the component rendering
/// when `on_change`/`on_key`/`on_selection_change`/`on_caret_moved` was called,
/// whose signals the closure most likely captured. Once that scope is disposed
/// the callback is **not called** any more, because the handle can outlive the
/// component: an app-level `EditorHandle` handed to a note view that registers
/// its popup hooks and then unmounts, after which the app `load_html`s the next
/// note. A live callback runs *inside* its owner, so a signal it creates
/// belongs to that component. Registered outside any render (from `main`, a
/// timer), there is no owner and the callback keeps **app lifetime**, running
/// `unowned` — the menu registry's rule.
struct Hook<F: ?Sized> {
    cb: Rc<F>,
    /// `None` for app lifetime. An `Owner` is a `Weak`: this keeps nothing
    /// alive.
    owner: Option<Owner>,
}

impl<F: ?Sized> Clone for Hook<F> {
    fn clone(&self) -> Self {
        Hook {
            cb: self.cb.clone(),
            owner: self.owner.clone(),
        }
    }
}

impl<F: ?Sized> Hook<F> {
    /// Wrap `cb`, recording the ambient owner.
    fn new(cb: Rc<F>) -> Self {
        Hook {
            cb,
            owner: current_owner(),
        }
    }

    /// Whether the callback can still run: registered outside any render, or
    /// its owner not yet disposed.
    fn is_live(&self) -> bool {
        self.owner.as_ref().is_none_or(Owner::is_alive)
    }

    /// Run `call` on the callback inside its owner — or not at all, answering
    /// `None`, once the owner is disposed.
    ///
    /// **Untracked** (#931): an app effect that moves the selection, runs a
    /// command or offers a key calls these hooks synchronously, and what a hook
    /// reads (a popup's own `open` signal) must not become that effect's
    /// dependency. The whole observer stack is suspended, not only its top —
    /// see [`untracked_handler`].
    fn invoke<R>(&self, call: impl FnOnce(&F) -> R) -> Option<R> {
        match &self.owner {
            Some(owner) if !owner.is_alive() => None,
            Some(owner) => Some(untracked_handler(|| owner.run(|| call(&self.cb)))),
            None => Some(untracked_handler(|| unowned(|| call(&self.cb)))),
        }
    }
}

/// How many overlay passes in a row a pending
/// [`EditorHandle::scroll_into_view`] may find no geometry for its start before
/// it is dropped. A reachable reveal needs one or two (a virtualized editor's
/// block is laid out by the pass after the request, and desktop may run a
/// caret pass before an event's layout as well as after it); the margin is for
/// a runtime that runs more than one pass before its first layout.
pub const REVEAL_PATIENCE: u8 = 8;

/// See [`EditorHandle::on_key`].
type KeyHook = Hook<dyn Fn(&EditorKey<'_>) -> bool>;
/// See [`EditorHandle::on_image_input`].
type ImageInputHook = Hook<dyn Fn(ImageInput) -> Option<(String, String)>>;
/// See [`EditorHandle::on_selection_change`].
type SelectionHook = Hook<dyn Fn(&Selection)>;

/// Whether a [`EditorCore::commit`] should bring the caret into view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scroll {
    /// Yes, if the document or the selection changed.
    IfChanged,
    /// No — a load, an app transaction that only mapped the selection, a
    /// stored-marks reset.
    No,
}

/// Where [`EditorHandle::scroll_into_view_aligned`] puts a range in the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollAlign {
    /// The least scroll that shows the range, 16px inside the edge it comes in
    /// by; nothing moves when it is already in view. What
    /// [`EditorHandle::scroll_into_view`] does.
    Nearest,
    /// The start's line this fraction of the way down the scroll container's
    /// visible height — `0.0` the top edge, `1.0 / 3.0` a third down — kept
    /// 16px inside either edge, the scroll clamped to what the content allows.
    /// Clamped to `0.0..=1.0` (NaN reads as `0.0`).
    Fraction(f32),
}

impl ScrollAlign {
    fn clamped(self) -> ScrollAlign {
        match self {
            ScrollAlign::Fraction(f) if f.is_nan() => ScrollAlign::Fraction(0.0),
            ScrollAlign::Fraction(f) => ScrollAlign::Fraction(f.clamp(0.0, 1.0)),
            nearest => nearest,
        }
    }
}

/// Whether the caret owes a scroll into view: ProseMirror's
/// `tr.scrollIntoView()`, held on the handle because the request is made by a
/// commit and fulfilled by a later caret pass, once there is geometry.
///
/// **Armed** by a local edit or selection move ([`Scroll::IfChanged`]) — and
/// not by focus: a press that places the caret already arms through
/// `set_selection`, while one that does not (a task checkbox, a right-click on
/// an image) must not pull the user away from what they clicked (#846's
/// review measured 0 -> 2255). [`EditorHandle::focus`] does not arm it either,
/// for the same reason (giving the keyboard back after a dialog must not
/// jump); an app that wants something shown asks with
/// [`EditorHandle::scroll_into_view`], which does not go through this gate.
/// But a gate armed while the editor was blurred (a `set_selection` of a
/// caret) stays armed until a caret pass of the *focused* editor fulfils it,
/// so the next `focus()` performs it: focus arms nothing, and fulfils what
/// was already owed.
/// Never by a remote edit (`collab_receive` does not
/// commit), a load, a resize, a scroll or a virtualized block being measured —
/// those change the caret's *geometry* without the user moving it, and a gate on
/// geometry is what #837's review measured pulling a user who scrolled away back
/// to the caret.
///
/// **Fulfilled** on the first caret pass that has a scroll anchor to scroll to,
/// whether or not the overlay moved on that pass (a Delete leaves the caret
/// where it was and still reveals it). A pass without an anchor — the caret's
/// block is not measured yet — leaves it owed, so an Enter that makes an
/// unmeasured paragraph still scrolls once that paragraph is laid out.
///
/// **Dropped** on a pass whose selection is a non-empty text range: a range has
/// no scroll anchor (a shift-arrow head is not revealed), and an arm left over
/// from one must not fire when something later collapses the range.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct ScrollGate {
    pending: bool,
}

impl ScrollGate {
    fn arm(&mut self) {
        self.pending = true;
    }

    /// Advance the gate over one caret pass. `has_anchor`: there is an element
    /// to scroll to; `range`: the selection is a non-empty text range. Returns
    /// whether to scroll to the anchor now.
    fn on_pass(&mut self, has_anchor: bool, range: bool) -> bool {
        if range {
            self.pending = false;
            return false;
        }
        if self.pending && has_anchor {
            self.pending = false;
            return true;
        }
        false
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

/// A weak [`EditorHandle`] (tests only).
#[cfg(test)]
pub(crate) struct WeakEditorHandle(Weak<RefCell<EditorCore>>);

#[cfg(test)]
impl WeakEditorHandle {
    pub(crate) fn upgrade(&self) -> Option<EditorHandle> {
        self.0.upgrade().map(|inner| EditorHandle { inner })
    }
}

impl fmt::Debug for EditorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The state/view aren't `Debug`; the host id is the useful identity.
        let mounted = self.core().view.is_some();
        f.debug_struct("EditorHandle")
            .field("mounted", &mounted)
            .finish_non_exhaustive()
    }
}

/// [`EditorState::run_command_mapped`], with the transaction stamped for the
/// history's typing-merge window (#940; see [`crate::clock`]).
fn run_stamped(
    state: &EditorState,
    cmd: &rinch_editor_core::Command,
) -> Option<(EditorState, Mapping)> {
    let mut result: Option<(EditorState, Mapping)> = None;
    let applied = {
        let mut dispatch = |mut tr: Transaction| {
            crate::clock::stamp(&mut tr);
            let mapping = tr.mapping().clone();
            result = Some((state.apply(tr), mapping));
        };
        let dispatch: rinch_editor_core::Dispatch = &mut dispatch;
        cmd(state, Some(dispatch))
    };
    if applied { result } else { None }
}
