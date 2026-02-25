//! ContentEditable bridge: connects the editor framework to DOM rendering.
//!
//! The bridge layer:
//! 1. Reconciles the EditorDocument model into DOM nodes inside a contentEditable div
//! 2. Intercepts keyboard events to route through editor commands
//! 3. Provides cursor/selection state for toolbar active-state feedback
//!
//! ContentEditable handles cursor rendering, selection display, and hit testing
//! natively — the bridge only manages the document model and DOM reconciliation.

pub mod cursor_sync;
pub mod input;
pub mod reconciler;
pub mod view_desc;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::rc::Weak;

use crate::document::{MarkData, Position, Range};
use crate::editor::Editor;
use crate::selection::Selection;
use rinch_core::ce::{self as ce_api, CeEvent, DomCursor};
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};

use self::view_desc::ViewDesc;

/// Block cache: maps block index to cached block state (DOM node + hashes).
type BlockCache = Rc<RefCell<HashMap<usize, reconciler::BlockCacheEntry>>>;

/// Shared ViewDesc for bidirectional DOM ↔ model position mapping.
type SharedViewDesc = Rc<RefCell<ViewDesc>>;

/// The editor bridge connecting the editor framework to contentEditable rendering.
pub struct EditorBridge {
    /// The editor instance.
    editor: Rc<RefCell<Editor>>,
    /// The contentEditable root element.
    ce_root: NodeHandle,
    /// Cache of block DOM nodes keyed by index, with content hash for diffing.
    block_cache: BlockCache,
    /// Bidirectional mapping between DOM node IDs and editor positions.
    view_desc: SharedViewDesc,
    /// Weak reference to the DomDocument for creating child scopes.
    doc_weak: Weak<RefCell<dyn DomDocument>>,
    /// The container node ID for creating RenderScopes.
    container_id: NodeId,
}

impl EditorBridge {
    /// Mount the editor bridge onto a contentEditable element.
    ///
    /// This:
    /// 1. Registers keyboard and CE event interceptors
    /// 2. Performs initial DOM reconciliation (renders document into CE div)
    /// 3. Returns the bridge for future reconciliation calls
    pub fn mount(
        scope: &mut RenderScope,
        editor: Rc<RefCell<Editor>>,
        ce_root: NodeHandle,
        on_change: Rc<dyn Fn()>,
    ) -> Self {
        let doc_weak = scope.doc_weak();
        let container_id = ce_root.node_id();
        let block_cache: BlockCache = Rc::new(RefCell::new(HashMap::new()));
        let view_desc: SharedViewDesc = Rc::new(RefCell::new(ViewDesc::new()));

        let on_change_for_events = on_change.clone();

        // Build the reconcile-on-change callback for click/drag interceptors
        let reconcile_callback = {
            let editor_rc = editor.clone();
            let doc_weak_rc = doc_weak.clone();
            let cache_rc = block_cache.clone();
            let vd_rc = view_desc.clone();
            let ce_root_rc = ce_root.clone();
            let user_on_change = on_change.clone();

            Rc::new(move || {
                // Reconcile DOM after each command
                if let Some(doc) = doc_weak_rc.upgrade() {
                    let mut scope = RenderScope::new(doc, container_id);
                    if let Ok(ed) = editor_rc.try_borrow() {
                        let mut cache = cache_rc.borrow_mut();
                        let mut vd = vd_rc.borrow_mut();
                        reconciler::reconcile(
                            &mut scope,
                            &ce_root_rc,
                            &ed.doc,
                            &mut cache,
                            &ed.tables,
                            &mut vd,
                            ed.table_selection.as_ref(),
                        );
                        sync_cursor_attrs(&ce_root_rc, ed.get_selection());
                    }
                }
                user_on_change();
            })
        };

        // Install click/drag interceptors (keyboard is handled by app.rs → CeOps)
        input::install_interceptors(editor.clone(), reconcile_callback);

        // Subscribe to CE events from app.rs (observer pattern).
        // When app.rs handles input directly (mouse clicks, unintercepted keys),
        // it dispatches CeEvents. The bridge observes these to keep the editor
        // model synchronized.
        {
            let editor_for_events = editor.clone();
            let doc_weak_for_events = doc_weak.clone();
            let ce_root_for_events = ce_root.clone();
            let cache_for_events = block_cache.clone();
            let vd_for_events = view_desc.clone();

            ce_api::subscribe_ce_events(Rc::new(move |event: &CeEvent| {
                handle_ce_event(
                    event,
                    &editor_for_events,
                    &doc_weak_for_events,
                    &ce_root_for_events,
                    &cache_for_events,
                    &vd_for_events,
                    container_id,
                    &*on_change_for_events,
                );
            }));
        }

        let bridge = Self {
            editor,
            ce_root,
            block_cache,
            view_desc,
            doc_weak,
            container_id,
        };

        // Initial render
        bridge.full_reconcile(scope);

        bridge
    }

    /// Perform a full reconciliation (initial render or after undo/redo).
    pub fn full_reconcile(&self, scope: &mut RenderScope) {
        if let Ok(ed) = self.editor.try_borrow() {
            let mut cache = self.block_cache.borrow_mut();
            let mut vd = self.view_desc.borrow_mut();
            reconciler::full_render(
                scope,
                &self.ce_root,
                &ed.doc,
                &mut cache,
                &ed.tables,
                &mut vd,
                ed.table_selection.as_ref(),
            );
            sync_cursor_attrs(&self.ce_root, ed.get_selection());
        }
    }

    /// Reconcile after a command (incremental update).
    pub fn reconcile(&self) {
        if let Some(doc) = self.doc_weak.upgrade() {
            let mut scope = RenderScope::new(doc, self.container_id);
            if let Ok(ed) = self.editor.try_borrow() {
                let mut cache = self.block_cache.borrow_mut();
                let mut vd = self.view_desc.borrow_mut();
                reconciler::reconcile(
                    &mut scope,
                    &self.ce_root,
                    &ed.doc,
                    &mut cache,
                    &ed.tables,
                    &mut vd,
                    ed.table_selection.as_ref(),
                );
                sync_cursor_attrs(&self.ce_root, ed.get_selection());
            }
        }
    }

    /// Get active marks at the current cursor position.
    pub fn active_marks(&self) -> Vec<String> {
        if let Ok(ed) = self.editor.try_borrow() {
            cursor_sync::active_marks(&ed)
        } else {
            Vec::new()
        }
    }

    /// Get the active block type at the current cursor position.
    pub fn active_block_type(&self) -> Option<String> {
        if let Ok(ed) = self.editor.try_borrow() {
            cursor_sync::active_block_type(&ed)
        } else {
            None
        }
    }

    /// Unmount the bridge, removing all interceptors.
    pub fn unmount(&self) {
        ce_api::clear_ce_event_listeners();
        input::remove_interceptors();
    }
}

impl Drop for EditorBridge {
    fn drop(&mut self) {
        self.unmount();
    }
}

/// Find the block index for a DOM node by walking up to find `data-block-index`.
///
/// The reconciler tags each block element with a `data-block-index` attribute.
/// This walks from the given node up the tree until it finds that attribute,
/// providing a simple DOM-node-to-block-index lookup.
fn find_block_for_node(
    ce_root: &NodeHandle,
    doc_weak: &Weak<RefCell<dyn DomDocument>>,
    node_id: usize,
) -> Option<usize> {
    let mut current = NodeHandle::new(NodeId(node_id), doc_weak.clone());
    let ce_root_id = ce_root.node_id();

    loop {
        if let Some(attr) = current.get_attribute("data-block-index") {
            return attr.parse().ok();
        }
        if current.node_id() == ce_root_id {
            return None; // Reached CE root without finding a block
        }
        current = current.parent_node()?;
    }
}

/// Handle a CeEvent from app.rs.
///
/// This is the observer side of the CE API architecture. When app.rs handles
/// input (mouse clicks, text input not intercepted by the bridge), it dispatches
/// CeEvents. The bridge observes these to keep the EditorDocument in sync.
#[allow(clippy::too_many_arguments)]
fn handle_ce_event(
    event: &CeEvent,
    editor: &Rc<RefCell<Editor>>,
    doc_weak: &Weak<RefCell<dyn DomDocument>>,
    ce_root: &NodeHandle,
    block_cache: &BlockCache,
    view_desc: &SharedViewDesc,
    container_id: NodeId,
    on_change: &dyn Fn(),
) {
    match event {
        CeEvent::UndoApplied | CeEvent::RedoApplied => {
            // Undo/redo can affect any block — full reconcile required
            full_reconcile_helper(
                editor,
                doc_weak,
                ce_root,
                block_cache,
                view_desc,
                container_id,
            );
            on_change();
        }

        CeEvent::SelectionChanged { selection } => {
            // When app.rs repositions the cursor (e.g., mouse click),
            // update the editor's selection model so toolbar active state
            // and subsequent commands use the correct position.
            //
            // Use ViewDesc for accurate DomCursor → Position translation.
            // Falls back to find_block_for_node if ViewDesc has no mapping
            // (e.g., cursor lands on a non-text node not tracked by ViewDesc).
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let vd = view_desc.borrow();

                let head_pos = vd.dom_cursor_to_position(&selection.head, &ed).or_else(|| {
                    // Fallback: walk DOM to find block, use approximate offset
                    let block_index =
                        find_block_for_node(ce_root, doc_weak, selection.head.node_id)?;
                    let block_start = ed.doc.block_start_position(block_index);
                    let block_len = ed.doc.block_text(block_index).map(|t| t.len()).unwrap_or(0);
                    let offset = selection.head.offset.min(block_len);
                    Some(Position::new(
                        (block_start + offset).min(ed.doc.text_length()),
                    ))
                });

                if let Some(head_pos) = head_pos {
                    let anchor_pos = if selection.is_collapsed() {
                        head_pos
                    } else {
                        vd.dom_cursor_to_position(&selection.anchor, &ed)
                            .or_else(|| {
                                let block_index = find_block_for_node(
                                    ce_root,
                                    doc_weak,
                                    selection.anchor.node_id,
                                )?;
                                let anchor_start = ed.doc.block_start_position(block_index);
                                let anchor_len =
                                    ed.doc.block_text(block_index).map(|t| t.len()).unwrap_or(0);
                                let anchor_off = selection.anchor.offset.min(anchor_len);
                                Some(Position::new(
                                    (anchor_start + anchor_off).min(ed.doc.text_length()),
                                ))
                            })
                            .unwrap_or(head_pos)
                    };

                    ed.set_selection(Selection::new(anchor_pos, head_pos));
                    sync_cursor_attrs(ce_root, ed.get_selection());
                }
            }
        }

        // ── Text Events (incremental sync — no reconciler) ────────────
        CeEvent::TextInserted {
            node_id,
            offset,
            text,
        } => {
            // Incremental sync: insert text into EditorDocument model.
            // This is the hot path — no reconciler, no rerender.
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let vd = view_desc.borrow();
                let cursor = DomCursor::new(*node_id, *offset);
                if let Some(pos) = vd.dom_cursor_to_position(&cursor, &ed) {
                    let block_index = vd.block_index_for_node(*node_id);
                    // Convert text-node-local offset to block-level byte offset
                    let block_from_byte = vd
                        .text_node_byte_start(*node_id)
                        .map(|start| start + *offset)
                        .unwrap_or(*offset);
                    drop(vd);
                    if let Err(e) = ed.doc.insert_text(pos, text) {
                        eprintln!("[bridge] TextInserted: model insert failed: {e:?}");
                        return;
                    }
                    let new_pos = Position::new((pos.0 + text.len()).min(ed.doc.text_length()));
                    ed.set_selection(Selection::cursor(new_pos));
                    sync_cursor_attrs(ce_root, ed.get_selection());
                    drop(ed);
                    // Shift ViewDesc byte ranges for the affected block
                    if let Some(bi) = block_index {
                        view_desc
                            .borrow_mut()
                            .shift_block_ranges(bi, block_from_byte, text.len() as isize);
                    }
                    on_change();
                }
            }
        }

        CeEvent::TextDeleted {
            node_id,
            offset,
            length,
        } => {
            // Incremental sync: delete text from EditorDocument model.
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let vd = view_desc.borrow();
                let start_cursor = DomCursor::new(*node_id, *offset);
                let end_cursor = DomCursor::new(*node_id, offset + length);
                if let (Some(start_pos), Some(end_pos)) = (
                    vd.dom_cursor_to_position(&start_cursor, &ed),
                    vd.dom_cursor_to_position(&end_cursor, &ed),
                ) {
                    let block_index = vd.block_index_for_node(*node_id);
                    // Convert text-node-local offset to block-level byte offset
                    let block_from_byte = vd
                        .text_node_byte_start(*node_id)
                        .map(|start| start + *offset)
                        .unwrap_or(*offset);
                    drop(vd);
                    let range = Range::new(start_pos, end_pos);
                    if let Err(e) = ed.doc.delete_range(range) {
                        eprintln!("[bridge] TextDeleted: model delete failed: {e:?}");
                        return;
                    }
                    ed.set_selection(Selection::cursor(start_pos));
                    sync_cursor_attrs(ce_root, ed.get_selection());
                    drop(ed);
                    // Shift ViewDesc byte ranges for the affected block
                    if let Some(bi) = block_index {
                        view_desc
                            .borrow_mut()
                            .shift_block_ranges(bi, block_from_byte, -(*length as isize));
                    }
                    on_change();
                }
            }
        }

        // ── Block Structure Events (model sync + full reconcile) ──────
        CeEvent::BlockSplit {
            original_block_id,
            split_offset,
            ..
        } => {
            // Sync model: split the block at the given offset.
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let vd = view_desc.borrow();
                if let Some(block_index) = vd.block_index_for_node(*original_block_id) {
                    let block_start = ed.doc.block_start_position(block_index);
                    let block_len = ed.doc.block_text(block_index).map(|t| t.len()).unwrap_or(0);
                    let clamped_offset = (*split_offset).min(block_len);
                    let pos = Position::new(block_start + clamped_offset);
                    drop(vd);
                    let _ = ed.doc.split_block(pos);
                    // Cursor moves to start of new block
                    let new_block_start = ed.doc.block_start_position(block_index + 1);
                    ed.set_selection(Selection::cursor(Position::new(new_block_start)));
                    sync_cursor_attrs(ce_root, ed.get_selection());
                }
            }
            // Full reconcile to rebuild ViewDesc after structural change
            full_reconcile_helper(
                editor,
                doc_weak,
                ce_root,
                block_cache,
                view_desc,
                container_id,
            );
            on_change();
        }

        CeEvent::BlockJoined {
            surviving_block_id,
            merge_offset,
            ..
        } => {
            // Sync model: join two adjacent blocks by deleting the block boundary.
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let vd = view_desc.borrow();
                if let Some(surviving_idx) = vd.block_index_for_node(*surviving_block_id) {
                    drop(vd);
                    let block_count = ed.doc.block_count();
                    if surviving_idx + 1 < block_count {
                        // Delete range spanning the block boundary:
                        // from end of surviving block's text to start of next block's text.
                        let surviving_text_len = ed
                            .doc
                            .block_text(surviving_idx)
                            .map(|t| t.len())
                            .unwrap_or(0);
                        let block_start = ed.doc.block_start_position(surviving_idx);
                        let del_start = Position::new(block_start + surviving_text_len);
                        let next_block_start = ed.doc.block_start_position(surviving_idx + 1);
                        let del_end = Position::new(next_block_start);
                        if del_end > del_start {
                            let _ = ed.doc.delete_range(Range::new(del_start, del_end));
                        }
                        // Place cursor at the merge point
                        let cursor_pos =
                            Position::new((block_start + merge_offset).min(ed.doc.text_length()));
                        ed.set_selection(Selection::cursor(cursor_pos));
                        sync_cursor_attrs(ce_root, ed.get_selection());
                    }
                }
            }
            full_reconcile_helper(
                editor,
                doc_weak,
                ce_root,
                block_cache,
                view_desc,
                container_id,
            );
            on_change();
        }

        CeEvent::BlockTypeChanged {
            old_node_id,
            new_tag,
            ..
        } => {
            // Sync model: change the block's type.
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let vd = view_desc.borrow();
                if let Some(block_index) = vd.block_index_for_node(*old_node_id) {
                    drop(vd);
                    let (node_type, attrs) = tag_to_block_type(new_tag);
                    let _ = ed.doc.set_block_type(block_index, node_type, attrs);
                }
            }
            full_reconcile_helper(
                editor,
                doc_weak,
                ce_root,
                block_cache,
                view_desc,
                container_id,
            );
            on_change();
        }

        // ── Inline Formatting Events (model sync + full reconcile) ────
        CeEvent::SelectionWrapped { tag, .. } => {
            // Sync model: add mark to the selected text range.
            if let Some(mark_type) = tag_to_mark_type(tag)
                && let Ok(mut ed) = editor.try_borrow_mut()
            {
                let sel = ed.get_selection().clone();
                if !sel.is_empty() {
                    let _ = ed.doc.add_mark(sel.range(), MarkData::new(mark_type));
                }
            }
            full_reconcile_helper(
                editor,
                doc_weak,
                ce_root,
                block_cache,
                view_desc,
                container_id,
            );
            on_change();
        }

        CeEvent::SelectionUnwrapped { tag, .. } => {
            // Sync model: remove mark from the selected text range.
            if let Some(mark_type) = tag_to_mark_type(tag)
                && let Ok(mut ed) = editor.try_borrow_mut()
            {
                let sel = ed.get_selection().clone();
                if !sel.is_empty() {
                    let _ = ed.doc.remove_mark(sel.range(), mark_type);
                }
            }
            full_reconcile_helper(
                editor,
                doc_weak,
                ce_root,
                block_cache,
                view_desc,
                container_id,
            );
            on_change();
        }

        // ── Other Events (full reconcile as safe fallback) ────────────
        CeEvent::HtmlPasted { .. }
        | CeEvent::TextNodeCreated { .. }
        | CeEvent::NodeRemoved { .. }
        | CeEvent::ListItemOutdented { .. }
        | CeEvent::BlockIndented { .. }
        | CeEvent::TableInserted { .. }
        | CeEvent::TableDeleted { .. } => {
            // These events involve complex structural changes.
            // Full reconcile keeps the model and ViewDesc consistent.
            full_reconcile_helper(
                editor,
                doc_weak,
                ce_root,
                block_cache,
                view_desc,
                container_id,
            );
            on_change();
        }
    }
}

/// Sync the editor's selection to DOM attributes for cursor rendering.
///
/// Sets `data-ce-cursor` (head position) and `data-ce-selection-start` (anchor)
/// on the contentEditable root. These are read by paint.rs to render the cursor
/// and selection highlight on desktop.
fn sync_cursor_attrs(ce_root: &NodeHandle, sel: &Selection) {
    ce_root.set_attribute("data-ce-cursor", &sel.head.0.to_string());
    ce_root.set_attribute("data-ce-selection-start", &sel.anchor.0.to_string());
}

/// Run a full reconcile: rebuild DOM from model and refresh ViewDesc.
///
/// Used for structural changes (block split/join/type change, undo/redo)
/// where incremental ViewDesc updates aren't feasible.
fn full_reconcile_helper(
    editor: &Rc<RefCell<Editor>>,
    doc_weak: &Weak<RefCell<dyn DomDocument>>,
    ce_root: &NodeHandle,
    block_cache: &BlockCache,
    view_desc: &SharedViewDesc,
    container_id: NodeId,
) {
    if let Some(doc) = doc_weak.upgrade() {
        let mut scope = RenderScope::new(doc, container_id);
        if let Ok(ed) = editor.try_borrow() {
            let mut cache = block_cache.borrow_mut();
            let mut vd = view_desc.borrow_mut();
            reconciler::full_render(
                &mut scope,
                ce_root,
                &ed.doc,
                &mut cache,
                &ed.tables,
                &mut vd,
                ed.table_selection.as_ref(),
            );
            sync_cursor_attrs(ce_root, ed.get_selection());
        }
    }
}

/// Map a DOM tag to an editor block type name and optional attributes.
fn tag_to_block_type(tag: &str) -> (&str, Option<std::collections::HashMap<String, String>>) {
    match tag {
        "h1" => (
            "heading",
            Some([("level".into(), "1".into())].into_iter().collect()),
        ),
        "h2" => (
            "heading",
            Some([("level".into(), "2".into())].into_iter().collect()),
        ),
        "h3" => (
            "heading",
            Some([("level".into(), "3".into())].into_iter().collect()),
        ),
        "h4" => (
            "heading",
            Some([("level".into(), "4".into())].into_iter().collect()),
        ),
        "h5" => (
            "heading",
            Some([("level".into(), "5".into())].into_iter().collect()),
        ),
        "h6" => (
            "heading",
            Some([("level".into(), "6".into())].into_iter().collect()),
        ),
        "blockquote" => ("blockquote", None),
        "pre" => ("code_block", None),
        _ => ("paragraph", None), // div, p, and default
    }
}

/// Map a DOM formatting tag to a mark type name.
fn tag_to_mark_type(tag: &str) -> Option<&str> {
    match tag {
        "strong" | "b" => Some("bold"),
        "em" | "i" => Some("italic"),
        "u" => Some("underline"),
        "s" | "strike" | "del" => Some("strike"),
        "code" => Some("code"),
        _ => None,
    }
}
