//! Mock DOM document for testing.
//!
//! The struct and its test hooks live here. Its [`DomDocument`](super::DomDocument)
//! impl is one file per supertrait: `tree.rs`, `content.rs`, `focus.rs`,
//! `layout.rs` and `scroll.rs`.

mod content;
mod focus;
mod layout;
mod scroll;
#[cfg(test)]
mod tests;
mod tree;

use super::NodeId;
use super::traits::{DomTree, SelectionDirection};

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
    /// Pending [`DomScroll::request_scroll_into_view`] targets, in request
    /// order. The mock queues them exactly as the desktop backend does — there is
    /// no layout here to scroll, so the queue *is* the observable behaviour.
    scroll_into_view_requests: Vec<NodeId>,
    /// Pending [`DomScroll::request_scroll_to_fraction`] requests, queued the
    /// same way.
    scroll_to_fraction_requests: Vec<(NodeId, f32, f32)>,
    /// The node [`DomFocus::focus_element`] last focused, which is what
    /// [`DomFocus::active_element`] answers (issue #434: a key handler is
    /// offered keys only while focus is inside its owner, so a component test
    /// needs a focus to put somewhere). Nothing moves it but `focus_element`.
    focused: Option<NodeId>,
    /// The last [`DomFocus::set_selection_range`] (and, through its
    /// default, [`DomFocus::select_text`]) request, whichever node it
    /// named and whether or not that node is `focused` — issue #552's "stash
    /// for the next focus" rule needs no separate stash here, since the mock
    /// has nothing else competing for the slot. Read by
    /// [`__selection_range`](MockDomDocument::__selection_range).
    selection_range: Option<(NodeId, usize, usize, SelectionDirection)>,
    /// **Test-only.** How many times [`DomTree::get_children`] has been
    /// called (issue #732). `get_children` takes `&self`, so this is a `Cell`
    /// rather than a plain counter. What makes a fixture able to measure
    /// the discard walk (`sweep_for_discard`) against the whole-document size
    /// rather than only the discarded subtree's: a bounded walk's count does
    /// not move when an unrelated sibling subtree grows.
    get_children_calls: std::cell::Cell<usize>,
    /// Calls to `is_text_node` (issue #1487): on the web each is a JS call.
    is_text_node_calls: std::cell::Cell<usize>,
    /// **Test-only.** Calls counted by kind since the document was made
    /// (issue #748) — see [`MockOpCounts`].
    ops: std::cell::Cell<MockOpCounts>,
    /// **Test-only.** Every [`DomLayout::watch_bounds`] call, in order
    /// (issue #1781).
    watched_bounds: Vec<NodeId>,
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

    /// **Test-only.** How many times [`DomTree::get_children`] has been
    /// called so far (issue #732) — see the field doc on `get_children_calls`.
    #[doc(hidden)]
    pub fn __get_children_calls(&self) -> usize {
        self.get_children_calls.get()
    }

    /// **Test-only.** How many times `is_text_node` has been called (issue
    /// #1487).
    #[doc(hidden)]
    pub fn __is_text_node_calls(&self) -> usize {
        self.is_text_node_calls.get()
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
    /// [`DomLayout::query_node_layout`] reports `(x, y, w, h)` for it.
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
    /// [`DomContent::live_value`] gets wrong. It dispatches nothing; a test
    /// follows it with the `oninput` it wants delivered.
    #[doc(hidden)]
    pub fn __type_into(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.live_value = Some(text.to_string());
        }
    }

    /// **Test-only.** The nodes [`DomLayout::watch_bounds`] was called for,
    /// in call order (issue #1781).
    #[doc(hidden)]
    pub fn __watched_bounds(&self) -> &[NodeId] {
        &self.watched_bounds
    }

    /// **Test-only.** The `(node, start, end, direction)` of the last
    /// [`DomFocus::set_selection_range`]/[`DomFocus::select_text`] call,
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
            is_text_node_calls: std::cell::Cell::new(0),
            ops: std::cell::Cell::new(MockOpCounts::default()),
            watched_bounds: Vec::new(),
        };

        // Create root and body
        doc.root_id = doc.create_element("html");
        doc.body_id = doc.create_element("body");
        doc.append_child(doc.root_id, doc.body_id);

        doc
    }

    fn next_id(&mut self) -> NodeId {
        let id = NodeId::from_index(self.next_id);
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
    /// [`DomTree::discard_node`] means by *retiring* a node.
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
