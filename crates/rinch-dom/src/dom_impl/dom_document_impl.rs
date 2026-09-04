//! `DomDocument` trait implementation for `RinchDocument`.

use rinch_core::dom::{DomDocument, NodeId};

use peniko::color::{AlphaColor, Srgb};

use style::properties::{
    LonghandId, PropertyDeclaration, PropertyDeclarationBlock, PropertyDeclarationId,
};
use style::values::generics::position::GenericInset;
use style::values::specified::{LengthPercentage, NoCalcLength};

use crate::computed_style::{LengthPercentageAutoValue, PositionValue};
use crate::node::{DirtyFlags, DisplayMode, Node, NodeContext, NodeKind, TextMeasure};

use super::{RinchDocument, parse_inline_style, parse_style_string};

impl DomDocument for RinchDocument {
    fn doc_key(&self) -> u64 {
        self.doc_key
    }

    fn create_element(&mut self, tag: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let mut node = Node::element(id, tag, self.tree.guard.clone());
        // Use CSS-standard defaults based on element type:
        // Block elements (div, p, h1, etc.): flex-column (emulates block stacking)
        // Inline elements (span, a, etc.): flex-row
        let is_block = matches!(node.display_mode, DisplayMode::Block);
        let default_style = taffy::Style {
            display: taffy::Display::Flex,
            flex_direction: if is_block {
                taffy::FlexDirection::Column
            } else {
                taffy::FlexDirection::Row
            },
            flex_wrap: taffy::FlexWrap::NoWrap,
            ..Default::default()
        };
        let taffy_id = if tag == "img" {
            // Image elements use NodeContext::Image for intrinsic sizing
            let context = NodeContext::Image {
                src: String::new(),
                width: 0,
                height: 0,
            };
            self.tree
                .taffy
                .new_leaf_with_context(default_style, context)
                .unwrap()
        } else {
            self.tree.taffy.new_leaf(default_style).unwrap()
        };
        node.taffy_id = Some(taffy_id);
        self.tree.taffy_map.insert(taffy_id, id);
        self.tree.nodes.insert(node);

        // Hidden elements should not participate in layout
        if matches!(tag, "style" | "script" | "head" | "meta" | "link" | "title") {
            let _ = self.tree.taffy.set_style(
                taffy_id,
                taffy::Style {
                    display: taffy::Display::None,
                    ..Default::default()
                },
            );
        }

        NodeId(id)
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let mut node = Node::text(id, text, self.tree.guard.clone());
        let context = NodeContext::Text(TextMeasure {
            content: text.to_string(),
            font_size: 16.0, // default, will be updated from parent before layout
            font_weight: 400.0,
            font_family: String::new(),
            line_height_css: String::new(),
            node_id: id,
            color: AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255), // default black, updated from parent
            no_wrap: false,                                      // default, updated from parent
            overflow_wrap: crate::computed_style::OverflowWrapValue::default(),
            text_overflow: crate::computed_style::TextOverflowValue::default(),
            parent_overflow_hidden: false,
        });
        let taffy_id = self
            .tree
            .taffy
            .new_leaf_with_context(taffy::Style::default(), context)
            .unwrap();
        node.taffy_id = Some(taffy_id);
        self.tree.taffy_map.insert(taffy_id, id);
        self.tree.nodes.insert(node);
        NodeId(id)
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let node = Node::comment(id, text, self.tree.guard.clone());
        // Comments do NOT get Taffy nodes
        self.tree.nodes.insert(node);
        NodeId(id)
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        let p = parent.0;
        let c = child.0;
        // Invalidate old IFC if child was in one
        self.invalidate_ifc_for_node(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any (both DOM and Taffy). The Taffy side
        // must remove the node's *contribution*, not just its own id — a
        // spliced `display: contents` node's slots are its children's (#517).
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            // Remove from old taffy parent
            if let Some(old_taffy_parent) = self.tree.nodes[old_parent].taffy_id {
                self.taffy_detach_contribution(old_taffy_parent, c);
            }
        }
        self.tree.nodes[c].parent = Some(p);
        self.tree.nodes[p].children.push(c);
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            // Mark child as dirty so it gets measured during next layout pass
            let _ = self.tree.taffy.mark_dirty(child_taffy);
        }
        // Invalidate parent's IFC (structure changed)
        self.invalidate_parent_ifc(p);
        self.tree.layout_dirty = true; // Structural change needs full layout
        self.tree.ifc_dirty = true; // Tree structure changed
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

        // Mark inserted subtree as paint-dirty so the dirty region includes
        // the new nodes' layout positions after layout runs.
        self.mark_subtree_paint_dirty_ids(c);

        // Recompute styles for the inserted subtree to pick up ancestor-based selectors.
        // Suppressed during bulk DOM operations (render_block_at) to batch into one pass.
        if !self.tree.suppress_inline_restyle {
            self.recompute_node_styles_recursive(c);
        }

        // If a text node is appended to a <style> element, load its content as CSS
        self.maybe_load_style_css(p);
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        let p = parent.0;
        let c = child.0;
        // Clear IFC state on removed child
        self.clear_ifc_root_recursive(c);
        self.tree.nodes[p].children.retain(|&x| x != c);
        self.tree.nodes[c].parent = None;
        // Sync taffy: remove the child's contribution — for a spliced
        // `display: contents` child that is its children's slots, not its
        // own already-detached id (#517).
        if let Some(parent_taffy) = self.tree.nodes[p].taffy_id {
            self.taffy_detach_contribution(parent_taffy, c);
        }
        // Invalidate parent's IFC
        self.invalidate_parent_ifc(p);
        self.tree.layout_dirty = true; // Structural change needs full layout
        self.tree.ifc_dirty = true; // Tree structure changed
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        let p = parent.0;
        let c = child.0;
        let r = reference.0;
        // Invalidate old IFC
        self.invalidate_ifc_for_node(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any — the node's contribution, not just
        // its own id (#517, see `taffy_detach_contribution`)
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            if let Some(old_taffy_parent) = self.tree.nodes[old_parent].taffy_id {
                self.taffy_detach_contribution(old_taffy_parent, c);
            }
        }
        self.tree.nodes[c].parent = Some(p);
        let insert_pos = if let Some(pos) = self.tree.nodes[p].children.iter().position(|&x| x == r)
        {
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
                // Count taffy children before this position to find taffy index
                let taffy_idx = self.compute_taffy_child_index(p, pos);
                let _ = self
                    .tree
                    .taffy
                    .insert_child_at_index(parent_taffy, taffy_idx, child_taffy);
            } else {
                let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            }
        }
        self.invalidate_parent_ifc(p);
        self.tree.layout_dirty = true; // Structural change needs full layout
        self.tree.ifc_dirty = true; // Tree structure changed
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

        // Mark inserted subtree as paint-dirty so the dirty region includes
        // the new nodes' layout positions after layout runs.
        self.mark_subtree_paint_dirty_ids(c);

        // Recompute styles for the inserted subtree to pick up ancestor-based selectors
        self.recompute_node_styles_recursive(c);
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
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
        self.invalidate_ifc_for_node(old.0);
        self.clear_ifc_root_recursive(old.0);
        self.invalidate_ifc_for_node(new.0);
        self.clear_ifc_root_recursive(new.0);
        if let Some(parent_id) = self.tree.nodes[old.0].parent {
            // Remove new from its old parent if any — the node's
            // contribution, not just its own id (#517)
            if let Some(old_parent) = self.tree.nodes[new.0].parent {
                self.tree.nodes[old_parent].children.retain(|&x| x != new.0);
                if let Some(old_taffy_parent) = self.tree.nodes[old_parent].taffy_id {
                    self.taffy_detach_contribution(old_taffy_parent, new.0);
                }
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
                        let _ = self.tree.taffy.insert_child_at_index(
                            parent_taffy,
                            taffy_idx,
                            new_taffy,
                        );
                    }
                }
            }
            self.tree.nodes[new.0].parent = Some(parent_id);
            self.tree.nodes[old.0].parent = None;
            self.invalidate_parent_ifc(parent_id);
            self.tree.layout_dirty = true; // Structural change needs full layout
            self.tree.ifc_dirty = true; // Tree structure changed
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

            // Recompute styles for the new subtree to pick up ancestor-based selectors
            self.recompute_node_styles_recursive(new.0);
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        // Mark the removed subtree as paint-dirty so the dirty region
        // includes the old layout positions (e.g., borders, backgrounds).
        // Without this, the old pixels aren't cleared on repaint.
        self.mark_subtree_paint_dirty(node.0);

        self.clear_ifc_root_recursive(node.0);
        if let Some(parent_id) = self.tree.nodes[node.0].parent {
            self.tree.nodes[parent_id].children.retain(|&x| x != node.0);
            // Sync taffy
            if let (Some(parent_taffy), Some(node_taffy)) = (
                self.tree.nodes[parent_id].taffy_id,
                self.tree.nodes[node.0].taffy_id,
            ) {
                self.taffy_remove_child_safe(parent_taffy, node_taffy);
            }
            self.invalidate_parent_ifc(parent_id);
            self.tree.layout_dirty = true; // Structural change needs full layout
            self.tree.ifc_dirty = true; // Tree structure changed
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
        }
        self.tree.nodes[node.0].parent = None;
        // Don't remove from slab yet — caller may still reference it
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        let n = node.0;

        // Fast path: skip if content is already identical.
        match &self.tree.nodes[n].kind {
            NodeKind::Text(t) if t.content == text => return,
            NodeKind::Element(_) => {
                let children = &self.tree.nodes[n].children;
                if children.len() == 1 {
                    if let NodeKind::Text(t) = &self.tree.nodes[children[0]].kind {
                        if t.content == text {
                            return;
                        }
                    }
                }
            }
            _ => {}
        }

        // Invalidate IFC if this node belongs to one
        self.invalidate_ifc_for_node(n);
        // Also invalidate parent's IFC
        if let Some(parent_id) = self.tree.nodes[n].parent {
            self.invalidate_parent_ifc(parent_id);
        }
        // Track which IFC root has dirty text content so the measure callback
        // can skip expensive Parley rebuilds for unchanged roots.
        {
            let ifc_root = self.tree.nodes[n]
                .ifc_root
                .or_else(|| self.tree.nodes[n].parent);
            if let Some(root_id) = ifc_root {
                self.tree.dirty_ifc_text_roots.insert(root_id);
                // Invalidate cached measure results for this root
                self.tree
                    .ifc_measure_cache
                    .retain(|&(rid, _), _| rid != root_id);
            }
        }
        match &mut self.tree.nodes[n].kind {
            NodeKind::Text(t) => {
                t.content = text.to_string();
                // Update the Taffy NodeContext too
                if let Some(taffy_id) = self.tree.nodes[n].taffy_id {
                    if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id)
                        && let NodeContext::Text(tm) = ctx
                    {
                        tm.content = text.to_string();
                    }
                    let _ = self.tree.taffy.mark_dirty(taffy_id);
                }
            }
            _ => {
                // Clear any IFC text_layout on this element (it is the IFC root
                // for its inline children, but ifc_root points to it from children,
                // not from itself, so invalidate_ifc_for_node misses it).
                self.tree.nodes[n].text_layout = None;
                // For elements: remove all children and add a text child
                let old_children: Vec<_> = self.tree.nodes[n].children.clone();
                for child in old_children {
                    self.tree.nodes[child].parent = None;
                    // Remove each child's contribution from taffy — for a
                    // spliced `display: contents` child that is its
                    // children's slots, not its own id (#517)
                    if let Some(parent_taffy) = self.tree.nodes[n].taffy_id {
                        self.taffy_detach_contribution(parent_taffy, child);
                    }
                }
                self.tree.nodes[n].children.clear();
                // Create text child with taffy node and context
                let text_id = self.tree.nodes.vacant_key();
                let mut text_node = Node::text(text_id, text, self.tree.guard.clone());
                text_node.parent = Some(n);
                let context = NodeContext::Text(TextMeasure {
                    content: text.to_string(),
                    font_size: 16.0,
                    font_weight: 400.0,
                    font_family: String::new(),
                    line_height_css: String::new(),
                    node_id: text_id,
                    color: AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255),
                    no_wrap: false,
                    overflow_wrap: crate::computed_style::OverflowWrapValue::default(),
                    text_overflow: crate::computed_style::TextOverflowValue::default(),
                    parent_overflow_hidden: false,
                });
                let taffy_id = self
                    .tree
                    .taffy
                    .new_leaf_with_context(taffy::Style::default(), context)
                    .unwrap();
                text_node.taffy_id = Some(taffy_id);
                self.tree.taffy_map.insert(taffy_id, text_id);
                self.tree.nodes.insert(text_node);
                self.tree.nodes[n].children.push(text_id);
                // Add to taffy parent
                if let Some(parent_taffy) = self.tree.nodes[n].taffy_id {
                    let _ = self.tree.taffy.add_child(parent_taffy, taffy_id);
                }
                self.tree.ifc_dirty = true; // Structural change (children replaced)
            }
        }
        self.tree.layout_dirty = true; // Text content change affects layout
        self.push_dirty(n);

        // If this node is a <style> element, reload its CSS
        self.maybe_load_style_css(n);
        // If the parent is a <style> element (text node content changed), reload
        if let Some(parent_id) = self.tree.nodes[n].parent {
            self.maybe_load_style_css(parent_id);
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        self.tree.nodes[node.0]
            .attributes
            .insert(name.to_string(), value.to_string());

        // Parse inline style into Stylo PropertyDeclarationBlock
        if name == "style" {
            if value.trim().is_empty() {
                // Empty/blank style: clear the cache entirely so Stylo sees None
                // (not Some(empty_pdb)) and falls back to class-based styles.
                self.tree.nodes[node.0].style_attribute_cache = None;
            } else {
                self.cache_inline_style(node.0, parse_inline_style(value));
            }
        }
        // Invalidate IFC if this node belongs to one (style/class changes affect inline layout).
        // Only for nodes that actually participate in inline formatting — block elements
        // don't need IFC invalidation and the mark_dirty propagation would defeat Taffy's cache.
        if (name == "style" || name == "class")
            && (self.tree.nodes[node.0].ifc_root.is_some()
                || self.tree.nodes[node.0].is_inline()
                || self.tree.nodes[node.0].text_layout.is_some())
        {
            self.invalidate_ifc_for_node(node.0);
            // Also invalidate parent's IFC in case this is an inline child
            if let Some(parent_id) = self.tree.nodes[node.0].parent {
                self.invalidate_parent_ifc(parent_id);
            }
        }
        // Handle <img src="..."> — trigger async image load
        if name == "src" && self.tree.nodes[node.0].tag() == Some("img") {
            self.request_image_load_for_node(node.0, value);
        }

        // Any attribute can participate in a selector — `[data-state=open]`,
        // `[aria-selected]`, `[data-pm-theme=dark] h1`, attribute-based component
        // styling, etc. — so an attribute change must re-resolve styles, not only
        // for `class`/`style`. (Browsers use a per-attribute invalidation map keyed
        // on which selectors reference the attribute; rinch-dom doesn't track that
        // yet, so it conservatively restyles this node and its subtree. This is
        // cheap in practice: most `set_attribute` calls happen at element creation
        // when the node has no descendants, and post-render attribute changes are
        // rare — the per-frame hot path uses `set_style`/`set_text`, which have
        // their own paths.)
        //
        // Invalidate cached Stylo data (deferred to resolve_layout) and mark the
        // subtree so descendant/sibling selectors re-match against the new value.
        *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
        self.tree.style_roots.push(node.0);
        self.tree.styles_dirty = true;
        self.push_dirty_flags(
            node.0,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
        self.invalidate_descendant_styles(node.0);
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        self.tree.nodes[node.0].attributes.remove(name);
        if name == "style" {
            self.tree.nodes[node.0].style_attribute_cache = None;
        }
        // Invalidate IFC for style/class changes on inline-participating nodes,
        // mirroring set_attribute.
        if (name == "style" || name == "class")
            && (self.tree.nodes[node.0].ifc_root.is_some()
                || self.tree.nodes[node.0].is_inline()
                || self.tree.nodes[node.0].text_layout.is_some())
        {
            self.invalidate_ifc_for_node(node.0);
            if let Some(parent_id) = self.tree.nodes[node.0].parent {
                self.invalidate_parent_ifc(parent_id);
            }
        }
        // Symmetric with set_attribute: any attribute can participate in a
        // selector (`[data-highlighted]`, `[aria-selected]`, …), so *removing*
        // one must re-resolve this node and its subtree too — otherwise a style
        // that matched only while the attribute was present stays applied (e.g. a
        // popup option keeps its highlight background after the attribute is
        // cleared).
        *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
        self.tree.style_roots.push(node.0);
        self.tree.styles_dirty = true;
        self.push_dirty_flags(
            node.0,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
        self.invalidate_descendant_styles(node.0);
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.tree.nodes.get(node.0)?.attributes.get(name).cloned()
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        self.set_styles(node, &[(property, value)]);
    }

    fn set_styles(&mut self, node: NodeId, properties: &[(&str, &str)]) {
        // Merge into the inline style attribute and parse it once; both paths
        // below store exactly this string and this declaration block.
        let style_str = self.merged_inline_style(node.0, properties);
        let pdb = parse_inline_style(&style_str);
        let insets = self.inset_fast_path_values(node.0, properties, &pdb);

        self.tree.nodes[node.0]
            .attributes
            .insert("style".to_string(), style_str);
        self.cache_inline_style(node.0, pdb);

        match insets {
            Some(insets) => self.apply_inset_fast_path(node.0, &insets),
            None => self.invalidate_inline_style(node.0),
        }
    }

    fn mark_dirty(&mut self, node: NodeId) {
        self.push_dirty(node.0);
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
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
        // Simple selector matching: supports #id, .class, tag
        self.query_recursive(self.tree.root_id, selector)
            .map(NodeId)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        // Query all matching nodes
        let mut results = Vec::new();
        self.query_all_recursive(self.tree.root_id, selector, &mut results);
        results.into_iter().map(NodeId).collect()
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.tree
            .nodes
            .get(node.0)
            .map(|n| n.children.iter().map(|&c| NodeId(c)).collect())
            .unwrap_or_default()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        let p = parent.0;
        let c = child.0;
        // Invalidate old IFC
        self.invalidate_ifc_for_node(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any — the node's contribution, not just
        // its own id (#517, see `taffy_detach_contribution`)
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            if let Some(old_taffy_parent) = self.tree.nodes[old_parent].taffy_id {
                self.taffy_detach_contribution(old_taffy_parent, c);
            }
        }
        self.tree.nodes[c].parent = Some(p);
        let len = self.tree.nodes[p].children.len();
        let actual_index = if index >= len {
            self.tree.nodes[p].children.push(c);
            len
        } else {
            self.tree.nodes[p].children.insert(index, c);
            index
        };
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            let taffy_idx = self.compute_taffy_child_index(p, actual_index);
            let _ = self
                .tree
                .taffy
                .insert_child_at_index(parent_taffy, taffy_idx, child_taffy);
        }
        self.invalidate_parent_ifc(p);
        self.tree.layout_dirty = true; // Structural change needs full layout
        self.tree.ifc_dirty = true; // Tree structure changed
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

        // Recompute styles for the inserted subtree to pick up ancestor-based selectors
        self.recompute_node_styles_recursive(c);
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

    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64) {
        if let Some(n) = self.tree.nodes.get_mut(node.0) {
            n.scroll_offset.1 = scroll_top;
        }
        self.push_dirty_flags(node.0, DirtyFlags::PAINT);
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        use crate::html_parser::parse_html_string;

        // Clear existing children (including taffy sync)
        let old_children: Vec<_> = self.tree.nodes[node.0].children.clone();
        for child in old_children {
            self.clear_ifc_root_recursive(child);
            // Remove each child's contribution from taffy — for a spliced
            // `display: contents` child that is its children's slots, not
            // its own id (#517). Must run before `remove_subtree`, which
            // drops the slab nodes the contribution walk reads.
            if let Some(parent_taffy) = self.tree.nodes[node.0].taffy_id {
                self.taffy_detach_contribution(parent_taffy, child);
            }
            self.tree.nodes[child].parent = None;
            self.tree.remove_subtree(child);
        }
        self.tree.nodes[node.0].children.clear();
        // Clearing a subtree is a structural change: without these flags a
        // clear-to-empty `set_inner_html` (nothing re-appended below) leaves
        // `resolve_layout`'s dirty gate closed and the old geometry — the
        // removed children's sizes included — stays on screen (#517).
        self.tree.layout_dirty = true;
        self.tree.ifc_dirty = true;

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

    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)> {
        use crate::text_query::caret_position_for_offset;
        caret_position_for_offset(self, node_id, byte_offset)
    }

    fn query_selection_rects(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        let Some(node) = self.tree.nodes.get(node_id as usize) else {
            return Vec::new();
        };
        let layout = match (&node.text_layout, &node.cached_text_parley) {
            (Some(inline), _) => &inline.layout,
            (None, Some(layout)) => layout,
            (None, None) => return Vec::new(),
        };
        crate::text_query::selection_rects_for_layout(layout, byte_a, byte_b)
    }

    fn query_glyph_bounds(
        &self,
        node_id: u64,
        byte_offset: usize,
    ) -> Option<rinch_core::dom::GlyphBounds> {
        use crate::text_query::glyph_bounds_for_offset;
        let bounds = glyph_bounds_for_offset(self, node_id, byte_offset)?;
        Some(rinch_core::dom::GlyphBounds {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        })
    }

    fn focus_element(&mut self, node_id: NodeId) {
        // Request focus via the event system - the runtime will apply it.
        // Keyed by this document so another document's runtime on the same
        // thread doesn't consume it (issue #134).
        rinch_core::request_focus(self.doc_key, node_id.0);
    }

    fn resolve_layout(&mut self, width: f32, height: f32) {
        // Delegate to the existing implementation method
        RinchDocument::resolve_layout(self, width, height);
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        let node = self.tree.nodes.get(node_id as usize)?;
        Some((
            node.layout.x,
            node.layout.y,
            node.layout.width,
            node.layout.height,
        ))
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

    fn text_content(&self, node: NodeId) -> Option<String> {
        let n = self.tree.nodes.get(node.0)?;
        match &n.kind {
            NodeKind::Text(data) => Some(data.content.clone()),
            NodeKind::Element(_) => {
                let mut result = String::new();
                self.collect_text_content(node.0, &mut result);
                Some(result)
            }
            _ => None,
        }
    }

    // ── Scroll query API ─────────────────────────────────────────────────

    fn scroll_top(&self, node: NodeId) -> f64 {
        self.tree
            .nodes
            .get(node.0)
            .map(|n| n.scroll_offset.1)
            .unwrap_or(0.0)
    }

    fn scroll_left(&self, node: NodeId) -> f64 {
        self.tree
            .nodes
            .get(node.0)
            .map(|n| n.scroll_offset.0)
            .unwrap_or(0.0)
    }

    fn set_scroll_left(&mut self, node: NodeId, scroll_left: f64) {
        if let Some(n) = self.tree.nodes.get_mut(node.0) {
            n.scroll_offset.0 = scroll_left;
        }
        self.push_dirty_flags(node.0, DirtyFlags::PAINT);
    }

    fn scroll_height(&self, node: NodeId) -> f64 {
        let node = match self.tree.nodes.get(node.0) {
            Some(n) => n,
            None => return 0.0,
        };
        // Taffy child.layout.y is relative to the parent's border box,
        // so it includes padding-top + border-top. Subtract that offset
        // to get the content-relative height (consistent with client_height).
        let content_top = (node.computed_style.padding_top.to_px()
            + node.computed_style.border_top_width.to_px()) as f64;
        let mut max_bottom: f64 = 0.0;
        for &child_id in &node.children {
            if let Some(child) = self.tree.nodes.get(child_id) {
                let bottom = (child.layout.y + child.layout.height) as f64 - content_top;
                if bottom > max_bottom {
                    max_bottom = bottom;
                }
            }
        }
        max_bottom
    }

    fn scroll_width(&self, node: NodeId) -> f64 {
        let node = match self.tree.nodes.get(node.0) {
            Some(n) => n,
            None => return 0.0,
        };
        let content_left = (node.computed_style.padding_left.to_px()
            + node.computed_style.border_left_width.to_px()) as f64;
        let mut max_right: f64 = 0.0;
        for &child_id in &node.children {
            if let Some(child) = self.tree.nodes.get(child_id) {
                let right = (child.layout.x + child.layout.width) as f64 - content_left;
                if right > max_right {
                    max_right = right;
                }
            }
        }
        max_right
    }

    fn client_height(&self, node: NodeId) -> f64 {
        let node = match self.tree.nodes.get(node.0) {
            Some(n) => n,
            None => return 0.0,
        };
        let cs = &node.computed_style;
        let pad_top = cs.padding_top.to_px() as f64;
        let pad_bottom = cs.padding_bottom.to_px() as f64;
        let border_top = cs.border_top_width.to_px() as f64;
        let border_bottom = cs.border_bottom_width.to_px() as f64;
        (node.layout.height as f64 - pad_top - pad_bottom - border_top - border_bottom).max(0.0)
    }

    fn client_width(&self, node: NodeId) -> f64 {
        let node = match self.tree.nodes.get(node.0) {
            Some(n) => n,
            None => return 0.0,
        };
        let cs = &node.computed_style;
        let pad_left = cs.padding_left.to_px() as f64;
        let pad_right = cs.padding_right.to_px() as f64;
        let border_left = cs.border_left_width.to_px() as f64;
        let border_right = cs.border_right_width.to_px() as f64;
        (node.layout.width as f64 - pad_left - pad_right - border_left - border_right).max(0.0)
    }

    fn request_scroll_into_view(&mut self, node: NodeId) {
        self.tree.scroll_into_view_requests.push(node.0);
    }

    fn drain_scroll_into_view_requests(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.tree.scroll_into_view_requests)
            .into_iter()
            .map(NodeId)
            .collect()
    }

    fn drain_scroll_clamps(&mut self) -> Vec<(NodeId, f64)> {
        std::mem::take(&mut self.tree.pending_scroll_clamps)
            .into_iter()
            .map(|(id, offset)| (NodeId(id), offset))
            .collect()
    }
}

/// The four inset longhands, in the order an [`InsetBatch`] stores them.
const INSET_SIDES: [(&str, LonghandId); 4] = [
    ("left", LonghandId::Left),
    ("top", LonghandId::Top),
    ("right", LonghandId::Right),
    ("bottom", LonghandId::Bottom),
];

/// The insets one `set_styles` batch asks the fast path to write, indexed like
/// [`INSET_SIDES`]; `None` is a side the batch does not touch.
type InsetBatch = [Option<LengthPercentageAutoValue>; 4];

/// The value of inset longhand `id` in `pdb`, if the fast path can hand it to
/// Taffy without a cascade: `auto`, an absolute length (`px`, `pt`, `in`, …)
/// or a percentage — what `inset_from_stylo_generic` would make of the
/// computed value. `None` for anything that needs the cascade — `em`/`rem`
/// (font size), `vh`/`vw` (Stylo's device), `calc()`, `var()`, a CSS-wide
/// keyword, an anchor function — or a declaration Stylo rejected (a unitless
/// `10`, `10 px`, `NaNpx`), so the block holds no value for it at all.
fn plain_inset(
    pdb: &PropertyDeclarationBlock,
    id: LonghandId,
) -> Option<LengthPercentageAutoValue> {
    let (declaration, _importance) = pdb.get(PropertyDeclarationId::Longhand(id))?;
    let inset = match declaration {
        PropertyDeclaration::Left(v)
        | PropertyDeclaration::Top(v)
        | PropertyDeclaration::Right(v)
        | PropertyDeclaration::Bottom(v) => v,
        _ => return None,
    };
    match inset {
        GenericInset::Auto => Some(LengthPercentageAutoValue::Auto),
        GenericInset::LengthPercentage(LengthPercentage::Length(NoCalcLength::Absolute(len))) => {
            let px = len.to_px();
            px.is_finite()
                .then_some(LengthPercentageAutoValue::Length(px))
        }
        GenericInset::LengthPercentage(LengthPercentage::Percentage(pct)) => {
            Some(LengthPercentageAutoValue::Percent(pct.0))
        }
        _ => None,
    }
}

impl RinchDocument {
    /// The node's inline `style` attribute with `properties` merged in — a
    /// later declaration of a property replaces the earlier one **in place**,
    /// keeping every other declaration where the author wrote it.
    ///
    /// Order is load-bearing, not cosmetic (#265). `set_styles` parses this
    /// string into the declaration block Stylo cascades, so whatever order
    /// comes out here *is* the order two declarations of the same longhand —
    /// or a shorthand and one of its longhands — resolve in. This used to
    /// round-trip through a `HashMap`, which reshuffled the whole attribute on
    /// every write with a per-process random order: `set_style("left", …)` on
    /// a node whose attribute already said `inset: 0` produced `left` first
    /// (loses) or `inset` first (wins) depending on the run, so a single
    /// `assert` could pass all day and fail in CI. A `Vec` makes it a fact
    /// about the input instead of a fact about the process.
    ///
    /// **Residual divergence from a browser**, deliberately accepted here: a
    /// longhand *already in the attribute* before a shorthand that covers it
    /// still loses to that shorthand — `"left: 5px; inset: 0"` plus
    /// `set_style("left", "10px")` yields `"left: 10px; inset: 0"`, so `inset`
    /// still wins and `left` computes to `0`. CSSOM expands `inset` into its
    /// four longhands at parse time, so a browser answers `10px`. Closing that
    /// gap means keeping Stylo's `PropertyDeclarationBlock` as the source of
    /// truth (`prepare_for_update`/`update`, then `to_css` for the attribute)
    /// rather than the string — a bigger change that inverts the
    /// `merged → parse_inline_style → cache` invariant `inset_fast_path_values`
    /// rests on, and canonicalises `get_attribute("style")` output. The
    /// reported shape — shorthand first, longhand written later — is correct
    /// with the `Vec`, and *every* shape is now deterministic.
    fn merged_inline_style(&self, node_id: usize, properties: &[(&str, &str)]) -> String {
        let mut decls: Vec<(String, String)> = self.tree.nodes[node_id]
            .attributes
            .get("style")
            .map(|s| parse_style_string(s))
            .unwrap_or_default();
        for &(property, value) in properties {
            match decls.iter_mut().find(|(k, _)| k == property) {
                Some(slot) => slot.1 = value.to_string(),
                None => decls.push((property.to_string(), value.to_string())),
            }
        }
        decls
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The normal path after an inline style change: drop the cached Stylo
    /// data so the next `resolve_layout` re-cascades this node from its
    /// declaration block, and invalidate the inline formatting context it
    /// takes part in.
    fn invalidate_inline_style(&mut self, node_id: usize) {
        *self.tree.nodes[node_id].stylo_element_data.borrow_mut() = None;
        self.tree.style_roots.push(node_id);
        self.tree.styles_dirty = true;
        // Only invalidate IFC state for nodes that participate in inline formatting.
        // Block elements (like slider track) don't affect IFC layout, and the
        // mark_dirty() in invalidate_parent_ifc would propagate to root, defeating
        // Taffy's cache for ALL InlineRoot measure callbacks.
        if self.tree.nodes[node_id].ifc_root.is_some()
            || self.tree.nodes[node_id].is_inline()
            || self.tree.nodes[node_id].text_layout.is_some()
        {
            self.invalidate_ifc_for_node(node_id);
            if let Some(parent_id) = self.tree.nodes[node_id].parent {
                self.invalidate_parent_ifc(parent_id);
            }
        }
        self.push_dirty_flags(
            node_id,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
    }

    /// What the inset fast path may write for this `set_styles` batch, or
    /// `None` when the batch must take the normal path.
    ///
    /// Changing only `left`/`top`/`right`/`bottom` on a `position: absolute`
    /// element moves it within its containing block — siblings and children
    /// are unaffected — so the cascade and IFC invalidation the normal path
    /// pays are skipped and `ComputedStyle` and the Taffy inset are written
    /// directly. The values come out of `pdb`, the declaration block Stylo
    /// just parsed from the merged style attribute, never from a parser of our
    /// own: what the fast path applies is by construction what the next full
    /// cascade of that same block applies, so the two cannot disagree (#236)
    /// — not on a unitless number Stylo drops, not on `inset:` versus `left:`
    /// precedence, not on rounding.
    ///
    /// Declined — `None`, nothing touched — for:
    /// - a batch that is empty or not inset-only;
    /// - a node that is not `position: absolute`. `fixed` is excluded on
    ///   purpose: `apply_stylo_styles_to_taffy` bakes its Taffy *size* from
    ///   its insets, so an inset change is not inset-only for it — children
    ///   would be laid out against the stale size until the next restyle. An
    ///   absolute with no positioned ancestor is excluded for the same reason
    ///   — its size is baked from the initial containing block (#204);
    /// - a requested inset that is not a plain value in the block (see
    ///   [`plain_inset`]).
    ///
    /// Known limitation: a stylesheet rule with `!important` on the same inset
    /// beats the inline declaration in the cascade; the fast path does not
    /// consult the cascade and applies the inline value until the next full
    /// restyle.
    fn inset_fast_path_values(
        &self,
        node_id: usize,
        properties: &[(&str, &str)],
        pdb: &PropertyDeclarationBlock,
    ) -> Option<InsetBatch> {
        if properties.is_empty()
            || self.tree.nodes[node_id].computed_style.position != PositionValue::Absolute
            // An absolute with no positioned ancestor is excluded for exactly
            // the reason `fixed` is: its Taffy *size* is baked from its insets
            // against the initial containing block (#204), so an inset change
            // is not inset-only for it either.
            || crate::out_of_flow::out_of_flow_kind(&self.tree, node_id)
                == Some(crate::out_of_flow::OutOfFlowKind::IcbAbsolute)
        {
            return None;
        }
        let mut batch: InsetBatch = [None; 4];
        for &(property, _) in properties {
            let slot = INSET_SIDES.iter().position(|(name, _)| *name == property)?;
            batch[slot] = Some(plain_inset(pdb, INSET_SIDES[slot].1)?);
        }
        Some(batch)
    }

    /// Write `insets` to `ComputedStyle` and the Taffy inset, and let Taffy
    /// place the node on the next `resolve_layout`.
    ///
    /// The node's *position* is never computed here. `LayoutResult` is
    /// parent-border-box-relative, an inset is padding-box-relative with the
    /// margin on top, and only Taffy knows the containing block, the border
    /// and the rounding. Taffy's `set_style` clears this node's cache but not
    /// its children's, so the recompute repositions the node and reuses
    /// everything below it. Writing the position by hand is how #236 happened.
    fn apply_inset_fast_path(&mut self, node_id: usize, insets: &InsetBatch) {
        let computed = &mut self.tree.nodes[node_id].computed_style;
        let [left, top, right, bottom] = *insets;
        if let Some(v) = left {
            computed.left = v;
        }
        if let Some(v) = top {
            computed.top = v;
        }
        if let Some(v) = right {
            computed.right = v;
        }
        if let Some(v) = bottom {
            computed.bottom = v;
        }
        // The Taffy inset is `to_taffy_style`'s image of these four fields;
        // re-derive all of it rather than patching one side.
        let inset = taffy::Rect {
            left: computed.left.to_taffy(),
            top: computed.top.to_taffy(),
            right: computed.right.to_taffy(),
            bottom: computed.bottom.to_taffy(),
        };
        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id
            && let Ok(mut ts) = self.tree.taffy.style(taffy_id).cloned()
        {
            ts.inset = inset;
            let _ = self.tree.taffy.set_style(taffy_id, ts);
        }

        // Let Taffy place the node on the next resolve_layout. Unconditional:
        // a node without a Taffy id must still be re-laid out rather than
        // silently kept where it was.
        self.tree.layout_dirty = true;
        self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
        // Dirty-region paint cannot see an out-of-flow move before layout has
        // run; rebuild the whole scene.
        self.tree.full_repaint_needed = true;
    }
}
