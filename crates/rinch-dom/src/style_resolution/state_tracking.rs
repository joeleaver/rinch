//! Interaction state tracking: hover, focus, active state changes and style recomputation.

use crate::RinchDocument;
use crate::node::DirtyFlags;

impl RinchDocument {
    /// Update hover state: set the hovered node and its ancestors as hovered,
    /// clear previous hover, and invalidate styles only for nodes that Stylo
    /// flagged as hover-sensitive during selector matching.
    ///
    /// Returns `true` if a repaint is needed (some hover-sensitive node changed
    /// state). The `hovered_changed` out-parameter is set when the hovered node
    /// identity changed (used by the caller to dispatch `data-onenter`
    /// regardless of whether repaint is needed).
    pub fn update_hover(&mut self, new_hovered: Option<usize>, hovered_changed: &mut bool) -> bool {
        let old_hovered = self.tree.hovered_node;
        *hovered_changed = old_hovered != new_hovered;
        if !*hovered_changed {
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

        // Collect nodes whose hover state changed (symmetric difference).
        // Only invalidate styles for nodes where Stylo's selector matching
        // actually evaluated `:hover` — meaning some CSS rule depends on
        // this node's hover state. The `hover_sensitive` flag is set by
        // `match_non_ts_pseudo_class` during style resolution.
        let mut needs_repaint = false;
        for &id in &old_chain {
            if new_chain.contains(&id) {
                continue;
            }
            if self.node_is_hover_sensitive(id) {
                self.invalidate_hover_node(id);
                needs_repaint = true;
            }
        }
        for &id in &new_chain {
            if old_chain.contains(&id) {
                continue;
            }
            if self.node_is_hover_sensitive(id) {
                self.invalidate_hover_node(id);
                needs_repaint = true;
            }
        }

        if needs_repaint {
            self.tree.styles_dirty = true;
        }

        needs_repaint
    }

    fn node_is_hover_sensitive(&self, id: usize) -> bool {
        self.tree
            .nodes
            .get(id)
            .is_some_and(|n| n.hover_sensitive.get())
    }

    fn node_is_active_sensitive(&self, id: usize) -> bool {
        self.tree
            .nodes
            .get(id)
            .is_some_and(|n| n.active_sensitive.get())
    }

    fn node_is_focus_sensitive(&self, id: usize) -> bool {
        self.tree
            .nodes
            .get(id)
            .is_some_and(|n| n.focus_sensitive.get())
    }

    fn invalidate_hover_node(&mut self, id: usize) {
        if let Some(node) = self.tree.nodes.get(id) {
            *node.stylo_element_data.borrow_mut() = None;
        }
        self.tree.style_roots.push(id);
        self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
    }

    /// Update focus state: set the focused node, clear previous focus,
    /// and invalidate styles only for nodes that Stylo flagged as
    /// focus-sensitive during selector matching.
    ///
    /// Returns `true` if a repaint is needed.
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

        // Only invalidate nodes that have CSS rules depending on :focus.
        let mut needs_repaint = false;
        if let Some(id) = old_focused {
            if self.node_is_focus_sensitive(id) {
                self.invalidate_hover_node(id);
                needs_repaint = true;
            }
        }
        if let Some(id) = new_focused {
            if self.node_is_focus_sensitive(id) {
                self.invalidate_hover_node(id);
                needs_repaint = true;
            }
        }

        if needs_repaint {
            self.tree.styles_dirty = true;
        }

        needs_repaint
    }

    /// Update active (mouse-pressed) state: set the active node and its
    /// ancestors, clear previous active, and invalidate styles only for
    /// nodes that Stylo flagged as active-sensitive during selector matching.
    ///
    /// Returns `true` if a repaint is needed.
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

        // Only invalidate nodes whose active state changed AND that have
        // CSS rules depending on :active.
        let mut needs_repaint = false;
        for &id in &old_chain {
            if old_chain.contains(&id) && new_chain.contains(&id) {
                continue;
            }
            if self.node_is_active_sensitive(id) {
                self.invalidate_hover_node(id);
                needs_repaint = true;
            }
        }
        for &id in &new_chain {
            if old_chain.contains(&id) {
                continue;
            }
            if self.node_is_active_sensitive(id) {
                self.invalidate_hover_node(id);
                needs_repaint = true;
            }
        }

        if needs_repaint {
            self.tree.styles_dirty = true;
        }

        needs_repaint
    }
}
