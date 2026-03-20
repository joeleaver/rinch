use super::*;

impl RinchApp {
    // ── DOM cursor navigation helpers ───────────────────────────────────

    /// Move a `DomCursor` according to an `EditCommand` movement direction.
    pub(crate) fn move_dom_cursor(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
        cmd: &rinch_editable::EditCommand,
    ) -> DomCursor {
        use rinch_editable::EditCommand;
        match cmd {
            EditCommand::MoveLeft | EditCommand::SelectLeft => {
                Self::move_cursor_left(tree, ce_root, cursor, false)
            }
            EditCommand::MoveRight | EditCommand::SelectRight => {
                Self::move_cursor_right(tree, ce_root, cursor, false)
            }
            EditCommand::MoveWordLeft | EditCommand::SelectWordLeft => {
                Self::move_cursor_left(tree, ce_root, cursor, true)
            }
            EditCommand::MoveWordRight | EditCommand::SelectWordRight => {
                Self::move_cursor_right(tree, ce_root, cursor, true)
            }
            EditCommand::MoveUp | EditCommand::SelectUp => {
                Self::move_cursor_vertical(tree, ce_root, cursor, -1)
            }
            EditCommand::MoveDown | EditCommand::SelectDown => {
                Self::move_cursor_vertical(tree, ce_root, cursor, 1)
            }
            EditCommand::MoveToLineStart | EditCommand::SelectToLineStart => {
                Self::move_cursor_home(tree, ce_root, cursor)
            }
            EditCommand::MoveToLineEnd | EditCommand::SelectToLineEnd => {
                Self::move_cursor_end(tree, ce_root, cursor)
            }
            _ => cursor,
        }
    }

    /// Move cursor left by one character (or one word if `word` is true).
    fn move_cursor_left(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
        word: bool,
    ) -> DomCursor {
        // Element cursor (empty block) — move to end of previous node
        if Self::is_element_cursor(tree, &cursor) {
            if let Some(prev) = Self::prev_text_node(tree, ce_root, cursor.node_id) {
                let len = tree
                    .get(prev)
                    .and_then(|n| n.text_content())
                    .map(|t| t.len())
                    .unwrap_or(0);
                return DomCursor {
                    node_id: prev,
                    offset: len,
                };
            }
            return cursor;
        }

        if let Some(node) = tree.get(cursor.node_id)
            && let Some(text) = node.text_content()
            && cursor.offset > 0
        {
            if word {
                let new_off = Self::find_word_start(text, cursor.offset);
                return DomCursor {
                    node_id: cursor.node_id,
                    offset: new_off,
                };
            }
            // Move back one character
            let new_off = text[..cursor.offset]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            return DomCursor {
                node_id: cursor.node_id,
                offset: new_off,
            };
        }
        // At start of node (or on a <br>) — move to previous position
        let cursor_is_br = tree
            .get(cursor.node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "br")
            .unwrap_or(false);

        let Some(prev) = Self::prev_text_node(tree, ce_root, cursor.node_id) else {
            return cursor;
        };
        let prev_is_br = tree
            .get(prev)
            .and_then(|n| n.tag())
            .map(|t| t == "br")
            .unwrap_or(false);

        if prev_is_br {
            if cursor_is_br || word {
                // Cursor is on a <br> or in word mode — skip the line-terminator <br>,
                // but stop at blank lines (consecutive <br>s).
                let Some(before) = Self::prev_text_node(tree, ce_root, prev) else {
                    return cursor;
                };
                let before_is_br = tree
                    .get(before)
                    .and_then(|n| n.tag())
                    .map(|t| t == "br")
                    .unwrap_or(false);
                if before_is_br {
                    // prev is preceded by another <br> — prev is a blank line, stop there
                    return DomCursor {
                        node_id: prev,
                        offset: 0,
                    };
                }
                let len = tree
                    .get(before)
                    .and_then(|n| n.text_content())
                    .map(|t| t.len())
                    .unwrap_or(0);
                if word && len > 0 {
                    let text = tree.get(before).and_then(|n| n.text_content()).unwrap();
                    return DomCursor {
                        node_id: before,
                        offset: Self::find_word_start(text, len),
                    };
                }
                return DomCursor {
                    node_id: before,
                    offset: len,
                };
            }
            // Cursor is on text, prev is <br>. Check what's before the <br>:
            // If another <br>, this <br> is a blank line — stop here.
            // If text, this <br> is a line terminator — skip to end of that text.
            let before = Self::prev_text_node(tree, ce_root, prev);
            let before_is_br = before
                .and_then(|id| tree.get(id))
                .and_then(|n| n.tag())
                .map(|t| t == "br")
                .unwrap_or(false);
            if before_is_br || before.is_none() {
                // Blank line — stop at the <br>
                return DomCursor {
                    node_id: prev,
                    offset: 0,
                };
            }
            // Line terminator — skip to end of text before it
            let before_id = before.unwrap();
            let len = tree
                .get(before_id)
                .and_then(|n| n.text_content())
                .map(|t| t.len())
                .unwrap_or(0);
            if word && len > 0 {
                let text = tree.get(before_id).and_then(|n| n.text_content()).unwrap();
                return DomCursor {
                    node_id: before_id,
                    offset: Self::find_word_start(text, len),
                };
            }
            return DomCursor {
                node_id: before_id,
                offset: len,
            };
        }

        // Prev is a text node — go to its end
        let len = tree
            .get(prev)
            .and_then(|n| n.text_content())
            .map(|t| t.len())
            .unwrap_or(0);
        if word && len > 0 {
            let text = tree.get(prev).and_then(|n| n.text_content()).unwrap();
            return DomCursor {
                node_id: prev,
                offset: Self::find_word_start(text, len),
            };
        }
        DomCursor {
            node_id: prev,
            offset: len,
        }
    }

    /// Move cursor right by one character (or one word if `word` is true).
    fn move_cursor_right(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
        word: bool,
    ) -> DomCursor {
        // Element cursor (empty block) — move to start of next node
        if Self::is_element_cursor(tree, &cursor) {
            if let Some(next) = Self::next_text_node(tree, ce_root, cursor.node_id) {
                return DomCursor {
                    node_id: next,
                    offset: 0,
                };
            }
            return cursor;
        }

        if let Some(node) = tree.get(cursor.node_id)
            && let Some(text) = node.text_content()
            && cursor.offset < text.len()
        {
            if word {
                let new_off = Self::find_word_end(text, cursor.offset);
                return DomCursor {
                    node_id: cursor.node_id,
                    offset: new_off,
                };
            }
            let new_off = text[cursor.offset..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| cursor.offset + i)
                .unwrap_or(text.len());
            return DomCursor {
                node_id: cursor.node_id,
                offset: new_off,
            };
        }
        // At end of node (or on a <br>) — move to next position
        let cursor_is_br = tree
            .get(cursor.node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "br")
            .unwrap_or(false);

        let Some(next) = Self::next_text_node(tree, ce_root, cursor.node_id) else {
            return cursor;
        };
        let next_is_br = tree
            .get(next)
            .and_then(|n| n.tag())
            .map(|t| t == "br")
            .unwrap_or(false);

        if next_is_br && !cursor_is_br && !word {
            // At end of text node, next is a <br> (line terminator).
            // Skip the line-terminator <br> and land on whatever follows.
            let Some(after) = Self::next_text_node(tree, ce_root, next) else {
                return cursor;
            };
            if word && let Some(text) = tree.get(after).and_then(|n| n.text_content()) {
                return DomCursor {
                    node_id: after,
                    offset: Self::find_word_end(text, 0),
                };
            }
            return DomCursor {
                node_id: after,
                offset: 0,
            };
        }

        if next_is_br && word {
            if cursor_is_br {
                // Already on a blank line, next is another <br> — stop there
                return DomCursor {
                    node_id: next,
                    offset: 0,
                };
            }
            // Cursor on text, next is <br>. Check what follows the <br>.
            let after = Self::next_text_node(tree, ce_root, next);
            let after_is_br = after
                .and_then(|id| tree.get(id))
                .and_then(|n| n.tag())
                .map(|t| t == "br")
                .unwrap_or(false);
            if after_is_br {
                // Blank line ahead — stop at it
                return DomCursor {
                    node_id: after.unwrap(),
                    offset: 0,
                };
            }
            // Line terminator — skip and proceed to next text
            if let Some(after_id) = after
                && let Some(text) = tree.get(after_id).and_then(|n| n.text_content())
            {
                return DomCursor {
                    node_id: after_id,
                    offset: Self::find_word_end(text, 0),
                };
            }
            return cursor;
        }

        // Next is a text node (or we're on a <br> and next is whatever)
        if word && let Some(text) = tree.get(next).and_then(|n| n.text_content()) {
            return DomCursor {
                node_id: next,
                offset: Self::find_word_end(text, 0),
            };
        }
        DomCursor {
            node_id: next,
            offset: 0,
        }
    }

    /// Move cursor up or down by one line using Parley layout.
    fn move_cursor_vertical(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
        direction: i32, // -1 = up, +1 = down
    ) -> DomCursor {
        // <br> cursors are inline — skip the element cursor path and use IFC movement
        let is_br_cursor = tree
            .get(cursor.node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "br")
            .unwrap_or(false);

        // Element cursor (empty block) — jump to adjacent block
        if !is_br_cursor && Self::is_element_cursor(tree, &cursor) {
            let upward = direction < 0;
            // Walk up the ancestor chain trying each level
            let mut walk_id = cursor.node_id;
            while walk_id != ce_root {
                if let Some(result) =
                    Self::move_to_adjacent_block(tree, ce_root, walk_id, 0.0, upward)
                {
                    return result;
                }
                match tree.get(walk_id).and_then(|n| n.parent) {
                    Some(pid) => walk_id = pid,
                    None => break,
                }
            }
            return cursor;
        }

        // Find the IFC containing this cursor's node
        if let Some((ifc_root_id, inline_layout)) =
            Self::find_ifc_for_node(tree, ce_root, cursor.node_id)
        {
            let ranges = &inline_layout.text_ranges;
            if let Some(ifc_offset) = rinch_dom::text_query::dom_cursor_to_ifc_offset(
                ranges,
                cursor.node_id,
                cursor.offset,
            ) {
                let layout = &inline_layout.layout;

                // Get current caret geometry
                let parley_cursor = parley::Cursor::from_byte_index(
                    layout,
                    ifc_offset,
                    parley::layout::Affinity::Downstream,
                );
                let geom = parley_cursor.geometry(layout, 0.0);
                let x = geom.x0 as f32;

                // Find current line and target line
                let lines: Vec<_> = layout.lines().collect();
                let mut current_line_idx = 0;
                for (i, _line) in lines.iter().enumerate() {
                    // Check if offset falls in this line by comparing with next line's text start
                    if i == lines.len() - 1 || ifc_offset < lines[i + 1].text_range().start {
                        current_line_idx = i;
                        break;
                    }
                }

                let target_line_idx = if direction < 0 {
                    if current_line_idx == 0 {
                        // Already on first line — walk up ancestor chain
                        let upward = true;
                        let mut walk_id = ifc_root_id;
                        while walk_id != ce_root {
                            if let Some(result) =
                                Self::move_to_adjacent_block(tree, ce_root, walk_id, x, upward)
                            {
                                return result;
                            }
                            match tree.get(walk_id).and_then(|n| n.parent) {
                                Some(pid) => walk_id = pid,
                                None => break,
                            }
                        }
                        return cursor;
                    }
                    current_line_idx - 1
                } else {
                    if current_line_idx >= lines.len() - 1 {
                        // Already on last line — walk up ancestor chain
                        let upward = false;
                        let mut walk_id = ifc_root_id;
                        while walk_id != ce_root {
                            if let Some(result) =
                                Self::move_to_adjacent_block(tree, ce_root, walk_id, x, upward)
                            {
                                return result;
                            }
                            match tree.get(walk_id).and_then(|n| n.parent) {
                                Some(pid) => walk_id = pid,
                                None => break,
                            }
                        }
                        return cursor;
                    }
                    current_line_idx + 1
                };

                // Get target line's y position and use from_point
                let target_line = &lines[target_line_idx];
                let target_metrics = target_line.metrics();
                let target_y =
                    target_metrics.baseline - target_metrics.ascent + target_metrics.ascent * 0.5;

                let new_parley_cursor = parley::Cursor::from_point(layout, x, target_y);
                let new_ifc_offset = new_parley_cursor.index();

                if let Some((nid, off)) =
                    rinch_dom::text_query::ifc_offset_to_dom_cursor(ranges, new_ifc_offset, true)
                {
                    return DomCursor {
                        node_id: nid,
                        offset: off,
                    };
                }
            }
        }

        // Fallback for blocks without IFC (cached_text_parley)
        // Get x position from cached layout, then jump to adjacent block
        if let Some(node) = tree.get(cursor.node_id)
            && let Some(ref cached_layout) = node.cached_text_parley
        {
            let (x, _y) = rinch_dom::text_query::caret_position_for_offset_layout(
                cached_layout,
                cursor.offset,
            );
            // Walk up the ancestor chain from the text node's parent
            if let Some(parent_id) = node.parent {
                let upward = direction < 0;
                let mut walk_id = parent_id;
                while walk_id != ce_root {
                    if let Some(result) =
                        Self::move_to_adjacent_block(tree, ce_root, walk_id, x, upward)
                    {
                        return result;
                    }
                    match tree.get(walk_id).and_then(|n| n.parent) {
                        Some(pid) => walk_id = pid,
                        None => break,
                    }
                }
            }
        }

        cursor
    }

    /// Move cursor to Home (start of line).
    fn move_cursor_home(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        cursor: DomCursor,
    ) -> DomCursor {
        // Element cursor (empty block) — already at start
        if Self::is_element_cursor(tree, &cursor) {
            return cursor;
        }
        if let Some((_ifc_root_id, inline_layout)) =
            Self::find_ifc_for_node(tree, ce_root, cursor.node_id)
        {
            let ranges = &inline_layout.text_ranges;
            if let Some(ifc_offset) = rinch_dom::text_query::dom_cursor_to_ifc_offset(
                ranges,
                cursor.node_id,
                cursor.offset,
            ) {
                let layout = &inline_layout.layout;
                let parley_cursor = parley::Cursor::from_byte_index(
                    layout,
                    ifc_offset,
                    parley::layout::Affinity::Downstream,
                );
                let geom = parley_cursor.geometry(layout, 0.0);
                // Use from_point with x=0 to get line start
                let line_start_cursor =
                    parley::Cursor::from_point(layout, 0.0, geom.y0 as f32 + 1.0);
                let new_offset = line_start_cursor.index();
                if let Some((nid, off)) =
                    rinch_dom::text_query::ifc_offset_to_dom_cursor(ranges, new_offset, false)
                {
                    return DomCursor {
                        node_id: nid,
                        offset: off,
                    };
                }
            }
        }
        // Fallback for blocks without IFC — move to start of text node
        DomCursor {
            node_id: cursor.node_id,
            offset: 0,
        }
    }

    /// Move cursor to End (end of line).
    fn move_cursor_end(tree: &rinch_dom::NodeTree, ce_root: usize, cursor: DomCursor) -> DomCursor {
        // Element cursor (empty block) — already at end
        if Self::is_element_cursor(tree, &cursor) {
            return cursor;
        }
        if let Some((_ifc_root_id, inline_layout)) =
            Self::find_ifc_for_node(tree, ce_root, cursor.node_id)
        {
            let ranges = &inline_layout.text_ranges;
            if let Some(ifc_offset) = rinch_dom::text_query::dom_cursor_to_ifc_offset(
                ranges,
                cursor.node_id,
                cursor.offset,
            ) {
                let layout = &inline_layout.layout;
                let parley_cursor = parley::Cursor::from_byte_index(
                    layout,
                    ifc_offset,
                    parley::layout::Affinity::Downstream,
                );
                let geom = parley_cursor.geometry(layout, 0.0);
                // Use from_point with large x to get line end
                let line_end_cursor = parley::Cursor::from_point(layout, 1e6, geom.y0 as f32 + 1.0);
                let new_offset = line_end_cursor.index();
                if let Some((nid, off)) =
                    rinch_dom::text_query::ifc_offset_to_dom_cursor(ranges, new_offset, false)
                {
                    return DomCursor {
                        node_id: nid,
                        offset: off,
                    };
                }
            }
        }
        // Fallback for blocks without IFC — move to end of text node
        if let Some(node) = tree.get(cursor.node_id)
            && let Some(text) = node.text_content()
        {
            return DomCursor {
                node_id: cursor.node_id,
                offset: text.len(),
            };
        }
        cursor
    }

    /// Find the IFC (InlineLayout) that contains the given node.
    /// Returns (ifc_root_node_id, &InlineLayout).
    fn find_ifc_for_node(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        node_id: usize,
    ) -> Option<(usize, &rinch_dom::InlineLayout)> {
        // Walk up from node_id to find nearest ancestor with text_layout
        let mut current = Some(node_id);
        while let Some(nid) = current {
            if let Some(node) = tree.get(nid) {
                if let Some(layout) = node.text_layout.as_ref() {
                    return Some((nid, layout));
                }
                if nid == ce_root {
                    break;
                }
                current = node.parent;
            } else {
                break;
            }
        }
        // Also check parent if node is a text node
        if let Some(node) = tree.get(node_id)
            && let Some(parent_id) = node.parent
        {
            let mut current = Some(parent_id);
            while let Some(nid) = current {
                if let Some(pnode) = tree.get(nid) {
                    if let Some(layout) = pnode.text_layout.as_ref() {
                        return Some((nid, layout));
                    }
                    if nid == ce_root {
                        break;
                    }
                    current = pnode.parent;
                } else {
                    break;
                }
            }
        }
        None
    }

    /// Move cursor to adjacent block's IFC when at the top/bottom of current IFC.
    fn move_to_adjacent_block(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        current_ifc_root: usize,
        x: f32,
        upward: bool,
    ) -> Option<DomCursor> {
        // Find the parent of the IFC root, then find the adjacent sibling
        let parent_id = tree.get(current_ifc_root)?.parent?;

        // Ensure parent is within the CE boundary — walk up from parent to check
        // ce_root is an ancestor. This prevents escaping the CE when the IFC root
        // IS the CE div (inline-only CE like Test 1).
        let mut ancestor = Some(parent_id);
        let mut within_ce = false;
        while let Some(nid) = ancestor {
            if nid == ce_root {
                within_ce = true;
                break;
            }
            ancestor = tree.get(nid).and_then(|n| n.parent);
        }
        if !within_ce {
            return None;
        }

        let siblings = &tree.get(parent_id)?.children;
        let pos = siblings.iter().position(|&c| c == current_ifc_root)?;

        let adj_id = if upward {
            if pos == 0 {
                return None;
            }
            siblings[pos - 1]
        } else {
            if pos + 1 >= siblings.len() {
                return None;
            }
            siblings[pos + 1]
        };

        // Find IFC in the adjacent block
        if let Some(adj_node) = tree.get(adj_id) {
            if let Some(ref il) = adj_node.text_layout {
                // Adjacent block has full IFC
                let target_y = if upward {
                    let height = il.layout.height();
                    height - 1.0
                } else {
                    1.0
                };
                let new_cursor = parley::Cursor::from_point(&il.layout, x, target_y);
                if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                    &il.text_ranges,
                    new_cursor.index(),
                    true,
                ) {
                    return Some(DomCursor {
                        node_id: nid,
                        offset: off,
                    });
                }
            } else if adj_node.children.is_empty()
                && adj_node.tag().map(Self::is_block_element).unwrap_or(false)
            {
                // Empty block element — return element-level cursor
                return Some(DomCursor {
                    node_id: adj_id,
                    offset: 0,
                });
            } else {
                // Recursively find the first/last text cursor in the subtree
                return Self::find_cursor_in_subtree(tree, adj_id, x, upward);
            }
        }
        None
    }

    /// Recursively find a text cursor position within a subtree.
    /// When `upward` is true, finds the LAST text block (deepest last child);
    /// when false, finds the FIRST text block (deepest first child).
    fn find_cursor_in_subtree(
        tree: &rinch_dom::NodeTree,
        node_id: usize,
        x: f32,
        upward: bool,
    ) -> Option<DomCursor> {
        let node = tree.get(node_id)?;

        // Check if this node itself has an IFC
        if let Some(ref il) = node.text_layout {
            let target_y = if upward {
                il.layout.height() - 1.0
            } else {
                1.0
            };
            let new_cursor = parley::Cursor::from_point(&il.layout, x, target_y);
            if let Some((nid, off)) = rinch_dom::text_query::ifc_offset_to_dom_cursor(
                &il.text_ranges,
                new_cursor.index(),
                true,
            ) {
                return Some(DomCursor {
                    node_id: nid,
                    offset: off,
                });
            }
        }

        // Check if this node has cached_text_parley (direct text node)
        if let Some(ref cached_layout) = node.cached_text_parley {
            let target_y = if upward {
                cached_layout.height() - 1.0
            } else {
                1.0
            };
            let off = rinch_dom::text_query::byte_offset_from_position(cached_layout, x, target_y);
            return Some(DomCursor {
                node_id,
                offset: off,
            });
        }

        // Empty block element
        if node.children.is_empty() && node.tag().map(Self::is_block_element).unwrap_or(false) {
            return Some(DomCursor { node_id, offset: 0 });
        }

        // Bare text node (no cached layout — e.g. in a collapsed/virtualized block)
        if node.text_content().is_some() {
            let off = if upward {
                node.text_content().map(|t| t.len()).unwrap_or(0)
            } else {
                0
            };
            return Some(DomCursor {
                node_id,
                offset: off,
            });
        }

        // Recurse into children (last-to-first for upward, first-to-last for downward)
        let children = &node.children;
        if upward {
            for &child_id in children.iter().rev() {
                if let Some(result) = Self::find_cursor_in_subtree(tree, child_id, x, upward) {
                    return Some(result);
                }
            }
        } else {
            for &child_id in children.iter() {
                if let Some(result) = Self::find_cursor_in_subtree(tree, child_id, x, upward) {
                    return Some(result);
                }
            }
        }

        None
    }

    // ── DOM traversal helpers ────────────────────────────────────────────

    /// Find the previous text node (or `<br>`) in document order within the CE.
    pub(crate) fn prev_text_node(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        node_id: usize,
    ) -> Option<usize> {
        let mut all_text = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_text);
        let pos = all_text.iter().position(|&id| id == node_id)?;
        if pos > 0 {
            Some(all_text[pos - 1])
        } else {
            None
        }
    }

    /// Find the next text node (or `<br>`) in document order within the CE.
    pub(crate) fn next_text_node(
        tree: &rinch_dom::NodeTree,
        ce_root: usize,
        node_id: usize,
    ) -> Option<usize> {
        let mut all_text = Vec::new();
        Self::collect_text_node_ids(tree, ce_root, &mut all_text);
        let pos = all_text.iter().position(|&id| id == node_id)?;
        if pos + 1 < all_text.len() {
            Some(all_text[pos + 1])
        } else {
            None
        }
    }

    /// Collect all cursor-target node IDs in document order under `root`.
    /// Cursor targets are: text nodes, `<br>` elements (inline-only CE),
    /// and empty block elements (element cursors for blank lines).
    pub(crate) fn collect_text_node_ids(
        tree: &rinch_dom::NodeTree,
        root: usize,
        out: &mut Vec<usize>,
    ) {
        let Some(node) = tree.get(root) else { return };
        if node.text_content().is_some() {
            out.push(root);
            return;
        }
        if node.tag() == Some("br") {
            out.push(root);
            return;
        }
        // Empty block element — cursor target for element cursors
        if node.children.is_empty() && node.tag().map(Self::is_block_element).unwrap_or(false) {
            out.push(root);
            return;
        }
        for &child_id in &node.children {
            Self::collect_text_node_ids(tree, child_id, out);
        }
    }
}
