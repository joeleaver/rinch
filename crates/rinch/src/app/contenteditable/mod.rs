mod ce_navigation;
mod ce_paste;
mod ce_selection;

use super::*;
use ce_selection::compute_ce_scroll_target;

// ── ContentEditable focus ────────────────────────────────────────────────────

/// A cursor position within the DOM: a specific text node and byte offset,
/// or a block element ID for empty blocks (offset always 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DomCursor {
    /// DOM node ID — either a text node or an empty block element.
    pub(super) node_id: usize,
    /// Byte offset within the text node's content (always 0 for element cursors).
    pub(super) offset: usize,
}

/// A snapshot of text node contents for undo.
#[derive(Debug, Clone)]
pub(super) struct UndoEntry {
    pub(super) cursor: DomCursor,
    pub(super) anchor: DomCursor,
    pub(super) text_snapshots: Vec<(usize, String)>, // (node_id, old_text_content)
    pub(super) created_nodes: Vec<usize>,            // nodes created during the edit (removed on undo)
}

/// State for a focused contenteditable element.
pub(crate) struct ContentEditableFocus {
    /// The node ID of the focused contenteditable root element.
    pub(super) ce_node_id: usize,
    /// Caret position.
    pub(super) cursor: DomCursor,
    /// Selection anchor (same as cursor when no selection).
    pub(super) anchor: DomCursor,
    /// Input handler for mapping keys to edit commands (from rinch_editable).
    pub(super) input_handler: InputHandler,
    /// Undo stack for text changes.
    pub(super) undo_stack: Vec<UndoEntry>,
}

impl std::fmt::Debug for ContentEditableFocus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentEditableFocus")
            .field("ce_node_id", &self.ce_node_id)
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .finish()
    }
}

impl RinchApp {
    // ── Contenteditable helpers ────────────────────────────────────────

    /// Extract plain text content from a node and its children.
    /// Block-level elements are separated by newlines (flat text model).
    pub(super) fn extract_text_content(tree: &rinch_dom::NodeTree, node_id: usize) -> String {
        let mut text = String::new();
        Self::collect_text_recursive(tree, node_id, &mut text);
        // Remove trailing newline if present
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    fn collect_text_recursive(tree: &rinch_dom::NodeTree, node_id: usize, out: &mut String) {
        if let Some(node) = tree.get(node_id) {
            if let Some(t) = node.text_content() {
                out.push_str(t);
            } else {
                // Check if this is a block element
                let is_block = node.tag().map(Self::is_block_element).unwrap_or(false);
                if is_block && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                for &child_id in &node.children {
                    Self::collect_text_recursive(tree, child_id, out);
                }
                // Add newline after block element content (if it had children)
                if is_block && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }

    /// Check if a tag name represents a block-level element.
    fn is_block_element(tag: &str) -> bool {
        matches!(
            tag,
            "div"
                | "p"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "li"
                | "ul"
                | "ol"
                | "section"
                | "article"
                | "blockquote"
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
    }

    fn is_heading(tag: &str) -> bool {
        matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
    }

    fn is_list_tag(tag: &str) -> bool {
        matches!(tag, "ul" | "ol")
    }

    /// Strip specific CSS properties from an inline style string.
    fn strip_css_properties(style: &str, properties: &[&str]) -> String {
        style
            .split(';')
            .filter(|decl| {
                let trimmed = decl.trim();
                if trimmed.is_empty() {
                    return false;
                }
                if let Some(prop) = trimmed.split(':').next() {
                    !properties.contains(&prop.trim())
                } else {
                    true
                }
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// Change a block element's tag while preserving children and attributes.
    /// Returns the new node's `NodeId`.
    fn convert_block_tag(
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
    fn outdent_li(
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
    fn find_block_and_parent(
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
    fn find_li_ancestor_for_outdent(
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

    /// Find the start of the word containing the given byte position.
    pub(super) fn find_word_start(text: &str, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let bytes = text.as_bytes();
        let mut i = pos.min(bytes.len());
        // Skip whitespace backwards
        while i > 0 && bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        // Skip word characters backwards
        while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        i
    }

    /// Find the end of the word containing the given byte position.
    pub(super) fn find_word_end(text: &str, pos: usize) -> usize {
        let len = text.len();
        if pos >= len {
            return len;
        }
        let bytes = text.as_bytes();
        let mut i = pos;
        // Skip word characters forwards
        while i < len && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // Skip whitespace forwards (so next call starts at next word)
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    }

    /// Compute the absolute position of a node by walking up through parents.
    fn compute_absolute_position(tree: &rinch_dom::NodeTree, node_id: usize) -> (f32, f32) {
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
    fn dom_cursor_to_global_offset(
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
    pub(super) fn compute_dom_cursor_from_click(
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
    pub(super) fn first_text_cursor(tree: &rinch_dom::NodeTree, root: usize) -> Option<DomCursor> {
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
    pub(super) fn last_text_cursor(tree: &rinch_dom::NodeTree, root: usize) -> Option<DomCursor> {
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

    /// Compute line-height in pixels from a block element's computed style.
    fn line_height_px(tree: &rinch_dom::NodeTree, block_id: usize) -> f32 {
        if let Some(node) = tree.get(block_id) {
            let cs = &node.computed_style;
            match cs.line_height {
                rinch_dom::computed_style::LineHeightValue::Absolute(px) => px,
                rinch_dom::computed_style::LineHeightValue::Relative(factor) => {
                    cs.font_size * factor
                }
                rinch_dom::computed_style::LineHeightValue::Normal => cs.font_size * 1.2,
            }
        } else {
            19.2 // fallback: 16px * 1.2
        }
    }

    /// Check if a DomCursor targets an element (empty block) rather than a text node.
    fn is_element_cursor(tree: &rinch_dom::NodeTree, cursor: &DomCursor) -> bool {
        tree.get(cursor.node_id)
            .map(|n| n.is_element())
            .unwrap_or(false)
    }

    /// Snapshot all text nodes under `root` for undo.
    fn snapshot_text_nodes(tree: &rinch_dom::NodeTree, root: usize) -> Vec<(usize, String)> {
        let mut result = Vec::new();
        Self::snapshot_text_nodes_recursive(tree, root, &mut result);
        result
    }

    fn snapshot_text_nodes_recursive(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        result: &mut Vec<(usize, String)>,
    ) {
        if let Some(node) = tree.get(node_id) {
            if let Some(text) = node.text_content() {
                result.push((node_id, text.to_string()));
            }
            for &child_id in &node.children {
                Self::snapshot_text_nodes_recursive(tree, child_id, result);
            }
        }
    }

    /// Collect all node IDs in a subtree (for undo diffing).
    fn collect_subtree_ids(tree: &rinch_dom::NodeTree, root: usize) -> Vec<usize> {
        let mut ids = Vec::new();
        Self::collect_subtree_ids_recursive(tree, root, &mut ids);
        ids
    }

    fn collect_subtree_ids_recursive(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        ids: &mut Vec<usize>,
    ) {
        ids.push(node_id);
        if let Some(node) = tree.get(node_id) {
            for &child_id in &node.children {
                Self::collect_subtree_ids_recursive(tree, child_id, ids);
            }
        }
    }

    /// Set/clear contenteditable cursor attributes on a DOM node.
    ///
    /// Converts `DomCursor` values to global flat offsets for paint compatibility.
    pub(super) fn set_contenteditable_attributes_dom(
        &self,
        ce_node_id: usize,
        focused: bool,
        cursor: DomCursor,
        anchor: DomCursor,
    ) {
        if let Some(doc) = &self.doc {
            let (cursor_off, anchor_off) = if focused {
                let d = doc.borrow();
                let c = Self::dom_cursor_to_global_offset(&d.tree, ce_node_id, cursor);
                let a = Self::dom_cursor_to_global_offset(&d.tree, ce_node_id, anchor);
                (c, a)
            } else {
                (0, 0)
            };
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(ce_node_id) {
                if focused {
                    node.attributes
                        .insert("data-ce-focused".to_string(), "true".to_string());
                    node.attributes
                        .insert("data-ce-cursor".to_string(), cursor_off.to_string());
                    node.attributes.insert(
                        "data-ce-selection-start".to_string(),
                        anchor_off.to_string(),
                    );
                } else {
                    node.attributes.remove("data-ce-focused");
                    node.attributes.remove("data-ce-cursor");
                    node.attributes.remove("data-ce-selection-start");
                }
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
            }

            // Mark scroll-into-view as pending; applied after layout resolve
            // when text_layout is valid.
            if focused {
                self.ce_scroll_pending.set(true);
            }

            d.tree.dirty_nodes.insert(ce_node_id);
        }
    }

    /// Apply deferred scroll-into-view for the focused contenteditable.
    ///
    /// Must be called AFTER `resolve_layout` so that text_layout is valid.
    pub(super) fn apply_ce_scroll_into_view(&mut self) {
        if !self.ce_scroll_pending.get() {
            return;
        }
        self.ce_scroll_pending.set(false);

        let Some(ref ce) = self.focused_contenteditable else {
            return;
        };
        let cursor = ce.cursor;
        let ce_node_id = ce.ce_node_id;

        if let Some(doc) = &self.doc {
            let cursor_off = {
                let d = doc.borrow();
                Self::dom_cursor_to_global_offset(&d.tree, ce_node_id, cursor)
            };
            let mut d = doc.borrow_mut();
            if let Some(new_scroll) =
                compute_ce_scroll_target(&d.tree, ce_node_id, cursor, cursor_off)
            {
                if let Some(node) = d.tree.nodes.get_mut(ce_node_id) {
                    node.scroll_offset.1 = new_scroll;
                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                }
                d.tree.dirty_nodes.insert(ce_node_id);
            }
        }
    }

    /// Legacy wrapper — keeps old call sites working during migration.
    pub(super) fn set_contenteditable_attributes(
        &self,
        node_id: usize,
        focused: bool,
        cursor: usize,
        selection_start: usize,
    ) {
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(node_id) {
                if focused {
                    node.attributes
                        .insert("data-ce-focused".to_string(), "true".to_string());
                    node.attributes
                        .insert("data-ce-cursor".to_string(), cursor.to_string());
                    node.attributes.insert(
                        "data-ce-selection-start".to_string(),
                        selection_start.to_string(),
                    );
                } else {
                    node.attributes.remove("data-ce-focused");
                    node.attributes.remove("data-ce-cursor");
                    node.attributes.remove("data-ce-selection-start");
                }
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
            }
            d.tree.dirty_nodes.insert(node_id);
        }
    }

    /// Handle a keyboard event for a focused contenteditable element.
    /// Returns true if the event was handled and a redraw is needed.
    pub(super) fn handle_contenteditable_key(
        &mut self,
        key: KeyCode,
        text: Option<&str>,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> bool {
        let ce = match self.focused_contenteditable.as_mut() {
            Some(ce) => ce,
            None => return false,
        };

        let modifiers = EditModifiers {
            ctrl,
            shift,
            alt,
            meta: false,
        };

        // Map KeyCode to rinch_editable::Key
        let edit_key = match key {
            KeyCode::ArrowLeft => Some(EditKey::Left),
            KeyCode::ArrowRight => Some(EditKey::Right),
            KeyCode::ArrowUp => Some(EditKey::Up),
            KeyCode::ArrowDown => Some(EditKey::Down),
            KeyCode::Home => Some(EditKey::Home),
            KeyCode::End => Some(EditKey::End),
            KeyCode::Backspace => Some(EditKey::Backspace),
            KeyCode::Delete => Some(EditKey::Delete),
            KeyCode::Enter => Some(EditKey::Enter),
            KeyCode::Tab => Some(EditKey::Tab),
            KeyCode::Escape => Some(EditKey::Escape),
            KeyCode::KeyA if ctrl => Some(EditKey::A),
            KeyCode::KeyC if ctrl => Some(EditKey::C),
            KeyCode::KeyX if ctrl => Some(EditKey::X),
            KeyCode::KeyZ if ctrl => Some(EditKey::Z),
            KeyCode::KeyY if ctrl => Some(EditKey::Y),
            _ => None,
        };

        // Try mapping the key to an edit command via InputHandler
        let cmd = if let Some(ek) = edit_key {
            ce.input_handler.map_key(ek, modifiers)
        } else {
            None
        };

        // If no command from key mapping, try text input (printable characters)
        let cmd = cmd.or_else(|| {
            if !ctrl && !alt {
                text.and_then(|t| ce.input_handler.map_text(t))
            } else {
                None
            }
        });

        // Special handling for paste (Ctrl+V)
        // Try HTML paste first for rich content, fall back to plain text
        if ctrl && key == KeyCode::KeyV && cmd.is_none() {
            #[cfg(feature = "clipboard")]
            {
                if let Ok(html) = crate::clipboard::paste_html()
                    && !html.is_empty()
                {
                    self.paste_html_into_ce(&html);
                    return true;
                }
            }
        }
        let cmd = cmd.or_else(|| {
            if ctrl && key == KeyCode::KeyV {
                #[cfg(feature = "clipboard")]
                {
                    if let Ok(clipboard_text) = crate::clipboard::paste_text() {
                        return Some(rinch_editable::EditCommand::Paste(clipboard_text));
                    }
                }
                None
            } else {
                None
            }
        });

        let Some(cmd) = cmd else {
            return false;
        };

        // Extract CE info before borrowing self.doc
        let ce_node_id = ce.ce_node_id;
        let cursor = ce.cursor;
        let anchor = ce.anchor;
        let has_selection = cursor != anchor;

        use rinch_editable::EditCommand;
        let mut text_changed = false;

        // Push undo snapshot before any mutating command
        let is_mutating = matches!(
            cmd,
            EditCommand::InsertText(_)
                | EditCommand::Paste(_)
                | EditCommand::DeleteBackward
                | EditCommand::DeleteForward
                | EditCommand::InsertNewline
                | EditCommand::Cut
                | EditCommand::Indent
                | EditCommand::Outdent
        );
        let mut pre_edit_ids: Vec<usize> = Vec::new();
        if is_mutating && let Some(doc) = &self.doc {
            let d = doc.borrow();
            let snapshots = Self::snapshot_text_nodes(&d.tree, ce_node_id);
            pre_edit_ids = Self::collect_subtree_ids(&d.tree, ce_node_id);
            let ce = self.focused_contenteditable.as_mut().unwrap();
            ce.undo_stack.push(UndoEntry {
                cursor,
                anchor,
                text_snapshots: snapshots,
                created_nodes: Vec::new(),
            });
            // Cap undo stack at 100 entries
            if ce.undo_stack.len() > 100 {
                ce.undo_stack.remove(0);
            }
        }

        match cmd {
            // ── Character insertion ──────────────────────────────────
            EditCommand::InsertText(ref insert_str) => {
                if has_selection {
                    self.ce_delete_selection();
                }
                let ce = self.focused_contenteditable.as_mut().unwrap();
                let cur = ce.cursor;
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
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
                        let text_id = d.create_text(insert_str);
                        d.insert_before(
                            rinch_core::dom::NodeId(parent_id),
                            text_id,
                            rinch_core::dom::NodeId(cur.node_id),
                        );
                        ce.cursor = DomCursor {
                            node_id: text_id.0,
                            offset: insert_str.len(),
                        };
                        ce.anchor = ce.cursor;
                    } else if let Some(node) = d.tree.get(cur.node_id)
                        && let Some(current) = node.text_content().map(|s| s.to_string())
                    {
                        let mut new_text = String::with_capacity(current.len() + insert_str.len());
                        let off = cur.offset.min(current.len());
                        new_text.push_str(&current[..off]);
                        new_text.push_str(insert_str);
                        new_text.push_str(&current[off..]);
                        d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                        ce.cursor = DomCursor {
                            node_id: cur.node_id,
                            offset: off + insert_str.len(),
                        };
                        ce.anchor = ce.cursor;
                    } else {
                        // Cursor is on an element node (empty block).
                        // Create a text node with the inserted text and remove min-height.
                        let text_id = d.create_text(insert_str);
                        d.append_child(rinch_core::dom::NodeId(cur.node_id), text_id);
                        d.set_style(rinch_core::dom::NodeId(cur.node_id), "min-height", "0");
                        ce.cursor = DomCursor {
                            node_id: text_id.0,
                            offset: insert_str.len(),
                        };
                        ce.anchor = ce.cursor;
                    }
                }
                text_changed = true;
            }
            EditCommand::Paste(ref paste_text) => {
                if has_selection {
                    self.ce_delete_selection();
                }
                let ce = self.focused_contenteditable.as_mut().unwrap();
                let cur = ce.cursor;
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
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
                        let text_id = d.create_text(paste_text);
                        d.insert_before(
                            rinch_core::dom::NodeId(parent_id),
                            text_id,
                            rinch_core::dom::NodeId(cur.node_id),
                        );
                        ce.cursor = DomCursor {
                            node_id: text_id.0,
                            offset: paste_text.len(),
                        };
                        ce.anchor = ce.cursor;
                    } else if let Some(node) = d.tree.get(cur.node_id)
                        && let Some(current) = node.text_content().map(|s| s.to_string())
                    {
                        let mut new_text = String::with_capacity(current.len() + paste_text.len());
                        let off = cur.offset.min(current.len());
                        new_text.push_str(&current[..off]);
                        new_text.push_str(paste_text);
                        new_text.push_str(&current[off..]);
                        d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                        ce.cursor = DomCursor {
                            node_id: cur.node_id,
                            offset: off + paste_text.len(),
                        };
                        ce.anchor = ce.cursor;
                    } else {
                        // Cursor is on an element node (empty block) — create text child
                        let text_id = d.create_text(paste_text);
                        d.append_child(rinch_core::dom::NodeId(cur.node_id), text_id);
                        d.set_style(rinch_core::dom::NodeId(cur.node_id), "min-height", "0");
                        ce.cursor = DomCursor {
                            node_id: text_id.0,
                            offset: paste_text.len(),
                        };
                        ce.anchor = ce.cursor;
                    }
                }
                text_changed = true;
            }

            // ── Backspace ────────────────────────────────────────────
            EditCommand::DeleteBackward => {
                if has_selection {
                    self.ce_delete_selection();
                    text_changed = true;
                } else if let Some(doc) = &self.doc {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    let cur = ce.cursor;

                    // Check if cursor is on a <br> element
                    let is_br_cursor = doc
                        .borrow()
                        .tree
                        .get(cur.node_id)
                        .and_then(|n| n.tag())
                        .map(|t| t == "br")
                        .unwrap_or(false);

                    if is_br_cursor {
                        // Remove the <br> and move cursor to end of prev text or start of next
                        let d = doc.borrow();
                        let prev = Self::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                        let next = Self::next_text_node(&d.tree, ce_node_id, cur.node_id);
                        let new_cursor = if let Some(prev_id) = prev {
                            // Check if prev is also a <br>
                            let prev_is_br = d
                                .tree
                                .get(prev_id)
                                .and_then(|n| n.tag())
                                .map(|t| t == "br")
                                .unwrap_or(false);
                            if prev_is_br {
                                DomCursor {
                                    node_id: prev_id,
                                    offset: 0,
                                }
                            } else {
                                let len = d
                                    .tree
                                    .get(prev_id)
                                    .and_then(|n| n.text_content())
                                    .map(|s| s.len())
                                    .unwrap_or(0);
                                DomCursor {
                                    node_id: prev_id,
                                    offset: len,
                                }
                            }
                        } else if let Some(next_id) = next {
                            DomCursor {
                                node_id: next_id,
                                offset: 0,
                            }
                        } else {
                            // No adjacent text — create empty placeholder text node
                            // so the CE is never completely empty
                            DomCursor {
                                node_id: 0,
                                offset: 0,
                            } // placeholder, set below
                        };
                        drop(d);
                        let mut d = doc.borrow_mut();
                        d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                        if new_cursor.node_id == 0 {
                            // Create an empty text node in the CE root
                            let text_id = d.create_text("");
                            d.append_child(rinch_core::dom::NodeId(ce_node_id), text_id);
                            ce.cursor = DomCursor {
                                node_id: text_id.0,
                                offset: 0,
                            };
                        } else {
                            ce.cursor = new_cursor;
                        }
                        ce.anchor = ce.cursor;
                        text_changed = true;
                    } else if Self::is_element_cursor(&doc.borrow().tree, &cur) {
                        // ── Cursor at empty block element ──
                        let d_ref = doc.borrow();
                        let cur_block =
                            Self::find_block_and_parent(&d_ref.tree, cur.node_id, ce_node_id);
                        if let Some((cur_block_id, block_parent_id)) = cur_block {
                            let cur_tag = d_ref
                                .tree
                                .get(cur_block_id)
                                .and_then(|n| n.tag())
                                .unwrap_or("")
                                .to_string();

                            // ── Backspace on empty <li>: outdent (any position) ──
                            if cur_tag == "li"
                                && Self::is_list_tag(
                                    d_ref
                                        .tree
                                        .get(block_parent_id)
                                        .and_then(|n| n.tag())
                                        .unwrap_or(""),
                                )
                            {
                                let list_id = block_parent_id;
                                drop(d_ref);
                                let mut d = doc.borrow_mut();
                                let new_el =
                                    Self::outdent_li(&mut d, cur_block_id, list_id, ce_node_id);
                                ce.cursor = DomCursor {
                                    node_id: new_el.0,
                                    offset: 0,
                                };
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else if let Some((li_id, list_id)) =
                                Self::find_li_ancestor_for_outdent(
                                    &d_ref.tree,
                                    cur_block_id,
                                    ce_node_id,
                                )
                            {
                                // Cursor is in a wrapper element inside an <li> — outdent the <li>
                                drop(d_ref);
                                let mut d = doc.borrow_mut();
                                let new_el = Self::outdent_li(&mut d, li_id, list_id, ce_node_id);
                                ce.cursor = DomCursor {
                                    node_id: new_el.0,
                                    offset: 0,
                                };
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else if Self::is_heading(&cur_tag) || cur_tag == "blockquote" {
                                // ── Backspace on empty heading/blockquote: convert to <div> ──
                                drop(d_ref);
                                let mut d = doc.borrow_mut();
                                let new_el = Self::convert_block_tag(&mut d, cur_block_id, "div");
                                ce.cursor = DomCursor {
                                    node_id: new_el.0,
                                    offset: 0,
                                };
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else {
                                // Default: remove the empty block, cursor to end of previous block
                                let siblings = &d_ref.tree.nodes[block_parent_id].children;
                                let pos = siblings.iter().position(|&c| c == cur_block_id);
                                let prev_block_id = pos
                                    .and_then(|p| if p > 0 { Some(siblings[p - 1]) } else { None });
                                let prev_cursor = prev_block_id
                                    .and_then(|pb| Self::last_text_cursor(&d_ref.tree, pb));
                                drop(d_ref);
                                let mut d = doc.borrow_mut();
                                d.remove_node(rinch_core::dom::NodeId(cur_block_id));
                                if let Some(pc) = prev_cursor {
                                    ce.cursor = pc;
                                } else if let Some(pb) = prev_block_id {
                                    ce.cursor = DomCursor {
                                        node_id: pb,
                                        offset: 0,
                                    };
                                }
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            }
                        } else if cur.node_id == ce_node_id {
                            // Cursor is on the CE root element itself — recover by
                            // finding the last cursor target in the CE.
                            let last = Self::last_text_cursor(&d_ref.tree, ce_node_id);
                            drop(d_ref);
                            if let Some(lc) = last {
                                ce.cursor = lc;
                                ce.anchor = ce.cursor;
                            }
                        }
                    } else if cur.offset > 0 {
                        // ── Delete char before cursor in current text node ──
                        let mut d = doc.borrow_mut();
                        if let Some(node) = d.tree.get(cur.node_id)
                            && let Some(current) = node.text_content().map(|s| s.to_string())
                        {
                            let off = cur.offset.min(current.len());
                            let prev_char_start = current[..off]
                                .char_indices()
                                .next_back()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                            let mut new_text = String::with_capacity(current.len());
                            new_text.push_str(&current[..prev_char_start]);
                            new_text.push_str(&current[off..]);
                            if new_text.is_empty() {
                                // Text node is now empty — find nearest cursor target.
                                // Use prev/next_text_node which traverses the full CE
                                // subtree and includes <br> elements as valid targets.
                                let prev = Self::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                                let next = Self::next_text_node(&d.tree, ce_node_id, cur.node_id);

                                if let Some(prev_id) = prev {
                                    let prev_is_br = d
                                        .tree
                                        .get(prev_id)
                                        .and_then(|n| n.tag())
                                        .map(|t| t == "br")
                                        .unwrap_or(false);
                                    d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                    if prev_is_br {
                                        ce.cursor = DomCursor {
                                            node_id: prev_id,
                                            offset: 0,
                                        };
                                    } else {
                                        let len = d
                                            .tree
                                            .get(prev_id)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.len())
                                            .unwrap_or(0);
                                        ce.cursor = DomCursor {
                                            node_id: prev_id,
                                            offset: len,
                                        };
                                    }
                                } else if let Some(next_id) = next {
                                    d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                    ce.cursor = DomCursor {
                                        node_id: next_id,
                                        offset: 0,
                                    };
                                } else {
                                    // CE is completely empty — keep as empty text node
                                    d.set_text_content(rinch_core::dom::NodeId(cur.node_id), "");
                                    ce.cursor = DomCursor {
                                        node_id: cur.node_id,
                                        offset: 0,
                                    };
                                }
                            } else {
                                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                                ce.cursor = DomCursor {
                                    node_id: cur.node_id,
                                    offset: prev_char_start,
                                };
                            }
                            ce.anchor = ce.cursor;
                            text_changed = true;
                        }
                    } else {
                        // ── At start of text node — find previous text node and merge ──
                        let d = doc.borrow();
                        if let Some(prev) = Self::prev_text_node(&d.tree, ce_node_id, cur.node_id) {
                            // Check if we're crossing a block boundary
                            let cur_block =
                                Self::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);
                            let prev_block = Self::find_block_and_parent(&d.tree, prev, ce_node_id);
                            let cross_block = cur_block.is_some()
                                && cur_block.map(|(b, _)| b) != prev_block.map(|(b, _)| b);

                            // ── Special backspace behaviors before cross-block merge ──
                            if let Some((cur_block_id, cur_block_parent)) = cur_block {
                                let cur_tag = d
                                    .tree
                                    .get(cur_block_id)
                                    .and_then(|n| n.tag())
                                    .unwrap_or("")
                                    .to_string();
                                let parent_tag = d
                                    .tree
                                    .get(cur_block_parent)
                                    .and_then(|n| n.tag())
                                    .unwrap_or("")
                                    .to_string();

                                // Backspace at start of <li>: always outdent (any position)
                                if cur_tag == "li" && Self::is_list_tag(&parent_tag) {
                                    let list_id = cur_block_parent;
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el =
                                        Self::outdent_li(&mut d, cur_block_id, list_id, ce_node_id);
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor {
                                            node_id: new_el.0,
                                            offset: 0,
                                        };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else if let Some((li_id, list_id)) =
                                    Self::find_li_ancestor_for_outdent(
                                        &d.tree,
                                        cur_block_id,
                                        ce_node_id,
                                    )
                                {
                                    // Cursor is in a wrapper element inside an <li> — outdent the <li>
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el =
                                        Self::outdent_li(&mut d, li_id, list_id, ce_node_id);
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor {
                                            node_id: new_el.0,
                                            offset: 0,
                                        };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else if Self::is_heading(&cur_tag) || cur_tag == "blockquote" {
                                    // Backspace at start of heading/blockquote: convert to <div>
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el =
                                        Self::convert_block_tag(&mut d, cur_block_id, "div");
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor {
                                            node_id: new_el.0,
                                            offset: 0,
                                        };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else {
                                    // Normal cross-block merge or same-block merge
                                    drop(d);
                                    let mut d = doc.borrow_mut();

                                    if cross_block {
                                        let (prev_block_id, _) = prev_block.unwrap();

                                        // Find merge point: end of last text in prev block
                                        let merge_cursor =
                                            Self::last_text_cursor(&d.tree, prev_block_id)
                                                .unwrap_or(DomCursor {
                                                    node_id: prev,
                                                    offset: 0,
                                                });

                                        // Collect current block's children
                                        let cur_children: Vec<usize> =
                                            d.tree.nodes[cur_block_id].children.clone();

                                        // Merge first text child with prev block's last text, move rest.
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
                                            // Move remaining children to prev block
                                            d.remove_node(rinch_core::dom::NodeId(child_id));
                                            d.append_child(
                                                rinch_core::dom::NodeId(prev_block_id),
                                                rinch_core::dom::NodeId(child_id),
                                            );
                                        }

                                        // Remove the now-empty current block
                                        d.remove_node(rinch_core::dom::NodeId(cur_block_id));

                                        ce.cursor = merge_cursor;
                                        ce.anchor = ce.cursor;
                                    } else {
                                        // Check if prev is a <br> — just remove it
                                        let prev_is_br = d
                                            .tree
                                            .get(prev)
                                            .and_then(|n| n.tag())
                                            .map(|t| t == "br")
                                            .unwrap_or(false);

                                        if prev_is_br {
                                            d.remove_node(rinch_core::dom::NodeId(prev));
                                            ce.cursor = cur;
                                            ce.anchor = ce.cursor;
                                        } else {
                                            // Same block or inline — merge text nodes
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
                                            d.set_text_content(
                                                rinch_core::dom::NodeId(prev),
                                                &merged,
                                            );
                                            d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                            ce.cursor = DomCursor {
                                                node_id: prev,
                                                offset: prev_len,
                                            };
                                            ce.anchor = ce.cursor;
                                        }
                                    }
                                    text_changed = true;
                                }
                            }
                            // close if let Some((cur_block_id, ...))
                            else {
                                // No block found — inline merge
                                drop(d);
                                let mut d = doc.borrow_mut();
                                let prev_is_br = d
                                    .tree
                                    .get(prev)
                                    .and_then(|n| n.tag())
                                    .map(|t| t == "br")
                                    .unwrap_or(false);
                                if prev_is_br {
                                    d.remove_node(rinch_core::dom::NodeId(prev));
                                    ce.cursor = cur;
                                    ce.anchor = ce.cursor;
                                } else {
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
                                    ce.cursor = DomCursor {
                                        node_id: prev,
                                        offset: prev_len,
                                    };
                                    ce.anchor = ce.cursor;
                                }
                                text_changed = true;
                            }
                        } else {
                            // No previous text node — cursor is at very start of CE.
                            // Still handle heading/li/blockquote conversion.
                            let cur_block =
                                Self::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);
                            if let Some((cur_block_id, cur_block_parent)) = cur_block {
                                let cur_tag = d
                                    .tree
                                    .get(cur_block_id)
                                    .and_then(|n| n.tag())
                                    .unwrap_or("")
                                    .to_string();
                                let parent_tag = d
                                    .tree
                                    .get(cur_block_parent)
                                    .and_then(|n| n.tag())
                                    .unwrap_or("")
                                    .to_string();

                                if cur_tag == "li" && Self::is_list_tag(&parent_tag) {
                                    // Outdent li (any position)
                                    let list_id = cur_block_parent;
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el =
                                        Self::outdent_li(&mut d, cur_block_id, list_id, ce_node_id);
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor {
                                            node_id: new_el.0,
                                            offset: 0,
                                        };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else if let Some((li_id, list_id)) =
                                    Self::find_li_ancestor_for_outdent(
                                        &d.tree,
                                        cur_block_id,
                                        ce_node_id,
                                    )
                                {
                                    // Cursor is in a wrapper element inside an <li> — outdent the <li>
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el =
                                        Self::outdent_li(&mut d, li_id, list_id, ce_node_id);
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor {
                                            node_id: new_el.0,
                                            offset: 0,
                                        };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                } else if Self::is_heading(&cur_tag) || cur_tag == "blockquote" {
                                    drop(d);
                                    let mut d = doc.borrow_mut();
                                    let new_el =
                                        Self::convert_block_tag(&mut d, cur_block_id, "div");
                                    if let Some(fc) = Self::first_text_cursor(&d.tree, new_el.0) {
                                        ce.cursor = fc;
                                    } else {
                                        ce.cursor = DomCursor {
                                            node_id: new_el.0,
                                            offset: 0,
                                        };
                                    }
                                    ce.anchor = ce.cursor;
                                    text_changed = true;
                                }
                            }
                        }
                    }
                }
            }

            // ── Delete ───────────────────────────────────────────────
            EditCommand::DeleteForward => {
                if has_selection {
                    self.ce_delete_selection();
                    text_changed = true;
                } else if let Some(doc) = &self.doc {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    let cur = ce.cursor;

                    // Check if cursor is on a <br> element
                    let is_br_cursor = doc
                        .borrow()
                        .tree
                        .get(cur.node_id)
                        .and_then(|n| n.tag())
                        .map(|t| t == "br")
                        .unwrap_or(false);

                    if is_br_cursor {
                        // Remove the <br> and move cursor to start of next text or end of prev
                        let d = doc.borrow();
                        let next = Self::next_text_node(&d.tree, ce_node_id, cur.node_id);
                        let prev = Self::prev_text_node(&d.tree, ce_node_id, cur.node_id);
                        let new_cursor = if let Some(next_id) = next {
                            DomCursor {
                                node_id: next_id,
                                offset: 0,
                            }
                        } else if let Some(prev_id) = prev {
                            let len = d
                                .tree
                                .get(prev_id)
                                .and_then(|n| n.text_content())
                                .map(|s| s.len())
                                .unwrap_or(0);
                            DomCursor {
                                node_id: prev_id,
                                offset: len,
                            }
                        } else {
                            DomCursor {
                                node_id: 0,
                                offset: 0,
                            } // placeholder, set below
                        };
                        drop(d);
                        let mut d = doc.borrow_mut();
                        d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                        if new_cursor.node_id == 0 {
                            let text_id = d.create_text("");
                            d.append_child(rinch_core::dom::NodeId(ce_node_id), text_id);
                            ce.cursor = DomCursor {
                                node_id: text_id.0,
                                offset: 0,
                            };
                        } else {
                            ce.cursor = new_cursor;
                        }
                        ce.anchor = ce.cursor;
                        text_changed = true;
                    } else if Self::is_element_cursor(&doc.borrow().tree, &cur) {
                        // ── Element cursor (empty block) — remove this block,
                        //    move cursor to start of next block ──
                        let d_ref = doc.borrow();
                        let cur_block =
                            Self::find_block_and_parent(&d_ref.tree, cur.node_id, ce_node_id);
                        if let Some((cur_block_id, block_parent_id)) = cur_block {
                            let siblings = &d_ref.tree.nodes[block_parent_id].children;
                            let pos = siblings.iter().position(|&c| c == cur_block_id);
                            let next_block_id = pos.and_then(|p| siblings.get(p + 1).copied());
                            let next_cursor = next_block_id
                                .and_then(|nb| Self::first_text_cursor(&d_ref.tree, nb));
                            drop(d_ref);
                            let mut d = doc.borrow_mut();
                            d.remove_node(rinch_core::dom::NodeId(cur_block_id));
                            if let Some(nc) = next_cursor {
                                ce.cursor = nc;
                            } else if let Some(nb) = next_block_id {
                                ce.cursor = DomCursor {
                                    node_id: nb,
                                    offset: 0,
                                };
                            }
                            ce.anchor = ce.cursor;
                            text_changed = true;
                        }
                    } else {
                        let mut d = doc.borrow_mut();
                        if let Some(node) = d.tree.get(cur.node_id)
                            && let Some(current) = node.text_content().map(|s| s.to_string())
                        {
                            let off = cur.offset.min(current.len());
                            if off < current.len() {
                                // Delete char after cursor
                                let next_char_end = current[off..]
                                    .char_indices()
                                    .nth(1)
                                    .map(|(i, _)| off + i)
                                    .unwrap_or(current.len());
                                let mut new_text = String::with_capacity(current.len());
                                new_text.push_str(&current[..off]);
                                new_text.push_str(&current[next_char_end..]);
                                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                                text_changed = true;
                            } else {
                                // At end of text node — find next and merge
                                drop(d);
                                let d = doc.borrow();
                                if let Some(next) =
                                    Self::next_text_node(&d.tree, ce_node_id, cur.node_id)
                                {
                                    let next_is_br = d
                                        .tree
                                        .get(next)
                                        .and_then(|n| n.tag())
                                        .map(|t| t == "br")
                                        .unwrap_or(false);
                                    let next_is_empty_block = Self::is_element_cursor(
                                        &d.tree,
                                        &DomCursor {
                                            node_id: next,
                                            offset: 0,
                                        },
                                    );

                                    // Check if we're crossing a block boundary
                                    let cur_block = Self::find_block_and_parent(
                                        &d.tree,
                                        cur.node_id,
                                        ce_node_id,
                                    );
                                    let next_block = if next_is_br || next_is_empty_block {
                                        None
                                    } else {
                                        Self::find_block_and_parent(&d.tree, next, ce_node_id)
                                    };
                                    let cross_block = next_block.is_some()
                                        && cur_block.map(|(b, _)| b) != next_block.map(|(b, _)| b);

                                    drop(d);
                                    let mut d = doc.borrow_mut();

                                    if next_is_br || next_is_empty_block {
                                        // Remove the <br> (inline CE) or empty block element
                                        d.remove_node(rinch_core::dom::NodeId(next));
                                    } else if cross_block {
                                        // Cross-block delete: merge next block into current block
                                        let (next_block_id, _) = next_block.unwrap();
                                        let (cur_block_id, _) = cur_block.unwrap();

                                        // Collect next block's children
                                        let next_children: Vec<usize> =
                                            d.tree.nodes[next_block_id].children.clone();

                                        // Merge first text child of next block with current text node
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
                                                    let merged =
                                                        format!("{}{}", current, child_text);
                                                    d.set_text_content(
                                                        rinch_core::dom::NodeId(cur.node_id),
                                                        &merged,
                                                    );
                                                    d.remove_node(rinch_core::dom::NodeId(
                                                        child_id,
                                                    ));
                                                    continue;
                                                }
                                            }
                                            // Move remaining children to current block
                                            d.remove_node(rinch_core::dom::NodeId(child_id));
                                            d.append_child(
                                                rinch_core::dom::NodeId(cur_block_id),
                                                rinch_core::dom::NodeId(child_id),
                                            );
                                        }

                                        // Remove the now-empty next block
                                        d.remove_node(rinch_core::dom::NodeId(next_block_id));
                                        // Cursor stays at current position
                                    } else {
                                        // Same block or inline — merge text nodes
                                        let next_text = d
                                            .tree
                                            .get(next)
                                            .and_then(|n| n.text_content())
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        let merged = format!("{}{}", current, next_text);
                                        d.set_text_content(
                                            rinch_core::dom::NodeId(cur.node_id),
                                            &merged,
                                        );
                                        d.remove_node(rinch_core::dom::NodeId(next));
                                    }
                                    text_changed = true;
                                }
                            }
                        }
                    }
                }
            }

            // ── Enter ────────────────────────────────────────────────
            EditCommand::InsertNewline => {
                if has_selection {
                    self.ce_delete_selection();
                }
                let ce = self.focused_contenteditable.as_mut().unwrap();
                let cur = ce.cursor;
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();

                    // Check if cursor is inside a block element
                    let block_info = Self::find_block_and_parent(&d.tree, cur.node_id, ce_node_id);

                    if let Some((block_id, block_parent_id)) = block_info {
                        let block_tag = d
                            .tree
                            .get(block_id)
                            .and_then(|n| n.tag())
                            .unwrap_or("div")
                            .to_string();

                        // If cursor is in a wrapper element inside an <li>,
                        // redirect to the <li> for Enter behavior (create new <li>, not <div>).
                        let (block_id, block_parent_id, block_tag) = if block_tag != "li" {
                            if let Some((li_id, list_id)) =
                                Self::find_li_ancestor_for_outdent(&d.tree, block_id, ce_node_id)
                            {
                                (li_id, list_id, "li".to_string())
                            } else {
                                (block_id, block_parent_id, block_tag)
                            }
                        } else {
                            (block_id, block_parent_id, block_tag)
                        };

                        // ── Enter in empty <li>: exit the list ──
                        if block_tag == "li" {
                            let is_empty_li = if Self::is_element_cursor(&d.tree, &cur) {
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
                                && Self::is_list_tag(
                                    d.tree
                                        .get(block_parent_id)
                                        .and_then(|n| n.tag())
                                        .unwrap_or(""),
                                )
                            {
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

                                // Collect siblings after the empty <li>
                                let siblings = d.tree.nodes[list_id].children.clone();
                                let li_pos =
                                    siblings.iter().position(|&c| c == block_id).unwrap_or(0);
                                let after_siblings: Vec<usize> = siblings[li_pos + 1..].to_vec();

                                // Create a new <div> to replace the exited <li>
                                let new_div = d.create_element("div");
                                let line_h = Self::line_height_px(&d.tree, block_id);
                                d.set_style(new_div, "min-height", &format!("{:.1}px", line_h));

                                // Remove the empty <li>
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
                                    d.append_child(
                                        rinch_core::dom::NodeId(grandparent_id),
                                        new_div,
                                    );
                                }

                                // If there are siblings after, move them to a new list after <div>
                                if !after_siblings.is_empty() {
                                    let new_list = d.create_element(&list_tag);
                                    for &sib_id in &after_siblings {
                                        d.remove_node(rinch_core::dom::NodeId(sib_id));
                                        d.append_child(new_list, rinch_core::dom::NodeId(sib_id));
                                    }
                                    // Insert new list after <div>
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
                                        d.append_child(
                                            rinch_core::dom::NodeId(grandparent_id),
                                            new_list,
                                        );
                                    }
                                }

                                // If original list is now empty, remove it
                                if d.tree.nodes[list_id].children.is_empty() {
                                    d.remove_node(rinch_core::dom::NodeId(list_id));
                                }

                                // Cursor → start of new <div>
                                ce.cursor = DomCursor {
                                    node_id: new_div.0,
                                    offset: 0,
                                };
                                ce.anchor = ce.cursor;
                            } else {
                                // Non-empty li or not inside a list — split into new li
                                let new_tag = "li";

                                let cur_text = if Self::is_element_cursor(&d.tree, &cur) {
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

                                let new_block_id = d.create_element(new_tag);
                                if after.is_empty() {
                                    let line_h = Self::line_height_px(&d.tree, block_id);
                                    d.set_style(
                                        new_block_id,
                                        "min-height",
                                        &format!("{:.1}px", line_h),
                                    );
                                } else {
                                    let new_text_id = d.create_text(after);
                                    d.append_child(new_block_id, new_text_id);
                                    if off == 0 {
                                        d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                        if d.tree.nodes[block_id].children.is_empty() {
                                            let line_h = Self::line_height_px(&d.tree, block_id);
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
                                    d.append_child(
                                        rinch_core::dom::NodeId(block_parent_id),
                                        new_block_id,
                                    );
                                }

                                if let Some(first) =
                                    Self::first_text_cursor(&d.tree, new_block_id.0)
                                {
                                    ce.cursor = first;
                                } else {
                                    ce.cursor = DomCursor {
                                        node_id: new_block_id.0,
                                        offset: 0,
                                    };
                                }
                                ce.anchor = ce.cursor;
                            }
                        } else {
                            // Non-li block: heading → div, else preserve tag
                            let new_tag = if Self::is_heading(&block_tag) {
                                "div"
                            } else {
                                &block_tag
                            };

                            let cur_text = if Self::is_element_cursor(&d.tree, &cur) {
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

                            let new_block_id = d.create_element(new_tag);
                            if after.is_empty() {
                                let line_h = Self::line_height_px(&d.tree, block_id);
                                d.set_style(
                                    new_block_id,
                                    "min-height",
                                    &format!("{:.1}px", line_h),
                                );
                            } else {
                                let new_text_id = d.create_text(after);
                                d.append_child(new_block_id, new_text_id);
                                if off == 0 {
                                    d.remove_node(rinch_core::dom::NodeId(cur.node_id));
                                    if d.tree.nodes[block_id].children.is_empty() {
                                        let line_h = Self::line_height_px(&d.tree, block_id);
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
                                d.append_child(
                                    rinch_core::dom::NodeId(block_parent_id),
                                    new_block_id,
                                );
                            }

                            // Move cursor to start of new block
                            if let Some(first) = Self::first_text_cursor(&d.tree, new_block_id.0) {
                                ce.cursor = first;
                            } else {
                                ce.cursor = DomCursor {
                                    node_id: new_block_id.0,
                                    offset: 0,
                                };
                            }
                            ce.anchor = ce.cursor;
                        }
                    } else {
                        // Inline-only CE — insert <br> at CE root level,
                        // splitting any inline ancestors (spans) along the way.

                        // If cursor is on a <br>, insert a new <br> before it
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
                            ce.cursor = cur;
                            ce.anchor = ce.cursor;
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
                            d.set_text_content(
                                rinch_core::dom::NodeId(cur.node_id),
                                &cur_text[..off],
                            );

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
                                    let pos =
                                        children.iter().position(|&c| c == child).unwrap_or(0);
                                    children[pos + 1..].to_vec()
                                };
                                // First add after-content to clone
                                d.append_child(clone_id, current_after);
                                // Then move siblings
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
                                .and_then(|pos| {
                                    d.tree.nodes[ce_node_id].children.get(pos + 1).copied()
                                });
                            if let Some(next) = next_sib {
                                d.insert_before(
                                    rinch_core::dom::NodeId(ce_node_id),
                                    current_after,
                                    rinch_core::dom::NodeId(next),
                                );
                                d.insert_before(
                                    rinch_core::dom::NodeId(ce_node_id),
                                    br_id,
                                    current_after,
                                );
                            } else {
                                d.append_child(rinch_core::dom::NodeId(ce_node_id), br_id);
                                d.append_child(rinch_core::dom::NodeId(ce_node_id), current_after);
                            }

                            ce.cursor = DomCursor {
                                node_id: after_text_id.0,
                                offset: 0,
                            };
                            ce.anchor = ce.cursor;
                        } // close else (non-br inline Enter)
                    }
                }
                text_changed = true;
            }

            // ── Cursor movement ──────────────────────────────────────
            EditCommand::MoveLeft
            | EditCommand::MoveRight
            | EditCommand::MoveWordLeft
            | EditCommand::MoveWordRight
            | EditCommand::MoveUp
            | EditCommand::MoveDown
            | EditCommand::MoveToLineStart
            | EditCommand::MoveToLineEnd
            | EditCommand::SelectLeft
            | EditCommand::SelectRight
            | EditCommand::SelectWordLeft
            | EditCommand::SelectWordRight
            | EditCommand::SelectUp
            | EditCommand::SelectDown
            | EditCommand::SelectToLineStart
            | EditCommand::SelectToLineEnd => {
                let is_select = matches!(
                    cmd,
                    EditCommand::SelectLeft
                        | EditCommand::SelectRight
                        | EditCommand::SelectWordLeft
                        | EditCommand::SelectWordRight
                        | EditCommand::SelectUp
                        | EditCommand::SelectDown
                        | EditCommand::SelectToLineStart
                        | EditCommand::SelectToLineEnd
                );

                if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    let new_cursor = Self::move_dom_cursor(&d.tree, ce_node_id, cursor, &cmd);
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = new_cursor;
                    if !is_select {
                        ce.anchor = new_cursor;
                    }
                }
            }

            // ── Select All ───────────────────────────────────────────
            EditCommand::SelectAll => {
                if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    if let Some(first) = Self::first_text_cursor(&d.tree, ce_node_id) {
                        ce.anchor = first;
                    }
                    if let Some(last) = Self::last_text_cursor(&d.tree, ce_node_id) {
                        ce.cursor = last;
                    }
                }
            }

            // ── Copy ─────────────────────────────────────────────────
            EditCommand::Copy =>
            {
                #[cfg(feature = "clipboard")]
                if has_selection && let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    let text = Self::extract_selection_text(&d.tree, ce_node_id, anchor, cursor);
                    let html = Self::extract_selection_html(&d.tree, ce_node_id, anchor, cursor);
                    let _ = crate::clipboard::copy_html(&html, Some(&text));
                }
            }

            // ── Cut ──────────────────────────────────────────────────
            EditCommand::Cut =>
            {
                #[cfg(feature = "clipboard")]
                if has_selection {
                    if let Some(doc) = &self.doc {
                        let d = doc.borrow();
                        let text =
                            Self::extract_selection_text(&d.tree, ce_node_id, anchor, cursor);
                        let html =
                            Self::extract_selection_html(&d.tree, ce_node_id, anchor, cursor);
                        let _ = crate::clipboard::copy_html(&html, Some(&text));
                    }
                    self.ce_delete_selection();
                    text_changed = true;
                }
            }

            // ── Undo ──────────────────────────────────────────────────
            EditCommand::Undo => {
                let ce = self.focused_contenteditable.as_mut().unwrap();
                if let Some(entry) = ce.undo_stack.pop() {
                    let restore_cursor = entry.cursor;
                    let restore_anchor = entry.anchor;
                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        // Remove nodes that were created during the edit
                        for &node_id in &entry.created_nodes {
                            if d.tree.get(node_id).is_some() {
                                d.remove_node(rinch_core::dom::NodeId(node_id));
                            }
                        }
                        // Restore text content
                        for (node_id, old_text) in &entry.text_snapshots {
                            if d.tree.get(*node_id).is_some() {
                                d.set_text_content(rinch_core::dom::NodeId(*node_id), old_text);
                            }
                        }
                    }
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = restore_cursor;
                    ce.anchor = restore_anchor;
                    text_changed = true;
                }
            }

            // ── Tab indent ────────────────────────────────────────────
            EditCommand::Indent => {
                if let Some(doc) = &self.doc {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    let cur = ce.cursor;
                    let d_ref = doc.borrow();

                    // Find the <li> the cursor is in
                    let block_info =
                        Self::find_block_and_parent(&d_ref.tree, cur.node_id, ce_node_id);
                    if let Some((li_id, list_id)) = block_info {
                        let li_tag = d_ref.tree.get(li_id).and_then(|n| n.tag()).unwrap_or("");
                        let list_tag = d_ref
                            .tree
                            .get(list_id)
                            .and_then(|n| n.tag())
                            .unwrap_or("")
                            .to_string();

                        // Resolve the actual <li> and list — either directly or via ancestor walk
                        let resolved = if li_tag == "li" && Self::is_list_tag(&list_tag) {
                            Some((li_id, list_id, list_tag.clone()))
                        } else {
                            // Cursor may be in a wrapper <div> inside an <li>
                            Self::find_li_ancestor_for_outdent(&d_ref.tree, li_id, ce_node_id).map(
                                |(real_li, real_list)| {
                                    let tag = d_ref
                                        .tree
                                        .get(real_list)
                                        .and_then(|n| n.tag())
                                        .unwrap_or("ul")
                                        .to_string();
                                    (real_li, real_list, tag)
                                },
                            )
                        };

                        if let Some((real_li_id, real_list_id, real_list_tag)) = resolved {
                            // Find previous sibling <li>
                            let siblings = d_ref.tree.nodes[real_list_id].children.clone();
                            let pos = siblings.iter().position(|&c| c == real_li_id).unwrap_or(0);

                            if pos > 0 {
                                let prev_li = siblings[pos - 1];
                                // Check if prev_li already has a nested list as last child
                                let prev_children = d_ref.tree.nodes[prev_li].children.clone();
                                let nested_list = prev_children.last().and_then(|&last| {
                                    d_ref.tree.get(last).and_then(|n| n.tag()).and_then(|t| {
                                        if Self::is_list_tag(t) {
                                            Some(last)
                                        } else {
                                            None
                                        }
                                    })
                                });

                                drop(d_ref);
                                let mut d = doc.borrow_mut();

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

                                // No flex style needed: the layout engine creates anonymous
                                // block boxes for mixed inline+block content automatically.

                                // Cursor stays in the same text node
                                ce.cursor = cur;
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else {
                                return false; // Can't indent first item
                            }
                        } else {
                            return false; // Not in a list item
                        }
                    } else {
                        return false; // Not in a block
                    }
                } else {
                    return false;
                }
            }

            // ── Shift+Tab outdent ────────────────────────────────────────
            EditCommand::Outdent => {
                if let Some(doc) = &self.doc {
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    let cur = ce.cursor;
                    let d_ref = doc.borrow();

                    // Find the <li> the cursor is in
                    let block_info =
                        Self::find_block_and_parent(&d_ref.tree, cur.node_id, ce_node_id);
                    if let Some((li_id, nested_list_id)) = block_info {
                        let li_tag = d_ref.tree.get(li_id).and_then(|n| n.tag()).unwrap_or("");
                        let nested_list_tag = d_ref
                            .tree
                            .get(nested_list_id)
                            .and_then(|n| n.tag())
                            .unwrap_or("")
                            .to_string();

                        // Resolve the actual <li> and its parent list
                        let resolved = if li_tag == "li" && Self::is_list_tag(&nested_list_tag) {
                            Some((li_id, nested_list_id, nested_list_tag.clone()))
                        } else {
                            Self::find_li_ancestor_for_outdent(&d_ref.tree, li_id, ce_node_id).map(
                                |(real_li, real_list)| {
                                    let tag = d_ref
                                        .tree
                                        .get(real_list)
                                        .and_then(|n| n.tag())
                                        .unwrap_or("ul")
                                        .to_string();
                                    (real_li, real_list, tag)
                                },
                            )
                        };

                        if let Some((real_li_id, real_nested_list_id, real_nested_list_tag)) =
                            resolved
                        {
                            // Check if this list is nested inside another <li>
                            let parent_li =
                                d_ref.tree.get(real_nested_list_id).and_then(|n| n.parent);
                            let parent_li_tag = parent_li
                                .and_then(|p| d_ref.tree.get(p))
                                .and_then(|n| n.tag())
                                .unwrap_or("");

                            if parent_li_tag == "li" {
                                let parent_li_id = parent_li.unwrap();
                                let outer_list_id = d_ref
                                    .tree
                                    .get(parent_li_id)
                                    .and_then(|n| n.parent)
                                    .unwrap_or(ce_node_id);

                                // Collect siblings after current <li> in the nested list
                                let nested_siblings =
                                    d_ref.tree.nodes[real_nested_list_id].children.clone();
                                let pos = nested_siblings
                                    .iter()
                                    .position(|&c| c == real_li_id)
                                    .unwrap_or(0);
                                let after_siblings: Vec<usize> =
                                    nested_siblings[pos + 1..].to_vec();

                                drop(d_ref);
                                let mut d = doc.borrow_mut();

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
                                ce.cursor = cur;
                                ce.anchor = ce.cursor;
                                text_changed = true;
                            } else {
                                return false; // Already top-level
                            }
                        } else {
                            return false; // Not in a list item
                        }
                    } else {
                        return false; // Not in a block
                    }
                } else {
                    return false;
                }
            }

            // ── Unhandled commands (Escape, Redo, etc.) ───────────────
            _ => {
                return false;
            }
        }

        // Record any newly created nodes in the undo entry
        if is_mutating
            && !pre_edit_ids.is_empty()
            && let Some(doc) = &self.doc
        {
            let d = doc.borrow();
            let post_edit_ids = Self::collect_subtree_ids(&d.tree, ce_node_id);
            let mut created = Vec::new();
            for &id in &post_edit_ids {
                if !pre_edit_ids.contains(&id) {
                    created.push(id);
                }
            }
            if !created.is_empty() {
                let ce = self.focused_contenteditable.as_mut().unwrap();
                if let Some(entry) = ce.undo_stack.last_mut() {
                    entry.created_nodes = created;
                }
            }
        }

        // Update cursor/selection attributes on the DOM node
        let ce = self.focused_contenteditable.as_ref().unwrap();
        let final_cursor = ce.cursor;
        let final_anchor = ce.anchor;
        let ce_nid = ce.ce_node_id;
        self.set_contenteditable_attributes_dom(ce_nid, true, final_cursor, final_anchor);

        // Dispatch oninput event if text changed
        if text_changed && let Some(doc) = &self.doc {
            let handler_id = {
                let d = doc.borrow();
                d.tree
                    .get(ce_nid)
                    .and_then(|n| n.attributes.get("data-oninput"))
                    .and_then(|s| s.parse::<usize>().ok())
            };
            if let Some(hid) = handler_id {
                let dispatch_text = {
                    let d = doc.borrow();
                    Self::extract_text_content(&d.tree, ce_nid)
                };
                events::dispatch_input_event(events::EventHandlerId(hid), dispatch_text);
            }
        }

        self.scene_dirty = true;
        true
    }
}

