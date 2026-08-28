//! Browser-native DOM implementation of the `DomDocument` trait.
//!
//! Instead of painting to a canvas, this implementation creates real browser DOM
//! elements via `web_sys`. The browser handles layout, CSS, text rendering, and
//! painting natively. The reactive system (Signal/Effect) and all components work
//! through NodeHandle -> DomDocument, so everything works automatically.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::event_delegation::{
    FOCUS_VALUE_PROP, get_expando_string, set_expando, utf16_offset_to_utf8_bytes,
};
use rinch_core::dom::{DomDocument, GlyphBounds, NodeId};
use rinch_editable::RewriteDiff;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Process-global NodeId counter. Shared across all `WebDocument`s so that
/// multiple coexisting roots (islands) never reuse a NodeId — keeping each
/// node's `__nid` unambiguous even when several documents share the page.
static NEXT_NODE_ID: AtomicUsize = AtomicUsize::new(0);

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
    nodes: HashMap<usize, web_sys::Node>,
    /// The root wrapper element (`<div id="rinch-root">`), or the adopted host
    /// element for an island mount (see [`WebDocument::new_into`]).
    root_id: NodeId,
    /// The body wrapper element (`<div id="rinch-body">`).
    body_id: NodeId,
}

thread_local! {
    /// Reverse map (`NodeId.0` → browser node) populated for every node `set_nid`
    /// tags. Lets non-`DomDocument` code — the editor input glue — resolve a rinch
    /// node id back to its DOM node for caret/selection geometry.
    ///
    /// The value is a **strong** `web_sys::Node`, so an entry pins its browser
    /// node against GC. Entries are therefore pruned as soon as the backend stops
    /// owning the node: by [`forget_subtree`] from `remove_node`, `replace_node`
    /// and `set_inner_html`, and by [`Drop for WebDocument`](WebDocument) for
    /// whatever a dying document still holds (issue #184). Node *creation* is not
    /// structural — a keyed `for`, a `show`, or the editor's per-keystroke
    /// `ViewDesc` churn creates and destroys nodes forever within one mounted
    /// root — so without pruning this map grows without bound for the life of the
    /// page.
    ///
    /// [`node_by_nid`] additionally returns `None` for a node that is present but
    /// detached, so its callers cannot tell a pruned id from a detached one.
    static NODE_REGISTRY: std::cell::RefCell<HashMap<usize, web_sys::Node>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Resolve a rinch `NodeId.0` to its live browser node — the editor glue uses this to
/// read caret/selection geometry for a model position. `None` if unknown or detached.
pub(crate) fn node_by_nid(nid: usize) -> Option<web_sys::Node> {
    NODE_REGISTRY
        .with(|m| m.borrow().get(&nid).cloned())
        .filter(|n| n.is_connected())
}

/// Set the `__nid` JS property on a browser node for reverse lookups.
fn set_nid(node: &web_sys::Node, id: NodeId) {
    let _ = js_sys::Reflect::set(node, &"__nid".into(), &JsValue::from(id.0 as u32));
    NODE_REGISTRY.with(|m| m.borrow_mut().insert(id.0, node.clone()));
}

/// Drop `node` and every `__nid`-tagged descendant of it from both node maps —
/// the document's own `nodes` and the page-global [`NODE_REGISTRY`] (#184).
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
fn forget_subtree(nodes: &mut HashMap<usize, web_sys::Node>, node: &web_sys::Node) {
    // One borrow for the whole recursion. Nothing inside it may call `set_nid`,
    // which borrows the registry mutably.
    NODE_REGISTRY.with(|m| {
        let mut reg = m.borrow_mut();
        forget_recursive(nodes, &mut reg, node);
    });
}

fn forget_recursive(
    nodes: &mut HashMap<usize, web_sys::Node>,
    reg: &mut HashMap<usize, web_sys::Node>,
    node: &web_sys::Node,
) {
    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i) {
            forget_recursive(nodes, reg, &child);
        }
    }
    if let Some(id) = get_nid(node) {
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
pub fn __node_registry_contains(nid: usize) -> bool {
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
            } else if let Some(textarea) = node.dyn_ref::<web_sys::HtmlTextAreaElement>()
                && textarea.value() != value
            {
                write_live_text(&TextControl::TextArea(textarea), value);
            } else if let Some(select) = node.dyn_ref::<web_sys::HtmlSelectElement>()
                && select.value() != value
            {
                select.set_value(value);
            }
        }
        "checked" => {
            if let Some(input) = node.dyn_ref::<web_sys::HtmlInputElement>() {
                let on = attr_is_truthy(value);
                if input.checked() != on {
                    input.set_checked(on);
                }
            }
        }
        "indeterminate" => {
            if let Some(input) = node.dyn_ref::<web_sys::HtmlInputElement>() {
                let on = attr_is_truthy(value);
                if input.indeterminate() != on {
                    input.set_indeterminate(on);
                }
            }
        }
        "selected" => {
            if let Some(option) = node.dyn_ref::<web_sys::HtmlOptionElement>() {
                let on = attr_is_truthy(value);
                if option.selected() != on {
                    option.set_selected(on);
                }
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

/// Truthiness for HTML boolean-ish attributes as rinch emits them.
///
/// rinch writes these via two conventions: the `rsx!` macro stringifies a `bool`
/// closure to `"true"`/`"false"`, while components set the bare presence form
/// (`""`, matching HTML where a present boolean attribute is true regardless of
/// value). Treat everything as "on" except the explicit falsey strings so both
/// conventions round-trip — in particular an empty string means *present* (true).
fn attr_is_truthy(value: &str) -> bool {
    !matches!(value, "false" | "0")
}

/// Get the `__nid` JS property from a browser node.
pub(crate) fn get_nid(node: &web_sys::Node) -> Option<NodeId> {
    js_sys::Reflect::get(node, &"__nid".into())
        .ok()
        .and_then(|v| v.as_f64())
        .map(|n| NodeId(n as usize))
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
    /// Create a new WebDocument backed by the browser's document.
    ///
    /// Creates a `<div id="rinch-root">` as root and `<div id="rinch-body">`
    /// as body, appending root to `document.body()`.
    pub fn new(browser_doc: web_sys::Document) -> Self {
        let mut doc = Self {
            doc_key: rinch_core::dom::next_doc_key(),
            browser_doc,
            nodes: HashMap::new(),
            root_id: NodeId(0),
            body_id: NodeId(0),
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
        let mut doc = Self {
            doc_key: rinch_core::dom::next_doc_key(),
            browser_doc,
            nodes: HashMap::new(),
            root_id: NodeId(0),
            body_id: NodeId(0),
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
    pub fn __contains(&self, nid: usize) -> bool {
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
            p.append_child(c).ok();
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
            p.insert_before(c, Some(r)).ok();
        }
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        let (Some(old_node), Some(new_node)) =
            (self.nodes.get(&old.0).cloned(), self.nodes.get(&new.0))
        else {
            return;
        };
        let Some(parent) = old_node.parent_node() else {
            return;
        };
        if parent.replace_child(new_node, &old_node).is_ok() {
            // `old` is orphaned here and every caller drops its handle in the
            // same breath (the editor's `ViewDesc` diff is a per-keystroke churn
            // path), so holding its entries would leak it for the life of the
            // page (#184). Gated on success: a rejected swap leaves `old` in the
            // tree, and a node still in the tree must keep its entries.
            forget_subtree(&mut self.nodes, &old_node);
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        let Some(n) = self.nodes.get(&node.0).cloned() else {
            return;
        };
        if let Some(parent) = n.parent_node() {
            parent.remove_child(&n).ok();
        }
        // Prune *unconditionally*, outside the `parent_node()` guard: a node that
        // was built and never appended (or is already detached) is stranded just
        // as hard as an attached one. Descendants stay attached to `n` when `n`
        // leaves its parent, so the sweep works either side of the detach.
        forget_subtree(&mut self.nodes, &n);
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            n.set_text_content(Some(text));
            // A <textarea>'s child text is its *default value*, which the browser
            // stops mirroring onto the live `value` property once the control is
            // user-edited — the same dirty-flag asymmetry `set_attribute("value")`
            // handles for other controls (issue #100). So when a textarea's text
            // changes — set directly on it, or (the reactive `textarea { {|| … } }`
            // case) on one of its child text nodes — re-sync its value property.
            let textarea = if let Some(ta) = n.dyn_ref::<web_sys::HtmlTextAreaElement>() {
                Some(ta.clone())
            } else if let Some(parent) = n.parent_node() {
                parent.dyn_into::<web_sys::HtmlTextAreaElement>().ok()
            } else {
                None
            };
            if let Some(ta) = textarea {
                let content = ta.text_content().unwrap_or_default();
                if ta.value() != content {
                    ta.set_value(&content);
                }
            }
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            if let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                match name {
                    // Boolean content attributes follow HTML *presence* semantics:
                    // a present attribute is true regardless of its string value.
                    // The rsx macro stringifies a `bool` closure to `"true"`/
                    // `"false"`, so writing the literal string would leave
                    // `checked="false"` *present* — meaning `defaultChecked` stays
                    // true, `[checked]` selectors match, and `form.reset()` would
                    // re-check it, all contradicting the property synced below.
                    // Mirror truthiness onto presence instead.
                    "checked" | "selected" => {
                        if attr_is_truthy(value) {
                            el.set_attribute(name, "").ok();
                        } else {
                            el.remove_attribute(name).ok();
                        }
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
                    _ => {
                        el.set_attribute(name, value).ok();
                    }
                }
            }
            // A handful of attributes are reflected *asymmetrically* onto a live
            // DOM property on form controls (`value`, `checked`, `selected`,
            // `indeterminate`). Writing the attribute alone only sets the
            // control's *default*: the moment the user edits the control it goes
            // "dirty" and the browser stops mirroring the attribute into the
            // property. A reactive `value:`/`checked:` binding would then silently
            // stop updating what is displayed. Mirror the property too so
            // signal-driven updates keep working after the control has been typed
            // into / toggled (issue #100).
            sync_reflected_property(n, name, value);
        }
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            if let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                // Removing `value` is a write of `""`: property first (for the
                // same pristine-control mirroring reason as in `set_attribute`)
                // and deferred during an IME composition, so both spellings of
                // "clear this field" obey one policy (issue #238).
                if name == "value" {
                    remove_value_attribute(&el);
                    return;
                }
                el.remove_attribute(name).ok();
            }
            // Keep the reflected property in sync when the attribute is removed,
            // otherwise a dirtied control keeps showing the stale property (#100).
            match name {
                "checked" | "selected" | "indeterminate" => {
                    sync_reflected_property(n, name, "false")
                }
                _ => {}
            }
        }
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        el.get_attribute(name)
    }

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
        let el = self.browser_doc.query_selector(selector).ok()??;
        let node: web_sys::Node = el.into();
        get_nid(&node)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        let mut result = Vec::new();
        if let Ok(node_list) = self.browser_doc.query_selector_all(selector) {
            for i in 0..node_list.length() {
                if let Some(node) = node_list.item(i)
                    && let Some(nid) = get_nid(&node)
                {
                    result.push(nid);
                }
            }
        }
        result
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
            let children = p.child_nodes();
            if index < children.length() as usize {
                if let Some(ref_node) = children.item(index as u32) {
                    p.insert_before(c, Some(&ref_node)).ok();
                } else {
                    p.append_child(c).ok();
                }
            } else {
                p.append_child(c).ok();
            }
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

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            // `set_inner_html` discards every existing child, so forget them
            // first — the desktop backend already does this (issue #184).
            let old_children = el.child_nodes();
            for i in 0..old_children.length() {
                if let Some(child) = old_children.item(i) {
                    forget_subtree(&mut self.nodes, &child);
                }
            }
            el.set_inner_html(html);
            // Walk all new child nodes and register them
            let children = el.child_nodes();
            for i in 0..children.length() {
                if let Some(child) = children.item(i) {
                    self.register_subtree(&child);
                }
            }
        }
    }

    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)> {
        let n = self.nodes.get(&(node_id as usize))?;
        let (text_node, utf16_offset) = find_text_node_at_byte_offset(n, byte_offset)?;
        let range = self.browser_doc.create_range().ok()?;
        range.set_start(&text_node, utf16_offset).ok()?;
        range.set_end(&text_node, utf16_offset).ok()?;
        let rect = range.get_bounding_client_rect();
        // Get the block element's rect to compute relative coordinates
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        let block_rect = el.get_bounding_client_rect();
        Some((
            (rect.x() - block_rect.x()) as f32,
            (rect.y() - block_rect.y()) as f32,
        ))
    }

    fn query_glyph_bounds(&self, node_id: u64, byte_offset: usize) -> Option<GlyphBounds> {
        let n = self.nodes.get(&(node_id as usize))?;
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

    fn focus_element(&mut self, node_id: NodeId) {
        if let Some(n) = self.nodes.get(&node_id.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>()
        {
            el.focus().ok();
        }
    }

    fn resolve_layout(&mut self, _width: f32, _height: f32) {
        // No-op: browser handles layout natively.
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        let n = self.nodes.get(&(node_id as usize))?;
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
        let Some(n) = self.nodes.get(&(node_id as usize)) else {
            return (0.0, 0.0);
        };
        match n.clone().dyn_into::<web_sys::Element>() {
            Ok(el) => (el.client_left() as f32, el.client_top() as f32),
            Err(_) => (0.0, 0.0),
        }
    }

    fn node_font(&self, node_id: u64) -> Option<rinch_core::dom::NodeFont> {
        // `getComputedStyle` so the IME preedit overlay (an absolutely-positioned span
        // in the editor container, which inherits only the container's default font)
        // can match the block it composes into — e.g. a 32px heading, not 16px body.
        let el: web_sys::Element = self
            .nodes
            .get(&(node_id as usize))?
            .clone()
            .dyn_into()
            .ok()?;
        let cs = web_sys::window()?.get_computed_style(&el).ok().flatten()?;
        let get = |p: &str| cs.get_property_value(p).unwrap_or_default();
        Some(rinch_core::dom::NodeFont {
            family: get("font-family"),
            size: get("font-size"),
            weight: get("font-weight"),
            style: get("font-style"),
        })
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
        let Some(n) = self.nodes.get(&(node_id as usize)) else {
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
        // hence true (e.g. `input.set_attribute("checked", "")`).
        assert!(attr_is_truthy(""));
        // Other falsey spellings.
        assert!(!attr_is_truthy("0"));
        // Anything else (a stray value on a boolean attribute) is still "on",
        // matching HTML's "present attribute = true" rule.
        assert!(attr_is_truthy("on"));
        assert!(attr_is_truthy("checked"));
    }
}
