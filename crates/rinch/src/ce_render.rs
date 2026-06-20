//! Block rendering helpers for contenteditable.
//!
//! Extracted from `ce_ops.rs` — pure functions that render `BlockData` into
//! the DOM and convert between block types and HTML tags.

use std::collections::HashMap;

use rinch_core::ce::{BlockData, InlineMarkData, InlineRunData};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

// ============================================================================
// Tag ↔ Block Type / Mark Type Conversions
// ============================================================================

/// Map an HTML formatting tag to a mark type string.
pub(crate) fn tag_to_mark_type(tag: &str) -> Option<&'static str> {
    match tag {
        "strong" | "b" => Some("bold"),
        "em" | "i" => Some("italic"),
        "u" => Some("underline"),
        "s" | "del" | "strike" => Some("strike"),
        "code" => Some("code"),
        "mark" => Some("highlight"),
        "sub" => Some("subscript"),
        "sup" => Some("superscript"),
        _ => None,
    }
}

/// Map a mark type string to an HTML formatting tag.
pub(crate) fn mark_type_to_tag(mark_type: &str) -> &str {
    match mark_type {
        "bold" => "strong",
        "italic" => "em",
        "underline" => "u",
        "strike" => "s",
        "code" => "code",
        "highlight" => "mark",
        "subscript" => "sub",
        "superscript" => "sup",
        "link" => "a",
        other => other,
    }
}

/// Map an HTML tag to a block type and attributes.
pub(crate) fn tag_to_block_type(tag: &str) -> (&str, HashMap<String, String>) {
    let mut attrs = HashMap::new();
    let block_type = match tag {
        "p" | "div" => "paragraph",
        "h1" => {
            attrs.insert("level".into(), "1".into());
            "heading"
        }
        "h2" => {
            attrs.insert("level".into(), "2".into());
            "heading"
        }
        "h3" => {
            attrs.insert("level".into(), "3".into());
            "heading"
        }
        "h4" => {
            attrs.insert("level".into(), "4".into());
            "heading"
        }
        "h5" => {
            attrs.insert("level".into(), "5".into());
            "heading"
        }
        "h6" => {
            attrs.insert("level".into(), "6".into());
            "heading"
        }
        "blockquote" => "blockquote",
        "pre" => "code_block",
        "hr" => "horizontal_rule",
        "li" => "list_item",
        other => other,
    };
    (block_type, attrs)
}

/// Map a block type + attrs to an HTML tag.
pub(crate) fn block_type_to_tag(block_type: &str, attrs: &HashMap<String, String>) -> &'static str {
    match block_type {
        "paragraph" => "p",
        "heading" => match attrs.get("level").map(|s| s.as_str()) {
            Some("1") => "h1",
            Some("2") => "h2",
            Some("3") => "h3",
            Some("4") => "h4",
            Some("5") => "h5",
            Some("6") => "h6",
            _ => "h1",
        },
        "blockquote" => "blockquote",
        "code_block" => "pre",
        "horizontal_rule" => "hr",
        "bullet_list" => "ul",
        "ordered_list" => "ol",
        "list_item" => "li",
        _ => "p",
    }
}

// ============================================================================
// Block Data Extraction
// ============================================================================

/// Extract inline runs from a DOM subtree, tracking active formatting marks.
pub(crate) fn extract_inline_runs(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    active_marks: &[InlineMarkData],
    out: &mut Vec<InlineRunData>,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };

    if let Some(text) = node.text_content() {
        // Strip ZWS (\u{200B}) — these are DOM-level cursor aids that don't
        // exist in the EditorDocument's position space.
        let clean = text.replace('\u{200B}', "");
        if !clean.is_empty() {
            out.push(InlineRunData {
                text: clean,
                marks: active_marks.to_vec(),
            });
        }
        return;
    }

    // Element node: check if it's a formatting tag
    let tag = node.tag().unwrap_or("");
    let mut marks = active_marks.to_vec();
    if tag == "a" {
        // Link mark — preserve all attributes (href, title, target, …) so
        // links round-trip through extract_content() → load_content().
        marks.push(InlineMarkData {
            mark_type: "link".to_string(),
            attrs: node.attributes.clone(),
        });
    } else if let Some(mark_type) = tag_to_mark_type(tag) {
        marks.push(InlineMarkData {
            mark_type: mark_type.to_string(),
            attrs: HashMap::new(),
        });
    }

    for &child_id in &node.children {
        extract_inline_runs(tree, child_id, &marks, out);
    }
}

/// Extract a block (direct child of CE root) into BlockData.
/// Handles list containers by recursing into `<li>` children with indent tracking.
pub(crate) fn extract_block(tree: &rinch_dom::NodeTree, node_id: usize, out: &mut Vec<BlockData>) {
    extract_block_at_depth(tree, node_id, 0, out);
}

/// Extract blocks from a node, tracking nesting depth for list items.
fn extract_block_at_depth(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    depth: usize,
    out: &mut Vec<BlockData>,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };
    let tag = node.tag().unwrap_or("").to_string();

    match tag.as_str() {
        "ul" | "ol" => {
            let block_type = if tag == "ul" {
                "bullet_list"
            } else {
                "ordered_list"
            };
            for &child_id in &node.children {
                let child_tag = tree.get(child_id).and_then(|n| n.tag()).unwrap_or("");
                if child_tag == "li" {
                    extract_li_block(tree, child_id, depth, block_type, out);
                }
            }
        }
        _ => {
            let (block_type, attrs) = tag_to_block_type(&tag);
            let mut content = Vec::new();
            for &child_id in &node.children {
                extract_inline_runs(tree, child_id, &[], &mut content);
            }
            out.push(BlockData {
                block_type: block_type.to_string(),
                attrs,
                content,
            });
        }
    }
}

/// Extract a single `<li>` as a block, then recurse into any nested lists.
fn extract_li_block(
    tree: &rinch_dom::NodeTree,
    li_id: usize,
    depth: usize,
    list_type: &str,
    out: &mut Vec<BlockData>,
) {
    let mut content = Vec::new();
    let mut nested_lists: Vec<(usize, &str)> = Vec::new();

    // Separate inline content from nested list children
    for &li_child in &tree.nodes[li_id].children {
        let child_tag = tree.get(li_child).and_then(|n| n.tag()).unwrap_or("");
        match child_tag {
            "ul" => nested_lists.push((li_child, "bullet_list")),
            "ol" => nested_lists.push((li_child, "ordered_list")),
            _ => extract_inline_runs(tree, li_child, &[], &mut content),
        }
    }

    let mut attrs = HashMap::new();
    if depth > 0 {
        attrs.insert("indent".into(), depth.to_string());
    }
    out.push(BlockData {
        block_type: list_type.to_string(),
        attrs,
        content,
    });

    // Recurse into nested lists at depth+1
    for (nested_id, nested_type) in nested_lists {
        for &child_id in &tree.nodes[nested_id].children {
            let child_tag = tree.get(child_id).and_then(|n| n.tag()).unwrap_or("");
            if child_tag == "li" {
                extract_li_block(tree, child_id, depth + 1, nested_type, out);
            }
        }
    }
}

// ============================================================================
// Block Rendering (DOM construction from BlockData)
// ============================================================================

/// Load a sequence of BlockData into the DOM under `ce_root`.
///
/// This is the preferred entry point because it handles list nesting correctly
/// by looking at consecutive list blocks and their indent levels.
pub(crate) fn load_blocks(d: &mut RinchDocument, ce_root: usize, blocks: &[BlockData]) {
    /// Track the current nesting stack for list rendering.
    /// Each entry is (list_dom_id, list_tag, depth).
    struct ListStack {
        entries: Vec<(usize, String)>, // (dom_node_id, list_tag) at each depth
    }

    impl ListStack {
        fn new() -> Self {
            Self {
                entries: Vec::new(),
            }
        }

        /// Ensure the stack has a list container at `depth` with `list_tag`.
        /// Creates new `<ul>`/`<ol>` nodes as needed, nesting inside `<li>` at
        /// depth-1. Returns the list container DOM ID at the requested depth.
        fn ensure_depth(
            &mut self,
            d: &mut RinchDocument,
            ce_root: usize,
            list_tag: &str,
            depth: usize,
        ) -> usize {
            // Pop entries deeper than requested
            while self.entries.len() > depth + 1 {
                self.entries.pop();
            }

            // If we already have the right container at this depth, reuse
            if self.entries.len() == depth + 1 {
                let (id, ref tag) = self.entries[depth];
                if tag == list_tag {
                    return id;
                }
                // Wrong list type at this depth — pop and recreate
                self.entries.pop();
            }

            // Create missing levels
            while self.entries.len() <= depth {
                let current_depth = self.entries.len();
                let parent_id = if current_depth == 0 {
                    ce_root
                } else {
                    // Nest inside the last <li> at the parent depth's list
                    let (parent_list_id, _) = self.entries[current_depth - 1];
                    let li_children = &d.tree.nodes[parent_list_id].children;
                    match li_children.last().copied() {
                        Some(last_li) => last_li,
                        None => parent_list_id, // shouldn't happen
                    }
                };

                let use_tag = if current_depth == depth {
                    list_tag
                } else {
                    // Intermediate levels use the same tag as target
                    list_tag
                };
                let list = d.create_element(use_tag);
                d.append_child(rinch_core::dom::NodeId(parent_id), list);
                self.entries.push((list.0, use_tag.to_string()));
            }

            self.entries[depth].0
        }

        /// Clear the list stack (call when encountering a non-list block).
        fn clear(&mut self) {
            self.entries.clear();
        }
    }

    let mut list_stack = ListStack::new();

    for block in blocks {
        match block.block_type.as_str() {
            "bullet_list" | "ordered_list" => {
                let list_tag = if block.block_type == "bullet_list" {
                    "ul"
                } else {
                    "ol"
                };
                let indent: usize = block
                    .attrs
                    .get("indent")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                let list_id = list_stack.ensure_depth(d, ce_root, list_tag, indent);

                let li = d.create_element("li");
                d.append_child(rinch_core::dom::NodeId(list_id), li);
                render_inline_content(d, li.0, &block.content);
            }
            _ => {
                list_stack.clear();
                let tag = block_type_to_tag(&block.block_type, &block.attrs);
                let el = d.create_element(tag);
                d.append_child(rinch_core::dom::NodeId(ce_root), el);
                render_inline_content(d, el.0, &block.content);
            }
        }
    }
}

/// Render inline content (runs with marks) into a parent element.
pub(crate) fn render_inline_content(
    d: &mut RinchDocument,
    parent_id: usize,
    runs: &[InlineRunData],
) {
    if runs.is_empty() || runs.iter().all(|r| r.text.is_empty()) {
        // Empty block: no children needed. The layout engine gives empty
        // block containers min-height of one line-height (CSS spec behavior),
        // so they render with correct height even without content.
        return;
    }

    for run in runs {
        if run.marks.is_empty() {
            let text = d.create_text(&run.text);
            d.append_child(rinch_core::dom::NodeId(parent_id), text);
        } else {
            // Nest mark wrappers: outermost mark first
            let mut wrapper_id = parent_id;
            for mark in &run.marks {
                let tag = mark_type_to_tag(&mark.mark_type);
                let el = d.create_element(tag);
                // Apply mark attributes (e.g. a link's `href`) so they survive
                // load_content() — symmetric with extract_inline_runs().
                for (name, value) in &mark.attrs {
                    d.set_attribute(el, name, value);
                }
                d.append_child(rinch_core::dom::NodeId(wrapper_id), el);
                wrapper_id = el.0;
            }
            let text = d.create_text(&run.text);
            d.append_child(rinch_core::dom::NodeId(wrapper_id), text);
        }
    }
}

// ============================================================================
// BlockMap — block_index ↔ DOM node_id mapping
// ============================================================================

/// Maps EditorDocument block indices to DOM node IDs.
///
/// `entries[i]` is the DOM node ID for EditorDocument block `i`.
/// For list items, this is the `<li>` element ID (not the `<ul>`/`<ol>` container).
/// For other blocks, this is the block element ID (e.g., `<p>`, `<h1>`).
#[derive(Debug, Clone)]
pub(crate) struct BlockMap {
    entries: Vec<usize>,
}

impl BlockMap {
    /// Create an empty BlockMap.
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Rebuild the BlockMap by walking the CE root's children.
    ///
    /// List containers (`<ul>`, `<ol>`) expand recursively — each `<li>` gets
    /// an entry, and nested lists inside `<li>` are walked depth-first.
    pub(crate) fn rebuild(&mut self, tree: &rinch_dom::NodeTree, ce_root: usize) {
        self.entries.clear();
        let children = &tree.nodes[ce_root].children;
        for &child_id in children {
            let tag = tree.get(child_id).and_then(|n| n.tag()).unwrap_or("");
            if tag == "ul" || tag == "ol" {
                Self::collect_list_items(tree, child_id, &mut self.entries);
            } else {
                self.entries.push(child_id);
            }
        }
    }

    /// Recursively collect `<li>` entries from a list, depth-first.
    /// Each `<li>` is added, then any nested `<ul>`/`<ol>` inside it is recursed.
    fn collect_list_items(tree: &rinch_dom::NodeTree, list_id: usize, entries: &mut Vec<usize>) {
        for &child_id in &tree.nodes[list_id].children {
            let tag = tree.get(child_id).and_then(|n| n.tag()).unwrap_or("");
            if tag == "li" {
                entries.push(child_id);
                // Check for nested lists inside this <li>
                for &li_child in &tree.nodes[child_id].children {
                    let li_child_tag = tree.get(li_child).and_then(|n| n.tag()).unwrap_or("");
                    if li_child_tag == "ul" || li_child_tag == "ol" {
                        Self::collect_list_items(tree, li_child, entries);
                    }
                }
            }
        }
    }

    /// Get the DOM node ID for a block index.
    pub(crate) fn dom_node(&self, block_idx: usize) -> Option<usize> {
        self.entries.get(block_idx).copied()
    }

    /// Get the block index for a DOM node ID.
    pub(crate) fn block_idx(&self, dom_node_id: usize) -> Option<usize> {
        self.entries.iter().position(|&id| id == dom_node_id)
    }

    /// Insert a new entry at position `idx`.
    pub(crate) fn insert(&mut self, idx: usize, dom_node_id: usize) {
        self.entries.insert(idx, dom_node_id);
    }

    /// Remove entry at position `idx`.
    pub(crate) fn remove(&mut self, idx: usize) {
        if idx < self.entries.len() {
            self.entries.remove(idx);
        }
    }

    /// Number of tracked blocks.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Find which block contains a given DOM node, walking ancestors.
    pub(crate) fn block_idx_for_node(
        &self,
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        ce_root: usize,
    ) -> Option<usize> {
        // Walk up from node_id to find an ancestor that is a block element
        // (either a direct child of ce_root, or an <li> in a list child).
        let mut current = node_id;
        loop {
            // Check if current node is in our entries
            if let Some(idx) = self.block_idx(current) {
                return Some(idx);
            }
            let parent = tree.get(current)?.parent?;
            if parent == ce_root {
                // current is a direct child of ce_root but not in entries
                // (shouldn't happen if rebuild was called)
                return None;
            }
            current = parent;
        }
    }
}

// ============================================================================
// Surgical Block Re-rendering
// ============================================================================

/// Re-render a single block in-place: clears existing children and re-renders
/// inline content from `block_data`. If the tag changed, replaces the element.
///
/// Returns the DOM node ID of the (possibly new) block element.
pub(crate) fn render_block_at(
    d: &mut RinchDocument,
    ce_root: usize,
    block_data: &BlockData,
    existing_dom_node: usize,
) -> usize {
    let current_tag = d
        .tree
        .get(existing_dom_node)
        .and_then(|n| n.tag())
        .unwrap_or("")
        .to_string();
    let desired_tag = block_data_to_dom_tag(block_data);

    if current_tag == desired_tag {
        // Same tag: just clear children and re-render inline content.
        // Suppress per-child style resolution during the re-render —
        // batch into one pass at the end. This turns N resolve_styles()
        // calls (one per append_child) into 1.
        clear_children(d, existing_dom_node);
        d.tree.suppress_inline_restyle = true;
        render_inline_content(d, existing_dom_node, &block_data.content);
        d.tree.suppress_inline_restyle = false;
        d.recompute_node_styles_recursive(existing_dom_node);
        existing_dom_node
    } else {
        // Tag changed: create new element, insert before old, remove old
        let new_el = d.create_element(desired_tag);
        let parent_id = d
            .tree
            .get(existing_dom_node)
            .and_then(|n| n.parent)
            .unwrap_or(ce_root);
        d.insert_before(
            rinch_core::dom::NodeId(parent_id),
            new_el,
            rinch_core::dom::NodeId(existing_dom_node),
        );
        render_inline_content(d, new_el.0, &block_data.content);
        d.remove_node(rinch_core::dom::NodeId(existing_dom_node));
        new_el.0
    }
}

/// Insert a new block element into the DOM after `after_dom_node`.
/// If `after_dom_node` is `None`, prepends as the first child of `ce_root`.
///
/// Returns the DOM node ID of the new block element.
pub(crate) fn render_block_insert(
    d: &mut RinchDocument,
    ce_root: usize,
    block_data: &BlockData,
    after_dom_node: Option<usize>,
) -> usize {
    let tag = block_data_to_dom_tag(block_data);

    // For list items, we need to find or create the list container
    if block_data.block_type == "bullet_list" || block_data.block_type == "ordered_list" {
        return render_list_item_insert(d, ce_root, block_data, after_dom_node);
    }

    let el = d.create_element(tag);

    match after_dom_node {
        Some(after_id) => {
            // Find the next sibling of after_id in its parent
            let parent_id = d
                .tree
                .get(after_id)
                .and_then(|n| n.parent)
                .unwrap_or(ce_root);
            let siblings = &d.tree.nodes[parent_id].children;
            let pos = siblings.iter().position(|&c| c == after_id);
            let next = pos.and_then(|p| siblings.get(p + 1).copied());
            if let Some(next_id) = next {
                d.insert_before(
                    rinch_core::dom::NodeId(parent_id),
                    el,
                    rinch_core::dom::NodeId(next_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(parent_id), el);
            }
        }
        None => {
            // Prepend as first child
            let first = d.tree.nodes[ce_root].children.first().copied();
            if let Some(first_id) = first {
                d.insert_before(
                    rinch_core::dom::NodeId(ce_root),
                    el,
                    rinch_core::dom::NodeId(first_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(ce_root), el);
            }
        }
    }

    render_inline_content(d, el.0, &block_data.content);
    el.0
}

/// Remove a block element from the DOM.
///
/// For list items, removes the `<li>` and cleans up the list container if empty.
pub(crate) fn remove_block(d: &mut RinchDocument, dom_node: usize) {
    let parent_id = match d.tree.get(dom_node).and_then(|n| n.parent) {
        Some(p) => p,
        None => {
            d.remove_node(rinch_core::dom::NodeId(dom_node));
            return;
        }
    };

    let parent_tag = d
        .tree
        .get(parent_id)
        .and_then(|n| n.tag())
        .unwrap_or("")
        .to_string();

    d.remove_node(rinch_core::dom::NodeId(dom_node));

    // If the parent is a list container and now empty, remove it too
    if (parent_tag == "ul" || parent_tag == "ol") && d.tree.nodes[parent_id].children.is_empty() {
        d.remove_node(rinch_core::dom::NodeId(parent_id));
    }
}

// ============================================================================
// Position Conversion: EditorPosition ↔ DomCursor
// ============================================================================

use rinch_core::ce::DomCursor;

/// Convert an EditorDocument flat position to a DomCursor.
///
/// Walks blocks via the BlockMap, accumulating text lengths and block separators
/// to find which block and offset within that block the position falls in.
/// Then walks the block's DOM children to find the text node and byte offset.
pub(crate) fn editor_pos_to_dom_cursor(
    tree: &rinch_dom::NodeTree,
    block_map: &BlockMap,
    pos: usize,
) -> Option<DomCursor> {
    let mut remaining = pos;

    for block_idx in 0..block_map.len() {
        let dom_node = block_map.dom_node(block_idx)?;

        // Calculate text length of this block
        let block_text_len = block_text_length(tree, dom_node);

        if remaining <= block_text_len {
            // Position is within this block — find the text node
            return find_dom_cursor_in_block(tree, dom_node, remaining);
        }

        // Account for block text + separator (newline between blocks)
        remaining -= block_text_len;
        if block_idx + 1 < block_map.len() {
            if remaining == 0 {
                // Position is at the very end of this block (before the separator).
                // Place cursor at end of last text node in this block.
                return find_dom_cursor_in_block(tree, dom_node, block_text_len);
            }
            // Skip the block separator
            remaining -= 1;
        }
    }

    // Past end of document — place at end of last block
    if block_map.len() > 0 {
        let last_dom = block_map.dom_node(block_map.len() - 1)?;
        let last_len = block_text_length(tree, last_dom);
        return find_dom_cursor_in_block(tree, last_dom, last_len);
    }

    None
}

/// Convert a DomCursor to an EditorDocument flat position.
///
/// Finds which block contains the cursor's node, then computes the byte
/// offset within that block and adds the block start position.
pub(crate) fn dom_cursor_to_editor_pos(
    tree: &rinch_dom::NodeTree,
    block_map: &BlockMap,
    ce_root: usize,
    cursor: DomCursor,
) -> usize {
    // Find which block this cursor is in
    let block_idx = match block_map.block_idx_for_node(tree, cursor.node_id, ce_root) {
        Some(idx) => idx,
        None => return 0,
    };

    // Compute block start position (sum of preceding blocks + separators)
    let mut block_start = 0;
    for i in 0..block_idx {
        if let Some(dom_node) = block_map.dom_node(i) {
            block_start += block_text_length(tree, dom_node);
        }
        block_start += 1; // block separator
    }

    // Compute offset within block
    let dom_node = match block_map.dom_node(block_idx) {
        Some(n) => n,
        None => return block_start,
    };

    let offset_in_block = offset_within_block(tree, dom_node, cursor);
    block_start + offset_in_block
}

// ============================================================================
// Internal Helpers
// ============================================================================

/// Get the appropriate DOM tag for a BlockData.
///
/// For list items, returns "li" (the caller handles list container creation).
fn block_data_to_dom_tag(block_data: &BlockData) -> &'static str {
    match block_data.block_type.as_str() {
        "bullet_list" | "ordered_list" => "li",
        _ => block_type_to_tag(&block_data.block_type, &block_data.attrs),
    }
}

/// Compute the total text length of a block's DOM subtree (ZWS-stripped).
fn block_text_length(tree: &rinch_dom::NodeTree, block_dom_id: usize) -> usize {
    let mut len = 0;
    collect_text_length(tree, block_dom_id, &mut len);
    len
}

fn collect_text_length(tree: &rinch_dom::NodeTree, node_id: usize, len: &mut usize) {
    let Some(node) = tree.get(node_id) else {
        return;
    };
    if let Some(text) = node.text_content() {
        *len += text.replace('\u{200B}', "").len();
        return;
    }
    for &child_id in &node.children {
        collect_text_length(tree, child_id, len);
    }
}

/// Find a DomCursor within a block element at a given byte offset.
///
/// Walks text nodes in document order, accumulating lengths until we reach
/// the target offset. Returns cursor pointing to the text node and offset.
fn find_dom_cursor_in_block(
    tree: &rinch_dom::NodeTree,
    block_dom_id: usize,
    target_offset: usize,
) -> Option<DomCursor> {
    let mut text_nodes = Vec::new();
    collect_text_nodes_with_lengths(tree, block_dom_id, &mut text_nodes);

    if text_nodes.is_empty() {
        // Empty block — return cursor at the block element itself
        return Some(DomCursor::new(block_dom_id, 0));
    }

    let mut remaining = target_offset;
    for &(node_id, clean_len, _raw_text_len) in &text_nodes {
        if remaining <= clean_len {
            // Convert clean offset back to raw offset (accounting for ZWS)
            let raw_offset = clean_to_raw_offset(tree, node_id, remaining);
            return Some(DomCursor::new(node_id, raw_offset));
        }
        remaining -= clean_len;
    }

    // Past end of block — place at end of last text node
    if let Some(&(node_id, _clean_len, raw_text_len)) = text_nodes.last() {
        return Some(DomCursor::new(node_id, raw_text_len));
    }

    None
}

/// Collect text nodes with their clean (ZWS-stripped) and raw lengths.
fn collect_text_nodes_with_lengths(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    out: &mut Vec<(usize, usize, usize)>, // (node_id, clean_len, raw_len)
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };
    if let Some(text) = node.text_content() {
        let clean_len = text.replace('\u{200B}', "").len();
        out.push((node_id, clean_len, text.len()));
        return;
    }
    for &child_id in &node.children {
        collect_text_nodes_with_lengths(tree, child_id, out);
    }
}

/// Convert a "clean" byte offset (ZWS-stripped) to a raw byte offset in the
/// actual text node content.
fn clean_to_raw_offset(tree: &rinch_dom::NodeTree, node_id: usize, clean_offset: usize) -> usize {
    let text = match tree.get(node_id).and_then(|n| n.text_content()) {
        Some(t) => t,
        None => return clean_offset,
    };

    let mut clean_count = 0;
    for (raw_idx, ch) in text.char_indices() {
        if clean_count == clean_offset {
            return raw_idx;
        }
        if ch != '\u{200B}' {
            clean_count += ch.len_utf8();
        }
    }
    text.len()
}

/// Compute a cursor's byte offset within a block element, walking text nodes
/// in document order. The offset accounts for ZWS stripping.
fn offset_within_block(
    tree: &rinch_dom::NodeTree,
    block_dom_id: usize,
    cursor: DomCursor,
) -> usize {
    // If cursor is on the block element itself (e.g., empty block),
    // the offset within the block is 0 (start of block).
    if cursor.node_id == block_dom_id {
        return 0;
    }

    let mut text_nodes = Vec::new();
    collect_text_nodes_with_lengths(tree, block_dom_id, &mut text_nodes);

    let mut offset = 0;
    for &(node_id, clean_len, _raw_len) in &text_nodes {
        if node_id == cursor.node_id {
            // Convert raw cursor offset to clean offset
            offset += raw_to_clean_offset(tree, node_id, cursor.offset);
            return offset;
        }
        offset += clean_len;
    }

    // Node not found in this block — return accumulated offset
    offset
}

/// Convert a raw byte offset to a "clean" offset (ZWS-stripped).
fn raw_to_clean_offset(tree: &rinch_dom::NodeTree, node_id: usize, raw_offset: usize) -> usize {
    let text = match tree.get(node_id).and_then(|n| n.text_content()) {
        Some(t) => t,
        None => return raw_offset,
    };

    let mut clean_count = 0;
    for (raw_idx, ch) in text.char_indices() {
        if raw_idx >= raw_offset {
            break;
        }
        if ch != '\u{200B}' {
            clean_count += ch.len_utf8();
        }
    }
    clean_count
}

/// Helper to insert a list item into the DOM, creating or reusing a list container.
fn render_list_item_insert(
    d: &mut RinchDocument,
    ce_root: usize,
    block_data: &BlockData,
    after_dom_node: Option<usize>,
) -> usize {
    let list_tag = if block_data.block_type == "bullet_list" {
        "ul"
    } else {
        "ol"
    };

    // Find the list container for the adjacent block
    let (list_id, insert_after_li) = match after_dom_node {
        Some(after_id) => {
            // Check if after_id is an <li> inside a matching list
            let parent_id = d.tree.get(after_id).and_then(|n| n.parent).unwrap_or(0);
            let parent_tag = d.tree.get(parent_id).and_then(|n| n.tag()).unwrap_or("");
            if parent_tag == list_tag {
                (parent_id, Some(after_id))
            } else {
                // Need to create a new list after the after_id's parent element
                let container_parent = d
                    .tree
                    .get(after_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(ce_root);
                let list = d.create_element(list_tag);
                let siblings = d.tree.nodes[container_parent].children.clone();
                let pos = siblings.iter().position(|&c| c == after_id);
                let next = pos.and_then(|p| siblings.get(p + 1).copied());
                if let Some(next_id) = next {
                    d.insert_before(
                        rinch_core::dom::NodeId(container_parent),
                        list,
                        rinch_core::dom::NodeId(next_id),
                    );
                } else {
                    d.append_child(rinch_core::dom::NodeId(container_parent), list);
                }
                (list.0, None)
            }
        }
        None => {
            // Create new list at the beginning
            let list = d.create_element(list_tag);
            let first = d.tree.nodes[ce_root].children.first().copied();
            if let Some(first_id) = first {
                d.insert_before(
                    rinch_core::dom::NodeId(ce_root),
                    list,
                    rinch_core::dom::NodeId(first_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(ce_root), list);
            }
            (list.0, None)
        }
    };

    let li = d.create_element("li");
    match insert_after_li {
        Some(after_li) => {
            let siblings = d.tree.nodes[list_id].children.clone();
            let pos = siblings.iter().position(|&c| c == after_li);
            let next = pos.and_then(|p| siblings.get(p + 1).copied());
            if let Some(next_id) = next {
                d.insert_before(
                    rinch_core::dom::NodeId(list_id),
                    li,
                    rinch_core::dom::NodeId(next_id),
                );
            } else {
                d.append_child(rinch_core::dom::NodeId(list_id), li);
            }
        }
        None => {
            d.append_child(rinch_core::dom::NodeId(list_id), li);
        }
    }

    render_inline_content(d, li.0, &block_data.content);
    li.0
}

/// Remove all children of a DOM node.
fn clear_children(d: &mut RinchDocument, node_id: usize) {
    let children: Vec<usize> = d.tree.nodes[node_id].children.clone();
    for child_id in children {
        d.remove_node(rinch_core::dom::NodeId(child_id));
    }
}
