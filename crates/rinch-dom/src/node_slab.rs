//! The node storage: a slab whose keys are generation-checked ids (issue #723).
//!
//! A [`RawNodeId`] is the same packed `u64` as `rinch_core::dom::NodeId`: the
//! slot index in the low 32 bits, the slot's generation in the high 32. Every
//! lookup compares the id it was given against the id the stored node carries
//! ([`Node::id`]), so an id issued for a slot's earlier occupant resolves to
//! nothing — [`NodeSlab::get`] answers `None` and indexing panics — instead of
//! naming whichever node holds the slot now.
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
    debug_assert!(slot <= u32::MAX as usize, "node slot {slot} exceeds 32 bits");
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

impl Index<RawNodeId> for NodeSlab {
    type Output = Node;

    /// The node `id` names.
    ///
    /// # Panics
    ///
    /// If `id` names no live node — its slot is vacant, or holds a node of
    /// another generation. Use [`NodeSlab::get`] for an id from outside the
    /// tree (an app's handle, an event target, a side table).
    #[inline]
    #[track_caller]
    fn index(&self, id: RawNodeId) -> &Node {
        match self.slab.get(slot_of(id)) {
            Some(n) if n.id == id => n,
            other => stale(id, other.map(|n| n.id)),
        }
    }
}

impl IndexMut<RawNodeId> for NodeSlab {
    #[inline]
    #[track_caller]
    fn index_mut(&mut self, id: RawNodeId) -> &mut Node {
        match self.slab.get_mut(slot_of(id)) {
            Some(n) => {
                if n.id != id {
                    stale(id, Some(n.id));
                }
                n
            }
            None => stale(id, None),
        }
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
}
