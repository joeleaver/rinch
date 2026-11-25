//! The [`DomTree`] supertrait of [`DomDocument`](super::DomDocument): the
//! document's identity, node creation and tree mutation, retiring nodes,
//! dirty tracking, selectors and traversal, connectedness, HTML parsing, and
//! reading a node's tag and type.

use crate::dom::NodeId;

/// The document's node tree: building it, mutating it and walking it.
///
/// One of the five supertraits [`DomDocument`](super::DomDocument) is made
/// of; see that trait for how they fit together. What a node *holds*
/// (attributes, styles, text) is [`DomContent`](super::DomContent).
///
/// # Required Operations
///
/// Implementors must provide:
/// - Node creation: [`create_element`](DomTree::create_element), [`create_text`](DomTree::create_text)
/// - Tree mutation: [`append_child`](DomTree::append_child), [`remove_child`](DomTree::remove_child), etc.
/// - Dirty tracking: [`mark_dirty`](DomTree::mark_dirty), [`take_dirty_nodes`](DomTree::take_dirty_nodes)
pub trait DomTree {
    /// A process-unique identity for this document instance.
    ///
    /// Node ids are per-document slab indices, so two documents on one thread
    /// (e.g. two embedded `RinchContext`s, issue #134) contain the same ids
    /// `0, 1, 2, …`. Thread-local registries that key state by node id must
    /// scope it with this key to avoid cross-document collisions (element
    /// bounds signals, the editor registry, pending focus requests).
    ///
    /// Implementations should allocate it once at construction from
    /// [`next_doc_key`](crate::dom::next_doc_key) — never reuse a key, even for a document at the same
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

    /// Give element `node` a fresh backing element, under the **same id**: one
    /// of the same tag, with the same attributes and children, in the same
    /// place. Whether it did.
    ///
    /// For backend state the element itself carries and that cannot be undone:
    /// a browser `<canvas>` keeps the first kind of context made on it for its
    /// whole life, so a render surface whose WebGPU device is gone needs a new
    /// canvas for its 2D fallback (#1768). Every handle to `node` reaches the
    /// new element afterwards. The default, for a backend with no such state,
    /// does nothing and returns `false`.
    fn renew_element(&mut self, node: NodeId) -> bool {
        let _ = node;
        false
    }

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
    /// [`NodeHandle`]: crate::dom::NodeHandle
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
    /// | `rinch-dom` (desktop) | detach; the slab entry stays | frees the node and its subtree (issue #723): slab slots, Taffy nodes, and every table the document keeps under their ids. Each slot is reused under a new generation |
    /// | `rinch-web` | detach; both node maps keep the node | drops the node and every descendant from the document map **and** the page-global registry (issue #184), releasing the `web_sys` node it was pinning against GC |
    /// | `MockDomDocument` | detach | retires, like the browser |
    ///
    /// **`remove_node`'s post-condition is identical on every backend, and so
    /// is `discard_node`'s**: a discarded node does not re-insert, keeps no
    /// subtree and takes no writes. Until #723 desktop's was strictly weaker
    /// (the trait default below, which reclaims nothing), so code that
    /// re-attached a discarded handle worked on desktop and died on the web;
    /// `MockDomDocument`, which retires like the browser, is what made that
    /// fail `cargo test` on the host.
    ///
    /// # A retired id is never re-issued
    ///
    /// No backend hands a **discarded** id to a later node, so a stale handle
    /// can only ever name nothing, never somebody else, and registries keyed by
    /// node id (focus, the mounted-editor registry) cannot be aimed at the
    /// wrong node by a discard. `rinch-web`'s counter is a monotonic
    /// `fetch_add` with no free list. `rinch-dom` re-issues the freed **slot**
    /// under a higher [`NodeId`] **generation**, on every route that frees
    /// (`discard_node`, `set_inner_html`, pseudo-element pruning on restyle):
    /// its node storage checks the generation on every lookup of an id from
    /// outside the tree, and its `DomDocument` verbs are no-ops for an id that
    /// names no live node. The slot is re-issued; the id is not (until the
    /// 21-bit generation wraps, after about two million reuses of one slot:
    /// see [`NodeId`]). Before #723 a slot freed by `set_inner_html` came back
    /// at generation 0, so a handle to the old node wrote into the new one
    /// (#304).
    ///
    /// The default is `remove_node`: correct but reclaiming nothing, for a
    /// backend whose bookkeeping dies with the document.
    fn discard_node(&mut self, node: NodeId) {
        self.remove_node(node);
    }

    /// Whether the backend holds nothing under `node` any more — it was
    /// [`discard_node`](Self::discard_node)ed, on a backend that reclaims
    /// (issue #733).
    ///
    /// Two kinds of caller: the reactive helpers ask it of the node a view
    /// hands them, to warn about a cache returning a subtree
    /// that was discarded under it, and a component that moved a node out of a
    /// subtree retires that node with the subtree (`NodeHandle::is_retired`,
    /// the `Stepper`'s content region, issue #2195). `rinch-web` and
    /// `MockDomDocument` answer from their node tables, and `rinch-dom` from its
    /// slab (issue #723: a freed slot, or one reused under another generation).
    /// The default is `false`, which is all a backend whose `discard_node`
    /// reclaims nothing can say.
    ///
    /// An id the document never issued also answers `true` where the backend
    /// can tell; no caller hands one in.
    fn is_retired(&self, _node: NodeId) -> bool {
        false
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
    /// Asked by [`NodeHandle::set_text`](crate::dom::NodeHandle::set_text) ahead of
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

    /// Whether `node` is still attached to this document's tree (issue #695).
    ///
    /// Part of the close half of an overlay's focus restore: a `Modal` opened
    /// from a row the dialog itself deleted must not hand focus back into a
    /// detached subtree. It is only *part* of it — being attached is not being
    /// focusable, and [`restore_focus`](super::DomFocus::restore_focus) asks both.
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

    /// Parse HTML string and create DOM nodes, returning the root node ID.
    /// Returns None if the HTML couldn't be parsed.
    fn parse_html(&mut self, html: &str) -> Option<NodeId>;

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
}
