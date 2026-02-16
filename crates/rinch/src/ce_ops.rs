//! ContentEditable operations backed by a RinchDocument.
//!
//! Implements the [`ContentEditableApi`] trait from `rinch-core`, providing
//! a unified API for contentEditable DOM mutations. Created when a CE element
//! gains focus and registered via [`set_active_ce_api`] so both app.rs and
//! the editor bridge can access it.
//!
//! Currently provides real implementations for selection management and
//! event dispatching. DOM mutation methods (insert_text, delete_backward, etc.)
//! are handled by app.rs directly for now; these stubs dispatch CeEvents so
//! observers (the editor bridge) stay informed.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::ce::{
    CeEvent, CeEventDispatcher, CeSelection, ContentEditableApi, DomCursor, dispatch_ce_event,
};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

// ============================================================================
// DOM Helpers for Inline Formatting
// ============================================================================

/// Walk ancestors from `node_id` to find a formatting element with the given `tag`.
/// Stops at `stop_at_id` (the CE root).
fn find_formatting_ancestor(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    tag: &str,
    stop_at_id: usize,
) -> Option<usize> {
    let mut current = node_id;
    loop {
        let parent_id = tree.get(current)?.parent?;
        if parent_id == stop_at_id {
            return None;
        }
        if tree.get(parent_id)?.tag() == Some(tag) {
            return Some(parent_id);
        }
        current = parent_id;
    }
}

/// Collect all text node IDs in document order under `root`.
fn collect_text_nodes(tree: &rinch_dom::NodeTree, root: usize, out: &mut Vec<usize>) {
    let Some(node) = tree.get(root) else { return };
    if node.text_content().is_some() {
        out.push(root);
        return;
    }
    for &child_id in &node.children {
        collect_text_nodes(tree, child_id, out);
    }
}

/// Get text node IDs that fall within a cursor range (inclusive).
fn text_nodes_in_range(
    tree: &rinch_dom::NodeTree,
    ce_root: usize,
    start: DomCursor,
    end: DomCursor,
) -> Vec<usize> {
    let mut all = Vec::new();
    collect_text_nodes(tree, ce_root, &mut all);
    let s = all.iter().position(|&id| id == start.node_id).unwrap_or(0);
    let e = all
        .iter()
        .position(|&id| id == end.node_id)
        .unwrap_or(all.len().saturating_sub(1));
    all[s..=e].to_vec()
}

/// Order two cursors by document order (returns (start, end)).
fn order_cursors(
    tree: &rinch_dom::NodeTree,
    ce_root: usize,
    a: DomCursor,
    b: DomCursor,
) -> (DomCursor, DomCursor) {
    if a.node_id == b.node_id {
        return if a.offset <= b.offset { (a, b) } else { (b, a) };
    }
    let mut all = Vec::new();
    collect_text_nodes(tree, ce_root, &mut all);
    let a_pos = all.iter().position(|&id| id == a.node_id);
    let b_pos = all.iter().position(|&id| id == b.node_id);
    match (a_pos, b_pos) {
        (Some(ap), Some(bp)) if ap <= bp => (a, b),
        _ => (b, a),
    }
}

/// Get the next sibling of `child_id` within `parent_id`.
fn next_sibling(tree: &rinch_dom::NodeTree, parent_id: usize, child_id: usize) -> Option<usize> {
    let parent = tree.get(parent_id)?;
    let pos = parent.children.iter().position(|&c| c == child_id)?;
    parent.children.get(pos + 1).copied()
}

/// Move all children of `element_id` to its parent (before the element), then remove it.
fn unwrap_element(d: &mut RinchDocument, element_id: usize) {
    let parent_id = match d.tree.get(element_id).and_then(|n| n.parent) {
        Some(p) => p,
        None => return,
    };
    let children: Vec<usize> = d.tree.get(element_id).map(|n| n.children.clone()).unwrap_or_default();
    for &child_id in &children {
        d.remove_node(rinch_core::dom::NodeId(child_id));
        d.insert_before(
            rinch_core::dom::NodeId(parent_id),
            rinch_core::dom::NodeId(child_id),
            rinch_core::dom::NodeId(element_id),
        );
    }
    d.remove_node(rinch_core::dom::NodeId(element_id));
}

// ============================================================================
// CeOps
// ============================================================================

/// ContentEditable operations for a focused CE element.
///
/// Wraps a `RinchDocument` and cursor state. Implements [`ContentEditableApi`]
/// so the editor bridge can query selection state and (in future) perform
/// DOM mutations through the same API that app.rs uses.
pub struct CeOps {
    /// The backing DOM document.
    doc: Rc<RefCell<RinchDocument>>,
    /// The CE root node ID.
    ce_node_id: usize,
    /// Current cursor (head of selection).
    cursor: DomCursor,
    /// Selection anchor.
    anchor: DomCursor,
    /// Per-instance event dispatcher.
    dispatcher: CeEventDispatcher,
}

impl CeOps {
    /// Create a new CeOps for a focused contentEditable element.
    pub fn new(
        doc: Rc<RefCell<RinchDocument>>,
        ce_node_id: usize,
        cursor: DomCursor,
    ) -> Self {
        Self {
            doc,
            ce_node_id,
            cursor,
            anchor: cursor,
            dispatcher: CeEventDispatcher::new(),
        }
    }

    /// Get the CE root node ID.
    pub fn ce_node_id(&self) -> usize {
        self.ce_node_id
    }

    /// Update cursor/anchor from app.rs after it handles input directly.
    ///
    /// Called by app.rs to keep CeOps in sync after operations that
    /// app.rs handles inline (e.g., text insertion, deletion, mouse clicks).
    pub fn sync_cursor(&mut self, cursor: DomCursor, anchor: DomCursor) {
        self.cursor = cursor;
        self.anchor = anchor;
    }
}

impl std::fmt::Debug for CeOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CeOps")
            .field("ce_node_id", &self.ce_node_id)
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .finish()
    }
}

impl ContentEditableApi for CeOps {
    // ── Text Operations ──────────────────────────────────────────────

    fn insert_text(&mut self, text: &str) {
        let cur = self.cursor;
        {
            let mut d = self.doc.borrow_mut();
            if let Some(node) = d.tree.get(cur.node_id)
                && let Some(current) = node.text_content().map(|s| s.to_string())
            {
                let off = cur.offset.min(current.len());
                let mut new_text = String::with_capacity(current.len() + text.len());
                new_text.push_str(&current[..off]);
                new_text.push_str(text);
                new_text.push_str(&current[off..]);
                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                self.cursor = DomCursor::new(cur.node_id, off + text.len());
                self.anchor = self.cursor;
            } else {
                // Cursor is on an element node — create text child
                let text_id = d.create_text(text);
                d.append_child(rinch_core::dom::NodeId(cur.node_id), text_id);
                self.cursor = DomCursor::new(text_id.0, text.len());
                self.anchor = self.cursor;
            }
        }
        dispatch_ce_event(&CeEvent::TextInserted {
            node_id: self.cursor.node_id,
            offset: self.cursor.offset.saturating_sub(text.len()),
            text: text.to_string(),
        });
    }

    fn delete_backward(&mut self) {
        if self.cursor != self.anchor {
            self.delete_selection();
            return;
        }
        let cur = self.cursor;
        let deleted = {
            let mut d = self.doc.borrow_mut();
            if let Some(node) = d.tree.get(cur.node_id)
                && let Some(current) = node.text_content().map(|s| s.to_string())
                && cur.offset > 0
            {
                // Find the previous character boundary
                let prev = current[..cur.offset]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let deleted_len = cur.offset - prev;
                let mut new_text = String::with_capacity(current.len() - deleted_len);
                new_text.push_str(&current[..prev]);
                new_text.push_str(&current[cur.offset..]);
                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                self.cursor = DomCursor::new(cur.node_id, prev);
                self.anchor = self.cursor;
                Some((prev, deleted_len))
            } else {
                None
            }
        };
        if let Some((offset, length)) = deleted {
            dispatch_ce_event(&CeEvent::TextDeleted {
                node_id: cur.node_id,
                offset,
                length,
            });
        }
    }

    fn delete_forward(&mut self) {
        if self.cursor != self.anchor {
            self.delete_selection();
            return;
        }
        let cur = self.cursor;
        let deleted = {
            let mut d = self.doc.borrow_mut();
            if let Some(node) = d.tree.get(cur.node_id)
                && let Some(current) = node.text_content().map(|s| s.to_string())
                && cur.offset < current.len()
            {
                // Find the next character boundary
                let next = current[cur.offset..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| cur.offset + i)
                    .unwrap_or(current.len());
                let deleted_len = next - cur.offset;
                let mut new_text = String::with_capacity(current.len() - deleted_len);
                new_text.push_str(&current[..cur.offset]);
                new_text.push_str(&current[next..]);
                d.set_text_content(rinch_core::dom::NodeId(cur.node_id), &new_text);
                // Cursor stays at same position
                Some(deleted_len)
            } else {
                None
            }
        };
        if let Some(length) = deleted {
            dispatch_ce_event(&CeEvent::TextDeleted {
                node_id: cur.node_id,
                offset: cur.offset,
                length,
            });
        }
    }

    fn delete_selection(&mut self) {
        if self.cursor == self.anchor {
            return;
        }
        // Same-node deletion (simple case)
        let (start, end) = if self.cursor.node_id == self.anchor.node_id {
            let s = self.cursor.offset.min(self.anchor.offset);
            let e = self.cursor.offset.max(self.anchor.offset);
            (
                DomCursor::new(self.cursor.node_id, s),
                DomCursor::new(self.cursor.node_id, e),
            )
        } else {
            // Cross-node: use cursor order (approximate — full ordering needs DOM walk)
            (self.anchor, self.cursor)
        };

        if start.node_id == end.node_id {
            let mut d = self.doc.borrow_mut();
            if let Some(node) = d.tree.get(start.node_id)
                && let Some(text) = node.text_content().map(|s| s.to_string())
            {
                let s = start.offset.min(text.len());
                let e = end.offset.min(text.len());
                let mut new_text = String::with_capacity(text.len() - (e - s));
                new_text.push_str(&text[..s]);
                new_text.push_str(&text[e..]);
                d.set_text_content(rinch_core::dom::NodeId(start.node_id), &new_text);
            }
        }
        // For cross-node deletion, app.rs handles the complex logic.
        // Update cursor to collapse at start.
        self.cursor = start;
        self.anchor = start;
    }

    // ── Block Structure ──────────────────────────────────────────────

    fn split_block(&mut self) {
        // Block splitting is complex and handled by:
        // - app.rs for raw CE (InsertNewline command)
        // - editor bridge for rich-text (structure commands)
        // Dispatch event for observers.
        dispatch_ce_event(&CeEvent::BlockSplit {
            original_block_id: self.ce_node_id,
            new_block_id: 0, // Filled in by actual implementation
            split_offset: self.cursor.offset,
        });
    }

    fn set_block_type(&mut self, tag: &str) {
        // Handled by editor bridge's command system.
        dispatch_ce_event(&CeEvent::BlockTypeChanged {
            old_node_id: 0,
            new_node_id: 0,
            old_tag: String::new(),
            new_tag: tag.to_string(),
        });
    }

    // ── Inline Formatting ────────────────────────────────────────────

    fn wrap_selection(&mut self, tag: &str) {
        if self.cursor == self.anchor {
            return;
        }
        let (start, end) = order_cursors(&self.doc.borrow().tree, self.ce_node_id, self.anchor, self.cursor);
        let mut d = self.doc.borrow_mut();
        let selected_ids = text_nodes_in_range(&d.tree, self.ce_node_id, start, end);
        let mut wrapped_ids = Vec::new();
        let mut last_wrapper = 0;

        for &nid in &selected_ids {
            if find_formatting_ancestor(&d.tree, nid, tag, self.ce_node_id).is_none()
                && d.tree.get(nid).is_some()
            {
                let parent_id = d.tree.get(nid).and_then(|n| n.parent).unwrap_or(self.ce_node_id);
                let wrapper = d.create_element(tag);
                d.insert_before(
                    rinch_core::dom::NodeId(parent_id),
                    wrapper,
                    rinch_core::dom::NodeId(nid),
                );
                d.remove_node(rinch_core::dom::NodeId(nid));
                d.append_child(wrapper, rinch_core::dom::NodeId(nid));
                last_wrapper = wrapper.0;
                wrapped_ids.push(nid);
            }
        }
        drop(d);
        if !wrapped_ids.is_empty() {
            dispatch_ce_event(&CeEvent::SelectionWrapped {
                tag: tag.to_string(),
                wrapper_node_id: last_wrapper,
                wrapped_node_ids: wrapped_ids,
            });
        }
    }

    fn unwrap_selection(&mut self, tag: &str) {
        if self.cursor == self.anchor {
            return;
        }
        let (start, end) = order_cursors(&self.doc.borrow().tree, self.ce_node_id, self.anchor, self.cursor);
        let mut d = self.doc.borrow_mut();
        let selected_ids = text_nodes_in_range(&d.tree, self.ce_node_id, start, end);

        // Collect unique formatting ancestors, then unwrap each.
        let mut fmt_ids = Vec::new();
        for &nid in &selected_ids {
            if let Some(fmt_id) = find_formatting_ancestor(&d.tree, nid, tag, self.ce_node_id) {
                if !fmt_ids.contains(&fmt_id) {
                    fmt_ids.push(fmt_id);
                }
            }
        }
        for &fmt_id in &fmt_ids {
            if d.tree.get(fmt_id).is_some() {
                unwrap_element(&mut d, fmt_id);
            }
        }
        drop(d);
        if !fmt_ids.is_empty() {
            dispatch_ce_event(&CeEvent::SelectionUnwrapped {
                tag: tag.to_string(),
                unwrapped_node_ids: selected_ids,
            });
        }
    }

    fn toggle_wrap(&mut self, tag: &str) {
        let has_selection = self.cursor != self.anchor;

        if !has_selection {
            // ── Cursor-only: enter or escape formatting ──
            let mut d = self.doc.borrow_mut();
            let fmt_ancestor =
                find_formatting_ancestor(&d.tree, self.cursor.node_id, tag, self.ce_node_id);

            if let Some(fmt_id) = fmt_ancestor {
                // Already inside formatting — escape it.
                // Place cursor in a text node after the formatting element.
                let parent_id = d
                    .tree
                    .get(fmt_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(self.ce_node_id);
                let next_sib = next_sibling(&d.tree, parent_id, fmt_id);
                let escape_id =
                    if let Some(next_id) = next_sib
                        && d.tree.get(next_id).and_then(|n| n.text_content()).is_some()
                    {
                        next_id
                    } else {
                        let zws = d.create_text("\u{200B}");
                        if let Some(next_id) = next_sib {
                            d.insert_before(
                                rinch_core::dom::NodeId(parent_id),
                                zws,
                                rinch_core::dom::NodeId(next_id),
                            );
                        } else {
                            d.append_child(rinch_core::dom::NodeId(parent_id), zws);
                        }
                        zws.0
                    };
                self.cursor = DomCursor::new(escape_id, 0);
                self.anchor = self.cursor;
            } else {
                // Not inside formatting — enter it.
                // Create a formatting wrapper with a zero-width space text node.
                let wrapper = d.create_element(tag);
                let inner = d.create_text("\u{200B}");
                d.append_child(wrapper, inner);

                if let Some(node) = d.tree.get(self.cursor.node_id)
                    && node.text_content().is_some()
                {
                    let text = node.text_content().unwrap().to_string();
                    let off = self.cursor.offset.min(text.len());
                    let parent_id = node.parent.unwrap_or(self.ce_node_id);

                    if off == 0 {
                        d.insert_before(
                            rinch_core::dom::NodeId(parent_id),
                            wrapper,
                            rinch_core::dom::NodeId(self.cursor.node_id),
                        );
                    } else if off >= text.len() {
                        let next = next_sibling(&d.tree, parent_id, self.cursor.node_id);
                        if let Some(next_id) = next {
                            d.insert_before(
                                rinch_core::dom::NodeId(parent_id),
                                wrapper,
                                rinch_core::dom::NodeId(next_id),
                            );
                        } else {
                            d.append_child(rinch_core::dom::NodeId(parent_id), wrapper);
                        }
                    } else {
                        // Split text node: keep [0..off], insert wrapper, then [off..]
                        let after_text = text[off..].to_string();
                        d.set_text_content(
                            rinch_core::dom::NodeId(self.cursor.node_id),
                            &text[..off],
                        );
                        let after = d.create_text(&after_text);
                        let next = next_sibling(&d.tree, parent_id, self.cursor.node_id);
                        if let Some(next_id) = next {
                            d.insert_before(
                                rinch_core::dom::NodeId(parent_id),
                                wrapper,
                                rinch_core::dom::NodeId(next_id),
                            );
                            d.insert_before(
                                rinch_core::dom::NodeId(parent_id),
                                after,
                                rinch_core::dom::NodeId(next_id),
                            );
                        } else {
                            d.append_child(rinch_core::dom::NodeId(parent_id), wrapper);
                            d.append_child(rinch_core::dom::NodeId(parent_id), after);
                        }
                    }
                } else {
                    // Element cursor (empty block): append wrapper inside the element
                    d.append_child(rinch_core::dom::NodeId(self.cursor.node_id), wrapper);
                    d.set_style(
                        rinch_core::dom::NodeId(self.cursor.node_id),
                        "min-height",
                        "0",
                    );
                }
                self.cursor = DomCursor::new(inner.0, 0);
                self.anchor = self.cursor;
            }
        } else {
            // ── Selection: check if all selected text is formatted ──
            let all_formatted = {
                let d = self.doc.borrow();
                let (start, end) =
                    order_cursors(&d.tree, self.ce_node_id, self.anchor, self.cursor);
                let ids = text_nodes_in_range(&d.tree, self.ce_node_id, start, end);
                !ids.is_empty()
                    && ids.iter().all(|&nid| {
                        find_formatting_ancestor(&d.tree, nid, tag, self.ce_node_id).is_some()
                    })
            };
            if all_formatted {
                self.unwrap_selection(tag);
            } else {
                self.wrap_selection(tag);
            }
        }
    }

    // ── List Operations ──────────────────────────────────────────────

    fn indent(&mut self) {
        // Handled by editor bridge's command system.
    }

    fn outdent(&mut self) {
        // Handled by editor bridge's command system.
    }

    // ── Selection ────────────────────────────────────────────────────

    fn get_selection(&self) -> CeSelection {
        if self.cursor == self.anchor {
            CeSelection::collapsed(self.cursor)
        } else {
            CeSelection::range(self.anchor, self.cursor)
        }
    }

    fn set_selection(&mut self, sel: CeSelection) {
        self.anchor = sel.anchor;
        self.cursor = sel.head;
        dispatch_ce_event(&CeEvent::SelectionChanged { selection: sel });
    }

    // ── Undo/Redo ────────────────────────────────────────────────────

    fn undo(&mut self) {
        // Undo stack is managed by app.rs (ContentEditableFocus.undo_stack)
        // or by the editor bridge (Editor.history). This dispatches the event
        // so observers can react.
        dispatch_ce_event(&CeEvent::UndoApplied);
    }

    fn redo(&mut self) {
        dispatch_ce_event(&CeEvent::RedoApplied);
    }

    // ── Event Access ─────────────────────────────────────────────────

    fn event_dispatcher(&self) -> &CeEventDispatcher {
        &self.dispatcher
    }

    fn event_dispatcher_mut(&mut self) -> &mut CeEventDispatcher {
        &mut self.dispatcher
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // CeOps needs a RinchDocument which requires rinch-dom test infrastructure.
    // Basic API surface tests here; integration tests in Task #20.

    #[test]
    fn get_selection_collapsed() {
        // Verify the selection API works without a real document
        let cursor = DomCursor::new(10, 5);
        let sel = CeSelection::collapsed(cursor);
        assert!(sel.is_collapsed());
        assert_eq!(sel.head, cursor);
        assert_eq!(sel.anchor, cursor);
    }

    #[test]
    fn get_selection_range() {
        let anchor = DomCursor::new(10, 0);
        let head = DomCursor::new(10, 5);
        let sel = CeSelection::range(anchor, head);
        assert!(!sel.is_collapsed());
    }
}
