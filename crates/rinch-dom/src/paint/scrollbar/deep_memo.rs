//! The memo of each scroll container's descendant walk ([`DeepOverflow`]):
//! kept until something the walk read changes where it could move the reach.

use super::deep_walk::{
    Snaps, Span, WalkSnap, box_origin, containing_rect, lines_span, walk_deep_overflow, walk_out,
};
use super::extents::Reach;
use super::measure::border_box_in;
use crate::NodeTree;
use crate::node::RawNodeId;

/// One container's kept [`walk_deep_overflow`].
#[derive(Debug, Clone)]
pub(super) struct DeepEntry {
    pub(super) reach: Reach,
    /// The transformed boxes the walk met, not entered: measured on each
    /// read, where they are drawn ([`border_box_in`]).
    pub(super) live: Vec<RawNodeId>,
    /// What the walk read of every box it met ([`WalkSnap`]), and found
    /// below it.
    pub(super) snaps: Snaps,
}

/// [`deep_overflow`](super::deep_walk::deep_overflow)'s memo: each scroll container's walk, kept until
/// something it read changes (#2054).
///
/// Every read first checks the boxes pushed paint-dirty, and the parents a
/// subtree left (`NodeTree::bar_check_parents`), since the last check — so
/// also before a paint consumes the tree and drains those lists. A box that
/// changes anything the walk reads (its box, its lines, whether it clips,
/// is transformed, positioned, displayed or collapsed) is pushed there,
/// whether or not it moved since the last paint. For each such box, the
/// entry of each container above it is dropped unless the walk recorded
/// that box with the same [`WalkSnap`], or recorded a box between them as a
/// stop (one it did not enter) that is itself unchanged: a hover over a row
/// keeps its scroller's entry, a box toggled to `overflow: hidden`, a
/// change and change-back between two paints, or a box added under one the
/// walk entered drops it. A box whose snapshot differs in its box or lines
/// alone keeps the entry when nothing of it was counted and all of it is
/// still inside its top box ([`moved_inside`], [`Keep::top`](super::deep_walk::Keep::top)): a row
/// resized in a page walks nothing. A parent a subtree left drops every entry above
/// it short of such a stop. A whole-document restyle drops every entry;
/// an entry of a container no longer in the tree is freed.
#[derive(Debug, Default)]
pub(crate) struct DeepOverflow {
    state: std::cell::RefCell<DeepOverflowState>,
}

#[derive(Debug, Default)]
pub(super) struct DeepOverflowState {
    entries: std::collections::HashMap<RawNodeId, DeepEntry>,
    /// How much of `paint_dirty_nodes` and `bar_check_parents` has been
    /// checked since the last paint consumed them.
    checked: (usize, usize),
    /// Whether this paint's whole-document restyle has dropped the entries.
    whole_dropped: bool,
    /// Walks made, for the tests that pin the memo.
    walks: u64,
    /// The record map of the biggest entry dropped since the last walk,
    /// emptied: the next walk fills it ([`walk_deep_overflow`]). A walk
    /// after a drop is most often of the same subtree, and growing its map
    /// from nothing rehashed every record several times over.
    spare: Snaps,
}

impl DeepOverflowState {
    /// Keep a dropped entry's record map for the next walk, if it is the
    /// biggest dropped since the last one.
    fn recycle(&mut self, mut snaps: Snaps) {
        if snaps.capacity() > self.spare.capacity() {
            snaps.clear();
            self.spare = snaps;
        }
    }
}

impl DeepOverflow {
    pub(super) fn reach(&self, tree: &NodeTree, container_id: RawNodeId) -> Reach {
        let mut state = self.state.borrow_mut();
        Self::check(tree, &mut state);
        if !state.entries.contains_key(&container_id) {
            let snaps = std::mem::take(&mut state.spare);
            let entry = walk_deep_overflow(tree, container_id, snaps);
            state.walks += 1;
            state.entries.insert(container_id, entry);
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
    /// from 0 took the parents it had already checked as new, and dropped —
    /// and walked again — every entry above them, on every frame a subtree
    /// left (as a whole-document frame's would drop every entry twice).
    pub(crate) fn after_paint_consumed(&self) {
        let mut state = self.state.borrow_mut();
        state.checked = (0, 0);
        state.whole_dropped = false;
    }

    /// Check what was pushed since the last check (see [`DeepOverflow`]).
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
            return;
        }
        if tree.whole_document_damaged {
            if !state.whole_dropped {
                let dropped = std::mem::take(&mut state.entries);
                for (_, entry) in dropped {
                    state.recycle(entry.snaps);
                }
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
        let mut chain: Vec<RawNodeId> = Vec::new();
        for (&id, forced) in dirty[from.0..]
            .iter()
            .map(|id| (id, false))
            .chain(parents[from.1..].iter().map(|id| (id, true)))
        {
            if state.entries.is_empty() {
                return;
            }
            let Some(node) = tree.get(id) else {
                continue;
            };
            let now = WalkSnap::of(tree, node, false);
            chain.clear();
            let mut current = Some(id);
            while let Some(a) = current {
                chain.push(a);
                current = tree.get_live(a).and_then(|n| n.parent);
            }
            for j in 0..chain.len() {
                let Some(entry) = state.entries.get_mut(&chain[j]) else {
                    continue;
                };
                let found = if j > 0 {
                    chain[..j]
                        .iter()
                        .enumerate()
                        .find_map(|(i, a)| entry.snaps.get(a).map(|rec| (i, rec.snap)))
                } else {
                    None
                };
                let keep = found.is_some_and(|(i, snap)| {
                    if i == 0 {
                        // The box itself: unchanged, and not a parent a
                        // subtree left (unless the walk stopped there) —
                        // or changed where nothing it adds is counted.
                        if forced {
                            same_snap(&snap, &now) && snap.is_stop()
                        } else {
                            same_snap(&snap, &now) || moved_inside(tree, chain[j], entry, id)
                        }
                    } else {
                        // A box between: a stop the walk did not enter,
                        // as it was.
                        snap.is_stop()
                            && tree
                                .get_live(chain[i])
                                .is_some_and(|n| same_snap(&snap, &WalkSnap::of(tree, n, snap.out)))
                    }
                });
                if !keep && let Some(dropped) = state.entries.remove(&chain[j]) {
                    state.recycle(dropped.snaps);
                }
            }
        }
    }

    /// How many walks the memo has made: a test reads it to see a read
    /// answered from the memo (`testing::deep_overflow_walks`).
    pub(crate) fn walks(&self) -> u64 {
        self.state.borrow().walks
    }
}

/// Whether box `id`, whose [`WalkSnap`] differs from the one `entry`'s walk
/// recorded, changed only where it adds nothing to the kept reach — and if
/// so, the record follows it. Only its box (where it is, inline offset
/// included, and how big) and its lines may have changed — nothing that
/// decides whether the walk counts it or goes into it, and not its content
/// origin, which moves what is placed from it with no change to their
/// `layout` ([`WalkSnap`]) — and
/// either it counts nothing (out of the container's overflow, displayed
/// `none`, or transformed, which each read measures anew), or nothing of it
/// was counted and all of it — its box, its lines and what the walk found
/// below it, moved with it — is now inside its top box ([`Keep::top`](super::deep_walk::Keep::top)); for
/// the top box itself, all that is below it is inside its new box.
///
/// A box below it that moved too is checked on its own, where it is now; one
/// that did not moved with it, and is in what the walk found below it. So no
/// counted contribution changed, and the entry stands. What is found inside
/// is added to the record of each box above it up to the top box, so a later
/// move of one of them is checked with it (a record only grows: a box that
/// shrank is checked as it was, which can only drop the entry).
pub(super) fn moved_inside(
    tree: &NodeTree,
    container_id: RawNodeId,
    entry: &mut DeepEntry,
    id: RawNodeId,
) -> bool {
    let Some(node) = tree.get_live(id) else {
        return false;
    };
    let Some(rec) = entry.snaps.get(&id).copied() else {
        return false;
    };
    // The box-tree path from the box up to the container, which positions
    // are summed along.
    let mut path: Vec<RawNodeId> = vec![id];
    loop {
        let Some(parent) = crate::RinchDocument::box_tree_parent(&tree.nodes, path[path.len() - 1])
        else {
            return false;
        };
        if parent == container_id {
            break;
        }
        if path.len() > 4096 {
            return false;
        }
        path.push(parent);
    }
    let top_level = path.len() == 1;
    let out = if top_level {
        !(node.taffy_style_owned_by_contents_splice()
            || crate::out_of_flow::contributes_to_scrollable_overflow(tree, container_id, node))
    } else {
        walk_out(tree, container_id, node)
    };
    let now = WalkSnap::of(tree, node, out);
    // Only where its box is, how big, and its lines may differ — not its
    // content origin: the record of what is below it holds the lines and
    // atomic inlines placed from that origin where they were, and they do
    // not move in their own `layout` when it does.
    if (WalkSnap {
        layout: rec.snap.layout,
        inline_offset: rec.snap.inline_offset,
        lines: rec.snap.lines,
        ..now
    }) != rec.snap
        || now.contents
    {
        return false;
    }
    let follow = |entry: &mut DeepEntry| {
        if let Some(r) = entry.snaps.get_mut(&id) {
            r.snap = now;
        }
    };
    if now.out || now.transformed || now.display == crate::computed_style::DisplayValue::None {
        follow(entry);
        return true;
    }
    if rec.counted {
        return false;
    }
    // Each box's children's origin now, from the container down
    // ([`box_origin`], summed in the walk's order).
    let mut at = vec![(0.0_f64, 0.0_f64); path.len()];
    let mut origin = (0.0_f64, 0.0_f64);
    for k in (0..path.len()).rev() {
        let Some(n) = tree.get_live(path[k]) else {
            return false;
        };
        origin = box_origin(tree, n, origin);
        at[k] = origin;
    }
    // The top box: the highest box on the path that is not a `display:
    // contents` wrapper.
    let Some(t) = (0..path.len()).rev().find(|&k| {
        tree.get_live(path[k])
            .is_some_and(|n| !n.taffy_style_owned_by_contents_splice())
    }) else {
        return false;
    };
    let Some(top) = tree.get_live(path[t]) else {
        return false;
    };
    let Some(rect) = containing_rect(
        top,
        at[t].0,
        at[t].1,
        top.layout.width as f64,
        top.layout.height as f64,
    ) else {
        return false;
    };
    // Everything of the box now: its box (not the top box's own, which the
    // walk does not count), its lines, and what was found below it, moved
    // as its children's origin moved.
    let (x, y) = at[0];
    let mut span = rec.below.moved((x - rec.at.0, y - rec.at.1));
    if WalkSnap::stops(node) {
        span = Span::EMPTY;
    } else if let Some((x0, x1, y1)) = lines_span(node, x, y) {
        span.grow(x0, x1, y1);
    }
    if t != 0 {
        let (w, h) = (node.layout.width as f64, node.layout.height as f64);
        if w > 0.0 || h > 0.0 {
            span.grow(x, x + w, y + h);
        }
    }
    if !span.inside(&rect) {
        return false;
    }
    // Every box above it, up to the top box, holds it now.
    for k in 1..=t {
        let Some(r) = entry.snaps.get_mut(&path[k]) else {
            return false;
        };
        r.below
            .join(span.moved((r.at.0 - at[k].0, r.at.1 - at[k].1)));
    }
    follow(entry);
    true
}

/// Whether a box's snapshot now matches the one a walk recorded: `out` is
/// the walk's own judgement (it depends on the box's ancestors, whose own
/// snapshots carry what decides it), so it is taken from the record.
pub(super) fn same_snap(recorded: &WalkSnap, now: &WalkSnap) -> bool {
    WalkSnap {
        out: recorded.out,
        ..*now
    } == *recorded
}
