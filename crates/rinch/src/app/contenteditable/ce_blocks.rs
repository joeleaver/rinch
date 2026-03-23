use super::*;

#[allow(dead_code)]
impl RinchApp {
    // ── Block manipulation ───────────────────────────────────────────────

    /// Change a block element's tag while preserving children and attributes.
    /// Returns the new node's `NodeId`.
    pub(crate) fn convert_block_tag(
        d: &mut RinchDocument,
        block_id: usize,
        new_tag: &str,
    ) -> rinch_core::dom::NodeId {
        let old_tag = d
            .tree
            .get(block_id)
            .and_then(|n| n.tag())
            .unwrap_or("")
            .to_string();
        let new_el = d.create_element(new_tag);
        // Copy style/class attributes
        if let Some(style) = d
            .tree
            .get(block_id)
            .and_then(|n| n.attributes.get("style"))
            .cloned()
        {
            // When converting heading → non-heading, strip heading-specific CSS properties
            // (font-size, font-weight) so the div reverts to normal text styling
            let style = if Self::is_heading(&old_tag) && !Self::is_heading(new_tag) {
                Self::strip_css_properties(&style, &["font-size", "font-weight"])
            } else {
                style
            };
            if !style.trim().is_empty() {
                d.set_attribute(new_el, "style", &style);
            }
        }
        if let Some(class) = d
            .tree
            .get(block_id)
            .and_then(|n| n.attributes.get("class"))
            .cloned()
        {
            d.set_attribute(new_el, "class", &class);
        }
        // Move all children
        let children: Vec<usize> = d.tree.nodes[block_id].children.clone();
        for &child_id in &children {
            d.remove_node(rinch_core::dom::NodeId(child_id));
            d.append_child(new_el, rinch_core::dom::NodeId(child_id));
        }
        // Replace in parent: insert new element at same position, then remove old
        let parent_id = d.tree.get(block_id).and_then(|n| n.parent).unwrap_or(0);
        let next_sib = {
            let siblings = &d.tree.nodes[parent_id].children;
            let pos = siblings.iter().position(|&c| c == block_id);
            pos.and_then(|p| siblings.get(p + 1).copied())
        };
        if let Some(next) = next_sib {
            d.insert_before(
                rinch_core::dom::NodeId(parent_id),
                new_el,
                rinch_core::dom::NodeId(next),
            );
        } else {
            d.append_child(rinch_core::dom::NodeId(parent_id), new_el);
        }
        d.remove_node(rinch_core::dom::NodeId(block_id));
        new_el
    }

    /// Outdent a `<li>` from its parent list: convert to `<div>`, split list if needed.
    /// Works for any position (first, middle, last).
    /// Returns the new `<div>` node id.
    pub(crate) fn outdent_li(
        d: &mut RinchDocument,
        li_id: usize,
        list_id: usize,
        ce_root: usize,
    ) -> rinch_core::dom::NodeId {
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
            .unwrap_or(ce_root);
        let grandparent_tag = d
            .tree
            .get(grandparent_id)
            .and_then(|n| n.tag())
            .unwrap_or("")
            .to_string();

        // If nested (parent <ul> is inside another <li>), move <li> up one level
        // like Shift+Tab. Only convert to <div> when at the top level.
        if grandparent_tag == "li" {
            let parent_li_id = grandparent_id;
            let outer_list_id = d
                .tree
                .get(parent_li_id)
                .and_then(|n| n.parent)
                .unwrap_or(ce_root);

            // Collect siblings after current <li> in the nested list
            let nested_siblings = d.tree.nodes[list_id].children.clone();
            let pos = nested_siblings
                .iter()
                .position(|&c| c == li_id)
                .unwrap_or(0);
            let after_siblings: Vec<usize> = nested_siblings[pos + 1..].to_vec();

            // Move current <li> to after parent_li in the outer list
            d.remove_node(rinch_core::dom::NodeId(li_id));
            let parent_li_next = {
                let siblings = &d.tree.nodes[outer_list_id].children;
                let ppos = siblings.iter().position(|&c| c == parent_li_id);
                ppos.and_then(|p| siblings.get(p + 1).copied())
            };
            if let Some(next) = parent_li_next {
                d.insert_before(
                    rinch_core::dom::NodeId(outer_list_id),
                    rinch_core::dom::NodeId(li_id),
                    rinch_core::dom::NodeId(next),
                );
            } else {
                d.append_child(
                    rinch_core::dom::NodeId(outer_list_id),
                    rinch_core::dom::NodeId(li_id),
                );
            }

            // If there are siblings after, create new nested list under current li
            if !after_siblings.is_empty() {
                let new_nested = d.create_element(&list_tag);
                for &sib_id in &after_siblings {
                    d.remove_node(rinch_core::dom::NodeId(sib_id));
                    d.append_child(new_nested, rinch_core::dom::NodeId(sib_id));
                }
                d.append_child(rinch_core::dom::NodeId(li_id), new_nested);
            }

            // If the original nested list is now empty, remove it
            if d.tree.nodes[list_id].children.is_empty() {
                d.remove_node(rinch_core::dom::NodeId(list_id));
            }

            return rinch_core::dom::NodeId(li_id);
        }

        // Top-level: convert <li> to <div> and remove from list

        // Get position and collect siblings after this <li>
        let siblings = d.tree.nodes[list_id].children.clone();
        let pos = siblings.iter().position(|&c| c == li_id).unwrap_or(0);
        let after_siblings: Vec<usize> = siblings[pos + 1..].to_vec();

        // Convert <li> to <div>
        let new_el = Self::convert_block_tag(d, li_id, "div");
        // convert_block_tag replaces in parent, so new_el is now a child of list_id.
        // Remove it from the list.
        d.remove_node(new_el);

        if pos == 0 {
            // First item: insert <div> before the list
            d.insert_before(
                rinch_core::dom::NodeId(grandparent_id),
                new_el,
                rinch_core::dom::NodeId(list_id),
            );
        } else {
            // Non-first: insert <div> after the list
            // Find what comes after list_id in grandparent
            let gp_children = d.tree.nodes[grandparent_id].children.clone();
            let list_pos = gp_children.iter().position(|&c| c == list_id);
            let next_after_list = list_pos.and_then(|p| gp_children.get(p + 1).copied());
            if let Some(next_id) = next_after_list {
                d.insert_before(
                    rinch_core::dom::NodeId(grandparent_id),
                    new_el,
                    rinch_core::dom::NodeId(next_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(grandparent_id), new_el);
            }
        }

        // If there are siblings after, move them to a new list after the <div>
        if !after_siblings.is_empty() {
            let new_list = d.create_element(&list_tag);
            // Copy list style if any
            if let Some(style) = d
                .tree
                .get(list_id)
                .and_then(|n| n.attributes.get("style"))
                .cloned()
            {
                d.set_attribute(new_list, "style", &style);
            }
            for &sib_id in &after_siblings {
                d.remove_node(rinch_core::dom::NodeId(sib_id));
                d.append_child(new_list, rinch_core::dom::NodeId(sib_id));
            }
            // Insert new list after the <div>
            let gp_children = d.tree.nodes[grandparent_id].children.clone();
            let div_pos = gp_children.iter().position(|&c| c == new_el.0);
            let next_after_div = div_pos.and_then(|p| gp_children.get(p + 1).copied());
            if let Some(next_id) = next_after_div {
                d.insert_before(
                    rinch_core::dom::NodeId(grandparent_id),
                    new_list,
                    rinch_core::dom::NodeId(next_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(grandparent_id), new_list);
            }
        }

        // If original list is now empty, remove it
        if d.tree.nodes[list_id].children.is_empty() {
            d.remove_node(rinch_core::dom::NodeId(list_id));
        }

        new_el
    }

    /// Walk up from `node_id` to find the nearest block-level ancestor
    /// and its parent. Stops at `ce_root_id` (the contenteditable element).
    /// Returns `(block_element_id, parent_of_block_id)`.
    pub(crate) fn find_block_and_parent(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        ce_root_id: usize,
    ) -> Option<(usize, usize)> {
        // Never return the CE root itself as a block — it would be removed
        if node_id == ce_root_id {
            return None;
        }
        let mut current = node_id;
        loop {
            let parent = tree.get(current)?.parent?;
            let is_block = tree
                .get(current)?
                .tag()
                .map(Self::is_block_element)
                .unwrap_or(false);
            // Skip anonymous block boxes — they're layout-internal wrappers
            // that editing operations should see through transparently.
            let is_anon = tree
                .get(current)
                .map(|n| n.is_anonymous_block_box)
                .unwrap_or(false);

            if parent == ce_root_id {
                if is_block && !is_anon {
                    return Some((current, parent));
                }
                return None;
            }
            if is_block && !is_anon {
                return Some((current, parent));
            }
            current = parent;
        }
    }

    /// Walk up from a block to find a containing `<li>` whose parent is a list.
    /// This handles the case where `find_block_and_parent` returns a wrapper `<div>`
    /// (created by Tab indent) inside an `<li>` — we want to outdent the `<li>`, not
    /// merge the `<div>` with the previous block.
    pub(crate) fn find_li_ancestor_for_outdent(
        tree: &rinch_dom::NodeTree,
        block_id: usize,
        ce_root: usize,
    ) -> Option<(usize, usize)> {
        let mut current = tree.get(block_id)?.parent?;
        while current != ce_root {
            let tag = tree.get(current)?.tag().unwrap_or("");
            if tag == "li" {
                let parent = tree.get(current)?.parent?;
                let parent_tag = tree.get(parent)?.tag().unwrap_or("");
                if Self::is_list_tag(parent_tag) {
                    return Some((current, parent)); // (li_id, list_id)
                }
            }
            current = tree.get(current)?.parent?;
        }
        None
    }
}
