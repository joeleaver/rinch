//! DomDocument implementation for rinch-dom.

use std::collections::HashMap;

use rinch_core::dom::{DomDocument, NodeId};

use peniko::Brush;

use crate::layout;
use crate::node::{DirtyFlags, Node, NodeKind, NodeTree, NodeContext, TextMeasure, LayoutResult, DisplayMode, InlineLayout};

/// The primary document type for rinch-dom.
///
/// Implements [`DomDocument`] using a slab-allocated node tree.
/// In later phases, this will integrate Taffy for layout,
/// Parley for text, and Vello for painting.
pub struct RinchDocument {
    /// The node tree.
    pub tree: NodeTree,
    /// Parley font context for text shaping.
    pub font_cx: parley::FontContext,
    /// Parley layout context for text measurement.
    pub layout_cx: parley::LayoutContext<[u8; 4]>,
}

impl RinchDocument {
    /// Create a new document with root and body nodes.
    pub fn new() -> Self {
        Self {
            tree: NodeTree::new(),
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
        }
    }

    /// Mark a node and its ancestors as needing layout.
    fn mark_dirty_up(&mut self, node_id: usize, flags: DirtyFlags) {
        let mut current = Some(node_id);
        while let Some(id) = current {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.dirty.insert(flags);
                current = node.parent;
            } else {
                break;
            }
        }
    }

    /// Push a node to the dirty list with layout+paint flags.
    fn push_dirty(&mut self, node_id: usize) {
        self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
    }

    /// Push a node to the dirty list with specific flags.
    fn push_dirty_flags(&mut self, node_id: usize, flags: DirtyFlags) {
        if self.tree.contains(node_id) {
            self.tree.nodes[node_id].dirty.insert(flags);
            self.tree.push_dirty(node_id);
            if flags.contains(DirtyFlags::LAYOUT) {
                self.mark_dirty_up(node_id, DirtyFlags::LAYOUT);
            }
        }
    }
}

impl DomDocument for RinchDocument {
    fn create_element(&mut self, tag: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let mut node = Node::element(id, tag);
        // Use CSS-standard defaults based on element type:
        // Block elements (div, p, h1, etc.): flex-column (emulates block stacking)
        // Inline elements (span, a, etc.): flex-row
        let is_block = matches!(node.display_mode, DisplayMode::Block);
        let taffy_id = self.tree.taffy.new_leaf(taffy::Style {
            display: taffy::Display::Flex,
            flex_direction: if is_block { taffy::FlexDirection::Column } else { taffy::FlexDirection::Row },
            flex_wrap: taffy::FlexWrap::NoWrap,
            ..Default::default()
        }).unwrap();
        node.taffy_id = Some(taffy_id);
        self.tree.taffy_map.insert(taffy_id, id);
        self.tree.nodes.insert(node);

        // Hidden elements should not participate in layout
        if matches!(tag, "style" | "script" | "head" | "meta" | "link" | "title") {
            let _ = self.tree.taffy.set_style(taffy_id, taffy::Style {
                display: taffy::Display::None,
                ..Default::default()
            });
        }

        NodeId(id)
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let mut node = Node::text(id, text);
        let context = NodeContext::Text(TextMeasure {
            content: text.to_string(),
            font_size: 16.0, // default, will be updated from parent before layout
            font_weight: 400.0,
            font_family: String::new(),
            line_height_css: String::new(),
        });
        let taffy_id = self.tree.taffy.new_leaf_with_context(taffy::Style::default(), context).unwrap();
        node.taffy_id = Some(taffy_id);
        self.tree.taffy_map.insert(taffy_id, id);
        self.tree.nodes.insert(node);
        NodeId(id)
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let node = Node::comment(id, text);
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
        // Remove from old parent if any (both DOM and Taffy)
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            // Remove from old taffy parent
            if let (Some(old_taffy_parent), Some(child_taffy)) = (
                self.tree.nodes[old_parent].taffy_id,
                self.tree.nodes[c].taffy_id,
            ) {
                self.taffy_remove_child_safe(old_taffy_parent, child_taffy);
            }
        }
        self.tree.nodes[c].parent = Some(p);
        self.tree.nodes[p].children.push(c);
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) = (
            self.tree.nodes[p].taffy_id,
            self.tree.nodes[c].taffy_id,
        ) {
            let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
        }
        // Invalidate parent's IFC (structure changed)
        self.invalidate_parent_ifc(p);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

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
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) = (
            self.tree.nodes[p].taffy_id,
            self.tree.nodes[c].taffy_id,
        ) {
            self.taffy_remove_child_safe(parent_taffy, child_taffy);
        }
        // Invalidate parent's IFC
        self.invalidate_parent_ifc(p);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        let p = parent.0;
        let c = child.0;
        let r = reference.0;
        // Invalidate old IFC
        self.invalidate_ifc_for_node(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            if let (Some(old_taffy_parent), Some(child_taffy)) = (
                self.tree.nodes[old_parent].taffy_id,
                self.tree.nodes[c].taffy_id,
            ) {
                self.taffy_remove_child_safe(old_taffy_parent, child_taffy);
            }
        }
        self.tree.nodes[c].parent = Some(p);
        let insert_pos = if let Some(pos) = self.tree.nodes[p].children.iter().position(|&x| x == r) {
            self.tree.nodes[p].children.insert(pos, c);
            Some(pos)
        } else {
            self.tree.nodes[p].children.push(c);
            None
        };
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) = (
            self.tree.nodes[p].taffy_id,
            self.tree.nodes[c].taffy_id,
        ) {
            if let Some(pos) = insert_pos {
                // Count taffy children before this position to find taffy index
                let taffy_idx = self.compute_taffy_child_index(p, pos);
                let _ = self.tree.taffy.insert_child_at_index(parent_taffy, taffy_idx, child_taffy);
            } else {
                let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            }
        }
        self.invalidate_parent_ifc(p);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        self.invalidate_ifc_for_node(old.0);
        self.clear_ifc_root_recursive(old.0);
        self.invalidate_ifc_for_node(new.0);
        self.clear_ifc_root_recursive(new.0);
        if let Some(parent_id) = self.tree.nodes[old.0].parent {
            // Remove new from its old parent if any
            if let Some(old_parent) = self.tree.nodes[new.0].parent {
                self.tree.nodes[old_parent].children.retain(|&x| x != new.0);
                if let (Some(old_taffy_parent), Some(new_taffy)) = (
                    self.tree.nodes[old_parent].taffy_id,
                    self.tree.nodes[new.0].taffy_id,
                ) {
                    self.taffy_remove_child_safe(old_taffy_parent, new_taffy);
                }
            }
            // Replace old with new in parent's children
            if let Some(pos) = self.tree.nodes[parent_id].children.iter().position(|&x| x == old.0) {
                self.tree.nodes[parent_id].children[pos] = new.0;
                // Sync taffy: remove old, insert new at same position
                if let Some(parent_taffy) = self.tree.nodes[parent_id].taffy_id {
                    if let Some(old_taffy) = self.tree.nodes[old.0].taffy_id {
                        self.taffy_remove_child_safe(parent_taffy, old_taffy);
                    }
                    if let Some(new_taffy) = self.tree.nodes[new.0].taffy_id {
                        let taffy_idx = self.compute_taffy_child_index(parent_id, pos);
                        let _ = self.tree.taffy.insert_child_at_index(parent_taffy, taffy_idx, new_taffy);
                    }
                }
            }
            self.tree.nodes[new.0].parent = Some(parent_id);
            self.tree.nodes[old.0].parent = None;
            self.invalidate_parent_ifc(parent_id);
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
        }
    }

    fn remove_node(&mut self, node: NodeId) {
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
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
        }
        self.tree.nodes[node.0].parent = None;
        // Don't remove from slab yet — caller may still reference it
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        let n = node.0;
        // Invalidate IFC if this node belongs to one
        self.invalidate_ifc_for_node(n);
        // Also invalidate parent's IFC
        if let Some(parent_id) = self.tree.nodes[n].parent {
            self.invalidate_parent_ifc(parent_id);
        }
        match &mut self.tree.nodes[n].kind {
            NodeKind::Text(t) => {
                t.content = text.to_string();
                // Update the Taffy NodeContext too
                if let Some(taffy_id) = self.tree.nodes[n].taffy_id {
                    if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id) {
                        if let NodeContext::Text(tm) = ctx {
                            tm.content = text.to_string();
                        }
                    }
                    let _ = self.tree.taffy.mark_dirty(taffy_id);
                }
            }
            _ => {
                // For elements: remove all children and add a text child
                let old_children: Vec<_> = self.tree.nodes[n].children.clone();
                for child in old_children {
                    self.tree.nodes[child].parent = None;
                    // Remove from taffy parent
                    if let (Some(parent_taffy), Some(child_taffy)) = (
                        self.tree.nodes[n].taffy_id,
                        self.tree.nodes[child].taffy_id,
                    ) {
                        self.taffy_remove_child_safe(parent_taffy, child_taffy);
                    }
                }
                self.tree.nodes[n].children.clear();
                // Create text child with taffy node and context
                let text_id = self.tree.nodes.vacant_key();
                let mut text_node = Node::text(text_id, text);
                text_node.parent = Some(n);
                let context = NodeContext::Text(TextMeasure {
                    content: text.to_string(),
                    font_size: 16.0,
                    font_weight: 400.0,
                    font_family: String::new(),
                    line_height_css: String::new(),
                });
                let taffy_id = self.tree.taffy.new_leaf_with_context(taffy::Style::default(), context).unwrap();
                text_node.taffy_id = Some(taffy_id);
                self.tree.taffy_map.insert(taffy_id, text_id);
                self.tree.nodes.insert(text_node);
                self.tree.nodes[n].children.push(text_id);
                // Add to taffy parent
                if let Some(parent_taffy) = self.tree.nodes[n].taffy_id {
                    let _ = self.tree.taffy.add_child(parent_taffy, taffy_id);
                }
            }
        }
        self.push_dirty(n);

        // If this node is a <style> element, reload its CSS
        self.maybe_load_style_css(n);
        // If the parent is a <style> element (text node content changed), reload
        if let Some(parent_id) = self.tree.nodes[n].parent {
            self.maybe_load_style_css(parent_id);
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        self.tree.nodes[node.0].attributes.insert(name.to_string(), value.to_string());
        // Invalidate IFC if this node belongs to one (style/class changes affect inline layout)
        if name == "style" || name == "class" {
            self.invalidate_ifc_for_node(node.0);
            // Also invalidate parent's IFC in case this is an inline child
            if let Some(parent_id) = self.tree.nodes[node.0].parent {
                self.invalidate_parent_ifc(parent_id);
            }
        }
        // SVG elements: width/height HTML attributes affect layout sizing
        let needs_style_recompute = name == "class" || name == "style"
            || ((name == "width" || name == "height" || name == "viewBox")
                && self.tree.nodes[node.0].tag() == Some("svg"));

        if needs_style_recompute {
            // Compute merged style: class-based + inline overlay
            let merged = self.compute_merged_style(node.0);
            self.tree.nodes[node.0].computed_style_str = layout::props_to_style_string(&merged);
            self.tree.nodes[node.0].cached_style_props = merged.clone();
            if let Some(taffy_id) = self.tree.nodes[node.0].taffy_id {
                let dd = self.default_display_for_node(node.0);
                let taffy_style = layout::build_taffy_style_full(&merged, &self.tree.viewport, dd);
                let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
            }
            // Sync display_mode from merged styles
            if let Some(display) = merged.get("display") {
                let mode = match display.as_str() {
                    "inline" => Some(DisplayMode::Inline),
                    "inline-block" => Some(DisplayMode::InlineBlock),
                    "block" => Some(DisplayMode::Block),
                    "flex" => Some(DisplayMode::Flex),
                    _ => None,
                };
                if let Some(m) = mode {
                    self.tree.nodes[node.0].display_mode = m;
                }
            }
            self.push_dirty_flags(node.0, DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
        } else {
            self.push_dirty(node.0);
        }
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        self.tree.nodes[node.0].attributes.remove(name);
        if name == "class" || name == "style" {
            self.push_dirty_flags(node.0, DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
        } else {
            self.push_dirty(node.0);
        }
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.tree.nodes.get(node.0)?.attributes.get(name).cloned()
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        let mut styles: HashMap<String, String> = self.tree.nodes[node.0]
            .attributes
            .get("style")
            .map(|s| parse_style_string(s))
            .unwrap_or_default();
        styles.insert(property.to_string(), value.to_string());
        let style_str = styles.iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        self.tree.nodes[node.0].attributes.insert("style".to_string(), style_str.clone());
        // Recompute merged styles (class + inline) and cache
        let merged = self.compute_merged_style(node.0);
        self.tree.nodes[node.0].computed_style_str = layout::props_to_style_string(&merged);
        self.tree.nodes[node.0].cached_style_props = merged;
        // Update taffy style
        if let Some(taffy_id) = self.tree.nodes[node.0].taffy_id {
            let props = &self.tree.nodes[node.0].cached_style_props;
            let dd = self.default_display_for_node(node.0);
            let taffy_style = layout::build_taffy_style_full(&props, &self.tree.viewport, dd);
            let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
        }
        // Sync display_mode
        if property == "display" {
            if let Some(mode) = layout::parse_display_mode(&style_str) {
                self.tree.nodes[node.0].display_mode = mode;
            }
        }
        self.push_dirty_flags(node.0, DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
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
        self.query_recursive(self.tree.root_id, selector).map(NodeId)
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.tree.nodes.get(node.0)
            .map(|n| n.children.iter().map(|&c| NodeId(c)).collect())
            .unwrap_or_default()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        let p = parent.0;
        let c = child.0;
        // Invalidate old IFC
        self.invalidate_ifc_for_node(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            if let (Some(old_taffy_parent), Some(child_taffy)) = (
                self.tree.nodes[old_parent].taffy_id,
                self.tree.nodes[c].taffy_id,
            ) {
                self.taffy_remove_child_safe(old_taffy_parent, child_taffy);
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
        if let (Some(parent_taffy), Some(child_taffy)) = (
            self.tree.nodes[p].taffy_id,
            self.tree.nodes[c].taffy_id,
        ) {
            let taffy_idx = self.compute_taffy_child_index(p, actual_index);
            let _ = self.tree.taffy.insert_child_at_index(parent_taffy, taffy_idx, child_taffy);
        }
        self.invalidate_parent_ifc(p);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
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

    fn set_inner_html(&mut self, node: NodeId, _html: &str) {
        // Phase 9: HTML parser integration
        // For now, clear children
        let old_children: Vec<_> = self.tree.nodes[node.0].children.clone();
        for child in old_children {
            self.clear_ifc_root_recursive(child);
            self.tree.nodes[child].parent = None;
            self.tree.remove_subtree(child);
        }
        self.tree.nodes[node.0].children.clear();
        self.invalidate_parent_ifc(node.0);
        self.push_dirty_flags(node.0, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
    }
}

impl RinchDocument {
    /// Load CSS into the document's stylesheet.
    ///
    /// Parses the CSS string and merges rules/variables into the existing stylesheet.
    /// Call this at startup to load theme and widget CSS.
    pub fn load_css(&mut self, css: &str) {
        self.tree.stylesheet.add_css(css);
    }

    /// If the given node is a `<style>` element, extract its text children's content
    /// and load it into the stylesheet.
    fn maybe_load_style_css(&mut self, node_id: usize) {
        let is_style = self.tree.nodes.get(node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "style")
            .unwrap_or(false);
        if !is_style { return; }

        // Collect text content from children
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();
        let mut css = String::new();
        for child_id in children {
            if let Some(text) = self.tree.nodes.get(child_id).and_then(|n| n.text_content()) {
                css.push_str(text);
            }
        }
        if !css.is_empty() {
            self.tree.stylesheet.add_css(&css);
            // Recompute all styles since new CSS rules may affect existing nodes
            self.recompute_all_styles();
        }
    }

    /// Set viewport dimensions for resolving vh/vw CSS units.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.tree.viewport = crate::layout::Viewport { width, height };
    }

    /// Get the default display type for a node based on its tag.
    fn default_display_for_node(&self, node_id: usize) -> layout::DefaultDisplay {
        match self.tree.nodes[node_id].display_mode {
            crate::node::DisplayMode::Inline | crate::node::DisplayMode::InlineBlock => layout::DefaultDisplay::Inline,
            _ => layout::DefaultDisplay::Block,
        }
    }

    /// Recompute taffy styles for all element nodes.
    /// Called when viewport dimensions change to update vh/vw-dependent styles.
    /// Uses cached style props to avoid re-running CSS selector matching.
    fn recompute_all_styles(&mut self) {
        let node_ids: Vec<usize> = self.tree.nodes.iter().map(|(id, _)| id).collect();
        for node_id in node_ids {
            // Skip root and html nodes — their Taffy styles are manually set in NodeTree::new()
            // and have no CSS representation (size: 100%, flex-direction: column).
            if node_id == self.tree.root_id || node_id == self.tree.html_id {
                continue;
            }
            if !self.tree.nodes[node_id].is_element() {
                continue;
            }
            // Skip elements that should never participate in layout.
            if matches!(self.tree.nodes[node_id].tag(), Some("style" | "script" | "head" | "meta" | "link" | "title")) {
                continue;
            }
            if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
                // Use cached style props to avoid expensive CSS selector re-matching.
                // On resize, CSS rules haven't changed — only vh/vw values need recalc.
                let merged = if !self.tree.nodes[node_id].cached_style_props.is_empty() {
                    self.tree.nodes[node_id].cached_style_props.clone()
                } else {
                    let m = self.compute_merged_style(node_id);
                    self.tree.nodes[node_id].computed_style_str = layout::props_to_style_string(&m);
                    self.tree.nodes[node_id].cached_style_props = m.clone();
                    m
                };
                let dd = self.default_display_for_node(node_id);
                let mut taffy_style = layout::build_taffy_style_full(&merged, &self.tree.viewport, dd);
                // Body node needs flex_grow: 1 and height: auto to fill the viewport,
                // but these aren't in CSS. Preserve them unless CSS explicitly sets them.
                if node_id == self.tree.body_id {
                    if !merged.contains_key("flex-grow") && !merged.contains_key("flex") {
                        taffy_style.flex_grow = 1.0;
                    }
                    if !merged.contains_key("height") {
                        taffy_style.size.height = taffy::Dimension::auto();
                    }
                    if !merged.contains_key("width") {
                        taffy_style.size.width = taffy::Dimension::percent(1.0);
                    }
                }
                let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
            }
        }
    }

    /// Compute merged style properties for a node (class-based + inline).
    /// Resolves var() and rem units.
    fn compute_merged_style(&self, node_id: usize) -> HashMap<String, String> {
        let node = &self.tree.nodes[node_id];
        let class_attr = node.attributes.get("class").map(|s| s.as_str());
        let inline_style = node.attributes.get("style").map(|s| s.as_str());
        let tag = node.tag();

        // Build ancestor chain for combinator selector matching
        let ancestors = self.build_ancestor_chain(node_id);

        // Build element state for this node
        let element_state = crate::stylesheet::ElementState {
            tag: tag.map(|t| t.to_string()),
            classes: class_attr
                .unwrap_or("")
                .split_whitespace()
                .map(|s| s.to_string())
                .collect(),
            attributes: node.attributes.clone(),
            ..Default::default()
        };

        let mut merged = crate::stylesheet::compute_merged_styles_with_state(
            &self.tree.stylesheet,
            class_attr,
            inline_style,
            Some(&element_state),
            &ancestors,
            tag,
        );

        // For SVG elements, inject width/height from HTML attributes into CSS props
        // so Taffy assigns them proper layout dimensions.
        if tag == Some("svg") {
            if !merged.contains_key("width") {
                if let Some(w) = node.attributes.get("width") {
                    // Add "px" if it's a bare number
                    let css_w = if w.ends_with("px") || w.ends_with('%') {
                        w.clone()
                    } else {
                        format!("{}px", w)
                    };
                    merged.insert("width".to_string(), css_w);
                }
            }
            if !merged.contains_key("height") {
                if let Some(h) = node.attributes.get("height") {
                    let css_h = if h.ends_with("px") || h.ends_with('%') {
                        h.clone()
                    } else {
                        format!("{}px", h)
                    };
                    merged.insert("height".to_string(), css_h);
                }
            }
            // SVG elements should not stretch in flex containers
            if !merged.contains_key("flex-shrink") {
                merged.insert("flex-shrink".to_string(), "0".to_string());
            }
        }

        merged
    }

    /// Build an ancestor chain from a node up to the root.
    /// Each ancestor is represented as an ElementState for selector matching.
    fn build_ancestor_chain(&self, node_id: usize) -> Vec<crate::stylesheet::ElementState> {
        let mut ancestors = Vec::new();
        let mut current = self.tree.nodes.get(node_id).and_then(|n| n.parent);
        while let Some(pid) = current {
            if let Some(parent) = self.tree.nodes.get(pid) {
                if parent.is_element() {
                    let tag = parent.tag().map(|t| t.to_string());
                    let classes = parent
                        .attributes
                        .get("class")
                        .map(|c| c.split_whitespace().map(|s| s.to_string()).collect())
                        .unwrap_or_default();
                    ancestors.push(crate::stylesheet::ElementState {
                        tag,
                        classes,
                        attributes: parent.attributes.clone(),
                        ..Default::default()
                    });
                }
                current = parent.parent;
            } else {
                break;
            }
        }
        ancestors
    }

    /// Compute the taffy child index for a DOM child at the given position.
    /// This counts only children that have taffy IDs (skipping comments).
    fn compute_taffy_child_index(&self, parent_id: usize, dom_index: usize) -> usize {
        let children = &self.tree.nodes[parent_id].children;
        let mut taffy_idx = 0;
        for i in 0..dom_index {
            if i < children.len() {
                if self.tree.nodes[children[i]].taffy_id.is_some() {
                    taffy_idx += 1;
                }
            }
        }
        taffy_idx
    }

    /// Resolve layout using Taffy.
    ///
    /// Computes layout for the entire tree given a viewport size,
    /// then reads layout results back into each node's `layout` field.
    /// Text nodes are measured using Parley for accurate text layout.
    pub fn resolve_layout(&mut self, width: f32, height: f32) {
        let old_viewport = self.tree.viewport;
        self.tree.viewport = crate::layout::Viewport { width, height };

        // Recompute all taffy styles when viewport changes (for vh/vw units)
        if (old_viewport.width - width).abs() > 0.5 || (old_viewport.height - height).abs() > 0.5 {
            self.recompute_all_styles();
        }

        let root_taffy = match self.tree.nodes[self.tree.root_id].taffy_id {
            Some(id) => id,
            None => return,
        };

        // Handle display:contents by rebuilding taffy children for affected nodes
        self.sync_display_contents();

        // Detect and set up inline formatting contexts
        self.setup_inline_formatting_contexts();

        // Pre-compute layout for inline-block children that were detached from Taffy.
        // They need their own subtree measured so walk_inline_children can read dimensions.
        self.compute_inline_block_layouts();

        // Sync font-size from parent elements to text node contexts
        self.sync_text_contexts();

        let available_space = taffy::Size {
            width: taffy::AvailableSpace::Definite(width),
            height: taffy::AvailableSpace::Definite(height),
        };

        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;
        let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        let nodes = &self.tree.nodes;

        self.tree.taffy.compute_layout_with_measure(
            root_taffy,
            available_space,
            |known_dims, avail_space, _node_id, context, _style| {
                let max_width = match avail_space.width {
                    taffy::AvailableSpace::Definite(w) => Some(w),
                    taffy::AvailableSpace::MaxContent => None,
                    taffy::AvailableSpace::MinContent => Some(0.0),
                };

                match context {
                    Some(NodeContext::Text(text)) => {
                        if text.content.is_empty() {
                            return taffy::Size { width: 0.0, height: 0.0 };
                        }

                        let mut builder = layout_cx.ranged_builder(font_cx, &text.content, 1.0, true);
                        builder.push_default(parley::style::StyleProperty::<[u8; 4]>::FontSize(text.font_size));
                        if (text.font_weight - 400.0).abs() > 1.0 {
                            builder.push_default(parley::style::StyleProperty::FontWeight(
                                parley::style::FontWeight::new(text.font_weight),
                            ));
                        }
                        if let Some(lh) = layout::css_line_height_to_parley(&text.line_height_css) {
                            builder.push_default(parley::style::StyleProperty::LineHeight(lh));
                        }
                        let font_stack = if !text.font_family.is_empty() {
                            std::borrow::Cow::Owned(text.font_family.clone())
                        } else {
                            std::borrow::Cow::Borrowed("sans-serif")
                        };
                        builder.push_default(parley::style::StyleProperty::<[u8; 4]>::FontStack(
                            parley::style::FontStack::Source(font_stack),
                        ));
                        let mut layout = builder.build(&text.content);
                        let wrap_width = known_dims.width.or(max_width);
                        layout.break_all_lines(wrap_width);

                        taffy::Size {
                            width: known_dims.width.unwrap_or(layout.width()),
                            height: known_dims.height.unwrap_or(layout.height()),
                        }
                    }
                    Some(NodeContext::InlineRoot(root_id)) => {
                        // Build Parley inline layout for this IFC root
                        let root_id = *root_id;
                        let inline_layout = Self::build_inline_layout(
                            nodes,
                            root_id,
                            max_width,
                            1.0,
                            font_cx,
                            &mut paint_layout_cx,
                        );
                        let w = inline_layout.layout.width();
                        let h = inline_layout.layout.height();
                        // Measure callback for IFC root
                        taffy::Size {
                            width: known_dims.width.unwrap_or(w),
                            height: known_dims.height.unwrap_or(h),
                        }
                    }
                    _ => taffy::Size::ZERO,
                }
            },
        ).unwrap();

        // Read layout results back into nodes
        self.read_layout_results(self.tree.root_id);

        // Build inline layouts for IFC roots (rebuild with final widths and store)
        self.build_ifc_layouts(&mut paint_layout_cx);
    }

    /// Sync font-size from parent elements into text node contexts.
    ///
    /// Walks all text nodes and updates their `TextMeasure.font_size`
    /// from the parent element's computed style.
    fn sync_text_contexts(&mut self) {
        let mut updates: Vec<(taffy::NodeId, f32, f32, String, String)> = Vec::new();

        for (_id, node) in &self.tree.nodes {
            if let NodeKind::Text(_) = &node.kind {
                let taffy_id = match node.taffy_id {
                    Some(t) => t,
                    None => continue,
                };
                let parent_style = node.parent
                    .and_then(|p| self.tree.nodes.get(p))
                    .map(|parent| {
                        if !parent.computed_style_str.is_empty() {
                            parent.computed_style_str.clone()
                        } else {
                            parent.attributes.get("style").cloned().unwrap_or_default()
                        }
                    })
                    .unwrap_or_default();
                let font_size = layout::parse_font_size(&parent_style).unwrap_or(16.0);
                let font_weight = layout::parse_font_weight(&parent_style).unwrap_or(400.0);
                let font_family = layout::parse_font_family(&parent_style).unwrap_or_default();
                let line_height_css = layout::parse_line_height_css(&parent_style).unwrap_or_default();
                updates.push((taffy_id, font_size, font_weight, font_family, line_height_css));
            }
        }

        for (taffy_id, font_size, font_weight, font_family, line_height_css) in updates {
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id) {
                if let NodeContext::Text(tm) = ctx {
                    tm.font_size = font_size;
                    tm.font_weight = font_weight;
                    tm.font_family = font_family;
                    tm.line_height_css = line_height_css;
                }
            }
        }
    }

    /// Recursively read Taffy layout results into node LayoutResult fields.
    fn read_layout_results(&mut self, node_id: usize) {
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
            if let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id) {
                let node = &mut self.tree.nodes[node_id];
                node.layout = LayoutResult {
                    x: taffy_layout.location.x,
                    y: taffy_layout.location.y,
                    width: taffy_layout.size.width,
                    height: taffy_layout.size.height,
                };
            }
        }

        for child_id in children {
            self.read_layout_results(child_id);
        }
    }

    /// Handle display:contents nodes by reparenting their taffy children
    /// to the taffy parent of the display:contents node.
    fn sync_display_contents(&mut self) {
        // Collect display:contents nodes
        let mut contents_nodes = Vec::new();
        for (id, node) in &self.tree.nodes {
            if let Some(style_str) = node.attributes.get("style") {
                if layout::is_display_contents(style_str) {
                    contents_nodes.push(id);
                }
            }
        }

        for node_id in contents_nodes {
            let parent_id = match self.tree.nodes[node_id].parent {
                Some(p) => p,
                None => continue,
            };
            let parent_taffy = match self.tree.nodes[parent_id].taffy_id {
                Some(t) => t,
                None => continue,
            };
            let node_taffy = match self.tree.nodes[node_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            // Remove the contents node from taffy parent
            self.taffy_remove_child_safe(parent_taffy, node_taffy);

            // Find the position of this node among parent's DOM children to know where
            // to insert its children in the taffy tree
            let parent_children: Vec<usize> = self.tree.nodes[parent_id].children.clone();
            let dom_pos = parent_children.iter().position(|&c| c == node_id).unwrap_or(0);

            // Compute taffy insert index (count taffy-having siblings before this position,
            // excluding the contents node itself)
            let mut taffy_insert_idx = 0;
            for i in 0..dom_pos {
                let sibling_id = parent_children[i];
                if sibling_id != node_id && self.tree.nodes[sibling_id].taffy_id.is_some() {
                    // Check if sibling is NOT also display:contents (already removed)
                    let is_contents = self.tree.nodes[sibling_id].attributes.get("style")
                        .map(|s| layout::is_display_contents(s))
                        .unwrap_or(false);
                    if !is_contents {
                        taffy_insert_idx += 1;
                    }
                }
            }

            // Add children of contents node directly to taffy parent
            let grandchildren: Vec<usize> = self.tree.nodes[node_id].children.clone();
            for (i, &grandchild_id) in grandchildren.iter().enumerate() {
                if let Some(gc_taffy) = self.tree.nodes[grandchild_id].taffy_id {
                    // Remove from contents node's taffy
                    self.taffy_remove_child_safe(node_taffy, gc_taffy);
                    let _ = self.tree.taffy.insert_child_at_index(parent_taffy, taffy_insert_idx + i, gc_taffy);
                }
            }

            // Set the contents node's taffy to display:none with zero size
            let _ = self.tree.taffy.set_style(node_taffy, taffy::Style {
                display: taffy::Display::None,
                ..Default::default()
            });
        }
    }

    /// Invalidate the IFC that owns a node (if any).
    ///
    /// Clears the IFC root's cached text_layout so it rebuilds on next layout pass.
    /// Also checks the parent's text_layout as a fallback when ifc_root hasn't been
    /// set yet (before the first layout pass).
    fn invalidate_ifc_for_node(&mut self, node_id: usize) {
        if let Some(ifc_root_id) = self.tree.nodes.get(node_id).and_then(|n| n.ifc_root) {
            if let Some(root) = self.tree.nodes.get_mut(ifc_root_id) {
                root.text_layout = None;
            }
        } else {
            // Fallback: walk ancestors to find one with text_layout (the IFC root)
            let mut cur = self.tree.nodes.get(node_id).and_then(|n| n.parent);
            while let Some(pid) = cur {
                if self.tree.nodes.get(pid).map(|p| p.text_layout.is_some()).unwrap_or(false) {
                    self.tree.nodes[pid].text_layout = None;
                    break;
                }
                cur = self.tree.nodes.get(pid).and_then(|n| n.parent);
            }
        }
    }

    /// Safely remove a child from a Taffy parent, checking membership first.
    /// Taffy's `remove_child` panics if the child isn't actually a child of the parent,
    /// which can happen when inline children were detached by `setup_inline_formatting_contexts`.
    fn taffy_remove_child_safe(&mut self, parent_taffy: taffy::NodeId, child_taffy: taffy::NodeId) {
        if let Ok(children) = self.tree.taffy.children(parent_taffy) {
            if children.contains(&child_taffy) {
                let _ = self.tree.taffy.remove_child(parent_taffy, child_taffy);
            }
        }
    }

    /// Clear ifc_root on a node and all its descendants.
    fn clear_ifc_root_recursive(&mut self, node_id: usize) {
        // Use iterative approach to avoid stack overflow
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.ifc_root = None;
                stack.extend(node.children.iter().copied());
            }
        }
    }

    /// Invalidate IFC state for a parent element.
    /// Clears text_layout on the parent and ifc_root on all its inline children.
    /// Also marks the Taffy node dirty so the measure callback re-fires.
    fn invalidate_parent_ifc(&mut self, parent_id: usize) {
        if let Some(parent) = self.tree.nodes.get_mut(parent_id) {
            parent.text_layout = None;
        }
        if let Some(taffy_id) = self.tree.nodes.get(parent_id).and_then(|n| n.taffy_id) {
            let _ = self.tree.taffy.mark_dirty(taffy_id);
        }
        let children: Vec<usize> = self.tree.nodes.get(parent_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child_id in children {
            if let Some(child) = self.tree.nodes.get_mut(child_id) {
                child.ifc_root = None;
            }
        }
    }

    /// Build inline layouts for all IFC roots after Taffy layout.
    ///
    /// Uses the computed width from Taffy as the available width for Parley line breaking.
    fn build_ifc_layouts(&mut self, paint_layout_cx: &mut parley::LayoutContext<Brush>) {
        // Collect IFC roots (elements that have inline children with ifc_root set)
        let mut ifc_roots: Vec<usize> = Vec::new();
        for (id, node) in &self.tree.nodes {
            if !node.is_element() { continue; }
            if matches!(node.display_mode, DisplayMode::Inline | DisplayMode::InlineBlock | DisplayMode::Flex) { continue; }
            // Check if any child has ifc_root pointing to this node
            let is_ifc = node.children.iter().any(|&child_id| {
                self.tree.nodes.get(child_id)
                    .map(|c| c.ifc_root == Some(id))
                    .unwrap_or(false)
            });
            if is_ifc {
                ifc_roots.push(id);
            }
        }

        for root_id in ifc_roots {
            let node = &self.tree.nodes[root_id];
            let available_width = node.layout.width;
            let max_width = if available_width > 0.0 { Some(available_width) } else { None };

            let inline_layout = Self::build_inline_layout(
                &self.tree.nodes,
                root_id,
                max_width,
                1.0, // scale=1.0 for layout; paint scales later
                &mut self.font_cx,
                paint_layout_cx,
            );

            // Write positions from Parley layout back to child nodes
            // Walk the layout lines to find positioned inline boxes and text runs
            self.write_inline_positions(root_id, &inline_layout);

            self.tree.nodes[root_id].text_layout = Some(Box::new(inline_layout));
        }
    }

    /// Write computed positions from an InlineLayout back into child node layout fields.
    fn write_inline_positions(&mut self, root_id: usize, inline_layout: &InlineLayout) {
        let root_layout = self.tree.nodes[root_id].layout;

        // Walk Parley layout lines to find positioned items
        for line in inline_layout.layout.lines() {
            for item in line.items() {
                match item {
                    parley::layout::PositionedLayoutItem::GlyphRun(_) => {
                        // Text runs don't map to individual child nodes
                    }
                    parley::layout::PositionedLayoutItem::InlineBox(positioned_box) => {
                        let child_id = positioned_box.id as usize;
                        if let Some(child) = self.tree.nodes.get_mut(child_id) {
                            child.layout.x = positioned_box.x;
                            child.layout.y = positioned_box.y;
                        }
                    }
                }
            }
        }

        // For text nodes that are direct children, set their layout to cover the
        // IFC root's content area (they're painted via Parley, not individually)
        let children: Vec<usize> = self.tree.nodes[root_id].children.clone();
        for child_id in children {
            if let Some(child) = self.tree.nodes.get(child_id) {
                if child.is_text() && child.ifc_root == Some(root_id) {
                    // Text nodes in IFC get zero layout — they're painted by the IFC root
                    // But we set their position relative to root for hit testing
                    if let Some(child) = self.tree.nodes.get_mut(child_id) {
                        child.layout.x = 0.0;
                        child.layout.y = 0.0;
                        child.layout.width = root_layout.width;
                        child.layout.height = root_layout.height;
                    }
                }
            }
        }
    }

    /// Detect IFC roots and mark inline children.
    ///
    /// An element is an IFC root if it's a block/flex container that has
    /// inline content that benefits from unified Parley layout — specifically:
    /// - Multiple inline children (text + text, text + inline element, etc.)
    /// - At least one inline element (span, em, etc.)
    ///
    /// Single text children under block parents continue using the existing
    /// Taffy text measurement path (no IFC needed).
    fn setup_inline_formatting_contexts(&mut self) {
        let mut ifc_roots: Vec<usize> = Vec::new();
        for (id, node) in &self.tree.nodes {
            if !node.is_element() { continue; }
            // Only block containers can be IFC roots — skip inline, inline-block, and flex
            if matches!(node.display_mode, DisplayMode::Inline | DisplayMode::InlineBlock | DisplayMode::Flex) { continue; }

            let inline_children: Vec<usize> = node.children.iter()
                .filter(|&&child_id| {
                    self.tree.nodes.get(child_id)
                        .map(|c| c.is_inline())
                        .unwrap_or(false)
                })
                .copied()
                .collect();

            // Only activate IFC when there's actual inline formatting context complexity
            let has_inline_elements = inline_children.iter().any(|&child_id| {
                self.tree.nodes.get(child_id)
                    .map(|c| matches!(c.kind, NodeKind::Element(_)) && matches!(c.display_mode, DisplayMode::Inline | DisplayMode::InlineBlock))
                    .unwrap_or(false)
            });
            let needs_ifc = has_inline_elements || inline_children.len() > 1;

            if needs_ifc {
                ifc_roots.push(id);
            }
        }

        for root_id in ifc_roots {
            let children: Vec<usize> = self.tree.nodes[root_id].children.clone();
            let root_taffy = match self.tree.nodes[root_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            // Remove inline children from Taffy (Parley will handle their layout)
            for &child_id in &children {
                let child = match self.tree.nodes.get(child_id) {
                    Some(c) => c,
                    None => continue,
                };
                if child.is_inline() {
                    if let Some(child_taffy) = child.taffy_id {
                        if let Ok(taffy_children) = self.tree.taffy.children(root_taffy) {
                            if taffy_children.contains(&child_taffy) {
                                let _ = self.tree.taffy.remove_child(root_taffy, child_taffy);
                            }
                        }
                    }
                    if let Some(c) = self.tree.nodes.get_mut(child_id) {
                        c.ifc_root = Some(root_id);
                    }
                }
            }

            // Set NodeContext::InlineRoot on the IFC root's Taffy node
            // so the measure function fires for it
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(root_taffy) {
                *ctx = NodeContext::InlineRoot(root_id);
            } else {
                // Element nodes don't have context by default — we need to set one.
                // Taffy only calls measure for nodes with context, so we must ensure it has one.
                let _ = self.tree.taffy.set_node_context(root_taffy, Some(NodeContext::InlineRoot(root_id)));
            }
        }
    }

    /// Pre-compute layout for inline-block children that were detached from Taffy.
    ///
    /// Inline-block children are removed from their parent's Taffy tree (so the parent
    /// uses InlineRoot measurement), but they still need their own subtree computed
    /// so `walk_inline_children` can read their width/height for Parley InlineBox.
    fn compute_inline_block_layouts(&mut self) {
        // Collect inline-block children that belong to an IFC
        let mut ib_taffy_ids: Vec<taffy::NodeId> = Vec::new();
        for (_id, node) in &self.tree.nodes {
            if node.ifc_root.is_some() && node.display_mode == DisplayMode::InlineBlock {
                if let Some(taffy_id) = node.taffy_id {
                    ib_taffy_ids.push(taffy_id);
                }
            }
        }

        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;

        for taffy_id in ib_taffy_ids {
            let avail = taffy::Size {
                width: taffy::AvailableSpace::MaxContent,
                height: taffy::AvailableSpace::MaxContent,
            };
            let _ = self.tree.taffy.compute_layout_with_measure(
                taffy_id,
                avail,
                |known_dims, avail_space, _node_id, context, _style| {
                    let max_width = match avail_space.width {
                        taffy::AvailableSpace::Definite(w) => Some(w),
                        taffy::AvailableSpace::MaxContent => None,
                        taffy::AvailableSpace::MinContent => Some(0.0),
                    };
                    match context {
                        Some(NodeContext::Text(text)) => {
                            if text.content.is_empty() {
                                return taffy::Size::ZERO;
                            }
                            let mut builder = layout_cx.ranged_builder(font_cx, &text.content, 1.0, true);
                            builder.push_default(parley::style::StyleProperty::<[u8; 4]>::FontSize(text.font_size));
                            if (text.font_weight - 400.0).abs() > 1.0 {
                                builder.push_default(parley::style::StyleProperty::FontWeight(
                                    parley::style::FontWeight::new(text.font_weight),
                                ));
                            }
                            if let Some(lh) = layout::css_line_height_to_parley(&text.line_height_css) {
                                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
                            }
                            let font_stack = if !text.font_family.is_empty() {
                                std::borrow::Cow::Owned(text.font_family.clone())
                            } else {
                                std::borrow::Cow::Borrowed("sans-serif")
                            };
                            builder.push_default(parley::style::StyleProperty::<[u8; 4]>::FontStack(
                                parley::style::FontStack::Source(font_stack),
                            ));
                            let mut layout = builder.build(&text.content);
                            let wrap_width = known_dims.width.or(max_width);
                            layout.break_all_lines(wrap_width);
                            taffy::Size {
                                width: known_dims.width.unwrap_or(layout.width()),
                                height: known_dims.height.unwrap_or(layout.height()),
                            }
                        }
                        _ => taffy::Size::ZERO,
                    }
                },
            );

            // Read the computed layout back into the node
            if let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id) {
                let node_id = self.tree.taffy_map.get(&taffy_id).copied();
                if let Some(nid) = node_id {
                    if let Some(node) = self.tree.nodes.get_mut(nid) {
                        node.layout.width = taffy_layout.size.width;
                        node.layout.height = taffy_layout.size.height;
                    }
                }
            }
        }
    }

    /// Build a Parley inline layout for an IFC root node.
    ///
    /// Walks the IFC root's children, collecting text nodes and inline elements
    /// into a single Parley TreeBuilder layout. Returns the InlineLayout.
    fn build_inline_layout(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        max_width: Option<f32>,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
    ) -> InlineLayout {
        // Get root style properties (use computed style which merges class + inline)
        let root_style_str_owned;
        let root_style_str = if !nodes[root_id].computed_style_str.is_empty() {
            &nodes[root_id].computed_style_str
        } else {
            root_style_str_owned = nodes[root_id].attributes.get("style").cloned().unwrap_or_default();
            &root_style_str_owned
        };
        let root_font_size = layout::parse_font_size(root_style_str).unwrap_or(16.0) * scale;
        let root_color = root_style_str
            .split(';')
            .find_map(|part| {
                let (k, v) = part.split_once(':')?;
                if k.trim() == "color" { layout::parse_color(v.trim()) } else { None }
            })
            .unwrap_or_else(|| peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(0, 0, 0, 255));

        let mut root_text_style = parley::style::TextStyle {
            font_size: root_font_size,
            brush: Brush::Solid(root_color),
            font_stack: parley::style::FontStack::Source("sans-serif".into()),
            ..Default::default()
        };

        // Apply font-weight from root style
        if let Some(fw) = layout::parse_font_weight(root_style_str) {
            root_text_style.font_weight = parley::style::FontWeight::new(fw);
        }
        // Apply font-style from root style
        if let Some(fs) = layout::parse_font_style(root_style_str) {
            root_text_style.font_style = match fs {
                "italic" => parley::style::FontStyle::Italic,
                "oblique" => parley::style::FontStyle::Oblique(None),
                _ => parley::style::FontStyle::Normal,
            };
        }
        // Apply line-height from root style
        if let Some(lh) = layout::parse_line_height(root_style_str) {
            if lh > 10.0 {
                // Absolute value (e.g. "24px")
                root_text_style.line_height = parley::style::LineHeight::Absolute(lh);
            } else {
                // Relative multiplier (e.g. "1.5")
                root_text_style.line_height = parley::style::LineHeight::FontSizeRelative(lh);
            }
        }
        // Apply text-decoration from root style
        if let Some(td) = layout::parse_text_decoration(root_style_str) {
            if td.contains("underline") {
                root_text_style.has_underline = true;
            }
            if td.contains("line-through") {
                root_text_style.has_strikethrough = true;
            }
        }

        let mut builder = layout_cx.tree_builder(font_cx, scale, true, &root_text_style);

        // Apply white-space mode
        if let Some(ws) = layout::parse_white_space(root_style_str) {
            let collapse = match ws.as_str() {
                "pre" | "pre-wrap" | "pre-line" => parley::style::WhiteSpaceCollapse::Preserve,
                _ => parley::style::WhiteSpaceCollapse::Collapse,
            };
            builder.set_white_space_mode(collapse);
        }
        let mut child_positions = Vec::new();

        // Walk children and build the Parley tree
        Self::walk_inline_children(nodes, root_id, &mut builder, &mut child_positions, scale);

        let (text_layout, text_content) = builder.build();
        let mut text_layout = text_layout;
        text_layout.break_all_lines(max_width);

        // Parse text-align
        let alignment = layout::parse_text_align(root_style_str)
            .map(|a| match a.as_str() {
                "center" => parley::layout::Alignment::Center,
                "right" | "end" => parley::layout::Alignment::End,
                "justify" => parley::layout::Alignment::Justify,
                _ => parley::layout::Alignment::Start,
            })
            .unwrap_or(parley::layout::Alignment::Start);
        text_layout.align(alignment, parley::layout::AlignmentOptions::default());

        InlineLayout {
            layout: text_layout,
            text_content,
            child_positions,
        }
    }

    /// Recursively walk inline children, pushing text and style spans into the TreeBuilder.
    fn walk_inline_children(
        nodes: &slab::Slab<Node>,
        parent_id: usize,
        builder: &mut parley::TreeBuilder<'_, Brush>,
        child_positions: &mut Vec<(usize, LayoutResult)>,
        scale: f32,
    ) {
        let children: Vec<usize> = nodes[parent_id].children.clone();
        for child_id in children {
            let child = match nodes.get(child_id) {
                Some(c) => c,
                None => continue,
            };
            match &child.kind {
                NodeKind::Text(text_data) => {
                    if !text_data.content.is_empty() {
                        builder.push_text(&text_data.content);
                        // Record position placeholder — actual position comes from layout
                        child_positions.push((child_id, LayoutResult::default()));
                    }
                }
                NodeKind::Element(_) if child.display_mode == DisplayMode::Inline => {
                    // Push style span for inline element (use computed style which merges class + inline)
                    let style_str = if !child.computed_style_str.is_empty() {
                        &child.computed_style_str
                    } else {
                        child.attributes.get("style").map(|s| s.as_str()).unwrap_or("")
                    };
                    let mut props: Vec<parley::style::StyleProperty<'_, Brush>> = Vec::new();

                    if let Some(fs) = layout::parse_font_size(style_str) {
                        props.push(parley::style::StyleProperty::FontSize(fs * scale));
                    }
                    if let Some(fw) = layout::parse_font_weight(style_str) {
                        props.push(parley::style::StyleProperty::FontWeight(
                            parley::style::FontWeight::new(fw),
                        ));
                    }
                    if let Some(fstyle) = layout::parse_font_style(style_str) {
                        props.push(parley::style::StyleProperty::FontStyle(match fstyle {
                            "italic" => parley::style::FontStyle::Italic,
                            "oblique" => parley::style::FontStyle::Oblique(None),
                            _ => parley::style::FontStyle::Normal,
                        }));
                    }
                    if let Some(color) = style_str.split(';').find_map(|part| {
                        let (k, v) = part.split_once(':')?;
                        if k.trim() == "color" { layout::parse_color(v.trim()) } else { None }
                    }) {
                        props.push(parley::style::StyleProperty::Brush(Brush::Solid(color)));
                    }
                    if let Some(td) = layout::parse_text_decoration(style_str) {
                        if td.contains("underline") {
                            props.push(parley::style::StyleProperty::Underline(true));
                        }
                        if td.contains("line-through") {
                            props.push(parley::style::StyleProperty::Strikethrough(true));
                        }
                    }
                    if let Some(lh) = layout::parse_line_height(style_str) {
                        if lh > 10.0 {
                            props.push(parley::style::StyleProperty::LineHeight(
                                parley::style::LineHeight::Absolute(lh * scale),
                            ));
                        } else {
                            props.push(parley::style::StyleProperty::LineHeight(
                                parley::style::LineHeight::FontSizeRelative(lh),
                            ));
                        }
                    }

                    builder.push_style_modification_span(props.iter());
                    child_positions.push((child_id, LayoutResult::default()));

                    // Recurse into inline element's children
                    Self::walk_inline_children(nodes, child_id, builder, child_positions, scale);

                    builder.pop_style_span();
                }
                NodeKind::Element(_) if child.display_mode == DisplayMode::InlineBlock => {
                    // Inline-block: measure via Taffy first, then embed as InlineBox
                    let child_layout = &child.layout;
                    builder.push_inline_box(parley::InlineBox {
                        id: child_id as u64,
                        index: 0, // will be set by builder
                        width: child_layout.width * scale,
                        height: child_layout.height * scale,
                        kind: parley::InlineBoxKind::InFlow,
                    });
                    child_positions.push((child_id, LayoutResult::default()));
                }
                NodeKind::Comment(_) => {
                    // Skip comments in inline layout
                }
                _ => {
                    // Block children break inline flow — stop here
                    break;
                }
            }
        }
    }

    /// Simple recursive query selector.
    fn query_recursive(&self, node_id: usize, selector: &str) -> Option<usize> {
        let node = self.tree.nodes.get(node_id)?;

        // Match by #id
        if let Some(id) = selector.strip_prefix('#') {
            if node.attributes.get("id").map(|v| v.as_str()) == Some(id) {
                return Some(node_id);
            }
        }
        // Match by .class
        else if let Some(class) = selector.strip_prefix('.') {
            if let Some(classes) = node.attributes.get("class") {
                if classes.split_whitespace().any(|c| c == class) {
                    return Some(node_id);
                }
            }
        }
        // Match by tag name
        else if node.tag() == Some(selector) {
            return Some(node_id);
        }

        // Search children
        let children: Vec<_> = node.children.clone();
        for child in children {
            if let Some(found) = self.query_recursive(child, selector) {
                return Some(found);
            }
        }
        None
    }
}

/// Parse a CSS style string like "display: flex; gap: 8px" into key-value pairs.
fn parse_style_string(style: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for part in style.split(';') {
        let part = part.trim();
        if part.is_empty() { continue; }
        if let Some((key, value)) = part.split_once(':') {
            result.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    result
}
