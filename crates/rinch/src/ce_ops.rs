//! ContentEditable operations backed by a RinchDocument.
//!
//! Implements the [`ContentEditableApi`] trait from `rinch-core`, providing
//! a unified API for contentEditable DOM mutations. Created when a CE element
//! gains focus and registered via [`set_active_ce_api`] so both app.rs and
//! the editor bridge can access it.
//!
//! Currently provides real implementations for selection management and
//! event dispatching. DOM mutation methods (insert_text, delete_backward, etc.)
//! are handled by app.rs directly for now; these stubs dispatch CeEvents so
//! observers (the editor bridge) stay informed.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::ce::{
    CeEvent, CeEventDispatcher, CeSelection, ContentEditableApi, DomCursor, dispatch_ce_event,
};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

use crate::app::RinchApp;

// ============================================================================
// DOM Helpers for Inline Formatting
// ============================================================================

/// Walk ancestors from `node_id` to find a formatting element with the given `tag`.
/// Stops at `stop_at_id` (the CE root).
fn find_formatting_ancestor(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    tag: &str,
    stop_at_id: usize,
) -> Option<usize> {
    let mut current = node_id;
    loop {
        let parent_id = tree.get(current)?.parent?;
        if parent_id == stop_at_id {
            return None;
        }
        if tree.get(parent_id)?.tag() == Some(tag) {
            return Some(parent_id);
        }
        current = parent_id;
    }
}

/// Collect all text node IDs in document order under `root`.
fn collect_text_nodes(tree: &rinch_dom::NodeTree, root: usize, out: &mut Vec<usize>) {
    let Some(node) = tree.get(root) else { return };
    if node.text_content().is_some() {
        out.push(root);
        return;
    }
    for &child_id in &node.children {
        collect_text_nodes(tree, child_id, out);
    }
}

/// Get text node IDs that fall within a cursor range (inclusive).
fn text_nodes_in_range(
    tree: &rinch_dom::NodeTree,
    ce_root: usize,
    start: DomCursor,
    end: DomCursor,
) -> Vec<usize> {
    let mut all = Vec::new();
    collect_text_nodes(tree, ce_root, &mut all);
    let s = all.iter().position(|&id| id == start.node_id).unwrap_or(0);
    let e = all
        .iter()
        .position(|&id| id == end.node_id)
        .unwrap_or(all.len().saturating_sub(1));
    all[s..=e].to_vec()
}

/// Order two cursors by document order (returns (start, end)).
fn order_cursors(
    tree: &rinch_dom::NodeTree,
    ce_root: usize,
    a: DomCursor,
    b: DomCursor,
) -> (DomCursor, DomCursor) {
    if a.node_id == b.node_id {
        return if a.offset <= b.offset { (a, b) } else { (b, a) };
    }
    let mut all = Vec::new();
    collect_text_nodes(tree, ce_root, &mut all);
    let a_pos = all.iter().position(|&id| id == a.node_id);
    let b_pos = all.iter().position(|&id| id == b.node_id);
    match (a_pos, b_pos) {
        (Some(ap), Some(bp)) if ap <= bp => (a, b),
        _ => (b, a),
    }
}

/// Get the next sibling of `child_id` within `parent_id`.
fn next_sibling(tree: &rinch_dom::NodeTree, parent_id: usize, child_id: usize) -> Option<usize> {
    let parent = tree.get(parent_id)?;
    let pos = parent.children.iter().position(|&c| c == child_id)?;
    parent.children.get(pos + 1).copied()
}

/// Walk up from `node_id` to find its ancestor that is a direct child of `ce_root`.
fn find_ce_root_child(tree: &rinch_dom::NodeTree, node_id: usize, ce_root: usize) -> Option<usize> {
    let mut current = node_id;
    loop {
        let parent = tree.get(current)?.parent?;
        if parent == ce_root {
            return Some(current);
        }
        current = parent;
    }
}

/// Find `descendant` (or its nearest ancestor) that is a direct child of `container`.
/// Used to locate which `<li>` in a list corresponds to a selection endpoint.
fn find_child_in(tree: &rinch_dom::NodeTree, descendant: usize, container: usize) -> Option<usize> {
    let mut current = descendant;
    loop {
        let parent = tree.get(current)?.parent?;
        if parent == container {
            return Some(current);
        }
        current = parent;
    }
}

/// Move all children of `element_id` to its parent (before the element), then remove it.
fn unwrap_element(d: &mut RinchDocument, element_id: usize) {
    let parent_id = match d.tree.get(element_id).and_then(|n| n.parent) {
        Some(p) => p,
        None => return,
    };
    let children: Vec<usize> = d
        .tree
        .get(element_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();
    for &child_id in &children {
        d.remove_node(rinch_core::dom::NodeId(child_id));
        d.insert_before(
            rinch_core::dom::NodeId(parent_id),
            rinch_core::dom::NodeId(child_id),
            rinch_core::dom::NodeId(element_id),
        );
    }
    d.remove_node(rinch_core::dom::NodeId(element_id));
}

/// Merge a list element with adjacent lists of the same type.
/// Checks the previous and next siblings — if they are the same list tag,
/// moves all items into one list and removes the others.
fn merge_adjacent_lists(
    d: &mut RinchDocument,
    list_id: usize,
    list_tag: &str,
    parent_id: usize,
) {
    // Merge with previous sibling list
    let prev_list = {
        let siblings = &d.tree.nodes[parent_id].children;
        let pos = siblings.iter().position(|&c| c == list_id);
        pos.and_then(|p| if p > 0 { Some(siblings[p - 1]) } else { None })
            .filter(|&prev_id| {
                d.tree
                    .get(prev_id)
                    .and_then(|n| n.tag())
                    .unwrap_or("")
                    == list_tag
            })
    };

    let target_list = if let Some(prev_id) = prev_list {
        // Move all items from our list into the previous list
        let our_items: Vec<usize> = d.tree.nodes[list_id].children.clone();
        for &item_id in &our_items {
            d.remove_node(rinch_core::dom::NodeId(item_id));
            d.append_child(rinch_core::dom::NodeId(prev_id), rinch_core::dom::NodeId(item_id));
        }
        d.remove_node(rinch_core::dom::NodeId(list_id));
        prev_id
    } else {
        list_id
    };

    // Merge with next sibling list
    let next_list = {
        let siblings = &d.tree.nodes[parent_id].children;
        let pos = siblings.iter().position(|&c| c == target_list);
        pos.and_then(|p| siblings.get(p + 1).copied())
            .filter(|&next_id| {
                d.tree
                    .get(next_id)
                    .and_then(|n| n.tag())
                    .unwrap_or("")
                    == list_tag
            })
    };

    if let Some(next_id) = next_list {
        // Move all items from next list into our target list
        let next_items: Vec<usize> = d.tree.nodes[next_id].children.clone();
        for &item_id in &next_items {
            d.remove_node(rinch_core::dom::NodeId(item_id));
            d.append_child(
                rinch_core::dom::NodeId(target_list),
                rinch_core::dom::NodeId(item_id),
            );
        }
        d.remove_node(rinch_core::dom::NodeId(next_id));
    }
}

// ============================================================================
// CeOps
// ============================================================================

/// ContentEditable operations for a focused CE element.
///
/// Wraps a `RinchDocument` and cursor state. Implements [`ContentEditableApi`]
/// so the editor bridge can query selection state and (in future) perform
/// DOM mutations through the same API that app.rs uses.
pub struct CeOps {
    /// The backing DOM document.
    doc: Rc<RefCell<RinchDocument>>,
    /// The CE root node ID.
    ce_node_id: usize,
    /// Current cursor (head of selection).
    cursor: DomCursor,
    /// Selection anchor.
    anchor: DomCursor,
    /// Per-instance event dispatcher.
    dispatcher: CeEventDispatcher,
}

impl CeOps {
    /// Create a new CeOps for a focused contentEditable element.
    pub fn new(doc: Rc<RefCell<RinchDocument>>, ce_node_id: usize, cursor: DomCursor) -> Self {
        Self {
            doc,
            ce_node_id,
            cursor,
            anchor: cursor,
            dispatcher: CeEventDispatcher::new(),
        }
    }

    /// Get the CE root node ID.
    pub fn ce_node_id(&self) -> usize {
        self.ce_node_id
    }

    /// Update cursor/anchor from app.rs after it handles input directly.
    ///
    /// Called by app.rs to keep CeOps in sync after operations that
    /// app.rs handles inline (e.g., text insertion, deletion, mouse clicks).
    pub fn sync_cursor(&mut self, cursor: DomCursor, anchor: DomCursor) {
        self.cursor = cursor;
        self.anchor = anchor;
    }

    /// Handle `set_block_type` when the selection spans blocks with different parents.
    ///
    /// Lifts the view to the CE root level, collects leaf blocks (with partial
    /// extraction for lists/blockquotes at selection boundaries), applies the
    /// target block type, and cleans up empty containers.
    fn set_block_type_cross_parent(
        &mut self,
        anchor_block: usize,
        cursor_block: usize,
        tag: &str,
    ) {
        let ce_root = self.ce_node_id;

        // 1. Find the CE root children that contain anchor/cursor blocks
        let (top_start, top_end, start_block, end_block) = {
            let d = self.doc.borrow();
            let ts = find_ce_root_child(&d.tree, anchor_block, ce_root);
            let te = find_ce_root_child(&d.tree, cursor_block, ce_root);
            let (ts, te) = match (ts, te) {
                (Some(a), Some(b)) => (a, b),
                _ => return,
            };
            // Order by document position
            let children = &d.tree.nodes[ce_root].children;
            let a_pos = children.iter().position(|&c| c == ts).unwrap_or(0);
            let b_pos = children.iter().position(|&c| c == te).unwrap_or(0);
            if a_pos <= b_pos {
                (ts, te, anchor_block, cursor_block)
            } else {
                (te, ts, cursor_block, anchor_block)
            }
        };

        // 2. Collect the range of CE root children between top_start..=top_end
        let root_children_in_range: Vec<usize> = {
            let d = self.doc.borrow();
            let children = &d.tree.nodes[ce_root].children;
            let s = children.iter().position(|&c| c == top_start).unwrap_or(0);
            let e = children
                .iter()
                .position(|&c| c == top_end)
                .unwrap_or(children.len().saturating_sub(1));
            children[s..=e].to_vec()
        };

        // 3. Collect leaf blocks with partial extraction for first/last containers
        //    Each entry: (node_id, needs_extraction_from_container)
        //    We gather IDs now and do mutations in a single pass.
        struct LeafBlock {
            id: usize,
            /// If this block is inside a container (list/bq), track the container
            /// so we can clean it up if empty later.
            source_container: Option<usize>,
        }

        let mut leaf_blocks: Vec<LeafBlock> = Vec::new();
        // Track containers we partially/fully extract from, so we can remove if empty
        let mut affected_containers: Vec<usize> = Vec::new();

        {
            let d = self.doc.borrow();
            for (i, &root_child) in root_children_in_range.iter().enumerate() {
                let child_tag = d
                    .tree
                    .get(root_child)
                    .and_then(|n| n.tag())
                    .unwrap_or("");
                let is_first = i == 0;
                let is_last = i == root_children_in_range.len() - 1;

                if RinchApp::is_list_tag(child_tag) || child_tag == "blockquote" {
                    // Container: extract relevant children
                    let container_children = d.tree.nodes[root_child].children.clone();
                    if container_children.is_empty() {
                        continue;
                    }

                    affected_containers.push(root_child);

                    // Determine start/end indices within this container
                    let extract_start = if is_first {
                        // Partial: from the child containing start_block to end
                        find_child_in(&d.tree, start_block, root_child)
                            .and_then(|child_id| {
                                container_children.iter().position(|&c| c == child_id)
                            })
                            .unwrap_or(0)
                    } else {
                        0
                    };

                    let extract_end = if is_last {
                        // Partial: from start to the child containing end_block
                        find_child_in(&d.tree, end_block, root_child)
                            .and_then(|child_id| {
                                container_children.iter().position(|&c| c == child_id)
                            })
                            .unwrap_or(container_children.len().saturating_sub(1))
                    } else {
                        container_children.len() - 1
                    };

                    for &child_id in &container_children[extract_start..=extract_end] {
                        leaf_blocks.push(LeafBlock {
                            id: child_id,
                            source_container: Some(root_child),
                        });
                    }
                } else {
                    // Plain block (p, h*, div, etc.) — include directly
                    leaf_blocks.push(LeafBlock {
                        id: root_child,
                        source_container: None,
                    });
                }
            }
        }

        if leaf_blocks.is_empty() {
            return;
        }

        // 4. Apply the target block type
        // First, extract all leaf blocks from their containers and place at CE root level.
        // Then apply the target type transformation.

        // Find insertion reference: the next sibling of top_end in CE root
        let insert_before_ref = {
            let d = self.doc.borrow();
            next_sibling(&d.tree, ce_root, top_end)
        };

        // Extract leaf blocks from containers → place at CE root before insert_before_ref
        let leaf_ids: Vec<usize> = leaf_blocks.iter().map(|lb| lb.id).collect();

        {
            let mut d = self.doc.borrow_mut();

            // Remove leaf blocks from their current parents
            for lb in &leaf_blocks {
                if lb.source_container.is_some() {
                    d.remove_node(rinch_core::dom::NodeId(lb.id));
                }
            }

            // Place them at CE root level (before insert_before_ref)
            // We place blocks that were inside containers; blocks already at CE root stay
            for lb in &leaf_blocks {
                if lb.source_container.is_some() {
                    if let Some(ref_id) = insert_before_ref {
                        d.insert_before(
                            rinch_core::dom::NodeId(ce_root),
                            rinch_core::dom::NodeId(lb.id),
                            rinch_core::dom::NodeId(ref_id),
                        );
                    } else {
                        d.append_child(
                            rinch_core::dom::NodeId(ce_root),
                            rinch_core::dom::NodeId(lb.id),
                        );
                    }
                }
            }

            // Clean up empty containers
            for &container_id in &affected_containers {
                if d.tree.nodes[container_id].children.is_empty() {
                    d.remove_node(rinch_core::dom::NodeId(container_id));
                }
            }
        }

        // Now all leaf blocks are direct children of CE root.
        // Apply the target block type.
        if RinchApp::is_list_tag(tag) {
            // Convert non-li blocks to li, then wrap all in a single list
            let mut d = self.doc.borrow_mut();

            // Find the first leaf block's position to know where to insert the new list
            let first_id = leaf_ids[0];
            let list_insert_before = next_sibling(&d.tree, ce_root, first_id);

            let list = d.create_element(tag);

            for &block_id in &leaf_ids {
                let block_tag = d
                    .tree
                    .get(block_id)
                    .and_then(|n| n.tag())
                    .unwrap_or("")
                    .to_string();

                let li_id = if block_tag == "li" {
                    rinch_core::dom::NodeId(block_id)
                } else {
                    RinchApp::convert_block_tag(&mut d, block_id, "li")
                };
                d.remove_node(li_id);
                d.append_child(list, li_id);
            }

            if let Some(ref_id) = list_insert_before {
                d.insert_before(
                    rinch_core::dom::NodeId(ce_root),
                    list,
                    rinch_core::dom::NodeId(ref_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(ce_root), list);
            }
        } else if tag == "blockquote" {
            // Convert li blocks to p, then wrap all in a single blockquote
            let mut d = self.doc.borrow_mut();

            let first_id = leaf_ids[0];
            let bq_insert_before = next_sibling(&d.tree, ce_root, first_id);

            let bq = d.create_element("blockquote");

            for &block_id in &leaf_ids {
                let block_tag = d
                    .tree
                    .get(block_id)
                    .and_then(|n| n.tag())
                    .unwrap_or("")
                    .to_string();

                let inner_id = if block_tag == "li" {
                    RinchApp::convert_block_tag(&mut d, block_id, "p")
                } else {
                    rinch_core::dom::NodeId(block_id)
                };
                d.remove_node(inner_id);
                d.append_child(bq, inner_id);
            }

            if let Some(ref_id) = bq_insert_before {
                d.insert_before(
                    rinch_core::dom::NodeId(ce_root),
                    bq,
                    rinch_core::dom::NodeId(ref_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(ce_root), bq);
            }
        } else {
            // Simple tag (h1-h6, p, div): convert each block independently
            let mut d = self.doc.borrow_mut();
            for &block_id in &leaf_ids {
                let block_tag = d
                    .tree
                    .get(block_id)
                    .and_then(|n| n.tag())
                    .unwrap_or("")
                    .to_string();

                if block_tag != tag || block_tag == "li" {
                    RinchApp::convert_block_tag(&mut d, block_id, tag);
                }
            }
        }

        dispatch_ce_event(&CeEvent::BlockTypeChanged {
            old_node_id: leaf_ids[0],
            new_node_id: 0,
            old_tag: String::new(),
            new_tag: tag.to_string(),
        });
    }

    /// Handle `set_block_type` when multiple blocks are selected.
    ///
    /// `block_ids` are sibling node IDs under `common_parent`, in document order.
    fn set_block_type_multi(&mut self, block_ids: &[usize], common_parent: usize, tag: &str) {
        if RinchApp::is_list_tag(tag) {
            // Check if all blocks are <li> inside a list container
            let (all_li, parent_tag) = {
                let d = self.doc.borrow();
                let parent_tag = d
                    .tree
                    .get(common_parent)
                    .and_then(|n| n.tag())
                    .unwrap_or("")
                    .to_string();
                let all_li = RinchApp::is_list_tag(&parent_tag)
                    && block_ids.iter().all(|&bid| {
                        d.tree.get(bid).and_then(|n| n.tag()).unwrap_or("") == "li"
                    });
                (all_li, parent_tag)
            };

            if all_li && parent_tag == tag {
                // ── Toggle off: extract <li> items as <p> from the list ──
                let mut d = self.doc.borrow_mut();
                let list_id = common_parent;
                let list_parent = d
                    .tree
                    .get(list_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(self.ce_node_id);

                // Collect items after the selection that need a new list
                let all_children = d.tree.nodes[list_id].children.clone();
                let first_selected = block_ids[0];
                let last_selected = *block_ids.last().unwrap();
                let first_pos = all_children
                    .iter()
                    .position(|&c| c == first_selected)
                    .unwrap_or(0);
                let last_pos = all_children
                    .iter()
                    .position(|&c| c == last_selected)
                    .unwrap_or(0);
                let after_items: Vec<usize> = all_children[last_pos + 1..].to_vec();
                let has_before = first_pos > 0;

                // Reference point: insert after the list in the list's parent
                let list_next_sib = next_sibling(&d.tree, list_parent, list_id);

                // Convert each selected <li> to <p> in-place, then move out of list
                let mut converted = Vec::new();
                for &block_id in block_ids {
                    let p = RinchApp::convert_block_tag(&mut d, block_id, "p");
                    d.remove_node(p);
                    converted.push(p);
                }

                // Insert converted <p> elements after the list
                for &p in &converted {
                    if let Some(next) = list_next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(list_parent),
                            p,
                            rinch_core::dom::NodeId(next),
                        );
                    } else {
                        d.append_child(rinch_core::dom::NodeId(list_parent), p);
                    }
                }

                // If there are items after the selection, create a new list for them
                if !after_items.is_empty() {
                    let new_list = d.create_element(tag);
                    for &item_id in &after_items {
                        d.remove_node(rinch_core::dom::NodeId(item_id));
                        d.append_child(new_list, rinch_core::dom::NodeId(item_id));
                    }
                    if let Some(next) = list_next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(list_parent),
                            new_list,
                            rinch_core::dom::NodeId(next),
                        );
                    } else {
                        d.append_child(rinch_core::dom::NodeId(list_parent), new_list);
                    }
                }

                // Remove the original list if empty (all items selected)
                if !has_before && after_items.is_empty() {
                    d.remove_node(rinch_core::dom::NodeId(list_id));
                }
            } else if all_li && parent_tag != tag {
                // ── Different list type: change the container tag ──
                let mut d = self.doc.borrow_mut();
                RinchApp::convert_block_tag(&mut d, common_parent, tag);
            } else {
                // ── Not in a list: convert all blocks to <li> in a new list ──
                let mut d = self.doc.borrow_mut();
                let last_block = *block_ids.last().unwrap();
                let after_sib = next_sibling(&d.tree, common_parent, last_block);

                let list = d.create_element(tag);
                for &block_id in block_ids {
                    let li = RinchApp::convert_block_tag(&mut d, block_id, "li");
                    d.remove_node(li);
                    d.append_child(list, li);
                }

                if let Some(after) = after_sib {
                    d.insert_before(
                        rinch_core::dom::NodeId(common_parent),
                        list,
                        rinch_core::dom::NodeId(after),
                    );
                } else {
                    d.append_child(rinch_core::dom::NodeId(common_parent), list);
                }

                // Merge with adjacent lists of the same type
                merge_adjacent_lists(&mut d, list.0, tag, common_parent);
            }
        } else if tag == "blockquote" {
            // Check if all blocks are inside a blockquote (toggle off)
            let parent_is_bq = {
                let d = self.doc.borrow();
                d.tree
                    .get(common_parent)
                    .and_then(|n| n.tag())
                    .unwrap_or("")
                    == "blockquote"
            };

            if parent_is_bq {
                // ── Toggle off: extract blocks from blockquote, splitting if needed ──
                let mut d = self.doc.borrow_mut();
                let bq_id = common_parent;
                let bq_parent = d
                    .tree
                    .get(bq_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(self.ce_node_id);

                // Determine which children are before/after the selection
                let bq_children = d.tree.nodes[bq_id].children.clone();
                let first_selected = block_ids[0];
                let last_selected = *block_ids.last().unwrap();
                let first_pos = bq_children
                    .iter()
                    .position(|&c| c == first_selected)
                    .unwrap_or(0);
                let last_pos = bq_children
                    .iter()
                    .position(|&c| c == last_selected)
                    .unwrap_or(0);
                let after_items: Vec<usize> = bq_children[last_pos + 1..].to_vec();
                let has_before = first_pos > 0;

                let bq_next_sib = next_sibling(&d.tree, bq_parent, bq_id);

                // Extract selected blocks (insert after original BQ)
                for &block_id in block_ids {
                    d.remove_node(rinch_core::dom::NodeId(block_id));
                    if let Some(next) = bq_next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(bq_parent),
                            rinch_core::dom::NodeId(block_id),
                            rinch_core::dom::NodeId(next),
                        );
                    } else {
                        d.append_child(
                            rinch_core::dom::NodeId(bq_parent),
                            rinch_core::dom::NodeId(block_id),
                        );
                    }
                }

                // If there are items after selection, move them to a new blockquote
                if !after_items.is_empty() {
                    let new_bq = d.create_element("blockquote");
                    for &item_id in &after_items {
                        d.remove_node(rinch_core::dom::NodeId(item_id));
                        d.append_child(new_bq, rinch_core::dom::NodeId(item_id));
                    }
                    if let Some(next) = bq_next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(bq_parent),
                            new_bq,
                            rinch_core::dom::NodeId(next),
                        );
                    } else {
                        d.append_child(rinch_core::dom::NodeId(bq_parent), new_bq);
                    }
                }

                // Remove original blockquote if empty
                if !has_before && after_items.is_empty() {
                    d.remove_node(rinch_core::dom::NodeId(bq_id));
                }
            } else {
                // ── Wrap all blocks in a single <blockquote> ──
                let mut d = self.doc.borrow_mut();
                let last_block = *block_ids.last().unwrap();
                let after_sib = next_sibling(&d.tree, common_parent, last_block);

                let bq = d.create_element("blockquote");
                for &block_id in block_ids {
                    d.remove_node(rinch_core::dom::NodeId(block_id));
                    d.append_child(bq, rinch_core::dom::NodeId(block_id));
                }

                if let Some(after) = after_sib {
                    d.insert_before(
                        rinch_core::dom::NodeId(common_parent),
                        bq,
                        rinch_core::dom::NodeId(after),
                    );
                } else {
                    d.append_child(rinch_core::dom::NodeId(common_parent), bq);
                }
            }
        } else {
            // ── Simple tag: convert each block independently ──
            // If ALL blocks are already the target tag, toggle to <p>.
            let all_same = {
                let d = self.doc.borrow();
                block_ids.iter().all(|&bid| {
                    d.tree.get(bid).and_then(|n| n.tag()).unwrap_or("") == tag
                })
            };
            let target = if all_same { "p" } else { tag };
            let mut d = self.doc.borrow_mut();
            for &block_id in block_ids {
                RinchApp::convert_block_tag(&mut d, block_id, target);
            }
        }

        dispatch_ce_event(&CeEvent::BlockTypeChanged {
            old_node_id: block_ids[0],
            new_node_id: 0,
            old_tag: String::new(),
            new_tag: tag.to_string(),
        });
    }
}

impl std::fmt::Debug for CeOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CeOps")
            .field("ce_node_id", &self.ce_node_id)
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .finish()
    }
}

impl ContentEditableApi for CeOps {
    // ── Text Operations ──────────────────────────────────────────────

    fn insert_text(&mut self, text: &str) {
        if self.cursor != self.anchor {
            self.delete_selection();
        }
        let cur = self.cursor;
        let ce_node_id = self.ce_node_id;
        {
            let mut d = self.doc.borrow_mut();
            // Check if cursor is on a <br> element
            let is_br = d
                .tree
                .get(cur.node_id)
                .and_then(|n| n.tag())
                .map(|t| t == "br")
                .unwrap_or(false);

            if is_br {
                // <br> cursor: create text node and insert before the <br>
                let parent_id = d
                    .tree
                    .get(cur.node_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(ce_node_id);
                let text_id = d.create_text(text);
                d.insert_before(
                    rinch_core::dom::NodeId(parent_id),
                    text_id,
                    rinch_core::dom::NodeId(cur.node_id),
                );
                self.cursor = DomCursor::new(text_id.0, text.len());
                self.anchor = self.cursor;
            } else if let Some(node) = d.tree.get(cur.node_id)
                && let Some(current) = node.text_content().map(|s| s.to_string())
            {
                let off = cur.offset.min(current.len());
                let mut new_text = String::with_capacity(current.len() + text.len());
                new_text.push_str(&current[..off]);
                new_text.push_str(text);
                new_text.push_str(&current[off..]);

                // Strip ZWS characters that were placeholders for cursor positioning
                // (created by toggle_wrap for entering/escaping formatting spans)
                if new_text.contains('\u{200B}') {
                    let cursor_before_strip = off + text.len();
                    // Count ZWS bytes before the cursor position to adjust offset
                    let zws_bytes_before_cursor = new_text[..cursor_before_strip]
                        .chars()
                        .filter(|c| *c == '\u{200B}')
                        .count()
                        * '\u{200B}'.len_utf8();
                    new_text = new_text.replace('\u{200B}', "");
                    let adjusted_offset = cursor_before_strip - zws_bytes_before_cursor;
                    d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                    self.cursor = DomCursor::new(cur.node_id, adjusted_offset);
                } else {
                    d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                    self.cursor = DomCursor::new(cur.node_id, off + text.len());
                }
                self.anchor = self.cursor;
            } else {
                // Cursor is on an element node (empty block) — create text child
                let text_id = d.create_text(text);
                d.append_child(rinch_core::dom::NodeId(cur.node_id), text_id);
                d.set_style(rinch_core::dom::NodeId(cur.node_id), "min-height", "0");
                self.cursor = DomCursor::new(text_id.0, text.len());
                self.anchor = self.cursor;
            }
        }
        dispatch_ce_event(&CeEvent::TextInserted {
            node_id: self.cursor.node_id,
            offset: self.cursor.offset.saturating_sub(text.len()),
            text: text.to_string(),
        });
    }

    fn delete_backward(&mut self) {
        if self.cursor != self.anchor {
            self.delete_selection();
            return;
        }
        let cur = self.cursor;
        let ce_node_id = self.ce_node_id;

        // Check if cursor is on a <br> element
        let is_br_cursor = self
            .doc
            .borrow()
            .tree
            .get(cur.node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "br")
            .unwrap_or(false);

        if is_br_cursor {
            // Remove the <br> and move cursor to end of prev text or start of next
            let (new_cursor, br_parent_id) = {
                let d = self.doc.borrow();
                let br_parent = d
                    .tree
                    .get(cur.node_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(ce_node_id);
                let prev = RinchApp::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                let next = RinchApp::next_text_node(&d.tree, ce_node_id, cur.node_id);
                let nc: Option<DomCursor> = if let Some(prev_id) = prev {
                    let prev_is_br = d
                        .tree
                        .get(prev_id)
                        .and_then(|n| n.tag())
                        .map(|t| t == "br")
                        .unwrap_or(false);
                    if prev_is_br {
                        Some(DomCursor::new(prev_id, 0))
                    } else {
                        let len = d
                            .tree
                            .get(prev_id)
                            .and_then(|n| n.text_content())
                            .map(|s| s.len())
                            .unwrap_or(0);
                        Some(DomCursor::new(prev_id, len))
                    }
                } else {
                    next.map(|next_id| DomCursor::new(next_id, 0))
                };
                (nc, br_parent)
            };
            {
                let mut d = self.doc.borrow_mut();
                d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                if let Some(cursor) = new_cursor {
                    self.cursor = cursor;
                } else {
                    let text_id = d.create_text("");
                    d.append_child(rinch_core::dom::NodeId(ce_node_id), text_id);
                    self.cursor = DomCursor::new(text_id.0, 0);
                }
                self.anchor = self.cursor;
            }
            dispatch_ce_event(&CeEvent::NodeRemoved {
                node_id: cur.node_id,
                parent_id: br_parent_id,
            });
            return;
        }

        let is_element = {
            let d = self.doc.borrow();
            RinchApp::is_element_cursor(&d.tree, &cur)
        };

        if is_element {
            // ── Cursor at empty block element ──
            let cur_block = {
                let d = self.doc.borrow();
                RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id)
            };
            if let Some((cur_block_id, block_parent_id)) = cur_block {
                let (cur_tag, parent_tag) = {
                    let d = self.doc.borrow();
                    let ct = d
                        .tree
                        .get(cur_block_id)
                        .and_then(|n| n.tag())
                        .unwrap_or("")
                        .to_string();
                    let pt = d
                        .tree
                        .get(block_parent_id)
                        .and_then(|n| n.tag())
                        .unwrap_or("")
                        .to_string();
                    (ct, pt)
                };

                if cur_tag == "li" && RinchApp::is_list_tag(&parent_tag) {
                    let new_el = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::outdent_li(&mut d, cur_block_id, block_parent_id, ce_node_id)
                    };
                    self.cursor = DomCursor::new(new_el.0, 0);
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::ListItemOutdented {
                        old_li_id: cur_block_id,
                        new_block_id: new_el.0,
                    });
                } else if let Some((li_id, list_id)) = {
                    let d = self.doc.borrow();
                    RinchApp::find_li_ancestor_for_outdent(&d.tree, cur_block_id, ce_node_id)
                } {
                    let new_el = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::outdent_li(&mut d, li_id, list_id, ce_node_id)
                    };
                    self.cursor = DomCursor::new(new_el.0, 0);
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::ListItemOutdented {
                        old_li_id: li_id,
                        new_block_id: new_el.0,
                    });
                } else if RinchApp::is_heading(&cur_tag) || cur_tag == "blockquote" {
                    let new_el = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::convert_block_tag(&mut d, cur_block_id, "p")
                    };
                    self.cursor = DomCursor::new(new_el.0, 0);
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::BlockTypeChanged {
                        old_node_id: cur_block_id,
                        new_node_id: new_el.0,
                        old_tag: cur_tag.clone(),
                        new_tag: "p".to_string(),
                    });
                } else {
                    // Default: remove the empty block, cursor to end of previous block
                    let (prev_cursor, prev_block_id) = {
                        let d = self.doc.borrow();
                        let siblings = &d.tree.nodes[block_parent_id].children;
                        let pos = siblings.iter().position(|&c| c == cur_block_id);
                        let prev_block_id =
                            pos.and_then(|p| if p > 0 { Some(siblings[p - 1]) } else { None });
                        let prev_cursor =
                            prev_block_id.and_then(|pb| RinchApp::last_text_cursor(&d.tree, pb));
                        (prev_cursor, prev_block_id)
                    };
                    {
                        let mut d = self.doc.borrow_mut();
                        d.remove_node(rinch_core::dom::NodeId(cur_block_id));
                    }
                    if let Some(pc) = prev_cursor {
                        self.cursor = pc;
                    } else if let Some(pb) = prev_block_id {
                        self.cursor = DomCursor::new(pb, 0);
                    }
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::NodeRemoved {
                        node_id: cur_block_id,
                        parent_id: block_parent_id,
                    });
                }
            } else if cur.node_id == ce_node_id {
                // Cursor is on the CE root element itself
                let last = {
                    let d = self.doc.borrow();
                    RinchApp::last_text_cursor(&d.tree, ce_node_id)
                };
                if let Some(lc) = last {
                    self.cursor = lc;
                    self.anchor = self.cursor;
                }
            }
            return;
        }

        if cur.offset > 0 {
            // ── Delete char before cursor in current text node ──
            let deleted_info = {
                let mut d = self.doc.borrow_mut();
                if let Some(node) = d.tree.get(cur.node_id)
                    && let Some(current) = node.text_content().map(|s| s.to_string())
                {
                    let off = cur.offset.min(current.len());
                    let prev_char_start = current[..off]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    let delete_len = off - prev_char_start;
                    let mut new_text = String::with_capacity(current.len());
                    new_text.push_str(&current[..prev_char_start]);
                    new_text.push_str(&current[off..]);
                    if new_text.is_empty() {
                        // Text node is now empty — find nearest cursor target
                        let prev = RinchApp::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                        let next = RinchApp::next_text_node(&d.tree, ce_node_id, cur.node_id);

                        if let Some(prev_id) = prev {
                            let prev_is_br = d
                                .tree
                                .get(prev_id)
                                .and_then(|n| n.tag())
                                .map(|t| t == "br")
                                .unwrap_or(false);
                            d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                            if prev_is_br {
                                self.cursor = DomCursor::new(prev_id, 0);
                            } else {
                                let len = d
                                    .tree
                                    .get(prev_id)
                                    .and_then(|n| n.text_content())
                                    .map(|s| s.len())
                                    .unwrap_or(0);
                                self.cursor = DomCursor::new(prev_id, len);
                            }
                        } else if let Some(next_id) = next {
                            d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                            self.cursor = DomCursor::new(next_id, 0);
                        } else {
                            // CE is completely empty — keep as empty text node
                            d.set_text_content(rinch_core::dom::NodeId(cur.node_id), "");
                            self.cursor = DomCursor::new(cur.node_id, 0);
                        }
                    } else {
                        d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                        self.cursor = DomCursor::new(cur.node_id, prev_char_start);
                    }
                    self.anchor = self.cursor;
                    Some((prev_char_start, delete_len))
                } else {
                    None
                }
            };
            if let Some((offset, length)) = deleted_info {
                dispatch_ce_event(&CeEvent::TextDeleted {
                    node_id: cur.node_id,
                    offset,
                    length,
                });
            }
            return;
        }

        // ── At start of text node — find previous text node and merge ──
        let prev = {
            let d = self.doc.borrow();
            RinchApp::prev_text_node(&d.tree, ce_node_id, cur.node_id)
        };

        if let Some(prev) = prev {
            let (cur_block, prev_block) = {
                let d = self.doc.borrow();
                let cb = RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);
                let pb = RinchApp::find_block_and_parent(&d.tree, prev, ce_node_id);
                (cb, pb)
            };
            let cross_block =
                cur_block.is_some() && cur_block.map(|(b, _)| b) != prev_block.map(|(b, _)| b);

            if let Some((cur_block_id, cur_block_parent)) = cur_block {
                let (cur_tag, parent_tag) = {
                    let d = self.doc.borrow();
                    let ct = d
                        .tree
                        .get(cur_block_id)
                        .and_then(|n| n.tag())
                        .unwrap_or("")
                        .to_string();
                    let pt = d
                        .tree
                        .get(cur_block_parent)
                        .and_then(|n| n.tag())
                        .unwrap_or("")
                        .to_string();
                    (ct, pt)
                };

                // Backspace at start of <li>
                if cur_tag == "li" && RinchApp::is_list_tag(&parent_tag) {
                    let is_first = {
                        let d = self.doc.borrow();
                        d.tree.nodes[cur_block_parent]
                            .children
                            .first()
                            == Some(&cur_block_id)
                    };

                    if is_first {
                        // First LI: outdent (exit list)
                        let new_el = {
                            let mut d = self.doc.borrow_mut();
                            RinchApp::outdent_li(
                                &mut d,
                                cur_block_id,
                                cur_block_parent,
                                ce_node_id,
                            )
                        };
                        self.cursor = {
                            let d = self.doc.borrow();
                            RinchApp::first_text_cursor(&d.tree, new_el.0)
                                .unwrap_or(DomCursor::new(new_el.0, 0))
                        };
                        self.anchor = self.cursor;
                        dispatch_ce_event(&CeEvent::ListItemOutdented {
                            old_li_id: cur_block_id,
                            new_block_id: new_el.0,
                        });
                    } else {
                        // Non-first LI: merge content into previous LI
                        let prev_li_id;
                        let merge_offset;
                        {
                            let mut d = self.doc.borrow_mut();
                            let siblings =
                                d.tree.nodes[cur_block_parent].children.clone();
                            let pos = siblings
                                .iter()
                                .position(|&c| c == cur_block_id)
                                .unwrap_or(0);
                            prev_li_id = siblings[pos - 1];

                            let merge_cursor =
                                RinchApp::last_text_cursor(&d.tree, prev_li_id)
                                    .unwrap_or(DomCursor::new(prev_li_id, 0));
                            merge_offset = merge_cursor.offset;

                            let cur_children: Vec<usize> =
                                d.tree.nodes[cur_block_id].children.clone();
                            let mut first = true;
                            for &child_id in &cur_children {
                                if first {
                                    first = false;
                                    let child_is_text = d
                                        .tree
                                        .get(child_id)
                                        .and_then(|n| n.text_content())
                                        .is_some();
                                    let merge_is_text = d
                                        .tree
                                        .get(merge_cursor.node_id)
                                        .and_then(|n| n.text_content())
                                        .is_some();
                                    if child_is_text && merge_is_text {
                                        let child_text = d
                                            .tree
                                            .get(child_id)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        let merge_text = d
                                            .tree
                                            .get(merge_cursor.node_id)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        let merged =
                                            format!("{}{}", merge_text, child_text);
                                        d.set_text_content(
                                            rinch_core::dom::NodeId(
                                                merge_cursor.node_id,
                                            ),
                                            &merged,
                                        );
                                        d.remove_node(rinch_core::dom::NodeId(
                                            child_id,
                                        ));
                                        continue;
                                    }
                                }
                                d.remove_node(rinch_core::dom::NodeId(child_id));
                                d.append_child(
                                    rinch_core::dom::NodeId(prev_li_id),
                                    rinch_core::dom::NodeId(child_id),
                                );
                            }
                            d.remove_node(rinch_core::dom::NodeId(cur_block_id));
                            self.cursor = merge_cursor;
                            self.anchor = self.cursor;
                        }
                        dispatch_ce_event(&CeEvent::BlockJoined {
                            surviving_block_id: prev_li_id,
                            removed_block_id: cur_block_id,
                            merge_offset,
                        });
                    }
                } else if let Some((li_id, list_id)) = {
                    let d = self.doc.borrow();
                    RinchApp::find_li_ancestor_for_outdent(&d.tree, cur_block_id, ce_node_id)
                } {
                    let new_el = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::outdent_li(&mut d, li_id, list_id, ce_node_id)
                    };
                    self.cursor = {
                        let d = self.doc.borrow();
                        RinchApp::first_text_cursor(&d.tree, new_el.0)
                            .unwrap_or(DomCursor::new(new_el.0, 0))
                    };
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::ListItemOutdented {
                        old_li_id: li_id,
                        new_block_id: new_el.0,
                    });
                } else if RinchApp::is_heading(&cur_tag) || cur_tag == "blockquote" {
                    let new_el = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::convert_block_tag(&mut d, cur_block_id, "p")
                    };
                    self.cursor = {
                        let d = self.doc.borrow();
                        RinchApp::first_text_cursor(&d.tree, new_el.0)
                            .unwrap_or(DomCursor::new(new_el.0, 0))
                    };
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::BlockTypeChanged {
                        old_node_id: cur_block_id,
                        new_node_id: new_el.0,
                        old_tag: cur_tag.clone(),
                        new_tag: "p".to_string(),
                    });
                } else {
                    // Normal cross-block merge or same-block merge
                    if cross_block {
                        let (prev_block_id, _) = prev_block.unwrap();
                        let merge_offset;
                        {
                            let mut d = self.doc.borrow_mut();
                            let merge_cursor = RinchApp::last_text_cursor(&d.tree, prev_block_id)
                                .unwrap_or(DomCursor::new(prev, 0));
                            merge_offset = merge_cursor.offset;
                            let cur_children: Vec<usize> =
                                d.tree.nodes[cur_block_id].children.clone();

                            let mut first = true;
                            for &child_id in &cur_children {
                                if first {
                                    first = false;
                                    let child_is_text = d
                                        .tree
                                        .get(child_id)
                                        .and_then(|n| n.text_content())
                                        .is_some();
                                    let merge_is_text = d
                                        .tree
                                        .get(merge_cursor.node_id)
                                        .and_then(|n| n.text_content())
                                        .is_some();
                                    if child_is_text && merge_is_text {
                                        let child_text = d
                                            .tree
                                            .get(child_id)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        let merge_text = d
                                            .tree
                                            .get(merge_cursor.node_id)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        let merged = format!("{}{}", merge_text, child_text);
                                        d.set_text_content(
                                            rinch_core::dom::NodeId(merge_cursor.node_id),
                                            &merged,
                                        );
                                        d.remove_node(rinch_core::dom::NodeId(child_id));
                                        continue;
                                    }
                                }
                                // Move remaining children to prev block
                                d.remove_node(rinch_core::dom::NodeId(child_id));
                                d.append_child(
                                    rinch_core::dom::NodeId(prev_block_id),
                                    rinch_core::dom::NodeId(child_id),
                                );
                            }
                            d.remove_node(rinch_core::dom::NodeId(cur_block_id));
                            self.cursor = merge_cursor;
                            self.anchor = self.cursor;
                        }
                        dispatch_ce_event(&CeEvent::BlockJoined {
                            surviving_block_id: prev_block_id,
                            removed_block_id: cur_block_id,
                            merge_offset,
                        });
                    } else {
                        // Check if prev is a <br> — just remove it
                        let prev_parent_id = {
                            let d = self.doc.borrow();
                            d.tree
                                .get(prev)
                                .and_then(|n| n.parent)
                                .unwrap_or(ce_node_id)
                        };
                        let prev_is_br = {
                            let d = self.doc.borrow();
                            d.tree
                                .get(prev)
                                .and_then(|n| n.tag())
                                .map(|t| t == "br")
                                .unwrap_or(false)
                        };
                        if prev_is_br {
                            {
                                let mut d = self.doc.borrow_mut();
                                d.remove_node(rinch_core::dom::NodeId(prev));
                            }
                            self.cursor = cur;
                            self.anchor = self.cursor;
                            dispatch_ce_event(&CeEvent::NodeRemoved {
                                node_id: prev,
                                parent_id: prev_parent_id,
                            });
                        } else {
                            // Same block or inline — merge text nodes
                            let cur_text_len = {
                                let d = self.doc.borrow();
                                d.tree
                                    .get(cur.node_id)
                                    .and_then(|n| n.text_content())
                                    .map(|s| s.len())
                                    .unwrap_or(0)
                            };
                            {
                                let mut d = self.doc.borrow_mut();
                                let prev_text = d
                                    .tree
                                    .get(prev)
                                    .and_then(|n| n.text_content())
                                    .map(|s| s.to_string())
                                    .unwrap_or_default();
                                let prev_len = prev_text.len();
                                let cur_text = d
                                    .tree
                                    .get(cur.node_id)
                                    .and_then(|n| n.text_content())
                                    .map(|s| s.to_string())
                                    .unwrap_or_default();
                                let merged = format!("{}{}", prev_text, cur_text);
                                d.set_text_content(rinch_core::dom::NodeId(prev), &merged);
                                d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                self.cursor = DomCursor::new(prev, prev_len);
                                self.anchor = self.cursor;
                            }
                            // Dispatch TextDeleted for the removed text node content
                            // (it was merged into prev, so effectively deleted at offset 0)
                            if cur_text_len > 0 {
                                dispatch_ce_event(&CeEvent::TextDeleted {
                                    node_id: cur.node_id,
                                    offset: 0,
                                    length: cur_text_len,
                                });
                            }
                        }
                    }
                }
            } else {
                // No block found — inline merge
                let (prev_is_br, prev_parent_id) = {
                    let d = self.doc.borrow();
                    let is_br = d
                        .tree
                        .get(prev)
                        .and_then(|n| n.tag())
                        .map(|t| t == "br")
                        .unwrap_or(false);
                    let parent = d
                        .tree
                        .get(prev)
                        .and_then(|n| n.parent)
                        .unwrap_or(ce_node_id);
                    (is_br, parent)
                };
                if prev_is_br {
                    {
                        let mut d = self.doc.borrow_mut();
                        d.remove_node(rinch_core::dom::NodeId(prev));
                    }
                    self.cursor = cur;
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::NodeRemoved {
                        node_id: prev,
                        parent_id: prev_parent_id,
                    });
                } else {
                    let cur_text_len = {
                        let d = self.doc.borrow();
                        d.tree
                            .get(cur.node_id)
                            .and_then(|n| n.text_content())
                            .map(|s| s.len())
                            .unwrap_or(0)
                    };
                    {
                        let mut d = self.doc.borrow_mut();
                        let prev_text = d
                            .tree
                            .get(prev)
                            .and_then(|n| n.text_content())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let prev_len = prev_text.len();
                        let cur_text_str = d
                            .tree
                            .get(cur.node_id)
                            .and_then(|n| n.text_content())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let merged = format!("{}{}", prev_text, cur_text_str);
                        d.set_text_content(rinch_core::dom::NodeId(prev), &merged);
                        d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                        self.cursor = DomCursor::new(prev, prev_len);
                        self.anchor = self.cursor;
                    }
                    if cur_text_len > 0 {
                        dispatch_ce_event(&CeEvent::TextDeleted {
                            node_id: cur.node_id,
                            offset: 0,
                            length: cur_text_len,
                        });
                    }
                }
            }
        } else {
            // No previous text node — cursor is at very start of CE.
            // Handle heading/li/blockquote conversion.
            let cur_block = {
                let d = self.doc.borrow();
                RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id)
            };
            if let Some((cur_block_id, cur_block_parent)) = cur_block {
                let (cur_tag, parent_tag) = {
                    let d = self.doc.borrow();
                    let ct = d
                        .tree
                        .get(cur_block_id)
                        .and_then(|n| n.tag())
                        .unwrap_or("")
                        .to_string();
                    let pt = d
                        .tree
                        .get(cur_block_parent)
                        .and_then(|n| n.tag())
                        .unwrap_or("")
                        .to_string();
                    (ct, pt)
                };

                if cur_tag == "li" && RinchApp::is_list_tag(&parent_tag) {
                    let new_el = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::outdent_li(&mut d, cur_block_id, cur_block_parent, ce_node_id)
                    };
                    self.cursor = {
                        let d = self.doc.borrow();
                        RinchApp::first_text_cursor(&d.tree, new_el.0)
                            .unwrap_or(DomCursor::new(new_el.0, 0))
                    };
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::ListItemOutdented {
                        old_li_id: cur_block_id,
                        new_block_id: new_el.0,
                    });
                } else if let Some((li_id, list_id)) = {
                    let d = self.doc.borrow();
                    RinchApp::find_li_ancestor_for_outdent(&d.tree, cur_block_id, ce_node_id)
                } {
                    let new_el = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::outdent_li(&mut d, li_id, list_id, ce_node_id)
                    };
                    self.cursor = {
                        let d = self.doc.borrow();
                        RinchApp::first_text_cursor(&d.tree, new_el.0)
                            .unwrap_or(DomCursor::new(new_el.0, 0))
                    };
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::ListItemOutdented {
                        old_li_id: li_id,
                        new_block_id: new_el.0,
                    });
                } else if RinchApp::is_heading(&cur_tag) || cur_tag == "blockquote" {
                    let new_el = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::convert_block_tag(&mut d, cur_block_id, "p")
                    };
                    self.cursor = {
                        let d = self.doc.borrow();
                        RinchApp::first_text_cursor(&d.tree, new_el.0)
                            .unwrap_or(DomCursor::new(new_el.0, 0))
                    };
                    self.anchor = self.cursor;
                    dispatch_ce_event(&CeEvent::BlockTypeChanged {
                        old_node_id: cur_block_id,
                        new_node_id: new_el.0,
                        old_tag: cur_tag.clone(),
                        new_tag: "p".to_string(),
                    });
                }
            }
        }
    }

    fn delete_forward(&mut self) {
        if self.cursor != self.anchor {
            self.delete_selection();
            return;
        }
        let cur = self.cursor;
        let ce_node_id = self.ce_node_id;

        // Check if cursor is on a <br> element
        let is_br_cursor = self
            .doc
            .borrow()
            .tree
            .get(cur.node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "br")
            .unwrap_or(false);

        if is_br_cursor {
            // Remove the <br> and move cursor to start of next text or end of prev
            let (new_cursor, br_parent_id) = {
                let d = self.doc.borrow();
                let br_parent = d
                    .tree
                    .get(cur.node_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(ce_node_id);
                let next = RinchApp::next_text_node(&d.tree, ce_node_id, cur.node_id);
                let prev = RinchApp::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                let nc: Option<DomCursor> = if let Some(next_id) = next {
                    Some(DomCursor::new(next_id, 0))
                } else if let Some(prev_id) = prev {
                    let len = d
                        .tree
                        .get(prev_id)
                        .and_then(|n| n.text_content())
                        .map(|s| s.len())
                        .unwrap_or(0);
                    Some(DomCursor::new(prev_id, len))
                } else {
                    None
                };
                (nc, br_parent)
            };
            {
                let mut d = self.doc.borrow_mut();
                d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                if let Some(cursor) = new_cursor {
                    self.cursor = cursor;
                } else {
                    let text_id = d.create_text("");
                    d.append_child(rinch_core::dom::NodeId(ce_node_id), text_id);
                    self.cursor = DomCursor::new(text_id.0, 0);
                }
                self.anchor = self.cursor;
            }
            dispatch_ce_event(&CeEvent::NodeRemoved {
                node_id: cur.node_id,
                parent_id: br_parent_id,
            });
            return;
        }

        let is_element = {
            let d = self.doc.borrow();
            RinchApp::is_element_cursor(&d.tree, &cur)
        };

        if is_element {
            // ── Element cursor (empty block) — remove, cursor to start of next ──
            let info = {
                let d = self.doc.borrow();
                RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id).map(
                    |(cur_block_id, block_parent_id)| {
                        let siblings = &d.tree.nodes[block_parent_id].children;
                        let pos = siblings.iter().position(|&c| c == cur_block_id);
                        let next_block_id = pos.and_then(|p| siblings.get(p + 1).copied());
                        let next_cursor =
                            next_block_id.and_then(|nb| RinchApp::first_text_cursor(&d.tree, nb));
                        (cur_block_id, block_parent_id, next_cursor, next_block_id)
                    },
                )
            };
            if let Some((cur_block_id, block_parent_id, next_cursor, next_block_id)) = info {
                {
                    let mut d = self.doc.borrow_mut();
                    d.remove_node(rinch_core::dom::NodeId(cur_block_id));
                }
                if let Some(nc) = next_cursor {
                    self.cursor = nc;
                } else if let Some(nb) = next_block_id {
                    self.cursor = DomCursor::new(nb, 0);
                }
                self.anchor = self.cursor;
                dispatch_ce_event(&CeEvent::NodeRemoved {
                    node_id: cur_block_id,
                    parent_id: block_parent_id,
                });
            }
            return;
        }

        // ── Text node: delete char or merge with next ──
        let current_text = {
            let d = self.doc.borrow();
            d.tree
                .get(cur.node_id)
                .and_then(|n| n.text_content())
                .map(|s| s.to_string())
        };

        if let Some(current) = current_text {
            let off = cur.offset.min(current.len());
            if off < current.len() {
                // Delete char after cursor
                let next_char_end = current[off..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| off + i)
                    .unwrap_or(current.len());
                let delete_len = next_char_end - off;
                let mut new_text = String::with_capacity(current.len());
                new_text.push_str(&current[..off]);
                new_text.push_str(&current[next_char_end..]);
                {
                    let mut d = self.doc.borrow_mut();
                    d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                }
                dispatch_ce_event(&CeEvent::TextDeleted {
                    node_id: cur.node_id,
                    offset: off,
                    length: delete_len,
                });
            } else {
                // At end of text node — find next and merge
                let next_info = {
                    let d = self.doc.borrow();
                    RinchApp::next_text_node(&d.tree, ce_node_id, cur.node_id).map(|next| {
                        let next_is_br = d
                            .tree
                            .get(next)
                            .and_then(|n| n.tag())
                            .map(|t| t == "br")
                            .unwrap_or(false);
                        let next_is_empty_block =
                            RinchApp::is_element_cursor(&d.tree, &DomCursor::new(next, 0));
                        let cur_block =
                            RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);
                        let next_block = if next_is_br || next_is_empty_block {
                            None
                        } else {
                            RinchApp::find_block_and_parent(&d.tree, next, ce_node_id)
                        };
                        let next_parent = d
                            .tree
                            .get(next)
                            .and_then(|n| n.parent)
                            .unwrap_or(ce_node_id);
                        let next_text_len = d
                            .tree
                            .get(next)
                            .and_then(|n| n.text_content())
                            .map(|s| s.len())
                            .unwrap_or(0);
                        (
                            next,
                            next_is_br,
                            next_is_empty_block,
                            cur_block,
                            next_block,
                            next_parent,
                            next_text_len,
                        )
                    })
                };

                if let Some((
                    next,
                    next_is_br,
                    next_is_empty_block,
                    cur_block,
                    next_block,
                    next_parent,
                    next_text_len,
                )) = next_info
                {
                    let cross_block = next_block.is_some()
                        && cur_block.map(|(b, _)| b) != next_block.map(|(b, _)| b);

                    if next_is_br || next_is_empty_block {
                        {
                            let mut d = self.doc.borrow_mut();
                            d.remove_node(rinch_core::dom::NodeId(next));
                        }
                        dispatch_ce_event(&CeEvent::NodeRemoved {
                            node_id: next,
                            parent_id: next_parent,
                        });
                    } else if cross_block {
                        // Cross-block delete: merge next block into current block
                        let (next_block_id, _) = next_block.unwrap();
                        let (cur_block_id, _) = cur_block.unwrap();
                        {
                            let mut d = self.doc.borrow_mut();
                            let next_children: Vec<usize> =
                                d.tree.nodes[next_block_id].children.clone();

                            let mut first = true;
                            for &child_id in &next_children {
                                if first {
                                    first = false;
                                    let child_is_text = d
                                        .tree
                                        .get(child_id)
                                        .and_then(|n| n.text_content())
                                        .is_some();
                                    if child_is_text {
                                        let child_text = d
                                            .tree
                                            .get(child_id)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        let merged = format!("{}{}", current, child_text);
                                        d.set_text_content(
                                            rinch_core::dom::NodeId(cur.node_id),
                                            &merged,
                                        );
                                        d.remove_node(rinch_core::dom::NodeId(child_id));
                                        continue;
                                    }
                                }
                                d.remove_node(rinch_core::dom::NodeId(child_id));
                                d.append_child(
                                    rinch_core::dom::NodeId(cur_block_id),
                                    rinch_core::dom::NodeId(child_id),
                                );
                            }
                            d.remove_node(rinch_core::dom::NodeId(next_block_id));
                        }
                        dispatch_ce_event(&CeEvent::BlockJoined {
                            surviving_block_id: cur_block_id,
                            removed_block_id: next_block_id,
                            merge_offset: off,
                        });
                    } else {
                        // Same block or inline — merge text nodes
                        {
                            let mut d = self.doc.borrow_mut();
                            let next_text = d
                                .tree
                                .get(next)
                                .and_then(|n| n.text_content())
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            let merged = format!("{}{}", current, next_text);
                            d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &merged);
                            d.remove_node(rinch_core::dom::NodeId(next));
                        }
                        if next_text_len > 0 {
                            dispatch_ce_event(&CeEvent::TextDeleted {
                                node_id: next,
                                offset: 0,
                                length: next_text_len,
                            });
                        }
                    }
                }
            }
        }
    }

    fn delete_selection(&mut self) {
        if self.cursor == self.anchor {
            return;
        }
        let ce_node_id = self.ce_node_id;

        // Determine document order (start, end)
        let (start, end) = {
            let d = self.doc.borrow();
            RinchApp::order_cursors(&d.tree, ce_node_id, self.cursor, self.anchor)
        };

        if start.node_id == end.node_id {
            // Same node — simple substring removal
            let deleted_len = {
                let mut d = self.doc.borrow_mut();
                if let Some(node) = d.tree.get(start.node_id)
                    && let Some(text) = node.text_content().map(|s| s.to_string())
                {
                    let s = start.offset.min(text.len());
                    let e = end.offset.min(text.len());
                    let mut new_text = String::with_capacity(text.len() - (e - s));
                    new_text.push_str(&text[..s]);
                    new_text.push_str(&text[e..]);
                    d.set_text_content(rinch_core::dom::NodeId(start.node_id), &new_text);
                    Some((s, e - s))
                } else {
                    None
                }
            };
            self.cursor = start;
            self.anchor = start;
            if let Some((offset, length)) = deleted_len {
                dispatch_ce_event(&CeEvent::TextDeleted {
                    node_id: start.node_id,
                    offset,
                    length,
                });
            }
        } else {
            // Cross-node deletion: truncate start, remove middle, truncate end, merge
            let mut all_text = Vec::new();
            let start_is_text;
            let end_is_text;
            let start_remaining;
            let end_remaining;
            let start_block;
            let end_block;
            {
                let d = self.doc.borrow();
                RinchApp::collect_text_node_ids(&d.tree, ce_node_id, &mut all_text);
                start_is_text = d
                    .tree
                    .get(start.node_id)
                    .and_then(|n| n.text_content())
                    .is_some();
                end_is_text = d
                    .tree
                    .get(end.node_id)
                    .and_then(|n| n.text_content())
                    .is_some();
                start_remaining = if start_is_text {
                    d.tree
                        .get(start.node_id)
                        .and_then(|n| n.text_content())
                        .map(|t| t[..start.offset.min(t.len())].to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                end_remaining = if end_is_text {
                    d.tree
                        .get(end.node_id)
                        .and_then(|n| n.text_content())
                        .map(|t| t[end.offset.min(t.len())..].to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                start_block = RinchApp::find_block_and_parent(&d.tree, start.node_id, ce_node_id);
                end_block = RinchApp::find_block_and_parent(&d.tree, end.node_id, ce_node_id);
            }
            let start_pos = all_text
                .iter()
                .position(|&id| id == start.node_id)
                .unwrap_or(0);
            let end_pos = all_text
                .iter()
                .position(|&id| id == end.node_id)
                .unwrap_or(all_text.len());

            let cross_block = start_block.is_some()
                && end_block.is_some()
                && start_block.map(|(b, _)| b) != end_block.map(|(b, _)| b);

            let merged = format!("{}{}", start_remaining, end_remaining);
            let new_cursor;

            {
                let mut d = self.doc.borrow_mut();
                if start_is_text {
                    // Start is a text node — merge into it, remove middle + end
                    d.set_text_content(rinch_core::dom::NodeId(start.node_id), &merged);
                    for &mid_id in &all_text[start_pos + 1..=end_pos] {
                        d.remove_node(rinch_core::dom::NodeId(mid_id));
                    }
                    new_cursor = DomCursor::new(start.node_id, start.offset);
                } else if end_is_text {
                    // Start is element cursor, end is text — remove start + middle, truncate end
                    d.set_text_content(rinch_core::dom::NodeId(end.node_id), &end_remaining);
                    for &mid_id in &all_text[start_pos..end_pos] {
                        d.remove_node(rinch_core::dom::NodeId(mid_id));
                    }
                    new_cursor = DomCursor::new(end.node_id, 0);
                } else {
                    // Both are element cursors — remove everything between them
                    for &mid_id in &all_text[start_pos..=end_pos] {
                        d.remove_node(rinch_core::dom::NodeId(mid_id));
                    }
                    let prev_target = if start_pos > 0 {
                        let prev_id = all_text[start_pos - 1];
                        let len = d
                            .tree
                            .get(prev_id)
                            .and_then(|n| n.text_content())
                            .map(|t| t.len())
                            .unwrap_or(0);
                        Some(DomCursor::new(prev_id, len))
                    } else {
                        RinchApp::first_text_cursor(&d.tree, ce_node_id)
                    };
                    new_cursor = prev_target.unwrap_or(DomCursor::new(ce_node_id, 0));
                }
            }

            self.cursor = new_cursor;
            self.anchor = new_cursor;

            // Cross-block: merge blocks by moving remaining end-block children
            // into the start block, then removing the end block and any middle blocks.
            if cross_block {
                let (start_block_id, start_parent) = start_block.unwrap();
                let (end_block_id, _) = end_block.unwrap();

                if start_block_id != end_block_id {
                    let mut d = self.doc.borrow_mut();

                    // Move remaining children from end block to start block
                    let end_children: Vec<usize> =
                        d.tree.nodes[end_block_id].children.clone();
                    for &child_id in &end_children {
                        d.remove_node(rinch_core::dom::NodeId(child_id));
                        d.append_child(
                            rinch_core::dom::NodeId(start_block_id),
                            rinch_core::dom::NodeId(child_id),
                        );
                    }

                    // Remove end block and any blocks between start and end
                    let parent_children =
                        d.tree.nodes[start_parent].children.clone();
                    let sp = parent_children
                        .iter()
                        .position(|&c| c == start_block_id);
                    let ep =
                        parent_children.iter().position(|&c| c == end_block_id);
                    if let (Some(sp), Some(ep)) = (sp, ep) {
                        for &block_id in
                            parent_children[sp + 1..=ep].iter().rev()
                        {
                            d.remove_node(rinch_core::dom::NodeId(block_id));
                        }
                    }
                }
            }

            // Dispatch appropriate event for the cross-node deletion
            if cross_block {
                let (surviving_block_id, _) = start_block.unwrap();
                let (removed_block_id, _) = end_block.unwrap();
                dispatch_ce_event(&CeEvent::BlockJoined {
                    surviving_block_id,
                    removed_block_id,
                    merge_offset: start.offset,
                });
            } else if start_is_text {
                // Same block — report as text deletion on the surviving node
                // The deleted range starts at start.offset in the original text
                let orig_start_len = {
                    let d = self.doc.borrow();
                    d.tree
                        .get(start.node_id)
                        .and_then(|n| n.text_content())
                        .map(|s| s.len())
                        .unwrap_or(0)
                };
                let deleted_from_start = orig_start_len.saturating_sub(start.offset);
                if deleted_from_start > 0 {
                    dispatch_ce_event(&CeEvent::TextDeleted {
                        node_id: start.node_id,
                        offset: start.offset,
                        length: deleted_from_start,
                    });
                }
            }
        }
        // Clean up empty text nodes (they break IFC navigation)
        self.cleanup_empty_cursor_node_internal();
    }

    // ── Block Structure ──────────────────────────────────────────────

    fn split_block(&mut self) {
        if self.cursor != self.anchor {
            self.delete_selection();
        }
        let cur = self.cursor;
        let ce_node_id = self.ce_node_id;

        // We'll collect event info and dispatch after dropping borrow_mut.
        // Option<(original_block_id, new_block_id, split_offset)> for BlockSplit,
        // or special events for list exit.
        enum SplitEvent {
            BlockSplit {
                original_block_id: usize,
                new_block_id: usize,
                split_offset: usize,
            },
            ListItemOutdented {
                old_li_id: usize,
                new_block_id: usize,
            },
        }

        let split_event;

        {
            let mut d = self.doc.borrow_mut();

            // Check if cursor is inside a block element
            let block_info = RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);

            if let Some((block_id, block_parent_id)) = block_info {
                let block_tag = d
                    .tree
                    .get(block_id)
                    .and_then(|n| n.tag())
                    .unwrap_or("div")
                    .to_string();

                // If cursor is in a wrapper element inside an <li>,
                // redirect to the <li> for Enter behavior
                let (block_id, block_parent_id, block_tag) = if block_tag != "li" {
                    if let Some((li_id, list_id)) =
                        RinchApp::find_li_ancestor_for_outdent(&d.tree, block_id, ce_node_id)
                    {
                        (li_id, list_id, "li".to_string())
                    } else {
                        (block_id, block_parent_id, block_tag)
                    }
                } else {
                    (block_id, block_parent_id, block_tag)
                };

                // ── Enter in <li> ──
                if block_tag == "li" {
                    let is_empty_li = if RinchApp::is_element_cursor(&d.tree, &cur) {
                        true
                    } else {
                        let text = d
                            .tree
                            .get(cur.node_id)
                            .and_then(|n| n.text_content())
                            .unwrap_or("");
                        text.is_empty() && d.tree.nodes[block_id].children.len() <= 1
                    };

                    if is_empty_li
                        && RinchApp::is_list_tag(
                            d.tree
                                .get(block_parent_id)
                                .and_then(|n| n.tag())
                                .unwrap_or(""),
                        )
                    {
                        // Exit the list
                        let list_id = block_parent_id;
                        let list_tag = d
                            .tree
                            .get(list_id)
                            .and_then(|n| n.tag())
                            .unwrap_or("ul")
                            .to_string();
                        let grandparent_id = d
                            .tree
                            .get(list_id)
                            .and_then(|n| n.parent)
                            .unwrap_or(ce_node_id);

                        let siblings = d.tree.nodes[list_id].children.clone();
                        let li_pos = siblings.iter().position(|&c| c == block_id).unwrap_or(0);
                        let after_siblings: Vec<usize> = siblings[li_pos + 1..].to_vec();

                        let new_div = d.create_element("div");
                        let line_h = RinchApp::line_height_px(&d.tree, block_id);
                        d.set_style(new_div, "min-height", &format!("{:.1}px", line_h));

                        d.remove_node(rinch_core::dom::NodeId(block_id));

                        // Insert <div> after the list in grandparent
                        let list_next_sib = {
                            let gp_children = &d.tree.nodes[grandparent_id].children;
                            let lpos = gp_children.iter().position(|&c| c == list_id);
                            lpos.and_then(|p| gp_children.get(p + 1).copied())
                        };
                        if let Some(next) = list_next_sib {
                            d.insert_before(
                                rinch_core::dom::NodeId(grandparent_id),
                                new_div,
                                rinch_core::dom::NodeId(next),
                            );
                        } else {
                            d.append_child(rinch_core::dom::NodeId(grandparent_id), new_div);
                        }

                        // If there are siblings after, move them to a new list after <div>
                        if !after_siblings.is_empty() {
                            let new_list = d.create_element(&list_tag);
                            for &sib_id in &after_siblings {
                                d.remove_node(rinch_core::dom::NodeId(sib_id));
                                d.append_child(new_list, rinch_core::dom::NodeId(sib_id));
                            }
                            let div_next = {
                                let gp_children = &d.tree.nodes[grandparent_id].children;
                                let dpos = gp_children.iter().position(|&c| c == new_div.0);
                                dpos.and_then(|p| gp_children.get(p + 1).copied())
                            };
                            if let Some(next) = div_next {
                                d.insert_before(
                                    rinch_core::dom::NodeId(grandparent_id),
                                    new_list,
                                    rinch_core::dom::NodeId(next),
                                );
                            } else {
                                d.append_child(rinch_core::dom::NodeId(grandparent_id), new_list);
                            }
                        }

                        if d.tree.nodes[list_id].children.is_empty() {
                            d.remove_node(rinch_core::dom::NodeId(list_id));
                        }

                        self.cursor = DomCursor::new(new_div.0, 0);
                        self.anchor = self.cursor;
                        split_event = SplitEvent::ListItemOutdented {
                            old_li_id: block_id,
                            new_block_id: new_div.0,
                        };
                    } else {
                        // Non-empty li — split into new li
                        let cur_text = if RinchApp::is_element_cursor(&d.tree, &cur) {
                            String::new()
                        } else {
                            d.tree
                                .get(cur.node_id)
                                .and_then(|n| n.text_content())
                                .map(|s| s.to_string())
                                .unwrap_or_default()
                        };
                        let off = cur.offset.min(cur_text.len());
                        let after = &cur_text[off..];

                        let new_block_id = d.create_element("li");
                        if after.is_empty() {
                            let line_h = RinchApp::line_height_px(&d.tree, block_id);
                            d.set_style(new_block_id, "min-height", &format!("{:.1}px", line_h));
                        } else {
                            let new_text_id = d.create_text(after);
                            d.append_child(new_block_id, new_text_id);
                            if off == 0 {
                                d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                if d.tree.nodes[block_id].children.is_empty() {
                                    let line_h = RinchApp::line_height_px(&d.tree, block_id);
                                    d.set_style(
                                        rinch_core::dom::NodeId(block_id),
                                        "min-height",
                                        &format!("{:.1}px", line_h),
                                    );
                                }
                            } else {
                                d.set_text_content(
                                    rinch_core::dom::NodeId(cur.node_id),
                                    &cur_text[..off],
                                );
                            }
                        }

                        let next_sib = d.tree.nodes[block_parent_id]
                            .children
                            .iter()
                            .position(|&c| c == block_id)
                            .and_then(|pos| {
                                d.tree.nodes[block_parent_id].children.get(pos + 1).copied()
                            });
                        if let Some(next) = next_sib {
                            d.insert_before(
                                rinch_core::dom::NodeId(block_parent_id),
                                new_block_id,
                                rinch_core::dom::NodeId(next),
                            );
                        } else {
                            d.append_child(rinch_core::dom::NodeId(block_parent_id), new_block_id);
                        }

                        self.cursor = RinchApp::first_text_cursor(&d.tree, new_block_id.0)
                            .unwrap_or(DomCursor::new(new_block_id.0, 0));
                        self.anchor = self.cursor;
                        split_event = SplitEvent::BlockSplit {
                            original_block_id: block_id,
                            new_block_id: new_block_id.0,
                            split_offset: off,
                        };
                    }
                } else {
                    // Non-li block: heading at end → p, else preserve tag (including heading mid-split)
                    let at_end = {
                        let text_len = if RinchApp::is_element_cursor(&d.tree, &cur) {
                            0
                        } else {
                            d.tree
                                .get(cur.node_id)
                                .and_then(|n| n.text_content())
                                .map(|s| s.len())
                                .unwrap_or(0)
                        };
                        let off = cur.offset.min(text_len);
                        off >= text_len
                    };
                    let new_tag = if RinchApp::is_heading(&block_tag) && at_end {
                        "p"
                    } else {
                        &block_tag
                    };

                    let cur_text = if RinchApp::is_element_cursor(&d.tree, &cur) {
                        String::new()
                    } else {
                        d.tree
                            .get(cur.node_id)
                            .and_then(|n| n.text_content())
                            .map(|s| s.to_string())
                            .unwrap_or_default()
                    };
                    let off = cur.offset.min(cur_text.len());
                    let after = cur_text[off..].to_string();

                    let new_block_id = d.create_element(new_tag);

                    if RinchApp::is_element_cursor(&d.tree, &cur) {
                        // Element cursor — create empty new block
                        let line_h = RinchApp::line_height_px(&d.tree, block_id);
                        d.set_style(
                            new_block_id,
                            "min-height",
                            &format!("{:.1}px", line_h),
                        );
                    } else {
                        // Text cursor — split with inline-ancestor awareness.
                        // Truncate original text to portion before cursor.
                        d.set_text_content(
                            rinch_core::dom::NodeId(cur.node_id),
                            &cur_text[..off],
                        );
                        let after_text_id = if !after.is_empty() {
                            Some(d.create_text(&after))
                        } else {
                            None
                        };

                        // Walk up from cursor text node to block element,
                        // cloning inline ancestors and moving post-cursor
                        // siblings into the clones.
                        let mut current_after = after_text_id;
                        let mut child = cur.node_id;
                        loop {
                            let parent_id = d
                                .tree
                                .get(child)
                                .and_then(|n| n.parent)
                                .unwrap_or(block_id);
                            if parent_id == block_id {
                                break;
                            }

                            // Parent is an inline element — clone it
                            let parent_tag = d
                                .tree
                                .get(parent_id)
                                .and_then(|n| n.tag())
                                .unwrap_or("span")
                                .to_string();
                            let clone_id = d.create_element(&parent_tag);

                            // Copy style and class attributes
                            if let Some(style) = d
                                .tree
                                .get(parent_id)
                                .and_then(|n| n.attributes.get("style"))
                                .map(|s| s.to_string())
                            {
                                d.set_attribute(clone_id, "style", &style);
                            }
                            if let Some(class) = d
                                .tree
                                .get(parent_id)
                                .and_then(|n| n.attributes.get("class"))
                                .map(|s| s.to_string())
                            {
                                d.set_attribute(clone_id, "class", &class);
                            }

                            // Move siblings after `child` from parent into clone
                            let siblings_after: Vec<usize> = {
                                let children = &d.tree.nodes[parent_id].children;
                                let pos = children
                                    .iter()
                                    .position(|&c| c == child)
                                    .unwrap_or(0);
                                children[pos + 1..].to_vec()
                            };
                            if let Some(after_node) = current_after {
                                d.append_child(clone_id, after_node);
                            }
                            for &sib_id in &siblings_after {
                                d.remove_node(rinch_core::dom::NodeId(sib_id));
                                d.append_child(
                                    clone_id,
                                    rinch_core::dom::NodeId(sib_id),
                                );
                            }

                            current_after =
                                if d.tree.nodes[clone_id.0].children.is_empty() {
                                    None
                                } else {
                                    Some(clone_id)
                                };
                            child = parent_id;
                        }

                        // `child` is now a direct child of block_id.
                        // Add cloned inline content to new block.
                        if let Some(after_node) = current_after {
                            d.append_child(new_block_id, after_node);
                        }

                        // Move block-level siblings after `child` to new block
                        let block_siblings_after: Vec<usize> = {
                            let children = &d.tree.nodes[block_id].children;
                            let pos = children
                                .iter()
                                .position(|&c| c == child)
                                .unwrap_or(0);
                            children[pos + 1..].to_vec()
                        };
                        for &sib_id in &block_siblings_after {
                            d.remove_node(rinch_core::dom::NodeId(sib_id));
                            d.append_child(
                                new_block_id,
                                rinch_core::dom::NodeId(sib_id),
                            );
                        }

                        // Clean up: if off == 0, the original text node is now
                        // empty. Remove it and any empty inline ancestors.
                        if off == 0 {
                            let mut cleanup = cur.node_id;
                            loop {
                                let parent_id = d
                                    .tree
                                    .get(cleanup)
                                    .and_then(|n| n.parent)
                                    .unwrap_or(block_id);
                                d.remove_node(rinch_core::dom::NodeId(cleanup));
                                if parent_id == block_id {
                                    break;
                                }
                                if d.tree.nodes[parent_id].children.is_empty() {
                                    cleanup = parent_id;
                                } else {
                                    break;
                                }
                            }
                        }

                        // Set min-height on empty blocks
                        if d.tree.nodes[block_id].children.is_empty() {
                            let line_h = RinchApp::line_height_px(&d.tree, block_id);
                            d.set_style(
                                rinch_core::dom::NodeId(block_id),
                                "min-height",
                                &format!("{:.1}px", line_h),
                            );
                        }
                        if d.tree.nodes[new_block_id.0].children.is_empty() {
                            let line_h = RinchApp::line_height_px(&d.tree, block_id);
                            d.set_style(
                                new_block_id,
                                "min-height",
                                &format!("{:.1}px", line_h),
                            );
                        }
                    }

                    // Insert new block after current block
                    let next_sib = d.tree.nodes[block_parent_id]
                        .children
                        .iter()
                        .position(|&c| c == block_id)
                        .and_then(|pos| {
                            d.tree.nodes[block_parent_id].children.get(pos + 1).copied()
                        });
                    if let Some(next) = next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(block_parent_id),
                            new_block_id,
                            rinch_core::dom::NodeId(next),
                        );
                    } else {
                        d.append_child(rinch_core::dom::NodeId(block_parent_id), new_block_id);
                    }

                    self.cursor = RinchApp::first_text_cursor(&d.tree, new_block_id.0)
                        .unwrap_or(DomCursor::new(new_block_id.0, 0));
                    self.anchor = self.cursor;
                    split_event = SplitEvent::BlockSplit {
                        original_block_id: block_id,
                        new_block_id: new_block_id.0,
                        split_offset: off,
                    };
                }
            } else {
                // Inline-only CE — insert <br> at CE root level,
                // splitting any inline ancestors along the way.
                let is_br = d
                    .tree
                    .get(cur.node_id)
                    .and_then(|n| n.tag())
                    .map(|t| t == "br")
                    .unwrap_or(false);

                if is_br {
                    let parent_id = d
                        .tree
                        .get(cur.node_id)
                        .and_then(|n| n.parent)
                        .unwrap_or(ce_node_id);
                    let new_br = d.create_element("br");
                    d.insert_before(
                        rinch_core::dom::NodeId(parent_id),
                        new_br,
                        rinch_core::dom::NodeId(cur.node_id),
                    );
                    // Cursor stays on the same <br> — visually moves down
                    self.cursor = cur;
                    self.anchor = self.cursor;
                    // For inline <br> split, use the direct child of CE root as the "block"
                    split_event = SplitEvent::BlockSplit {
                        original_block_id: cur.node_id,
                        new_block_id: new_br.0,
                        split_offset: 0,
                    };
                } else {
                    let cur_text = d
                        .tree
                        .get(cur.node_id)
                        .and_then(|n| n.text_content())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let off = cur.offset.min(cur_text.len());

                    // Split text node at cursor
                    let after = cur_text[off..].to_string();
                    d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &cur_text[..off]);

                    let after_text_id = d.create_text(&after);

                    // Walk up from cursor.node_id to the direct child of CE root,
                    // cloning inline ancestors and moving post-cursor content.
                    let mut current_after = after_text_id;
                    let mut child = cur.node_id;
                    loop {
                        let parent_id = d
                            .tree
                            .get(child)
                            .and_then(|n| n.parent)
                            .unwrap_or(ce_node_id);
                        if parent_id == ce_node_id {
                            break; // child is direct child of CE root
                        }

                        // Parent is an inline element — clone it
                        let parent_tag = d
                            .tree
                            .get(parent_id)
                            .and_then(|n| n.tag())
                            .unwrap_or("span")
                            .to_string();
                        let clone_id = d.create_element(&parent_tag);

                        // Copy style and class attributes
                        if let Some(style) = d
                            .tree
                            .get(parent_id)
                            .and_then(|n| n.attributes.get("style"))
                            .map(|s| s.to_string())
                        {
                            d.set_attribute(clone_id, "style", &style);
                        }
                        if let Some(class) = d
                            .tree
                            .get(parent_id)
                            .and_then(|n| n.attributes.get("class"))
                            .map(|s| s.to_string())
                        {
                            d.set_attribute(clone_id, "class", &class);
                        }

                        // Move siblings after `child` from parent into clone
                        let siblings_after: Vec<usize> = {
                            let children = &d.tree.nodes[parent_id].children;
                            let pos = children.iter().position(|&c| c == child).unwrap_or(0);
                            children[pos + 1..].to_vec()
                        };
                        d.append_child(clone_id, current_after);
                        for &sib_id in &siblings_after {
                            d.remove_node(rinch_core::dom::NodeId(sib_id));
                            d.append_child(clone_id, rinch_core::dom::NodeId(sib_id));
                        }

                        current_after = clone_id;
                        child = parent_id;
                    }

                    // Now `child` is a direct child of CE root.
                    // Insert <br> after `child`, then `current_after` after <br>.
                    let br_id = d.create_element("br");
                    let next_sib = d.tree.nodes[ce_node_id]
                        .children
                        .iter()
                        .position(|&c| c == child)
                        .and_then(|pos| d.tree.nodes[ce_node_id].children.get(pos + 1).copied());
                    if let Some(next) = next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(ce_node_id),
                            current_after,
                            rinch_core::dom::NodeId(next),
                        );
                        d.insert_before(rinch_core::dom::NodeId(ce_node_id), br_id, current_after);
                    } else {
                        d.append_child(rinch_core::dom::NodeId(ce_node_id), br_id);
                        d.append_child(rinch_core::dom::NodeId(ce_node_id), current_after);
                    }

                    self.cursor = DomCursor::new(after_text_id.0, 0);
                    self.anchor = self.cursor;
                    // For inline text split, use the direct child as the "block"
                    split_event = SplitEvent::BlockSplit {
                        original_block_id: child,
                        new_block_id: current_after.0,
                        split_offset: off,
                    };
                }
            }
        } // borrow_mut dropped here

        match split_event {
            SplitEvent::BlockSplit {
                original_block_id,
                new_block_id,
                split_offset,
            } => {
                dispatch_ce_event(&CeEvent::BlockSplit {
                    original_block_id,
                    new_block_id,
                    split_offset,
                });
            }
            SplitEvent::ListItemOutdented {
                old_li_id,
                new_block_id,
            } => {
                dispatch_ce_event(&CeEvent::ListItemOutdented {
                    old_li_id,
                    new_block_id,
                });
            }
        }
    }

    fn set_block_type(&mut self, tag: &str) {
        let ce_node_id = self.ce_node_id;

        // ── Check for multi-block selection ──
        // First check same-parent multi-block, then cross-parent.
        enum MultiBlockKind {
            SameParent(Vec<usize>, usize),
            CrossParent(usize, usize),
        }

        let multi_kind: Option<MultiBlockKind> = {
            let d = self.doc.borrow();
            let anchor_block =
                RinchApp::find_block_and_parent(&d.tree, self.anchor.node_id, ce_node_id);
            let cursor_block =
                RinchApp::find_block_and_parent(&d.tree, self.cursor.node_id, ce_node_id);
            match (anchor_block, cursor_block) {
                (Some((ab, ap)), Some((cb, cp))) if ab != cb && ap == cp => {
                    // Same parent — existing path
                    let children = &d.tree.nodes[ap].children;
                    let a_pos = children.iter().position(|&c| c == ab);
                    let c_pos = children.iter().position(|&c| c == cb);
                    if let (Some(a), Some(c)) = (a_pos, c_pos) {
                        let (start, end) = if a <= c { (a, c) } else { (c, a) };
                        let ids: Vec<usize> = children[start..=end].to_vec();
                        if ids.len() > 1 {
                            Some(MultiBlockKind::SameParent(ids, ap))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                (Some((ab, ap)), Some((cb, cp))) if ab != cb && ap != cp => {
                    // Different parents — cross-parent path
                    Some(MultiBlockKind::CrossParent(ab, cb))
                }
                _ => None,
            }
        };

        match multi_kind {
            Some(MultiBlockKind::SameParent(block_ids, common_parent)) => {
                self.set_block_type_multi(&block_ids, common_parent, tag);
                return;
            }
            Some(MultiBlockKind::CrossParent(ab, cb)) => {
                self.set_block_type_cross_parent(ab, cb, tag);
                return;
            }
            None => {}
        }

        // ── Single-block path ──
        let cur = self.cursor;
        let block_info = {
            let d = self.doc.borrow();
            RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id)
        };
        let Some((block_id, block_parent_id)) = block_info else {
            return;
        };

        let (old_tag, parent_tag) = {
            let d = self.doc.borrow();
            let old_tag = d
                .tree
                .get(block_id)
                .and_then(|n| n.tag())
                .unwrap_or("")
                .to_string();
            let parent_tag = d
                .tree
                .get(block_parent_id)
                .and_then(|n| n.tag())
                .unwrap_or("")
                .to_string();
            (old_tag, parent_tag)
        };

        let new_node_id;

        if RinchApp::is_list_tag(tag) {
            // ── Target is a list (ul/ol) ──
            if old_tag == "li" && RinchApp::is_list_tag(&parent_tag) {
                if parent_tag == tag {
                    // Already in same list type → toggle off: extract from list as <p>
                    let extracted = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::outdent_li(&mut d, block_id, block_parent_id, ce_node_id)
                    };
                    // outdent_li converts to <div> at top level; convert to <p>
                    new_node_id = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::convert_block_tag(&mut d, extracted.0, "p")
                    };
                } else {
                    // Different list type → change the list container tag
                    new_node_id = {
                        let mut d = self.doc.borrow_mut();
                        RinchApp::convert_block_tag(&mut d, block_parent_id, tag)
                    };
                }
            } else {
                // Not in a list → convert block to <li>, wrap in new list
                let mut d = self.doc.borrow_mut();
                let li = RinchApp::convert_block_tag(&mut d, block_id, "li");
                let list = d.create_element(tag);
                let li_parent = d
                    .tree
                    .get(li.0)
                    .and_then(|n| n.parent)
                    .unwrap_or(ce_node_id);
                let next_sib = {
                    let siblings = &d.tree.nodes[li_parent].children;
                    let pos = siblings.iter().position(|&c| c == li.0);
                    pos.and_then(|p| siblings.get(p + 1).copied())
                };
                if let Some(next) = next_sib {
                    d.insert_before(
                        rinch_core::dom::NodeId(li_parent),
                        list,
                        rinch_core::dom::NodeId(next),
                    );
                } else {
                    d.append_child(rinch_core::dom::NodeId(li_parent), list);
                }
                d.remove_node(li);
                d.append_child(list, li);

                // Merge with adjacent lists of the same type
                merge_adjacent_lists(&mut d, list.0, tag, li_parent);

                new_node_id = li;
            }
        } else if tag == "blockquote" {
            // ── Target is blockquote ──
            if parent_tag == "blockquote" {
                // Already inside a blockquote → unwrap, splitting BQ if needed
                let mut d = self.doc.borrow_mut();
                let bq_id = block_parent_id;
                let bq_parent = d
                    .tree
                    .get(bq_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(ce_node_id);

                // Find position in blockquote's children
                let bq_children = d.tree.nodes[bq_id].children.clone();
                let pos = bq_children
                    .iter()
                    .position(|&c| c == block_id)
                    .unwrap_or(0);
                let after_items: Vec<usize> = bq_children[pos + 1..].to_vec();
                let has_before = pos > 0;

                let bq_next_sib = next_sibling(&d.tree, bq_parent, bq_id);

                // Extract the block (insert after BQ)
                d.remove_node(rinch_core::dom::NodeId(block_id));
                if let Some(next) = bq_next_sib {
                    d.insert_before(
                        rinch_core::dom::NodeId(bq_parent),
                        rinch_core::dom::NodeId(block_id),
                        rinch_core::dom::NodeId(next),
                    );
                } else {
                    d.append_child(
                        rinch_core::dom::NodeId(bq_parent),
                        rinch_core::dom::NodeId(block_id),
                    );
                }

                // Move items after selection to a new blockquote
                if !after_items.is_empty() {
                    let new_bq = d.create_element("blockquote");
                    for &item_id in &after_items {
                        d.remove_node(rinch_core::dom::NodeId(item_id));
                        d.append_child(new_bq, rinch_core::dom::NodeId(item_id));
                    }
                    if let Some(next) = bq_next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(bq_parent),
                            new_bq,
                            rinch_core::dom::NodeId(next),
                        );
                    } else {
                        d.append_child(rinch_core::dom::NodeId(bq_parent), new_bq);
                    }
                }

                // Remove original blockquote if empty
                if !has_before && after_items.is_empty() {
                    d.remove_node(rinch_core::dom::NodeId(bq_id));
                }
                new_node_id = rinch_core::dom::NodeId(block_id);
            } else if old_tag == "blockquote" {
                // Block IS a blockquote → convert to <p>
                new_node_id = {
                    let mut d = self.doc.borrow_mut();
                    RinchApp::convert_block_tag(&mut d, block_id, "p")
                };
            } else {
                // Not in blockquote → wrap current block in <blockquote>
                let mut d = self.doc.borrow_mut();
                let bq = d.create_element("blockquote");
                let next_sib = {
                    let siblings = &d.tree.nodes[block_parent_id].children;
                    let pos = siblings.iter().position(|&c| c == block_id);
                    pos.and_then(|p| siblings.get(p + 1).copied())
                };
                if let Some(next) = next_sib {
                    d.insert_before(
                        rinch_core::dom::NodeId(block_parent_id),
                        bq,
                        rinch_core::dom::NodeId(next),
                    );
                } else {
                    d.append_child(rinch_core::dom::NodeId(block_parent_id), bq);
                }
                d.remove_node(rinch_core::dom::NodeId(block_id));
                d.append_child(bq, rinch_core::dom::NodeId(block_id));
                new_node_id = rinch_core::dom::NodeId(block_id);
            }
        } else {
            // ── Simple tag change (h1, h2, h3, p, div, etc.) ──
            if old_tag == "li" && RinchApp::is_list_tag(&parent_tag) {
                // Currently in a list → extract from list, convert to target
                let extracted = {
                    let mut d = self.doc.borrow_mut();
                    RinchApp::outdent_li(&mut d, block_id, block_parent_id, ce_node_id)
                };
                // outdent_li converts to <div>; convert to target tag
                new_node_id = {
                    let mut d = self.doc.borrow_mut();
                    RinchApp::convert_block_tag(&mut d, extracted.0, tag)
                };
            } else if old_tag == tag {
                // Already the target type → toggle back to <p>
                new_node_id = {
                    let mut d = self.doc.borrow_mut();
                    RinchApp::convert_block_tag(&mut d, block_id, "p")
                };
            } else {
                // Convert to target tag
                new_node_id = {
                    let mut d = self.doc.borrow_mut();
                    RinchApp::convert_block_tag(&mut d, block_id, tag)
                };
            }
        }

        dispatch_ce_event(&CeEvent::BlockTypeChanged {
            old_node_id: block_id,
            new_node_id: new_node_id.0,
            old_tag,
            new_tag: tag.to_string(),
        });
    }


    // ── Inline Formatting ────────────────────────────────────────────

    fn wrap_selection(&mut self, tag: &str) {
        if self.cursor == self.anchor {
            return;
        }
        let (start, end) = order_cursors(
            &self.doc.borrow().tree,
            self.ce_node_id,
            self.anchor,
            self.cursor,
        );
        let mut d = self.doc.borrow_mut();

        // ── Split boundary text nodes so only the selected portion is wrapped ──
        // We may need to split the start and/or end text nodes at their offsets.
        // After splitting, `real_start` and `real_end` point to the text nodes
        // that should be fully wrapped.

        let mut real_start_nid = start.node_id;
        let mut real_end_nid = end.node_id;

        // Split start text node if selection starts mid-text
        if start.offset > 0 {
            if let Some(text) = d.tree.get(start.node_id).and_then(|n| n.text_content()).map(|s| s.to_string()) {
                let off = start.offset.min(text.len());
                if off < text.len() {
                    // Split: keep [0..off] in original, create new node for [off..]
                    let parent_id = d.tree.get(start.node_id).and_then(|n| n.parent).unwrap_or(self.ce_node_id);
                    d.set_text_content(rinch_core::dom::NodeId(start.node_id), &text[..off]);
                    let selected_part = d.create_text(&text[off..]);
                    let next = next_sibling(&d.tree, parent_id, start.node_id);
                    if let Some(next_id) = next {
                        d.insert_before(rinch_core::dom::NodeId(parent_id), selected_part, rinch_core::dom::NodeId(next_id));
                    } else {
                        d.append_child(rinch_core::dom::NodeId(parent_id), selected_part);
                    }
                    real_start_nid = selected_part.0;
                    // If start and end are the same node, update end to point to new node
                    if start.node_id == end.node_id {
                        real_end_nid = selected_part.0;
                    }
                }
            }
        }

        // Split end text node if selection ends mid-text
        if let Some(text) = d.tree.get(real_end_nid).and_then(|n| n.text_content()).map(|s| s.to_string()) {
            // Compute effective end offset within this (possibly split) node
            let eff_end_off = if start.node_id == end.node_id && start.offset > 0 {
                // The node was split above at start.offset, so adjust end offset
                end.offset.saturating_sub(start.offset)
            } else {
                end.offset
            };
            let off = eff_end_off.min(text.len());
            if off > 0 && off < text.len() {
                // Split: keep [0..off] to wrap, create new node for [off..] after
                let parent_id = d.tree.get(real_end_nid).and_then(|n| n.parent).unwrap_or(self.ce_node_id);
                d.set_text_content(rinch_core::dom::NodeId(real_end_nid), &text[..off]);
                let after_part = d.create_text(&text[off..]);
                let next = next_sibling(&d.tree, parent_id, real_end_nid);
                if let Some(next_id) = next {
                    d.insert_before(rinch_core::dom::NodeId(parent_id), after_part, rinch_core::dom::NodeId(next_id));
                } else {
                    d.append_child(rinch_core::dom::NodeId(parent_id), after_part);
                }
            }
        }

        // Now collect text nodes in the (possibly adjusted) range and wrap them
        let selected_ids = text_nodes_in_range(&d.tree, self.ce_node_id, DomCursor::new(real_start_nid, 0), DomCursor::new(real_end_nid, 0));
        let mut wrapped_ids = Vec::new();
        let mut last_wrapper = 0;

        for &nid in &selected_ids {
            if find_formatting_ancestor(&d.tree, nid, tag, self.ce_node_id).is_none()
                && d.tree.get(nid).is_some()
            {
                let parent_id = d
                    .tree
                    .get(nid)
                    .and_then(|n| n.parent)
                    .unwrap_or(self.ce_node_id);
                let wrapper = d.create_element(tag);
                d.insert_before(
                    rinch_core::dom::NodeId(parent_id),
                    wrapper,
                    rinch_core::dom::NodeId(nid),
                );
                d.remove_node(rinch_core::dom::NodeId(nid));
                d.append_child(wrapper, rinch_core::dom::NodeId(nid));
                last_wrapper = wrapper.0;
                wrapped_ids.push(nid);
            }
        }

        // Update selection to cover the wrapped text
        if !wrapped_ids.is_empty() {
            let first_wrapped = wrapped_ids[0];
            let last_wrapped = *wrapped_ids.last().unwrap();
            let end_len = d.tree.get(last_wrapped).and_then(|n| n.text_content()).map(|s| s.len()).unwrap_or(0);
            self.anchor = DomCursor::new(first_wrapped, 0);
            self.cursor = DomCursor::new(last_wrapped, end_len);
        }

        drop(d);
        if !wrapped_ids.is_empty() {
            dispatch_ce_event(&CeEvent::SelectionWrapped {
                tag: tag.to_string(),
                wrapper_node_id: last_wrapper,
                wrapped_node_ids: wrapped_ids,
            });
        }
    }

    fn unwrap_selection(&mut self, tag: &str) {
        if self.cursor == self.anchor {
            return;
        }
        let (start, end) = order_cursors(
            &self.doc.borrow().tree,
            self.ce_node_id,
            self.anchor,
            self.cursor,
        );
        let mut d = self.doc.borrow_mut();
        let selected_ids = text_nodes_in_range(&d.tree, self.ce_node_id, start, end);

        // Collect unique formatting ancestors, then unwrap each.
        let mut fmt_ids = Vec::new();
        for &nid in &selected_ids {
            if let Some(fmt_id) = find_formatting_ancestor(&d.tree, nid, tag, self.ce_node_id) {
                if !fmt_ids.contains(&fmt_id) {
                    fmt_ids.push(fmt_id);
                }
            }
        }
        for &fmt_id in &fmt_ids {
            if d.tree.get(fmt_id).is_some() {
                unwrap_element(&mut d, fmt_id);
            }
        }
        drop(d);
        if !fmt_ids.is_empty() {
            dispatch_ce_event(&CeEvent::SelectionUnwrapped {
                tag: tag.to_string(),
                unwrapped_node_ids: selected_ids,
            });
        }
    }

    fn toggle_wrap(&mut self, tag: &str) {
        let has_selection = self.cursor != self.anchor;

        if !has_selection {
            // ── Cursor-only: enter or escape formatting ──
            let mut d = self.doc.borrow_mut();
            let fmt_ancestor =
                find_formatting_ancestor(&d.tree, self.cursor.node_id, tag, self.ce_node_id);

            if let Some(fmt_id) = fmt_ancestor {
                // Already inside formatting — escape it.
                // Place cursor in a text node after the formatting element.
                let parent_id = d
                    .tree
                    .get(fmt_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(self.ce_node_id);
                let next_sib = next_sibling(&d.tree, parent_id, fmt_id);
                let escape_id = if let Some(next_id) = next_sib
                    && d.tree.get(next_id).and_then(|n| n.text_content()).is_some()
                {
                    next_id
                } else {
                    let zws = d.create_text("\u{200B}");
                    if let Some(next_id) = next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(parent_id),
                            zws,
                            rinch_core::dom::NodeId(next_id),
                        );
                    } else {
                        d.append_child(rinch_core::dom::NodeId(parent_id), zws);
                    }
                    zws.0
                };
                self.cursor = DomCursor::new(escape_id, 0);
                self.anchor = self.cursor;
            } else {
                // Not inside formatting — enter it.
                // Create a formatting wrapper with a zero-width space text node.
                let wrapper = d.create_element(tag);
                let inner = d.create_text("\u{200B}");
                d.append_child(wrapper, inner);

                if let Some(node) = d.tree.get(self.cursor.node_id)
                    && node.text_content().is_some()
                {
                    let text = node.text_content().unwrap().to_string();
                    let off = self.cursor.offset.min(text.len());
                    let parent_id = node.parent.unwrap_or(self.ce_node_id);

                    if off == 0 {
                        d.insert_before(
                            rinch_core::dom::NodeId(parent_id),
                            wrapper,
                            rinch_core::dom::NodeId(self.cursor.node_id),
                        );
                    } else if off >= text.len() {
                        let next = next_sibling(&d.tree, parent_id, self.cursor.node_id);
                        if let Some(next_id) = next {
                            d.insert_before(
                                rinch_core::dom::NodeId(parent_id),
                                wrapper,
                                rinch_core::dom::NodeId(next_id),
                            );
                        } else {
                            d.append_child(rinch_core::dom::NodeId(parent_id), wrapper);
                        }
                    } else {
                        // Split text node: keep [0..off], insert wrapper, then [off..]
                        let after_text = text[off..].to_string();
                        d.set_text_content(
                            rinch_core::dom::NodeId(self.cursor.node_id),
                            &text[..off],
                        );
                        let after = d.create_text(&after_text);
                        let next = next_sibling(&d.tree, parent_id, self.cursor.node_id);
                        if let Some(next_id) = next {
                            d.insert_before(
                                rinch_core::dom::NodeId(parent_id),
                                wrapper,
                                rinch_core::dom::NodeId(next_id),
                            );
                            d.insert_before(
                                rinch_core::dom::NodeId(parent_id),
                                after,
                                rinch_core::dom::NodeId(next_id),
                            );
                        } else {
                            d.append_child(rinch_core::dom::NodeId(parent_id), wrapper);
                            d.append_child(rinch_core::dom::NodeId(parent_id), after);
                        }
                    }
                } else {
                    // Element cursor (empty block): append wrapper inside the element
                    d.append_child(rinch_core::dom::NodeId(self.cursor.node_id), wrapper);
                    d.set_style(
                        rinch_core::dom::NodeId(self.cursor.node_id),
                        "min-height",
                        "0",
                    );
                }
                self.cursor = DomCursor::new(inner.0, 0);
                self.anchor = self.cursor;
            }
        } else {
            // ── Selection: check if all selected text is formatted ──
            let all_formatted = {
                let d = self.doc.borrow();
                let (start, end) =
                    order_cursors(&d.tree, self.ce_node_id, self.anchor, self.cursor);
                let ids = text_nodes_in_range(&d.tree, self.ce_node_id, start, end);
                !ids.is_empty()
                    && ids.iter().all(|&nid| {
                        find_formatting_ancestor(&d.tree, nid, tag, self.ce_node_id).is_some()
                    })
            };
            if all_formatted {
                self.unwrap_selection(tag);
            } else {
                self.wrap_selection(tag);
            }
        }
    }

    // ── List Operations ──────────────────────────────────────────────

    fn indent(&mut self) {
        let cur = self.cursor;
        let ce_node_id = self.ce_node_id;

        let (real_li_id, real_list_tag, prev_li, nested_list) = {
            let d = self.doc.borrow();
            let Some((li_id, list_id)) =
                RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id)
            else {
                return;
            };

            let li_tag = d.tree.get(li_id).and_then(|n| n.tag()).unwrap_or("");
            let list_tag = d
                .tree
                .get(list_id)
                .and_then(|n| n.tag())
                .unwrap_or("")
                .to_string();

            let resolved = if li_tag == "li" && RinchApp::is_list_tag(&list_tag) {
                Some((li_id, list_id, list_tag))
            } else {
                RinchApp::find_li_ancestor_for_outdent(&d.tree, li_id, ce_node_id).map(
                    |(real_li, real_list)| {
                        let tag = d
                            .tree
                            .get(real_list)
                            .and_then(|n| n.tag())
                            .unwrap_or("ul")
                            .to_string();
                        (real_li, real_list, tag)
                    },
                )
            };
            let Some((real_li_id, real_list_id, real_list_tag)) = resolved else {
                return;
            };

            let siblings = d.tree.nodes[real_list_id].children.clone();
            let pos = siblings.iter().position(|&c| c == real_li_id).unwrap_or(0);
            if pos == 0 {
                return; // Can't indent first item
            }
            let prev_li = siblings[pos - 1];

            let prev_children = d.tree.nodes[prev_li].children.clone();
            let nested_list = prev_children.last().and_then(|&last| {
                d.tree.get(last).and_then(|n| n.tag()).and_then(|t| {
                    if RinchApp::is_list_tag(t) {
                        Some(last)
                    } else {
                        None
                    }
                })
            });

            (real_li_id, real_list_tag, prev_li, nested_list)
        };

        let mut d = self.doc.borrow_mut();
        if let Some(existing_nested) = nested_list {
            // Move li into existing nested list
            d.remove_node(rinch_core::dom::NodeId(real_li_id));
            d.append_child(
                rinch_core::dom::NodeId(existing_nested),
                rinch_core::dom::NodeId(real_li_id),
            );
        } else {
            // Create new nested list, move li into it, append to prev_li
            let new_nested = d.create_element(&real_list_tag);
            d.set_attribute(new_nested, "style", "padding-left: 40px");
            d.remove_node(rinch_core::dom::NodeId(real_li_id));
            d.append_child(new_nested, rinch_core::dom::NodeId(real_li_id));
            d.append_child(rinch_core::dom::NodeId(prev_li), new_nested);
        }
        // Cursor stays in the same text node
    }

    fn outdent(&mut self) {
        let cur = self.cursor;
        let ce_node_id = self.ce_node_id;

        let (
            real_li_id,
            real_nested_list_id,
            real_nested_list_tag,
            parent_li_id,
            outer_list_id,
            after_siblings,
        ) = {
            let d = self.doc.borrow();
            let Some((li_id, nested_list_id)) =
                RinchApp::find_block_and_parent(&d.tree, cur.node_id, ce_node_id)
            else {
                return;
            };

            let li_tag = d.tree.get(li_id).and_then(|n| n.tag()).unwrap_or("");
            let nested_list_tag = d
                .tree
                .get(nested_list_id)
                .and_then(|n| n.tag())
                .unwrap_or("")
                .to_string();

            let resolved = if li_tag == "li" && RinchApp::is_list_tag(&nested_list_tag) {
                Some((li_id, nested_list_id, nested_list_tag))
            } else {
                RinchApp::find_li_ancestor_for_outdent(&d.tree, li_id, ce_node_id).map(
                    |(real_li, real_list)| {
                        let tag = d
                            .tree
                            .get(real_list)
                            .and_then(|n| n.tag())
                            .unwrap_or("ul")
                            .to_string();
                        (real_li, real_list, tag)
                    },
                )
            };
            let Some((real_li_id, real_nested_list_id, real_nested_list_tag)) = resolved else {
                return;
            };

            // Check if this list is nested inside another <li>
            let parent_li = d.tree.get(real_nested_list_id).and_then(|n| n.parent);
            let parent_li_tag = parent_li
                .and_then(|p| d.tree.get(p))
                .and_then(|n| n.tag())
                .unwrap_or("");
            if parent_li_tag != "li" {
                // Top-level list — exit the list via outdent_li
                drop(d);
                let new_el = {
                    let mut d = self.doc.borrow_mut();
                    RinchApp::outdent_li(
                        &mut d,
                        real_li_id,
                        real_nested_list_id,
                        ce_node_id,
                    )
                };
                self.cursor = {
                    let d = self.doc.borrow();
                    RinchApp::first_text_cursor(&d.tree, new_el.0)
                        .unwrap_or(DomCursor::new(new_el.0, 0))
                };
                self.anchor = self.cursor;
                return;
            }
            let parent_li_id = parent_li.unwrap();
            let outer_list_id = d
                .tree
                .get(parent_li_id)
                .and_then(|n| n.parent)
                .unwrap_or(ce_node_id);

            let nested_siblings = d.tree.nodes[real_nested_list_id].children.clone();
            let pos = nested_siblings
                .iter()
                .position(|&c| c == real_li_id)
                .unwrap_or(0);
            let after_siblings: Vec<usize> = nested_siblings[pos + 1..].to_vec();

            (
                real_li_id,
                real_nested_list_id,
                real_nested_list_tag,
                parent_li_id,
                outer_list_id,
                after_siblings,
            )
        };

        let mut d = self.doc.borrow_mut();

        // Move current <li> to after parent_li in the outer list
        d.remove_node(rinch_core::dom::NodeId(real_li_id));
        let parent_li_next = {
            let siblings = &d.tree.nodes[outer_list_id].children;
            let ppos = siblings.iter().position(|&c| c == parent_li_id);
            ppos.and_then(|p| siblings.get(p + 1).copied())
        };
        if let Some(next) = parent_li_next {
            d.insert_before(
                rinch_core::dom::NodeId(outer_list_id),
                rinch_core::dom::NodeId(real_li_id),
                rinch_core::dom::NodeId(next),
            );
        } else {
            d.append_child(
                rinch_core::dom::NodeId(outer_list_id),
                rinch_core::dom::NodeId(real_li_id),
            );
        }

        // If there are siblings after, create new nested list under current li
        if !after_siblings.is_empty() {
            let new_nested = d.create_element(&real_nested_list_tag);
            for &sib_id in &after_siblings {
                d.remove_node(rinch_core::dom::NodeId(sib_id));
                d.append_child(new_nested, rinch_core::dom::NodeId(sib_id));
            }
            d.append_child(rinch_core::dom::NodeId(real_li_id), new_nested);
        }

        // If the original nested list is now empty, remove it
        if d.tree.nodes[real_nested_list_id].children.is_empty() {
            d.remove_node(rinch_core::dom::NodeId(real_nested_list_id));
        }
        // Cursor stays in the same text node
    }

    // ── Selection ────────────────────────────────────────────────────

    fn get_selection(&self) -> CeSelection {
        if self.cursor == self.anchor {
            CeSelection::collapsed(self.cursor)
        } else {
            CeSelection::range(self.anchor, self.cursor)
        }
    }

    fn set_selection(&mut self, sel: CeSelection) {
        self.anchor = sel.anchor;
        self.cursor = sel.head;
        dispatch_ce_event(&CeEvent::SelectionChanged { selection: sel });
    }

    // ── Undo/Redo ────────────────────────────────────────────────────

    fn undo(&mut self) {
        // Undo stack is managed by app.rs (ContentEditableFocus.undo_stack)
        // or by the editor bridge (Editor.history). This dispatches the event
        // so observers can react.
        dispatch_ce_event(&CeEvent::UndoApplied);
    }

    fn redo(&mut self) {
        dispatch_ce_event(&CeEvent::RedoApplied);
    }

    // ── Event Access ─────────────────────────────────────────────────

    fn event_dispatcher(&self) -> &CeEventDispatcher {
        &self.dispatcher
    }

    fn event_dispatcher_mut(&mut self) -> &mut CeEventDispatcher {
        &mut self.dispatcher
    }
}

impl CeOps {
    /// If the cursor is on an empty text node, move it to an adjacent sibling
    /// and remove the empty node. Empty text nodes have no IfcTextRange and
    /// break IFC-based navigation (up/down).
    fn cleanup_empty_cursor_node_internal(&mut self) {
        let cur = self.cursor;
        let needs_cleanup = {
            let d = self.doc.borrow();
            d.tree
                .get(cur.node_id)
                .and_then(|n| n.text_content())
                .map(|t| t.is_empty())
                .unwrap_or(false)
        };
        if !needs_cleanup {
            return;
        }
        let mut sibling_cursor = None;
        {
            let d = self.doc.borrow();
            if let Some(pid) = d.tree.get(cur.node_id).and_then(|n| n.parent) {
                let siblings = d.tree.nodes[pid].children.clone();
                if let Some(idx) = siblings.iter().position(|&c| c == cur.node_id) {
                    // Try next sibling (e.g., a <br> on blank lines)
                    if idx + 1 < siblings.len() {
                        let next = siblings[idx + 1];
                        let next_is_br = d
                            .tree
                            .get(next)
                            .and_then(|n| n.tag())
                            .map(|t| t == "br")
                            .unwrap_or(false);
                        if next_is_br || d.tree.get(next).and_then(|n| n.text_content()).is_some() {
                            sibling_cursor = Some(DomCursor::new(next, 0));
                        }
                    }
                    // Try prev sibling
                    if sibling_cursor.is_none() && idx > 0 {
                        let prev_sib = siblings[idx - 1];
                        let prev_is_br = d
                            .tree
                            .get(prev_sib)
                            .and_then(|n| n.tag())
                            .map(|t| t == "br")
                            .unwrap_or(false);
                        if prev_is_br {
                            sibling_cursor = Some(DomCursor::new(prev_sib, 0));
                        } else if let Some(tc) = d.tree.get(prev_sib).and_then(|n| n.text_content())
                        {
                            sibling_cursor = Some(DomCursor::new(prev_sib, tc.len()));
                        }
                    }
                }
            }
        }
        if let Some(sc) = sibling_cursor {
            let mut d = self.doc.borrow_mut();
            d.remove_node(rinch_core::dom::NodeId(cur.node_id));
            self.cursor = sc;
            self.anchor = sc;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // CeOps needs a RinchDocument which requires rinch-dom test infrastructure.
    // Basic API surface tests here; integration tests in Task #20.

    #[test]
    fn get_selection_collapsed() {
        // Verify the selection API works without a real document
        let cursor = DomCursor::new(10, 5);
        let sel = CeSelection::collapsed(cursor);
        assert!(sel.is_collapsed());
        assert_eq!(sel.head, cursor);
        assert_eq!(sel.anchor, cursor);
    }

    #[test]
    fn get_selection_range() {
        let anchor = DomCursor::new(10, 0);
        let head = DomCursor::new(10, 5);
        let sel = CeSelection::range(anchor, head);
        assert!(!sel.is_collapsed());
    }
}
