use super::*;

#[allow(dead_code)]
impl RinchApp {
    // ── Text helpers ─────────────────────────────────────────────────────

    /// Extract plain text content from a node and its children.
    /// Block-level elements are separated by newlines (flat text model).
    pub(crate) fn extract_text_content(tree: &rinch_dom::NodeTree, node_id: usize) -> String {
        let mut text = String::new();
        Self::collect_text_recursive(tree, node_id, &mut text);
        // Remove trailing newline if present
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    pub(crate) fn collect_text_recursive(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        out: &mut String,
    ) {
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
    pub(crate) fn is_block_element(tag: &str) -> bool {
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

    pub(crate) fn is_heading(tag: &str) -> bool {
        matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
    }

    pub(crate) fn is_list_tag(tag: &str) -> bool {
        matches!(tag, "ul" | "ol")
    }

    /// Strip specific CSS properties from an inline style string.
    pub(crate) fn strip_css_properties(style: &str, properties: &[&str]) -> String {
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

    // ── Word boundary helpers ────────────────────────────────────────────

    /// Find the start of the word containing the given byte position.
    pub(in crate::app) fn find_word_start(text: &str, pos: usize) -> usize {
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
    pub(in crate::app) fn find_word_end(text: &str, pos: usize) -> usize {
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

    // ── Misc utilities ───────────────────────────────────────────────────

    /// Compute line-height in pixels from a block element's computed style.
    pub(crate) fn line_height_px(tree: &rinch_dom::NodeTree, block_id: usize) -> f32 {
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
    pub(crate) fn is_element_cursor(tree: &rinch_dom::NodeTree, cursor: &DomCursor) -> bool {
        tree.get(cursor.node_id)
            .map(|n| n.is_element())
            .unwrap_or(false)
    }

    /// Snapshot all text nodes under `root` for undo.
    /// Kept for debugging; undo now uses CeOps structured ops.
    #[allow(dead_code)]
    pub(crate) fn snapshot_text_nodes(
        tree: &rinch_dom::NodeTree,
        root: usize,
    ) -> Vec<(usize, String)> {
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
    /// Kept for debugging; undo now uses CeOps structured ops.
    #[allow(dead_code)]
    pub(crate) fn collect_subtree_ids(tree: &rinch_dom::NodeTree, root: usize) -> Vec<usize> {
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
}
