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
//! # Freeing a slot bumps its generation
//!
//! [`NodeSlab::remove`] frees the slot and advances its generation, so the
//! next [`NodeSlab::vacant_key`] for that slot names a generation no earlier
//! id carries: a handle kept from before the free resolves to nothing through
//! the checked lookups, and a `DomDocument` verb handed it is a no-op. Every
//! slot starts at generation 0, so an id of a slot never freed is just its
//! index (`NodeId(4)`).
//!
//! The generation wraps at [`GENERATION_BITS`] (21) bits, so a packed id is
//! below 2^53 and survives a trip through a JSON number parsed as an `f64`
//! (the debug protocol, the MCP server, DevTools, `rinch-web`'s `__nid`):
//! see [`GENERATION_BITS`] for the trade (issue #723).

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

/// How many bits of a slot's generation are used: it wraps after
/// `2^GENERATION_BITS` frees of one slot (issue #723's wire decision).
///
/// 32 index bits plus 21 generation bits is 53, so every id is an integer an
/// `f64` holds exactly, and a JavaScript JSON client (the MCP host, the debug
/// protocol's browser tools) reads back the id it was sent. The price is that
/// a handle kept across exactly 2^21 (about two million) reuses of its one
/// slot names that slot's occupant again; any other stale handle names
/// nothing. Ids stay plain JSON numbers, so no client changes.
pub const GENERATION_BITS: u32 = 21;

/// The largest generation a slot reaches before it wraps to 0.
pub const MAX_GENERATION: u32 = (1 << GENERATION_BITS) - 1;

/// The generation after `generation`, wrapping at [`GENERATION_BITS`].
#[inline(always)]
pub fn next_generation(generation: u32) -> u32 {
    (generation + 1) & MAX_GENERATION
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
    /// The generation the next node stored in each slot gets, by slot. A slot
    /// past the end has never been freed: generation 0.
    generations: Vec<u32>,
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
        let slot = self.slab.vacant_key();
        pack(slot, self.generation_of_slot(slot))
    }

    /// The generation the next node stored in `slot` gets.
    #[inline]
    fn generation_of_slot(&self, slot: usize) -> u32 {
        self.generations.get(slot).copied().unwrap_or(0)
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

    /// Take the node `id` names out of the slab, freeing its slot and
    /// advancing the slot's generation, so `id` names nothing from here on
    /// (and does not come back until the generation wraps, [`GENERATION_BITS`]).
    ///
    /// # Panics
    ///
    /// If `id` names no live node.
    #[track_caller]
    pub fn remove(&mut self, id: RawNodeId) -> Node {
        let slot = slot_of(id);
        match self.slab.get(slot) {
            Some(n) if n.id == id => {
                if self.generations.len() <= slot {
                    self.generations.resize(slot + 1, 0);
                }
                self.generations[slot] = next_generation(generation_of(id));
                self.slab.remove(slot)
            }
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

    /// A comment node in a fresh slot of `tree`.
    fn insert_comment(tree: &mut crate::node::NodeTree) -> RawNodeId {
        let id = tree.nodes.vacant_key();
        let node = crate::node::Node::comment(id, "x", tree.guard.clone());
        tree.nodes.insert(node)
    }

    #[test]
    fn freeing_a_slot_moves_its_generation_on() {
        let mut tree = crate::node::NodeTree::new();
        let first = insert_comment(&mut tree);
        assert_eq!(generation_of(first), 0, "a slot never freed: generation 0");
        tree.nodes.remove(first);
        assert!(!tree.nodes.contains(first));
        let second = insert_comment(&mut tree);
        assert_eq!(
            slot_of(second),
            slot_of(first),
            "the free list reuses the slot"
        );
        assert_eq!(generation_of(second), 1, "#723: under the next generation");
        assert!(tree.nodes.contains(second));
        assert!(!tree.nodes.contains(first), "the old id names nothing");
        assert!(tree.nodes.get(first).is_none());
    }

    /// The wire decision (#723): the generation wraps at 21 bits, so every id
    /// is below 2^53 and an `f64` holds it exactly.
    #[test]
    fn the_generation_wraps_at_21_bits_so_every_id_fits_an_f64() {
        assert_eq!(next_generation(0), 1);
        assert_eq!(next_generation(MAX_GENERATION - 1), MAX_GENERATION);
        assert_eq!(next_generation(MAX_GENERATION), 0, "wraps");
        let largest = pack(u32::MAX as usize, MAX_GENERATION);
        assert!(largest < 1u64 << 53, "the largest id is below 2^53");
        assert_eq!(largest as f64 as u64, largest, "and survives an f64");
        let mut tree = crate::node::NodeTree::new();
        let id = insert_comment(&mut tree);
        let slot = slot_of(id);
        tree.nodes.remove(id);
        tree.nodes.generations[slot] = MAX_GENERATION;
        let last = insert_comment(&mut tree);
        assert_eq!(generation_of(last), MAX_GENERATION);
        tree.nodes.remove(last);
        let wrapped = insert_comment(&mut tree);
        assert_eq!((slot_of(wrapped), generation_of(wrapped)), (slot, 0));
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
