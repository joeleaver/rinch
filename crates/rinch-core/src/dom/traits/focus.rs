//! The [`DomFocus`] supertrait of [`DomDocument`](super::DomDocument):
//! focusing an element, a text control's selection and caret, the active
//! element, rendering, focusing into and restoring focus out of a subtree,
//! the work deferred until the document's borrow is released, and attribute
//! writes that revoke focus.

use super::content::DomContent;
use super::types::{FocusIntoPolicy, SelectionDirection};
use crate::dom::NodeId;

/// Keyboard focus and text-control selection.
///
/// One of the five supertraits [`DomDocument`](super::DomDocument) is made
/// of; see that trait for how they fit together. Its supertrait
/// [`DomContent`] gives [`select_text`](DomFocus::select_text) the control's
/// [`live_value`](DomContent::live_value).
pub trait DomFocus: DomContent {
    /// Focus a specific element programmatically.
    ///
    /// This sets the element as the currently focused element, allowing it to
    /// receive keyboard input. For input/textarea elements, this enables text input.
    ///
    /// # Arguments
    /// * `node_id` - The ID of the element to focus
    ///
    /// On the web this only **queues** the browser work: [`NodeHandle`](crate::dom::NodeHandle)'s verb
    /// runs it once its borrow is released. A direct caller must run
    /// [`take_after_borrow`](Self::take_after_borrow) after releasing its own.
    fn focus_element(&mut self, node_id: NodeId);

    /// Select a `[start, end)` range of a text control's text, in **UTF-16
    /// code units** — matching the DOM's `HTMLInputElement.setSelectionRange`
    /// exactly, so one offset pair means the same thing on desktop and on the
    /// web (issue #552). `start`/`end` out of order are swapped, as a
    /// browser's own `setSelectionRange` does; `direction` decides which end
    /// becomes the anchor (see [`SelectionDirection`]).
    ///
    /// **Applied now if `node_id` already holds the keyboard; stashed for the
    /// next time it gains it otherwise** — the same thing a browser does for
    /// `setSelectionRange`/`select()` called on an unfocused control: the
    /// range is set immediately, before the control is ever focused, and is
    /// simply what the control shows the first time it is. There is no
    /// "no-op on an unfocused control" case to document for a text control,
    /// unlike [`focus_element`](Self::focus_element) — a selection has
    /// somewhere to live even without the keyboard.
    ///
    /// **A node that is not a text control — a checkbox, a `<select>`, a
    /// generic `tabindex` node — is a no-op, silently, on desktop** (review of
    /// #1325, finding 4): such a node never installs the `EditableState` a
    /// selection would apply to, so there is nowhere for the "stash" half of
    /// the rule above to land, and desktop drops the request rather than
    /// holding it forever unconsumed. A browser's own `setSelectionRange`
    /// throws `InvalidStateError` on such a control (`<input type="checkbox">`,
    /// a `<select>`); rinch's backends do not raise an equivalent error —
    /// `rinch-web` gets the browser's own refusal for free (the call itself
    /// throws, caught and discarded), desktop's refusal is this paragraph.
    ///
    /// [`NodeHandle::set_selection_range`](crate::dom::NodeHandle::set_selection_range)
    /// is the caller-facing spelling. Defaulted to a no-op so a backend with
    /// no text-control selection to set (and `MockDomDocument`, which
    /// overrides it) keeps compiling.
    fn set_selection_range(
        &mut self,
        _node_id: NodeId,
        _start: usize,
        _end: usize,
        _direction: SelectionDirection,
    ) {
    }

    /// A text control's selection, `(start, end, direction)` in **UTF-16 code
    /// units** with `start <= end` — the DOM's `selectionStart`,
    /// `selectionEnd` and `selectionDirection` (issue #1678), the read half of
    /// [`set_selection_range`](Self::set_selection_range).
    ///
    /// - **Desktop** answers, newest first: a `set_selection_range` posted and
    ///   not yet applied; the focused control's live selection; the selection
    ///   the control had when it last lost the keyboard, or one stashed for its
    ///   next focus — a `value` write that changes an unfocused control's
    ///   value puts it at the end, as a browser's value setter does; else a
    ///   caret at the start, `(0, 0, Forward)`, Chrome 155's answer for a
    ///   parsed or default value. Offsets are clamped to the control's current
    ///   value, and an unspecified direction reads `Forward`, as Chrome on Linux
    ///   and Windows reads it (Chrome on macOS reads `"none"`; not measured).
    ///   The selection a programmatic or Tab focus shows is this one.
    /// - **Web** answers the element's own properties.
    ///
    /// `None` for a node that is not an `<input>` or `<textarea>` (a
    /// browser's `selectionStart` is `null` for an `<input>` whose type has no
    /// text selection — a checkbox — and rinch answers `None` there too).
    /// Defaulted to `None` so a backend with no text controls keeps compiling.
    fn text_selection_range(&self, _node_id: NodeId) -> Option<(usize, usize, SelectionDirection)> {
        None
    }

    /// Where a caret at UTF-16 offset `offset` of a text control's value is
    /// drawn — `None` for `offset` means at the selection's head, the caret
    /// itself (issue #1678). The rect is `width` 0 and as tall as the caret,
    /// in logical window px on desktop and client px on the web, the space
    /// `EditorHandle::caret_rect` answers in, so a popup is placed from it the
    /// same way on both.
    ///
    /// Desktop measures with the layout paint draws the field's text with
    /// (`paint_input_value`), so the answer is the painted caret, password
    /// bullets and a single-line field's centring included; an IME
    /// composition's text, which is not in the value, is not counted. The web
    /// lays the value out in a hidden copy of the control with its computed
    /// text styles, at the width it lays its text out in (browsers expose no
    /// caret geometry for a form control). Checked in Chrome 155 and Firefox
    /// 155 against `document.caretPositionFromPoint` at the rect's centre,
    /// which answers the same offset for a textarea scrolled under its
    /// scrollbar, right-to-left text, line starts after a break, `wrap="off"`
    /// and `white-space: pre`, a centred or horizontally scrolled `<input>`,
    /// a password field, spaced text, a field under `transform: scale(2)` and
    /// a scaled or zoomed page (Firefox's overlay-scrollbar strip excepted,
    /// where its caret-from-point answers the scrollbar); other styles the
    /// copy does not carry are not covered.
    ///
    /// `None` for a node that is not a text control, or not rendered
    /// (`display: none`, `visibility: hidden`, out of the document).
    fn text_caret_rect(
        &mut self,
        _node_id: NodeId,
        _offset: Option<usize>,
    ) -> Option<crate::reactive::ElementBounds> {
        None
    }

    /// Select a text control's **entire** text — `NodeHandle::select()`
    /// (issue #552), the DOM's `HTMLInputElement.select()`.
    ///
    /// The default is exactly `set_selection_range(node_id, 0, len, Forward)`
    /// where `len` is [`live_value`](DomContent::live_value)'s UTF-16 length (`0` if
    /// the node carries no value this backend can name), so a backend needs
    /// only to implement [`set_selection_range`](Self::set_selection_range) to
    /// get this for free — both inherit its "apply now or stash for the next
    /// focus" rule.
    fn select_text(&mut self, node_id: NodeId) {
        let len = self
            .live_value(node_id)
            .map(|v| v.encode_utf16().count())
            .unwrap_or(0);
        self.set_selection_range(node_id, 0, len, SelectionDirection::Forward);
    }

    /// The node that currently holds keyboard focus in this document, if any
    /// (issue #695).
    ///
    /// The portable read [`focus_element`](Self::focus_element) had no
    /// counterpart: an overlay could *give* focus and never find out who had it
    /// first, which is the whole of "restore it on close".
    ///
    /// - **Desktop** answers with the document's own `focused_node`, the DOM
    ///   mirror the focus arbiter keeps in step with `FocusTarget`.
    /// - **Web** answers with `document.activeElement` mapped back through the
    ///   `__nid` expando, so an element the browser focused that rinch did not
    ///   create (anything outside the mounted root) reads as `None` rather than
    ///   as somebody else's node.
    ///
    /// Defaulted to `None` — "this backend does not model focus" — so
    /// `MockDomDocument` and any other impl keep compiling. A caller must treat
    /// `None` as *unknown*, not as *nothing is focused*.
    fn active_element(&self) -> Option<NodeId> {
        None
    }

    /// Whether `node` is on screen as far as CSS is concerned: connected to
    /// this document, neither it nor any ancestor `display: none`, and its own
    /// `visibility` not `hidden` or `collapse` (issue #802).
    ///
    /// `visibility` is asked of the node alone because it inherits: a
    /// `visibility: hidden` ancestor reaches the node through its computed
    /// value, and a descendant that declares `visibility: visible` *is* shown,
    /// in CSS and here. `display` does not inherit, hence the walk. A
    /// `display: contents` node counts as rendered — it has no box of its own,
    /// but its children do.
    ///
    /// The question an open-time Escape entry asks before it consumes the key:
    /// an overlay whose ancestor stopped rendering it without closing it (a
    /// `ColorInput` picker inside a `Modal` the app closed) must not swallow an
    /// Escape the user cannot see it answer.
    ///
    /// Desktop answers from the styles of the **last resolve**, so a class
    /// change made since then and not yet laid out is not seen; the web asks
    /// the browser, which resolves style on demand. The default answers `true`
    /// — every caller of this question treats "rendered" as "behave as before"
    /// — which is what `MockDomDocument`, with no CSS engine, answers.
    fn is_rendered(&self, _node: NodeId) -> bool {
        true
    }

    /// Move keyboard focus *into* the subtree at `root`, the way a browser's
    /// `showModal()` does (issue #695).
    ///
    /// The element chosen is the first **focusable** descendant in DOM order,
    /// except that a descendant carrying `autofocus` wins wherever it sits —
    /// HTML's own rule. `policy` decides what happens when there is no
    /// `autofocus`: [`FocusIntoPolicy::FirstFocusable`] falls back to the first
    /// stop (a modal dialog), [`FocusIntoPolicy::AutofocusOnly`] moves nothing
    /// (the HTML popover API, which focuses an `auto` popover only when it asks
    /// to be focused).
    ///
    /// **Why this is a backend method and not a walk in the component.**
    /// "Focusable" is each backend's own computation and the two already differ
    /// by construction — desktop walks its tree with
    /// `RinchApp::collect_focusable_nodes_from`, `rinch-web` runs a CSS
    /// selector and then asks the browser. A third rule written in
    /// `rinch-components` would be wrong on both.
    ///
    /// **Desktop resolves this after the next layout, not now.** An overlay
    /// opening is a class removal in the same effect flush, so at call time its
    /// children still have zero-size boxes and every visibility filter would
    /// reject them. The desktop implementation therefore posts a request the
    /// runtime applies once layout has run, exactly as
    /// [`focus_element`](Self::focus_element) already does.
    ///
    /// On the web this only **queues** the browser work: [`NodeHandle`](crate::dom::NodeHandle)'s verb
    /// runs it once its borrow is released. A direct caller must run
    /// [`take_after_borrow`](Self::take_after_borrow) after releasing its own.
    ///
    /// Defaulted to a no-op.
    fn focus_into(&mut self, _root: NodeId, _policy: FocusIntoPolicy) {}

    /// The close half: give the keyboard back to `opener` now that the overlay
    /// rooted at `root` has closed, or let it go (issue #695).
    ///
    /// Three decisions, and **none of them can be made by the caller**, which is
    /// why this is one method and not a blur plus a hopeful focus:
    ///
    /// 1. **Is the claim this overlay's to return?** If the keyboard has moved
    ///    outside `root` since — the user clicked something on the page — it is
    ///    theirs and nothing happens at all. HTML's dialog rule, which returns
    ///    focus only when the dialog contained it (or nothing did).
    /// 2. **Can `opener` still take it?** Not merely "is it attached": an opener
    ///    that went `disabled` while the dialog worked, or that now sits inside
    ///    another overlay that has since closed, is connected and cannot be
    ///    focused. A caller that committed on attachment alone would leave the
    ///    claim inside the subtree that just went `display: none` — the state
    ///    this whole feature removes.
    /// 3. **If not, release** — but only the claim inside `root`, by rule 1.
    ///
    /// Both facts are a **layout** old in the caller's hands, because the close
    /// is a class change in the same effect flush. Desktop therefore parks this
    /// and answers it after the next layout pass; `rinch-web` answers now and
    /// lets the browser arbitrate, by focusing and then checking whether the
    /// focus took.
    ///
    /// `opener` is `None` when nothing held the keyboard at open time, which is
    /// a real answer and not a missing one: it means "there is nothing to hand
    /// back to", i.e. go straight to rule 3.
    ///
    /// On the web this only **queues** the browser work: [`NodeHandle`](crate::dom::NodeHandle)'s verb
    /// runs it once its borrow is released. A direct caller must run
    /// [`take_after_borrow`](Self::take_after_borrow) after releasing its own.
    ///
    /// Defaulted to a no-op.
    fn restore_focus(&mut self, _opener: Option<NodeId>, _root: NodeId) {}

    /// Work the last call left to run **once the caller has released its
    /// borrow of this document**, or `None`.
    ///
    /// [`NodeHandle::focus`], [`NodeHandle::focus_into`] and
    /// [`NodeHandle::restore_focus`] take this inside their `borrow_mut`, drop
    /// the borrow, then run it before they return — so the call is still
    /// complete when the handle method returns (an `active_element()` right
    /// after it answers the new focus), but nothing the work triggers sees the
    /// document borrowed.
    ///
    /// `rinch-web` needs it because a browser `focus()` / `blur()` runs every
    /// `focus`, `blur`, `focusin` and `focusout` listener synchronously, and
    /// rinch's own listeners touch the document (the editor repaints its caret
    /// on `blur`; `focusin` asks each key entry's owner for `active_element()`,
    /// which flushes pending effects). Called under the borrow, each of those
    /// panicked with "RefCell already borrowed" inside the listener, which
    /// aborted it (a key entry's `on_focus_leave`, the editor's caret repaint)
    /// and, since wasm does not unwind, left whatever that listener itself held
    /// (the editor's core) borrowed for good. The document was not: the frame
    /// that borrowed it drops the borrow normally. Desktop posts focus as a
    /// request the runtime applies later, outside any borrow, and answers
    /// `None`.
    ///
    /// **A direct caller of [`focus_element`](Self::focus_element),
    /// [`focus_into`](Self::focus_into) or
    /// [`restore_focus`](Self::restore_focus) must take this and run it after
    /// releasing its borrow.** On the web those three only queue; nothing else
    /// drains the queue until the next `NodeHandle` focus verb, by which time
    /// the work is stale.
    ///
    /// [`NodeHandle::focus`]: crate::dom::NodeHandle::focus
    /// [`NodeHandle::focus_into`]: crate::dom::NodeHandle::focus_into
    /// [`NodeHandle::restore_focus`]: crate::dom::NodeHandle::restore_focus
    fn take_after_borrow(&mut self) -> Option<Box<dyn FnOnce()>> {
        None
    }

    /// Work to run **before** a mutation takes `node` out of place, with no
    /// borrow of this document held, or `None` (issue #1478).
    ///
    /// `children_only` is `false` for a mutation that removes or moves `node`
    /// itself with its subtree (`remove_node`, `discard_node`, `remove_child`,
    /// `replace_node` for both nodes, and `append_child` / `insert_before` /
    /// `insert_child` for a child that is already somewhere), and `true` for
    /// one that replaces only what is under it (`set_inner_html`,
    /// `set_text_content` on an element).
    ///
    /// `rinch-web` answers `Some` when the keyboard focus is inside the part
    /// that goes, and the work is a `blur()` of the focused element. Chrome
    /// fires `focusout`, `blur` and (for a modified field) `change`
    /// synchronously inside the DOM call that removes or moves a focused node,
    /// and rinch's own listeners touch the document, so made under the
    /// caller's `borrow_mut` that call panicked inside the listener with
    /// "RefCell already borrowed": the key entry of an open `Select` never
    /// heard its trigger lose the focus. Released first, the same listeners
    /// run against a free document in which the node is still where it was,
    /// and a node that holds no focus leaves with no event. Firefox fires
    /// nothing for a removed node; the `blur()` makes the two agree.
    ///
    /// Every [`NodeHandle`](crate::dom::NodeHandle) verb named above asks this,
    /// runs the answer, and asks again (a listener may put the focus back)
    /// before it borrows to mutate. It does not ask for a mutation it can see
    /// the backend will refuse (an insertion whose reference is not a child of
    /// the parent, or into the node's own subtree), which moves nothing.
    ///
    /// `last_round` is the last time the verb will ask. The work must then
    /// leave no focus in the part that goes **and no way for a listener to put
    /// one back**: `rinch-web` sets `inert` on `node` for the length of that
    /// blur (a `focus()` into an inert subtree is refused) and takes it off
    /// again before the mutation. Only a listener that refocuses in there on
    /// every earlier round reaches it; a `MutationObserver` or an `[inert]`
    /// selector of the app's can see that one toggle. **A direct caller of one of those
    /// mutations does not get it**: it must ask before it borrows, or its
    /// mutation runs the browser's listeners under its borrow as before
    /// ([`UpdateBatch::apply`]'s `SetText` is one, since it is handed the
    /// document already borrowed).
    ///
    /// Defaulted to `None`: desktop's focus is the runtime's own state, and a
    /// node leaving the tree dispatches nothing.
    ///
    /// [`UpdateBatch::apply`]: crate::dom::UpdateBatch::apply
    fn release_focus_within(
        &self,
        _node: NodeId,
        _children_only: bool,
        _last_round: bool,
    ) -> Option<Box<dyn FnOnce()>> {
        None
    }

    /// Whether inserting `child` under `parent` is a **move the backend makes
    /// in place**: the child and everything in it keep their state, the
    /// keyboard focus included, and no focus event is dispatched (issue #1483).
    ///
    /// The insertion verbs of [`NodeHandle`](crate::dom::NodeHandle)
    /// (`append_child`, `insert_before`, `insert_after`) ask this once
    /// [`release_focus_within`](Self::release_focus_within) has answered that
    /// there is a focus to release (so not at all while nothing in the moved
    /// part is focused), and do not run that release when it answers `true`:
    /// the move leaves the focus where it is.
    ///
    /// `rinch-web` answers `true` when the browser has `Node.moveBefore` (the
    /// real one: a page's polyfill over `insertBefore` is told apart and not
    /// used) and both nodes are connected, in one document; its insertion methods then
    /// make that call where they made `insertBefore` / `appendChild`, which is
    /// a removal and an insertion and drops the focus (measured in Chrome 153:
    /// `moveBefore` of a row holding the focused `<input>` leaves
    /// `activeElement` on it and fires neither `blur` nor `focusout`). A keyed
    /// `for` reorder is that move. Everything else answers `false` and takes
    /// the release: a browser without the method, a node that is in no
    /// document yet, a move into or out of a detached subtree. **The answer
    /// and the method must agree**: a backend that answers `true` and then
    /// moves the node the old way runs the browser's focus listeners under the
    /// caller's borrow.
    ///
    /// Defaulted to `false`, which costs nothing where
    /// `release_focus_within` is `None` anyway: desktop's focus is the
    /// runtime's own state, keyed by node, and a move never touched it. So the
    /// two backends agree wherever the browser has the method.
    fn moves_keeping_focus(&self, _parent: NodeId, _child: NodeId) -> bool {
        false
    }

    /// Work to run **before** writing (`Some(value)`) or removing (`None`) the
    /// attribute `name` on `node`, with no borrow of this document held, or
    /// `None` (issue #1478). Asked only for a name
    /// [`is_focus_revoking_attribute`](crate::dom::is_focus_revoking_attribute)
    /// accepts.
    ///
    /// Chrome blurs the focused element synchronously, inside the attribute
    /// write, when the write leaves it unfocusable: `hidden` set on it, and
    /// `tabindex` or `contenteditable` removed from an element that was
    /// focusable only through it (measured in Chrome 153; `disabled`, `inert`,
    /// `display: none`, and any of them on an *ancestor*, fire nothing inside
    /// the write). Only the browser knows whether a given write is such a one
    /// (`tabindex="0"` over `-1` on the focused item of a roving list is not,
    /// and must keep the focus), so `rinch-web`'s answer for a write to the
    /// focused element is **the browser write itself**: made here, outside the
    /// borrow, it fires whatever the browser fires against a free document,
    /// and the write the caller then makes under its borrow repeats it and
    /// changes nothing.
    ///
    /// `NodeHandle::set_attribute`, `write_attribute` and `remove_attribute`
    /// ask this. A direct caller of
    /// [`set_attribute`](DomContent::set_attribute) / [`remove_attribute`](DomContent::remove_attribute)
    /// does not get it.
    ///
    /// Defaulted to `None`.
    fn before_attribute_write(
        &self,
        _node: NodeId,
        _name: &str,
        _value: Option<&str>,
    ) -> Option<Box<dyn FnOnce()>> {
        None
    }
}
