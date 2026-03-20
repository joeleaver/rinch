use super::*;

impl RinchApp {
    // ── Selection helpers ────────────────────────────────────────────────

    /// Delete the current selection, updating the CE cursor.
    /// Fires `TextDeleted` / `NodeRemoved` CE events for each mutation.
    pub(crate) fn ce_delete_selection(&mut self) {
        let ce = match self.focused_contenteditable.as_mut() {
            Some(ce) => ce,
            None => return,
        };
        let ce_node_id = ce.ce_node_id;
        let cursor = ce.cursor;
        let anchor = ce.anchor;
        if cursor == anchor {
            return;
        }

        if let Some(doc) = &self.doc {
            // Determine document order (start, end)
            let (start, end) = Self::order_cursors(&doc.borrow().tree, ce_node_id, cursor, anchor);

            if start.node_id == end.node_id {
                // Same node — simple substring removal
                let delete_len;
                {
                    let mut d = doc.borrow_mut();
                    if let Some(node) = d.tree.get(start.node_id)
                        && let Some(text) = node.text_content().map(|s| s.to_string())
                    {
                        let s = start.offset.min(text.len());
                        let e = end.offset.min(text.len());
                        delete_len = e - s;
                        let mut new_text = String::with_capacity(text.len() - delete_len);
                        new_text.push_str(&text[..s]);
                        new_text.push_str(&text[e..]);
                        d.set_text_content(rinch_core::dom::NodeId(start.node_id), &new_text);
                    } else {
                        delete_len = 0;
                    }
                }
                if delete_len > 0 {
                    rinch_core::ce::dispatch_ce_event(&rinch_core::ce::CeEvent::TextDeleted {
                        node_id: start.node_id,
                        offset: start.offset,
                        length: delete_len,
                    });
                }
                let ce = self.focused_contenteditable.as_mut().unwrap();
                ce.cursor = start;
                ce.anchor = start;
                // Don't return yet — cleanup_empty_cursor_node runs at the end
            } else {
                // Cross-node deletion: truncate start, remove middle, truncate end, merge
                let mut all_text = Vec::new();
                let start_is_text;
                let end_is_text;
                let start_remaining;
                let end_remaining;
                let start_orig_len;
                {
                    let d = doc.borrow();
                    Self::collect_text_node_ids(&d.tree, ce_node_id, &mut all_text);
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
                    start_orig_len = if start_is_text {
                        d.tree
                            .get(start.node_id)
                            .and_then(|n| n.text_content())
                            .map(|t| t.len())
                            .unwrap_or(0)
                    } else {
                        0
                    };
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
                }
                let start_pos = all_text
                    .iter()
                    .position(|&id| id == start.node_id)
                    .unwrap_or(0);
                let end_pos = all_text
                    .iter()
                    .position(|&id| id == end.node_id)
                    .unwrap_or(all_text.len());

                let merged = format!("{}{}", start_remaining, end_remaining);
                let new_cursor;
                // Collect removed node IDs and parent for NodeRemoved events
                let mut removed_ids: Vec<(usize, usize)> = Vec::new();

                {
                    let mut d = doc.borrow_mut();
                    if start_is_text {
                        // Start is a text node — merge into it, remove middle + end
                        d.set_text_content(rinch_core::dom::NodeId(start.node_id), &merged);
                        for &mid_id in &all_text[start_pos + 1..=end_pos] {
                            let parent = d.tree.get(mid_id).and_then(|n| n.parent).unwrap_or(0);
                            d.remove_node(rinch_core::dom::NodeId(mid_id));
                            removed_ids.push((mid_id, parent));
                        }
                        new_cursor = DomCursor {
                            node_id: start.node_id,
                            offset: start.offset,
                        };
                    } else if end_is_text {
                        // Start is element cursor, end is text — remove start + middle, truncate end
                        d.set_text_content(rinch_core::dom::NodeId(end.node_id), &end_remaining);
                        for &mid_id in &all_text[start_pos..end_pos] {
                            let parent = d.tree.get(mid_id).and_then(|n| n.parent).unwrap_or(0);
                            d.remove_node(rinch_core::dom::NodeId(mid_id));
                            removed_ids.push((mid_id, parent));
                        }
                        new_cursor = DomCursor {
                            node_id: end.node_id,
                            offset: 0,
                        };
                    } else {
                        // Both are element cursors — remove everything between them
                        for &mid_id in &all_text[start_pos..=end_pos] {
                            let parent = d.tree.get(mid_id).and_then(|n| n.parent).unwrap_or(0);
                            d.remove_node(rinch_core::dom::NodeId(mid_id));
                            removed_ids.push((mid_id, parent));
                        }
                        // Find a valid cursor: previous text node or first in CE
                        let prev_target = if start_pos > 0 {
                            let prev_id = all_text[start_pos - 1];
                            let len = d
                                .tree
                                .get(prev_id)
                                .and_then(|n| n.text_content())
                                .map(|t| t.len())
                                .unwrap_or(0);
                            Some(DomCursor {
                                node_id: prev_id,
                                offset: len,
                            })
                        } else {
                            Self::first_text_cursor(&d.tree, ce_node_id)
                        };
                        new_cursor = prev_target.unwrap_or(DomCursor {
                            node_id: ce_node_id,
                            offset: 0,
                        });
                    }
                }

                // Fire CE events for the mutations
                use rinch_core::ce::{CeEvent, dispatch_ce_event};
                if start_is_text {
                    // Text deleted from start node: from start.offset to original end
                    let deleted_from_start = start_orig_len - start_remaining.len();
                    if deleted_from_start > 0 {
                        dispatch_ce_event(&CeEvent::TextDeleted {
                            node_id: start.node_id,
                            offset: start.offset,
                            length: deleted_from_start,
                        });
                    }
                } else if end_is_text {
                    dispatch_ce_event(&CeEvent::TextDeleted {
                        node_id: end.node_id,
                        offset: 0,
                        length: end.offset,
                    });
                }
                for (node_id, parent_id) in removed_ids {
                    dispatch_ce_event(&CeEvent::NodeRemoved { node_id, parent_id });
                }

                let ce = self.focused_contenteditable.as_mut().unwrap();
                ce.cursor = new_cursor;
                ce.anchor = new_cursor;
            }
        }
        // Clean up empty text nodes (they break IFC navigation)
        self.cleanup_empty_cursor_node();
    }

    /// If the cursor is on an empty text node, move it to an adjacent sibling
    /// and remove the empty node.  Empty text nodes have no IfcTextRange and
    /// break IFC-based navigation (up/down).
    pub(crate) fn cleanup_empty_cursor_node(&mut self) {
        let ce = match self.focused_contenteditable.as_ref() {
            Some(ce) => ce,
            None => return,
        };
        let cur = ce.cursor;
        let Some(doc) = &self.doc else { return };
        let needs_cleanup = {
            let d = doc.borrow();
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
            let d = doc.borrow();
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
                            sibling_cursor = Some(DomCursor {
                                node_id: next,
                                offset: 0,
                            });
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
                            sibling_cursor = Some(DomCursor {
                                node_id: prev_sib,
                                offset: 0,
                            });
                        } else if let Some(tc) = d.tree.get(prev_sib).and_then(|n| n.text_content())
                        {
                            sibling_cursor = Some(DomCursor {
                                node_id: prev_sib,
                                offset: tc.len(),
                            });
                        }
                    }
                }
            }
        }
        if let Some(sc) = sibling_cursor {
            let mut d = doc.borrow_mut();
            d.remove_node(rinch_core::dom::NodeId(cur.node_id));
            let ce = self.focused_contenteditable.as_mut().unwrap();
            ce.cursor = sc;
            ce.anchor = sc;
        }
    }

    /// Order two cursors into (start, end) in document order.
    pub(crate) fn order_cursors(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        a: DomCursor,
        b: DomCursor,
    ) -> (DomCursor, DomCursor) {
        if a.node_id == b.node_id {
            return if a.offset <= b.offset { (a, b) } else { (b, a) };
        }
        // Walk document order to determine which comes first
        let mut all_text = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_text);
        let a_pos = all_text.iter().position(|&id| id == a.node_id);
        let b_pos = all_text.iter().position(|&id| id == b.node_id);
        match (a_pos, b_pos) {
            (Some(ap), Some(bp)) if ap <= bp => (a, b),
            _ => (b, a),
        }
    }

    /// Extract text between two cursors (for copy/cut).
    #[allow(dead_code)]
    pub(crate) fn extract_selection_text(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        anchor: DomCursor,
        cursor: DomCursor,
    ) -> String {
        let (start, end) = Self::order_cursors(tree, ce_root, anchor, cursor);

        if start.node_id == end.node_id {
            if let Some(node) = tree.get(start.node_id)
                && let Some(text) = node.text_content()
            {
                let s = start.offset.min(text.len());
                let e = end.offset.min(text.len());
                return text[s..e].to_string();
            }
            return String::new();
        }

        let mut all_text = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_text);
        let start_pos = all_text
            .iter()
            .position(|&id| id == start.node_id)
            .unwrap_or(0);
        let end_pos = all_text
            .iter()
            .position(|&id| id == end.node_id)
            .unwrap_or(all_text.len());

        let mut result = String::new();
        for &nid in &all_text[start_pos..=end_pos.min(all_text.len() - 1)] {
            if let Some(node) = tree.get(nid) {
                if node.tag() == Some("br") {
                    result.push('\n');
                } else if node.is_element()
                    && node.children.is_empty()
                    && node.tag().map(Self::is_block_element).unwrap_or(false)
                {
                    // Empty block element — represents a blank line
                    result.push('\n');
                } else if let Some(text) = node.text_content() {
                    if nid == start.node_id {
                        result.push_str(&text[start.offset.min(text.len())..]);
                    } else if nid == end.node_id {
                        result.push_str(&text[..end.offset.min(text.len())]);
                    } else {
                        result.push_str(text);
                    }
                }
            }
        }
        result
    }

    /// Extract HTML between two cursors (for copy/cut with rich formatting).
    ///
    /// Walks the DOM tree and serializes the selected range as an HTML fragment,
    /// preserving element tags, inline styles, and classes.
    #[allow(dead_code)]
    pub(crate) fn extract_selection_html(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        anchor: DomCursor,
        cursor: DomCursor,
    ) -> String {
        let (start, end) = Self::order_cursors(tree, ce_root, anchor, cursor);

        if start.node_id == end.node_id {
            // Single-node selection: just return the text slice (no wrapping tags needed
            // unless we want to preserve inline formatting of the parent)
            if let Some(node) = tree.get(start.node_id)
                && let Some(text) = node.text_content()
            {
                let s = start.offset.min(text.len());
                let e = end.offset.min(text.len());
                let slice = &text[s..e];
                // Wrap in ancestor inline formatting tags
                return Self::wrap_in_ancestor_tags(
                    tree,
                    start.node_id,
                    ce_root,
                    &html_escape(slice),
                );
            }
            return String::new();
        }

        // Collect all leaf (text/br/empty-block) node IDs in document order
        let mut all_leaves = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_leaves);
        let start_pos = all_leaves
            .iter()
            .position(|&id| id == start.node_id)
            .unwrap_or(0);
        let end_pos = all_leaves
            .iter()
            .position(|&id| id == end.node_id)
            .unwrap_or(all_leaves.len().saturating_sub(1));

        // Build a set of selected leaf IDs for fast lookup
        let selected: std::collections::HashSet<usize> = all_leaves
            [start_pos..=end_pos.min(all_leaves.len().saturating_sub(1))]
            .iter()
            .copied()
            .collect();

        // Recursively serialize the CE root's subtree, only including
        // branches that contain selected leaves
        let mut html = String::new();
        Self::serialize_html_range(tree, ce_root, &selected, start, end, &mut html);
        html
    }

    /// Recursively serialize a DOM subtree as HTML, including only branches
    /// that contain leaf nodes in the `selected` set.
    fn serialize_html_range(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        selected: &std::collections::HashSet<usize>,
        start: DomCursor,
        end: DomCursor,
        out: &mut String,
    ) {
        let Some(node) = tree.get(node_id) else {
            return;
        };

        // Text node
        if let Some(text) = node.text_content() {
            if !selected.contains(&node_id) {
                return;
            }
            let text_str = if node_id == start.node_id && node_id == end.node_id {
                &text[start.offset.min(text.len())..end.offset.min(text.len())]
            } else if node_id == start.node_id {
                &text[start.offset.min(text.len())..]
            } else if node_id == end.node_id {
                &text[..end.offset.min(text.len())]
            } else {
                text
            };
            out.push_str(&html_escape(text_str));
            return;
        }

        // <br> element
        if node.tag() == Some("br") {
            if selected.contains(&node_id) {
                out.push_str("<br>");
            }
            return;
        }

        // Empty block element (cursor placeholder)
        if node.children.is_empty()
            && node.tag().map(Self::is_block_element).unwrap_or(false)
            && selected.contains(&node_id)
        {
            if let Some(tag) = node.tag() {
                out.push('<');
                out.push_str(tag);
                out.push('>');
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
            return;
        }

        // Element node with children — only include if a descendant is selected
        if let Some(tag) = node.tag() {
            // Check if any descendant leaf is in the selection
            if !Self::has_selected_descendant(tree, node_id, selected) {
                return;
            }

            // Don't emit the CE root tag itself, just its children
            if node_id == start.node_id && node.is_element() && node.children.is_empty() {
                // Element cursor (empty block) — already handled above
                return;
            }

            // Skip the contenteditable root wrapper — emit children directly
            let emit_tag =
                node_id != start.node_id || !node.is_element() || !selected.contains(&node_id);
            // Actually, never emit the CE root tag — it's the container, not content.
            // We always want to emit tags for elements inside the CE root though.
            let emit_tag = emit_tag && node.tag().is_some();
            // Skip anonymous/internal nodes
            let emit_tag = emit_tag && !node.is_anonymous_block_box;

            if emit_tag {
                out.push('<');
                out.push_str(tag);
                // Include style and class attributes
                Self::emit_html_attributes(node, out);
                out.push('>');
            }

            for &child_id in &node.children {
                // Skip anonymous block boxes
                if let Some(child) = tree.get(child_id)
                    && child.is_anonymous_block_box
                {
                    // Recurse into anonymous box children directly
                    for &grandchild_id in &child.children {
                        Self::serialize_html_range(tree, grandchild_id, selected, start, end, out);
                    }
                    continue;
                }
                Self::serialize_html_range(tree, child_id, selected, start, end, out);
            }

            if emit_tag && !Self::is_void_element(tag) {
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
        } else {
            // Document or other non-element — recurse into children
            for &child_id in &node.children {
                Self::serialize_html_range(tree, child_id, selected, start, end, out);
            }
        }
    }

    /// Check if any descendant leaf of `node_id` is in the selected set.
    fn has_selected_descendant(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        selected: &std::collections::HashSet<usize>,
    ) -> bool {
        if selected.contains(&node_id) {
            return true;
        }
        let Some(node) = tree.get(node_id) else {
            return false;
        };
        for &child_id in &node.children {
            if Self::has_selected_descendant(tree, child_id, selected) {
                return true;
            }
        }
        false
    }

    /// Emit safe HTML attributes (style, class) for an element.
    fn emit_html_attributes(node: &rinch_dom::node::Node, out: &mut String) {
        // Include style attribute if present
        if let Some(style) = node.attributes.get("style")
            && !style.is_empty()
        {
            out.push_str(" style=\"");
            out.push_str(&html_escape_attr(style));
            out.push('"');
        }
        // Include class attribute if present
        if let Some(class) = node.attributes.get("class")
            && !class.is_empty()
        {
            out.push_str(" class=\"");
            out.push_str(&html_escape_attr(class));
            out.push('"');
        }
    }

    /// Whether an HTML element is a void element (self-closing, no end tag).
    fn is_void_element(tag: &str) -> bool {
        matches!(tag, "br" | "hr" | "img" | "input" | "meta" | "link" | "wbr")
    }

    /// Wrap a text string in the inline formatting tags of its ancestors,
    /// up to (but not including) the CE root.
    fn wrap_in_ancestor_tags(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        ce_root: usize,
        inner: &str,
    ) -> String {
        let mut tags = Vec::new();
        let mut current = node_id;
        // Walk up from the text node's parent to the CE root
        while let Some(node) = tree.get(current) {
            if let Some(parent_id) = node.parent {
                if parent_id == ce_root {
                    break;
                }
                if let Some(parent) = tree.get(parent_id)
                    && let Some(tag) = parent.tag()
                    && !parent.is_anonymous_block_box
                {
                    let mut opening = format!("<{}", tag);
                    Self::emit_html_attributes(parent, &mut opening);
                    opening.push('>');
                    tags.push((opening, format!("</{}>", tag)));
                }
                current = parent_id;
            } else {
                break;
            }
        }

        // Tags are innermost-first, we need outermost-first for wrapping
        tags.reverse();
        let mut result = String::new();
        for (open, _) in &tags {
            result.push_str(open);
        }
        result.push_str(inner);
        for (_, close) in tags.iter().rev() {
            result.push_str(close);
        }
        result
    }

    /// Calculate byte offset from click coordinates relative to text start.
    #[allow(dead_code)]
    fn byte_offset_from_xy(
        layout: &parley::layout::Layout<peniko::Brush>,
        click_x: f32,
        click_y: f32,
    ) -> usize {
        byte_offset_from_position(layout, click_x, click_y)
    }
}

/// Compute the scroll target for a contenteditable element so the cursor stays visible.
///
/// Returns `Some(new_scroll_y)` if scrolling is needed, `None` if already visible
/// or the element is not a scroll container.
pub(super) fn compute_ce_scroll_target(
    tree: &rinch_dom::NodeTree,
    ce_node_id: usize,
    cursor: DomCursor,
    cursor_off: usize,
) -> Option<f64> {
    use rinch_dom::computed_style::{LineHeightValue, OverflowValue};

    let node = tree.get(ce_node_id)?;

    // Only applies to scroll containers
    if !matches!(
        node.computed_style.overflow_y,
        OverflowValue::Auto | OverflowValue::Scroll
    ) {
        return None;
    }

    let current_scroll = node.scroll_offset.1;
    let visible_height = compute_visible_content_area_height(tree, ce_node_id);
    if visible_height <= 0.0 {
        return None;
    }

    let cs = &node.computed_style;

    // Helper: compute line height from a computed style
    let line_height = |cs: &rinch_dom::computed_style::ComputedStyle| -> f64 {
        let lh = match cs.line_height {
            LineHeightValue::Relative(r) => cs.font_size * r,
            LineHeightValue::Absolute(a) => a,
            LineHeightValue::Normal => cs.font_size * 1.2,
        };
        lh as f64
    };

    // Find the cursor's Y position and height relative to the content box.
    let (cursor_y, cursor_height) = if cursor.node_id == ce_node_id {
        // Cursor at the CE root itself (empty CE) — position 0
        (0.0_f64, line_height(cs))
    } else if let Some(ref inline_layout) = node.text_layout {
        // Inline CE with IFC layout — use caret position query
        let offset = cursor_off.min(inline_layout.text_content.len());
        let (_, y) = caret_position_for_offset_layout(&inline_layout.layout, offset);
        (y as f64, line_height(cs))
    } else {
        // Block CE — find which direct child of the CE root contains cursor.node_id
        let block_child_id = {
            let mut current = cursor.node_id;
            loop {
                match tree.get(current) {
                    Some(n) if n.parent == Some(ce_node_id) => break Some(current),
                    Some(n) => match n.parent {
                        Some(p) => current = p,
                        None => break None,
                    },
                    None => break None,
                }
            }
        };

        let child_id = block_child_id?;
        let child = tree.get(child_id)?;

        // Try to get line-level precision within the block using its text layout
        let child_pad_top = child.computed_style.padding_top.to_px() as f64;
        if let Some(ref text_layout) = child.text_layout {
            // Compute local offset within this child's IFC
            // Walk prior siblings to find accumulated offset
            let mut accumulated = 0usize;
            let mut first_block = true;
            for &sib_id in &node.children {
                if sib_id == child_id {
                    break;
                }
                if !first_block {
                    accumulated += 1; // \n separator
                }
                first_block = false;
                accumulated += flat_text_len_for_subtree(tree, sib_id);
            }
            if !first_block {
                accumulated += 1; // \n before this block
            }
            let local_offset = cursor_off.saturating_sub(accumulated);
            let clamped = local_offset.min(text_layout.text_content.len());
            let (_, y) = caret_position_for_offset_layout(&text_layout.layout, clamped);
            (
                child.layout.y as f64 + child_pad_top + y as f64,
                line_height(&child.computed_style),
            )
        } else {
            // Fallback: use the child block's full layout bounds
            (child.layout.y as f64, child.layout.height as f64)
        }
    };

    // Determine if scrolling is needed
    let margin = 4.0_f64;
    let new_scroll = if cursor_y < current_scroll + margin {
        // Cursor above visible area — scroll up
        (cursor_y - margin).max(0.0)
    } else if cursor_y + cursor_height > current_scroll + visible_height - margin {
        // Cursor below visible area — scroll down
        cursor_y + cursor_height - visible_height + margin
    } else {
        return None; // already visible
    };

    // Clamp to valid range
    let content_height = compute_content_height(tree, ce_node_id);
    let max_scroll = (content_height - visible_height).max(0.0);
    Some(new_scroll.clamp(0.0, max_scroll))
}

/// Compute the flat text length for a subtree (for scroll offset calculation).
/// Matches the logic in paint.rs's `get_flat_text_len`.
fn flat_text_len_for_subtree(tree: &rinch_dom::NodeTree, node_id: usize) -> usize {
    let mut len = 0usize;
    let mut ends_with_newline = false;
    flat_text_len_recursive(tree, node_id, &mut len, &mut ends_with_newline);
    if ends_with_newline && len > 0 {
        len -= 1;
    }
    len
}

fn flat_text_len_recursive(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    len: &mut usize,
    ends_with_newline: &mut bool,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };
    if let Some(t) = node.text_content() {
        *len += t.len();
        *ends_with_newline = t.ends_with('\n');
    } else if node.tag() == Some("br") {
        *len += 1;
        *ends_with_newline = true;
    } else {
        let is_block = node
            .tag()
            .map(|t| {
                matches!(
                    t,
                    "div"
                        | "p"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "ul"
                        | "ol"
                        | "li"
                        | "blockquote"
                        | "section"
                        | "article"
                        | "pre"
                        | "hr"
                        | "table"
                        | "tr"
                        | "header"
                        | "footer"
                        | "main"
                        | "nav"
                        | "aside"
                        | "figure"
                        | "figcaption"
                        | "details"
                        | "summary"
                )
            })
            .unwrap_or(false);
        if is_block && !*ends_with_newline && *len > 0 {
            *len += 1;
            *ends_with_newline = true;
        }
        for &child_id in &node.children {
            flat_text_len_recursive(tree, child_id, len, ends_with_newline);
        }
    }
}
