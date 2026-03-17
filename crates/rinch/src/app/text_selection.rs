//! Read-only text selection for `user-select: text` elements.

use super::*;

impl RinchApp {
    /// Find the IFC root node (block element with an inline layout) at or
    /// above `hit_id` that has `user_select.is_selectable()`.
    pub(super) fn find_selectable_ifc(tree: &rinch_dom::NodeTree, hit_id: usize) -> Option<usize> {
        let mut check = Some(hit_id);
        while let Some(nid) = check {
            if let Some(node) = tree.get(nid) {
                if node.computed_style.user_select.is_selectable() && node.text_layout.is_some() {
                    return Some(nid);
                }
                check = node.parent;
            } else {
                break;
            }
        }
        None
    }

    /// Compute a byte offset into an IFC layout from a click position.
    pub(super) fn compute_ifc_offset_from_click(
        tree: &rinch_dom::NodeTree,
        ifc_node_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> usize {
        let Some(node) = tree.get(ifc_node_id) else {
            return 0;
        };
        let Some(ref inline_layout) = node.text_layout else {
            return 0;
        };
        let (abs_x, abs_y) = Self::compute_absolute_position(tree, ifc_node_id);
        let padding_left = node.computed_style.padding_left.to_px();
        let padding_top = node.computed_style.padding_top.to_px();
        let border_left = node.computed_style.border_left_width.to_px();
        let border_top = node.computed_style.border_top_width.to_px();
        let rel_x = click_x - abs_x - padding_left - border_left + node.scroll_offset.0 as f32;
        let rel_y = click_y - abs_y - padding_top - border_top + node.scroll_offset.1 as f32;
        byte_offset_from_position(&inline_layout.layout, rel_x, rel_y)
    }

    /// Set data attributes on the IFC node for paint to read.
    pub(super) fn set_text_selection_attributes(
        &self,
        ifc_node_id: usize,
        anchor: usize,
        focus: usize,
    ) {
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(ifc_node_id) {
                if anchor != focus {
                    let sel_start = anchor.min(focus);
                    let sel_end = anchor.max(focus);
                    node.attributes
                        .insert("data-text-sel".to_string(), "true".to_string());
                    node.attributes
                        .insert("data-text-sel-start".to_string(), sel_start.to_string());
                    node.attributes
                        .insert("data-text-sel-end".to_string(), sel_end.to_string());
                } else {
                    node.attributes.remove("data-text-sel");
                    node.attributes.remove("data-text-sel-start");
                    node.attributes.remove("data-text-sel-end");
                }
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
            }
        }
    }

    /// Clear the current text selection state and DOM attributes.
    pub(super) fn clear_text_selection(&mut self) {
        if let Some(sel) = self.text_selection.take() {
            // Inline the attribute clearing to avoid borrowing self.doc through &self
            if let Some(doc) = &self.doc {
                let mut d = doc.borrow_mut();
                if let Some(node) = d.tree.nodes.get_mut(sel.ifc_node_id) {
                    node.attributes.remove("data-text-sel");
                    node.attributes.remove("data-text-sel-start");
                    node.attributes.remove("data-text-sel-end");
                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                }
                d.tree.dirty_nodes.insert(sel.ifc_node_id);
            }
            self.scene_dirty = true;
        }
        self.text_selecting = false;
    }

    /// Copy the currently selected text to the clipboard.
    pub(super) fn copy_text_selection(&self) {
        let Some(ref sel) = self.text_selection else {
            return;
        };
        if sel.anchor_offset == sel.focus_offset {
            return;
        }
        let Some(doc) = &self.doc else {
            return;
        };
        let d = doc.borrow();
        let Some(node) = d.tree.get(sel.ifc_node_id) else {
            return;
        };
        let Some(ref inline_layout) = node.text_layout else {
            return;
        };
        let start = sel.anchor_offset.min(sel.focus_offset);
        let end = sel.anchor_offset.max(sel.focus_offset);
        let text = &inline_layout.text_content;
        let selected = &text[start.min(text.len())..end.min(text.len())];
        if !selected.is_empty() {
            #[cfg(feature = "clipboard")]
            {
                let _ = crate::clipboard::copy_text(selected);
            }
        }
    }
}
