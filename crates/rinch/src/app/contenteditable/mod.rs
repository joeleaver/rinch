mod ce_blocks;
mod ce_cursor;
mod ce_helpers;
mod ce_navigation;
mod ce_paste;
mod ce_selection;

use super::*;
use ce_selection::compute_ce_scroll_target;

// ── ContentEditable focus ────────────────────────────────────────────────────

/// A cursor position within the DOM: a specific text node and byte offset,
/// or a block element ID for empty blocks (offset always 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct DomCursor {
    /// DOM node ID — either a text node or an empty block element.
    pub(in crate::app) node_id: usize,
    /// Byte offset within the text node's content (always 0 for element cursors).
    pub(in crate::app) offset: usize,
}

/// A snapshot of text node contents for undo.
#[derive(Debug, Clone)]
pub(in crate::app) struct UndoEntry {
    pub(in crate::app) cursor: DomCursor,
    pub(in crate::app) anchor: DomCursor,
    pub(in crate::app) text_snapshots: Vec<(usize, String)>, // (node_id, old_text_content)
    pub(in crate::app) created_nodes: Vec<usize>,            // nodes created during the edit (removed on undo)
}

/// State for a focused contenteditable element.
pub(crate) struct ContentEditableFocus {
    /// The node ID of the focused contenteditable root element.
    pub(in crate::app) ce_node_id: usize,
    /// Caret position.
    pub(in crate::app) cursor: DomCursor,
    /// Selection anchor (same as cursor when no selection).
    pub(in crate::app) anchor: DomCursor,
    /// Input handler for mapping keys to edit commands (from rinch_editable).
    pub(in crate::app) input_handler: InputHandler,
    /// Undo stack for text changes.
    pub(in crate::app) undo_stack: Vec<UndoEntry>,
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
    /// Set/clear contenteditable cursor attributes on a DOM node.
    ///
    /// Converts `DomCursor` values to global flat offsets for paint compatibility.
    pub(in crate::app) fn set_contenteditable_attributes_dom(
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
    pub(in crate::app) fn set_contenteditable_attributes(
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
    pub(in crate::app) fn handle_contenteditable_key(
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

