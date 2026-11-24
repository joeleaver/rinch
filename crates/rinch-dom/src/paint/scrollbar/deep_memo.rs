//! The memo of each scroll container's descendant walk ([`DeepOverflow`]):
//! kept, and brought up to date in place, as what the walk read changes.

use super::deep_mark::Marker;
use super::deep_record::{AGAIN, BELOW, Level, Rec, Records, SELF, Walker};
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
/// it up to its container, listing each marked box in its parent's list of
/// marked children (`deep_mark::Marker`); a parent a subtree left, or one
/// that gained a box with no record, is marked to sum all its children
/// again; a box met under another parent than the one it is under now has
/// its records dropped (it is recorded again where it is). Then each
/// container marked is brought up to date (`deep_record::Walker::visit`):
/// from the container down, only the marked boxes are visited; each is
/// measured where it is now, and what is below it is taken from its record
/// and brought up to date from its marked children alone, unless its
/// children are to be summed again. Records are kept in each box's own
/// frame, so a box that moved carries what is below it. A box with no
/// record (one added) is walked and recorded. So a row removed from a long
/// list measures the list and each row after it, where it is now, and
/// nothing below them; a box resized measures it and the boxes above it,
/// and visits none of their other children.
///
/// A change the records cannot follow walks the container again: a box
/// whose kind changed (it clips, or stops; is transformed, positioned,
/// displayed, collapsed or contained differently; its padding or border
/// moved its content), an element outside the box tree changed (a split
/// inline, which may be an absolute box's containing block), or the
/// container's own containing-block bits. A whole-document restyle drops
/// every entry.
///
/// **Records live as long as their boxes are where the walks met them.** A
/// subtree taken out of the tree (`forget_detached`, from every removal's
/// `mark_subtree_paint_dirty`) or freed (`forget_freed`) drops its records,
/// and the walks of the scroll containers in it; an entry dropped to be
/// walked again drops the records its walk made. The parent a dropped
/// record was under is marked at the next check to sum its children again.
#[derive(Debug, Default)]
pub(crate) struct DeepOverflow {
    pub(super) state: std::cell::RefCell<DeepOverflowState>,
}

#[derive(Debug, Default)]
pub(super) struct DeepOverflowState {
    pub(super) entries: super::deep_slots::Slots<DeepEntry>,
    /// Every entry's records, by box ([`Records`]).
    pub(super) recs: Records,
    /// How much of `paint_dirty_nodes` and `bar_check_parents` has been
    /// checked since the last paint consumed them.
    checked: (usize, usize),
    /// The parents of records dropped since the last check
    /// ([`Self::forget`]): each lost a child the next check sums them
    /// without.
    lost: Vec<RawNodeId>,
    /// The current check, counted from 1: what a record's marks belong to
    /// (`deep_record::Rec::epoch`).
    pub(super) epoch: u64,
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
        let entry = state
            .entries
            .get(&container_id)
            .expect("walked when it had no entry");
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

    /// Walk `container_id`'s subtree and record it.
    fn walk(tree: &NodeTree, state: &mut DeepOverflowState, container_id: RawNodeId) {
        let mut live = Vec::new();
        let DeepOverflowState { recs, epoch, .. } = state;
        let mut walker = Walker {
            tree,
            owner: container_id,
            recs,
            live: &mut live,
            measured: 0,
            again: false,
            epoch: *epoch,
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

    /// Called by `NodeTree::remove_subtree` (and wherever else a node is
    /// freed) with the ids it frees: their records and walks go now, so
    /// nothing keeps a freed box's record, and a later node given its slot
    /// finds none.
    pub(crate) fn forget_freed(&self, ids: &[RawNodeId]) {
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        if state.entries.is_empty() && state.recs.is_empty() {
            return;
        }
        for id in ids {
            if !state.entries.is_empty() {
                state.entries.remove(id);
            }
            state.forget(*id);
        }
    }

    /// Called before box `root`'s subtree is taken out of the tree (every
    /// removal and move records its pixels first, `mark_subtree_paint_dirty`):
    /// the walks of the scroll containers in it, and the records of the
    /// boxes in it, go. Put back, it is walked again where it is.
    pub(crate) fn forget_detached(&self, tree: &NodeTree, root: RawNodeId) {
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        if state.entries.is_empty() {
            // Nothing is kept: the next check drops any records left.
            return;
        }
        let inside: Vec<RawNodeId> = state
            .entries
            .keys()
            .copied()
            .filter(|&e| within(tree, e, root))
            .collect();
        for container in inside {
            state.drop_entry(tree, container);
        }
        state.purge(tree, root);
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
            state.lost.clear();
            return;
        }
        if tree.whole_document_damaged {
            if !state.whole_dropped {
                state.entries.clear();
                state.recs = Records::default();
                state.lost.clear();
                state.whole_dropped = true;
            }
            return;
        }
        if from == state.checked && state.lost.is_empty() {
            return;
        }
        state.epoch += 1;
        let mut lost = std::mem::take(&mut state.lost);
        let mut marker = Marker::new(tree, state);
        // The parents that lost a child first: each sums all its children
        // again, so the boxes marked below it need not be listed in it.
        for &id in parents[from.1..].iter().chain(lost.iter()) {
            marker.note(id, true);
        }
        for &id in &dirty[from.0..] {
            marker.note(id, false);
        }
        // The parents of records the marking dropped (a box moved), in this
        // same check.
        loop {
            lost.clear();
            std::mem::swap(&mut lost, &mut marker.state.lost);
            if lost.is_empty() {
                break;
            }
            for &id in &lost {
                marker.note(id, true);
            }
        }
        state.lost = lost;
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
            let Some(node) = tree.get(container) else {
                // Freed: `forget_freed` dropped its records.
                state.entries.remove(&container);
                continue;
            };
            let DeepOverflowState {
                entries,
                recs,
                measured,
                epoch,
                ..
            } = &mut *state;
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
                    epoch: *epoch,
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
                state.drop_entry(tree, container);
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

    /// How many walks and records the memo holds
    /// (`testing::deep_overflow_held`).
    pub(crate) fn held(&self) -> (usize, usize) {
        let state = self.state.borrow();
        (state.entries.len(), state.recs.len())
    }
}

impl DeepOverflowState {
    /// Drop box `id`'s record, if it has one, and note its parent as one
    /// that lost a child.
    pub(super) fn forget(&mut self, id: RawNodeId) {
        let Some(rec) = self.drop_rec(id) else {
            return;
        };
        if self.recs.contains_key(&rec.parent) || self.entries.contains_key(&rec.parent) {
            self.lost.push(rec.parent);
        }
    }

    /// Drop box `id`'s record, and its place in its walk's transformed
    /// boxes; the record.
    fn drop_rec(&mut self, id: RawNodeId) -> Option<Rec> {
        let rec = self.recs.remove(&id)?;
        if rec.snap.transformed
            && let Some(entry) = self.entries.get_mut(&rec.owner)
        {
            entry.live.retain(|&l| l != id);
        }
        Some(rec)
    }

    /// Drop the records of box `id` and of every recorded box below it.
    /// `id`'s parent is noted as one that lost a child, and so is a scroll
    /// container below it whose walk's records go (moved with it, its walk is
    /// kept, and its children are recorded again); the other parents below
    /// it go too, and are not noted (a hidden branch of 1000 rows noted each,
    /// and the next check climbed from each, `branch_hide` in `rinch-bench`).
    pub(super) fn purge(&mut self, tree: &NodeTree, id: RawNodeId) {
        if !self.recs.contains_key(&id) {
            return;
        }
        self.forget(id);
        self.drop_below(tree, id);
    }

    /// Drop the records of every recorded box below box `id`, noting a
    /// scroll container whose walk's records they were ([`Self::purge`]).
    fn drop_below(&mut self, tree: &NodeTree, id: RawNodeId) {
        for &child in crate::RinchDocument::box_tree_children(&tree.nodes, id).iter() {
            let Some(rec) = self.drop_rec(child) else {
                continue;
            };
            if rec.parent == rec.owner
                && self.lost.last() != Some(&rec.owner)
                && self.entries.contains_key(&rec.owner)
            {
                self.lost.push(rec.owner);
            }
            self.drop_below(tree, child);
        }
    }

    /// Drop `container`'s walk and the records it made.
    fn drop_entry(&mut self, tree: &NodeTree, container: RawNodeId) {
        self.entries.remove(&container);
        self.purge_owned(tree, container, container);
    }

    /// Drop the records `owner`'s walk made below box `id`.
    fn purge_owned(&mut self, tree: &NodeTree, owner: RawNodeId, id: RawNodeId) {
        for &child in crate::RinchDocument::box_tree_children(&tree.nodes, id).iter() {
            if self.recs.get(&child).is_some_and(|r| r.owner == owner) {
                self.recs.remove(&child);
                self.purge_owned(tree, owner, child);
            }
        }
    }
}

/// Whether box `id` is `root` or below it in the box tree.
fn within(tree: &NodeTree, id: RawNodeId, root: RawNodeId) -> bool {
    let mut at = Some(id);
    while let Some(a) = at {
        if a == root {
            return true;
        }
        at = tree.get(a).and_then(|n| n.box_tree_parent());
    }
    false
}
