//! The check's marking of what was pushed ([`Marker`]): which records of
//! the kept walks ([`DeepOverflow`](super::DeepOverflow)) a change reached,
//! and which of each box's children it reached (#2054).

use super::deep_memo::DeepOverflowState;
use super::deep_record::{AGAIN, BELOW, SELF, STRUCT};
use crate::NodeTree;
use crate::node::RawNodeId;

/// Marks the records a check's pushed boxes reach: each box pushed
/// ([`SELF`]), each recorded box above it up to its container ([`BELOW`]),
/// and in each of those the child the marking came up from, listed in its
/// marked children (`Rec::first`, `Rec::next`) — or [`STRUCT`] where its
/// children changed as a list, which sums them all instead.
pub(super) struct Marker<'a> {
    tree: &'a NodeTree,
    pub(super) state: &'a mut DeepOverflowState,
    /// The box the last marking ([`Self::climb`]) started from, the
    /// container it reached, and whether that box lists no marked children
    /// (it is the container, or is marked [`STRUCT`] in this check). The
    /// rows of a list share their parent, and the second marks nothing the
    /// first did not — nor lists itself, where the parent sums all its
    /// children (a row taken out shifts the rest).
    last: Option<Last>,
}

/// What [`Marker::climb`] remembers of the last marking.
#[derive(Clone, Copy)]
struct Last {
    start: RawNodeId,
    owner: Option<RawNodeId>,
    /// The start lists no marked children.
    all: bool,
}

impl<'a> Marker<'a> {
    pub(super) fn new(tree: &'a NodeTree, state: &'a mut DeepOverflowState) -> Self {
        Self {
            tree,
            state,
            last: None,
        }
    }

    /// Note box `id`, pushed paint-dirty or (`lost_child`) a parent a subtree
    /// left.
    pub(super) fn note(&mut self, id: RawNodeId, lost_child: bool) {
        let flag = if lost_child { BELOW | STRUCT } else { SELF };
        if let Some(entry) = self.state.entries.get_mut(&id) {
            entry.touch |= flag;
        }
        let tree = self.tree;
        let Some(node) = tree.get(id) else {
            // Freed (`DeepOverflow::forget_freed` dropped its record); its
            // parent was noted as one a subtree left.
            self.state.forget(id);
            self.last = None;
            return;
        };
        if node.is_text() {
            // Its lines are its IFC root's, which is pushed when they move.
            return;
        }
        let parent = node.box_tree_parent();
        let epoch = self.state.epoch;
        let Some(rec) = self.state.recs.get_mut(&id) else {
            if node.is_split_inline() {
                // Not a box, so not recorded, but its style may decide an
                // absolute box's containing block below it.
                if let Some(owner) = self.climb(parent, None, false)
                    && let Some(entry) = self.state.entries.get_mut(&owner)
                    && entry.touch & BELOW != 0
                {
                    entry.touch |= AGAIN;
                }
            } else {
                // A box added (or one below a box the walk did not enter,
                // which the marking stops at): its parent gained a child.
                self.climb(parent, None, true);
            }
            return;
        };
        let recorded = rec.parent;
        if parent != Some(recorded) {
            // Moved, or taken out, since it was met: its records go, its old
            // parent lost it, and where it is now records it again.
            self.state.purge(tree, id);
            self.last = None;
            self.climb(Some(recorded), None, true);
            self.climb(parent, None, true);
            return;
        }
        let newly = rec.mark(epoch, flag);
        self.climb(parent, newly.then_some(id), false);
    }

    /// Mark the recorded boxes from `start` up to the container whose walk
    /// met them as having something below them changed, listing `link` (a
    /// child of `start` marked for the first time in this check) in
    /// `start`'s marked children, and marking `start` to sum all its
    /// children again where `structural` (one was added or taken out). The
    /// container, if the marking reaches one — or the one an earlier marking
    /// from a box it reached went on to.
    ///
    /// Stops at a box the walk did not go into (nothing below it is read),
    /// and at a box already marked in this check, whose marking went on.
    pub(super) fn climb(
        &mut self,
        start: Option<RawNodeId>,
        link: Option<RawNodeId>,
        structural: bool,
    ) -> Option<RawNodeId> {
        let plain = link.is_none() && !structural;
        if let (Some(s), Some(last)) = (start, self.last)
            && s == last.start
            && (plain || last.all)
        {
            return last.owner;
        }
        let (owner, all) = self.climb_from(start, link, structural);
        self.last = start.map(|start| Last { start, owner, all });
        owner
    }

    /// [`Self::climb`]'s marking: the container, and whether `start` lists
    /// no marked children (see [`Marker::last`]).
    fn climb_from(
        &mut self,
        start: Option<RawNodeId>,
        mut link: Option<RawNodeId>,
        mut structural: bool,
    ) -> (Option<RawNodeId>, bool) {
        let epoch = self.state.epoch;
        let mut current = start;
        // Whether the box at `start` lists no marked children; false where
        // there is no record at `start`, whose climb is never folded.
        let mut start_all = None;
        let mut answer = |all: bool| *start_all.get_or_insert(all);
        while let Some(a) = current {
            if let Some(entry) = self.state.entries.get_mut(&a) {
                // The container visits all its children.
                entry.touch |= BELOW;
                return (Some(a), answer(true));
            }
            let Some(rec) = self.state.recs.get_mut(&a) else {
                // A box the walk did not record (one added): the recorded
                // box above it gained a child, which no list names.
                answer(false);
                structural = true;
                link = None;
                current = self.tree.get(a).and_then(|n| n.box_tree_parent());
                continue;
            };
            if !rec.snap.entered() {
                return (None, answer(false));
            }
            let newly = rec.mark(epoch, if structural { BELOW | STRUCT } else { BELOW });
            let (parent, owner, all) = (rec.parent, rec.owner, rec.touch & STRUCT != 0);
            answer(all);
            if let Some(child) = link
                && !all
            {
                let head = std::mem::replace(&mut rec.first, child);
                if let Some(child) = self.state.recs.get_mut(&child) {
                    child.next = head;
                }
            }
            if !newly {
                return (Some(owner), answer(all));
            }
            structural = false;
            link = Some(a);
            current = Some(parent);
        }
        (None, answer(false))
    }
}

#[cfg(test)]
mod tests {
    use super::Marker;
    use crate::RinchDocument;
    use rinch_core::dom::DomDocument;

    /// A marking from the box the last one started from is folded into it,
    /// and still answers the container that one reached: a split inline's
    /// marking needs it to flag the container to be walked again (review of
    /// ec70c112c: it answered none).
    #[test]
    fn a_marking_folded_into_the_last_one_answers_its_container() {
        let mut d = RinchDocument::new();
        d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
        let body = d.body();
        d.set_inner_html(
            body,
            r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div id="p" style="width:150px"><div style="width:300px;height:10px"></div></div></div>"#,
        );
        d.resolve_layout(800.0, 600.0);
        let (s, p) = (
            d.query_selector("#s").unwrap(),
            d.query_selector("#p").unwrap(),
        );
        assert_eq!(d.scroll_width(s), 300.0, "control: the walk is kept");
        let tree = &d.tree;
        let mut state = tree.deep_overflow.state.borrow_mut();
        state.epoch += 1;
        let mut marker = Marker::new(tree, &mut state);
        assert_eq!(marker.climb(Some(p.0), None, false), Some(s.0));
        assert_eq!(
            marker.climb(Some(p.0), None, false),
            Some(s.0),
            "folded into the first, it answers the same container"
        );
    }
}
