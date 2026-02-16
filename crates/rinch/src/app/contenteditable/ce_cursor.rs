use super::*;

impl RinchApp {
    // ── Cursor / position helpers ────────────────────────────────────────

    /// Compute the absolute position of a node by walking up through parents.
    pub(super) fn compute_absolute_position(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
    ) -> (f32, f32) {
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut current = Some(node_id);
        while let Some(nid) = current {
            if let Some(node) = tree.get(nid) {
                x += node.layout.x;
                y += node.layout.y;
                if let Some(parent_id) = node.parent
                    && let Some(parent) = tree.get(parent_id)
                {
                    x -= parent.scroll_offset.0 as f32;
                    y -= parent.scroll_offset.1 as f32;
                }
                current = node.parent;
            } else {
                break;
            }
        }
        (x, y)
    }

    /// Convert a `DomCursor` to a global flat byte offset within the CE element.
    ///
    /// Walks the DOM tree depth-first from `ce_root`, accumulating text lengths
    /// and block separators until reaching `cursor.node_id`, then adds
    /// `cursor.offset`.  Used only for writing paint attributes.
    pub(super) fn dom_cursor_to_global_offset(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
    ) -> usize {
        let mut offset = 0usize;
        let mut found = false;
        let mut ends_with_newline = false;
        Self::walk_for_global_offset(
            tree,
            ce_root,
            cursor,
            &mut offset,
            &mut found,
            &mut ends_with_newline,
        );
        offset
    }

    /// Recursive helper for `dom_cursor_to_global_offset`.
    ///
    /// Uses the same `ends_with_newline` deduplication as paint.rs's
    /// `collect_text_len_recursive` to ensure global offsets are consistent
    /// between the cursor-setting code and the cursor-rendering code.
    fn walk_for_global_offset(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        cursor: DomCursor,
        offset: &mut usize,
        found: &mut bool,
        ends_with_newline: &mut bool,
    ) {
        if *found {
            return;
        }
        let Some(node) = tree.get(node_id) else {
            return;
        };

        if let Some(text) = node.text_content() {
            if node_id == cursor.node_id {
                *offset += cursor.offset.min(text.len());
                *found = true;
                return;
            }
            *offset += text.len();
            *ends_with_newline = text.ends_with('\n');
            return;
        }

        // <br> elements contribute \n to IFC text — must match paint.rs
        if node.tag() == Some("br") {
            if node_id == cursor.node_id {
                // Cursor is AT this <br> — offset points to start of the \n,
                // not after it. This makes empty lines (e.g., br br sequences)
                // each get a distinct global offset.
                *found = true;
                return;
            }
            *offset += 1;
            *ends_with_newline = true;
            return;
        }

        let is_block = node.tag().map(Self::is_block_element).unwrap_or(false);
        if is_block && *offset > 0 && !*ends_with_newline {
            *offset += 1; // block separator \n
            *ends_with_newline = true;
        }

        // Element-level cursor (empty block) — found at the block boundary
        if node_id == cursor.node_id && node.is_element() {
            *found = true;
            return;
        }

        for &child_id in &node.children {
            Self::walk_for_global_offset(tree, child_id, cursor, offset, found, ends_with_newline);
            if *found {
                return;
            }
        }

        // Empty block elements must reset ends_with_newline so consecutive
        // empty blocks each get a unique global offset via their own separator.
        if is_block && node.children.is_empty() {
            *ends_with_newline = false;
        }

        if is_block && *offset > 0 && !*ends_with_newline {
            *offset += 1; // after-block separator \n
            *ends_with_newline = true;
        }
    }

    /// Compute a `DomCursor` from a click within a contenteditable element.
    ///
    /// Finds the IFC containing the click, uses Parley `from_point()` to get
    /// the IFC flat offset, then converts via `IfcTextRange` to a `DomCursor`.
    pub(in crate::app) fn compute_dom_cursor_from_click(
        tree: &rinch_dom::NodeTree,
        ce_node_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> DomCursor {
        // Case 1: CE element itself has an IFC (inline-only content)
        if let Some(node) = tree.get(ce_node_id)
            && let Some(ref inline_layout) = node.text_layout
        {
            let (abs_x, abs_y) = Self::compute_absolute_position(tree, ce_node_id);
            let padding_left = node.computed_style.padding_left.to_px();
            let padding_top = node.computed_style.padding_top.to_px();
            let rel_x = click_x - abs_x - padding_left + node.scroll_offset.0 as f32;
            let rel_y = click_y - abs_y - padding_top + node.scroll_offset.1 as f32;
            let ifc_offset = rinch_dom::text_query::byte_offset_from_position(
                &inline_layout.layout,
                rel_x,
                rel_y,
            );
            if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                &inline_layout.text_ranges,
                ifc_offset,
                true,
            ) {
                return DomCursor {
                    node_id: nid,
                    offset: off,
                };
            }
        }

        // Case 2: Recursive search through block children (handles any nesting depth)
        if let Some(cursor) = Self::find_cursor_in_block(tree, ce_node_id, click_x, click_y) {
            return cursor;
        }

        // Click was below all blocks — position at end of last text node
        if let Some(cursor) = Self::last_text_cursor(tree, ce_node_id) {
            return cursor;
        }

        // Fallback: first text node
        Self::first_text_cursor(tree, ce_node_id).unwrap_or(DomCursor {
            node_id: ce_node_id,
            offset: 0,
        })
    }

    /// Recursively find the cursor position in a block element at the given click coordinates.
    /// Handles arbitrary nesting depth (nested lists, divs inside lis, etc.).
    fn find_cursor_in_block(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> Option<DomCursor> {
        let node = tree.get(node_id)?;

        // Check if this node has an IFC (inline formatting context)
        if let Some(ref inline_layout) = node.text_layout {
            let (abs_x, abs_y) = Self::compute_absolute_position(tree, node_id);
            let pad_left = node.computed_style.padding_left.to_px();
            let pad_top = node.computed_style.padding_top.to_px();
            let rel_x = click_x - abs_x - pad_left + node.scroll_offset.0 as f32;
            let rel_y = click_y - abs_y - pad_top + node.scroll_offset.1 as f32;
            let ifc_offset = rinch_dom::text_query::byte_offset_from_position(
                &inline_layout.layout,
                rel_x,
                rel_y,
            );
            if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                &inline_layout.text_ranges,
                ifc_offset,
                true,
            ) {
                return Some(DomCursor {
                    node_id: nid,
                    offset: off,
                });
            }
        }

        // Check direct text children with cached_text_parley
        for &child_id in &node.children {
            if let Some(child) = tree.get(child_id)
                && let Some(ref cached_layout) = child.cached_text_parley
            {
                let (tc_abs_x, tc_abs_y) = Self::compute_absolute_position(tree, child_id);
                let rx = click_x - tc_abs_x;
                let ry = click_y - tc_abs_y;
                let byte_off = byte_offset_from_position(cached_layout, rx, ry);
                return Some(DomCursor {
                    node_id: child_id,
                    offset: byte_off,
                });
            }
        }

        // Recurse into children by y-range
        let (_, node_abs_y) = Self::compute_absolute_position(tree, node_id);
        let scroll_y = node.scroll_offset.1 as f32;
        for &child_id in &node.children {
            if let Some(child) = tree.get(child_id) {
                let child_abs_y = node_abs_y + child.layout.y - scroll_y;
                let child_bottom = child_abs_y + child.layout.height;
                if click_y >= child_abs_y
                    && click_y < child_bottom
                    && let Some(cursor) =
                        Self::find_cursor_in_block(tree, child_id, click_x, click_y)
                {
                    return Some(cursor);
                }
            }
        }

        // Fallback: first text node in this subtree
        Self::first_text_cursor(tree, node_id)
    }

    /// Find the first text node (depth-first) under `root` and return cursor at offset 0.
    /// For empty block elements, returns an element-level cursor at the block.
    pub(in crate::app) fn first_text_cursor(tree: &rinch_dom::NodeTree, root: usize) -> Option<DomCursor> {
        let node = tree.get(root)?;
        if node.text_content().is_some() {
            return Some(DomCursor {
                node_id: root,
                offset: 0,
            });
        }
        for &child_id in &node.children {
            if let Some(c) = Self::first_text_cursor(tree, child_id) {
                return Some(c);
            }
        }
        // Empty block element — return element-level cursor
        if node.is_element()
            && node.children.is_empty()
            && node.tag().map(Self::is_block_element).unwrap_or(false)
        {
            return Some(DomCursor {
                node_id: root,
                offset: 0,
            });
        }
        None
    }

    /// Find the last text node (depth-first) under `root` and return cursor at end.
    /// For empty block elements, returns an element-level cursor at the block.
    pub(in crate::app) fn last_text_cursor(tree: &rinch_dom::NodeTree, root: usize) -> Option<DomCursor> {
        let node = tree.get(root)?;
        // Check children in reverse
        for &child_id in node.children.iter().rev() {
            if let Some(c) = Self::last_text_cursor(tree, child_id) {
                return Some(c);
            }
        }
        if let Some(text) = node.text_content() {
            return Some(DomCursor {
                node_id: root,
                offset: text.len(),
            });
        }
        // Empty block element — return element-level cursor
        if node.is_element()
            && node.children.is_empty()
            && node.tag().map(Self::is_block_element).unwrap_or(false)
        {
            return Some(DomCursor {
                node_id: root,
                offset: 0,
            });
        }
        None
    }
}
