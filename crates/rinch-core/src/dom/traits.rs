//! DOM traits: IntoNode, DomDocument, GlyphBounds.

use super::{NodeHandle, NodeId, RenderScope};

/// Trait for converting values into DOM nodes.
///
/// This trait enables the rsx! macro to handle both NodeHandle returns
/// (from component functions) and text values in a uniform way.
pub trait IntoNode {
    /// Convert this value into a DOM node.
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle;
}

impl IntoNode for NodeHandle {
    #[inline]
    fn into_node(self, _scope: &mut RenderScope) -> NodeHandle {
        self
    }
}

impl IntoNode for String {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(&self)
    }
}

impl IntoNode for &str {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self)
    }
}

impl IntoNode for &String {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self.as_str())
    }
}

impl IntoNode for std::borrow::Cow<'_, str> {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self.as_ref())
    }
}

impl IntoNode for Vec<NodeHandle> {
    /// Append all NodeHandles as children of a transparent wrapper.
    ///
    /// This enables the `.iter().map(|x| rsx!{...}).collect::<Vec<_>>()` pattern in RSX.
    /// For reactive lists that update when signals change, prefer `for` loops with keyed
    /// reconciliation instead.
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        let container = scope.create_element("div");
        container.set_attribute("style", "display:contents");
        for child in &self {
            container.append_child(child);
        }
        container
    }
}

impl IntoNode for Option<NodeHandle> {
    /// A present-or-absent child: `Some(node)` renders the node, `None` renders
    /// nothing. Routed through the `Vec<NodeHandle>` impl (a 0-or-1 list) so a
    /// `maybe.map(|t| rsx! { … })` embeds directly as `{maybe}` — no
    /// `.into_iter().collect::<Vec<_>>()` dance. For a child that toggles
    /// reactively, prefer an `if`/`if let` in rsx instead.
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        self.into_iter()
            .collect::<Vec<NodeHandle>>()
            .into_node(scope)
    }
}

// Implement IntoNode for numeric types
macro_rules! impl_into_node_for_display {
    ($($ty:ty),*) => {
        $(
            impl IntoNode for $ty {
                #[inline]
                fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
                    scope.create_text(&self.to_string())
                }
            }
        )*
    };
}

impl_into_node_for_display!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char
);

/// Which side of a soft line wrap a text caret belongs to (#301).
///
/// The end of one visual line and the start of the next are one text position,
/// so a caret there could be drawn in either place. `Downstream` — the default —
/// draws it at the start of the lower line; `Upstream` at the end of the upper
/// one. Anywhere but a soft wrap the two are the same caret. The rich-text
/// editor keeps the hint beside its selection
/// (`EditorHandle::caret_affinity`); Parley calls the same thing
/// `parley::Affinity`, CodeMirror `SelectionRange.assoc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum CaretAffinity {
    /// At a soft wrap, the start of the lower line.
    #[default]
    Downstream,
    /// At a soft wrap, the end of the upper line.
    Upstream,
}

/// Bounding box for a glyph cluster.
#[derive(Debug, Clone, Copy)]
pub struct GlyphBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The resolved font of a node, as CSS-ready strings — used by the IME preedit
/// overlay so the composing text matches the text it composes into (a large
/// heading vs. body text). Each field is a CSS value (e.g. `size = "32px"`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeFont {
    pub family: String,
    pub size: String,
    pub weight: String,
    pub style: String,
}

/// What [`DomDocument::focus_into`] should do when the subtree has no
/// `autofocus` descendant (issue #695).
///
/// The two values are the two things browsers actually do, and rinch's three
/// overlays are split between them: a `<dialog>` opened with `showModal()`
/// focuses its first focusable whether or not anything asked for it, while an
/// `auto` popover moves focus **only** for an `autofocus` element and otherwise
/// leaves the keyboard where it is. `Modal` and `Drawer` take the first;
/// `Popover` takes the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusIntoPolicy {
    /// Fall back to the first focusable descendant — a modal dialog.
    FirstFocusable,
    /// Move nothing unless something inside asked for focus — a popover.
    AutofocusOnly,
}

/// Allocate a fresh process-unique document key for [`DomDocument::doc_key`].
///
/// Call once per document at construction and store the result. Monotonic and
/// never reused, so it is immune to allocator address reuse (unlike keying by
/// `Rc` pointer).
pub fn next_doc_key() -> u64 {
    static NEXT_DOC_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT_DOC_KEY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Trait for DOM documents that support mutation operations.
///
/// This trait abstracts the DOM mutation API, allowing different
/// implementations (rinch-dom, web-sys, etc.) to be used.
///
/// # Required Operations
///
/// Implementors must provide:
/// - Node creation: [`create_element`](DomDocument::create_element), [`create_text`](DomDocument::create_text)
/// - Tree mutation: [`append_child`](DomDocument::append_child), [`remove_child`](DomDocument::remove_child), etc.
/// - Attribute mutation: [`set_attribute`](DomDocument::set_attribute), [`remove_attribute`](DomDocument::remove_attribute)
/// - Text mutation: [`set_text_content`](DomDocument::set_text_content)
/// - Dirty tracking: [`mark_dirty`](DomDocument::mark_dirty), [`take_dirty_nodes`](DomDocument::take_dirty_nodes)
pub trait DomDocument {
    /// A process-unique identity for this document instance.
    ///
    /// Node ids are per-document slab indices, so two documents on one thread
    /// (e.g. two embedded `RinchContext`s, issue #134) contain the same ids
    /// `0, 1, 2, …`. Thread-local registries that key state by node id must
    /// scope it with this key to avoid cross-document collisions (element
    /// bounds signals, the editor registry, pending focus requests).
    ///
    /// Implementations should allocate it once at construction from
    /// [`next_doc_key`] — never reuse a key, even for a document at the same
    /// address (an `Rc` pointer is not a substitute: addresses can be reused).
    fn doc_key(&self) -> u64;

    /// Create a new element node with the given tag name.
    fn create_element(&mut self, tag: &str) -> NodeId;

    /// Create a new text node with the given content.
    fn create_text(&mut self, text: &str) -> NodeId;

    /// Create a comment node (useful for markers).
    fn create_comment(&mut self, text: &str) -> NodeId;

    /// Append a child node to a parent element.
    fn append_child(&mut self, parent: NodeId, child: NodeId);

    /// Remove a child node from a parent element.
    fn remove_child(&mut self, parent: NodeId, child: NodeId);

    /// Insert a child before a reference node.
    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId);

    /// Replace a node with another node.
    ///
    /// `old` is **detached**, not retired: it keeps its identity and its own
    /// subtree, and a handle to it may be inserted again. Say you are finished
    /// with it by calling [`discard_node`](Self::discard_node) — see the
    /// post-condition table there.
    fn replace_node(&mut self, old: NodeId, new: NodeId);

    /// Remove a node from its parent, **leaving it re-insertable**.
    ///
    /// The node and its whole subtree keep their identity: a [`NodeHandle`] for
    /// any of them may be appended, inserted, read or styled afterwards, and
    /// appending the removed node again puts the whole subtree back exactly as
    /// it was. That is the post-condition every backend owes (issue #719), and
    /// it is what a reactive branch that re-shows a **captured** handle rests on
    /// — `show_dom` and `match_dom` both do, and so does app code that stashes a
    /// `NodeHandle` and toggles it.
    ///
    /// The backend therefore still owns this node's bookkeeping. A caller that
    /// is finished with the subtree for good must say so with
    /// [`discard_node`](Self::discard_node), or it is kept alive for the life of
    /// the document.
    ///
    /// [`NodeHandle`]: super::NodeHandle
    fn remove_node(&mut self, node: NodeId);

    /// Remove a node **and release the backend's bookkeeping** for it and every
    /// descendant — the caller is finished with the subtree for good.
    ///
    /// This is the only route on which a backend **may** retire an id. What it
    /// is allowed to do is bounded from one side only: a `discard_node` is at
    /// least a [`remove_node`](Self::remove_node), and may be as much as
    /// dropping the subtree's bookkeeping entirely. Nothing stronger is
    /// promised, and **nothing weaker may be relied on**: build a fresh node
    /// rather than re-attaching a discarded one, because the backend that does
    /// retire will make that a silent no-op.
    ///
    /// Where a backend does retire, it does so *silently*: every operation on a
    /// retired id does nothing, never panics, and never writes to some other
    /// node (no backend re-issues an id it retired — see below).
    ///
    /// # When to call it rather than [`remove_node`](Self::remove_node)
    ///
    /// Call it wherever the caller *knows* it is throwing the subtree away and
    /// the handle dies in the same breath: a keyed list dropping a row, a pool
    /// shrinking, a component re-render replacing its own output. Do **not**
    /// call it where the same handle may be shown again — a branch helper
    /// toggling a captured node — which is the whole of issue #719.
    ///
    /// Reaching for `remove_node` when `discard_node` was meant costs memory;
    /// reaching for `discard_node` when `remove_node` was meant costs the
    /// subtree. Neither is reported, so the choice is the caller's to make
    /// deliberately.
    ///
    /// # What each backend actually reclaims
    ///
    /// | backend | `remove_node` | `discard_node` |
    /// |---|---|---|
    /// | `rinch-dom` (desktop) | detach; the slab entry stays | **the same** — the default below, which reclaims nothing. A discarded node there still re-inserts, still keeps its subtree, still takes writes |
    /// | `rinch-web` | detach; both node maps keep the node | drops the node and every descendant from the document map **and** the page-global registry (issue #184), releasing the `web_sys` node it was pinning against GC |
    /// | `MockDomDocument` | detach | retires, like the browser |
    ///
    /// **`remove_node`'s post-condition is identical on both; `discard_node`'s
    /// is not.** Desktop's is strictly weaker — measured, not argued: after
    /// `discard_node(panel)` on a `RinchDocument`, `append_child` puts the panel
    /// back with its children and `set_attribute` writes through. Its slab is
    /// per-document and dies with the document, and freeing the entry would
    /// recycle the id (issue #304); that the slab therefore only grows is
    /// **issue #723**.
    ///
    /// So the contract above is deliberately one-sided, and this is the same
    /// shape as issue #719 one verb along: a rule asserted once, honoured by one
    /// backend, failing in the direction "works on desktop, dies on web". What
    /// keeps that from being a trap is [`MockDomDocument`](super::mock), which
    /// retires like the browser, so a caller that re-attaches a discarded handle
    /// fails `cargo test` on the host rather than only in a browser.
    ///
    /// # A retired id is never re-issued
    ///
    /// No backend hands a **discarded** id to a later node: `rinch-web`'s
    /// counter is a monotonic `fetch_add` with no free list, and `rinch-dom`
    /// frees nothing on this route. So a stale *discard* handle can only ever
    /// name nothing, never somebody else, and registries keyed by node id
    /// (focus, the mounted-editor registry) cannot be aimed at the wrong node by
    /// a discard.
    ///
    /// That is a claim about **this route only**. `rinch-dom` does free slab
    /// keys elsewhere — `set_inner_html` and pseudo-element pruning on restyle
    /// both reach `NodeTree::remove_subtree` — and `slab::Slab` has a free list,
    /// so an id freed *that* way is handed to a different node (measured:
    /// `NodeId(4)`). #304's recycled-slot hazard is live on desktop today,
    /// independently of this method, and belongs to #723's audit.
    ///
    /// The default is `remove_node`: correct but reclaiming nothing, which is
    /// what a backend whose bookkeeping dies with the document wants.
    fn discard_node(&mut self, node: NodeId) {
        self.remove_node(node);
    }

    /// Set the text content of a node.
    fn set_text_content(&mut self, node: NodeId, text: &str);

    /// Set an attribute on an element.
    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str);

    /// Remove an attribute from an element.
    fn remove_attribute(&mut self, node: NodeId, name: &str);

    /// Get an attribute value.
    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String>;

    /// The text a form control **holds right now** — what the user sees and
    /// what the next keystroke edits — as opposed to its `value` *content
    /// attribute* (issue #238).
    ///
    /// The two are the same thing on desktop and different things in a
    /// browser, and a component that reads the attribute to ask "what does the
    /// field say?" gets the wrong answer on the web the moment the user types:
    ///
    /// - **Desktop** (`RinchDocument`) keeps them equal by construction. The
    ///   runtime mirrors the edited text into the `value` attribute *before* it
    ///   dispatches `oninput`, and adopts a programmatic write to the focused
    ///   field back into the text engine (#287). So the default — the `value`
    ///   attribute — is the live text, and desktop does not override it.
    /// - **Web** (`rinch-web`) answers from the element's `.value` **property**
    ///   for `<input>`, `<textarea>` and `<select>`. There the attribute holds
    ///   only what was last written programmatically: typing moves the property
    ///   and leaves the attribute behind. Any other element answers from the
    ///   attribute, as the default does.
    ///
    /// **What it is for:** a controlled component's `value_fn` effect asks it
    /// whether the field already shows the value it is about to write, and
    /// writes nothing if so. That echo — keystroke → `oninput` → signal →
    /// effect → the same text written back — is the common case, and skipping
    /// it keeps the write off the focused field altogether. It also lets a
    /// component tell "the author's text still denotes this value" without
    /// keeping its own record of every keystroke, which is what `ColorPicker`
    /// and `ColorInput` did until this existed (GH #231, #235).
    ///
    /// `None` means the node carries no value this backend can name — a
    /// desktop control with no `value` attribute yet, or a node id that no
    /// longer exists. A web form control always answers `Some`, `""` when
    /// empty, because the property always has a value. A caller comparing
    /// against the text it is about to write should therefore treat `None` as
    /// "differs" and write, which is what the empty-string case costs.
    fn live_value(&self, node: NodeId) -> Option<String> {
        self.get_attribute(node, "value")
    }

    /// Set a CSS style property on an element.
    fn set_style(&mut self, node: NodeId, property: &str, value: &str);

    /// Set multiple CSS style properties on an element in a single operation.
    /// More efficient than calling `set_style` multiple times because it only
    /// parses the style string once.
    fn set_styles(&mut self, node: NodeId, properties: &[(&str, &str)]) {
        // Default implementation falls back to calling set_style for each property.
        for &(property, value) in properties {
            self.set_style(node, property, value);
        }
    }

    /// Mark a node as dirty (needs re-layout).
    fn mark_dirty(&mut self, node: NodeId);

    /// Get and clear the set of dirty nodes.
    fn take_dirty_nodes(&mut self) -> Vec<NodeId>;

    /// Get the root node of the document.
    fn root(&self) -> NodeId;

    /// Get the body element of the document.
    fn body(&self) -> NodeId;

    /// Query selector to find a node.
    fn query_selector(&self, selector: &str) -> Option<NodeId>;

    /// Query selector to find all matching nodes.
    fn query_selector_all(&self, selector: &str) -> Vec<NodeId>;

    /// Get the children of a node.
    fn get_children(&self, node: NodeId) -> Vec<NodeId>;

    /// Insert a child at a specific index.
    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize);

    /// Get the parent of a node.
    fn parent_node(&self, node: NodeId) -> Option<NodeId>;

    /// Get the next sibling of a node.
    fn next_sibling(&self, node: NodeId) -> Option<NodeId>;

    /// Parse HTML string and create DOM nodes, returning the root node ID.
    /// Returns None if the HTML couldn't be parsed.
    fn parse_html(&mut self, html: &str) -> Option<NodeId>;

    /// Set the scroll position of an element.
    /// For elements with overflow: auto/scroll, this sets the scroll offset.
    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64);

    /// Replace the children of an element by parsing an HTML string.
    ///
    /// All existing children are removed and replaced with the DOM tree
    /// produced by parsing `html`. This provides an atomic update path
    /// that avoids incremental mutations which can leave the document
    /// in an inconsistent state.
    ///
    /// The discarded children are **retired** like a
    /// [`remove_node`](Self::remove_node): handles held for them must not be
    /// re-attached or written to afterwards.
    fn set_inner_html(&mut self, node: NodeId, html: &str);

    /// Query the screen position of a text caret at the given byte offset.
    ///
    /// Returns the (x, y) coordinates where a text cursor would be rendered
    /// at the specified byte offset within the text node.
    ///
    /// # Arguments
    /// * `node_id` - The ID of the text node or element containing text
    /// * `byte_offset` - The UTF-8 byte offset within the text content
    ///
    /// # Returns
    /// Some((x, y)) if the node has text layout and the offset is valid, None otherwise
    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)>;

    /// Query the bounding box of a glyph cluster at the given byte offset.
    ///
    /// Returns the bounding box of the glyph cluster containing the specified
    /// byte offset within the text node.
    ///
    /// # Arguments
    /// * `node_id` - The ID of the text node or element containing text
    /// * `byte_offset` - The UTF-8 byte offset within the text content
    ///
    /// # Returns
    /// Some(GlyphBounds) if the node has text layout and the offset is valid, None otherwise
    fn query_glyph_bounds(&self, node_id: u64, byte_offset: usize) -> Option<GlyphBounds>;

    /// Focus a specific element programmatically.
    ///
    /// This sets the element as the currently focused element, allowing it to
    /// receive keyboard input. For input/textarea elements, this enables text input.
    ///
    /// # Arguments
    /// * `node_id` - The ID of the element to focus
    fn focus_element(&mut self, node_id: NodeId);

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

    /// Whether `node` is still attached to this document's tree (issue #695).
    ///
    /// Part of the close half of an overlay's focus restore: a `Modal` opened
    /// from a row the dialog itself deleted must not hand focus back into a
    /// detached subtree. It is only *part* of it — being attached is not being
    /// focusable, and [`restore_focus`](Self::restore_focus) asks both.
    ///
    /// One implementation, the default: a walk from `node` up
    /// [`parent_node`](Self::parent_node) to [`root`](Self::root). Both backends
    /// use it — on `rinch-web` the walk stops at the first parent with no
    /// `__nid`, so a detached subtree and the page outside the mounted root both
    /// answer `false`, which is what the browser's own `isConnected` would say
    /// for anything rinch can name.
    ///
    /// **This is a liveness test, not an identity test.** A node id that was
    /// freed and handed to a *different*, attached node answers `true` — the
    /// recycled-slot hazard of issue #304, which is live on desktop through
    /// `NodeTree::remove_subtree` and is not created or cured here.
    fn is_connected(&self, node: NodeId) -> bool {
        let root = self.root();
        let mut cur = Some(node);
        while let Some(id) = cur {
            if id == root {
                return true;
            }
            cur = self.parent_node(id);
        }
        false
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
    /// Defaulted to a no-op.
    fn restore_focus(&mut self, _opener: Option<NodeId>, _root: NodeId) {}

    /// Lock or unlock document-level scrolling on behalf of `root` (issue #474).
    ///
    /// This is what `Modal`/`Drawer`'s `lock_scroll` reaches. An overlay calls it
    /// with `true` when it opens and `false` when it closes or unmounts; the
    /// backend decides what "locked" means, because the two have nothing in
    /// common mechanically:
    ///
    /// - **Desktop** rejects a *scroll gesture* whose container is outside every
    ///   locking root, so the page behind does not move while the overlay's own
    ///   `overflow: auto` body still does. Nothing is restyled.
    /// - **Web** sets `overflow: hidden` on the real `<html>`, which is the only
    ///   mechanism there is — rinch cannot gate the browser's own wheel.
    ///
    /// **`root` is the locking overlay's root node**, not the node to lock. It is
    /// the reason this takes a node at all: desktop needs to know which subtree
    /// is still allowed to scroll, and a bare `bool` cannot say. Web ignores it.
    ///
    /// **Implementations must count, not latch.** Two overlays open and the inner
    /// one closing must leave the page locked, so a lock is held per `root` and
    /// released per `root`. `NodeHandle::set_scroll_locked` is the caller-facing
    /// spelling; `rinch-components`' `overlay_scroll_lock` is the caller.
    ///
    /// Defaulted to a no-op so a backend with no page to lock (and
    /// `MockDomDocument`) keeps compiling.
    fn set_scroll_locked(&mut self, _locked: bool, _root: NodeId) {}

    /// Resolve layout for the document at the given viewport size.
    ///
    /// This computes Taffy layout and builds text layouts (IFC) for all dirty nodes.
    /// Must be called before querying caret positions or glyph bounds.
    ///
    /// # Arguments
    /// * `width` - Viewport width in pixels
    /// * `height` - Viewport height in pixels
    fn resolve_layout(&mut self, width: f32, height: f32);

    /// Query the layout bounds of a node relative to its parent.
    ///
    /// Returns the (x, y, width, height) of the node's layout box.
    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)>;

    /// The `(left, top)` inset from a node's box origin (the origin
    /// [`query_node_layout`](Self::query_node_layout) reports against) to the origin an
    /// absolutely-positioned child anchors to. On the web that is the node's border
    /// width (`clientLeft`/`clientTop`): CSS positions an `absolute` child against the
    /// padding box, while `getBoundingClientRect` differences report against the border
    /// box, so the editor's overlay container offset must subtract this once. Default
    /// `(0, 0)` — the desktop renderer positions overlays against the same origin it
    /// lays children out against.
    fn content_origin_inset(&self, _node_id: u64) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// The resolved font of `node_id` ([`NodeFont`]), so the IME preedit overlay can
    /// match the text it composes into (e.g. a 32px heading vs. 16px body). Default
    /// `None` — the overlay keeps its inherited font. Backends that can read computed
    /// styles (the browser's `getComputedStyle`) override this.
    fn node_font(&self, _node_id: u64) -> Option<NodeFont> {
        None
    }

    /// Per-line selection rectangles `(x, y, width, height)`, layout-local to the
    /// node's inline layout, covering the byte range `[a, b)`. Used to render a
    /// text selection's highlight. Default: empty (no inline layout / mock).
    fn query_selection_rects(
        &self,
        _node_id: u64,
        _byte_a: usize,
        _byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        Vec::new()
    }

    /// Where a text caret at `byte_offset` inside the text-bearing element
    /// `node_id` is **on screen**, as `(x, y, height)`: its top, in the frame an
    /// app positions a popup in. Logical window pixels on desktop (the frame of
    /// [`NodeHandle::bounds_signal`] and of a `position: fixed` element), viewport
    /// client pixels in the browser (`getBoundingClientRect`).
    ///
    /// An element with no text to measure (an empty paragraph) answers its own
    /// box's origin, one line high, which is where an editor paints the caret on
    /// a blank line. `None` when the node is unknown or has not been laid out.
    ///
    /// Backs `EditorHandle::caret_rect`. Default `None`: a host with no geometry
    /// (the mock document) has no screen to answer for.
    fn query_caret_rect(&self, _node_id: u64, _byte_offset: usize) -> Option<(f32, f32, f32)> {
        None
    }

    /// [`Self::query_caret_rect`] for a caret with `affinity` at a soft wrap
    /// (#301): an `Upstream` caret there is drawn at the end of the upper line,
    /// a `Downstream` one at the start of the lower. Anywhere else it is the same
    /// answer. Default: the affinity-blind [`Self::query_caret_rect`].
    fn query_caret_rect_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        _affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        self.query_caret_rect(node_id, byte_offset)
    }

    /// [`Self::query_caret_position`] for a caret with `affinity` at a soft
    /// wrap (#301), layout-local like it. Default: the affinity-blind
    /// [`Self::query_caret_position`].
    fn query_caret_position_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        _affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        self.query_caret_position(node_id, byte_offset)
    }

    /// Get the tag name of an element node.
    ///
    /// Returns `Some("div")`, `Some("p")`, etc. for elements, `None` for text/comment nodes.
    fn tag_name(&self, _node: NodeId) -> Option<String> {
        None
    }

    /// Get the node type (W3C-style).
    ///
    /// Returns 1 for elements, 3 for text nodes, 8 for comments. Returns `None` if unknown.
    fn node_type(&self, _node: NodeId) -> Option<u16> {
        None
    }

    /// Get the text content of a node.
    ///
    /// For text nodes, returns the text. For elements, returns concatenated descendant text.
    fn text_content(&self, _node: NodeId) -> Option<String> {
        None
    }

    // ── Scroll query API ─────────────────────────────────────────────────

    /// Get the vertical scroll position of an element.
    /// Equivalent to `element.scrollTop` in the web DOM.
    fn scroll_top(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the horizontal scroll position of an element.
    /// Equivalent to `element.scrollLeft` in the web DOM.
    fn scroll_left(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Set the horizontal scroll position of an element.
    /// Equivalent to `element.scrollLeft = value` in the web DOM.
    fn set_scroll_left(&mut self, _node: NodeId, _scroll_left: f64) {}

    /// Get the total scrollable content height of an element.
    /// Equivalent to `element.scrollHeight` in the web DOM.
    fn scroll_height(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the total scrollable content width of an element.
    /// Equivalent to `element.scrollWidth` in the web DOM.
    fn scroll_width(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the visible content area height (layout height minus padding and border).
    /// Equivalent to `element.clientHeight` in the web DOM.
    fn client_height(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the visible content area width (layout width minus padding and border).
    /// Equivalent to `element.clientWidth` in the web DOM.
    fn client_width(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Request that the given node be scrolled into view after the next layout.
    ///
    /// The scroll is deferred because layout must be resolved first to know
    /// the element's position relative to its scroll container.
    fn request_scroll_into_view(&mut self, _node: NodeId) {}

    /// Drain all pending scroll-into-view requests.
    ///
    /// Called by the runtime after `resolve_layout()` to apply deferred scrolls.
    fn drain_scroll_into_view_requests(&mut self) -> Vec<NodeId> {
        Vec::new()
    }

    /// Request that `node` be scrolled to a set place in its scroll container:
    /// its top `fraction` of the way down the container's visible height —
    /// its padding box, from the inside of its top border, a browser's
    /// `clientTop` / `clientHeight` (`0.0` the top edge, `1.0` the bottom), but never nearer than `margin`
    /// px to either edge, with the scroll clamped to what the content allows.
    /// Unlike [`Self::request_scroll_into_view`] it moves even when `node` is
    /// already in view. Deferred until after the next layout, like that one.
    ///
    /// The default treats it as a plain [`Self::request_scroll_into_view`], so
    /// a backend that does not implement it still brings the node on screen.
    fn request_scroll_to_fraction(&mut self, node: NodeId, fraction: f32, margin: f32) {
        let _ = (fraction, margin);
        self.request_scroll_into_view(node);
    }

    /// Drain the pending [`Self::request_scroll_to_fraction`] requests as
    /// `(node, fraction, margin)`. Called by the runtime right after
    /// [`Self::drain_scroll_into_view_requests`], and applied after those.
    fn drain_scroll_to_fraction_requests(&mut self) -> Vec<(NodeId, f32, f32)> {
        Vec::new()
    }

    /// Drain the scroll offsets clamped during layout, as (node, clamped
    /// offset) pairs — coalesced to one entry per node (last value wins).
    ///
    /// Layout clamps a container's scroll offset when its content shrinks or
    /// its viewport grows. Firing handlers mid-layout would re-enter user code
    /// while the document is borrowed, so the engine queues the clamps and the
    /// runtime drains them after `resolve_layout()` to dispatch the same
    /// scroll events input-driven scrolling produces.
    fn drain_scroll_clamps(&mut self) -> Vec<(NodeId, f64)> {
        Vec::new()
    }
}
