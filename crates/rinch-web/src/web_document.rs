//! Browser-native DOM implementation of the `DomDocument` trait.
//!
//! Instead of painting to a canvas, this implementation creates real browser DOM
//! elements via `web_sys`. The browser handles layout, CSS, text rendering, and
//! painting natively. The reactive system (Signal/Effect) and all components work
//! through NodeHandle -> DomDocument, so everything works automatically.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::event_delegation::{
    FOCUS_VALUE_PROP, get_expando_string, set_expando, utf16_offset_to_utf8_bytes,
};
use rinch_core::dom::{CaretAffinity, DomDocument, GlyphBounds, NodeId, attr_is_truthy};
use rinch_editable::RewriteDiff;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Process-global NodeId counter. Shared across all `WebDocument`s so that
/// multiple coexisting roots (islands) never reuse a NodeId — keeping each
/// node's `__nid` unambiguous even when several documents share the page.
static NEXT_NODE_ID: AtomicU64 = AtomicU64::new(0);

/// A `DomDocument` backed by real browser DOM elements.
///
/// Each rinch `NodeId` maps to a `web_sys::Node` stored in a HashMap.
/// Reverse lookups (browser node -> NodeId) use a `__nid` JS property
/// set on each node via `Reflect::set`.
pub struct WebDocument {
    /// Process-unique document identity (see [`DomDocument::doc_key`]).
    doc_key: u64,
    /// The browser's document object.
    browser_doc: web_sys::Document,
    /// Map from rinch NodeId to browser DOM node.
    nodes: HashMap<u64, web_sys::Node>,
    /// The root wrapper element (`<div id="rinch-root">`), or the adopted host
    /// element for an island mount (see [`WebDocument::new_into`]).
    root_id: NodeId,
    /// The body wrapper element (`<div id="rinch-body">`).
    body_id: NodeId,
    /// Browser focus work queued by the focus methods, handed to the caller by
    /// [`DomDocument::take_after_borrow`] to run once its borrow is released:
    /// a browser `focus()`/`blur()` runs focus listeners synchronously, and
    /// rinch's own listeners touch this document.
    after_borrow: Vec<Box<dyn FnOnce()>>,
}

thread_local! {
    /// Reverse map (`NodeId.0` → browser node) populated for every node `set_nid`
    /// tags. Lets non-`DomDocument` code — the editor input glue — resolve a rinch
    /// node id back to its DOM node for caret/selection geometry.
    ///
    /// The value is a **strong** `web_sys::Node`, so an entry pins its browser
    /// node against GC. Node *creation* is not structural — a keyed `for`, a
    /// `show`, or the editor's per-keystroke `ViewDesc` churn creates and destroys
    /// nodes forever within one mounted root — so without pruning this map grows
    /// without bound for the life of the page.
    ///
    /// # Where it is pruned, and where it deliberately is not (issues #184, #719)
    ///
    /// Entries leave by [`forget_subtree`]/[`forget_children`] from
    /// [`DomDocument::discard_node`] and `set_inner_html`, and by
    /// [`Drop for WebDocument`](WebDocument) for whatever a dying document still
    /// holds.
    ///
    /// **`remove_node` and `replace_node` are not on that list**, and used to be.
    /// They are *detaches*: the node keeps both entries so a `NodeHandle` can put
    /// it back, which is the post-condition desktop has always had and the one a
    /// reactive branch re-showing a *captured* handle rests on (issue #719).
    /// Pruning there is what made that silently lose the subtree on this backend
    /// alone. So the bound on this map is no longer "everything removed" but
    /// "everything a caller has not said it is finished with" — and every
    /// unbounded churn source in the workspace says so: `for_each_dom_typed`,
    /// `virtual_list`, `reactive_component_dom` and the editor's `ViewDesc` diff
    /// all call `discard_node`. `crates/rinch-web/tests/node_registry_pruning.rs`
    /// is the pin, `churning_a_for_loop_does_not_grow_the_registry` the
    /// end-to-end one.
    ///
    /// `set_text_content` on an element is the other non-pruning site, for a
    /// different reason: it discards its children too, but the browser replaces
    /// them with an untagged text node, so only the *first* call on a given
    /// element can strand anything — at most one entry per element, which is
    /// bounded by the DOM rather than by churn. Pruning there would retire nodes
    /// in the reactive-text hot path and needs its own caller audit.
    ///
    /// [`node_by_nid`] additionally returns `None` for a node that is present but
    /// detached, so its callers cannot tell a pruned id from a detached one.
    static NODE_REGISTRY: std::cell::RefCell<HashMap<u64, web_sys::Node>> =
        std::cell::RefCell::new(HashMap::new());

    /// The page's scroll lock (#474): how many overlays hold one, and what
    /// `<html>`'s inline `overflow` was before the first of them took it.
    ///
    /// **Page-global, not per-`WebDocument`.** There is one `<html>` however many
    /// island roots share the page, so a per-document counter would let the
    /// second island's modal restore an `overflow` the first island's modal is
    /// still relying on. Keyed by nothing for the same reason: the lock is a
    /// property of the page.
    static SCROLL_LOCK: std::cell::RefCell<ScrollLock> =
        const { std::cell::RefCell::new(ScrollLock { held: 0, saved: None, roots: Vec::new() }) };
}

/// The inline declarations the page scroll lock overwrites, in the order they
/// are saved and restored.
///
/// **The longhands, not just the shorthand.** `overflow: hidden` sets both, and
/// `remove_property("overflow")` removes both — so saving only the shorthand
/// loses an inline `overflow-x` that had no matching `overflow-y`: the shorthand
/// serializes to `""` for that element, the restore takes the "there was
/// nothing" path, and a declaration the lock never wrote is destroyed. Saving
/// all three and restoring all three is exact in every case, including the
/// ordinary one where only the shorthand was set (it round-trips through the two
/// longhands unchanged).
const OVERFLOW_PROPS: [&str; 3] = ["overflow", "overflow-x", "overflow-y"];

/// See [`SCROLL_LOCK`].
struct ScrollLock {
    /// How many overlays currently hold the lock. The page is locked while this
    /// is non-zero, so an inner modal closing over an outer one changes nothing.
    held: u32,
    /// `<html>`'s inline [`OVERFLOW_PROPS`] as they were when `held` went
    /// 0 → 1, restored when it goes back to 0. An empty entry means "there was
    /// no inline declaration", which restores by *removing* the property rather
    /// than setting it to the empty string — the two differ to `style.cssText`,
    /// and a page whose `overflow` comes from a stylesheet must get that value
    /// back, not an empty inline override.
    saved: Option<[String; 3]>,
    /// The overlays holding the lock, by node id with their element: what a
    /// drag's auto-scroll may still scroll while the page is locked
    /// ([`scroll_locked_out`]). The browser's own gestures are not gated by
    /// them; only rinch's programmatic scroll asks.
    roots: Vec<(u64, web_sys::Element)>,
}

/// `<html>` as an `HtmlElement`, the element the page's scroll lock acts on.
///
/// Deliberately **not** `WebDocument::body()`: that is `<div id="rinch-body">`
/// (or the island host), a descendant of the real `<body>`, and `overflow:
/// hidden` there does not stop the page scrolling.
fn document_element_style() -> Option<web_sys::CssStyleDeclaration> {
    let doc = web_sys::window()?.document()?;
    let html: web_sys::HtmlElement = doc.document_element()?.dyn_into().ok()?;
    Some(html.style())
}

/// Web's half of `lock_scroll` (#474), page-global and counted.
///
/// rinch cannot gate the browser's own wheel, so the only mechanism available is
/// `overflow: hidden` on `<html>` — which is also what every web overlay library
/// does. That is the documented divergence from desktop: this removes the page's
/// scrollbar as well as its scrolling, so a page with a classic (non-overlay)
/// scrollbar shifts horizontally when an overlay opens. Desktop's scrollbars are
/// overlays with no gutter and it gates the gesture instead, so it does not.
fn set_page_scroll_locked(locked: bool, root: u64, root_el: Option<web_sys::Element>) {
    SCROLL_LOCK.with(|cell| {
        let mut lock = cell.borrow_mut();
        if locked {
            lock.roots.extend(root_el.map(|el| (root, el)));
            lock.held += 1;
            if lock.held == 1
                && let Some(style) = document_element_style()
            {
                lock.saved =
                    Some(OVERFLOW_PROPS.map(|p| style.get_property_value(p).unwrap_or_default()));
                style.set_property("overflow", "hidden").ok();
            }
        } else {
            // An unlock with nothing held is ignored, not underflowed: an
            // overlay's `on_cleanup` releases a lock its own effect may already
            // have released.
            lock.held = lock.held.saturating_sub(1);
            if let Some(i) = lock.roots.iter().rposition(|(n, _)| *n == root) {
                lock.roots.remove(i);
            }
            if lock.held == 0 {
                lock.roots.clear();
            }
            if lock.held == 0
                && let Some(style) = document_element_style()
            {
                let saved = lock.saved.take();
                // Clear the shorthand first, which clears both longhands with
                // it, then put back only what was actually there. Order matters:
                // restoring a longhand and *then* removing the shorthand would
                // remove the longhand again.
                style.remove_property("overflow").ok();
                if let Some(saved) = saved {
                    for (prop, prev) in OVERFLOW_PROPS.iter().zip(saved.iter()) {
                        if !prev.is_empty() {
                            style.set_property(prop, prev).ok();
                        }
                    }
                }
            }
        }
    });
    // A drag whose pointer rests over a list the lock had kept still scrolls
    // it now, as desktop's next frame does, not only after a pointer move.
    if !locked {
        crate::event_delegation::restart_drag_auto_scroll_if_dragging();
    }
}

/// Reset the page scroll lock. Test-only: a fixture that fails between a lock
/// and its unlock would otherwise leave `<html>` hidden for every test after it.
#[doc(hidden)]
pub fn __reset_scroll_lock() {
    SCROLL_LOCK.with(|cell| {
        *cell.borrow_mut() = ScrollLock {
            held: 0,
            saved: None,
            roots: Vec::new(),
        };
    });
    if let Some(style) = document_element_style() {
        for prop in OVERFLOW_PROPS {
            style.remove_property(prop).ok();
        }
    }
}

/// Whether rinch's own programmatic scrolling (a drag's auto-scroll) must leave
/// `el` alone while the page is locked: desktop's `NodeTree::scroll_locked_out`
/// rule. Nothing is locked out with no lock held. While one is, a container
/// inside a locking overlay's root, or inside (or carrying) an element marked
/// `data-scroll-lock-exempt` that holds no locking root, may scroll; anything
/// else — the page's scrolling element included — is locked out.
pub(crate) fn scroll_locked_out(el: &web_sys::Element) -> bool {
    SCROLL_LOCK.with(|cell| {
        let lock = cell.borrow();
        if lock.held == 0 {
            return false;
        }
        let mut current = Some(el.clone());
        while let Some(node) = current {
            if lock
                .roots
                .iter()
                .any(|(_, root)| root.is_same_node(Some(&node)))
            {
                return false;
            }
            if node
                .get_attribute(rinch_core::events::SCROLL_LOCK_EXEMPT_ATTRIBUTE)
                .is_some_and(|v| rinch_core::dom::data_attr_is_on(&v))
                && !lock
                    .roots
                    .iter()
                    .any(|(_, root)| !root.is_same_node(Some(&node)) && node.contains(Some(root)))
            {
                return false;
            }
            current = node.parent_element();
        }
        true
    })
}

/// How many overlays currently hold the page scroll lock. Test-only.
#[doc(hidden)]
pub fn __scroll_lock_depth() -> u32 {
    SCROLL_LOCK.with(|cell| cell.borrow().held)
}

/// Resolve a rinch `NodeId.0` to its live browser node — the editor glue uses this to
/// read caret/selection geometry for a model position. `None` if unknown or detached.
pub(crate) fn node_by_nid(nid: u64) -> Option<web_sys::Node> {
    NODE_REGISTRY
        .with(|m| m.borrow().get(&nid).cloned())
        .filter(|n| n.is_connected())
}

/// The browser node rinch registered under `nid`, attached or not.
pub(crate) fn registered_node(nid: u64) -> Option<web_sys::Node> {
    NODE_REGISTRY.with(|m| m.borrow().get(&nid).cloned())
}

/// The browser element behind `handle`.
///
/// Attached or not: a node view is mounted on its host while the editor's
/// container is still being built, before it joins the page.
pub(crate) fn element_of(handle: &rinch_core::dom::NodeHandle) -> Option<web_sys::Element> {
    NODE_REGISTRY
        .with(|m| m.borrow().get(&handle.node_id().0).cloned())?
        .dyn_into::<web_sys::Element>()
        .ok()
}

/// Set the `__nid` JS property on a browser node for reverse lookups.
fn set_nid(node: &web_sys::Node, id: NodeId) {
    let _ = js_sys::Reflect::set(node, &"__nid".into(), &JsValue::from(id.0 as u32));
    NODE_REGISTRY.with(|m| m.borrow_mut().insert(id.0, node.clone()));
}

/// Drop `node` and every `__nid`-tagged descendant of it from both node maps —
/// the document's own `nodes` and the page-global [`NODE_REGISTRY`] (#184).
///
/// Reached only from [`DomDocument::discard_node`] and `set_inner_html`, the two
/// routes on which the caller has said it is finished with the subtree (#719).
///
/// Retiring an id is safe because `NEXT_NODE_ID` is a monotonic `fetch_add` with
/// no free list: a retired id can never be re-issued to a different node, so this
/// backend cannot hit the recycled-slot identity hazard the slab-based desktop
/// backend has to worry about. The worst case is a stale `NodeHandle` naming an
/// absent id, and every accessor here is `get`-guarded, so that is a silent no-op.
///
/// The walk goes over the **browser** DOM: `WebDocument` keeps a flat map and no
/// parent/child bookkeeping of its own, so the browser is the only structural
/// source of truth. This is the exact inverse of
/// [`WebDocument::register_subtree`].
fn forget_subtree(nodes: &mut HashMap<u64, web_sys::Node>, node: &web_sys::Node) {
    // One borrow for the whole recursion. Nothing inside it may call `set_nid`,
    // which borrows the registry mutably.
    NODE_REGISTRY.with(|m| {
        let mut reg = m.borrow_mut();
        forget_recursive(nodes, &mut reg, node);
    });
}

/// [`forget_subtree`] for every child of `parent`, under a **single** registry
/// borrow — what `set_inner_html` needs, since it discards all of them at once.
fn forget_children(nodes: &mut HashMap<u64, web_sys::Node>, parent: &web_sys::Node) {
    NODE_REGISTRY.with(|m| {
        let mut reg = m.borrow_mut();
        let mut child = parent.first_child();
        while let Some(c) = child {
            let next = c.next_sibling();
            forget_recursive(nodes, &mut reg, &c);
            child = next;
        }
    });
}

fn forget_recursive(
    nodes: &mut HashMap<u64, web_sys::Node>,
    reg: &mut HashMap<u64, web_sys::Node>,
    node: &web_sys::Node,
) {
    // `first_child`/`next_sibling`, not `child_nodes()[i]`: this now runs on the
    // per-keystroke `replace_node` path and on every list removal, and indexing a
    // live `NodeList` is the classic quadratic walk.
    let mut child = node.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        forget_recursive(nodes, reg, &c);
        child = next;
    }
    // An `<img>` the app's image sources recorded leaves that record with it.
    crate::images::forget_node(node);
    if let Some(id) = get_nid(node) {
        // `nodes` is this document's map but the registry is page-global, so a
        // descendant belonging to a *different* `WebDocument` (an island mounted
        // with `new_into` inside this document's tree) loses its registry entry
        // here while staying in that document's own map. The node is genuinely
        // being destroyed either way, so this is safe; it just means the island's
        // half of the leak is that document's `Drop` to reclaim, not ours.
        nodes.remove(&id.0);
        reg.remove(&id.0);
    }
}

/// **Test-only.** Live entries in the page-global node registry (#184).
///
/// The counter behind `NodeId` is process-global, so a test must compare this
/// against its own baseline — never against an absolute number.
#[doc(hidden)]
pub fn __node_registry_len() -> usize {
    NODE_REGISTRY.with(|m| m.borrow().len())
}

/// **Test-only.** Whether the page-global node registry still holds `nid` (#184).
///
/// Raw membership, unlike [`node_by_nid`], which additionally filters on
/// `is_connected()` — a leak test has to tell "pruned" from "merely detached".
#[doc(hidden)]
pub fn __node_registry_contains(nid: u64) -> bool {
    NODE_REGISTRY.with(|m| m.borrow().contains_key(&nid))
}

/// Coerces a scroll offset to whatever `Element::set_scroll_{top,left}` expects.
///
/// web-sys types these setters as `i32` on stable, but as `f64` under
/// `--cfg=web_sys_unstable_apis` (which an OPFS storage backend needs for
/// `FileSystemSyncAccessHandle`). Converting here keeps the call sites clean and
/// preserves sub-pixel scroll offsets on the unstable path.
#[cfg(web_sys_unstable_apis)]
fn scroll_px(v: f64) -> f64 {
    v
}

#[cfg(not(web_sys_unstable_apis))]
fn scroll_px(v: f64) -> i32 {
    v as i32
}

/// Returns true if the tag name is an SVG element.
///
/// SVG elements must be created with `createElementNS` using the SVG namespace,
/// otherwise the browser treats them as unknown HTML elements and won't render them.
fn is_svg_tag(tag: &str) -> bool {
    matches!(
        tag,
        "svg"
            | "path"
            | "circle"
            | "ellipse"
            | "line"
            | "polyline"
            | "polygon"
            | "rect"
            | "g"
            | "defs"
            | "use"
            | "text"
            | "tspan"
            | "clipPath"
            | "mask"
            | "pattern"
            | "image"
            | "foreignObject"
            | "animate"
            | "animateTransform"
            | "set"
            | "stop"
            | "linearGradient"
            | "radialGradient"
            | "filter"
            | "feGaussianBlur"
            | "feOffset"
            | "feMerge"
            | "feMergeNode"
            | "feFlood"
            | "feComposite"
            | "feBlend"
            | "symbol"
            | "marker"
            | "title"
            | "desc"
    )
}

/// Mirror an attribute onto the live DOM *property* for the form-control
/// attributes that browsers reflect asymmetrically (`value`, `checked`,
/// `selected`, `indeterminate`).
///
/// For `<input>`/`<textarea>`/`<select>`/`<option>` the attribute only seeds the
/// control's *default* value. Once the user edits the control it becomes "dirty"
/// and the browser no longer mirrors the attribute into the property, so a
/// reactive `value:`/`checked:` binding that writes the attribute would silently
/// stop updating the displayed value. Writing the property keeps the control in
/// sync with the signal even after it has been typed into / toggled (issue #100).
///
/// The property is only written when it actually differs from the requested
/// value. For `value` that avoids resetting the caret to the end on the common
/// echo path (user types → `oninput` → signal → effect re-writes the same
/// string); for the boolean properties it just skips redundant DOM writes.
/// The exception is a `<textarea>`, whose equal write goes through `set_value`
/// anyway: that sets its dirty value flag (it stops following its text
/// children) without moving the cursor (#1222).
///
/// A `value` write to the control that holds focus is the user's live text
/// being rewritten under them (issue #238): the selection is mapped through
/// the rewrite and restored (see [`write_live_text`]) rather than left where
/// `set_value` puts it — at the end — and a write during an IME composition is
/// deferred to `compositionend`.
fn sync_reflected_property(node: &web_sys::Node, name: &str, value: &str) {
    match name {
        "value" => {
            // Each control type exposes value()/set_value(); write only when it
            // differs (the `!= value` guard avoids a caret reset on the echo path).
            if let Some(input) = node.dyn_ref::<web_sys::HtmlInputElement>() {
                // A file input's value setter throws `InvalidStateError` for any
                // non-empty string (the browser forbids setting a file path
                // programmatically), and web-sys' `set_value` is not a `catch`
                // binding — the exception would unwind uncaught across the wasm
                // boundary. Only `""` is permitted (it clears the selection), so
                // skip any other value on a file input.
                let settable = input.type_() != "file" || value.is_empty();
                if settable && input.value() != value {
                    write_live_text(&TextControl::Input(input), value);
                }
            } else if let Some(textarea) = node.dyn_ref::<web_sys::HtmlTextAreaElement>() {
                if textarea.value() != value {
                    write_live_text(&TextControl::TextArea(textarea), value);
                } else {
                    // An equal write is still a write: it sets the dirty value
                    // flag, as a script `.value` write does in Chrome 153, so
                    // the field stops following its text children — as on
                    // desktop, where the flag is the attribute's presence
                    // (#1222). The setter moves the cursor only when the value
                    // changes, so the echo path keeps its caret.
                    textarea.set_value(value);
                }
            } else if let Some(select) = node.dyn_ref::<web_sys::HtmlSelectElement>()
                && select.value() != value
            {
                select.set_value(value);
            }
        }
        // `indeterminate` is the one member of the reflected family with **no**
        // content attribute at all — it is a property-only IDL flag, HTML has no
        // `indeterminate` attribute, and `is_boolean_attribute` does not list
        // it. So it has no presence for anyone to read and the string is all
        // there is: truthiness is the only mapping available, and it stays.
        // `checked` and `selected` are real attributes and go through
        // [`sync_presence_property`] instead (issue #622).
        "indeterminate" => {
            if let Some(input) = node.dyn_ref::<web_sys::HtmlInputElement>() {
                let on = attr_is_truthy(value);
                if input.indeterminate() != on {
                    input.set_indeterminate(on);
                }
            }
        }
        _ => {}
    }
}

/// Mirror a **presence** boolean attribute (`checked`, `selected`, `muted`)
/// onto the live IDL property it reflects.
///
/// The content attribute is only the control's *default*: the browser sets a
/// dirty checkedness / selectedness flag on the first user toggle and stops
/// mirroring the attribute into the property from then on. rinch has no such
/// flag: desktop's `:checked` (`stylo_impl.rs`) reads the attribute and nothing
/// else, and an `<option>`'s selectedness (`select.rs`) is *seeded* from the
/// attribute and then moved only by rinch's own writes of it (#692) — never by
/// a user's pick, which the desktop popup records on the `<select>`'s own
/// `value`. So the app's write has to win on both backends, which is why the
/// property is written here at all (issue #100).
///
/// What it is told is the attribute's **presence**, never its string. A present
/// `checked` checks the box whatever it holds, `"false"` included — the
/// browser's own rule for raw markup, measured in
/// `tests/boolean_attributes.rs::html_reads_a_present_boolean_attribute_as_true_whatever_its_value`
/// — and absence is the only off state (issue #622). The truthiness mapping that
/// used to sit here belongs to the *writer*,
/// [`rinch_core::dom::NodeHandle::write_attribute`], which removes the attribute
/// for a falsey value before the backend sees it (issue #551).
///
/// `muted` (issue #754) is NOT a browser-reflected property — HTML gives the
/// `muted` *content* attribute no dynamic effect at all; it only seeds
/// `defaultMuted` once, at the media element's load algorithm. rinch mirrors it
/// dynamically anyway, deliberately more dynamic than HTML, because a
/// reactive `muted: {|| m.get()}` binding is otherwise unmutable after the
/// first load — the same shape #100 fixed for `checked`/`selected`, chosen as
/// option 1 of the issue's three.
fn sync_presence_property(node: &web_sys::Node, name: &str, present: bool) {
    match name {
        "checked" => {
            if let Some(input) = node.dyn_ref::<web_sys::HtmlInputElement>()
                && input.checked() != present
            {
                input.set_checked(present);
            }
        }
        "selected" => {
            if let Some(option) = node.dyn_ref::<web_sys::HtmlOptionElement>()
                && option.selected() != present
            {
                option.set_selected(present);
            }
        }
        "muted" => {
            if let Some(media) = node.dyn_ref::<web_sys::HtmlMediaElement>()
                && media.muted() != present
            {
                media.set_muted(present);
            }
        }
        _ => {}
    }
}

/// JS expando flag a form control carries for the duration of an IME
/// composition — set at `compositionstart`, cleared at `compositionend` by the
/// document-level listeners in `event_delegation` (issue #238). A programmatic
/// value write while it is set is deferred: setting `.value` mid-composition
/// would move the caret under the composition and corrupt it.
pub(crate) const COMPOSING_PROP: &str = "__rinch_composing";

/// JS expando holding the value write a composition deferred, applied by
/// [`flush_deferred_value`] at `compositionend`. The latest write wins.
pub(crate) const PENDING_VALUE_PROP: &str = "__rinch_pending_value";

/// JS expando holding the control's live `.value` at the moment a write was
/// deferred. If the value has moved on by `compositionend` the composition
/// committed text under the parked write, which is then dropped rather than
/// pasted over it (issue #238).
pub(crate) const PENDING_BASE_PROP: &str = "__rinch_pending_base";

/// The two text controls with a selection API, for [`write_live_text`].
enum TextControl<'a> {
    Input(&'a web_sys::HtmlInputElement),
    TextArea(&'a web_sys::HtmlTextAreaElement),
}

impl TextControl<'_> {
    fn element(&self) -> &web_sys::Element {
        match self {
            Self::Input(i) => i,
            Self::TextArea(t) => t,
        }
    }

    fn value(&self) -> String {
        match self {
            Self::Input(i) => i.value(),
            Self::TextArea(t) => t.value(),
        }
    }

    fn set_value(&self, value: &str) {
        match self {
            Self::Input(i) => i.set_value(value),
            Self::TextArea(t) => t.set_value(value),
        }
    }

    /// Whether `selectionStart`/`setSelectionRange` apply: every textarea, and
    /// the text-like input types. On the others (`number`, `email`, `color`,
    /// `date`, ...) `selectionStart` is `null` and `setSelectionRange` throws
    /// `InvalidStateError` — the same guard shape as the `file` check above.
    fn has_selection_api(&self) -> bool {
        match self {
            Self::Input(i) => matches!(
                i.type_().as_str(),
                "text" | "search" | "url" | "tel" | "password"
            ),
            Self::TextArea(_) => true,
        }
    }

    /// `(selectionStart, selectionEnd, selectionDirection)` in UTF-16 units.
    fn selection(&self) -> Option<(u32, u32, String)> {
        let (start, end, direction) = match self {
            Self::Input(i) => (
                i.selection_start(),
                i.selection_end(),
                i.selection_direction(),
            ),
            Self::TextArea(t) => (
                t.selection_start(),
                t.selection_end(),
                t.selection_direction(),
            ),
        };
        let direction = direction
            .ok()
            .flatten()
            .unwrap_or_else(|| "none".to_string());
        Some((start.ok()??, end.ok()??, direction))
    }

    fn set_selection(&self, start: u32, end: u32, direction: &str) {
        let _ = match self {
            Self::Input(i) => i.set_selection_range_with_direction(start, end, direction),
            Self::TextArea(t) => t.set_selection_range_with_direction(start, end, direction),
        };
    }
}

/// The `value` attribute write (issue #238): the live property first — via
/// [`sync_reflected_property`], which carries a focused control's selection
/// through the rewrite — then the content attribute, which is inert once the
/// property write has dirtied the control. While the control is in an IME
/// composition the whole write is held in `__rinch_pending_value` (the latest
/// wins) and applied by [`flush_deferred_value`] at `compositionend`; writing
/// even the content attribute mid-composition would mirror into a pristine
/// control's `.value` and move the caret under the composition.
///
/// A write that merely echoes what the control already holds is *not* stashed:
/// the composition's own `input` events drive `data-oninput`, so a controlled
/// field re-writes its own text on every keystroke of the composition, and
/// stashing that would hand `compositionend` a pre-commit snapshot to paste
/// over the text the composition just committed. The live value at stash time
/// is recorded next to the pending write for the same reason — see
/// [`flush_deferred_value`].
pub(crate) fn write_value_attribute(el: &web_sys::Element, value: &str) {
    write_value(el, Some(value));
}

/// Remove the `value` content attribute — a write of `""` to the live property,
/// then the removal, and deferred during a composition exactly like a write
/// (issue #238): on a pristine control the *absence* of the attribute mirrors
/// into `.value` just as its presence does, so removing it mid-composition
/// moves the caret under the composition too.
pub(crate) fn remove_value_attribute(el: &web_sys::Element) {
    write_value(el, None);
}

/// The shared body: `Some(v)` sets the attribute, `None` removes it.
fn write_value(el: &web_sys::Element, value: Option<&str>) {
    if is_composing(el) {
        // An echo changes nothing and must not displace the user's composition.
        let live = live_text_value(el);
        if live.as_deref() != Some(value.unwrap_or("")) {
            set_expando(
                el,
                PENDING_VALUE_PROP,
                &value.map_or(JsValue::NULL, JsValue::from_str),
            );
            set_expando(
                el,
                PENDING_BASE_PROP,
                &live.map_or(JsValue::UNDEFINED, |v| JsValue::from_str(&v)),
            );
        }
        return;
    }
    let node: &web_sys::Node = el;
    sync_reflected_property(node, "value", value.unwrap_or(""));
    match value {
        Some(v) => {
            el.set_attribute("value", v).ok();
        }
        None => {
            el.remove_attribute("value").ok();
        }
    }
}

/// Write `value` into a text control's live `.value` (issue #238).
///
/// When the control holds focus this is the user's text being rewritten under
/// them — a normalizing `oninput`, a `value_fn` echoing a filtered signal — so
/// the selection is read first and restored afterwards, mapped through the
/// rewrite (see [`RewriteDiff`], the desktop engine's own rule) instead of
/// collapsing to the
/// end as a bare `set_value` does. The `data-onchange` Enter-commit baseline
/// follows the write while the field is untouched, so a purely programmatic
/// change never commits by itself (issue #226).
fn write_live_text(control: &TextControl<'_>, value: &str) {
    let el = control.element();
    let focused = is_active_element(el);
    let old = control.value();
    let selection = if focused && control.has_selection_api() {
        control.selection()
    } else {
        None
    };
    let baseline_follows =
        focused && get_expando_string(el, FOCUS_VALUE_PROP).as_deref() == Some(old.as_str());
    control.set_value(value);
    if !baseline_follows && selection.is_none() {
        return;
    }
    // Map against what the control actually STORES, not what was requested: the
    // browser sanitizes on the way in (every text-ish `<input>` strips newlines,
    // `url` also trims surrounding whitespace, `<textarea>` normalizes CRLF), so
    // offsets computed against the requested string can overshoot the stored one
    // — and a baseline recorded from it could never match the sanitized value
    // the commit path compares against, committing a purely programmatic change
    // (issue #226). Identical to `value` whenever nothing was sanitized.
    let stored = control.value();
    if baseline_follows {
        set_expando(el, FOCUS_VALUE_PROP, &JsValue::from_str(&stored));
    }
    if let Some((start, end, direction)) = selection {
        // The caret rule is the desktop engine's, so a controlled rewrite moves
        // the selection identically on both backends. `map` can land inside a
        // multi-byte char in a same-length rewrite, so snap down to a boundary —
        // exactly what `EditableState::adopt_text` does after mapping.
        let diff = RewriteDiff::between(&old, &stored);
        let map = |offset16: u32| {
            let byte = utf16_offset_to_utf8_bytes(&old, offset16);
            let mut mapped = diff.map(byte);
            while !stored.is_char_boundary(mapped) {
                mapped -= 1;
            }
            utf8_byte_to_utf16_offset(&stored, mapped) as u32
        };
        control.set_selection(map(start), map(end), &direction);
    }
}

/// Apply the value write a composition deferred on `el`, if any (called at
/// `compositionend`, after the composing flag is cleared).
///
/// The stash is dropped rather than applied when the control's own text moved
/// while it was parked (issue #238): the pending write was computed against the
/// pre-commit text, so pasting it over the text the composition just committed
/// would silently delete the composed characters. A controlled field re-writes
/// itself from the `input` event that accompanies the commit, so the correct
/// value lands anyway.
pub(crate) fn flush_deferred_value(el: &web_sys::Element) {
    // Absent = nothing parked; a string = a parked write; NULL = a parked
    // removal (`as_string()` is `None` for both absent and NULL, so the two are
    // told apart by the raw JsValue).
    let pending = js_sys::Reflect::get(el, &PENDING_VALUE_PROP.into()).ok();
    let Some(pending) = pending.filter(|v| !v.is_undefined()) else {
        return;
    };
    let base = get_expando_string(el, PENDING_BASE_PROP);
    set_expando(el, PENDING_VALUE_PROP, &JsValue::UNDEFINED);
    set_expando(el, PENDING_BASE_PROP, &JsValue::UNDEFINED);
    if base.is_some() && base != live_text_value(el) {
        // The composition changed the text under the parked write; it wins.
        return;
    }
    match pending.as_string() {
        Some(value) => write_value_attribute(el, &value),
        None => remove_value_attribute(el),
    }
}

/// The live `.value` of a text control, or `None` for anything else.
fn live_text_value(el: &web_sys::Element) -> Option<String> {
    if let Some(input) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        Some(input.value())
    } else {
        el.dyn_ref::<web_sys::HtmlTextAreaElement>()
            .map(|t| t.value())
    }
}

fn is_composing(el: &web_sys::Element) -> bool {
    js_sys::Reflect::get(el, &COMPOSING_PROP.into()).is_ok_and(|v| v.is_truthy())
}

fn is_active_element(el: &web_sys::Element) -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .is_some_and(|active| active.is_same_node(Some(el)))
}

// Truthiness for HTML boolean-ish attributes as rinch emits them
// (`""`/`"true"` are on, `"false"`/`"0"` are off) now lives in `rinch-core`,
// beside the boolean-attribute set and `NodeHandle::write_attribute` — the
// writer `rsx!` generates. A second hand-rolled copy here is how the writer and
// this reader come to disagree about `checked=""` (issue #551).

/// The rinch ids of a `querySelectorAll` answer, in its order; elements
/// rinch never registered are skipped.
fn node_list_ids(list: Option<web_sys::NodeList>) -> Vec<NodeId> {
    let Some(list) = list else {
        return Vec::new();
    };
    (0..list.length())
        .filter_map(|i| list.item(i))
        .filter_map(|n| get_nid(&n))
        .collect()
}

/// A `querySelector` answer with the browser's `SyntaxError` turned into
/// `None` and [`rinch_core::dom::warn_invalid_selector`]'s warning, as
/// desktop does (#1796).
fn warn_invalid_selector<T>(selector: &str, r: Result<T, wasm_bindgen::JsValue>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(_) => {
            rinch_core::dom::warn_invalid_selector(selector);
            None
        }
    }
}

/// Get the `__nid` JS property from a browser node.
pub(crate) fn get_nid(node: &web_sys::Node) -> Option<NodeId> {
    js_sys::Reflect::get(node, &"__nid".into())
        .ok()
        .and_then(|v| v.as_f64())
        .map(|n| NodeId(n as u64))
}

/// Marker attribute identifying the one page-global theme `<style>` element.
///
/// Private to the theme path on purpose: a theme update *replaces the text* of
/// the element it matches, so anything else wearing this marker would be
/// silently clobbered by the next dark-mode toggle. App CSS injected through
/// [`WebDocument::inject_style`] is deliberately left unmarked (#155).
const THEME_STYLE_MARKER: &str = "data-rinch-theme";
const THEME_STYLE_SELECTOR: &str = "[data-rinch-theme]";

/// Append a fresh `<style>` to `<head>`, optionally stamped with a marker attribute.
fn append_style_element(doc: &web_sys::Document, css: &str, marker: Option<&str>) {
    let Ok(style) = doc.create_element("style") else {
        return;
    };
    if let Some(marker) = marker {
        style.set_attribute(marker, "true").ok();
    }
    style.set_text_content(Some(css));
    if let Some(head) = doc.head() {
        head.append_child(&style).ok();
    }
}

/// Update the theme `<style>` in place, or create it if this is the first call.
///
/// Updating in place (rather than appending a regenerated sheet) keeps the theme
/// at a stable document position *before* every app stylesheet, so app CSS always
/// cascades over it — the same invariant `RinchDocument::set_theme_css` maintains
/// on desktop.
fn upsert_theme_style(doc: &web_sys::Document, css: &str) {
    if let Ok(Some(el)) = doc.query_selector(THEME_STYLE_SELECTOR) {
        el.set_text_content(Some(css));
    } else {
        append_style_element(doc, css, Some(THEME_STYLE_MARKER));
    }
}

/// Update (or inject) the page-global theme `<style data-rinch-theme>` in the
/// document head, independent of any one `WebDocument`.
///
/// Theme CSS is page-global (one `<style>` shared by every root), so the
/// signal-change hook uses this to keep it current regardless of which root
/// triggered the change. Idempotent: updates the existing element if present.
pub fn update_theme_style_global(css: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    upsert_theme_style(&doc, css);
}

impl WebDocument {
    /// The caret at flat UTF-8 `byte_offset` in `block`, as a viewport
    /// `(x, y, height)`, on `affinity`'s side of a soft wrap (#301).
    ///
    /// The browser draws a collapsed range at a soft-wrap point at the end of
    /// the upper line (measured, Chrome 153, after a hanging space and inside an
    /// `overflow-wrap`-broken word alike): that is the `Upstream` caret. A
    /// `Downstream` one there is drawn at the start of the character after it,
    /// on the lower line. A wrap point is recognised by that character sitting
    /// on a later line than the collapsed range; its start edge is its left for
    /// a left-to-right run and its right for a right-to-left one, told apart by
    /// where a caret just after it draws. A caret the browser gives no rect
    /// (beside a preserved `"\n"`) is drawn where what follows it starts
    /// ([`Self::caret_where_next_starts_at`]). `None` for a block with no text.
    fn text_caret_viewport_rect(
        &self,
        block: &web_sys::Node,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        let (text_node, off) = find_text_node_at_byte_offset(block, byte_offset)?;
        let range = self.browser_doc.create_range().ok()?;
        range.set_start(&text_node, off).ok()?;
        range.set_end(&text_node, off).ok()?;
        let collapsed = range.get_bounding_client_rect();
        if collapsed.height() <= 0.0 {
            // Chrome gives no rect to a collapsed range in preserved-newline
            // text right before a `"\n"` (the empty line of a code block's
            // `ab\n\ncd`, the start of `\nab`) and at the end of a text node
            // ending in one (`ab\n`, #1172). The caret is drawn where what
            // follows it starts (#1202).
            return self.caret_where_next_starts_at(&text_node, off, block);
        }
        // Firefox does give that caret a rect, and for the one right after a
        // preserved `"\n"` it is the newline's own box: the line the newline
        // ends, one above the caret's (measured, Firefox 155 and 157: `ab\n\ncd`
        // in a `<pre>` at offset 3 answers the end of `ab`, and `ab\n` at 3,
        // before the trailing-break placeholder, the same). A hard line break
        // is not a soft wrap, so the caret after it has one line whatever its
        // affinity: where what follows starts, when that is a line further
        // down. Chrome's collapsed range is already there (or has no rect,
        // above), so nothing moves in it.
        if self.follows_a_preserved_newline(&text_node, off)
            && let Some(next) = self.caret_where_next_starts_at(&text_node, off, block)
            && next.1 as f64 >= collapsed.y() + collapsed.height() * 0.5
        {
            return Some(next);
        }
        let upstream = (
            collapsed.x() as f32,
            collapsed.y() as f32,
            collapsed.height() as f32,
        );
        if affinity == CaretAffinity::Upstream {
            return Some(upstream);
        }
        let Some((next_node, start, len)) = find_char_at_byte_offset(block, byte_offset) else {
            return Some(upstream);
        };
        match self.char_start_edge(&next_node, start, len) {
            Some(ch) if ch.1 as f64 >= collapsed.y() + collapsed.height() * 0.5 => Some(ch),
            _ => Some(upstream),
        }
    }

    /// Whether UTF-16 offset `off` of `text_node` is right after a `"\n"` the
    /// text's `white-space` preserves as a line break (`pre`, `pre-wrap`,
    /// `pre-line`, `break-spaces`: every editor block). In collapsing text a
    /// `"\n"` is a space, and the caret after it is an ordinary one.
    fn follows_a_preserved_newline(&self, text_node: &web_sys::Node, off: u32) -> bool {
        if off == 0 {
            return false;
        }
        let text = text_node.text_content().unwrap_or_default();
        if text.encode_utf16().nth(off as usize - 1) != Some(u16::from(b'\n')) {
            return false;
        }
        let Some(parent) = text_node.parent_element() else {
            return false;
        };
        let Some(style) = self
            .browser_doc
            .default_view()
            .and_then(|w| w.get_computed_style(&parent).ok().flatten())
        else {
            return false;
        };
        let ws = style.get_property_value("white-space").unwrap_or_default();
        ws.starts_with("pre") || ws.contains("break-spaces") || ws.contains("preserve")
    }

    /// The start edge of the character at UTF-16 `start..start + len` of
    /// `text_node`, as a viewport `(x, y, height)`: its left for a
    /// left-to-right run and its right for a right-to-left one, told apart by
    /// where a caret just after it draws. `None` when the character has no
    /// box.
    fn char_start_edge(
        &self,
        text_node: &web_sys::Node,
        start: u32,
        len: u32,
    ) -> Option<(f32, f32, f32)> {
        let range = self.browser_doc.create_range().ok()?;
        range.set_start(text_node, start).ok()?;
        range.set_end(text_node, start + len).ok()?;
        // The glyph's own box: the LAST client rect. After a hyphenated
        // soft-hyphen break the character's range also covers the break, so its
        // bounding rect spans both lines and starts on the upper one — which
        // read as "not on a later line" and answered upstream (#1115).
        let ch = match range.get_client_rects() {
            Some(list) if list.length() > 0 => list.item(list.length() - 1)?,
            _ => range.get_bounding_client_rect(),
        };
        if ch.height() <= 0.0 {
            return None;
        }
        // A caret just after the character is at its end edge; the start edge
        // is the other one. (A `"\n"`'s box has no width: either edge.)
        range.collapse_with_to_start(false);
        let after = range.get_bounding_client_rect().x();
        let x = if (after - ch.left()).abs() < (after - ch.right()).abs() {
            ch.right()
        } else {
            ch.left()
        };
        Some((x as f32, ch.y() as f32, ch.height() as f32))
    }

    /// Where whatever follows UTF-16 offset `off` of `text_node` in `block`
    /// starts, as a viewport `(x, y, height)`: the start edge of the
    /// character at `off`, else (at the node's end) the start of the next
    /// text or `<br>` in `block` ([`Self::caret_where_next_starts`]).
    ///
    /// This is the caret the browser gives no rect (#1202). Measured in
    /// Chrome 153, a collapsed range has none right before a preserved
    /// segment break (`"\n"`, `"\r"`), at the end of a text node ending in
    /// `"\n"`, and in an empty text node — though not at every such position
    /// (`pre`'s `a\n \nb` has one after the space, `pre-wrap`'s does not).
    /// The range over the character that follows does have one: for a
    /// newline, a zero-width box at its start, on the line the caret is on.
    fn caret_where_next_starts_at(
        &self,
        text_node: &web_sys::Node,
        off: u32,
        block: &web_sys::Node,
    ) -> Option<(f32, f32, f32)> {
        let text = text_node.text_content().unwrap_or_default();
        let units: Vec<u16> = text.encode_utf16().collect();
        if let Some(&unit) = units.get(off as usize) {
            // A surrogate pair is one character.
            let len = if (0xD800..0xDC00).contains(&unit) {
                2
            } else {
                1
            };
            return self.char_start_edge(text_node, off, len);
        }
        self.caret_where_next_starts(text_node, block)
    }

    /// The caret beside the `<br>` `br` inside `block`, as a viewport
    /// `(x, y, height)`: before it (its own box: the end of its line), or
    /// after it — the start of the line it ends, which is where whatever comes
    /// next in `block` is drawn: the first character of the next text, or the
    /// next `<br>` when that line is empty (#1172).
    ///
    /// A `<br>` has no text bytes in the browser, so the byte offsets on its two
    /// sides are one number, and so are those around two in a row: the byte
    /// query drew the caret after a break at the next *character*, a line too
    /// far down after `a<br><br>b`'s first break, and at the end of line 1
    /// after a trailing one, where nothing follows but the editor's
    /// trailing-break placeholder. The rich-text view names the break instead
    /// ([`DomDocument::query_inline_box_caret_rect`]).
    ///
    /// `None` when `br` is not a `<br>` in `block`, or when what follows it is
    /// neither text nor a `<br>` (an image: the byte query answers that). A
    /// node view's inline host (a claimed image) is answered from its box.
    fn br_caret_viewport_rect(
        &self,
        block_id: u64,
        br_id: u64,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        let block = self.nodes.get(&block_id)?;
        let br = self.nodes.get(&br_id)?;
        let el = br.dyn_ref::<web_sys::Element>()?;
        if !block.contains(Some(br)) {
            return None;
        }
        let rect_of = |r: web_sys::DomRect| {
            (r.height() > 0.0).then(|| (r.x() as f32, r.y() as f32, r.height() as f32))
        };
        // A claimed inline image (a node view's inline-block `<span>`): the caret
        // before it is at its left edge, after it at its right, as tall as its box.
        if is_node_view_host(br) {
            let r = el.get_bounding_client_rect();
            let x = if after { r.right() } else { r.left() };
            return rect_of(r).map(|(_, y, h)| (x as f32, y, h));
        }
        if !el.tag_name().eq_ignore_ascii_case("br") {
            return None;
        }
        if !after {
            return rect_of(el.get_bounding_client_rect());
        }
        self.caret_where_next_starts(br, block)
    }

    /// The caret at the start of whatever comes after `node` in `block` —
    /// the first position of the next text, or the next `<br>`'s box — as a
    /// viewport `(x, y, height)`. `None` when that is neither (an image), or
    /// when nothing follows.
    fn caret_where_next_starts(
        &self,
        node: &web_sys::Node,
        block: &web_sys::Node,
    ) -> Option<(f32, f32, f32)> {
        let rect_of = |r: web_sys::DomRect| {
            (r.height() > 0.0).then(|| (r.x() as f32, r.y() as f32, r.height() as f32))
        };
        let mut next = next_leaf_in(node, block);
        while let Some(node) = next {
            if node.node_type() == web_sys::Node::TEXT_NODE {
                if !node.text_content().unwrap_or_default().is_empty() {
                    let range = self.browser_doc.create_range().ok()?;
                    range.set_start(&node, 0).ok()?;
                    range.set_end(&node, 0).ok()?;
                    // A text beginning with a `"\n"` has no rect at its
                    // start either: that newline's own box (#1202).
                    return rect_of(range.get_bounding_client_rect())
                        .or_else(|| self.caret_where_next_starts_at(&node, 0, block));
                }
            } else if let Some(e) = node.dyn_ref::<web_sys::Element>() {
                if e.tag_name().eq_ignore_ascii_case("br") {
                    return rect_of(e.get_bounding_client_rect());
                }
                return None;
            }
            next = next_leaf_in(&node, block);
        }
        None
    }

    /// Create a new WebDocument backed by the browser's document.
    ///
    /// Creates a `<div id="rinch-root">` as root and `<div id="rinch-body">`
    /// as body, appending root to `document.body()`.
    pub fn new(browser_doc: web_sys::Document) -> Self {
        crate::images::install_reloader();
        watch_focus(&browser_doc);
        let mut doc = Self {
            doc_key: rinch_core::dom::next_doc_key(),
            browser_doc,
            nodes: HashMap::new(),
            root_id: NodeId(0),
            body_id: NodeId(0),
            after_borrow: Vec::new(),
        };

        // Create root element
        let root_el = doc.browser_doc.create_element("div").unwrap();
        root_el.set_id("rinch-root");
        let root_node: web_sys::Node = root_el.into();
        let root_id = doc.alloc_id();
        set_nid(&root_node, root_id);
        doc.nodes.insert(root_id.0, root_node.clone());
        doc.root_id = root_id;

        // Create body element
        let body_el = doc.browser_doc.create_element("div").unwrap();
        body_el.set_id("rinch-body");
        let body_node: web_sys::Node = body_el.into();
        let body_id = doc.alloc_id();
        set_nid(&body_node, body_id);
        doc.nodes.insert(body_id.0, body_node.clone());
        doc.body_id = body_id;

        // Append body to root
        root_node.append_child(&body_node).ok();

        // Append root to the real document.body
        if let Some(real_body) = doc.browser_doc.body() {
            real_body.append_child(&root_node).ok();
        }

        doc
    }

    /// Create a `WebDocument` that mounts into an **existing** browser element
    /// (an "island" host), instead of creating `#rinch-root`/`#rinch-body` and
    /// appending to `document.body`.
    ///
    /// The `host` element is adopted as both the document root and body, so the
    /// component tree is appended directly inside it. No fixed ids are set, so
    /// any number of islands can coexist on one page without id collisions.
    pub fn new_into(browser_doc: web_sys::Document, host: web_sys::Element) -> Self {
        crate::images::install_reloader();
        watch_focus(&browser_doc);
        let mut doc = Self {
            doc_key: rinch_core::dom::next_doc_key(),
            browser_doc,
            nodes: HashMap::new(),
            root_id: NodeId(0),
            body_id: NodeId(0),
            after_borrow: Vec::new(),
        };

        let host_node: web_sys::Node = host.into();
        let id = doc.alloc_id();
        set_nid(&host_node, id);
        doc.nodes.insert(id.0, host_node);
        // The host is both root and body: content mounts directly inside it.
        doc.root_id = id;
        doc.body_id = id;

        doc
    }

    /// Allocate the next (process-globally unique) NodeId.
    fn alloc_id(&mut self) -> NodeId {
        NodeId(NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// Get a reference to the browser document.
    pub fn browser_document(&self) -> &web_sys::Document {
        &self.browser_doc
    }

    /// Inject app CSS as a plain `<style>` element in `<head>`.
    ///
    /// The element is deliberately **unmarked**: theme updates only ever rewrite
    /// the element carrying the private theme marker, so CSS injected here is
    /// never clobbered by a later dark-mode toggle (#155). Each call appends a
    /// new element — this is an append, not an upsert.
    pub fn inject_style(&self, css: &str) {
        append_style_element(&self.browser_doc, css, None);
    }

    /// Update the theme `<style>` element, or inject one if it doesn't exist.
    pub fn update_theme_style(&self, css: &str) {
        upsert_theme_style(&self.browser_doc, css);
    }

    /// Recursively walk a DOM subtree and assign `__nid` + register in HashMap.
    /// Used after `set_inner_html` which creates new child nodes without IDs.
    fn register_subtree(&mut self, node: &web_sys::Node) {
        // Assign nid to this node if it doesn't have one
        if get_nid(node).is_none() {
            let id = self.alloc_id();
            set_nid(node, id);
            self.nodes.insert(id.0, node.clone());
        }
        // Recurse into children
        let children = node.child_nodes();
        for i in 0..children.length() {
            if let Some(child) = children.item(i) {
                self.register_subtree(&child);
            }
        }
    }

    /// **Test-only.** Entries in this document's own node map (#184).
    #[doc(hidden)]
    pub fn __node_count(&self) -> usize {
        self.nodes.len()
    }

    /// **Test-only.** Whether this document still maps `nid` (#184).
    #[doc(hidden)]
    pub fn __contains(&self, nid: u64) -> bool {
        self.nodes.contains_key(&nid)
    }
}

impl Drop for WebDocument {
    /// Release this document's remaining [`NODE_REGISTRY`] entries (#184).
    ///
    /// `remove_node` already prunes churned nodes; this catches whatever a live
    /// document was still holding — most importantly its own root/body wrappers,
    /// which no `remove_node` ever reaches. `self.nodes` *is* this document's
    /// registry footprint: the two maps are populated in lockstep by the same
    /// call sites, which is why the registry needs no per-document keying.
    /// Removing an id the registry no longer holds is a no-op, which is what
    /// makes this safe even when another document's prune already swept a node
    /// of ours out of the page-global map (see [`forget_recursive`]).
    ///
    /// The document's `#rinch-root` element (when built by [`WebDocument::new`])
    /// stays attached to the real `document.body` — pre-existing behaviour, and
    /// only reachable through `mount()`, which never unmounts.
    fn drop(&mut self) {
        // `try_with` / `try_borrow_mut`: a TLS-destructor ordering hazard is
        // unreachable on wasm (thread-locals outlive the page), but a `Drop` is
        // the wrong place to panic about it.
        let _ = NODE_REGISTRY.try_with(|m| {
            if let Ok(mut reg) = m.try_borrow_mut() {
                for id in self.nodes.keys() {
                    reg.remove(id);
                }
            }
        });
        for node in self.nodes.values() {
            crate::images::forget_node(node);
        }
        crate::bounds::forget_document(self.doc_key);
    }
}

/// Walk a DOM subtree depth-first to find the text node containing the given UTF-8 byte offset.
/// Returns `(text_node, utf16_offset_within_node)`.
pub(crate) fn find_text_node_at_byte_offset(
    node: &web_sys::Node,
    byte_offset: usize,
) -> Option<(web_sys::Node, u32)> {
    let mut remaining = byte_offset;
    find_text_node_recursive(node, &mut remaining)
}

/// [`find_text_node_at_byte_offset`], but the **last** position at that byte:
/// at a text node's end, the start of the next text node, past the elements
/// with no bytes between them (an image, a `<br>`). `None` when no text
/// follows the byte (only images, or nothing, after it).
fn find_text_node_at_byte_offset_latest(
    node: &web_sys::Node,
    byte_offset: usize,
) -> Option<(web_sys::Node, u32)> {
    let mut remaining = byte_offset;
    find_text_node_latest_recursive(node, &mut remaining)
}

fn find_text_node_latest_recursive(
    node: &web_sys::Node,
    remaining: &mut usize,
) -> Option<(web_sys::Node, u32)> {
    if node.node_type() == web_sys::Node::TEXT_NODE {
        let text = node.text_content().unwrap_or_default();
        if *remaining < text.len() {
            let utf16_offset = utf8_byte_to_utf16_offset(&text, *remaining);
            return Some((node.clone(), utf16_offset as u32));
        }
        *remaining -= text.len();
        return None;
    }
    if is_node_view_host(node) {
        return None;
    }
    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i)
            && let Some(result) = find_text_node_latest_recursive(&child, remaining)
        {
            return Some(result);
        }
    }
    None
}

fn find_text_node_recursive(
    node: &web_sys::Node,
    remaining: &mut usize,
) -> Option<(web_sys::Node, u32)> {
    // If this is a text node, check if the offset falls within it
    if node.node_type() == web_sys::Node::TEXT_NODE {
        let text = node.text_content().unwrap_or_default();
        let byte_len = text.len();
        if *remaining <= byte_len {
            // Convert remaining UTF-8 byte offset to UTF-16 code unit offset
            let utf16_offset = utf8_byte_to_utf16_offset(&text, *remaining);
            return Some((node.clone(), utf16_offset as u32));
        }
        *remaining -= byte_len;
        return None;
    }
    if is_node_view_host(node) {
        return None;
    }

    // Recurse into children
    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i)
            && let Some(result) = find_text_node_recursive(&child, remaining)
        {
            return Some(result);
        }
    }
    None
}

/// The next node after `node` in document order that has no children — a
/// text node, or an empty element — without leaving `root`.
fn next_leaf_in(node: &web_sys::Node, root: &web_sys::Node) -> Option<web_sys::Node> {
    let mut cur = node.clone();
    let mut sibling = loop {
        if cur.is_same_node(Some(root)) {
            return None;
        }
        if let Some(s) = cur.next_sibling() {
            break s;
        }
        cur = cur.parent_node()?;
    };
    while !is_node_view_host(&sibling)
        && let Some(first) = sibling.first_child()
    {
        sibling = first;
    }
    Some(sibling)
}

/// Whether `node` is the host of a mounted node view
/// ([`NODE_VIEW_ATTR`](rinch_editor_view::NODE_VIEW_ATTR)): an app's component
/// drawn in an editor, whose text is not the editor's. A claimed inline image's
/// `<span>` is one atomic inline of its textblock, no text bytes wide, so the
/// byte walks below step over what it holds, as the desktop's inline formatting
/// context does (an inline-block is its own).
fn is_node_view_host(node: &web_sys::Node) -> bool {
    node.dyn_ref::<web_sys::Element>()
        .is_some_and(|e| e.has_attribute(rinch_editor_view::NODE_VIEW_ATTR))
}

/// The text node holding the character that STARTS at flat UTF-8 `byte_offset`
/// under `node`, with that character's UTF-16 start and length — unlike
/// [`find_text_node_at_byte_offset`], which answers the end of the earlier node
/// at a boundary between two. `None` past the last character.
fn find_char_at_byte_offset(
    node: &web_sys::Node,
    byte_offset: usize,
) -> Option<(web_sys::Node, u32, u32)> {
    fn walk(node: &web_sys::Node, remaining: &mut usize) -> Option<(web_sys::Node, u32, u32)> {
        if node.node_type() == web_sys::Node::TEXT_NODE {
            let text = node.text_content().unwrap_or_default();
            if *remaining < text.len() {
                let ch = text.get(*remaining..)?.chars().next()?;
                let start = utf8_byte_to_utf16_offset(&text, *remaining);
                return Some((node.clone(), start as u32, ch.len_utf16() as u32));
            }
            *remaining -= text.len();
            return None;
        }
        if is_node_view_host(node) {
            return None;
        }
        let children = node.child_nodes();
        for i in 0..children.length() {
            if let Some(child) = children.item(i)
                && let Some(found) = walk(&child, remaining)
            {
                return Some(found);
            }
        }
        None
    }
    let mut remaining = byte_offset;
    walk(node, &mut remaining)
}

/// Convert a UTF-8 byte offset to a UTF-16 code unit offset within a string.
fn utf8_byte_to_utf16_offset(text: &str, byte_offset: usize) -> usize {
    let mut utf16_offset = 0;
    for (i, ch) in text.char_indices() {
        if i >= byte_offset {
            break;
        }
        utf16_offset += ch.len_utf16();
    }
    utf16_offset
}

thread_local! {
    /// See [`leading_floors`]; measured once per page.
    static LEADING_FLOORS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// How this browser splits a line's leading around a text run's content area
/// (#1677): `true` when the half above is floored to a whole pixel (Chrome
/// 155: 17px of text in a 24px line starts 3px down), `false` when it is the
/// exact half (Firefox: 3.5px). Measured once, on a hidden probe at a few font
/// sizes until one gives an odd leading, rather than assumed from the engine's
/// name. `true` (Chrome's) when nothing tells them apart.
pub(crate) fn leading_floors() -> bool {
    if let Some(known) = LEADING_FLOORS.with(|c| c.get()) {
        return known;
    }
    let measured = (|| {
        let doc = web_sys::window()?.document()?;
        let body = doc.body()?;
        let probe = doc.create_element("div").ok()?;
        probe
            .set_attribute(
                "style",
                "position: absolute; visibility: hidden; left: -9999px; top: 0; \
                 margin: 0; padding: 0; border: 0; line-height: 24px; \
                 font-family: monospace; white-space: nowrap",
            )
            .ok()?;
        probe.set_text_content(Some("x"));
        body.append_child(&probe).ok()?;
        let text = probe.first_child()?;
        let range = doc.create_range().ok()?;
        range.select_node_contents(&text).ok()?;
        let mut answer = None;
        for size in 10..=24 {
            probe
                .dyn_ref::<web_sys::HtmlElement>()?
                .style()
                .set_property("font-size", &format!("{size}px"))
                .ok()?;
            let r = range.get_bounding_client_rect();
            let half = (24.0 - r.height()) / 2.0;
            let frac = half - half.floor();
            if r.height() <= 0.0 || !(0.25..=0.75).contains(&frac) {
                continue;
            }
            let above = r.top() - probe.get_bounding_client_rect().top();
            answer = Some((above - half.floor()).abs() < (above - half).abs());
            break;
        }
        probe.remove();
        answer
    })()
    .unwrap_or(true);
    LEADING_FLOORS.with(|c| c.set(Some(measured)));
    measured
}

/// [`WebDocument::restore_focus`]'s browser half, run with no document
/// borrowed ([`DomDocument::take_after_borrow`]).
fn restore_focus_now(
    browser_doc: &web_sys::Document,
    root_el: &web_sys::Element,
    opener: Option<web_sys::HtmlElement>,
) {
    // A focused editor's capture field answers its container (issue #1126):
    // the field is in `<body>`, so read as itself an editor inside the overlay
    // looked like a focus the user had moved outside, and the closed overlay
    // kept the keyboard.
    let editor = crate::editor_input::keyboard_editor(browser_doc);
    let active = match &editor {
        Some((_, container)) => Some(container.clone()),
        None => browser_doc.active_element(),
    };
    // `<body>` is the browser's spelling of "nothing is focused", and a
    // browser restores from it too, so it counts as inside.
    let body = browser_doc.body();
    let nowhere = match (&active, &body) {
        (None, _) => true,
        (Some(a), Some(b)) => a.is_same_node(Some(b.unchecked_ref())),
        (Some(_), None) => false,
    };
    let inside = nowhere
        || active
            .as_ref()
            .is_some_and(|a| root_el.contains(Some(a.unchecked_ref())));
    if !inside {
        return;
    }
    // The editor inside lets the keyboard go first, as a press elsewhere would
    // take it: its capture field is blurred and the editor released (a
    // composition ends into it), whether or not the opener then takes it.
    if let Some((id, _)) = &editor {
        crate::editor_input::release_editor(*id);
    }

    if let Some(el) = opener {
        // An editor opener is given the keyboard back through its capture
        // field: its container is no focus target of its own.
        if el.has_attribute("data-pm-editor") {
            if let Some(id) = get_nid(el.unchecked_ref())
                && crate::editor_input::give_editor_keyboard(browser_doc, id.0)
            {
                return;
            }
        } else {
            let _ = el.focus();
            if browser_doc
                .active_element()
                .is_some_and(|a| el.is_same_node(Some(a.unchecked_ref())))
            {
                return;
            }
        }
    }

    // An editor was released above; anything else that held the keyboard
    // inside the overlay is blurred.
    if editor.is_none()
        && let Some(el) = active.and_then(|a| a.dyn_into::<web_sys::HtmlElement>().ok())
    {
        el.blur().ok();
    }
}

thread_local! {
    /// `Element.prototype.moveBefore`, looked up once per page; `None` in a
    /// browser that does not have it (issue #1483).
    static MOVE_BEFORE: Option<js_sys::Function> = web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"Element".into()).ok())
        .and_then(|ctor| js_sys::Reflect::get(&ctor, &"prototype".into()).ok())
        .and_then(|proto| js_sys::Reflect::get(&proto, &"moveBefore".into()).ok())
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        .filter(refuses_a_disconnected_node);
}

/// Whether `move_before` is the state-preserving move and not a polyfill over
/// `insertBefore` (review of #1486): the real one throws for a node that is in
/// no document, a polyfill inserts it. Two scratch elements, attached to
/// nothing, so the probe has no effect on the page either way. A polyfill
/// would fire `blur` / `focusout` inside the call, under the caller's borrow,
/// after `moves_keeping_focus` had told the verb to release nothing.
fn refuses_a_disconnected_node(move_before: &js_sys::Function) -> bool {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return false;
    };
    match (doc.create_element("div"), doc.create_element("div")) {
        (Ok(p), Ok(c)) => move_before
            .call2(&p, &c, &wasm_bindgen::JsValue::NULL)
            .is_err(),
        _ => false,
    }
}

/// Whether putting `c` under `p` is a move `p.moveBefore(c, …)` makes: the
/// browser has the method, `c` is in the document already (asked first: a
/// mount inserts nodes that are in none, and pays this one getter), `p` is an
/// element of the same document, and `c` is a node kind the method takes (an
/// element or character data).
///
/// `moveBefore` moves a node without removing it: the focus inside it stays
/// and no `blur` / `focusout` fires, an `<iframe>` is not reloaded and a
/// scroller keeps its offset, where `insertBefore` of a connected node is a
/// removal and an insertion that drops all three (measured in Chrome 153). It
/// throws for a node of another document or of none, so those take the old
/// call. A parent that is inside `c`, or a reference that is not `p`'s child,
/// throws in both calls and moves nothing.
fn moves_in_place(p: &web_sys::Node, c: &web_sys::Node) -> bool {
    use web_sys::Node;
    MOVE_BEFORE.with(Option::is_some)
        && c.is_connected()
        && p.is_connected()
        && p.node_type() == Node::ELEMENT_NODE
        && matches!(
            c.node_type(),
            Node::ELEMENT_NODE
                | Node::TEXT_NODE
                | Node::COMMENT_NODE
                | Node::CDATA_SECTION_NODE
                | Node::PROCESSING_INSTRUCTION_NODE
        )
        && match (p.owner_document(), c.owner_document()) {
            (Some(a), Some(b)) => a.is_same_node(Some(&b)),
            _ => false,
        }
}

/// Put `c` under `p` before `reference` (at the end for `None`): as a move in
/// place where that applies ([`moves_in_place`]), and otherwise, or if the
/// browser refuses the move, with `insertBefore` (issue #1483).
///
/// [`WebDocument::moves_keeping_focus`] gives the same answer to the
/// `NodeHandle` verb, which then releases no focus first; the two must stay
/// one predicate.
fn insert_or_move(p: &web_sys::Node, c: &web_sys::Node, reference: Option<&web_sys::Node>) {
    if moves_in_place(p, c) {
        let before = reference.map_or(wasm_bindgen::JsValue::NULL, |r| r.into());
        let moved = MOVE_BEFORE.with(|f| f.as_ref().map(|f| f.call2(p, c, &before).is_ok()));
        if moved == Some(true) {
            return;
        }
    }
    p.insert_before(c, reference).ok();
}

thread_local! {
    /// Whether an element other than `<body>` may hold the page's focus
    /// (issue #1478). `true` until [`watch_focus`] has looked.
    ///
    /// [`WebDocument::release_focus_within`] is asked before every structural
    /// verb and every text write; while this is `false` it answers without
    /// asking the browser anything. Raised by every `focusin` on the document,
    /// and lowered by a `focusout` that leaves `activeElement` at `<body>` or
    /// nothing (a window losing the focus fires one too, and leaves it where
    /// it was). It errs towards `true`, and is wrong only for a focus the
    /// browser reports with no `focusin` in this document: the focus moving
    /// into an `<iframe>`, and a `focus()` made while the page itself is not
    /// focused in a browser that holds its events back until it is. A node
    /// removed around such a focus is not blurred first.
    static FOCUS_MAY_BE_HELD: Cell<bool> = const { Cell::new(true) };
    static FOCUS_WATCHED: Cell<bool> = const { Cell::new(false) };
}

/// Whether `doc`'s focus is nowhere: no active element, or `<body>`.
fn focus_is_nowhere(doc: &web_sys::Document) -> bool {
    match (doc.active_element(), doc.body()) {
        (None, _) => true,
        (Some(a), Some(b)) => a.is_same_node(Some(b.unchecked_ref())),
        (Some(_), None) => false,
    }
}

/// Start keeping [`FOCUS_MAY_BE_HELD`] for the page, once.
fn watch_focus(doc: &web_sys::Document) {
    if FOCUS_WATCHED.with(|w| w.replace(true)) {
        return;
    }
    let listen = |kind: &str, f: Box<dyn FnMut(web_sys::Event)>| {
        let cb = Closure::wrap(f);
        let _ =
            doc.add_event_listener_with_callback_and_bool(kind, cb.as_ref().unchecked_ref(), true);
        cb.forget();
    };
    listen(
        "focusin",
        Box::new(|_| FOCUS_MAY_BE_HELD.with(|f| f.set(true))),
    );
    let d = doc.clone();
    listen(
        "focusout",
        Box::new(move |_| {
            if focus_is_nowhere(&d) {
                FOCUS_MAY_BE_HELD.with(|f| f.set(false));
            }
        }),
    );
    FOCUS_MAY_BE_HELD.with(|f| f.set(!focus_is_nowhere(doc)));
}

/// `blur()` on whatever kind of element holds the focus: an `HTMLElement`, or
/// an SVG or MathML one (focusable through `tabindex`), which has the method
/// without being one.
fn blur_now(active: &web_sys::Element) {
    if let Some(el) = active.dyn_ref::<web_sys::HtmlElement>() {
        el.blur().ok();
    } else if let Ok(blur) = js_sys::Reflect::get(active, &JsValue::from_str("blur"))
        && let Some(blur) = blur.dyn_ref::<js_sys::Function>()
    {
        blur.call0(active).ok();
    }
}

impl DomDocument for WebDocument {
    fn doc_key(&self) -> u64 {
        self.doc_key
    }

    fn create_element(&mut self, tag: &str) -> NodeId {
        let el = if is_svg_tag(tag) {
            self.browser_doc
                .create_element_ns(Some("http://www.w3.org/2000/svg"), tag)
                .unwrap()
        } else {
            self.browser_doc.create_element(tag).unwrap()
        };
        let node: web_sys::Node = el.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let text_node = self.browser_doc.create_text_node(text);
        let node: web_sys::Node = text_node.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let comment = self.browser_doc.create_comment(text);
        let node: web_sys::Node = comment.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            insert_or_move(p, c, None);
        }
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            p.remove_child(c).ok();
        }
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        if let (Some(p), Some(c), Some(r)) = (
            self.nodes.get(&parent.0),
            self.nodes.get(&child.0),
            self.nodes.get(&reference.0),
        ) {
            insert_or_move(p, c, Some(r));
        }
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        // `replaceChild(n, n)` succeeds — the DOM spec re-inserts `n` before its
        // own next sibling — so guard it rather than let the detach below
        // report a node as displaced when it is still exactly where it was.
        if old == new {
            return;
        }
        let (Some(old_node), Some(new_node)) =
            (self.nodes.get(&old.0).cloned(), self.nodes.get(&new.0))
        else {
            return;
        };
        let Some(parent) = old_node.parent_node() else {
            return;
        };
        // `old` is **detached**, not retired: it keeps both map entries and a
        // handle to it can be inserted again, exactly as on desktop (issue
        // #719). The callers that are finished with it — the editor's
        // `ViewDesc` diff, a per-keystroke path — say so with `discard_node`,
        // which is what releases it (issue #184).
        parent.replace_child(new_node, &old_node).ok();
    }

    fn remove_node(&mut self, node: NodeId) {
        // No `cloned()`, unlike `discard_node` below: nothing here mutates
        // `self.nodes`, so the borrow can live across the detach.
        let Some(n) = self.nodes.get(&node.0) else {
            return;
        };
        if let Some(parent) = n.parent_node() {
            parent.remove_child(n).ok();
        }
        // Both map entries stay. A removed node must be re-insertable — append
        // its handle again and the whole subtree comes back, which is the
        // post-condition desktop has always had and the one a reactive branch
        // re-showing a *captured* handle rests on (issue #719). Pruning here is
        // what made that silently lose the subtree on this backend.
        //
        // The bookkeeping is released by `discard_node` instead, which every
        // caller that throws a subtree away calls (issue #184).
    }

    /// Detach `node` and drop it and every `__nid`-tagged descendant from both
    /// node maps (issues #184, #719).
    ///
    /// This is where the browser backend actually lets go: both maps hold a
    /// *strong* `web_sys::Node`, so an entry pins its browser node against GC
    /// for the life of the wasm module, and node creation is not structural —
    /// a keyed `for`, a `show`, or the editor's per-keystroke `ViewDesc` churn
    /// creates and destroys nodes forever within one mounted root.
    fn discard_node(&mut self, node: NodeId) {
        let Some(n) = self.nodes.get(&node.0).cloned() else {
            return;
        };
        if let Some(parent) = n.parent_node() {
            parent.remove_child(&n).ok();
        }
        // Prune *unconditionally*, outside the `parent_node()` guard: a node
        // that was built and never appended (or is already detached — every
        // `discard` after a `replace_node` is) is stranded just as hard as an
        // attached one. Descendants stay attached to `n` when `n` leaves its
        // parent, so the sweep works either side of the detach.
        forget_subtree(&mut self.nodes, &n);
    }

    fn is_retired(&self, node: NodeId) -> bool {
        !self.nodes.contains_key(&node.0)
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            // A <textarea>'s child text is its *default value*, and nothing
            // more (issue #1206): the browser shows it until the textarea's
            // dirty value flag is set — by a user edit or a `value` write — and
            // from then on a child change reaches `defaultValue` only. That is
            // HTML's rule and desktop's (#1186), so it is left to the browser.
            // This used to write `.value` from the children on every change
            // (#100), which made `textarea { {|| draft.get()} }` a controlled
            // field on the web alone; a controlled textarea is spelled
            // `value_fn` or a reactive `value:`, which `set_attribute("value")`
            // mirrors onto the property.
            n.set_text_content(Some(text));
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            // Fold the name ASCII-lowercase before deciding which of the few
            // special-cased attributes below this is — matching the browser's
            // own fold of an HTML attribute name (`Element.setAttribute` lowers
            // it for an element in the HTML namespace, per the DOM spec), which
            // is why `el.set_attribute` below is still given the ORIGINAL
            // `name`: the browser folds that write itself for HTML elements,
            // and an SVG element's attribute name is case-SIGNIFICANT (`viewBox`)
            // and must reach it unfolded. Without this, `set_attribute("CHECKED",
            // "")` set the content attribute (the browser folded that) but fell
            // through every match arm here by missing them literally, so the
            // live `.checked`/`.selected`/`.muted` property was never mirrored
            // (issue #758, the web-only gap #738 closed on desktop).
            let folded = name.to_ascii_lowercase();
            let matched = folded.as_str();
            if let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                match matched {
                    // `checked`, `selected` and `muted` are HTML **boolean**
                    // attributes: presence is the whole value. `set_attribute`
                    // is the literal primitive on both backends, so the string
                    // is written as given and the control follows the
                    // attribute's *presence*, whatever that string says (issue
                    // #622). That is the browser's own rule for raw markup —
                    // `<input checked="false">` is checked, measured in
                    // `tests/boolean_attributes.rs` — and desktop's, whose
                    // `:checked` and `<option selected>` readers look at presence
                    // alone.
                    //
                    // This arm used to mirror `attr_is_truthy` onto presence,
                    // because `rsx!` stringified a `bool` closure straight into
                    // here and nothing else would ever have turned the attribute
                    // off. #551 moved that mapping into the writer —
                    // `NodeHandle::write_attribute` removes a boolean attribute
                    // whose value is falsey, on both backends — so a reactive
                    // `checked: {|| flag.get()}` still turns off, and what was
                    // left here was a `set_attribute` that meant two different
                    // things on the two backends for exactly two names.
                    //
                    // Reach for `write_attribute` when you are rendering a value;
                    // reach for `remove_attribute` when you mean off.
                    "checked" | "selected" | "muted" => {
                        el.set_attribute(name, value).ok();
                        // Presence, not the string: see `sync_presence_property`
                        // for why the live property is written at all.
                        sync_presence_property(n, matched, true);
                        return;
                    }
                    // `indeterminate` is a property-only flag with no HTML content
                    // attribute; don't materialize a bogus one (the property is
                    // synced below).
                    "indeterminate" => {}
                    // `value` goes through `write_value_attribute`: the live
                    // property is written FIRST (with the focused control's
                    // selection carried through, issue #238), then the content
                    // attribute. On a pristine control the attribute still
                    // mirrors into `.value` — and that mirror resets the caret
                    // — so writing it first would both move the caret and make
                    // the property write below a no-op. During an IME
                    // composition the whole write is deferred.
                    "value" => {
                        write_value_attribute(&el, value);
                        return;
                    }
                    // An `<img>` source the app answers for (`images.rs`) is
                    // shown from the URL the app resolves it to; the source
                    // itself is kept beside it and is what `get_attribute`
                    // answers.
                    "src" if el.tag_name().eq_ignore_ascii_case("img") => {
                        crate::images::set_img_src(&el, name, value);
                        return;
                    }
                    _ => {
                        el.set_attribute(name, value).ok();
                    }
                }
            }
            // A handful of attributes are reflected *asymmetrically* onto a live
            // DOM property on form controls (`value`, `checked`, `selected`,
            // `indeterminate`, `muted`). Writing the attribute alone only sets
            // the control's *default*: the moment the user edits the control it
            // goes "dirty" and the browser stops mirroring the attribute into the
            // property. A reactive `value:`/`checked:` binding would then silently
            // stop updating what is displayed. Mirror the property too so
            // signal-driven updates keep working after the control has been typed
            // into / toggled (issue #100).
            //
            // `checked`, `selected` and `muted` returned above, having mirrored
            // their own presence; `value` returned into `write_value_attribute`.
            // What reaches here is `indeterminate` and everything with no
            // reflected property at all.
            sync_reflected_property(n, matched, value);
        }
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            // See the matching comment in `set_attribute`: fold only for the
            // match decisions below, never for the literal write.
            let folded = name.to_ascii_lowercase();
            let matched = folded.as_str();
            if let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                // Removing `value` is a write of `""`: property first (for the
                // same pristine-control mirroring reason as in `set_attribute`)
                // and deferred during an IME composition, so both spellings of
                // "clear this field" obey one policy (issue #238).
                if matched == "value" {
                    remove_value_attribute(&el);
                    return;
                }
                if matched == "src" {
                    crate::images::remove_img_src(&el);
                }
                el.remove_attribute(name).ok();
            }
            // Keep the reflected property in sync when the attribute is removed,
            // otherwise a dirtied control keeps showing the stale property (#100).
            match matched {
                // Absence is the off state for the presence trio, and the only
                // one — a dirtied control would otherwise keep showing the stale
                // property (#100, #622, #754).
                "checked" | "selected" | "muted" => sync_presence_property(n, matched, false),
                // `indeterminate` has no content attribute to be absent, so
                // "remove it" is simply "off".
                "indeterminate" => sync_reflected_property(n, matched, "false"),
                _ => {}
            }
        }
    }

    /// The element's local name: lowercase for an HTML element, the author's
    /// spelling for an SVG one — what desktop stores (#1784, so a key listener
    /// can ask `active_element()?.tag_name()` on both backends).
    fn tag_name(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        let el: &web_sys::Element = n.dyn_ref()?;
        Some(el.local_name())
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        // An app-resolved `<img>` answers the source it was given, not the
        // object URL the browser is loading for it.
        if name.eq_ignore_ascii_case("src")
            && let Some(logical) = crate::images::logical_src(&el)
        {
            return Some(logical);
        }
        el.get_attribute(name)
    }

    /// The `.value` **property** of an `<input>`, `<textarea>` or `<select>`
    /// (issue #238) — the text the user sees — never the content attribute,
    /// which typing leaves at whatever was last written programmatically. Any
    /// other element answers from its `value` attribute, as the trait default
    /// does. A `file` input's `.value` is the browser's fake path
    /// (`C:\fakepath\…`), which is what the browser itself reports.
    fn live_value(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        if let Some(input) = n.dyn_ref::<web_sys::HtmlInputElement>() {
            Some(input.value())
        } else if let Some(textarea) = n.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            Some(textarea.value())
        } else if let Some(select) = n.dyn_ref::<web_sys::HtmlSelectElement>() {
            Some(select.value())
        } else {
            n.dyn_ref::<web_sys::Element>()?.get_attribute("value")
        }
    }

    /// Straight to the browser's own CSSOM, which is why nothing here restates
    /// the property-name rules desktop needs (#711): `setProperty` lowercases a
    /// non-custom name and compares custom ones exactly, and no `rinch-web` code
    /// compares a property name itself. The `style:` prop's merge is a
    /// different path and *is* shared — [`StyleProp`] reads and writes the
    /// whole `style` attribute through `rinch-core`'s parser on both backends,
    /// so it takes the fold from there.
    ///
    /// [`StyleProp`]: rinch_core::dom::StyleProp
    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>()
        {
            el.style().set_property(property, value).ok();
        }
    }

    fn mark_dirty(&mut self, _node: NodeId) {
        // No-op: browser handles reflow automatically.
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        // No-op: browser handles reflow automatically.
        Vec::new()
    }

    fn root(&self) -> NodeId {
        self.root_id
    }

    fn body(&self) -> NodeId {
        self.body_id
    }

    fn query_selector(&self, selector: &str) -> Option<NodeId> {
        let el = warn_invalid_selector(selector, self.browser_doc.query_selector(selector))??;
        let node: web_sys::Node = el.into();
        get_nid(&node)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        node_list_ids(warn_invalid_selector(
            selector,
            self.browser_doc.query_selector_all(selector),
        ))
    }

    /// `Element.querySelector` on the scope (#1796); a document scope asks
    /// it, and any other node (a text node) has no descendants.
    fn query_selector_in(&self, scope: NodeId, selector: &str) -> Option<NodeId> {
        let n = self.nodes.get(&scope.0)?;
        let found = if let Some(el) = n.dyn_ref::<web_sys::Element>() {
            el.query_selector(selector)
        } else {
            n.dyn_ref::<web_sys::Document>()?.query_selector(selector)
        };
        let node: web_sys::Node = warn_invalid_selector(selector, found)??.into();
        get_nid(&node)
    }

    fn query_selector_all_in(&self, scope: NodeId, selector: &str) -> Vec<NodeId> {
        let Some(n) = self.nodes.get(&scope.0) else {
            return Vec::new();
        };
        let found = if let Some(el) = n.dyn_ref::<web_sys::Element>() {
            el.query_selector_all(selector)
        } else if let Some(d) = n.dyn_ref::<web_sys::Document>() {
            d.query_selector_all(selector)
        } else {
            return Vec::new();
        };
        node_list_ids(warn_invalid_selector(selector, found))
    }

    fn is_text_node(&self, node: NodeId) -> bool {
        self.nodes
            .get(&node.0)
            .is_some_and(|n| n.node_type() == web_sys::Node::TEXT_NODE)
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        if let Some(n) = self.nodes.get(&node.0) {
            let children = n.child_nodes();
            for i in 0..children.length() {
                if let Some(child) = children.item(i)
                    && let Some(nid) = get_nid(&child)
                {
                    result.push(nid);
                }
            }
        }
        result
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            // `item` past the end is `None`, which appends.
            let reference = p.child_nodes().item(index as u32);
            insert_or_move(p, c, reference.as_ref());
        }
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        let n = self.nodes.get(&node.0)?;
        let parent = n.parent_node()?;
        get_nid(&parent)
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        let n = self.nodes.get(&node.0)?;
        let sibling = n.next_sibling()?;
        get_nid(&sibling)
    }

    fn parse_html(&mut self, html: &str) -> Option<NodeId> {
        let temp = self.browser_doc.create_element("div").ok()?;
        temp.set_inner_html(html);
        let first_child = temp.first_child()?;
        // Register the subtree
        self.register_subtree(&first_child);
        get_nid(&first_child)
    }

    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            el.set_scroll_top(scroll_px(scroll_top));
        }
    }

    fn scroll_top(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_top() as f64)
            .unwrap_or(0.0)
    }

    fn scroll_left(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_left() as f64)
            .unwrap_or(0.0)
    }

    fn set_scroll_left(&mut self, node: NodeId, scroll_left: f64) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            el.set_scroll_left(scroll_px(scroll_left));
        }
    }

    fn scroll_height(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_height() as f64)
            .unwrap_or(0.0)
    }

    fn scroll_width(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_width() as f64)
            .unwrap_or(0.0)
    }

    fn client_height(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.client_height() as f64)
            .unwrap_or(0.0)
    }

    fn client_width(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.client_width() as f64)
            .unwrap_or(0.0)
    }

    /// Scroll `node` into view **now**, rather than queueing the request the way
    /// the desktop backend does. The trait defers because Taffy layout has to be
    /// resolved before an element's position relative to its scroll container is
    /// known; in the browser layout is synchronous, so by the time a caller holds
    /// a node its box is already measurable and `scrollIntoView` reads it itself.
    /// There is correspondingly nothing for `drain_scroll_into_view_requests` to
    /// return — the default (empty) is right here.
    ///
    /// `block`/`inline: "nearest"` is the minimal scroll: an element already in
    /// view does not move, and one out of view is brought just inside the edge it
    /// left by. Centring would jump the page on every caret move. The behaviour is
    /// left at the default (`auto`, i.e. instant) deliberately — `smooth` animates,
    /// and a caret that keeps typing would chase a still-moving viewport.
    fn request_scroll_into_view(&mut self, node: NodeId) {
        let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
        opts.set_inline(web_sys::ScrollLogicalPosition::Nearest);
        el.scroll_into_view_with_scroll_into_view_options(&opts);
    }

    /// Place `node` in its scroll container **now** (see
    /// [`Self::request_scroll_into_view`] for why the browser needs no queue):
    /// the nearest ancestor whose `overflow-y` scrolls, or the page's scrolling
    /// element when none does, gets the `scrollTop` that puts the node's top
    /// `fraction` of the way down its visible height (at least `margin` inside
    /// either edge), clamped to `0..=scrollHeight - clientHeight`. The node then
    /// gets the same "nearest" `scrollIntoView` a caret gets, which leaves that
    /// container alone (the node is inside it now) and brings it into the view
    /// of every scroller further out, the page included.
    fn request_scroll_to_fraction(&mut self, node: NodeId, fraction: f32, margin: f32) {
        let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let mut scroller = None;
        let mut cur = el.parent_element();
        while let Some(ancestor) = cur {
            let scrolls = window
                .get_computed_style(&ancestor)
                .ok()
                .flatten()
                .and_then(|cs| cs.get_property_value("overflow-y").ok())
                .is_some_and(|o| o == "auto" || o == "scroll");
            if scrolls {
                scroller = Some(ancestor);
                break;
            }
            cur = ancestor.parent_element();
        }
        let page = scroller.is_none();
        let Some(scroller) =
            scroller.or_else(|| window.document().and_then(|d| d.scrolling_element()))
        else {
            return;
        };
        let rect = el.get_bounding_client_rect();
        // The top of the scroller's visible area in client px: the page's is
        // the viewport's top; an element's is its padding box's top.
        let view_top = if page {
            0.0
        } else {
            scroller.get_bounding_client_rect().top() + f64::from(scroller.client_top())
        };
        let visible = f64::from(scroller.client_height());
        let (fraction, margin) = (f64::from(fraction), f64::from(margin));
        let at = (fraction * visible)
            .min(visible - margin - rect.height())
            .max(margin);
        let current = f64::from(scroller.scroll_top());
        let max = f64::from(scroller.scroll_height() - scroller.client_height()).max(0.0);
        let target = (current + (rect.top() - view_top) - at).clamp(0.0, max);
        scroller.set_scroll_top(target.round() as i32);
        if !page {
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
            opts.set_inline(web_sys::ScrollLogicalPosition::Nearest);
            el.scroll_into_view_with_scroll_into_view_options(&opts);
        }
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            // `set_inner_html` discards every existing child, so forget them
            // first — the desktop backend already does this (issue #184). One
            // registry borrow for the whole sweep, not one per child.
            forget_children(&mut self.nodes, &el);
            el.set_inner_html(html);
            // Walk all new child nodes and register them
            let children = el.child_nodes();
            for i in 0..children.length() {
                if let Some(child) = children.item(i) {
                    self.register_subtree(&child);
                }
            }
            // Parsed markup skipped `set_attribute`: an `<img>` source the
            // app answers for goes through the same path as one set one.
            crate::images::adopt_parsed_images(&el);
        }
    }

    /// A `<br>` only ([`WebDocument::br_caret_viewport_rect`]); an image is
    /// still asked by byte.
    fn query_inline_box_caret(
        &self,
        node_id: u64,
        box_id: u64,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        let (x, y, h) = self.br_caret_viewport_rect(node_id, box_id, after)?;
        let block = self.nodes.get(&node_id)?;
        let r = block
            .dyn_ref::<web_sys::Element>()?
            .get_bounding_client_rect();
        Some((x - r.x() as f32, y - r.y() as f32, h))
    }

    fn query_inline_box_caret_rect(
        &self,
        node_id: u64,
        box_id: u64,
        after: bool,
    ) -> Option<(f32, f32, f32)> {
        self.br_caret_viewport_rect(node_id, box_id, after)
    }

    /// Downstream at a soft wrap, as on desktop — see
    /// [`Self::query_caret_position_with_affinity`].
    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)> {
        self.query_caret_position_with_affinity(node_id, byte_offset, CaretAffinity::Downstream)
    }

    fn query_caret_position_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        let n = self.nodes.get(&node_id)?;
        let (x, y, _) = self.text_caret_viewport_rect(n, byte_offset, affinity)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let block_rect = el.get_bounding_client_rect();
        Some((x - block_rect.x() as f32, y - block_rect.y() as f32))
    }

    /// [`Self::query_line_boxes`] for one caret.
    fn query_line_box(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        self.query_line_boxes(node_id, &[(byte_offset, affinity)])
            .pop()
            .flatten()
    }

    /// The browser exposes no line box, so it is rebuilt the way CSS builds
    /// it: the union of the boxes of what is on a caret's line. A piece of
    /// text contributes its inline box — its content area grown (or shrunk)
    /// to **its own element's** computed `line-height`, the smaller whole-px
    /// half of the leading above as Chrome splits it; under `line-height:
    /// normal`, the content area itself (so a line of mixed sizes there is
    /// within a pixel) — and a picture or a node view's host its own box.
    /// What is "on a caret's line" is every piece whose content area overlaps
    /// the line's caret rect by more than half the smaller height: a picture
    /// taller than the text, a `<sup>`, a `code` run, and no piece of a
    /// neighbouring line (#1677).
    ///
    /// The block is read **once** for all of `carets`: one walk of its text
    /// (each text node's string fetched once, each one's client rects once),
    /// then for each caret one range rect — the character after it, which is
    /// on the line a downstream caret is drawn on; the last character's end
    /// at the block's end. `affinity` is not read: a margin anchor's line is
    /// its downstream one.
    fn query_line_boxes(
        &self,
        node_id: u64,
        carets: &[(usize, CaretAffinity)],
    ) -> Vec<Option<(f32, f32)>> {
        let none = || vec![None; carets.len()];
        let Some(block) = self.nodes.get(&node_id) else {
            return none();
        };
        let Some(el) = block.dyn_ref::<web_sys::Element>() else {
            return none();
        };
        let Ok(range) = self.browser_doc.create_range() else {
            return none();
        };
        let window = web_sys::window();
        // A piece of the line: its content area, and its inline box.
        struct Piece {
            top: f32,
            bottom: f32,
            box_top: f32,
            box_bottom: f32,
        }
        let mut pieces: Vec<Piece> = Vec::new();
        // Text nodes in flat-byte order: (node, its text, its first flat byte).
        let mut texts: Vec<(web_sys::Node, String, usize)> = Vec::new();
        let mut line_heights: Vec<(web_sys::Element, Option<f32>)> = Vec::new();
        let mut flat = 0usize;
        let mut stack: Vec<web_sys::Node> = Vec::new();
        let kids = block.child_nodes();
        for i in (0..kids.length()).rev() {
            stack.extend(kids.item(i));
        }
        while let Some(node) = stack.pop() {
            if node.node_type() == web_sys::Node::TEXT_NODE {
                let text = node.text_content().unwrap_or_default();
                let lh = node.parent_element().and_then(|parent| {
                    if let Some((_, lh)) = line_heights.iter().find(|(e, _)| *e == parent) {
                        return *lh;
                    }
                    let lh = window
                        .as_ref()
                        .and_then(|w| w.get_computed_style(&parent).ok().flatten())
                        .and_then(|cs| cs.get_property_value("line-height").ok())
                        .and_then(|v| v.trim().strip_suffix("px")?.parse::<f32>().ok())
                        .filter(|h| *h > 0.0);
                    line_heights.push((parent, lh));
                    lh
                });
                if range.select_node_contents(&node).is_ok()
                    && let Some(rects) = range.get_client_rects()
                {
                    for j in 0..rects.length() {
                        let Some(r) = rects.item(j) else { continue };
                        let (t, b) = (r.top() as f32, r.bottom() as f32);
                        if b <= t {
                            continue;
                        }
                        let (box_top, box_bottom) = match lh {
                            Some(lh) => {
                                let half = (lh - (b - t)) / 2.0;
                                let above = if leading_floors() { half.floor() } else { half };
                                (t - above, t - above + lh)
                            }
                            None => (t, b),
                        };
                        pieces.push(Piece {
                            top: t,
                            bottom: b,
                            box_top,
                            box_bottom,
                        });
                    }
                }
                let len = text.len();
                texts.push((node, text, flat));
                flat += len;
                continue;
            }
            let Some(child) = node.dyn_ref::<web_sys::Element>() else {
                continue;
            };
            let tag = child.tag_name().to_ascii_lowercase();
            if tag == "br" {
                continue;
            }
            let atomic = matches!(
                tag.as_str(),
                "img" | "svg" | "video" | "canvas" | "iframe" | "input" | "button"
            ) || child.has_attribute(rinch_editor_view::NODE_VIEW_ATTR);
            if atomic {
                let r = child.get_bounding_client_rect();
                let (t, b) = (r.top() as f32, r.bottom() as f32);
                if b > t {
                    pieces.push(Piece {
                        top: t,
                        bottom: b,
                        box_top: t,
                        box_bottom: b,
                    });
                }
                continue;
            }
            let kids = node.child_nodes();
            for i in (0..kids.length()).rev() {
                stack.extend(kids.item(i));
            }
        }
        let block_top = el.get_bounding_client_rect().y() as f32;
        // Each caret's UTF-16 offset, found walking the text once in byte
        // order (from the start of its text node for each caret was quadratic
        // in a long paragraph's anchors).
        let mut order: Vec<usize> = (0..carets.len()).collect();
        order.sort_by_key(|&i| carets[i].0);
        let mut at: Vec<Option<(usize, u32, u32)>> = vec![None; carets.len()];
        let (mut ti, mut byte_in, mut utf16_in) = (0usize, 0usize, 0u32);
        for i in order {
            let byte = carets[i].0;
            // The text node holding the character that starts at `byte`.
            while ti < texts.len() && byte >= texts[ti].2 + texts[ti].1.len() {
                ti += 1;
                byte_in = 0;
                utf16_in = 0;
            }
            let (t, local) = match texts.get(ti) {
                Some((_, text, start)) => (ti, (byte - start).min(text.len())),
                None => match texts.last() {
                    Some((_, text, _)) => (texts.len() - 1, text.len()),
                    None => continue,
                },
            };
            if t != ti {
                // Past the end: the last node, from its start.
                let text = &texts[t].1;
                let off = utf8_byte_to_utf16_offset(text, local) as u32;
                at[i] = Some((t, off, off));
                continue;
            }
            let text = &texts[t].1;
            for ch in text[byte_in..].chars() {
                if byte_in >= local {
                    break;
                }
                byte_in += ch.len_utf8();
                utf16_in += ch.len_utf16() as u32;
            }
            let end = text[byte_in..]
                .chars()
                .next()
                .map_or(utf16_in, |ch| utf16_in + ch.len_utf16() as u32);
            at[i] = Some((t, utf16_in, end));
        }
        at.into_iter()
            .map(|at| {
                let (t, off, end) = at?;
                let node = &texts[t].0;
                range.set_start(node, off).ok()?;
                range.set_end(node, end).ok()?;
                let r = range.get_bounding_client_rect();
                let (cy, ch) = (r.top() as f32, r.height() as f32);
                if ch <= 0.0 {
                    return None;
                }
                let (mut top, mut bottom) = (f32::INFINITY, f32::NEG_INFINITY);
                for p in &pieces {
                    let overlap = p.bottom.min(cy + ch) - p.top.max(cy);
                    if overlap > 0.0 && overlap > 0.5 * (p.bottom - p.top).min(ch) {
                        top = top.min(p.box_top);
                        bottom = bottom.max(p.box_bottom);
                    }
                }
                top.is_finite().then_some((top - block_top, bottom - top))
            })
            .collect()
    }

    fn query_caret_rect_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        let block = self.nodes.get(&node_id)?;
        if let Some(rect) = self.text_caret_viewport_rect(block, byte_offset, affinity) {
            return Some(rect);
        }
        // No text node (an empty block): the block's content box, one line
        // high — where a padded block's text would start (#1667).
        let el = block.dyn_ref::<web_sys::Element>()?;
        let r = el.get_bounding_client_rect();
        let (l, t, _, b) = self.content_box_inset(node_id);
        let ch = self
            .line_box_height(node_id)
            .unwrap_or(r.height() as f32 - t - b);
        let h = if ch > 0.0 { ch } else { 18.0 };
        Some((r.x() as f32 + l, r.y() as f32 + t, h))
    }

    /// Downstream at a soft wrap, as on desktop — see
    /// [`Self::query_caret_rect_with_affinity`].
    fn query_caret_rect(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32, f32)> {
        self.query_caret_rect_with_affinity(node_id, byte_offset, CaretAffinity::Downstream)
    }

    fn query_glyph_bounds(&self, node_id: u64, byte_offset: usize) -> Option<GlyphBounds> {
        let n = self.nodes.get(&node_id)?;
        let (text_node, utf16_offset) = find_text_node_at_byte_offset(n, byte_offset)?;
        let text_content = text_node.text_content().unwrap_or_default();
        let text_utf16_len: usize = text_content.encode_utf16().count();

        let range = self.browser_doc.create_range().ok()?;
        // If we're at the end of text, use the last character's bounds
        if utf16_offset as usize >= text_utf16_len {
            if text_utf16_len == 0 {
                return None;
            }
            range
                .set_start(&text_node, (text_utf16_len - 1) as u32)
                .ok()?;
            range.set_end(&text_node, text_utf16_len as u32).ok()?;
        } else {
            range.set_start(&text_node, utf16_offset).ok()?;
            range.set_end(&text_node, utf16_offset + 1).ok()?;
        }

        let rect = range.get_bounding_client_rect();
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let block_rect = el.get_bounding_client_rect();

        Some(GlyphBounds {
            x: (rect.x() - block_rect.x()) as f32,
            y: (rect.y() - block_rect.y()) as f32,
            width: rect.width() as f32,
            height: rect.height() as f32,
        })
    }

    /// The `focus()` runs after the caller's borrow is released
    /// ([`DomDocument::take_after_borrow`]).
    fn focus_element(&mut self, node_id: NodeId) {
        if let Some(n) = self.nodes.get(&node_id.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>()
        {
            self.after_borrow.push(Box::new(move || {
                el.focus().ok();
            }));
        }
    }

    fn take_after_borrow(&mut self) -> Option<Box<dyn FnOnce()>> {
        if self.after_borrow.is_empty() {
            return None;
        }
        let work = std::mem::take(&mut self.after_borrow);
        Some(Box::new(move || {
            for job in work {
                job();
            }
        }))
    }

    /// [`moves_in_place`] for the two nodes: the question
    /// `append_child` / `insert_before` / `insert_child` ask themselves
    /// (issue #1483).
    fn moves_keeping_focus(&self, parent: NodeId, child: NodeId) -> bool {
        match (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            (Some(p), Some(c)) => moves_in_place(p, c),
            _ => false,
        }
    }

    /// The focused element, when it is inside the part of `node` that is about
    /// to go: its `blur()` (issue #1478).
    ///
    /// No JS call while nothing on the page holds the focus
    /// ([`FOCUS_MAY_BE_HELD`]), which is the whole of a mount and every
    /// update made from a timer or a network reply. With the focus somewhere:
    /// one (`nodeType`) for a text write to a text node, which can hold no
    /// focused child, and otherwise `activeElement` and `contains`.
    fn release_focus_within(
        &self,
        node: NodeId,
        children_only: bool,
        last_round: bool,
    ) -> Option<Box<dyn FnOnce()>> {
        if !FOCUS_MAY_BE_HELD.with(Cell::get) {
            return None;
        }
        let n = self.nodes.get(&node.0)?;
        if children_only && n.node_type() != web_sys::Node::ELEMENT_NODE {
            return None;
        }
        // A focused editor in the part that goes holds the keyboard through
        // its capture field in `<body>`, which no removal reaches (issue
        // #1126): release the editor, as the browser blurs a focused element
        // it takes out.
        if let Some((id, container)) = crate::editor_input::keyboard_editor(&self.browser_doc) {
            let goes = n.contains(Some(container.unchecked_ref()))
                && !(children_only && n.is_same_node(Some(container.unchecked_ref())));
            return goes.then(|| {
                Box::new(move || crate::editor_input::release_editor(id)) as Box<dyn FnOnce()>
            });
        }
        let active = self.browser_doc.active_element()?;
        if !n.contains(Some(active.unchecked_ref())) {
            return None;
        }
        if children_only && n.is_same_node(Some(active.unchecked_ref())) {
            return None;
        }
        // `<body>` is the browser's spelling of "nothing is focused" (an
        // island mounted into it contains it), and blurring it does nothing.
        if self
            .browser_doc
            .body()
            .is_some_and(|b| b.is_same_node(Some(active.unchecked_ref())))
        {
            return None;
        }
        if !last_round {
            return Some(Box::new(move || blur_now(&active)));
        }
        // A listener has put the focus back inside the part that goes on every
        // round so far. Make that part unfocusable for the length of this last
        // blur: `focus()` into an inert subtree is refused, so when the blur
        // returns nothing in there holds the focus and no listener is running.
        // The attribute is gone again before the mutation.
        let holder = n.dyn_ref::<web_sys::Element>().cloned();
        Some(Box::new(move || {
            let added = holder
                .as_ref()
                .filter(|el| !el.has_attribute("inert"))
                .map(|el| el.set_attribute("inert", "").is_ok())
                .unwrap_or(false);
            blur_now(&active);
            if added && let Some(el) = &holder {
                el.remove_attribute("inert").ok();
            }
        }))
    }

    /// For the focused element: the literal browser write, which is what
    /// `set_attribute` / `remove_attribute` make for these names (issue #1478).
    fn before_attribute_write(
        &self,
        node: NodeId,
        name: &str,
        value: Option<&str>,
    ) -> Option<Box<dyn FnOnce()>> {
        let el = self.nodes.get(&node.0)?.dyn_ref::<web_sys::Element>()?;
        let active = self.browser_doc.active_element()?;
        if !el.is_same_node(Some(active.unchecked_ref())) {
            return None;
        }
        let (el, name, value) = (el.clone(), name.to_owned(), value.map(str::to_owned));
        Some(Box::new(move || {
            match value {
                Some(value) => el.set_attribute(&name, &value).ok(),
                None => el.remove_attribute(&name).ok(),
            };
        }))
    }

    /// `HTMLInputElement`/`HTMLTextAreaElement.setSelectionRange(start, end,
    /// direction)` directly (issue #552) — `start`/`end` are already UTF-16
    /// code units on this backend, a JS string's own unit, so no conversion
    /// is needed the way desktop's `EditableState` (byte offsets) requires.
    /// The browser itself implements "set now, or honour it the first time
    /// this control is focused" for an unfocused target, so there is nothing
    /// here to stash.
    fn set_selection_range(
        &mut self,
        node_id: NodeId,
        start: usize,
        end: usize,
        direction: rinch_core::dom::SelectionDirection,
    ) {
        let Some(n) = self.nodes.get(&node_id.0) else {
            return;
        };
        let start = start as u32;
        let end = end as u32;
        let direction = match direction {
            rinch_core::dom::SelectionDirection::Forward => "forward",
            rinch_core::dom::SelectionDirection::Backward => "backward",
            rinch_core::dom::SelectionDirection::None => "none",
        };
        if let Some(input) = n.dyn_ref::<web_sys::HtmlInputElement>() {
            let _ = input.set_selection_range_with_direction(start, end, direction);
        } else if let Some(textarea) = n.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            let _ = textarea.set_selection_range_with_direction(start, end, direction);
        }
    }

    /// The element's own `selectionStart` / `selectionEnd` /
    /// `selectionDirection` (issue #1678) — UTF-16 code units, a JS string's
    /// own. `None` where the browser answers `null` (an `<input>` type with
    /// no text selection) or for any other element.
    fn text_selection_range(
        &self,
        node_id: NodeId,
    ) -> Option<(usize, usize, rinch_core::dom::SelectionDirection)> {
        let n = self.nodes.get(&node_id.0)?;
        let (start, end, direction) = if let Some(i) = n.dyn_ref::<web_sys::HtmlInputElement>() {
            (
                i.selection_start().ok().flatten()?,
                i.selection_end().ok().flatten()?,
                i.selection_direction().ok().flatten(),
            )
        } else {
            let t = n.dyn_ref::<web_sys::HtmlTextAreaElement>()?;
            (
                t.selection_start().ok().flatten()?,
                t.selection_end().ok().flatten()?,
                t.selection_direction().ok().flatten(),
            )
        };
        let direction = match direction.as_deref() {
            Some("backward") => rinch_core::dom::SelectionDirection::Backward,
            Some("none") => rinch_core::dom::SelectionDirection::None,
            _ => rinch_core::dom::SelectionDirection::Forward,
        };
        let (start, end) = (start as usize, end as usize);
        Some((start.min(end), start.max(end), direction))
    }

    /// Where a caret at a UTF-16 offset of the control's value sits, in
    /// client px (issue #1678). A browser exposes no caret geometry for a
    /// form control, so the value is laid out in a hidden `<div>` appended to
    /// `<body>` for the call (an app's `MutationObserver` on the body sees it
    /// come and go) with the control's computed text styles, direction,
    /// `white-space` and padding, at the width the control lays its text out
    /// in (`clientWidth` less the horizontal padding: a scrollbar's gutter is
    /// not text room), and the caret is a collapsed `Range` at the offset in
    /// it — placed by the browser's own line breaking and bidi rules. That
    /// position is moved by the control's own scroll, unscaled by the
    /// mirror's own painted/layout ratio (a transform or zoom on `<body>` or
    /// `<html>`), scaled by the control's, and placed at its painted box. A
    /// single-line field centres its line in its content box, as Chrome draws
    /// it. `None` for a control that is not rendered.
    fn text_caret_rect(
        &mut self,
        node_id: NodeId,
        offset: Option<usize>,
    ) -> Option<rinch_core::reactive::ElementBounds> {
        const MIRRORED: &[&str] = &[
            "direction",
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
            "font-style",
            "font-variant",
            "font-weight",
            "font-stretch",
            "font-size",
            "font-size-adjust",
            "line-height",
            "font-family",
            "font-kerning",
            "font-feature-settings",
            "font-variation-settings",
            "text-align",
            "text-transform",
            "text-indent",
            "letter-spacing",
            "word-spacing",
            "tab-size",
            "overflow-wrap",
            "word-break",
            "unicode-bidi",
        ];
        if !self.is_rendered(node_id) {
            return None;
        }
        let (start, end, direction) = self.text_selection_range(node_id)?;
        let n = self.nodes.get(&node_id.0)?;
        let el: web_sys::HtmlElement = n.clone().dyn_into().ok()?;
        let (value, single_line, password) =
            if let Some(i) = n.dyn_ref::<web_sys::HtmlInputElement>() {
                (i.value(), true, i.type_().eq_ignore_ascii_case("password"))
            } else {
                (
                    n.dyn_ref::<web_sys::HtmlTextAreaElement>()?.value(),
                    false,
                    false,
                )
            };
        let units: Vec<u16> = value.encode_utf16().collect();
        let head = offset.unwrap_or(
            if direction == rinch_core::dom::SelectionDirection::Backward {
                start
            } else {
                end
            },
        );
        let head = head.min(units.len());
        // The mirror's text and the caret's offset into it. A password field
        // draws a bullet per character, so the offset becomes a char count.
        // A zero-width space after the value gives a caret at its very end
        // (after a trailing newline, in an empty field) a line box to sit in;
        // it is neutral to bidi and takes no width.
        let (text, at) = if password {
            let chars = String::from_utf16_lossy(&units[..head]).chars().count();
            (
                format!("{}\u{200b}", "\u{2022}".repeat(value.chars().count())),
                chars,
            )
        } else {
            (format!("{value}\u{200b}"), head)
        };

        let win = web_sys::window()?;
        let doc = win.document()?;
        let body = doc.body()?;
        let cs = win.get_computed_style(&el).ok()??;
        let px = |p: &str| {
            cs.get_property_value(p)
                .ok()
                .and_then(|v| v.trim_end_matches("px").parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        let (pad_l, pad_r, pad_t, pad_b) = (
            px("padding-left"),
            px("padding-right"),
            px("padding-top"),
            px("padding-bottom"),
        );
        // The border widths as computed, not `clientLeft`/`clientTop`: Gecko
        // answers those for an `<input>` as border plus padding (Firefox 155:
        // `clientLeft` 9 for a 1px border and 8px padding).
        let (border_l, border_t, border_b) = (
            px("border-left-width"),
            px("border-top-width"),
            px("border-bottom-width"),
        );
        // A textarea's text room is its client width (a scrollbar's gutter is
        // not text room); an `<input>` has no scrollbar, and Gecko's
        // `clientWidth` for one already leaves its padding out, so its room
        // comes from the border box.
        let border_r = px("border-right-width");
        let text_width = if single_line {
            el.offset_width() as f64 - border_l - border_r - pad_l - pad_r
        } else {
            el.client_width() as f64 - pad_l - pad_r
        }
        .max(0.0);
        let mirror: web_sys::HtmlElement = doc.create_element("div").ok()?.dyn_into().ok()?;
        let ms = mirror.style();
        for prop in MIRRORED {
            if let Ok(v) = cs.get_property_value(prop) {
                let _ = ms.set_property(prop, &v);
            }
        }
        // A textarea wraps as its own `white-space` says (`wrap="off"` is
        // `pre`); a single-line field never wraps.
        let white_space = if single_line {
            "pre".to_string()
        } else {
            cs.get_property_value("white-space")
                .unwrap_or_else(|_| "pre-wrap".into())
        };
        let width = format!("{text_width}px");
        for (prop, v) in [
            ("position", "absolute"),
            ("visibility", "hidden"),
            ("top", "0"),
            ("left", "0"),
            ("overflow", "hidden"),
            ("box-sizing", "content-box"),
            ("border", "0"),
            ("margin", "0"),
            ("height", "auto"),
            ("transform", "none"),
            ("width", width.as_str()),
            ("white-space", white_space.as_str()),
        ] {
            let _ = ms.set_property(prop, v);
        }
        let node = doc.create_text_node(&text);
        mirror.append_child(&node).ok()?;
        body.append_child(&mirror).ok()?;
        let m = mirror.get_bounding_client_rect();
        // A collapsed range at the offset: the browser places it by the same
        // line-breaking and bidi rules it lays the control's text out with,
        // where a marker's edge is the wrong end of a right-to-left run.
        // Right after a line break the caret starts the next line, but a
        // collapsed range there is the end of the line before in Gecko
        // (Firefox 155; Chrome puts it on the next line). The character at
        // the offset — the zero-width space at the very end — is on that next
        // line in both, so the caret is its start edge.
        let after_break = !password && head > 0 && units[head - 1] == u16::from(b'\n');
        let rtl = cs.get_property_value("direction").is_ok_and(|d| d == "rtl");
        let caret = doc.create_range().ok().and_then(|range| {
            range.set_start(&node, at as u32).ok()?;
            if after_break {
                range.set_end(&node, at as u32 + 1).ok()?;
                let r = range.get_client_rects()?.get(0)?;
                let x = if rtl { r.right() } else { r.left() };
                return web_sys::DomRect::new_with_x_and_y_and_width_and_height(
                    x,
                    r.top(),
                    0.0,
                    r.height(),
                )
                .ok();
            }
            range.collapse_with_to_start(true);
            let rects = range.get_client_rects()?;
            rects
                .get(0)
                .or_else(|| Some(range.get_bounding_client_rect()))
        });
        // The mirror's own painted/layout ratio: a transform or zoom on
        // `<body>` or `<html>` scales the mirror too, and the control's ratio
        // below applies it again.
        let (mw, mh) = (mirror.offset_width() as f64, mirror.offset_height() as f64);
        let mx = if mw > 0.0 { m.width() / mw } else { 1.0 };
        let my = if mh > 0.0 { m.height() / mh } else { 1.0 };
        let _ = body.remove_child(&mirror);
        let caret = caret?;
        // In the control's own (untransformed) padding-box coordinates.
        let local_x = (caret.left() - m.left()) / mx - el.scroll_left() as f64;
        let height = caret.height() / my;
        let local_y = if single_line {
            // From the border box: Gecko's `clientHeight` / `clientLeft` for
            // an `<input>` are its inner editor's, padding excluded.
            let content = el.offset_height() as f64 - border_t - border_b - pad_t - pad_b;
            pad_t + (content - height) / 2.0
        } else {
            (caret.top() - m.top()) / my - el.scroll_top() as f64
        };
        // Onto the painted box: a transform on the control or an ancestor
        // scales its layout box into the rect `getBoundingClientRect` reports.
        let e = el.get_bounding_client_rect();
        let (ow, oh) = (el.offset_width() as f64, el.offset_height() as f64);
        let sx = if ow > 0.0 { e.width() / ow } else { 1.0 };
        let sy = if oh > 0.0 { e.height() / oh } else { 1.0 };
        // A textarea's padding box starts at `clientLeft`/`clientTop`, which
        // count a vertical scrollbar on its left (a right-to-left textarea in
        // Chrome); an `<input>` has no scrollbar, and Gecko's `clientLeft` for
        // one is border plus padding, so its origin is the computed border.
        let (origin_x, origin_y) = if single_line {
            (border_l, border_t)
        } else {
            (el.client_left() as f64, el.client_top() as f64)
        };
        let x = e.left() + (origin_x + local_x) * sx;
        let y = e.top() + (origin_y + local_y) * sy;
        Some(rinch_core::reactive::ElementBounds {
            x: x as f32,
            y: y as f32,
            width: 0.0,
            height: (height * sy) as f32,
        })
    }

    /// Whether `node` is shown, asked of the browser (issue #802; see
    /// [`DomDocument::is_rendered`]).
    ///
    /// The same three questions desktop asks, from `getComputedStyle`, which
    /// resolves style on demand: `isConnected`, no `display: none` on the
    /// element or an ancestor (`display` does not inherit, hence the walk), and
    /// the element's own — inherited — `visibility`. An unknown node, or one
    /// with no element to ask, answers `true`, as the trait's default does.
    fn is_rendered(&self, node: NodeId) -> bool {
        let Some(n) = self.nodes.get(&node.0) else {
            return true;
        };
        if !n.is_connected() {
            return false;
        }
        let el = match n.clone().dyn_into::<web_sys::Element>() {
            Ok(el) => el,
            Err(n) => match n.parent_element() {
                Some(el) => el,
                None => return true,
            },
        };
        let Some(win) = web_sys::window() else {
            return true;
        };
        let prop = |el: &web_sys::Element, name: &str| -> Option<String> {
            win.get_computed_style(el)
                .ok()
                .flatten()
                .and_then(|s| s.get_property_value(name).ok())
        };
        if matches!(
            prop(&el, "visibility").as_deref(),
            Some("hidden") | Some("collapse")
        ) {
            return false;
        }
        let mut cur = Some(el);
        while let Some(e) = cur {
            if prop(&e, "display").as_deref() == Some("none") {
                return false;
            }
            cur = e.parent_element();
        }
        true
    }

    /// `document.activeElement`, mapped back through the `__nid` expando
    /// (issue #695).
    ///
    /// An element rinch did not create carries no `__nid`, so focus sitting
    /// outside the mounted root — the page around an island mount, `<body>`
    /// itself — answers `None`. That is the documented meaning of `None`
    /// (*unknown*, not *nothing*), and it is the honest answer: this document
    /// cannot name that node.
    ///
    /// The focused rich-text editor's capture textarea answers the editor's
    /// container (issue #1126): it is how that editor holds the keyboard, and
    /// it sits in `<body>`, outside everything. So an overlay opened from an
    /// editor remembers the editor, and a key entry whose owner holds the
    /// editor counts the focus as inside it.
    fn active_element(&self) -> Option<NodeId> {
        let active = crate::editor_input::effective_active_element(&self.browser_doc)?;
        get_nid(&active)
    }

    /// `showModal()`'s focusing steps (issue #695), answered on the spot —
    /// unlike desktop, which must wait for a layout, the browser lays out on
    /// demand for `getBoundingClientRect` inside the filter below.
    ///
    /// The candidate set is [`trap_focusables`], the **same** set `Tab`
    /// containment cycles, so where an opening dialog puts the keyboard and
    /// where Tab can take it afterwards cannot disagree. `autofocus` is read by
    /// presence, HTML's rule for a boolean attribute, and looked for *within*
    /// that set so an `autofocus` on a hidden or disabled node falls through to
    /// the first real stop.
    ///
    /// The move is **verified** and a refusal steps on, for the reason
    /// `handle_trapped_tab` documents at length: `focus()` is a request the
    /// browser may decline for anything the filter does not know about, and
    /// taking the first candidate on trust would leave an opening dialog with
    /// focus still outside it.
    fn focus_into(&mut self, root: NodeId, policy: rinch_core::dom::FocusIntoPolicy) {
        let Some(root_el) = self
            .nodes
            .get(&root.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let items = crate::event_delegation::trap_focusables(&root_el);
        let autofocus = items.iter().position(|el| el.has_attribute("autofocus"));
        let start = match (autofocus, policy) {
            (Some(i), _) => i,
            (None, rinch_core::dom::FocusIntoPolicy::FirstFocusable) => 0,
            (None, rinch_core::dom::FocusIntoPolicy::AutofocusOnly) => return,
        };
        // Stepping on past a refusal is right for a dialog, which wants *some*
        // stop, and wrong for a popover, which wants that **one** element or
        // none: moving focus to a neighbouring control the author never pointed
        // at is not what `autofocus` asked for, and desktop would not do it.
        let limit = match policy {
            rinch_core::dom::FocusIntoPolicy::FirstFocusable => items.len(),
            rinch_core::dom::FocusIntoPolicy::AutofocusOnly => start + 1,
        };
        // The focus-then-verify loop runs once the caller's borrow is released
        // (`take_after_borrow`): each `focus()` dispatches listeners that touch
        // this document.
        let browser_doc = self.browser_doc.clone();
        let candidates: Vec<web_sys::HtmlElement> =
            items.into_iter().take(limit).skip(start).collect();
        self.after_borrow.push(Box::new(move || {
            for el in candidates {
                let _ = el.focus();
                if browser_doc
                    .active_element()
                    .is_some_and(|a| el.is_same_node(Some(a.unchecked_ref())))
                {
                    return;
                }
            }
        }));
    }

    /// The close half (issue #695): hand the keyboard back to `opener`, or let
    /// it go — with the **browser** as the authority on whether the hand-back
    /// can happen.
    ///
    /// Desktop has to predicate this (is it attached, does it have a box, is it
    /// disabled) because `try_focus_input` answers without asking. Here the same
    /// three refusals — detached, `display: none`, `disabled` — are already the
    /// browser's: `HTMLElement.focus()` on any of them does nothing. So the
    /// shape is focus-then-verify, which is `handle_trapped_tab`'s rule in this
    /// file's sibling module and needs no enumeration to be right.
    ///
    /// The bounding rule is the same on both backends and is *not* the
    /// browser's to give: a claim that has moved **outside** `root` belongs to
    /// the user, and this returns without touching it.
    fn restore_focus(&mut self, opener: Option<NodeId>, root: NodeId) {
        let Some(root_el) = self
            .nodes
            .get(&root.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let opener = opener
            .and_then(|o| self.nodes.get(&o.0))
            .and_then(|n| n.clone().dyn_into::<web_sys::HtmlElement>().ok());
        let browser_doc = self.browser_doc.clone();
        // Decided and done once the caller's borrow is released
        // (`take_after_borrow`): the `focus()`/`blur()` dispatch listeners that
        // touch this document.
        self.after_borrow.push(Box::new(move || {
            restore_focus_now(&browser_doc, &root_el, opener)
        }));
    }

    /// The browser owns the wheel, so `root` gates nothing the user does; it is
    /// recorded for rinch's own programmatic scrolling (a drag's auto-scroll,
    /// [`scroll_locked_out`]). See [`set_page_scroll_locked`].
    fn set_scroll_locked(&mut self, locked: bool, root: NodeId) {
        let el = self
            .nodes
            .get(&root.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok());
        set_page_scroll_locked(locked, root.0, el);
    }

    fn resolve_layout(&mut self, _width: f32, _height: f32) {
        // No-op: browser handles layout natively.
    }

    /// Refresh `node`'s bounds signals from the browser from now on (issue
    /// #1781, `crate::bounds`). A text node has no box and is not watched.
    fn watch_bounds(&mut self, node: NodeId) {
        if let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.dyn_ref::<web_sys::Element>())
        {
            crate::bounds::watch(self.doc_key, node.0, el);
        }
    }

    /// `documentElement.clientWidth/clientHeight` (issue #1764): the layout
    /// viewport less a classic scrollbar, which is the space a dropdown can be
    /// placed in without being hidden under one.
    fn viewport_size(&self) -> Option<(f32, f32)> {
        let el = web_sys::window()?.document()?.document_element()?;
        let (w, h) = (el.client_width(), el.client_height());
        (w > 0 && h > 0).then_some((w as f32, h as f32))
    }

    /// `getBoundingClientRect()` (issue #1779). A disconnected element answers
    /// `None`, as rinch-dom does; the browser gives a zero rect there.
    fn bounding_rect(&mut self, node: NodeId) -> Option<rinch_core::reactive::ElementBounds> {
        let n = self.nodes.get(&node.0)?;
        if !n.is_connected() {
            return None;
        }
        let el: &web_sys::Element = n.dyn_ref()?;
        let r = el.get_bounding_client_rect();
        Some(rinch_core::reactive::ElementBounds {
            x: r.x() as f32,
            y: r.y() as f32,
            width: r.width() as f32,
            height: r.height() as f32,
        })
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        let n = self.nodes.get(&node_id)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let rect = el.get_bounding_client_rect();
        // The trait contract is **immediate-parent-relative** `(x, y)` (the desktop
        // Taffy backend returns this, and the editor's `block_offset_in_container`
        // sums it up the ancestor chain). `offsetLeft/offsetTop` would be
        // *offsetParent*-relative, which double-counts for a nested block (a
        // paragraph inside a blockquote/list, whose offsetParent is the editor
        // container, not its immediate parent). Use border-box differences instead.
        let (px, py) = match el.parent_element() {
            Some(parent) => {
                let pr = parent.get_bounding_client_rect();
                (pr.x(), pr.y())
            }
            None => (0.0, 0.0),
        };
        Some((
            (rect.x() - px) as f32,
            (rect.y() - py) as f32,
            rect.width() as f32,
            rect.height() as f32,
        ))
    }

    fn content_origin_inset(&self, node_id: u64) -> (f32, f32) {
        // clientLeft/clientTop = the node's left/top border width. The editor's overlay
        // container is position:relative with a border, so an absolutely-positioned
        // overlay anchors to its padding box; the block offset (computed from border-box
        // getBoundingClientRect differences) must subtract this to land on the glyphs.
        let Some(n) = self.nodes.get(&node_id) else {
            return (0.0, 0.0);
        };
        // Less its own scroll: an absolute child of a scrolled box is placed
        // against its *scrolled* content, so an editor container that is
        // itself the scroller (`overflow: auto` on the `Editor`) puts its
        // overlays at the unscrolled offset, where the desktop's parent-
        // relative layout already is (#1677).
        match n.clone().dyn_into::<web_sys::Element>() {
            Ok(el) => (
                el.client_left() as f32 - el.scroll_left() as f32,
                el.client_top() as f32 - el.scroll_top() as f32,
            ),
            Err(_) => (0.0, 0.0),
        }
    }

    /// From `getComputedStyle`: a px `line-height`; `None` for `normal`.
    fn line_box_height(&self, node_id: u64) -> Option<f32> {
        let el = self
            .nodes
            .get(&node_id)?
            .clone()
            .dyn_into::<web_sys::Element>()
            .ok()?;
        let cs = web_sys::window()?.get_computed_style(&el).ok().flatten()?;
        let v = cs.get_property_value("line-height").ok()?;
        v.trim()
            .strip_suffix("px")?
            .parse::<f32>()
            .ok()
            .filter(|h| *h > 0.0)
    }

    /// From `getComputedStyle`: the used padding and border widths, in px.
    fn content_box_inset(&self, node_id: u64) -> (f32, f32, f32, f32) {
        let zero = (0.0, 0.0, 0.0, 0.0);
        let Some(el) = self
            .nodes
            .get(&node_id)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return zero;
        };
        let Some(cs) = web_sys::window().and_then(|w| w.get_computed_style(&el).ok().flatten())
        else {
            return zero;
        };
        let px = |p: &str| -> f32 {
            cs.get_property_value(p)
                .ok()
                .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
                .unwrap_or(0.0)
                .max(0.0)
        };
        let side = |s: &str| px(&format!("padding-{s}")) + px(&format!("border-{s}-width"));
        (side("left"), side("top"), side("right"), side("bottom"))
    }

    fn node_font(&self, node_id: u64) -> Option<rinch_core::dom::NodeFont> {
        // `getComputedStyle` so the IME preedit overlay (an absolutely-positioned span
        // in the editor container, which inherits only the container's default font)
        // can match the block it composes into — e.g. a 32px heading, not 16px body.
        let el: web_sys::Element = self.nodes.get(&node_id)?.clone().dyn_into().ok()?;
        let cs = web_sys::window()?.get_computed_style(&el).ok().flatten()?;
        let get = |p: &str| cs.get_property_value(p).unwrap_or_default();
        Some(rinch_core::dom::NodeFont {
            family: get("font-family"),
            size: get("font-size"),
            weight: get("font-weight"),
            style: get("font-style"),
        })
    }

    /// The range starts at the last position at `byte_a` (past an unselected
    /// image there) or before the first selected box, whichever comes first,
    /// and ends at the first position at `byte_b` or after the last selected
    /// box, whichever comes last (#1120): an image has no bytes here either,
    /// so the byte range alone started a selection from right after an image
    /// at the end of the text before it, over the image, and drew an image
    /// selected alone not at all.
    fn query_selection_rects_with_boxes(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
        selected_boxes: &[u64],
    ) -> Vec<(f32, f32, f32, f32)> {
        if byte_a >= byte_b && selected_boxes.is_empty() {
            return Vec::new();
        }
        let Some(n) = self.nodes.get(&node_id) else {
            return Vec::new();
        };
        let boxes: Vec<&web_sys::Node> = selected_boxes
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .collect();
        let precedes = |a: &web_sys::Node, b: &web_sys::Node| {
            b.compare_document_position(a) & web_sys::Node::DOCUMENT_POSITION_PRECEDING != 0
        };
        let resolve = || -> Option<web_sys::DomRectList> {
            let range = self.browser_doc.create_range().ok()?;
            let start = find_text_node_at_byte_offset_latest(n, byte_a);
            match (boxes.first(), start) {
                (Some(first), Some((t, _))) if precedes(first, &t) => {
                    range.set_start_before(first).ok()?
                }
                (_, Some((t, off))) => range.set_start(&t, off).ok()?,
                (Some(first), None) => range.set_start_before(first).ok()?,
                (None, None) => return None,
            }
            // Byte 0 with boxes: the end is after the last selected box (the
            // first text node's start is after every leading image).
            let end = if byte_b == 0 && !boxes.is_empty() {
                None
            } else {
                find_text_node_at_byte_offset(n, byte_b)
            };
            match (boxes.last(), end) {
                (Some(last), Some((t, _))) if precedes(&t, last) => {
                    range.set_end_after(last).ok()?
                }
                (_, Some((t, off))) => range.set_end(&t, off).ok()?,
                (Some(last), None) => range.set_end_after(last).ok()?,
                (None, None) => return None,
            }
            range.get_client_rects()
        };
        let Some(rects) = resolve() else {
            return Vec::new();
        };
        self.block_local_rects(n, &rects)
    }

    fn query_selection_rects(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        // Per-line selection rectangles for the flat UTF-8 byte range `[a, b)` within
        // the block's concatenated inline text, returned **block-local** (relative to
        // the block element's top-left) — the editor view adds the block's container
        // offset itself. Mirrors `query_caret_position`'s coordinate convention.
        if byte_a >= byte_b {
            return Vec::new();
        }
        let Some(n) = self.nodes.get(&node_id) else {
            return Vec::new();
        };
        let resolve = || -> Option<web_sys::DomRectList> {
            let (start_node, start_off) = find_text_node_at_byte_offset(n, byte_a)?;
            let (end_node, end_off) = find_text_node_at_byte_offset(n, byte_b)?;
            let range = self.browser_doc.create_range().ok()?;
            range.set_start(&start_node, start_off).ok()?;
            range.set_end(&end_node, end_off).ok()?;
            range.get_client_rects()
        };
        let Some(rects) = resolve() else {
            return Vec::new();
        };
        self.block_local_rects(n, &rects)
    }
}

impl WebDocument {
    /// `rects` (client px) relative to the block element `n`'s top-left,
    /// zero-area ones dropped: what [`DomDocument::query_selection_rects`]
    /// answers.
    fn block_local_rects(
        &self,
        n: &web_sys::Node,
        rects: &web_sys::DomRectList,
    ) -> Vec<(f32, f32, f32, f32)> {
        let Ok(el) = n.clone().dyn_into::<web_sys::Element>() else {
            return Vec::new();
        };
        let block = el.get_bounding_client_rect();
        let (bx, by) = (block.x(), block.y());
        let mut out = Vec::with_capacity(rects.length() as usize);
        for i in 0..rects.length() {
            if let Some(r) = rects.get(i) {
                // Skip zero-area rects (collapsed line fragments) the browser can emit.
                if r.width() <= 0.0 || r.height() <= 0.0 {
                    continue;
                }
                out.push((
                    (r.x() - bx) as f32,
                    (r.y() - by) as f32,
                    r.width() as f32,
                    r.height() as f32,
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_truthiness_covers_both_emit_conventions() {
        // The rsx! macro stringifies bool closures.
        assert!(attr_is_truthy("true"));
        assert!(!attr_is_truthy("false"));
        // Components use the HTML presence form: an empty string is *present*,
        // hence true (e.g. a `hidden: {|| flag.get()}` binding rendering `""`).
        assert!(attr_is_truthy(""));
        // Other falsey spellings.
        assert!(!attr_is_truthy("0"));
        // Anything else (a stray value on a boolean attribute) is still "on",
        // matching HTML's "present attribute = true" rule.
        assert!(attr_is_truthy("on"));
        assert!(attr_is_truthy("checked"));
    }
}
