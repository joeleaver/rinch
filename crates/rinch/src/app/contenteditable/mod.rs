mod ce_blocks;
mod ce_cursor;
mod ce_helpers;
mod ce_navigation;
mod ce_paste;
mod ce_selection;

use super::*;
use ce_selection::compute_ce_scroll_target;

// ── ContentEditable focus ────────────────────────────────────────────────────

/// Re-use the public DomCursor from rinch_core::ce.
pub(crate) use rinch_core::ce::DomCursor;

/// A snapshot of text node contents for undo.
#[derive(Debug, Clone)]
pub(in crate::app) struct UndoEntry {
    pub(in crate::app) cursor: DomCursor,
    pub(in crate::app) anchor: DomCursor,
    pub(in crate::app) text_snapshots: Vec<(usize, String)>, // (node_id, old_text_content)
    pub(in crate::app) created_nodes: Vec<usize>, // nodes created during the edit (removed on undo)
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
    pub(in crate::app) undo_stack: std::collections::VecDeque<UndoEntry>,
    /// Redo stack for undone changes.
    pub(in crate::app) redo_stack: std::collections::VecDeque<UndoEntry>,
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
        #[allow(unused)] // used in Copy/Cut under cfg(clipboard)
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
        let mut pre_edit_ids: std::collections::HashSet<usize> = std::collections::HashSet::new();
        if is_mutating && let Some(doc) = &self.doc {
            let d = doc.borrow();
            let snapshots = Self::snapshot_text_nodes(&d.tree, ce_node_id);
            pre_edit_ids = Self::collect_subtree_ids(&d.tree, ce_node_id).into_iter().collect();
            let ce = self.focused_contenteditable.as_mut().unwrap();
            ce.undo_stack.push_back(UndoEntry {
                cursor,
                anchor,
                text_snapshots: snapshots,
                created_nodes: Vec::new(),
            });
            // Cap undo stack at 100 entries
            if ce.undo_stack.len() > 100 {
                ce.undo_stack.pop_front();
            }
            ce.redo_stack.clear();
        }

        match cmd {
            // ── Character insertion / paste ──────────────────────────
            EditCommand::InsertText(ref s) | EditCommand::Paste(ref s) => {
                self.sync_ce_ops_cursor();
                if let Some(ops) = &self.ce_ops
                    && let Ok(mut ops) = ops.try_borrow_mut()
                {
                    ops.insert_text(s);
                    let sel = ops.get_selection();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = sel.head;
                    ce.anchor = sel.anchor;
                }
                text_changed = true;
            }

            // ── Backspace ────────────────────────────────────────────
            EditCommand::DeleteBackward => {
                self.sync_ce_ops_cursor();
                if let Some(ops) = &self.ce_ops
                    && let Ok(mut ops) = ops.try_borrow_mut()
                {
                    ops.delete_backward();
                    let sel = ops.get_selection();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = sel.head;
                    ce.anchor = sel.anchor;
                }
                text_changed = true;
            }

            // ── Delete ───────────────────────────────────────────────
            EditCommand::DeleteForward => {
                self.sync_ce_ops_cursor();
                if let Some(ops) = &self.ce_ops
                    && let Ok(mut ops) = ops.try_borrow_mut()
                {
                    ops.delete_forward();
                    let sel = ops.get_selection();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = sel.head;
                    ce.anchor = sel.anchor;
                }
                text_changed = true;
            }

            // ── Enter ────────────────────────────────────────────────
            EditCommand::InsertNewline => {
                self.sync_ce_ops_cursor();
                if let Some(ops) = &self.ce_ops
                    && let Ok(mut ops) = ops.try_borrow_mut()
                {
                    ops.split_block();
                    let sel = ops.get_selection();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = sel.head;
                    ce.anchor = sel.anchor;
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
                    self.sync_ce_ops_cursor();
                    if let Some(ops) = &self.ce_ops
                        && let Ok(mut ops) = ops.try_borrow_mut()
                    {
                        ops.delete_selection();
                        let sel = ops.get_selection();
                        let ce = self.focused_contenteditable.as_mut().unwrap();
                        ce.cursor = sel.head;
                        ce.anchor = sel.anchor;
                    }
                    text_changed = true;
                }
            }

            // ── Undo ──────────────────────────────────────────────────
            EditCommand::Undo => {
                let ce = self.focused_contenteditable.as_mut().unwrap();
                if let Some(entry) = ce.undo_stack.pop_back() {
                    // Snapshot current state for redo before restoring
                    if let Some(doc) = &self.doc {
                        let d = doc.borrow();
                        let current_snapshots = Self::snapshot_text_nodes(&d.tree, ce_node_id);
                        ce.redo_stack.push_back(UndoEntry {
                            cursor: ce.cursor,
                            anchor: ce.anchor,
                            text_snapshots: current_snapshots,
                            created_nodes: entry.created_nodes.clone(),
                        });
                    }

                    let restore_cursor = entry.cursor;
                    let restore_anchor = entry.anchor;
                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        for &node_id in &entry.created_nodes {
                            if d.tree.get(node_id).is_some() {
                                d.remove_node(rinch_core::dom::NodeId(node_id));
                            }
                        }
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
                self.sync_ce_ops_cursor();
                if let Some(ops) = &self.ce_ops
                    && let Ok(mut ops) = ops.try_borrow_mut()
                {
                    ops.indent();
                    let sel = ops.get_selection();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = sel.head;
                    ce.anchor = sel.anchor;
                }
                text_changed = true;
            }

            // ── Shift+Tab outdent ────────────────────────────────────────
            EditCommand::Outdent => {
                self.sync_ce_ops_cursor();
                if let Some(ops) = &self.ce_ops
                    && let Ok(mut ops) = ops.try_borrow_mut()
                {
                    ops.outdent();
                    let sel = ops.get_selection();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = sel.head;
                    ce.anchor = sel.anchor;
                }
                text_changed = true;
            }

            // ── Inline formatting ─────────────────────────────────────
            rinch_editable::EditCommand::ToggleBold
            | rinch_editable::EditCommand::ToggleItalic
            | rinch_editable::EditCommand::ToggleUnderline
            | rinch_editable::EditCommand::ToggleStrikethrough
            | rinch_editable::EditCommand::ToggleCode => {
                let tag = match cmd {
                    rinch_editable::EditCommand::ToggleBold => "strong",
                    rinch_editable::EditCommand::ToggleItalic => "em",
                    rinch_editable::EditCommand::ToggleUnderline => "u",
                    rinch_editable::EditCommand::ToggleStrikethrough => "s",
                    rinch_editable::EditCommand::ToggleCode => "code",
                    _ => unreachable!(),
                };
                // Delegate to CeOps — the CE API owns formatting operations
                self.sync_ce_ops_cursor();
                if let Some(ops) = &self.ce_ops
                    && let Ok(mut ops) = ops.try_borrow_mut()
                {
                    ops.toggle_wrap(tag);
                    let sel = ops.get_selection();
                    let ce = self.focused_contenteditable.as_mut().unwrap();
                    ce.cursor = sel.head;
                    ce.anchor = sel.anchor;
                }
                text_changed = true;
            }

            // ── Redo ──────────────────────────────────────────────────
            EditCommand::Redo => {
                let ce = self.focused_contenteditable.as_mut().unwrap();
                if let Some(entry) = ce.redo_stack.pop_back() {
                    // Snapshot current state for undo before restoring
                    if let Some(doc) = &self.doc {
                        let d = doc.borrow();
                        let current_snapshots = Self::snapshot_text_nodes(&d.tree, ce_node_id);
                        ce.undo_stack.push_back(UndoEntry {
                            cursor: ce.cursor,
                            anchor: ce.anchor,
                            text_snapshots: current_snapshots,
                            created_nodes: entry.created_nodes.clone(),
                        });
                    }

                    let restore_cursor = entry.cursor;
                    let restore_anchor = entry.anchor;
                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        for &node_id in &entry.created_nodes {
                            if d.tree.get(node_id).is_some() {
                                d.remove_node(rinch_core::dom::NodeId(node_id));
                            }
                        }
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

            // ── Unhandled commands (Escape, etc.) ───────────────
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
                if let Some(entry) = ce.undo_stack.back_mut() {
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

        // Sync cursor state to CeOps so the bridge sees updated positions
        self.sync_ce_ops_cursor();

        // Dispatch CE event for the editor bridge
        {
            use rinch_core::ce::{self, CeEvent, CeSelection};
            let anchor = rinch_core::ce::DomCursor {
                node_id: final_anchor.node_id,
                offset: final_anchor.offset,
            };
            let head = rinch_core::ce::DomCursor {
                node_id: final_cursor.node_id,
                offset: final_cursor.offset,
            };
            ce::dispatch_ce_event(&CeEvent::SelectionChanged {
                selection: CeSelection::range(anchor, head),
            });
        }

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
