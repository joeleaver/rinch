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

/// A stretch of a text-bearing element's flat byte offsets that holds CSS
/// generated content, `start..end`
/// ([`DomDocument::generated_flat_ranges`], #891). `after` when it is an
/// `::after`'s text, which belongs after the content before it: a caret at
/// `start` is drawn before it. A `::before`'s text belongs before the content
/// after it, so a caret at its `start` is drawn past it, at `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeneratedFlatRange {
    /// The first byte of the generated text.
    pub start: usize,
    /// The byte after it.
    pub end: usize,
    /// An `::after`'s text (else a `::before`'s, or a list marker's).
    pub after: bool,
}

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

/// The width, in CSS px, of the text caret the rich-text editor draws (its
/// overlay). The editor keeps that whole width inside the textblock's border
/// box when spaces hang past a line's end (#1041), as a browser keeps its own
/// 1px caret.
pub const CARET_WIDTH: f32 = 2.0;

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

/// The direction a text selection extends in — [`DomDocument::set_selection_range`]'s
/// fourth argument, matching the DOM's `HTMLInputElement.setSelectionRange(start,
/// end, direction)` exactly (issue #552): `"forward"`, `"backward"` and
/// `"none"` are literally these three variants, lowercased.
///
/// `Forward` and `Backward` decide which end is the *anchor* (fixed) and which
/// is the *head* (what Shift+Arrow would move from) — not merely how the
/// range paints, since a control's own keyboard handling continues from the
/// head. `None` — "unspecified", a browser's own default when the third
/// argument is omitted — is applied the same way [`Forward`](Self::Forward)
/// is: it selects the same text, and the two differ only in what
/// `selectionDirection` would read back, which rinch does not model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionDirection {
    /// The anchor is `start`, the head is `end`.
    #[default]
    Forward,
    /// The anchor is `end`, the head is `start`.
    Backward,
    /// Unspecified, as when a browser's `setSelectionRange` is called with no
    /// third argument. Applied the same way [`Forward`](Self::Forward) is.
    None,
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

    /// Whether the backend holds nothing under `node` any more — it was
    /// [`discard_node`](Self::discard_node)ed, on a backend that reclaims
    /// (issue #733).
    ///
    /// This is a diagnostic, not part of the removal contract: the reactive
    /// helpers ask it of the node a view hands them, to warn about a cache
    /// returning a subtree that was discarded under it. `rinch-web` and
    /// `MockDomDocument` answer from their node tables. The default is
    /// `false`, which is all a backend whose `discard_node` reclaims nothing
    /// can say — `rinch-dom` today (issue #723), where such a node still
    /// re-inserts and nothing is lost to warn about.
    ///
    /// An id the document never issued also answers `true` where the backend
    /// can tell; no caller hands one in.
    fn is_retired(&self, _node: NodeId) -> bool {
        false
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
    ///   attribute — is the live text. Desktop overrides it for one case: a
    ///   `<textarea>` with no `value` attribute answers its text children,
    ///   its default value, as the browser's property does (#1159).
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
    ///
    /// CSSOM's `setProperty`: afterwards the element's inline style gives
    /// `property` this value however the attribute declared it before. A
    /// backend that keeps the attribute as a string of declarations must not
    /// leave the write in front of a later declaration that covers it — a
    /// shorthand, or a logical twin — or that later declaration wins the
    /// cascade and the attribute names a value the element is not laid out
    /// with (#470).
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

    /// The first element of the whole document matching the CSS `selector`,
    /// in document order (`document.querySelector`). `None` for an invalid
    /// selector, which a browser throws a `SyntaxError` for.
    fn query_selector(&self, selector: &str) -> Option<NodeId>;

    /// Every element of the whole document matching the CSS `selector`, in
    /// document order (`document.querySelectorAll`). Empty for an invalid
    /// selector.
    fn query_selector_all(&self, selector: &str) -> Vec<NodeId>;

    /// The first element **under** `scope` matching the CSS `selector`, in
    /// document order: `Element.querySelector` (issue #1796). `scope` itself is
    /// never a candidate, the selector is matched against the whole tree (an
    /// ancestor outside `scope` can satisfy a combinator), and `:scope` is
    /// `scope`. `None` for an invalid selector.
    ///
    /// Defaulted to [`query_selector_all_in`](Self::query_selector_all_in)'s
    /// first answer.
    fn query_selector_in(&self, scope: NodeId, selector: &str) -> Option<NodeId> {
        self.query_selector_all_in(scope, selector)
            .into_iter()
            .next()
    }

    /// Every element under `scope` matching the CSS `selector`, in document
    /// order: `Element.querySelectorAll`, by the rules of
    /// [`query_selector_in`](Self::query_selector_in). Empty for an invalid
    /// selector.
    ///
    /// Defaulted to the document-wide answer kept to `scope`'s descendants (a
    /// [`parent_node`](Self::parent_node) walk each), which cannot see
    /// `:scope` and misses a detached `scope`; both backends override it.
    fn query_selector_all_in(&self, scope: NodeId, selector: &str) -> Vec<NodeId> {
        self.query_selector_all(selector)
            .into_iter()
            .filter(|&n| {
                let mut cur = self.parent_node(n);
                while let Some(id) = cur {
                    if id == scope {
                        return true;
                    }
                    cur = self.parent_node(id);
                }
                false
            })
            .collect()
    }

    /// Get the children of a node.
    fn get_children(&self, node: NodeId) -> Vec<NodeId>;

    /// Whether `node` is a **text node** — a node that has no child list.
    ///
    /// Asked by [`NodeHandle::set_text`](super::NodeHandle::set_text) ahead of
    /// [`get_children`](Self::get_children) while a removal observer is
    /// registered, so the write every reactive `{|| text}` makes reads no
    /// child list (issue #1440). `false` is always a safe answer: it only
    /// costs that read. Defaulted to `false`.
    fn is_text_node(&self, _node: NodeId) -> bool {
        false
    }

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
    /// **Relative to the node's border box** — the box
    /// [`query_node_layout`](Self::query_node_layout) reports — so a padded
    /// or bordered block's caret includes that padding and border (#1667).
    /// The browser backend measures from `getBoundingClientRect`; rinch-dom
    /// adds the offset at which it draws the Parley layout. The same frame
    /// for [`Self::query_glyph_bounds`], [`Self::query_selection_rects`],
    /// [`Self::query_inline_box_caret`] and
    /// [`Self::query_caret_position_with_affinity`].
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
    /// byte offset within the text node, relative to the node's border box
    /// (the frame of [`Self::query_caret_position`]).
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
    ///
    /// On the web this only **queues** the browser work: [`NodeHandle`](super::NodeHandle)'s verb
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
    /// [`NodeHandle::set_selection_range`](super::NodeHandle::set_selection_range)
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
    /// where `len` is [`live_value`](Self::live_value)'s UTF-16 length (`0` if
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
    /// On the web this only **queues** the browser work: [`NodeHandle`](super::NodeHandle)'s verb
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
    /// On the web this only **queues** the browser work: [`NodeHandle`](super::NodeHandle)'s verb
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
    /// [`NodeHandle::focus`]: super::NodeHandle::focus
    /// [`NodeHandle::focus_into`]: super::NodeHandle::focus_into
    /// [`NodeHandle::restore_focus`]: super::NodeHandle::restore_focus
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
    /// Every [`NodeHandle`](super::NodeHandle) verb named above asks this,
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
    /// [`UpdateBatch::apply`]: super::UpdateBatch::apply
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
    /// The insertion verbs of [`NodeHandle`](super::NodeHandle)
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
    /// [`is_focus_revoking_attribute`](super::is_focus_revoking_attribute)
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
    /// [`set_attribute`](Self::set_attribute) / [`remove_attribute`](Self::remove_attribute)
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

    /// The box element `node` is **painted** in, in viewport-relative logical
    /// pixels: the frame of `ClickContext::element_x` and of
    /// [`NodeHandle::bounds_signal`], so a transform on the element or an
    /// ancestor moves and scales it (issue #1779). The browser backend answers
    /// `getBoundingClientRect()`; rinch-dom answers the last layout's painted
    /// border box (it does not lay out on demand).
    ///
    /// A flowed inline element (`<span>`, `<a>`) answers the union of its line
    /// fragments, as `getBoundingClientRect()` does. `None` for a text node,
    /// an unknown node, or one not in the document. A `Some` zero rect for an
    /// element under `display: none` (itself or an ancestor) or one that is
    /// `display: contents` itself, as Chrome answers. `&mut` because rinch-dom
    /// may shape a font probe to measure an inline's fragments.
    /// Default `None`: a host with no geometry (the mock document).
    fn bounding_rect(&mut self, _node: NodeId) -> Option<crate::reactive::ElementBounds> {
        None
    }

    /// The viewport's size in logical pixels — the box a `position: fixed;
    /// inset: 0` element fills, and the frame [`Self::bounding_rect`] answers
    /// in (issue #1764). rinch-dom answers the size of its last layout
    /// (`None` before the first); the browser backend answers
    /// `documentElement.clientWidth/clientHeight` (the viewport less any
    /// classic scrollbar). Default `None`: a host with no geometry.
    fn viewport_size(&self) -> Option<(f32, f32)> {
        None
    }

    /// [`NodeHandle::bounds_signal`] was called for `node` (issue #1781) —
    /// on **every** call, so a backend must ignore a node it already watches.
    /// A backend that does not refresh bounds after its own layout passes — the
    /// web, where the browser lays out — starts watching the element here.
    /// Called with the document mutably borrowed: it must not run app code.
    /// Default: nothing (rinch-dom refreshes every registered node after each
    /// layout).
    fn watch_bounds(&mut self, _node: NodeId) {}

    /// The `(left, top)` inset from a node's box origin (the origin
    /// [`query_node_layout`](Self::query_node_layout) reports against) to the origin an
    /// absolutely-positioned child anchors to. On the web that is the node's border
    /// width (`clientLeft`/`clientTop`): CSS positions an `absolute` child against the
    /// padding box, while `getBoundingClientRect` differences report against the border
    /// box, so the editor's overlay container offset must subtract this once. The
    /// desktop renderer is the same (its layout origins are border-box-relative, an
    /// absolute child's insets padding-box-relative) and answers the border widths
    /// too. An absolute child of a box that scrolls sits against its *scrolled*
    /// content, so the web answers its border less its own `scrollLeft`/
    /// `scrollTop` (#1677), while the desktop's parent-relative layouts are
    /// unscrolled already. Default `(0, 0)`, for a backend with no borders to
    /// report.
    fn content_origin_inset(&self, _node_id: u64) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// A node's padding plus border on each side, `(left, top, right,
    /// bottom)`: from its border box (the box
    /// [`query_node_layout`](Self::query_node_layout) reports) in to its
    /// content box, where its text starts. The editor places the caret in an
    /// empty textblock (no text to ask [`Self::query_caret_position`] about)
    /// at the block's content origin (#1667). Default zero, for a backend
    /// with no box model to report.
    fn content_box_inset(&self, _node_id: u64) -> (f32, f32, f32, f32) {
        (0.0, 0.0, 0.0, 0.0)
    }

    /// The height of one line box of `node_id`: its computed `line-height` in
    /// px. The editor draws the caret in an empty textblock this tall (#1667):
    /// its box need not be one line plus its padding (desktop sizes
    /// `min-height: 1lh` as a border box, #1278). Default `None` (the caller
    /// falls back to the block's content height).
    fn line_box_height(&self, _node_id: u64) -> Option<f32> {
        None
    }

    /// The resolved font of `node_id` ([`NodeFont`]), so the IME preedit overlay can
    /// match the text it composes into (e.g. a 32px heading vs. 16px body). Default
    /// `None` — the overlay keeps its inherited font. Backends that can read computed
    /// styles (the browser's `getComputedStyle`) override this.
    fn node_font(&self, _node_id: u64) -> Option<NodeFont> {
        None
    }

    /// How many bytes a `<br>` element occupies in the flat byte offsets this
    /// backend's text queries take and its hit tests answer
    /// ([`Self::query_caret_position`], [`Self::query_caret_rect`],
    /// [`Self::query_selection_rects`], and the byte a pointer hit resolves to).
    ///
    /// A flat byte offset counts the text of the text nodes under the element, in
    /// document order, as the backend lays that text out — a host may lay a
    /// character out as something of another length ([`Self::tab_flat_bytes`],
    /// [`Self::substituted_char_flat_bytes`]). A
    /// `<br>` has no text node, so a backend that counts only
    /// text gives it none — the browser's DOM ranges, and this default — while one
    /// that lays inline content out as a single string may give it the line break
    /// it lays out: rinch-dom's inline formatting context pushes `"\n"`, one byte.
    /// A caller mapping its own model onto flat offsets (the rich-text editor's
    /// view) asks this so a caret after a break lands after it (#1099).
    fn line_break_flat_bytes(&self) -> usize {
        0
    }

    /// How many bytes a tab (`'\t'`) in a text node occupies in the same flat
    /// byte offsets as [`Self::line_break_flat_bytes`]. Its UTF-8 length, `1`, by
    /// default — the browser's DOM ranges count the character, and rinch-dom
    /// lays a preserved tab out as one space widened to its tab stop (#1122).
    /// A host that laid a tab out as several characters would answer their
    /// bytes, so the rich-text editor's caret map counts each tab as that many
    /// (#1109: rinch-dom laid a tab out as four spaces until #1122).
    ///
    /// The answer is per host, not per node, and it holds only where
    /// `white-space` preserves the tab (`pre`, `pre-wrap`, `break-spaces`);
    /// under `normal`, `nowrap` and `pre-line` a tab collapses like a space.
    /// The editor's stylesheet sets `pre-wrap` on its container and its code
    /// blocks, which is why the editor may use it.
    fn tab_flat_bytes(&self) -> usize {
        1
    }

    /// The characters of a text node under preserved `white-space` that this
    /// backend lays out as something of another length, and the bytes each
    /// then occupies in the flat byte offsets of [`Self::tab_flat_bytes`]
    /// (#1181). None by default — the browser's DOM ranges count the
    /// characters themselves. rinch-dom hands parley a substitute for U+2028,
    /// U+2029, U+0085 and U+000C (its `ifc::PRESERVED_SUBSTITUTES`, the same
    /// table its layout reads), so the rich-text editor's caret map counts
    /// each as that many bytes. Like a tab's, the answer holds only where
    /// `white-space` preserves the text, which the editor's does.
    fn substituted_char_flat_bytes(&self) -> &'static [(char, usize)] {
        &[]
    }

    /// The stretches of `node_id`'s flat byte offsets (the offsets every text
    /// query here takes) that hold CSS generated content — the text of a
    /// `::before` / `::after` — sorted and disjoint (#891). Such text is laid
    /// out among the element's own but is not in its DOM, so a caller that
    /// counts the DOM's (or a model's) text steps over these bytes. None by
    /// default — the browser's DOM ranges never count generated content.
    /// rinch-dom lays it out in the inline formatting context, and answers
    /// each pseudo-element's text there.
    fn generated_flat_ranges(&self, _node_id: u64) -> Vec<GeneratedFlatRange> {
        Vec::new()
    }

    /// The caret beside the atomic inline `box_id` (an `<img>`) that the
    /// text-bearing element `node_id` lays out in its inline layout, as
    /// `(x, y, height)`, border-box-local like [`Self::query_caret_position`]:
    /// on the box's leading edge, or its trailing edge when `after`, at the top
    /// of its line and one line high.
    ///
    /// A flat byte offset cannot name this side: an atomic inline occupies no
    /// bytes of the flat text, so the offsets on its two sides are one number,
    /// and so are those around two such boxes in a row. A caller that knows
    /// which box a caret is beside (the rich-text editor's view, for an image)
    /// asks this instead (#1104). `None` when the box is not in `node_id`'s
    /// inline layout; the caller then asks by byte. Default `None`, so a host
    /// that does not implement it (the mock) is asked by byte as before. The
    /// browser backend answers it for a `<br>` only, which has no bytes there
    /// either (the start of the line it ends, when text or another `<br>`
    /// follows, #1172), and asks an image by byte.
    fn query_inline_box_caret(
        &self,
        _node_id: u64,
        _box_id: u64,
        _after: bool,
    ) -> Option<(f32, f32, f32)> {
        None
    }

    /// The caret on one side of one edge of the inline element `element` (a
    /// `<code>`, a `<mark>`) in the text-bearing element `node_id`, as
    /// [`Self::query_inline_box_caret`] answers for an atomic inline: its
    /// inline-start margin, border and padding (`end == false`) or its
    /// inline-end ones (`end == true`), on their trailing side with `after`
    /// (inside the element at its start; outside it after its end) or their
    /// leading side (outside before its start; inside at its end), at the top
    /// of the line and one line high.
    ///
    /// Those take room on their lines (#1426) but hold no flat bytes, so the
    /// offsets on either side of an edge are one number, and a byte's caret
    /// is drawn before every edge at that byte. A caller that knows which
    /// side of an element the caret is on (the rich-text editor's view, from
    /// the marks the caret carries) asks this. `None` when the element takes
    /// no room at that edge or is not in `node_id`'s inline layout; the
    /// caller then asks by byte. Default `None`: the browser backend draws
    /// its own caret, and the mock has no geometry.
    ///
    /// rinch-dom counts the element's own generated text on that edge (its
    /// `::after` at the end, its `::before` at the start) as part of it
    /// (#891): the inner side is the caret on the element's side of that
    /// text, the outer side past it and past any padding.
    fn query_inline_edge_caret(
        &self,
        _node_id: u64,
        _element: u64,
        _end: bool,
        _after: bool,
    ) -> Option<(f32, f32, f32)> {
        None
    }

    /// [`Self::query_inline_edge_caret`] **on screen**, in the frame of
    /// [`Self::query_caret_rect`]. Default `None`.
    fn query_inline_edge_caret_rect(
        &self,
        _node_id: u64,
        _element: u64,
        _end: bool,
        _after: bool,
    ) -> Option<(f32, f32, f32)> {
        None
    }

    /// [`Self::query_inline_box_caret`] **on screen**, in the frame of
    /// [`Self::query_caret_rect`]. Default `None`.
    fn query_inline_box_caret_rect(
        &self,
        _node_id: u64,
        _box_id: u64,
        _after: bool,
    ) -> Option<(f32, f32, f32)> {
        None
    }

    /// Selection rectangles `(x, y, width, height)`, relative to the node's
    /// border box (the frame of [`Self::query_caret_position`]), covering the
    /// byte range `[a, b)`: on rinch-dom one per line, or one per visual piece
    /// of a line holding a right-to-left run (#1114); on the web the browser's
    /// `Range.getClientRects` boxes. Used to render a
    /// text selection's highlight. Default: empty (no inline layout / mock).
    fn query_selection_rects(
        &self,
        _node_id: u64,
        _byte_a: usize,
        _byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        Vec::new()
    }

    /// [`Self::query_selection_rects`] for a selection that names the
    /// zero-byte inline leaves it holds (images; on a host whose `<br>` has no
    /// bytes, hard breaks too): `selected_boxes` lists every selected one by
    /// its host element id (as for [`Self::query_inline_box_caret`]), and no
    /// other is selected.
    ///
    /// Such a leaf has no bytes, so the byte range cannot say whether one at
    /// `byte_a` or `byte_b` is inside the selection, nor whether one at
    /// `byte_a == byte_b` is selected at all (#1120). Default: the byte range
    /// alone ([`Self::query_selection_rects`]). rinch-dom and the browser
    /// backend answer it.
    fn query_selection_rects_with_boxes(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
        selected_boxes: &[u64],
    ) -> Vec<(f32, f32, f32, f32)> {
        let _ = selected_boxes;
        self.query_selection_rects(node_id, byte_a, byte_b)
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
    /// At a soft line wrap it answers the **downstream** caret, at the start of
    /// the lower line, on both backends; see
    /// [`Self::query_caret_rect_with_affinity`] for the other side.
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
    /// wrap (#301), border-box-local like it. Default: the affinity-blind
    /// [`Self::query_caret_position`].
    fn query_caret_position_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        _affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        self.query_caret_position(node_id, byte_offset)
    }

    /// The line box of the line a caret at `byte_offset` in text-bearing
    /// element `node_id` is drawn on (with `affinity` at a soft wrap), as
    /// `(top, height)`, border-box-local like
    /// [`Self::query_caret_position`]: the whole line, its leading included,
    /// not the glyphs of the character there. The editor's margin anchors
    /// (`EditorHandle::set_margin_anchors`) report it. Default `None`: the
    /// editor then derives the line from the caret rect and the block's
    /// [`line_box_height`](Self::line_box_height).
    fn query_line_box(
        &self,
        _node_id: u64,
        _byte_offset: usize,
        _affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        None
    }

    /// [`Self::query_line_box`] for several carets in one text-bearing
    /// element, in order — for a backend that reads the element's lines once
    /// rather than once per caret (the browser). Default: one query each.
    fn query_line_boxes(
        &self,
        node_id: u64,
        carets: &[(usize, CaretAffinity)],
    ) -> Vec<Option<(f32, f32)>> {
        carets
            .iter()
            .map(|&(byte, affinity)| self.query_line_box(node_id, byte, affinity))
            .collect()
    }

    /// Whether the document has changes its geometry does not show yet: a
    /// mutation or a style change made since the last
    /// [`resolve_layout`](Self::resolve_layout). A backend that lays out on
    /// demand (the browser) answers `false`, as does the default. The editor's
    /// margin pass waits for a layout while it is `true` rather than report
    /// lines that are about to move.
    fn layout_pending(&self) -> bool {
        false
    }

    /// A stamp of the text layout `node_id`'s lines are read from (the one
    /// [`Self::query_line_box`] reads), which changes whenever that layout is
    /// rebuilt — the one way its lines move with no box moving. The editor's
    /// margin pass skips measuring a block whose stamp and boxes are what they
    /// were. Default `None`: unknown, so only the boxes are compared (the
    /// browser, which lays out on demand).
    fn text_layout_generation(&self, _node_id: u64) -> Option<u64> {
        None
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
