use super::*;

impl RinchApp {
    // ── HTML paste ────────────────────────────────────────────────────────

    /// Paste HTML content into the focused contenteditable element.
    ///
    /// Parses the HTML fragment, creates DOM nodes, and inserts them at the
    /// cursor position. Handles splitting text nodes, inserting block elements,
    /// and updating the cursor position.
    #[allow(dead_code)]
    pub(super) fn paste_html_into_ce(&mut self, html: &str) {
        let ce = match self.focused_contenteditable.as_mut() {
            Some(ce) => ce,
            None => return,
        };
        let ce_node_id = ce.ce_node_id;
        let has_selection = ce.cursor != ce.anchor;

        // Delete selection first if any
        if has_selection {
            self.ce_delete_selection();
        }

        let parsed = parse_html_fragment(html);
        if parsed.is_empty() {
            return;
        }

        let ce = self.focused_contenteditable.as_mut().unwrap();
        let cur = ce.cursor;

        let Some(doc) = &self.doc else { return };
        let mut d = doc.borrow_mut();

        // Flatten parsed nodes: collect all top-level nodes to insert.
        // If the parsed content is just inline text/spans, insert inline.
        // If it contains block elements, insert as blocks.
        let has_blocks = parsed
            .iter()
            .any(|n| matches!(n, ParsedNode::Element { tag, .. } if Self::is_block_element(tag)));

        if !has_blocks {
            // All inline content — insert text/inline elements at cursor position
            let last_cursor = Self::insert_parsed_inline(&mut d, ce_node_id, cur, &parsed);
            let ce = self.focused_contenteditable.as_mut().unwrap();
            ce.cursor = last_cursor;
            ce.anchor = last_cursor;
        } else {
            // Contains block elements — need to split at cursor and insert blocks
            let last_cursor = Self::insert_parsed_blocks(&mut d, ce_node_id, cur, &parsed);
            let ce = self.focused_contenteditable.as_mut().unwrap();
            ce.cursor = last_cursor;
            ce.anchor = last_cursor;
        }

        // Request redraw
        drop(d);
    }

    /// Insert inline parsed nodes at the cursor position.
    /// Returns the new cursor position after insertion.
    #[allow(dead_code)]
    fn insert_parsed_inline(
        d: &mut rinch_dom::RinchDocument,
        ce_node_id: usize,
        cur: DomCursor,
        nodes: &[ParsedNode],
    ) -> DomCursor {
        // Collect all text from the parsed nodes (flattened for inline insertion)
        let mut flat_text = String::new();
        Self::flatten_parsed_text(nodes, &mut flat_text);

        if flat_text.is_empty() {
            return cur;
        }

        // Check if cursor is on a <br> element
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
            // Insert parsed nodes before the <br>
            let last_id = Self::create_parsed_nodes(
                d,
                rinch_core::dom::NodeId(parent_id),
                nodes,
                Some(rinch_core::dom::NodeId(cur.node_id)),
            );
            if let Some(last) = last_id {
                return last;
            }
            return cur;
        }

        if let Some(node) = d.tree.get(cur.node_id)
            && node.text_content().is_some()
        {
            let current = node.text_content().unwrap().to_string();
            let off = cur.offset.min(current.len());

            // If we're inserting plain inline text only (no tags), do simple text splice
            let all_text_nodes = nodes.iter().all(|n| matches!(n, ParsedNode::Text(_)));
            if all_text_nodes {
                let mut new_text = String::with_capacity(current.len() + flat_text.len());
                new_text.push_str(&current[..off]);
                new_text.push_str(&flat_text);
                new_text.push_str(&current[off..]);
                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                return DomCursor {
                    node_id: cur.node_id,
                    offset: off + flat_text.len(),
                };
            }

            // Mixed inline content — split text node and insert elements between
            let parent_id = d
                .tree
                .get(cur.node_id)
                .and_then(|n| n.parent)
                .unwrap_or(ce_node_id);

            let before = &current[..off];
            let after = &current[off..];

            // Update current text node to just the "before" part
            if !before.is_empty() {
                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), before);
            }

            // Find reference node for insertion (sibling after current text node)
            let ref_node = {
                let children = &d.tree.nodes[parent_id].children;
                let pos = children.iter().position(|&c| c == cur.node_id);
                pos.and_then(|p| children.get(p + 1).copied())
            };

            // Create "after" text node if needed
            let after_id = if !after.is_empty() {
                let id = d.create_text(after);
                if let Some(ref_id) = ref_node {
                    d.insert_before(
                        rinch_core::dom::NodeId(parent_id),
                        id,
                        rinch_core::dom::NodeId(ref_id),
                    );
                } else {
                    d.append_child(rinch_core::dom::NodeId(parent_id), id);
                }
                Some(id)
            } else {
                None
            };

            // Insert parsed nodes between "before" and "after"
            let insert_ref = after_id
                .map(|id| rinch_core::dom::NodeId(id.0))
                .or_else(|| ref_node.map(rinch_core::dom::NodeId));
            let last_id =
                Self::create_parsed_nodes(d, rinch_core::dom::NodeId(parent_id), nodes, insert_ref);

            if let Some(last) = last_id {
                return last;
            }
            // Fall back to end of before text
            if !before.is_empty() {
                return DomCursor {
                    node_id: cur.node_id,
                    offset: before.len(),
                };
            }
            return cur;
        }

        // Cursor on element node (empty block) — insert as children
        let last_id =
            Self::create_parsed_nodes(d, rinch_core::dom::NodeId(cur.node_id), nodes, None);
        d.set_style(rinch_core::dom::NodeId(cur.node_id), "min-height", "0");
        if let Some(last) = last_id {
            return last;
        }
        cur
    }

    /// Insert parsed nodes that contain block elements at the cursor position.
    /// Splits the current block at the cursor and inserts blocks between.
    #[allow(dead_code)]
    fn insert_parsed_blocks(
        d: &mut rinch_dom::RinchDocument,
        ce_node_id: usize,
        cur: DomCursor,
        nodes: &[ParsedNode],
    ) -> DomCursor {
        // Find which direct child of CE root contains the cursor
        let cursor_block =
            Self::find_ancestor_child_of(&d.tree, cur.node_id, ce_node_id).unwrap_or(cur.node_id);

        // Find the insertion point: after the cursor's block
        let ref_node = {
            let children = &d.tree.nodes[ce_node_id].children;
            let pos = children.iter().position(|&c| c == cursor_block);
            pos.and_then(|p| children.get(p + 1).copied())
        };

        // Create and insert the parsed nodes
        let insert_ref = ref_node.map(rinch_core::dom::NodeId);
        let last_id =
            Self::create_parsed_nodes(d, rinch_core::dom::NodeId(ce_node_id), nodes, insert_ref);

        if let Some(last) = last_id {
            return last;
        }
        cur
    }

    /// Find the ancestor of `node_id` that is a direct child of `root_id`.
    #[allow(dead_code)]
    fn find_ancestor_child_of(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        root_id: usize,
    ) -> Option<usize> {
        let mut current = node_id;
        loop {
            let parent = tree.get(current)?.parent?;
            if parent == root_id {
                return Some(current);
            }
            current = parent;
        }
    }

    /// Create DOM nodes from parsed HTML nodes and insert them into the tree.
    /// Returns the cursor position at the end of the last inserted node.
    #[allow(dead_code)]
    fn create_parsed_nodes(
        d: &mut rinch_dom::RinchDocument,
        parent: rinch_core::dom::NodeId,
        nodes: &[ParsedNode],
        insert_before: Option<rinch_core::dom::NodeId>,
    ) -> Option<DomCursor> {
        let mut last_cursor = None;

        for node in nodes {
            match node {
                ParsedNode::Text(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    let text_id = d.create_text(text);
                    if let Some(ref_id) = insert_before {
                        d.insert_before(parent, text_id, ref_id);
                    } else {
                        d.append_child(parent, text_id);
                    }
                    last_cursor = Some(DomCursor {
                        node_id: text_id.0,
                        offset: text.len(),
                    });
                }
                ParsedNode::Element {
                    tag,
                    attributes,
                    children,
                } => {
                    if tag == "br" {
                        let br_id = d.create_element("br");
                        if let Some(ref_id) = insert_before {
                            d.insert_before(parent, br_id, ref_id);
                        } else {
                            d.append_child(parent, br_id);
                        }
                        last_cursor = Some(DomCursor {
                            node_id: br_id.0,
                            offset: 0,
                        });
                        continue;
                    }

                    let el_id = d.create_element(tag);

                    // Apply safe attributes
                    for (name, value) in attributes {
                        match name.as_str() {
                            "style" => d.set_attribute(el_id, "style", value),
                            "class" => d.set_attribute(el_id, "class", value),
                            "href" => d.set_attribute(el_id, "href", value),
                            // Skip all other attributes (data-rid, onclick, etc.)
                            _ => {}
                        }
                    }

                    // Recursively create children
                    let child_cursor = Self::create_parsed_nodes(d, el_id, children, None);
                    if let Some(cc) = child_cursor {
                        last_cursor = Some(cc);
                    } else {
                        last_cursor = Some(DomCursor {
                            node_id: el_id.0,
                            offset: 0,
                        });
                    }

                    if let Some(ref_id) = insert_before {
                        d.insert_before(parent, el_id, ref_id);
                    } else {
                        d.append_child(parent, el_id);
                    }
                }
            }
        }

        last_cursor
    }

    /// Flatten parsed nodes into a single text string (stripping all tags).
    #[allow(dead_code)]
    fn flatten_parsed_text(nodes: &[ParsedNode], out: &mut String) {
        for node in nodes {
            match node {
                ParsedNode::Text(t) => out.push_str(t),
                ParsedNode::Element { tag, children, .. } => {
                    if tag == "br" {
                        out.push('\n');
                    } else {
                        Self::flatten_parsed_text(children, out);
                        if Self::is_block_element(tag) {
                            out.push('\n');
                        }
                    }
                }
            }
        }
    }
}
