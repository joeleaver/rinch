//! Browser-native DOM implementation of the `DomDocument` trait.
//!
//! Instead of painting to a canvas, this implementation creates real browser DOM
//! elements via `web_sys`. The browser handles layout, CSS, text rendering, and
//! painting natively. The reactive system (Signal/Effect) and all components work
//! through NodeHandle -> DomDocument, so everything works automatically.
//!
//! The `DomDocument` impl is one file per supertrait, under `web_document/`:
//! `tree.rs`, `content.rs`, `focus.rs`, `layout.rs` and `scroll.rs`.

mod content;
mod focus;
mod layout;
mod scroll;
mod tree;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::event_delegation::{
    FOCUS_VALUE_PROP, get_expando_string, set_expando, utf16_offset_to_utf8_bytes,
};
use rinch_core::dom::{CaretAffinity, NodeId, attr_is_truthy};
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
    /// Process-unique document identity (see [`DomTree::doc_key`](rinch_core::dom::DomTree::doc_key)).
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
    /// [`DomFocus::take_after_borrow`](rinch_core::dom::DomFocus::take_after_borrow) to run once its borrow is released:
    /// a browser `focus()`/`blur()` runs focus listeners synchronously, and
    /// rinch's own listeners touch this document.
    after_borrow: Vec<Box<dyn FnOnce()>>,
    /// The selection-highlight boxes this document paints on the text or the
    /// cell they cover rather than as boxes (#2667).
    highlights: crate::highlight::OverlayHighlights,
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
    /// [`DomTree::discard_node`](rinch_core::dom::DomTree::discard_node) and `set_inner_html`, and by
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
    // As an `f64`, exact for every id (all are below 2^53, #723): a `u32`
    // would drop the generation half of a packed id.
    let _ = js_sys::Reflect::set(node, &"__nid".into(), &JsValue::from(id.0 as f64));
    NODE_REGISTRY.with(|m| m.borrow_mut().insert(id.0, node.clone()));
}

/// Drop `node` and every `__nid`-tagged descendant of it from both node maps —
/// the document's own `nodes` and the page-global [`NODE_REGISTRY`] (#184).
///
/// Reached only from [`DomTree::discard_node`](rinch_core::dom::DomTree::discard_node) and `set_inner_html`, the two
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
        let mut child = children_of(parent).first_child();
        while let Some(c) = child {
            let next = c.next_sibling();
            forget_recursive(nodes, &mut reg, &c);
            child = next;
        }
    });
}

/// The open shadow root `isolate_styles` attached to `node` (#2357).
fn shadow_of(node: &web_sys::Node) -> Option<web_sys::ShadowRoot> {
    node.dyn_ref::<web_sys::Element>()?.shadow_root()
}

/// Where `node`'s children live: the shadow root `isolate_styles` attached
/// to it (#2357), or the node itself. The children it had before that stay
/// in the light tree, unrendered, and are reached through the browser only.
fn children_of(node: &web_sys::Node) -> web_sys::Node {
    shadow_of(node).map_or_else(|| node.clone(), Into::into)
}

/// `textContent` with open shadow roots' text, each after its host's light
/// children's (#2357): the order desktop's tree has.
fn composed_text(node: &web_sys::Node, out: &mut String) {
    if node.node_type() == web_sys::Node::TEXT_NODE {
        out.push_str(&node.text_content().unwrap_or_default());
        return;
    }
    let mut child = node.first_child();
    while let Some(c) = child {
        composed_text(&c, out);
        child = c.next_sibling();
    }
    if let Some(shadow) = shadow_of(node) {
        composed_text(&shadow, out);
    }
}

/// `node`'s parent, with a shadow root standing for its host: the parent an
/// app's tree has (#2357).
fn parent_of(node: &web_sys::Node) -> Option<web_sys::Node> {
    let parent = node.parent_node()?;
    match parent.dyn_ref::<web_sys::ShadowRoot>() {
        Some(shadow) => Some(shadow.host().into()),
        None => Some(parent),
    }
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
    // And the children an `isolate_styles` element keeps in its shadow root.
    if let Some(shadow) = shadow_of(node) {
        let mut child = shadow.first_child();
        while let Some(c) = child {
            let next = c.next_sibling();
            forget_recursive(nodes, reg, &c);
            child = next;
        }
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
    /// ([`DomLayout::query_inline_box_caret_rect`](rinch_core::dom::DomLayout::query_inline_box_caret_rect)).
    ///
    /// `None` when `br` is not a `<br>` in `block`, or when what follows it is
    /// neither text nor a `<br>` (an image: the byte query answers that). A
    /// node view's inline host (a claimed image) and an `<img>` are answered
    /// from their box: the caret before one at its left edge, after it at its
    /// right, as tall as the box. An `<img>` has no bytes either, so the byte
    /// query drew a caret between two images that a soft wrap separates past
    /// the second one (#1712).
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
        // An image, or a claimed one (a node view's inline-block `<span>`): the
        // caret before it is at its left edge, after it at its right, as tall
        // as its box.
        if is_node_view_host(br) || el.tag_name().eq_ignore_ascii_case("img") {
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
            highlights: Default::default(),
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
            highlights: Default::default(),
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
        // Recurse into children (an `isolate_styles` element's, in its
        // shadow root).
        let children = children_of(node).child_nodes();
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
    /// Release this document's remaining `NODE_REGISTRY` entries (#184).
    ///
    /// `remove_node` already prunes churned nodes; this catches whatever a live
    /// document was still holding — most importantly its own root/body wrappers,
    /// which no `remove_node` ever reaches. `self.nodes` *is* this document's
    /// registry footprint: the two maps are populated in lockstep by the same
    /// call sites, which is why the registry needs no per-document keying.
    /// Removing an id the registry no longer holds is a no-op, which is what
    /// makes this safe even when another document's prune already swept a node
    /// of ours out of the page-global map (see `forget_recursive`).
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
        // The highlight registry is the page's, not this document's (#2667).
        self.highlights.clear();
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

/// [`DomFocus::restore_focus`](rinch_core::dom::DomFocus::restore_focus)'s browser half, run with no document
/// borrowed ([`DomFocus::take_after_borrow`](rinch_core::dom::DomFocus::take_after_borrow)).
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
/// [`DomFocus::moves_keeping_focus`](rinch_core::dom::DomFocus::moves_keeping_focus) gives the same answer to the
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

/// The DOM range of the flat byte range `[byte_a, byte_b)` of the block `n`
/// with the zero-byte leaves `selected_boxes` (host ids) it holds: what
/// [`DomLayout::query_selection_rects_with_boxes`](rinch_core::dom::DomLayout::query_selection_rects_with_boxes) measures, and what the
/// selection highlight paints (#2667).
///
/// The range starts at the last position at `byte_a` (past an unselected
/// image there) or before the first selected box, whichever comes first,
/// and ends at the first position at `byte_b` or after the last selected
/// box, whichever comes last (#1120).
pub(crate) fn selection_range(
    browser_doc: &web_sys::Document,
    nodes: &HashMap<u64, web_sys::Node>,
    n: &web_sys::Node,
    byte_a: usize,
    byte_b: usize,
    selected_boxes: &[u64],
) -> Option<web_sys::Range> {
    let boxes: Vec<&web_sys::Node> = selected_boxes
        .iter()
        .filter_map(|id| nodes.get(id))
        .collect();
    let precedes = |a: &web_sys::Node, b: &web_sys::Node| {
        b.compare_document_position(a) & web_sys::Node::DOCUMENT_POSITION_PRECEDING != 0
    };
    let range = browser_doc.create_range().ok()?;
    let start = find_text_node_at_byte_offset_latest(n, byte_a);
    match (boxes.first(), start) {
        (Some(first), Some((t, _))) if precedes(first, &t) => range.set_start_before(first).ok()?,
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
        (Some(last), Some((t, _))) if precedes(&t, last) => range.set_end_after(last).ok()?,
        (_, Some((t, off))) => range.set_end(&t, off).ok()?,
        (Some(last), None) => range.set_end_after(last).ok()?,
        (None, None) => return None,
    }
    Some(range)
}

impl WebDocument {
    /// `rects` (client px) relative to the block element `n`'s top-left,
    /// zero-area ones dropped: what [`DomLayout::query_selection_rects`](rinch_core::dom::DomLayout::query_selection_rects)
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
