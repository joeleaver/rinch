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
        // The click in the IFC root's own space — where Parley laid the text out
        // — rather than a window point minus a transform-blind parent-chain sum
        // (#203).
        let (local_x, local_y) = pointer_in_node(tree, ifc_node_id, click_x, click_y);
        let padding_left = node.computed_style.padding_left.to_px();
        let padding_top = node.computed_style.padding_top.to_px();
        let border_left = node.computed_style.border_left_width.to_px();
        let border_top = node.computed_style.border_top_width.to_px();
        let rel_x = local_x - padding_left - border_left + node.scroll_offset.0 as f32;
        let rel_y = local_y - padding_top - border_top + node.scroll_offset.1 as f32;
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
            }
            // The highlight is painted from these attributes on the IFC root,
            // so the root is the damage.
            d.tree.mark_paint_dirty(ifc_node_id);
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
                }
                // Where the highlight was painted is the damage.
                d.tree.mark_paint_dirty(sel.ifc_node_id);
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

#[cfg(test)]
mod tests {
    use super::RinchApp;
    use rinch_core::dom::{DomDocument, NodeId};
    use rinch_dom::RinchDocument;

    fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
        let el = doc.create_element(tag);
        doc.set_attribute(el, "style", style);
        doc.append_child(parent, el);
        el
    }

    fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) -> NodeId {
        let t = doc.create_text(text);
        doc.append_child(parent, t);
        t
    }

    /// #505 — `user_select` is on `ComputedStyle::for_anonymous_box`'s
    /// inherited copy list, justified in prose only: dropping it from the
    /// literal (falling through to `..Self::default()`) passed the entire
    /// workspace. `find_selectable_ifc` is its only consumer, and it requires
    /// **both** `user_select.is_selectable()` **and** `text_layout.is_some()`
    /// on the *same* node — which for an anonymous IFC root is exactly the
    /// struct `for_anonymous_box` builds.
    ///
    /// Sampling the drop at `user-select: none` would sit on a fixed point
    /// (CLAUDE.md "fixed-point blindness"): `UserSelectValue::default()` is
    /// `Auto`, and `Auto.is_selectable()` is `false` — the same answer `None`
    /// gives — so a dropped copy and a correct copy of `none` agree. `text`
    /// is the value that discriminates: copied, the anonymous root answers
    /// selectable; dropped, it silently falls back to `Auto` and does not,
    /// however the real parent declared it.
    ///
    /// Mixed content (a trailing block sibling) forces the text into an
    /// anonymous block box rather than the container's own IFC — the shape
    /// `for_anonymous_box` exists for.
    #[test]
    fn an_anonymous_roots_selectability_comes_from_its_parents_user_select() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = child_of(
            &mut doc,
            body,
            "div",
            "width: 400px; font-size: 16px; line-height: 20px; user-select: text",
        );
        let text = text_in(&mut doc, container, "select me");
        child_of(&mut doc, container, "div", "height: 20px");
        doc.resolve_layout(800.0, 600.0);

        let anon = doc
            .tree
            .get(text.0)
            .unwrap()
            .ifc_root
            .expect("mixed content must put the inline run in an anonymous block box");
        assert!(
            doc.tree.get(anon).unwrap().is_anonymous_block_box,
            "the IFC root must be the synthetic anonymous box, or this test \
             is not exercising the shape it claims to"
        );
        assert!(
            doc.tree.get(anon).unwrap().text_layout.is_some(),
            "the anonymous root must carry the IFC's inline layout, or \
             find_selectable_ifc's second condition is vacuous here"
        );

        // Hitting the anonymous root directly is what a real hit test
        // resolves to for text painted inside one — same node `paint` and
        // `ifc_content_box_offset` treat as the drawn box
        // (`ifc_anonymous_box_tests.rs`).
        assert_eq!(
            RinchApp::find_selectable_ifc(&doc.tree, anon),
            Some(anon),
            "the anonymous root must inherit `user-select: text` from its \
             real parent and answer selectable at itself — `None` here means \
             the copy list dropped `user_select` (it falls back to `Auto`, \
             not selectable), and no ancestor of an anonymous root both \
             carries the inline layout and a `user_select` to fall back on \
             (#505)"
        );
    }

    /// The fixed-point control named above: `none` does not discriminate,
    /// and both the real and the hypothetically-dropped copy must agree it
    /// is not selectable — matching what a real (non-anonymous) root with
    /// the same declaration does.
    #[test]
    fn user_select_none_is_not_selectable_inside_or_outside_an_anonymous_box() {
        let mut mixed = RinchDocument::new();
        let body = mixed.body();
        let container = child_of(
            &mut mixed,
            body,
            "div",
            "width: 400px; font-size: 16px; line-height: 20px; user-select: none",
        );
        let text = text_in(&mut mixed, container, "not selectable");
        child_of(&mut mixed, container, "div", "height: 20px");
        mixed.resolve_layout(800.0, 600.0);
        let anon = mixed.tree.get(text.0).unwrap().ifc_root.unwrap();
        assert_eq!(RinchApp::find_selectable_ifc(&mixed.tree, anon), None);

        let mut plain = RinchDocument::new();
        let body = plain.body();
        let container = child_of(
            &mut plain,
            body,
            "div",
            "width: 400px; font-size: 16px; line-height: 20px; user-select: none",
        );
        text_in(&mut plain, container, "not selectable either");
        plain.resolve_layout(800.0, 600.0);
        assert_eq!(
            RinchApp::find_selectable_ifc(&plain.tree, container.0),
            None,
            "a real root with the same declaration must agree"
        );
    }
}
