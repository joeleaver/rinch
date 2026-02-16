//! Interaction state tracking: hover, focus, active state changes and style recomputation.

use crate::RinchDocument;
use crate::node::DirtyFlags;

impl RinchDocument {
    /// Update hover state: set the hovered node and its ancestors as hovered,
    /// clear previous hover, and recompute styles for affected nodes.
    /// Returns true if the hovered node changed (caller should repaint).
    pub fn update_hover(&mut self, new_hovered: Option<usize>) -> bool {
        let old_hovered = self.tree.hovered_node;
        if old_hovered == new_hovered {
            return false;
        }

        // Collect old hovered chain (node + ancestors)
        let mut old_chain = Vec::new();
        if let Some(old_id) = old_hovered {
            let mut current = Some(old_id);
            while let Some(id) = current {
                old_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Collect new hovered chain (node + ancestors)
        let mut new_chain = Vec::new();
        if let Some(new_id) = new_hovered {
            let mut current = Some(new_id);
            while let Some(id) = current {
                new_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Clear old hover state
        for &id in &old_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_hovered = false;
            }
        }

        // Set new hover state
        for &id in &new_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_hovered = true;
            }
        }

        self.tree.hovered_node = new_hovered;

        // Recompute styles for nodes that changed hover state
        // (nodes in old chain but not new, and vice versa)
        let mut dirty_nodes: Vec<usize> = Vec::new();
        for &id in &old_chain {
            if !new_chain.contains(&id) {
                dirty_nodes.push(id);
            }
        }
        for &id in &new_chain {
            if !old_chain.contains(&id) {
                dirty_nodes.push(id);
            }
        }

        // Clear cached styles for affected nodes and their descendants
        // so resolve_styles_recursive() will re-resolve their CSS
        for &id in &dirty_nodes {
            self.invalidate_style_subtree(id);
        }

        // Recompute styles using Stylo for affected nodes
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();

        // Mark dirty nodes for repaint
        for id in dirty_nodes {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }

        true
    }

    /// Update focus state: set the focused node, clear previous focus,
    /// and recompute styles for affected nodes.
    /// Returns true if the focused node changed (caller should repaint).
    pub fn update_focus(&mut self, new_focused: Option<usize>) -> bool {
        let old_focused = self.tree.focused_node;
        if old_focused == new_focused {
            return false;
        }

        // Clear old focus state
        if let Some(old_id) = old_focused
            && let Some(node) = self.tree.nodes.get_mut(old_id)
        {
            node.is_focused = false;
        }

        // Set new focus state
        if let Some(new_id) = new_focused
            && let Some(node) = self.tree.nodes.get_mut(new_id)
        {
            node.is_focused = true;
        }

        self.tree.focused_node = new_focused;

        // Clear cached styles for affected nodes and their descendants
        if let Some(id) = old_focused {
            self.invalidate_style_subtree(id);
        }
        if let Some(id) = new_focused {
            self.invalidate_style_subtree(id);
        }

        // Recompute styles
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();

        // Mark dirty nodes for repaint
        if let Some(id) = old_focused {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }
        if let Some(id) = new_focused {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }

        true
    }

    /// Update active (mouse-pressed) state: set the active node and its
    /// ancestors, clear previous active, and recompute styles.
    /// Returns true if the active node changed (caller should repaint).
    pub fn update_active(&mut self, new_active: Option<usize>) -> bool {
        let old_active = self.tree.active_node;
        if old_active == new_active {
            return false;
        }

        // Collect old active chain (node + ancestors)
        let mut old_chain = Vec::new();
        if let Some(old_id) = old_active {
            let mut current = Some(old_id);
            while let Some(id) = current {
                old_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Collect new active chain (node + ancestors)
        let mut new_chain = Vec::new();
        if let Some(new_id) = new_active {
            let mut current = Some(new_id);
            while let Some(id) = current {
                new_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Clear old active state
        for &id in &old_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_active = false;
            }
        }

        // Set new active state
        for &id in &new_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_active = true;
            }
        }

        self.tree.active_node = new_active;

        // Collect nodes whose active state changed (symmetric difference)
        let mut dirty_nodes: Vec<usize> = Vec::new();
        for &id in &old_chain {
            if !new_chain.contains(&id) {
                dirty_nodes.push(id);
            }
        }
        for &id in &new_chain {
            if !old_chain.contains(&id) {
                dirty_nodes.push(id);
            }
        }

        // Clear cached styles for affected nodes and their descendants
        for &id in &dirty_nodes {
            self.invalidate_style_subtree(id);
        }

        // Recompute styles
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();

        // Mark dirty nodes for repaint
        for id in dirty_nodes {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }

        true
    }
}
