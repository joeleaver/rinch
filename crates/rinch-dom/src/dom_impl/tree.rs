//! [`DomTree`] for [`RinchDocument`]: node creation (built in `create.rs`),
//! tree mutation, retiring nodes, dirty tracking, selectors and traversal,
//! HTML parsing.

use rinch_core::dom::{DomTree, NodeId};

use crate::ifc_scope::IfcSeed;
use crate::node::{DirtyFlags, NodeKind};

use super::RinchDocument;

impl DomTree for RinchDocument {
    fn doc_key(&self) -> u64 {
        self.doc_key
    }

    fn create_element(&mut self, tag: &str) -> NodeId {
        self.build_element(tag)
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        self.build_text(text)
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        self.build_comment(text)
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(parent.0) || !self.tree.nodes.contains(child.0) {
            return;
        }
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        let p = parent.0;
        let c = child.0;
        // Invalidate the IFC the child was a member of — not its own (#914).
        self.invalidate_ifc_left_by(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any (both DOM and Taffy). The Taffy side
        // must remove the node's *contribution*, not just its own id — a
        // spliced `display: contents` node's slots are its children's (#517).
        let restyle = !self.keeps_style_across_move(c, p);
        self.record_pixels_left_by_move(c, p);
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.seed_ifc(old_parent, IfcSeed::Children);
            let old_index = self.remove_from_children(old_parent, c);
            self.note_child_list_changed(old_parent, old_index);
            // Remove from old taffy parent
            if let Some(old_taffy_parent) = self.tree.nodes[old_parent].taffy_id {
                self.taffy_detach_contribution(old_taffy_parent, c);
            }
            // A move *out of* the document is a detach (#702).
            self.detach_subtree_styles_if_moved_out(c, old_parent, p);
            // The select the option left is sized from its options too (review
            // of #1737).
            self.note_select_content_changed(old_parent);
        }
        self.tree.nodes[c].parent = Some(p);
        // At the end of the element's own children: before an `::after` box
        // (#1629).
        let len = self.tree.nodes[p].children.len();
        let pos = self.clamp_between_generated(p, c, len);
        self.tree.nodes[p].children.insert(pos, c);
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            if pos == len {
                self.taffy_add_child_checked(parent_taffy, child_taffy);
            } else {
                let taffy_idx = self.compute_taffy_child_index(p, pos);
                self.attach_taffy_child_at(parent_taffy, taffy_idx, child_taffy, p, c);
            }
            // Mark child as dirty so it gets measured during next layout pass
            let _ = self.tree.taffy.mark_dirty(child_taffy);
        }
        // Invalidate parent's IFC (structure changed)
        self.invalidate_parent_ifc(p);
        self.tree.layout_dirty = true; // Structural change needs full layout
        // Where the tree changed, for the scoped structural pass
        // (`crate::ifc_scope`).
        self.tree.seed_ifc(p, IfcSeed::Children);
        self.tree.seed_ifc(c, IfcSeed::Subtree);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

        // Mark inserted subtree as paint-dirty so the dirty region includes
        // the new nodes' layout positions after layout runs.
        self.mark_subtree_paint_dirty_ids(c);

        // Siblings a structural selector ties to the new child (`+`, `~`,
        // `:nth-child`, `:last-child`, the parent's `:empty`).
        self.note_child_inserted(p, c, true);

        // Recompute styles for the inserted subtree to pick up ancestor-based selectors.
        // Suppressed during bulk DOM operations (render_block_at) to batch into one pass.
        if !restyle {
            // A move within one parent (#914): see `keeps_style_across_move`.
        } else if !self.tree.suppress_inline_restyle {
            self.recompute_node_styles_recursive(c);
        } else {
            // Deferred, not skipped: the subtree is left unstyled and recorded
            // as a style root, so the next `resolve_styles` cascades it.
            self.invalidate_subtree_styles(c);
        }

        // An inserted `<option>` carrying selectedness takes the selection with
        // it, as it does in a browser (#692).
        crate::select::options_inserted(&mut self.tree, c);
        self.note_select_content_changed(p);

        // If a text node is appended to a <style> element, load its content as CSS
        self.maybe_load_style_css(p);
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(parent.0) || !self.tree.nodes.contains(child.0) {
            return;
        }
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        let p = parent.0;
        let c = child.0;
        // Its pixels are recorded where it was painted and its painted state
        // forgotten, as `remove_node` does (#966).
        self.mark_subtree_paint_dirty(c);
        // Clear IFC state on removed child
        self.clear_ifc_root_recursive(c);
        // The subtree is leaving the document, so it has no before-change style
        // to animate from if it ever comes back (#699).
        self.detach_subtree_styles(c);
        let old_index = self.remove_from_children(p, c);
        self.tree.nodes[c].parent = None;
        self.note_child_list_changed(p, old_index);
        // Sync taffy: remove the child's contribution — for a spliced
        // `display: contents` child that is its children's slots, not its
        // own already-detached id (#517).
        if let Some(parent_taffy) = self.tree.nodes[p].taffy_id {
            self.taffy_detach_contribution(parent_taffy, c);
        }
        // Invalidate parent's IFC
        self.invalidate_parent_ifc(p);
        self.tree.layout_dirty = true; // Structural change needs full layout
        // Where the tree changed, for the scoped structural pass
        // (`crate::ifc_scope`).
        self.tree.seed_ifc(p, IfcSeed::Children);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
        self.note_select_content_changed(p);
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(parent.0)
            || !self.tree.nodes.contains(child.0)
            || !self.tree.nodes.contains(reference.0)
        {
            return;
        }
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        let p = parent.0;
        let c = child.0;
        let r = reference.0;
        // Invalidate the IFC the child was a member of — not its own (#914).
        self.invalidate_ifc_left_by(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any — the node's contribution, not just
        // its own id (#517, see `taffy_detach_contribution`)
        let restyle = !self.keeps_style_across_move(c, p);
        self.record_pixels_left_by_move(c, p);
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.seed_ifc(old_parent, IfcSeed::Children);
            let old_index = self.remove_from_children(old_parent, c);
            self.note_child_list_changed(old_parent, old_index);
            if let Some(old_taffy_parent) = self.tree.nodes[old_parent].taffy_id {
                self.taffy_detach_contribution(old_taffy_parent, c);
            }
            // A move *out of* the document is a detach (#702).
            self.detach_subtree_styles_if_moved_out(c, old_parent, p);
            // The select the option left is sized from its options too (review
            // of #1737).
            self.note_select_content_changed(old_parent);
        }
        self.tree.nodes[c].parent = Some(p);
        // A reference that is no child of `p` is an append. Either way the
        // child stays between the generated boxes (#1629): a reference that
        // is the `::before` box itself puts it after that box.
        let len = self.tree.nodes[p].children.len();
        let asked = self.tree.nodes[p]
            .children
            .iter()
            .position(|&x| x == r)
            .unwrap_or(len);
        let pos = self.clamp_between_generated(p, c, asked);
        let insert_pos = if pos < len {
            self.tree.nodes[p].children.insert(pos, c);
            Some(pos)
        } else {
            self.tree.nodes[p].children.push(c);
            None
        };
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            if let Some(pos) = insert_pos {
                // Where this DOM position lands in the parent's *attached*
                // Taffy children (#477) — not a count of DOM siblings.
                let taffy_idx = self.compute_taffy_child_index(p, pos);
                self.attach_taffy_child_at(parent_taffy, taffy_idx, child_taffy, p, c);
            } else {
                self.taffy_add_child_checked(parent_taffy, child_taffy);
            }
        }
        self.invalidate_parent_ifc(p);
        self.tree.layout_dirty = true; // Structural change needs full layout
        // Where the tree changed, for the scoped structural pass
        // (`crate::ifc_scope`).
        self.tree.seed_ifc(p, IfcSeed::Children);
        self.tree.seed_ifc(c, IfcSeed::Subtree);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

        // Mark inserted subtree as paint-dirty so the dirty region includes
        // the new nodes' layout positions after layout runs.
        self.mark_subtree_paint_dirty_ids(c);

        self.note_child_inserted(p, c, true);

        // Recompute styles for the inserted subtree to pick up ancestor-based
        // selectors — unless it only moved within its parent (#914).
        if restyle {
            self.recompute_node_styles_recursive(c);
        }

        // The insertion rule (#692), as in `append_child`. This is the path a
        // keyed `for` row arrives through: `rinch_core::for_loop` places each
        // row with `NodeHandle::insert_after`, which routes here whenever the
        // anchor has a next sibling and to `append_child` only when it does
        // not. So an `<option selected>` rendered in a list reaches its select
        // through this method for every position but the last.
        crate::select::options_inserted(&mut self.tree, c);
        self.note_select_content_changed(p);
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(old.0) || !self.tree.nodes.contains(new.0) {
            return;
        }
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        // Replacing a node with itself is a no-op the browser accepts (the DOM
        // spec re-inserts `node` before its own next sibling), and the same must
        // hold here. Without this guard the "detach `new` from its old parent"
        // step below unlinks the node from the very parent the replace is about
        // to look it up in, `position(|x| x == old.0)` then misses, and the node
        // is left out of its parent's children *and* out of taffy while still
        // claiming `parent = Some(..)` — a tree the rest of this file cannot
        // reason about (issue #184).
        if old == new {
            return;
        }
        // A non-empty child swapped for a non-empty one leaves the parent's
        // `:empty` where it was (the editor's view diff does this for every
        // mark it applies).
        let keeps_nonempty = crate::stylo_impl::child_defeats_empty(&self.tree.nodes[old.0])
            && crate::stylo_impl::child_defeats_empty(&self.tree.nodes[new.0]);
        self.invalidate_ifc_for_node(old.0);
        self.clear_ifc_root_recursive(old.0);
        self.invalidate_ifc_for_node(new.0);
        self.clear_ifc_root_recursive(new.0);
        if let Some(parent_id) = self.tree.nodes[old.0].parent {
            // `old` leaves the document: its pixels are recorded where it was
            // painted, under the chain it was painted under, and its painted
            // state is forgotten, as `remove_node` does (#966). Otherwise only
            // the parent's box was repainted, and an `old` appended elsewhere
            // later had its old rect placed along its new parents.
            self.mark_subtree_paint_dirty(old.0);
            let new_moved = self.record_pixels_left_by_move(new.0, parent_id);
            // Remove new from its old parent if any — the node's
            // contribution, not just its own id (#517)
            if let Some(old_parent) = self.tree.nodes[new.0].parent {
                self.tree.seed_ifc(old_parent, IfcSeed::Children);
                let old_index = self.remove_from_children(old_parent, new.0);
                self.note_child_list_changed(old_parent, old_index);
                if let Some(old_taffy_parent) = self.tree.nodes[old_parent].taffy_id {
                    self.taffy_detach_contribution(old_taffy_parent, new.0);
                }
                // A move *out of* the document is a detach (#702).
                self.detach_subtree_styles_if_moved_out(new.0, old_parent, parent_id);
                // The select the option left is sized from its options too (review
                // of #1737).
                self.note_select_content_changed(old_parent);
            }
            // Replace old with new in parent's children
            if let Some(pos) = self.tree.nodes[parent_id]
                .children
                .iter()
                .position(|&x| x == old.0)
            {
                self.tree.nodes[parent_id].children[pos] = new.0;
                // Sync taffy: remove old's contribution (for a spliced
                // `display: contents` node that is its children's slots,
                // not its own already-detached id — #517), insert new at
                // the same position
                if let Some(parent_taffy) = self.tree.nodes[parent_id].taffy_id {
                    self.taffy_detach_contribution(parent_taffy, old.0);
                    if let Some(new_taffy) = self.tree.nodes[new.0].taffy_id {
                        let taffy_idx = self.compute_taffy_child_index(parent_id, pos);
                        self.attach_taffy_child_at(
                            parent_taffy,
                            taffy_idx,
                            new_taffy,
                            parent_id,
                            new.0,
                        );
                    }
                }
            }
            self.tree.nodes[new.0].parent = Some(parent_id);
            self.tree.nodes[old.0].parent = None;
            // `old` has left the document (#699). `new` has left it only if
            // the parent it was spliced into is itself detached, which
            // `detach_subtree_styles_if_moved_out` decided above (#702); an
            // ordinary splice is a move and a move resets nothing.
            self.detach_subtree_styles(old.0);
            self.invalidate_parent_ifc(parent_id);
            self.tree.layout_dirty = true; // Structural change needs full layout
            self.tree.seed_ifc(parent_id, IfcSeed::Children);
            self.tree.seed_ifc(new.0, IfcSeed::Subtree);
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

            self.note_child_inserted(parent_id, new.0, !keeps_nonempty);
            if new_moved {
                self.mark_subtree_paint_dirty_ids(new.0);
            }

            // Recompute styles for the new subtree to pick up ancestor-based selectors
            self.recompute_node_styles_recursive(new.0);

            // A replacement is an insertion for the node arriving (#692).
            // Measured in Chrome 150: `replaceChild` of a `selected` option
            // over option 0, while option 1 is selected, gives
            // `selectedIndex == 0`. The node *leaving* needs nothing — it is
            // detached, so it drops out of the select's option list and
            // `resolve_select_model` falls back to the first enabled option,
            // which is what a browser does too.
            crate::select::options_inserted(&mut self.tree, new.0);
            self.note_select_content_changed(parent_id);
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        // Mark the removed subtree as paint-dirty so the dirty region
        // includes the old layout positions (e.g., borders, backgrounds).
        // Without this, the old pixels aren't cleared on repaint.
        self.mark_subtree_paint_dirty(node.0);

        self.clear_ifc_root_recursive(node.0);
        if let Some(parent_id) = self.tree.nodes[node.0].parent {
            let old_index = self.remove_from_children(parent_id, node.0);
            self.note_child_list_changed(parent_id, old_index);
            // Sync taffy: remove this node's *contribution* to the parent's
            // Taffy child list, which for a `display: contents` wrapper is its
            // spliced-in descendants rather than its own (detached) id.
            //
            // `sync_display_contents` polyfills `display: contents` — which
            // Taffy has no native concept of — by setting the wrapper's own
            // Taffy style to `display: none` and splicing its children's Taffy
            // ids directly into the parent's list. So the wrapper's own
            // `taffy_id` is not among `parent_taffy`'s children, and removing
            // *that* id is a silent no-op (`taffy_remove_child_safe` swallows
            // a "not actually a child" removal by design). The node that
            // really did occupy the slot — the child spliced in on the
            // wrapper's behalf — was never asked to leave, and stayed a
            // permanent invisible sibling still claiming its share of
            // `flex-grow`.
            //
            // Reported against a route table where the one arm whose body is a
            // reactive `if` gets an rsx `display: contents` marker div: the
            // first navigation away from it stranded that screen's real root
            // as a phantom `flex: 1` sibling of the app root, so every screen
            // reached afterwards split the height with a ghost. Two `flex: 1`
            // divs under a `flex: 1` column, one behind a contents wrapper, is
            // the whole reproduction.
            //
            // This goes through the shared `taffy_detach_contribution` (#517)
            // rather than an either/or of its own, which matters twice over.
            // It removes the flattened set **and** the node's own id instead
            // of choosing between them, so a node that computes `Contents`
            // while its own Taffy node is still attached — the window between
            // a display toggle and the sync that heals it — is covered; and it
            // consults `Node::contents_spliced` as well as computed display
            // (#520), which is the only record of a wrapper still spliced
            // after its computed display has already changed back.
            if let Some(parent_taffy) = self.tree.nodes[parent_id].taffy_id {
                self.taffy_detach_contribution(parent_taffy, node.0);
            }
            self.invalidate_parent_ifc(parent_id);
            self.tree.layout_dirty = true; // Structural change needs full layout
            self.tree.seed_ifc(parent_id, IfcSeed::Children);
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
            self.note_select_content_changed(parent_id);
        }
        let old_parent = self.tree.nodes[node.0].parent.take();
        // Asked again now that the node is out: `note_child_list_changed` ran
        // while it was still linked to its parent (#884).
        if let Some(parent_id) = old_parent {
            self.note_style_elements_moved(parent_id);
        }
        // The whole removed subtree loses its before-change style, so a
        // re-insertion under a changed ancestor is a first style rather than a
        // change to animate (#699). Unconditional: `remove_node` on an already
        // detached node is a no-op for a subtree that is already reset.
        self.detach_subtree_styles(node.0);
        // Not freed: a removed node may be inserted again (#719). Freeing is
        // `discard_node`'s.
    }

    /// Detach `node` as [`Self::remove_node`] does, then **free** it and its
    /// subtree (#723): the slab slots, the Taffy nodes, and every entry the
    /// document keeps under their ids. Each slot's generation moves on, so a
    /// handle kept from before names nothing: a write through it is a no-op,
    /// as on `rinch-web` and `MockDomDocument`, and never reaches the node a
    /// later `create_*` puts in the same slot.
    fn discard_node(&mut self, node: NodeId) {
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        // Detach first: it records the subtree's painted pixels, takes its
        // Taffy contribution out of the parent's list and resets its styles,
        // all of which read the nodes freed below.
        self.remove_node(node);
        self.free_subtree(node.0);
    }

    fn is_retired(&self, node: NodeId) -> bool {
        !self.tree.nodes.contains(node.0)
    }

    fn mark_dirty(&mut self, node: NodeId) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        self.push_dirty(node.0);
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        // What a skipped sibling mark would have pushed again (#2627).
        crate::node::invalidate_sibling_marks();
        std::mem::take(&mut self.tree.dirty_nodes)
            .into_iter()
            .map(NodeId)
            .collect()
    }

    fn root(&self) -> NodeId {
        NodeId(self.tree.root_id)
    }

    fn body(&self) -> NodeId {
        NodeId(self.tree.body_id)
    }

    fn query_selector(&self, selector: &str) -> Option<NodeId> {
        crate::query::query_first(&self.tree, self.tree.root_id, selector).map(NodeId)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        RinchDocument::query_selector_all(self, selector)
    }

    fn query_selector_in(&self, scope: NodeId, selector: &str) -> Option<NodeId> {
        crate::query::query_first(&self.tree, scope.0, selector).map(NodeId)
    }

    fn query_selector_all_in(&self, scope: NodeId, selector: &str) -> Vec<NodeId> {
        crate::query::query_all(&self.tree, scope.0, selector)
            .into_iter()
            .map(NodeId)
            .collect()
    }

    fn is_text_node(&self, node: NodeId) -> bool {
        self.tree
            .nodes
            .get(node.0)
            .is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.tree
            .nodes
            .get(node.0)
            .map(|n| n.children.iter().map(|&c| NodeId(c)).collect())
            .unwrap_or_default()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(parent.0) || !self.tree.nodes.contains(child.0) {
            return;
        }
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        let p = parent.0;
        let c = child.0;
        // Invalidate the IFC the child was a member of — not its own (#914).
        self.invalidate_ifc_left_by(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any — the node's contribution, not just
        // its own id (#517, see `taffy_detach_contribution`)
        let restyle = !self.keeps_style_across_move(c, p);
        let moved_between_parents = self.record_pixels_left_by_move(c, p);
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.seed_ifc(old_parent, IfcSeed::Children);
            let old_index = self.remove_from_children(old_parent, c);
            self.note_child_list_changed(old_parent, old_index);
            if let Some(old_taffy_parent) = self.tree.nodes[old_parent].taffy_id {
                self.taffy_detach_contribution(old_taffy_parent, c);
            }
            // A move *out of* the document is a detach (#702).
            self.detach_subtree_styles_if_moved_out(c, old_parent, p);
            // The select the option left is sized from its options too (review
            // of #1737).
            self.note_select_content_changed(old_parent);
        }
        self.tree.nodes[c].parent = Some(p);
        let len = self.tree.nodes[p].children.len();
        // Counted among the element's children as `get_children` lists them,
        // generated boxes included, and kept between those boxes (#1629).
        let actual_index = self.clamp_between_generated(p, c, index.min(len));
        self.tree.nodes[p].children.insert(actual_index, c);
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            // Including the append leg (`actual_index == len`), which has no
            // separate `add_child` path — an out-of-range index used to lose
            // the child here with nothing to catch it (#477).
            let taffy_idx = self.compute_taffy_child_index(p, actual_index);
            self.attach_taffy_child_at(parent_taffy, taffy_idx, child_taffy, p, c);
        }
        self.invalidate_parent_ifc(p);
        self.tree.layout_dirty = true; // Structural change needs full layout
        // Where the tree changed, for the scoped structural pass
        // (`crate::ifc_scope`).
        self.tree.seed_ifc(p, IfcSeed::Children);
        self.tree.seed_ifc(c, IfcSeed::Subtree);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
        // A subtree moved here from another parent was forgotten as painted
        // (`record_pixels_left_by_move`) and has to be named where it lands,
        // as `append_child` and `insert_before` name every insertion.
        if moved_between_parents {
            self.mark_subtree_paint_dirty_ids(c);
        }

        self.note_child_inserted(p, c, true);

        // Recompute styles for the inserted subtree to pick up ancestor-based
        // selectors — unless it only moved within its parent (#914).
        if restyle {
            self.recompute_node_styles_recursive(c);
        }

        // The same insertion rule as `append_child` (#692) — and the one that
        // shows it is "last inserted" rather than "last in tree order": Chrome
        // 150 hands the selection to a `selected` option inserted *before* an
        // already-selected one.
        crate::select::options_inserted(&mut self.tree, c);
        self.note_select_content_changed(p);
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        self.tree.nodes.get(node.0)?.parent.map(NodeId)
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        let parent_id = self.tree.nodes.get(node.0)?.parent?;
        let siblings = &self.tree.nodes[parent_id].children;
        let pos = siblings.iter().position(|&c| c == node.0)?;
        siblings.get(pos + 1).map(|&c| NodeId(c))
    }

    fn parse_html(&mut self, _html: &str) -> Option<NodeId> {
        // Phase 9: HTML parser integration
        // For now, return None
        None
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        use crate::html_parser::parse_html_string;

        // Clear the element's own children (including taffy sync). Its
        // generated `::before` / `::after` boxes stay (#1630); the parsed
        // children are appended between them below (`append_child` keeps
        // them there, #1629).
        let (lo, hi) = self.own_children_range(node.0);
        let old_children: Vec<_> = self.tree.nodes[node.0].children[lo..hi].to_vec();
        for child in old_children {
            // The child's pixels are recorded where it was painted before it
            // is freed, as `remove_node` does (#966): nothing can reach its
            // rects once `remove_subtree` has dropped it.
            self.mark_subtree_paint_dirty(child);
            self.clear_ifc_root_recursive(child);
            // Remove each child's contribution from taffy — for a spliced
            // `display: contents` child that is its children's slots, not
            // its own id (#517). Must run before `remove_subtree`, which
            // drops the slab nodes the contribution walk reads.
            if let Some(parent_taffy) = self.tree.nodes[node.0].taffy_id {
                self.taffy_detach_contribution(parent_taffy, child);
            }
            self.tree.nodes[child].parent = None;
            self.free_subtree(child);
        }
        self.tree.nodes[node.0].children.drain(lo..hi);
        // `:empty` on the node, and whatever a structural selector ties to
        // its children, can have moved (each re-append below notes its own
        // insertion; a clear-to-empty has none).
        self.note_child_list_changed(node.0, 0);
        // Under a select the cleared children were (part of) a label; a
        // clear-to-empty re-appends nothing that would say so (review of
        // #1737).
        self.note_select_content_changed(node.0);
        // Clearing a subtree is a structural change: without these flags a
        // clear-to-empty `set_inner_html` (nothing re-appended below) leaves
        // `resolve_layout`'s dirty gate closed and the old geometry — the
        // removed children's sizes included — stays on screen (#517).
        self.tree.layout_dirty = true;
        // The children are freed, not detached: whatever IFC state they held
        // (anonymous boxes, splits, measure leaves) is swept as orphaned by
        // the scoped pass. The parsed children seed themselves as they are
        // appended below.
        self.tree.seed_ifc(node.0, IfcSeed::Children);

        // Parse HTML and create nodes
        if let Some(parsed_nodes) = parse_html_string(html) {
            for parsed in parsed_nodes {
                let child_id = self.create_node_from_parsed(&parsed);
                self.append_child(node, child_id);
            }
        }

        self.invalidate_parent_ifc(node.0);
        self.push_dirty_flags(node.0, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
    }

    fn tag_name(&self, node: NodeId) -> Option<String> {
        let n = self.tree.nodes.get(node.0)?;
        match &n.kind {
            NodeKind::Element(data) => Some(data.tag.clone()),
            _ => None,
        }
    }

    fn node_type(&self, node: NodeId) -> Option<u16> {
        let n = self.tree.nodes.get(node.0)?;
        Some(match &n.kind {
            NodeKind::Element(_) => 1,
            NodeKind::Text(_) => 3,
            NodeKind::Comment(_) => 8,
            NodeKind::Document => 9,
        })
    }
}
