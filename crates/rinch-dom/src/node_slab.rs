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
//! **Freeing bumps the generation** (#723 PR 2). [`NodeSlab::remove`] leaves
//! the slot remembering the generation its next occupant gets, so a node
//! minted into a freed slot carries a new id and every id issued for the
//! slot's earlier occupants names nothing. `NodeTree::remove_subtree` is the
//! route that frees DOM nodes (`discard_node`, `set_inner_html`, pseudo
//! pruning) and clears the tree's side tables of the freed ids; the IFC passes
//! free their anonymous boxes directly. A slot is retired after
//! [`MAX_GENERATION`] (see there).

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

/// The highest generation a slot is issued at (2^21 − 1).
///
/// A packed id is then below 2^53, so it survives a JSON round trip through a
/// client that reads numbers as `f64` — the MCP host, any JavaScript client of
/// the debug protocol — exactly (the wire decision recorded on #723). A slot
/// whose occupant at this generation is freed is **retired**: it is never
/// handed out again, so no id is ever re-issued and none goes past 2^53. That
/// costs one slot (one empty entry) per 2^21 reuses of a single slot.
pub const MAX_GENERATION: u32 = (1 << 21) - 1;

/// One slot: its node, or the generation its next node will get and the next
/// free slot after it.
enum Entry {
    Occupied(Node),
    Vacant {
        /// The free slot after this one; `u32::MAX` for none (or a retired
        /// slot, which is on no list).
        next: u32,
        /// The generation the slot's next occupant is issued.
        generation: u32,
    },
}

const NO_SLOT: u32 = u32::MAX;

/// The nodes of a [`crate::node::NodeTree`], keyed by generation-checked id.
///
/// A slab with a free list, like `slab::Slab` (which it replaced, #723 PR 2),
/// with ids in place of slab keys: a vacant slot remembers the generation its
/// next occupant gets, one past its last occupant's, so an id issued for the
/// slot earlier never names the node stored there later. Freed slots are
/// reused last-freed first, as `slab::Slab` did. Iteration yields each node's
/// own id.
pub struct NodeSlab {
    entries: Vec<Entry>,
    /// The first free slot; `entries.len()` when none is free.
    next: usize,
    len: usize,
}

impl Default for NodeSlab {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next: 0,
            len: 0,
        }
    }
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

    /// The node in slot `slot`, whatever its generation.
    #[inline(always)]
    fn slot(&self, slot: usize) -> Option<&Node> {
        match self.entries.get(slot) {
            Some(Entry::Occupied(n)) => Some(n),
            _ => None,
        }
    }

    /// [`Self::slot`], mutably.
    #[inline(always)]
    fn slot_mut(&mut self, slot: usize) -> Option<&mut Node> {
        match self.entries.get_mut(slot) {
            Some(Entry::Occupied(n)) => Some(n),
            _ => None,
        }
    }

    /// The node `id` names, or `None` if its slot is vacant or holds a node of
    /// another generation.
    #[inline]
    pub fn get(&self, id: RawNodeId) -> Option<&Node> {
        match self.slot(slot_of(id)) {
            Some(n) if n.id == id => Some(n),
            _ => None,
        }
    }

    /// The node `id` names, mutably; `None` as [`Self::get`].
    #[inline]
    pub fn get_mut(&mut self, id: RawNodeId) -> Option<&mut Node> {
        match self.slot_mut(slot_of(id)) {
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
        let n = self.slot(slot_of(id))?;
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
        let n = self.slot_mut(slot_of(id))?;
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

    /// The id the next [`Self::insert`] will store its node under: the most
    /// recently freed slot at its next generation, or a new slot at
    /// generation 0. The node must carry it as its [`Node::id`].
    #[inline]
    pub fn vacant_key(&self) -> RawNodeId {
        match self.entries.get(self.next) {
            Some(Entry::Vacant { generation, .. }) => pack(self.next, *generation),
            _ => pack(self.next, 0),
        }
    }

    /// Store `node` under its own id, which must be [`Self::vacant_key`].
    /// Returns that id.
    pub fn insert(&mut self, node: Node) -> RawNodeId {
        let id = node.id;
        assert_eq!(
            self.vacant_key(),
            id,
            "a node must be inserted under the id vacant_key() gave it"
        );
        let slot = self.next;
        if slot == self.entries.len() {
            self.entries.push(Entry::Occupied(node));
            self.next = self.entries.len();
        } else {
            let Entry::Vacant { next, .. } =
                std::mem::replace(&mut self.entries[slot], Entry::Occupied(node))
            else {
                unreachable!("the free list names only vacant slots")
            };
            self.next = if next == NO_SLOT {
                self.entries.len()
            } else {
                next as usize
            };
        }
        self.len += 1;
        id
    }

    /// Take the node `id` names out of the slab, freeing its slot for a node
    /// of the next generation — or retiring it, past [`MAX_GENERATION`].
    ///
    /// # Panics
    ///
    /// If `id` names no live node.
    #[track_caller]
    pub fn remove(&mut self, id: RawNodeId) -> Node {
        let slot = slot_of(id);
        match self.slot(slot) {
            Some(n) if n.id == id => {}
            other => stale(id, other.map(|n| n.id)),
        }
        let generation = generation_of(id);
        let retire = generation >= MAX_GENERATION;
        let vacant = Entry::Vacant {
            next: if retire {
                NO_SLOT
            } else if self.next == self.entries.len() {
                NO_SLOT
            } else {
                self.next as u32
            },
            generation: generation.saturating_add(1),
        };
        let Entry::Occupied(node) = std::mem::replace(&mut self.entries[slot], vacant) else {
            unreachable!("checked above")
        };
        if !retire {
            self.next = slot;
        }
        self.len -= 1;
        node
    }

    /// How many nodes are stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no node is stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// One past the highest slot ever used: the length a table indexed by
    /// [`slot_of`] needs to cover every node.
    #[inline]
    pub fn slot_bound(&self) -> usize {
        self.entries.len()
    }

    /// Every node with its id, in slot order.
    #[inline]
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            entries: self.entries.iter(),
        }
    }

    /// Every node with its id, mutably, in slot order.
    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (RawNodeId, &mut Node)> + '_ {
        self.entries.iter_mut().filter_map(|e| match e {
            Entry::Occupied(n) => Some((n.id, n)),
            Entry::Vacant { .. } => None,
        })
    }
}

#[cfg(test)]
impl NodeSlab {
    /// Set the generation vacant slot `slot`'s next occupant gets.
    fn set_vacant_generation_for_tests(&mut self, slot: usize, to: u32) {
        match &mut self.entries[slot] {
            Entry::Vacant { generation, .. } => *generation = to,
            Entry::Occupied(_) => panic!("slot {slot} is occupied"),
        }
    }
}

/// [`NodeSlab::iter`]'s iterator.
pub struct Iter<'a> {
    entries: std::slice::Iter<'a, Entry>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (RawNodeId, &'a Node);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for e in self.entries.by_ref() {
            if let Entry::Occupied(n) = e {
                return Some((n.id, n));
            }
        }
        None
    }
}

impl<'a> IntoIterator for &'a NodeSlab {
    type Item = (RawNodeId, &'a Node);
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
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
        match self.slot(slot_of(id)) {
            Some(n) if n.id == id => n,
            other => stale(id, other.map(|n| n.id)),
        }
        #[cfg(not(debug_assertions))]
        match self.slot(slot_of(id)) {
            Some(n) => n,
            None => stale(id, None),
        }
    }
}

impl IndexMut<RawNodeId> for NodeSlab {
    #[inline]
    #[track_caller]
    fn index_mut(&mut self, id: RawNodeId) -> &mut Node {
        match self.slot_mut(slot_of(id)) {
            Some(n) => {
                #[cfg(debug_assertions)]
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

    fn comment(tree: &crate::node::NodeTree) -> crate::node::Node {
        crate::node::Node::comment(tree.nodes.vacant_key(), "x", tree.guard.clone())
    }

    #[test]
    fn a_freed_slot_is_reused_last_freed_first_under_the_next_generation() {
        let mut tree = crate::node::NodeTree::new();
        let a = tree.nodes.insert(comment(&tree));
        let b = tree.nodes.insert(comment(&tree));
        let len = tree.nodes.len();
        tree.nodes.remove(a);
        tree.nodes.remove(b);
        assert_eq!(tree.nodes.len(), len - 2);
        assert!(!tree.nodes.contains(a) && !tree.nodes.contains(b));
        // Last freed first, as `slab::Slab` did.
        let b2 = tree.nodes.insert(comment(&tree));
        let a2 = tree.nodes.insert(comment(&tree));
        assert_eq!((slot_of(b2), generation_of(b2)), (slot_of(b), 1));
        assert_eq!((slot_of(a2), generation_of(a2)), (slot_of(a), 1));
        assert!(tree.nodes.get(a).is_none(), "the old id names nothing");
        assert_eq!(tree.nodes.get(a2).map(|n| n.id), Some(a2));
        // A third occupant gets generation 2.
        tree.nodes.remove(a2);
        let a3 = tree.nodes.insert(comment(&tree));
        assert_eq!((slot_of(a3), generation_of(a3)), (slot_of(a), 2));
        assert_eq!(
            tree.nodes
                .iter()
                .filter(|(id, _)| slot_of(*id) == slot_of(a))
                .count(),
            1
        );
    }

    /// The wire decision on #723: no id is ever past 2^53, so a JSON client
    /// that reads numbers as `f64` gets it exactly. A slot whose occupant at
    /// [`MAX_GENERATION`] is freed is retired rather than wrapped, so no id is
    /// ever issued twice either.
    #[test]
    fn a_slot_past_the_last_generation_is_retired_not_wrapped() {
        assert!(pack(u32::MAX as usize, MAX_GENERATION) < (1u64 << 53));
        assert!(
            (pack(u32::MAX as usize, MAX_GENERATION) as f64) as u64
                == pack(u32::MAX as usize, MAX_GENERATION)
        );
        let mut tree = crate::node::NodeTree::new();
        let a = tree.nodes.insert(comment(&tree));
        let slot = slot_of(a);
        tree.nodes.remove(a);
        tree.nodes
            .set_vacant_generation_for_tests(slot, MAX_GENERATION);
        let last = tree.nodes.insert(comment(&tree));
        assert_eq!((slot_of(last), generation_of(last)), (slot, MAX_GENERATION));
        let bound = tree.nodes.slot_bound();
        tree.nodes.remove(last);
        let next = tree.nodes.insert(comment(&tree));
        assert_ne!(
            slot_of(next),
            slot,
            "the retired slot is not handed out again"
        );
        assert_eq!(generation_of(next), 0);
        assert_eq!(slot_of(next), bound, "a new slot instead");
        assert!(tree.nodes.get(last).is_none());
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
