//! Mock DOM document for testing.

use super::NodeId;
use super::traits::{DomDocument, GlyphBounds, SelectionDirection};

/// A mock DOM document for testing.
pub struct MockDomDocument {
    doc_key: u64,
    next_id: usize,
    nodes: std::collections::HashMap<NodeId, MockNode>,
    /// Dirty nodes in the order they were first marked. May hold a node that
    /// was discarded since; `dirty_set` is the membership, and
    /// `take_dirty_nodes` reports only what it still holds.
    dirty: Vec<NodeId>,
    /// The nodes currently dirty. Marking and discarding are O(1): a `contains`
    /// over the list made a command that marks n nodes O(n²) — 2.8 s for a
    /// 40,000-cell table row through a mounted editor (#1214).
    dirty_set: std::collections::HashSet<NodeId>,
    root_id: NodeId,
    body_id: NodeId,
    /// Box geometry injected by a test, keyed by node (see
    /// [`__set_node_layout`](MockDomDocument::__set_node_layout)). Empty by
    /// default, which is what makes `query_node_layout` report "not measured".
    layout: std::collections::HashMap<NodeId, (f32, f32, f32, f32)>,
    /// Pending [`DomDocument::request_scroll_into_view`] targets, in request
    /// order. The mock queues them exactly as the desktop backend does — there is
    /// no layout here to scroll, so the queue *is* the observable behaviour.
    scroll_into_view_requests: Vec<NodeId>,
    /// Pending [`DomDocument::request_scroll_to_fraction`] requests, queued the
    /// same way.
    scroll_to_fraction_requests: Vec<(NodeId, f32, f32)>,
    /// The node [`DomDocument::focus_element`] last focused, which is what
    /// [`DomDocument::active_element`] answers (issue #434: a key handler is
    /// offered keys only while focus is inside its owner, so a component test
    /// needs a focus to put somewhere). Nothing moves it but `focus_element`.
    focused: Option<NodeId>,
    /// The last [`DomDocument::set_selection_range`] (and, through its
    /// default, [`DomDocument::select_text`]) request, whichever node it
    /// named and whether or not that node is `focused` — issue #552's "stash
    /// for the next focus" rule needs no separate stash here, since the mock
    /// has nothing else competing for the slot. Read by
    /// [`__selection_range`](MockDomDocument::__selection_range).
    selection_range: Option<(NodeId, usize, usize, SelectionDirection)>,
    /// **Test-only.** How many times [`DomDocument::get_children`] has been
    /// called (issue #732). `get_children` takes `&self`, so this is a `Cell`
    /// rather than a plain counter. What makes a fixture able to measure
    /// the discard walk (`sweep_for_discard`) against the whole-document size
    /// rather than only the discarded subtree's: a bounded walk's count does
    /// not move when an unrelated sibling subtree grows.
    get_children_calls: std::cell::Cell<usize>,
    /// **Test-only.** Calls counted by kind since the document was made
    /// (issue #748) — see [`MockOpCounts`].
    ops: std::cell::Cell<MockOpCounts>,
}

/// **Test-only.** How many of each kind of call a [`MockDomDocument`] has
/// served (issue #748).
///
/// What lets a fixture state the cost of a pass as a count rather than a
/// timing: a container that re-derives every item per change reads and writes
/// in proportion to the items it holds, and one that touches only what moved
/// does not. Read it with [`MockDomDocument::__op_counts`] and subtract a
/// baseline taken in the same test.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MockOpCounts {
    /// `get_attribute` calls.
    pub attribute_reads: usize,
    /// `set_attribute`, `remove_attribute` and `set_style` calls, whether or
    /// not the value changed — the mock, like `rinch-dom`'s `set_attribute`,
    /// does not early-out on an equal write.
    pub attribute_writes: usize,
    /// `set_text_content` calls.
    pub text_writes: usize,
    /// `text_content` calls made by a caller (not the recursion inside one).
    pub text_reads: usize,
    /// `get_children` calls.
    pub children_reads: usize,
}

impl MockOpCounts {
    /// The calls made between `earlier` and `self`.
    pub fn since(self, earlier: MockOpCounts) -> MockOpCounts {
        MockOpCounts {
            attribute_reads: self.attribute_reads - earlier.attribute_reads,
            attribute_writes: self.attribute_writes - earlier.attribute_writes,
            text_writes: self.text_writes - earlier.text_writes,
            text_reads: self.text_reads - earlier.text_reads,
            children_reads: self.children_reads - earlier.children_reads,
        }
    }

    /// Every write of either kind.
    pub fn writes(self) -> usize {
        self.attribute_writes + self.text_writes
    }

    /// Every read of any kind.
    pub fn reads(self) -> usize {
        self.attribute_reads + self.text_reads + self.children_reads
    }
}

struct MockNode {
    kind: MockNodeKind,
    text: String,
    attributes: std::collections::HashMap<String, String>,
    /// A form control's live text — the browser's `.value` property — once
    /// something has set it (issue #238). `None` means "never moved", and the
    /// control then reads as its `value` attribute, as a pristine browser
    /// control does.
    live_value: Option<String>,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
}

/// Tags whose live text is a property separate from the `value` attribute —
/// the three `rinch-web` reads `.value` from.
fn is_value_control(tag: &str) -> bool {
    matches!(tag, "input" | "textarea" | "select")
}

enum MockNodeKind {
    Element(String),
    Text,
    Comment,
}

impl Default for MockDomDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl MockDomDocument {
    /// **Test-only.** How many nodes the table still holds (issues #184, #719).
    ///
    /// The mock's half of `rinch-web`'s `__node_registry_len`, and what makes an
    /// unbounded-growth test runnable on the host: a helper that releases the
    /// wrong way leaks here exactly as it leaks in a browser. Compare against a
    /// baseline taken in the same test — `next_id` counts up for the life of the
    /// document, so absolute numbers mean nothing.
    #[doc(hidden)]
    pub fn __node_count(&self) -> usize {
        self.nodes.len()
    }

    /// **Test-only.** How many times [`DomDocument::get_children`] has been
    /// called so far (issue #732) — see the field doc on `get_children_calls`.
    #[doc(hidden)]
    pub fn __get_children_calls(&self) -> usize {
        self.get_children_calls.get()
    }

    /// **Test-only.** The calls served so far, by kind (issue #748).
    #[doc(hidden)]
    pub fn __op_counts(&self) -> MockOpCounts {
        self.ops.get()
    }

    fn count(&self, bump: impl FnOnce(&mut MockOpCounts)) {
        let mut ops = self.ops.get();
        bump(&mut ops);
        self.ops.set(ops);
    }

    fn text_of(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node)?;
        match n.kind {
            MockNodeKind::Text | MockNodeKind::Comment => Some(n.text.clone()),
            MockNodeKind::Element(_) => {
                // Concatenate descendant text (depth-first), matching the real DOM.
                // Text written over the element itself (`set_text_content`)
                // comes first: it replaced the children there were, and
                // anything listed now was appended after it.
                let mut out = n.text.clone();
                for &child in &n.children {
                    if let Some(t) = self.text_of(child) {
                        out.push_str(&t);
                    }
                }
                Some(out)
            }
        }
    }

    /// **Test-only.** Give `node` a border box, so
    /// [`DomDocument::query_node_layout`] reports `(x, y, w, h)` for it.
    ///
    /// The mock has no layout engine, and every geometry query returns `None`
    /// unless a test says otherwise — which is right for the many tests that only
    /// care about tree structure, but leaves anything reading a *measured* box
    /// (the editor's caret placement, for one) untestable on the host. This is the
    /// injection point: positions are parent-relative, like Taffy's.
    #[doc(hidden)]
    pub fn __set_node_layout(&mut self, node: NodeId, x: f32, y: f32, w: f32, h: f32) {
        self.layout.insert(node, (x, y, w, h));
    }

    /// **Test-only.** Put `text` in a form control the way a **user** does in a
    /// browser: the live text moves and the `value` attribute does not (issue
    /// #238).
    ///
    /// The mock otherwise behaves like desktop, where the two never part — a
    /// `set_attribute("value")` moves both, as `rinch-web`'s reflected-property
    /// mirror does. This is the one way to reach the web's other state, the one
    /// a component that reads the attribute instead of
    /// [`DomDocument::live_value`] gets wrong. It dispatches nothing; a test
    /// follows it with the `oninput` it wants delivered.
    #[doc(hidden)]
    pub fn __type_into(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.live_value = Some(text.to_string());
        }
    }

    /// **Test-only.** The `(node, start, end, direction)` of the last
    /// [`DomDocument::set_selection_range`]/[`DomDocument::select_text`] call,
    /// whichever node it named — a test reads this to assert on a selection
    /// request regardless of whether that node was focused when it arrived
    /// (issue #552).
    #[doc(hidden)]
    pub fn __selection_range(&self) -> Option<(NodeId, usize, usize, SelectionDirection)> {
        self.selection_range
    }

    pub fn new() -> Self {
        let mut doc = Self {
            doc_key: crate::dom::next_doc_key(),
            next_id: 0,
            nodes: std::collections::HashMap::new(),
            dirty: Vec::new(),
            dirty_set: std::collections::HashSet::new(),
            root_id: NodeId(0),
            body_id: NodeId(0),
            layout: std::collections::HashMap::new(),
            scroll_into_view_requests: Vec::new(),
            scroll_to_fraction_requests: Vec::new(),
            focused: None,
            selection_range: None,
            get_children_calls: std::cell::Cell::new(0),
            ops: std::cell::Cell::new(MockOpCounts::default()),
        };

        // Create root and body
        doc.root_id = doc.create_element("html");
        doc.body_id = doc.create_element("body");
        doc.append_child(doc.root_id, doc.body_id);

        doc
    }

    fn next_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Unlink `child` from whatever parent currently lists it.
    ///
    /// `append_child`/`insert_before` call this first so a *move* does not leave
    /// the node listed twice, matching `RinchDocument` and the web backend.
    fn detach(&mut self, child: NodeId) {
        let old_parent = self.nodes.get(&child).and_then(|n| n.parent);
        if let Some(old_parent) = old_parent {
            if let Some(node) = self.nodes.get_mut(&old_parent) {
                node.children.retain(|&c| c != child);
            }
            self.mark_dirty(old_parent);
        }
    }

    /// Drop `node` and its whole subtree from the node table — what
    /// [`DomDocument::discard_node`] means by *retiring* a node.
    ///
    /// Ids are never recycled here (`next_id` only counts up), so a retired id
    /// can never name a different node later; a stale handle just resolves to
    /// nothing, and every accessor on this mock is `get`-guarded.
    fn forget_subtree(&mut self, node: NodeId) {
        let children = self
            .nodes
            .get(&node)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child in children {
            self.forget_subtree(child);
        }
        self.nodes.remove(&node);
        // A retired id must not come back out of `take_dirty_nodes`: a consumer
        // that resolves the ids it is handed would find nothing there.
        self.dirty_set.remove(&node);
    }

    /// Whether `child` still names a node — the guard every structural mutation
    /// applies before listing it under a parent.
    ///
    /// Re-attaching a **discarded** child (one a previous `discard_node`
    /// dropped) is a silent no-op on the web backend, whose
    /// `self.nodes.get(&child.0)` simply misses. It has to be one here too, or
    /// the parent lists an id that resolves to nothing and the caller's bug
    /// hides behind a plausible child count (issues #184, #719).
    ///
    /// A merely **removed** child is still live and still re-insertable — that
    /// is the other half of the same oracle (issue #719).
    fn is_live(&self, child: NodeId) -> bool {
        self.nodes.contains_key(&child)
    }
}

impl DomDocument for MockDomDocument {
    fn doc_key(&self) -> u64 {
        self.doc_key
    }

    fn create_element(&mut self, tag: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Element(tag.to_string()),
                text: String::new(),
                attributes: std::collections::HashMap::new(),
                live_value: None,
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Text,
                text: text.to_string(),
                attributes: std::collections::HashMap::new(),
                live_value: None,
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Comment,
                text: text.to_string(),
                attributes: std::collections::HashMap::new(),
                live_value: None,
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        // Re-parenting detaches first, exactly as `RinchDocument` and the web
        // backend do. Without this a *move* leaves the node listed twice in its
        // old parent's `children`, so sibling-order assertions in tests read a
        // reordered node as a second mount.
        self.detach(child);
        // Both ends must exist, like the web backend's
        // `if let (Some(p), Some(c)) = …` (see [`MockDomDocument::is_live`]).
        if !self.is_live(child) {
            return;
        }
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.push(child);
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.retain(|&c| c != child);
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = None;
        }
        self.mark_dirty(parent);
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        self.detach(child);
        // Same discarded-child guard as `append_child` — the web backend's
        // `if let (Some(p), Some(c), Some(r)) = …` misses on a discarded `child`
        // too. `NodeHandle::insert_after` routes here when the anchor has a next
        // sibling and to `append_child` when it does not, so without this the
        // mock would answer the *same* operation differently depending on where
        // in the list it lands (issue #184).
        if !self.is_live(child) {
            return;
        }
        if let Some(node) = self.nodes.get_mut(&parent)
            && let Some(pos) = node.children.iter().position(|&c| c == reference)
        {
            node.children.insert(pos, child);
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        // Replacing a node with itself is a no-op the browser accepts (the DOM
        // spec re-inserts `node` before its own next sibling), so it must not
        // report the node that is still in the tree as displaced.
        if old == new {
            return;
        }
        let parent = self.nodes.get(&old).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            if let Some(parent_node) = self.nodes.get_mut(&parent_id)
                && let Some(pos) = parent_node.children.iter().position(|&c| c == old)
            {
                parent_node.children[pos] = new;
            }
            if let Some(node) = self.nodes.get_mut(&new) {
                node.parent = Some(parent_id);
            }
            self.mark_dirty(parent_id);
            // `old` is **detached**, not retired — it stays in the table and a
            // caller may insert it again, which is what both real backends do
            // (issue #719). A caller that is finished with it says so with
            // `discard_node`.
            if let Some(node) = self.nodes.get_mut(&old) {
                node.parent = None;
            }
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        let parent = self.nodes.get(&node).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            self.remove_child(parent_id, node);
        }
        // Detach only. The node and its subtree stay in the table and stay
        // re-insertable, per the trait contract (issue #719) — that is the
        // post-condition both real backends owe, so a caller that toggles a
        // captured handle must pass here too.
    }

    /// Detach and **retire**: the node and its subtree leave the table, so every
    /// later operation on those ids is a silent no-op.
    ///
    /// This is the half of the oracle that catches the *other* mistake (issues
    /// #184, #719): the web backend releases a discarded subtree from its maps,
    /// so a caller that discards a node and then re-attaches it would pass every
    /// test in this workspace and break only on the web. Retiring here makes
    /// that a failure on the host.
    fn discard_node(&mut self, node: NodeId) {
        let parent = self.nodes.get(&node).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            self.remove_child(parent_id, node);
        }
        self.forget_subtree(node);
    }

    fn is_retired(&self, node: NodeId) -> bool {
        !self.nodes.contains_key(&node)
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        self.count(|ops| ops.text_writes += 1);
        // On an element the text *replaces* the child list, as it does on both
        // real backends (issue #1440): every child is orphaned — detached, not
        // retired, so a handle to one still re-inserts, which is what
        // `rinch-dom` (`parent = None`, slab entry kept) and the browser
        // (`textContent = …`, the old nodes still in `rinch-web`'s table) leave.
        // The text itself is kept on the element and read back ahead of any
        // child appended later (`text_of`); it has no node id, as the browser's
        // new text node has none (`rinch-dom` mints one).
        let orphans = match self.nodes.get_mut(&node) {
            Some(n) => {
                n.text = text.to_string();
                match n.kind {
                    MockNodeKind::Element(_) => std::mem::take(&mut n.children),
                    MockNodeKind::Text | MockNodeKind::Comment => Vec::new(),
                }
            }
            None => Vec::new(),
        };
        for orphan in orphans {
            if let Some(o) = self.nodes.get_mut(&orphan) {
                o.parent = None;
            }
        }
        self.mark_dirty(node);
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        self.count(|ops| ops.attribute_writes += 1);
        if let Some(n) = self.nodes.get_mut(&node) {
            // A programmatic write reaches the live text too, as `rinch-web`'s
            // `sync_reflected_property` makes it (issue #100).
            if name == "value"
                && matches!(&n.kind, MockNodeKind::Element(tag) if is_value_control(tag))
            {
                n.live_value = Some(value.to_string());
            }
            n.attributes.insert(name.to_string(), value.to_string());
        }
        self.mark_dirty(node);
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        self.count(|ops| ops.attribute_writes += 1);
        if let Some(n) = self.nodes.get_mut(&node) {
            // `rinch-web` empties a control's live text when its `value`
            // attribute goes (`remove_value_attribute` writes `""`).
            if name == "value"
                && matches!(&n.kind, MockNodeKind::Element(tag) if is_value_control(tag))
            {
                n.live_value = Some(String::new());
            }
            n.attributes.remove(name);
        }
        self.mark_dirty(node);
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.count(|ops| ops.attribute_reads += 1);
        self.nodes.get(&node)?.attributes.get(name).cloned()
    }

    /// `rinch-web`'s answer, not the default's: a form control reads its live
    /// text, which [`__type_into`](MockDomDocument::__type_into) can move away
    /// from the attribute, and always answers `Some` — `""` when empty.
    fn live_value(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node)?;
        match &n.kind {
            MockNodeKind::Element(tag) if is_value_control(tag) => Some(
                n.live_value
                    .clone()
                    .or_else(|| n.attributes.get("value").cloned())
                    // A pristine `<textarea>`'s value is its text children,
                    // its default value (#1159).
                    .or_else(|| {
                        (tag == "textarea").then(|| {
                            n.children
                                .iter()
                                .filter_map(|c| self.nodes.get(c))
                                .filter(|c| matches!(c.kind, MockNodeKind::Text))
                                .map(|c| c.text.as_str())
                                .collect()
                        })
                    })
                    .unwrap_or_default(),
            ),
            _ => n.attributes.get("value").cloned(),
        }
    }

    /// Merge one declaration into the node's inline `style`, the way both real
    /// backends do (#666).
    ///
    /// This used to append `"{property}: {value};"` to whatever string was
    /// there, with no separator and no replacement: a node whose `style` said
    /// `"color: red"` came out of `set_style("padding", "12px")` as
    /// `"color: redpadding: 12px;"` — one declaration whose value is
    /// `redpadding: 12px` — and setting the same property twice appended
    /// twice. That made the mock unusable as an oracle for anything about
    /// inline-style *composition*, which is the one thing a `style:` prop and
    /// a style shorthand on the same element are about.
    ///
    /// The parse and the join are
    /// [`split_declarations`](crate::dom::split_declarations)/[`serialize_declarations`](crate::dom::serialize_declarations),
    /// which is also what `RinchDocument::set_styles` uses (#670) — so the mock
    /// now agrees with desktop declaration for declaration, quoted and
    /// bracketed values included. A property already declared is replaced
    /// **where it stands**, matching CSSOM's `setProperty` and desktop's
    /// `dom_tests::set_style_replaces_a_declaration_in_place`, and `property`
    /// is matched ASCII case-insensitively unless it is a custom property
    /// ([`normalize_property_name`](crate::dom::normalize_property_name), #711).
    ///
    /// The mock has no property table, so it does **not** move a write past a
    /// later declaration that covers it (`left` before `inset`), which
    /// `RinchDocument` does and CSSOM's longhand list makes moot (#470); the
    /// strings agree everywhere else.
    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        self.count(|ops| ops.attribute_writes += 1);
        if let Some(n) = self.nodes.get_mut(&node) {
            let style = n.attributes.entry("style".to_string()).or_default();
            let mut decls = super::split_declarations(style);
            let property = super::normalize_property_name(property);
            match decls.iter_mut().find(|(k, _)| k.as_str() == &*property) {
                Some(slot) => slot.1 = value.to_string(),
                None => decls.push((property.into_owned(), value.to_string())),
            }
            *style = super::serialize_declarations(&decls);
        }
        self.mark_dirty(node);
    }

    fn mark_dirty(&mut self, node: NodeId) {
        if self.dirty_set.insert(node) {
            self.dirty.push(node);
        }
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        let marked = std::mem::take(&mut self.dirty);
        // `remove` answers true once per node still dirty: a discarded node
        // is dropped, and one marked again after a discard is reported once.
        marked
            .into_iter()
            .filter(|n| self.dirty_set.remove(n))
            .collect()
    }

    fn root(&self) -> NodeId {
        self.root_id
    }

    fn body(&self) -> NodeId {
        self.body_id
    }

    fn query_selector(&self, _selector: &str) -> Option<NodeId> {
        // Mock implementation - just return body
        Some(self.body_id)
    }

    fn query_selector_all(&self, _selector: &str) -> Vec<NodeId> {
        // Mock implementation - return empty vec
        Vec::new()
    }

    fn is_text_node(&self, node: NodeId) -> bool {
        matches!(
            self.nodes.get(&node).map(|n| &n.kind),
            Some(MockNodeKind::Text)
        )
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.get_children_calls
            .set(self.get_children_calls.get() + 1);
        self.count(|ops| ops.children_reads += 1);
        self.nodes
            .get(&node)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        // Discarded children are not listed — see [`MockDomDocument::is_live`].
        if !self.is_live(child) {
            return;
        }
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            let len = parent_node.children.len();
            let idx = index.min(len);
            parent_node.children.insert(idx, child);
        }
        if let Some(child_node) = self.nodes.get_mut(&child) {
            child_node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        self.nodes.get(&node)?.parent
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        let parent_id = self.nodes.get(&node)?.parent?;
        let parent = self.nodes.get(&parent_id)?;
        let pos = parent.children.iter().position(|&c| c == node)?;
        parent.children.get(pos + 1).copied()
    }

    fn parse_html(&mut self, html: &str) -> Option<NodeId> {
        // Simple mock implementation - just create a text node with the content
        // Real implementations would parse the HTML properly
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Text,
                text: html.to_string(),
                attributes: std::collections::HashMap::new(),
                live_value: None,
                children: Vec::new(),
                parent: None,
            },
        );
        Some(id)
    }

    fn set_scroll_top(&mut self, _node: NodeId, _scroll_top: f64) {
        // Mock implementation - does nothing
    }

    /// Retires every existing child and parses **nothing**: the mock has no
    /// HTML parser, so the element is left with no children at all. The first
    /// half is what both real backends do to the children there were (issue
    /// #184: `rinch-dom` frees them, `rinch-web` forgets them), and it is the
    /// half a test of what the replaced children leave behind needs (#1440).
    fn set_inner_html(&mut self, node: NodeId, _html: &str) {
        let children = match self.nodes.get_mut(&node) {
            Some(n) => {
                n.text.clear();
                std::mem::take(&mut n.children)
            }
            None => return,
        };
        for child in children {
            self.forget_subtree(child);
        }
        self.mark_dirty(node);
    }

    fn query_caret_position(&self, _node_id: u64, _byte_offset: usize) -> Option<(f32, f32)> {
        None // Mock returns None
    }

    fn query_glyph_bounds(&self, _node_id: u64, _byte_offset: usize) -> Option<GlyphBounds> {
        None // Mock returns None
    }

    fn focus_element(&mut self, node_id: NodeId) {
        self.focused = Some(node_id);
    }

    fn active_element(&self) -> Option<NodeId> {
        self.focused
    }

    /// Records `(node_id, start, end, direction)` as-is — swapped so `start
    /// <= end`, the way a browser's own `setSelectionRange` would (issue
    /// #552). The mock does not model "stash until this node is focused"
    /// separately from "applied now": there is nothing else that could be
    /// holding a selection, so recording unconditionally gives both for free.
    fn set_selection_range(
        &mut self,
        node_id: NodeId,
        start: usize,
        end: usize,
        direction: SelectionDirection,
    ) {
        let (start, end) = (start.min(end), start.max(end));
        self.selection_range = Some((node_id, start, end, direction));
    }

    fn resolve_layout(&mut self, _width: f32, _height: f32) {
        // Mock does nothing
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        // `None` unless a test injected a box with `__set_node_layout` — the mock
        // does not lay anything out.
        self.layout.get(&NodeId(node_id as usize)).copied()
    }

    fn request_scroll_into_view(&mut self, node: NodeId) {
        self.scroll_into_view_requests.push(node);
    }

    fn drain_scroll_into_view_requests(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.scroll_into_view_requests)
    }

    fn request_scroll_to_fraction(&mut self, node: NodeId, fraction: f32, margin: f32) {
        self.scroll_to_fraction_requests
            .push((node, fraction, margin));
    }

    fn drain_scroll_to_fraction_requests(&mut self) -> Vec<(NodeId, f32, f32)> {
        std::mem::take(&mut self.scroll_to_fraction_requests)
    }

    fn tag_name(&self, node: NodeId) -> Option<String> {
        match &self.nodes.get(&node)?.kind {
            MockNodeKind::Element(tag) => Some(tag.clone()),
            _ => None,
        }
    }

    fn node_type(&self, node: NodeId) -> Option<u16> {
        match &self.nodes.get(&node)?.kind {
            MockNodeKind::Element(_) => Some(1),
            MockNodeKind::Text => Some(3),
            MockNodeKind::Comment => Some(8),
        }
    }

    fn text_content(&self, node: NodeId) -> Option<String> {
        self.count(|ops| ops.text_reads += 1);
        self.text_of(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every structural mutation refuses a **discarded** child, not just
    /// `append_child` (issue #184).
    ///
    /// `NodeHandle::insert_after` routes to `insert_before` when the anchor has a
    /// next sibling and to `append_child` when it does not, so a mock that guarded
    /// only one of them would answer the *same* operation differently depending on
    /// where in the list it landed — and would list an id that resolves to nothing,
    /// hiding the caller's bug behind a plausible child count.
    #[test]
    fn a_discarded_child_is_refused_by_every_insertion_path() {
        type Relist = fn(&mut MockDomDocument, NodeId, NodeId, NodeId);
        let paths: [Relist; 3] = [
            |doc, parent, child, _anchor| doc.append_child(parent, child),
            |doc, parent, child, anchor| doc.insert_before(parent, child, anchor),
            |doc, parent, child, _anchor| doc.insert_child(parent, child, 0),
        ];
        for relist in paths {
            let mut doc = MockDomDocument::new();
            let body = doc.body();
            let anchor = doc.create_element("div");
            doc.append_child(body, anchor);
            let child = doc.create_element("div");
            doc.append_child(body, child);

            doc.discard_node(child);
            relist(&mut doc, body, child, anchor);

            assert!(
                !doc.get_children(body).contains(&child),
                "#184: a discarded child must not be relisted under its parent"
            );
            for id in doc.get_children(body) {
                assert!(
                    doc.tag_name(id).is_some(),
                    "#184: every listed child must still resolve to a node"
                );
            }
        }
    }

    /// The other half: every one of those paths **accepts** a merely *removed*
    /// child, because `remove_node` is a detach and the subtree must come back
    /// (issue #719).
    ///
    /// The pair is what makes this mock an oracle for both mistakes rather than
    /// one. Sitting on `remove_node` alone — the shape before #719 — a mock that
    /// retires everything and a mock that retires nothing are both "consistent";
    /// only testing the two verbs against each other tells them apart.
    #[test]
    fn a_removed_child_is_accepted_by_every_insertion_path() {
        type Relist = fn(&mut MockDomDocument, NodeId, NodeId, NodeId);
        let paths: [Relist; 3] = [
            |doc, parent, child, _anchor| doc.append_child(parent, child),
            |doc, parent, child, anchor| doc.insert_before(parent, child, anchor),
            |doc, parent, child, _anchor| doc.insert_child(parent, child, 0),
        ];
        for relist in paths {
            let mut doc = MockDomDocument::new();
            let body = doc.body();
            let anchor = doc.create_element("div");
            doc.append_child(body, anchor);
            let child = doc.create_element("div");
            let grandchild = doc.create_element("span");
            doc.append_child(child, grandchild);
            doc.append_child(body, child);

            doc.remove_node(child);
            assert!(
                !doc.get_children(body).contains(&child),
                "precondition: remove_node unlinks the child"
            );

            relist(&mut doc, body, child, anchor);

            assert!(
                doc.get_children(body).contains(&child),
                "#719: a removed child must be re-insertable"
            );
            assert_eq!(
                doc.get_children(child),
                vec![grandchild],
                "#719: its subtree must come back with it"
            );
        }
    }

    /// `replace_node` **detaches** the node it displaced, like both real backends
    /// (issue #719): it keeps its identity and its subtree, and can be inserted
    /// again.
    #[test]
    fn replacing_a_node_detaches_it_and_keeps_its_subtree() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let old = doc.create_element("div");
        doc.append_child(body, old);
        let grandchild = doc.create_element("span");
        doc.append_child(old, grandchild);
        let new = doc.create_element("p");

        doc.replace_node(old, new);

        assert_eq!(doc.get_children(body), vec![new]);
        assert!(doc.tag_name(old).is_some(), "#719: `old` must stay live");
        assert_eq!(
            doc.parent_node(old),
            None,
            "#719: but it must be unlinked from its parent"
        );
        assert_eq!(
            doc.get_children(old),
            vec![grandchild],
            "#719: `old` keeps its own subtree"
        );

        doc.append_child(body, old);
        assert_eq!(
            doc.get_children(body),
            vec![new, old],
            "#719: a displaced node can be inserted again"
        );
    }

    /// `discard_node` is the route that retires: the subtree leaves the table
    /// (issue #184).
    #[test]
    fn discarding_a_node_retires_it_and_its_subtree() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let old = doc.create_element("div");
        doc.append_child(body, old);
        let grandchild = doc.create_element("span");
        doc.append_child(old, grandchild);

        doc.discard_node(old);

        assert!(doc.get_children(body).is_empty());
        assert!(doc.tag_name(old).is_none(), "#184: `old` must be retired");
        assert!(
            doc.tag_name(grandchild).is_none(),
            "#184: `old`'s subtree must be retired with it"
        );
    }

    /// Replacing a node with itself is a no-op the browser accepts, so it must not
    /// unlink a node that is still in the tree (issue #184).
    #[test]
    fn replacing_a_node_with_itself_does_not_unlink_it() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let node = doc.create_element("div");
        doc.append_child(body, node);

        doc.replace_node(node, node);

        assert_eq!(doc.get_children(body), vec![node]);
        assert_eq!(
            doc.parent_node(node),
            Some(body),
            "#184: a self-replace must leave the node in its parent"
        );
    }

    /// A retired id must not resurface from `take_dirty_nodes` — a consumer that
    /// resolves what it is handed would find nothing there (issue #184).
    #[test]
    fn a_discarded_node_leaves_the_dirty_list() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let node = doc.create_element("div");
        doc.append_child(body, node);
        doc.set_attribute(node, "class", "x");

        doc.discard_node(node);

        assert!(
            !doc.take_dirty_nodes().contains(&node),
            "#184: a discarded node must not be reported dirty"
        );
    }

    /// A node marked twice is reported once, in the order first marked, and a
    /// node reported once is reported again when marked after the take (#1214:
    /// the list is backed by a set now, which a take must empty).
    #[test]
    fn dirty_nodes_are_reported_once_per_take() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let a = doc.create_element("div");
        let b = doc.create_element("div");
        doc.append_child(body, a);
        doc.append_child(body, b);
        doc.take_dirty_nodes();
        doc.set_attribute(b, "class", "x");
        doc.set_attribute(a, "class", "x");
        doc.set_attribute(b, "class", "y");
        assert_eq!(doc.take_dirty_nodes(), vec![b, a]);
        assert!(doc.take_dirty_nodes().is_empty());
        doc.set_attribute(a, "class", "z");
        assert_eq!(doc.take_dirty_nodes(), vec![a]);
    }

    // === set_style composes an inline style (#666) ===
    //
    // The mock is the oracle two other layers are tested against — `StyleProp`
    // in this crate, and every component test that renders onto a
    // `MockDomDocument` — so its `style` attribute has to be the string a real
    // backend would leave. It used to be `push_str("{prop}: {value};")` with no
    // separator and no replacement, which is neither parseable CSS nor
    // anything `RinchDocument` or a browser would produce.

    fn style_of(doc: &MockDomDocument, node: NodeId) -> String {
        doc.get_attribute(node, "style")
            .expect("a style was written")
    }

    /// A `set_style` onto an attribute somebody else wrote joins it with the
    /// separator that makes it a second declaration.
    ///
    /// Appending with none gave `"color: redpadding: 12px;"` — one declaration
    /// whose value is `redpadding: 12px`.
    #[test]
    fn set_style_separates_itself_from_an_existing_declaration() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", "color: red");
        doc.set_style(div, "padding", "12px");
        assert_eq!(style_of(&doc, div), "color: red; padding: 12px");
    }

    /// Setting a property twice replaces it **where it stands**, which is what
    /// CSSOM's `setProperty` does and what
    /// `rinch-dom`'s `dom_tests::set_style_replaces_a_declaration_in_place`
    /// pins for desktop. Appending instead left both declarations in the block.
    ///
    /// The `gap` between them is load-bearing: with only the two `color`
    /// declarations, replace-in-place and remove-then-append give the same
    /// string, so the fixture would sit on a fixed point and pass either way.
    #[test]
    fn set_style_replaces_a_declaration_in_place() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_style(div, "color", "red");
        doc.set_style(div, "gap", "4px");
        doc.set_style(div, "color", "blue");
        assert_eq!(style_of(&doc, div), "color: blue; gap: 4px");
    }

    /// `property` is matched ASCII case-insensitively, so the mock composes an
    /// inline style the way desktop and a browser do (#711). The `gap` is here
    /// for the same reason as above — without it, folding and appending give
    /// the same string.
    ///
    /// Kills the mutant that drops `normalize_property_name` from
    /// `MockDomDocument::set_style`.
    #[test]
    fn set_style_matches_a_property_name_case_insensitively() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", "color: red; gap: 4px");
        doc.set_style(div, "COLOR", "blue");
        assert_eq!(style_of(&doc, div), "color: blue; gap: 4px");
    }

    /// …but a **custom** property is compared exactly, so these are two.
    /// Chrome 150: `setProperty("--Foo", "3px")` on a block holding
    /// `--foo: 2px` gives `cssText === "--foo: 2px; --Foo: 3px;"`.
    ///
    /// Kills the mutant that lowercases unconditionally.
    #[test]
    fn set_style_keeps_two_custom_properties_that_differ_only_in_case() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", "--foo: 2px");
        doc.set_style(div, "--Foo", "3px");
        assert_eq!(style_of(&doc, div), "--foo: 2px; --Foo: 3px");
    }

    /// And it goes through the workspace's one inline-style parser (#670), so a
    /// `;` inside a `url()` already in the attribute is part of that value
    /// here too — the mock diverging from desktop on this would make it an
    /// oracle for the wrong thing.
    #[test]
    fn set_style_does_not_split_a_url_already_in_the_attribute() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            "background-image: url(data:image/png;base64,AAA=)",
        );
        doc.set_style(div, "padding", "12px");
        assert_eq!(
            style_of(&doc, div),
            "background-image: url(data:image/png;base64,AAA=); padding: 12px"
        );
    }

    /// The first write onto a node with no style attribute is still just the
    /// one declaration — no leading separator, no trailing `;`.
    #[test]
    fn the_first_set_style_writes_one_bare_declaration() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_style(div, "color", "red");
        assert_eq!(style_of(&doc, div), "color: red");
    }

    /// The mock models the web's split (issue #238): a user's typing moves the
    /// live text and leaves the `value` attribute behind, and `live_value` is
    /// what reports the field — the attribute is a fossil of the last write.
    /// Removing a control's `value` attribute empties its live text, as the
    /// web backend's `remove_value_attribute` does — not only the attribute.
    #[test]
    fn removing_the_value_attribute_empties_the_live_text() {
        let mut doc = MockDomDocument::new();
        let input = doc.create_element("input");
        doc.set_attribute(input, "value", "x");
        doc.__type_into(input, "typed");
        doc.remove_attribute(input, "value");
        assert_eq!(doc.live_value(input).as_deref(), Some(""));
    }

    #[test]
    fn live_value_reports_typed_text_while_the_attribute_lags() {
        let mut doc = MockDomDocument::new();
        let input = doc.create_element("input");
        doc.set_attribute(input, "value", "mount");

        doc.__type_into(input, "typed");

        assert_eq!(doc.live_value(input).as_deref(), Some("typed"));
        assert_eq!(
            doc.get_attribute(input, "value").as_deref(),
            Some("mount"),
            "typing must not reach the attribute, or the split is not modelled"
        );

        // A programmatic write reaches both, as the web's reflected-property
        // mirror makes it.
        doc.set_attribute(input, "value", "written");
        assert_eq!(doc.live_value(input).as_deref(), Some("written"));
        assert_eq!(
            doc.get_attribute(input, "value").as_deref(),
            Some("written")
        );
    }

    /// A form control always has a live value — `""` before anything set one —
    /// and an element that is not a form control answers from its attribute.
    #[test]
    fn a_pristine_textareas_live_value_is_its_text_children() {
        let mut doc = MockDomDocument::new();
        let textarea = doc.create_element("textarea");
        let a = doc.create_text("one ");
        let b = doc.create_text("two");
        doc.append_child(textarea, a);
        doc.append_child(textarea, b);
        assert_eq!(doc.live_value(textarea).as_deref(), Some("one two"));
        doc.set_attribute(textarea, "value", "");
        assert_eq!(doc.live_value(textarea).as_deref(), Some(""));

        let input = doc.create_element("input");
        let c = doc.create_text("child");
        doc.append_child(input, c);
        assert_eq!(doc.live_value(input).as_deref(), Some(""));
    }

    #[test]
    fn live_value_of_an_empty_control_and_of_a_non_control() {
        let mut doc = MockDomDocument::new();
        let textarea = doc.create_element("textarea");
        let div = doc.create_element("div");
        assert_eq!(doc.live_value(textarea).as_deref(), Some(""));
        assert_eq!(doc.live_value(div), None);

        doc.set_attribute(div, "value", "attr");
        doc.__type_into(div, "ignored");
        assert_eq!(doc.live_value(div).as_deref(), Some("attr"));
    }

    /// `DomDocument::set_selection_range` records the normalized `(node,
    /// start, end, direction)` exactly — issue #552's basic contract, which a
    /// mutant swapping `start.min(end)`/`start.max(end)` (or dropping the
    /// swap) would still pass if `start <= end` already, so the range here is
    /// deliberately given reversed (`5, 1`) to prove the swap runs.
    #[test]
    fn set_selection_range_records_a_normalized_range() {
        let mut doc = MockDomDocument::new();
        let input = doc.create_element("input");
        doc.set_selection_range(input, 5, 1, SelectionDirection::Backward);
        assert_eq!(
            doc.__selection_range(),
            Some((input, 1, 5, SelectionDirection::Backward))
        );
    }

    /// `DomDocument::select_text`'s trait default is exactly
    /// `set_selection_range(node, 0, <UTF-16 len>, Forward)` (issue #552) —
    /// using a string with a surrogate pair (an emoji, 2 UTF-16 units) so a
    /// mutant reporting the **byte** length instead would be caught: `"a🙂"`
    /// is 2 chars / 5 bytes / 3 UTF-16 units.
    #[test]
    fn select_text_selects_the_full_utf16_length() {
        let mut doc = MockDomDocument::new();
        let input = doc.create_element("input");
        doc.set_attribute(input, "value", "a🙂");
        doc.select_text(input);
        assert_eq!(
            doc.__selection_range(),
            Some((input, 0, 3, SelectionDirection::Forward))
        );
    }
}
