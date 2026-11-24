//! The memo of each scroll container's descendant walk ([`DeepOverflow`]):
//! kept, and brought up to date in place, as what the walk read changes.

use super::deep_record::{AGAIN, BELOW, Level, Records, SELF, Walker};
use super::deep_walk::WalkSnap;
use super::extents::Reach;
use super::measure::border_box_in;
use crate::NodeTree;
use crate::node::RawNodeId;

/// One container's kept walk.
#[derive(Debug, Clone)]
pub(super) struct DeepEntry {
    pub(super) reach: Reach,
    /// The transformed boxes the walk met, not entered: measured on each
    /// read, where they are drawn ([`border_box_in`]).
    pub(super) live: Vec<RawNodeId>,
    /// What the walk read of the container itself: whether it is the
    /// containing block of an absolute or a fixed box below it decides
    /// whether that box is its overflow.
    pub(super) own: WalkSnap,
    /// What was pushed of the container or below it since the last check
    /// (`deep_record::SELF`, `BELOW`, `AGAIN`).
    pub(super) touch: u8,
}

/// [`deep_overflow`](super::deep_walk::deep_overflow)'s memo: each scroll
/// container's walk, kept with a record of every box it met
/// (`deep_record::Rec`), and brought up to date in place as they change
/// (#2054).
///
/// Every read first checks the boxes pushed paint-dirty, and the parents a
/// subtree left (`NodeTree::bar_check_parents`), since the last check — so
/// also before a paint consumes the tree and drains those lists. A box that
/// changes anything the walk reads (its box, its lines, whether it clips,
/// is transformed, positioned, displayed or collapsed) is pushed there,
/// whether or not it moved since the last paint.
///
/// The check marks each recorded box pushed, and each recorded box above
/// it up to its container; a parent a subtree left is marked as having
/// something below it changed, and a box met under another parent than the
/// one it is under now has its records dropped (it is recorded again where
/// it is). Then each container marked is brought up to date
/// (`deep_record::Walker::visit`): from the container down, only the marked
/// boxes are visited; each is measured where it is now, and what is below it
/// is taken from its record unless something below it was marked. Records
/// are kept in each box's own frame, so a box that moved carries what is
/// below it. A box with no record (one added) is walked and recorded; a
/// freed box's record is dropped. So a row removed from a long list
/// measures the list and each row after it, where it is now, and nothing
/// below them; a row resized measures it and the boxes above it.
///
/// A change the records cannot follow walks the container again: a box
/// whose kind changed (it clips, or stops; is transformed, positioned,
/// displayed, collapsed or contained differently; its padding or border
/// moved its content), an element outside the box tree changed (a split
/// inline, which may be an absolute box's containing block), or the
/// container's own containing-block bits. A whole-document restyle drops
/// every entry; a removed container's entry is freed.
#[derive(Debug, Default)]
pub(crate) struct DeepOverflow {
    state: std::cell::RefCell<DeepOverflowState>,
}

#[derive(Debug, Default)]
pub(super) struct DeepOverflowState {
    entries: rustc_hash::FxHashMap<RawNodeId, DeepEntry>,
    /// Every entry's records, by box ([`Records`]).
    recs: Records,
    /// How much of `paint_dirty_nodes` and `bar_check_parents` has been
    /// checked since the last paint consumed them.
    checked: (usize, usize),
    /// Whether this paint's whole-document restyle has dropped the entries.
    whole_dropped: bool,
    /// Walks of a container's whole subtree, for the tests that pin the memo.
    walks: u64,
    /// Boxes measured, walked or brought up to date.
    measured: u64,
}

impl DeepOverflow {
    pub(super) fn reach(&self, tree: &NodeTree, container_id: RawNodeId) -> Reach {
        let mut state = self.state.borrow_mut();
        Self::check(tree, &mut state);
        if !state.entries.contains_key(&container_id) {
            Self::walk(tree, &mut state, container_id);
        }
        let entry = &state.entries[&container_id];
        let mut reach = entry.reach;
        for &id in &entry.live {
            if let Some(drawn) = border_box_in(tree, id, container_id)
                && (drawn.width() > 0.0 || drawn.height() > 0.0)
            {
                reach.grow(drawn.x0, drawn.x1, drawn.y1);
            }
        }
        reach
    }

    /// Walk `container_id`'s subtree and record it (overwriting any record
    /// left from an entry dropped before).
    fn walk(tree: &NodeTree, state: &mut DeepOverflowState, container_id: RawNodeId) {
        let mut live = Vec::new();
        let DeepOverflowState { recs, .. } = state;
        let mut walker = Walker {
            tree,
            owner: container_id,
            recs,
            live: &mut live,
            measured: 0,
            again: false,
        };
        walker.children(container_id, Level::Top, true);
        let reach = walker.top_reach(container_id);
        let measured = walker.measured;
        let own = match tree.get(container_id) {
            Some(node) => WalkSnap::of(tree, node, false),
            None => return,
        };
        state.measured += measured;
        state.walks += 1;
        state.entries.insert(
            container_id,
            DeepEntry {
                reach,
                live,
                own,
                touch: 0,
            },
        );
    }

    /// Called by `NodeTree::consume_paint_dirty` before it drains the lists
    /// [`Self::check`] reads: what changed since the last check is checked
    /// here, while they still name it.
    pub(crate) fn before_paint_consumed(&self, tree: &NodeTree) {
        Self::check(tree, &mut self.state.borrow_mut());
    }

    /// Called by `NodeTree::consume_paint_dirty` once it has drained both
    /// lists and cleared `whole_document_damaged`: the next check starts from
    /// the empty lists. Not in [`Self::before_paint_consumed`]: the consume
    /// reads the bars it records ([`scrollbars`](super::scrollbars)) before
    /// it clears `bar_check_parents`, and a check there that started again
    /// from 0 took the parents it had already checked as new, and measured
    /// them again on every frame a subtree left (as a whole-document frame's
    /// would drop every entry twice).
    pub(crate) fn after_paint_consumed(&self) {
        let mut state = self.state.borrow_mut();
        state.checked = (0, 0);
        state.whole_dropped = false;
    }

    /// Check what was pushed since the last check, and bring each entry it
    /// reached up to date (see [`DeepOverflow`]).
    fn check(tree: &NodeTree, state: &mut DeepOverflowState) {
        let dirty = &tree.paint_dirty_nodes;
        let parents = &tree.bar_check_parents;
        // A list shorter than what was checked was drained some other way:
        // check it all.
        if state.checked.0 > dirty.len() {
            state.checked.0 = 0;
        }
        if state.checked.1 > parents.len() {
            state.checked.1 = 0;
        }
        let from = state.checked;
        state.checked = (dirty.len(), parents.len());
        if state.entries.is_empty() {
            if !state.recs.is_empty() {
                state.recs = Records::default();
            }
            return;
        }
        if tree.whole_document_damaged {
            if !state.whole_dropped {
                state.entries.clear();
                state.recs = Records::default();
                state.whole_dropped = true;
            }
            return;
        }
        if from == state.checked {
            return;
        }
        // The memo's keys and the two lists can outlive their nodes (the
        // lists are not pruned when a node is freed, `NodeTree::remove_subtree`),
        // and a freed slot may hold a new node: checked lookups.
        state.entries.retain(|&id, _| tree.get(id).is_some());
        let mut marker = Marker {
            tree,
            state,
            last: None,
        };
        for (&id, lost_child) in dirty[from.0..]
            .iter()
            .map(|id| (id, false))
            .chain(parents[from.1..].iter().map(|id| (id, true)))
        {
            marker.note(id, lost_child);
        }
        Self::bring_up_to_date(tree, state);
    }

    /// Bring every entry the check marked up to date, or drop it to be
    /// walked again where the records cannot follow.
    fn bring_up_to_date(tree: &NodeTree, state: &mut DeepOverflowState) {
        let marked: Vec<RawNodeId> = state
            .entries
            .iter()
            .filter(|(_, e)| e.touch != 0)
            .map(|(&id, _)| id)
            .collect();
        for container in marked {
            let DeepOverflowState {
                entries,
                recs,
                measured,
                ..
            } = state;
            let Some(node) = tree.get(container) else {
                entries.remove(&container);
                continue;
            };
            let Some(entry) = entries.get_mut(&container) else {
                continue;
            };
            let touch = std::mem::take(&mut entry.touch);
            let own = WalkSnap::of(tree, node, false);
            let mut again = touch & AGAIN != 0
                || !(node.scrolls_x() || node.scrolls_y())
                || (touch & SELF != 0 && !own.same_kind(&entry.own));
            if !again && touch & BELOW != 0 {
                let mut walker = Walker {
                    tree,
                    owner: container,
                    recs,
                    live: &mut entry.live,
                    measured: 0,
                    again: false,
                };
                walker.children(container, Level::Top, false);
                *measured += walker.measured;
                if walker.again {
                    again = true;
                } else {
                    entry.reach = walker.top_reach(container);
                }
            }
            if again {
                entries.remove(&container);
            } else {
                entry.own = own;
            }
        }
    }

    /// How many times the memo has walked a container's whole subtree: a test
    /// reads it to see a range answered from the memo
    /// (`testing::deep_overflow_walks`).
    pub(crate) fn walks(&self) -> u64 {
        self.state.borrow().walks
    }

    /// How many boxes the memo has measured, in walks and in bringing its
    /// records up to date (`testing::deep_overflow_measured`).
    pub(crate) fn measured(&self) -> u64 {
        self.state.borrow().measured
    }
}

/// The check's marking of what was pushed ([`DeepOverflow`]).
struct Marker<'a> {
    tree: &'a NodeTree,
    state: &'a mut DeepOverflowState,
    /// The box the last upward marking started from: the rows of a list
    /// share their parent, and the second marks nothing the first did not.
    last: Option<RawNodeId>,
}

impl Marker<'_> {
    /// Note box `id`, pushed paint-dirty or (`lost_child`) a parent a subtree
    /// left.
    fn note(&mut self, id: RawNodeId, lost_child: bool) {
        let flag = if lost_child { BELOW } else { SELF };
        if let Some(entry) = self.state.entries.get_mut(&id) {
            entry.touch |= flag;
        }
        let tree = self.tree;
        let Some(node) = tree.get(id) else {
            // Freed: its record goes, and its parent was pushed as one a
            // subtree left.
            self.forget(id);
            return;
        };
        if node.is_text() {
            // Its lines are its IFC root's, which is pushed when they move.
            return;
        }
        let parent = node.box_tree_parent();
        let Some(rec) = self.state.recs.get_mut(&id) else {
            if node.is_split_inline() {
                // Not a box, so not recorded, but its style may decide an
                // absolute box's containing block below it.
                if let Some(owner) = self.mark_from(parent) {
                    if let Some(entry) = self.state.entries.get_mut(&owner) {
                        entry.touch |= AGAIN;
                    }
                }
            } else {
                // A box added (or one below a box the walk did not enter,
                // which the marking stops at).
                self.mark_from(parent);
            }
            return;
        };
        rec.touch |= flag;
        let recorded = rec.parent;
        if parent != Some(recorded) {
            // Moved, or taken out, since it was met: its records go, its old
            // parent lost it, and where it is now records it again.
            self.purge(id);
            self.last = None;
            self.mark_from(Some(recorded));
            self.last = None;
            self.mark_from(parent);
            return;
        }
        self.mark_from(parent);
    }

    /// Mark the recorded boxes from `start` up to the container whose walk
    /// met them as having something below them changed; the container's
    /// entry, if the marking reaches one. Stops at a box the walk did not go
    /// into (nothing below it is read), and where an earlier marking passed.
    fn mark_from(&mut self, start: Option<RawNodeId>) -> Option<RawNodeId> {
        if start.is_some() && start == self.last {
            return None;
        }
        self.last = start;
        let mut current = start;
        while let Some(a) = current {
            if let Some(entry) = self.state.entries.get_mut(&a) {
                entry.touch |= BELOW;
                return Some(a);
            }
            match self.state.recs.get_mut(&a) {
                Some(rec) => {
                    if !rec.snap.entered() {
                        return None;
                    }
                    if rec.touch & BELOW != 0
                        && self
                            .state
                            .entries
                            .get(&rec.owner)
                            .is_some_and(|e| e.touch & BELOW != 0)
                    {
                        return Some(rec.owner);
                    }
                    rec.touch |= BELOW;
                    current = Some(rec.parent);
                }
                None => current = self.tree.get(a).and_then(|n| n.box_tree_parent()),
            }
        }
        None
    }

    /// Drop freed box `id`'s record.
    fn forget(&mut self, id: RawNodeId) {
        if let Some(rec) = self.state.recs.remove(&id)
            && rec.snap.transformed
            && let Some(entry) = self.state.entries.get_mut(&rec.owner)
        {
            entry.live.retain(|&l| l != id);
        }
    }

    /// Drop the records of box `id` and of every recorded box below it.
    fn purge(&mut self, id: RawNodeId) {
        self.forget(id);
        let tree = self.tree;
        for &child in crate::RinchDocument::box_tree_children(&tree.nodes, id).iter() {
            if self.state.recs.contains_key(&child) {
                self.purge(child);
            }
        }
    }
}
