//! The node storage: a slab whose keys are generation-checked ids (issue #723).
//!
//! A [`RawNodeId`] is the same packed `u64` as `rinch_core::dom::NodeId`: the
//! slot index in the low 32 bits, the slot's generation in the high 32.
//!
//! # Two lookups, and which one a site uses
//!
//! - [`NodeSlab::get`] / [`NodeSlab::get_mut`] / [`NodeSlab::contains`] are
//!   **checked**: they compare the id against the id the stored node carries
//!   ([`Node::id`]), so an id issued for a slot's earlier occupant resolves to
//!   nothing instead of naming whichever node holds the slot now. Use them for
//!   every id that comes from **outside the tree**: a `DomDocument` /
//!   `NodeHandle` argument, an app-held handle, an event target, a side table
//!   or cache keyed by node id that can outlive the node.
//! - [`NodeSlab::get_live`] / [`NodeSlab::get_live_mut`] and indexing
//!   (`nodes[id]`) are for ids the tree **hands itself** during a walk — a
//!   node's `children` / `parent`, a layout, paint or hit-test traversal —
//!   which are live by construction. A release build looks only at the slot; a
//!   debug build asserts the generation, so a misuse fails the tests.
//!
//! When unsure, use the checked lookup.
//!
//! **Every id is generation 0 today.** Slots freed by `NodeTree::remove_subtree`
//! (`set_inner_html`, pseudo-element pruning) and by the IFC passes' anonymous
//! boxes are still reused at generation 0, exactly as before this type existed,
//! so nothing a test or an app observes changed. Bumping the generation on a
//! free, and freeing on `discard_node`, is the second half of #723: it needs the
//! side tables keyed by a raw id cleared first (`dirty_nodes`,
//! `paint_dirty_nodes`, `taffy_map`, …), or a stale entry would index a freed
//! slot and panic.

use std::ops::{Index, IndexMut};

use crate::node::{Node, RawNodeId};

/// The slot index of `id` (its low 32 bits).
#[inline(always)]
pub fn slot_of(id: RawNodeId) -> usize {
    (id & 0xFFFF_FFFF) as usize
}

/// The generation of `id` (its high 32 bits).
#[inline(always)]
pub fn generation_of(id: RawNodeId) -> u32 {
    (id >> 32) as u32
}

/// The id of slot `slot` at `generation`.
#[inline(always)]
pub fn pack(slot: usize, generation: u32) -> RawNodeId {
    debug_assert!(
        slot <= u32::MAX as usize,
        "node slot {slot} exceeds 32 bits"
    );
    ((generation as u64) << 32) | slot as u64
}

/// The nodes of a [`crate::node::NodeTree`], keyed by generation-checked id.
///
/// The API mirrors the parts of `slab::Slab` the tree used, with ids in place
/// of slab keys. Iteration yields each node's own id.
#[derive(Default)]
pub struct NodeSlab {
    slab: slab::Slab<Node>,
}

#[cold]
#[inline(never)]
#[track_caller]
fn stale(id: RawNodeId, occupant: Option<RawNodeId>) -> ! {
    match occupant {
        Some(now) => panic!(
            "stale node id: slot {} generation {} is now held by generation {} (issue #723)",
            slot_of(id),
            generation_of(id),
            generation_of(now),
        ),
        None => panic!(
            "node id names no node: slot {} generation {} is vacant",
            slot_of(id),
            generation_of(id),
        ),
    }
}

impl NodeSlab {
    /// An empty slab.
    pub fn new() -> Self {
        Self::default()
    }

    /// The node `id` names, or `None` if its slot is vacant or holds a node of
    /// another generation.
    #[inline]
    pub fn get(&self, id: RawNodeId) -> Option<&Node> {
        match self.slab.get(slot_of(id)) {
            Some(n) if n.id == id => Some(n),
            _ => None,
        }
    }

    /// The node `id` names, mutably; `None` as [`Self::get`].
    #[inline]
    pub fn get_mut(&mut self, id: RawNodeId) -> Option<&mut Node> {
        match self.slab.get_mut(slot_of(id)) {
            Some(n) if n.id == id => Some(n),
            _ => None,
        }
    }

    /// The node a **tree-internal** id names: one the tree handed itself during
    /// a walk (a `children`/`parent` link, a layout, paint or hit-test
    /// traversal), which is live by construction. `None` only for a vacant slot.
    ///
    /// A release build looks the slot up without comparing the generation —
    /// the compare cost 4–7% on the hit-test and relayout benches (#723). A
    /// debug build asserts it, so a site that hands this an id from outside the
    /// tree fails the tests rather than reading another node in production. An
    /// id from outside the tree (a `DomDocument`/`NodeHandle` argument, an app's
    /// handle, an event target, a side table or cache that can outlive a node)
    /// goes through [`Self::get`]. When unsure, use [`Self::get`].
    #[inline]
    #[track_caller]
    pub fn get_live(&self, id: RawNodeId) -> Option<&Node> {
        let n = self.slab.get(slot_of(id))?;
        #[cfg(debug_assertions)]
        if n.id != id {
            stale(id, Some(n.id));
        }
        Some(n)
    }

    /// [`Self::get_live`], mutably.
    #[inline]
    #[track_caller]
    pub fn get_live_mut(&mut self, id: RawNodeId) -> Option<&mut Node> {
        let n = self.slab.get_mut(slot_of(id))?;
        #[cfg(debug_assertions)]
        if n.id != id {
            stale(id, Some(n.id));
        }
        Some(n)
    }

    /// Whether `id` names a live node.
    #[inline]
    pub fn contains(&self, id: RawNodeId) -> bool {
        self.get(id).is_some()
    }

    /// The id the next [`Self::insert`] will store its node under. The node
    /// must carry it as its [`Node::id`].
    #[inline]
    pub fn vacant_key(&self) -> RawNodeId {
        pack(self.slab.vacant_key(), 0)
    }

    /// Store `node` under its own id, which must be [`Self::vacant_key`].
    /// Returns that id.
    pub fn insert(&mut self, node: Node) -> RawNodeId {
        let id = node.id;
        let entry = self.slab.vacant_entry();
        assert_eq!(
            pack(entry.key(), generation_of(id)),
            id,
            "a node must be inserted under the id vacant_key() gave it"
        );
        entry.insert(node);
        id
    }

    /// Take the node `id` names out of the slab, freeing its slot.
    ///
    /// # Panics
    ///
    /// If `id` names no live node.
    #[track_caller]
    pub fn remove(&mut self, id: RawNodeId) -> Node {
        let slot = slot_of(id);
        match self.slab.get(slot) {
            Some(n) if n.id == id => self.slab.remove(slot),
            other => stale(id, other.map(|n| n.id)),
        }
    }

    /// How many nodes are stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.slab.len()
    }

    /// Whether no node is stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slab.is_empty()
    }

    /// One past the highest slot ever used: the length a table indexed by
    /// [`slot_of`] needs to cover every node.
    #[inline]
    pub fn slot_bound(&self) -> usize {
        self.slab.capacity().max(self.slab.len())
    }

    /// Every node with its id, in slot order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (RawNodeId, &Node)> + '_ {
        self.slab.iter().map(|(_, n)| (n.id, n))
    }

    /// Every node with its id, mutably, in slot order.
    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (RawNodeId, &mut Node)> + '_ {
        self.slab.iter_mut().map(|(_, n)| (n.id, n))
    }
}

impl<'a> IntoIterator for &'a NodeSlab {
    type Item = (RawNodeId, &'a Node);
    type IntoIter = std::iter::Map<slab::Iter<'a, Node>, fn((usize, &'a Node)) -> Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        fn with_id(entry: (usize, &Node)) -> (RawNodeId, &Node) {
            (entry.1.id, entry.1)
        }
        self.slab
            .iter()
            .map(with_id as fn((usize, &'a Node)) -> Self::Item)
    }
}

impl Index<RawNodeId> for NodeSlab {
    type Output = Node;

    /// The node `id` names.
    ///
    /// # Panics
    ///
    /// If `id`'s slot is vacant. A **debug** build also panics when the slot
    /// holds a node of another generation; a release build indexes by slot
    /// alone, because this is the path of the ids the tree hands itself
    /// (`parent`, `children`, a layout walk), which are live by construction,
    /// and the compare cost several percent on the layout and paint benches
    /// (#723). An id from outside the tree — an app's handle, an event target,
    /// a side table kept across frames — goes through [`NodeSlab::get`], which
    /// checks the generation in every build.
    #[inline]
    #[track_caller]
    fn index(&self, id: RawNodeId) -> &Node {
        #[cfg(debug_assertions)]
        match self.slab.get(slot_of(id)) {
            Some(n) if n.id == id => n,
            other => stale(id, other.map(|n| n.id)),
        }
        #[cfg(not(debug_assertions))]
        &self.slab[slot_of(id)]
    }
}

impl IndexMut<RawNodeId> for NodeSlab {
    #[inline]
    #[track_caller]
    fn index_mut(&mut self, id: RawNodeId) -> &mut Node {
        #[cfg(debug_assertions)]
        match self.slab.get_mut(slot_of(id)) {
            Some(n) => {
                if n.id != id {
                    stale(id, Some(n.id));
                }
                n
            }
            None => stale(id, None),
        }
        #[cfg(not(debug_assertions))]
        &mut self.slab[slot_of(id)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_round_trips() {
        let id = pack(7, 3);
        assert_eq!(slot_of(id), 7);
        assert_eq!(generation_of(id), 3);
        assert_eq!(pack(7, 0), 7, "generation 0 is just the slot");
        let public = rinch_core::dom::NodeId::new(7, 3);
        assert_eq!(public.0, id, "the same packing as the public NodeId");
    }

    /// The tree's own `<body>`, and the same slot one generation on: what a
    /// handle kept from before the slot was freed and reused would hold.
    fn body_and_stale() -> (crate::node::NodeTree, RawNodeId, RawNodeId) {
        let tree = crate::node::NodeTree::new();
        let body = tree.body_id;
        assert_eq!(generation_of(body), 0, "every id is issued at generation 0");
        (tree, body, pack(slot_of(body), 1))
    }

    #[test]
    fn a_stale_generation_names_nothing() {
        let (mut tree, body, stale) = body_and_stale();
        assert!(tree.nodes.get(body).is_some());
        assert!(tree.nodes.contains(body));
        assert!(
            tree.nodes.get(stale).is_none(),
            "the slot is live, the id is not"
        );
        assert!(!tree.nodes.contains(stale));
        assert!(tree.nodes.get_mut(stale).is_none());
        // A vacant slot answers the same.
        let vacant = pack(tree.nodes.slot_bound() + 10, 0);
        assert!(tree.nodes.get(vacant).is_none());
        // Iteration hands back each node's own id.
        assert!(tree.nodes.iter().any(|(id, n)| id == body && n.id == body));
    }

    #[test]
    #[should_panic(expected = "stale node id: slot 2 generation 1 is now held by generation 0")]
    fn indexing_with_a_stale_generation_panics_with_the_slot_and_both_generations() {
        let (tree, body, stale) = body_and_stale();
        assert_eq!(slot_of(body), 2, "root, html, body");
        let _ = &tree.nodes[stale];
    }

    #[test]
    #[should_panic(expected = "stale node id")]
    fn indexing_mutably_with_a_stale_generation_panics() {
        let (mut tree, _, stale) = body_and_stale();
        tree.nodes[stale].scroll_offset = (1.0, 1.0);
    }

    /// The debug assert in `get_live` is what makes a site that hands it an
    /// id from outside the tree fail the tests (#723 PR 2's safety net), so it
    /// is pinned. A release build does not compare (see the next test).
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "stale node id: slot 2 generation 1 is now held by generation 0")]
    fn get_live_with_a_stale_generation_panics_in_debug() {
        let (tree, body, stale) = body_and_stale();
        assert!(tree.nodes.get_live(body).is_some(), "positive control");
        let _ = tree.nodes.get_live(stale);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "stale node id")]
    fn get_live_mut_with_a_stale_generation_panics_in_debug() {
        let (mut tree, body, stale) = body_and_stale();
        assert!(tree.nodes.get_live_mut(body).is_some(), "positive control");
        let _ = tree.nodes.get_live_mut(stale);
    }

    /// The release half of the contract: no compare, the slot's node.
    #[test]
    #[cfg(not(debug_assertions))]
    fn get_live_does_not_compare_the_generation_in_release() {
        let (mut tree, body, stale) = body_and_stale();
        assert_eq!(tree.nodes.get_live(stale).map(|n| n.id), Some(body));
        assert_eq!(tree.nodes.get_live_mut(stale).map(|n| n.id), Some(body));
    }

    #[test]
    #[should_panic(expected = "a node must be inserted under the id vacant_key() gave it")]
    fn a_node_carrying_another_id_is_refused() {
        let mut tree = crate::node::NodeTree::new();
        let wrong = pack(tree.nodes.slot_bound() + 5, 0);
        let node = crate::node::Node::comment(wrong, "x", tree.guard.clone());
        tree.nodes.insert(node);
    }
}
