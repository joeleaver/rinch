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
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::ce::{
    BlockData, CeEvent, CeEventDispatcher, CeSelection, ContentEditableApi, DomCursor,
    dispatch_ce_event,
};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

use crate::ce_render::{extract_block, tag_to_block_type, tag_to_mark_type};

use rinch_editor::{
    EditorDocument, MarkData as EditorMarkData, Position as EditorPosition, Range as EditorRange,
};

// ============================================================================
// Pending EditorDocument Registry (collaboration)
// ============================================================================

thread_local! {
    /// Pre-registered EditorDocuments for CE elements, keyed by node ID.
    ///
    /// Users call `set_pending_editor_doc(node_id, doc)` to associate an
    /// EditorDocument with a CE element *before* it gains focus. When CeOps
    /// is created for that element (via factory or register_ce_ops), it
    /// automatically takes the pending doc and enables collaboration.
    ///
    /// This solves the chicken-and-egg problem: the EditorDocument with shared
    /// CRDT history must exist before the first keystroke, but CeOps is created
    /// lazily on focus.
    static PENDING_EDITOR_DOCS: RefCell<HashMap<usize, EditorDocument>> =
        RefCell::new(HashMap::new());
}

/// Pre-register an EditorDocument for a contenteditable element.
///
/// Call this before the element gains focus to ensure the CeOps created
/// for it will have collaboration enabled with shared CRDT history from
/// the start.
///
/// ```ignore
/// // Load document from server bytes (shared history)
/// let doc = EditorDocument::from_bytes(&server_bytes).unwrap();
///
/// // Pre-register for the CE element
/// set_pending_editor_doc(ce_node_id, doc);
///
/// // Later, when user clicks the CE element, CeOps is created with
/// // collaboration already enabled — no race condition.
/// ```
pub fn set_pending_editor_doc(node_id: usize, doc: EditorDocument) {
    PENDING_EDITOR_DOCS.with(|m| {
        m.borrow_mut().insert(node_id, doc);
    });
}

/// Take a pending EditorDocument for a node (if one was pre-registered).
///
/// Called internally by CeOps::new to auto-enable collaboration.
fn take_pending_editor_doc(node_id: usize) -> Option<EditorDocument> {
    PENDING_EDITOR_DOCS.with(|m| m.borrow_mut().remove(&node_id))
}

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

/// Walk up from `node_id` to find its ancestor that is a direct child of `ce_root`.
fn find_ce_root_child(tree: &rinch_dom::NodeTree, node_id: usize, ce_root: usize) -> Option<usize> {
    let mut current = node_id;
    loop {
        let parent = tree.get(current)?.parent?;
        if parent == ce_root {
            return Some(current);
        }
        current = parent;
    }
}

/// Find `descendant` (or its nearest ancestor) that is a direct child of `container`.
/// Used to locate which `<li>` in a list corresponds to a selection endpoint.
/// Check if a tag is a known inline formatting tag.
fn is_formatting_tag(tag: &str) -> bool {
    matches!(
        tag,
        "strong" | "b" | "em" | "i" | "u" | "ins" | "s" | "strike" | "del" | "code" | "mark"
    )
}

/// Collect formatting tags between `node_id` and `stop_at` (exclusive),
/// walking from child up to ancestor. Returns tags innermost-first.
fn collect_inner_formatting_tags(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    stop_at: usize,
) -> Vec<String> {
    let mut tags = Vec::new();
    let mut current = node_id;
    while let Some(parent_id) = tree.get(current).and_then(|n| n.parent) {
        if parent_id == stop_at {
            break;
        }
        if let Some(tag) = tree.get(parent_id).and_then(|n| n.tag()) {
            if is_formatting_tag(tag) {
                tags.push(tag.to_string());
            }
        }
        current = parent_id;
    }
    tags
}

// Block data extraction/loading helpers are in ce_render.rs.
// Imported at the top of this file.

// ============================================================================
// Undo / Redo
// ============================================================================

/// A structured inverse operation for undo/redo.
///
/// Each forward CeOps mutation pushes one or more `UndoOp`s describing how to
/// reverse the operation. Undo pops and replays these through CeOps methods so
/// both the DOM **and** the CRDT see the inverse operation.
#[derive(Debug, Clone)]
pub(crate) enum UndoOp {
    /// Undo an insert: delete the inserted range.
    DeleteRange { start: usize, end: usize },
    /// Undo a delete: re-insert the deleted text at position.
    InsertText { pos: usize, text: String },
    /// Undo a block split: join blocks by deleting the block separator.
    JoinBlock { pos: usize },
    /// Undo a block join: split at position.
    SplitBlock { pos: usize },
    /// Undo a block type change.
    #[allow(dead_code)]
    SetBlockType {
        block_idx: usize,
        block_type: String,
        attrs: HashMap<String, String>,
    },
    /// Undo a mark add: remove the mark.
    RemoveMark {
        start: usize,
        end: usize,
        mark_type: String,
    },
    /// Undo a mark remove: add the mark back.
    AddMark {
        start: usize,
        end: usize,
        mark_type: String,
    },
    /// Restore cursor/anchor after replaying ops.
    RestoreCursor {
        cursor: DomCursor,
        anchor: DomCursor,
    },
    /// Full content snapshot for operations too complex to express as ops
    /// (e.g., indent/outdent which restructure lists).
    Snapshot { blocks: Vec<BlockData> },
}

/// A group of undo ops that should be replayed together atomically.
#[derive(Debug, Clone)]
pub(crate) struct UndoGroup {
    pub(crate) ops: Vec<UndoOp>,
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
    /// Block virtualization window for large documents.
    pub(crate) virtual_window:
        Option<crate::app::contenteditable::ce_virtualization::CeVirtualWindow>,
    /// CRDT-backed editor document — single source of truth for content.
    /// Always present. All mutations go through EditorDocument first,
    /// then re-render affected DOM blocks.
    pub(crate) editor_doc: EditorDocument,
    /// When true, skip the next `sync_editor_doc_from_dom` call.
    /// Set when the editor_doc is known to already match the DOM (e.g.,
    /// loaded from the same content via `set_pending_editor_doc`).
    pub(crate) skip_next_sync: bool,
    /// Block index ↔ DOM node ID mapping.
    pub(crate) block_map: crate::ce_render::BlockMap,
    /// Undo stack: groups of inverse operations, most recent at back.
    pub(crate) undo_stack: std::collections::VecDeque<UndoGroup>,
    /// Redo stack: groups of forward operations from undone edits.
    pub(crate) redo_stack: std::collections::VecDeque<UndoGroup>,
    /// Accumulator for the current undo group being built.
    /// Flushed to `undo_stack` by `push_undo_group()`.
    pending_undo_ops: Vec<UndoOp>,
    /// When true, suppress undo recording only.
    /// Set during undo/redo replay — CeOps methods still dual-write to CRDT
    /// so the document stays in sync.
    suppress_undo_recording: bool,
    /// When true, suppress CRDT dual-writes (and undo recording).
    /// Set during `applying_remote()` — the EditorDocument already has the
    /// changes from `load_incremental`, so CeOps only updates the DOM.
    suppress_crdt_writes: bool,
}

impl CeOps {
    /// Create a new CeOps for a focused contentEditable element.
    ///
    /// If a pending EditorDocument was pre-registered for this node via
    /// [`set_pending_editor_doc`], collaboration is automatically enabled.
    pub fn new(doc: Rc<RefCell<RinchDocument>>, ce_node_id: usize, cursor: DomCursor) -> Self {
        // Use a pre-registered EditorDocument if one was set, otherwise create
        // from current DOM content.
        let pending_doc = take_pending_editor_doc(ce_node_id);
        let has_pending = pending_doc.is_some();

        let editor_doc = if let Some(ed) = pending_doc {
            ed
        } else {
            // Extract content from DOM to initialize EditorDocument
            let d = doc.borrow();
            let mut blocks = Vec::new();
            let children = &d.tree.nodes[ce_node_id].children;
            for &child_id in children {
                crate::ce_render::extract_block(&d.tree, child_id, &mut blocks);
            }
            drop(d);
            EditorDocument::from_block_data(&blocks)
        };

        let mut block_map = crate::ce_render::BlockMap::new();
        block_map.rebuild(&doc.borrow().tree, ce_node_id);

        Self {
            doc,
            ce_node_id,
            cursor,
            anchor: cursor,
            dispatcher: CeEventDispatcher::new(),
            virtual_window: None,
            editor_doc,
            skip_next_sync: has_pending,
            block_map,
            undo_stack: std::collections::VecDeque::new(),
            redo_stack: std::collections::VecDeque::new(),
            pending_undo_ops: Vec::new(),
            suppress_undo_recording: false,
            suppress_crdt_writes: false,
        }
    }

    /// Replace the EditorDocument with one loaded from shared CRDT history.
    ///
    /// Use this to attach a document received from a collaboration server.
    pub fn enable_collaboration(&mut self, editor_doc: EditorDocument) {
        self.editor_doc = editor_doc;
        self.skip_next_sync = true;
    }

    /// Re-create the EditorDocument from current DOM content.
    pub fn enable_collaboration_from_content(&mut self) {
        let blocks = self.extract_content();
        self.editor_doc = EditorDocument::from_block_data(&blocks);
        self.skip_next_sync = true;
    }

    /// Access the EditorDocument.
    pub fn editor_doc(&self) -> &EditorDocument {
        &self.editor_doc
    }

    /// Mutably access the EditorDocument.
    pub fn editor_doc_mut(&mut self) -> &mut EditorDocument {
        &mut self.editor_doc
    }

    /// Get the CE root node ID.
    pub fn ce_node_id(&self) -> usize {
        self.ce_node_id
    }

    /// Suppress CRDT dual-writes and undo recording for the duration of a closure.
    ///
    /// Use this when applying remote CRDT operations to the DOM. The
    /// EditorDocument already has the changes (from `load_incremental`),
    /// so CeOps methods should only update the DOM without writing back
    /// to the CRDT or recording undo entries.
    ///
    /// ```ignore
    /// ops.applying_remote(|ops| {
    ///     for remote_op in &ce_remote_ops {
    ///         match remote_op {
    ///             CeRemoteOp::InsertText { pos, text } => {
    ///                 // Sets cursor to pos, calls insert_text — DOM only
    ///             }
    ///             // ...
    ///         }
    ///     }
    /// });
    /// ```
    pub fn applying_remote<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let was = self.suppress_crdt_writes;
        self.suppress_crdt_writes = true;
        let result = f(self);
        self.suppress_crdt_writes = was;
        result
    }

    /// Returns true if currently suppressing CRDT writes.
    pub fn is_applying_remote(&self) -> bool {
        self.suppress_crdt_writes
    }

    /// Apply remote CRDT changes and re-render affected DOM blocks.
    ///
    /// Call this after receiving bytes from a collaboration peer. The bytes
    /// are loaded into the EditorDocument, and the DOM is fully re-rendered
    /// from the new EditorDocument state.
    #[cfg(feature = "collaboration")]
    pub fn apply_remote_changes(&mut self, bytes: &[u8]) -> Result<(), rinch_editor::EditorError> {
        // Save cursor position in EditorDocument space
        let cursor_pos = self.cursor_editor_pos();

        // Load remote changes into EditorDocument
        self.editor_doc.load_incremental(bytes)?;

        // Full re-render: clear DOM and rebuild from EditorDocument
        let blocks = self.editor_doc.to_block_data();
        let ce_root = self.ce_node_id;
        {
            let mut d = self.doc.borrow_mut();
            let children: Vec<usize> = d.tree.nodes[ce_root].children.clone();
            for &child_id in &children {
                d.remove_node(rinch_core::dom::NodeId(child_id));
            }
            if blocks.is_empty() {
                let p = d.create_element("p");
                d.append_child(rinch_core::dom::NodeId(ce_root), p);
                let text = d.create_text("");
                d.append_child(p, text);
            } else {
                crate::ce_render::load_blocks(&mut d, ce_root, &blocks);
            }
        }
        self.rebuild_block_map();

        // Restore cursor as close to original position as possible
        let max_pos = self.editor_doc.text_length();
        self.set_cursor_from_editor_pos(cursor_pos.min(max_pos));

        self.notify_blocks_changed();
        Ok(())
    }

    // =========================================================================
    // CRDT-first helpers: position conversion + block re-rendering
    // =========================================================================

    /// Convert the current cursor position to an EditorDocument flat position.
    pub(crate) fn cursor_editor_pos(&self) -> usize {
        let d = self.doc.borrow();
        crate::ce_render::dom_cursor_to_editor_pos(
            &d.tree,
            &self.block_map,
            self.ce_node_id,
            self.cursor,
        )
    }

    /// Convert the current anchor position to an EditorDocument flat position.
    pub(crate) fn anchor_editor_pos(&self) -> usize {
        let d = self.doc.borrow();
        crate::ce_render::dom_cursor_to_editor_pos(
            &d.tree,
            &self.block_map,
            self.ce_node_id,
            self.anchor,
        )
    }

    /// Get ordered (start, end) EditorPositions for the current selection.
    pub(crate) fn ordered_editor_selection(&self) -> (usize, usize) {
        let c = self.cursor_editor_pos();
        let a = self.anchor_editor_pos();
        if c <= a { (c, a) } else { (a, c) }
    }

    /// Update the DOM cursor from an EditorPosition.
    /// Sets both cursor and anchor to the same position (collapsed).
    pub(crate) fn set_cursor_from_editor_pos(&mut self, pos: usize) {
        let d = self.doc.borrow();
        if let Some(dom_cursor) =
            crate::ce_render::editor_pos_to_dom_cursor(&d.tree, &self.block_map, pos)
        {
            self.cursor = dom_cursor;
            self.anchor = dom_cursor;
        }
    }

    /// Re-render the block containing the given EditorPosition.
    /// Returns the block index that was re-rendered.
    pub(crate) fn render_block_containing(&mut self, pos: usize) -> Option<usize> {
        let resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(pos));
        let block_idx = match resolved {
            Ok(r) => r.block_index,
            Err(_) => {
                // Position past end — re-render last block
                self.editor_doc.block_count().saturating_sub(1)
            }
        };
        self.render_block_by_index(block_idx);
        Some(block_idx)
    }

    /// Re-render a single block by its EditorDocument index.
    pub(crate) fn render_block_by_index(&mut self, block_idx: usize) {
        let block_data = self.editor_doc_block_data(block_idx);
        let Some(block_data) = block_data else { return };
        let dom_node = self.block_map.dom_node(block_idx);

        match dom_node {
            Some(existing) => {
                let mut d = self.doc.borrow_mut();
                let new_id = crate::ce_render::render_block_at(
                    &mut d,
                    self.ce_node_id,
                    &block_data,
                    existing,
                );
                drop(d);
                if new_id != existing {
                    // BlockMap entry needs updating
                    self.block_map.remove(block_idx);
                    self.block_map.insert(block_idx, new_id);
                }
            }
            None => {
                // Block doesn't exist in DOM yet — insert it
                let after = if block_idx > 0 {
                    self.block_map.dom_node(block_idx - 1)
                } else {
                    None
                };
                let mut d = self.doc.borrow_mut();
                let new_id = crate::ce_render::render_block_insert(
                    &mut d,
                    self.ce_node_id,
                    &block_data,
                    after,
                );
                drop(d);
                self.block_map.insert(block_idx, new_id);
            }
        }
    }

    /// Get BlockData for a specific block from the EditorDocument.
    fn editor_doc_block_data(&self, block_idx: usize) -> Option<BlockData> {
        if block_idx >= self.editor_doc.block_count() {
            return None;
        }
        let block_type = self.editor_doc.block_type(block_idx)?;
        let attrs = self.editor_doc.block_attrs(block_idx).unwrap_or_default();
        let runs = self.editor_doc.block_inline_runs(block_idx);
        let content = runs
            .into_iter()
            .map(|r| {
                use rinch_core::ce::InlineMarkData;
                use rinch_core::ce::InlineRunData;
                InlineRunData {
                    text: r.text,
                    marks: r
                        .marks
                        .into_iter()
                        .map(|m| InlineMarkData {
                            mark_type: m.mark_type,
                            attrs: m.attrs,
                        })
                        .collect(),
                }
            })
            .collect();
        Some(BlockData {
            block_type,
            attrs,
            content,
        })
    }

    /// Try to surgically update a single text node instead of full block re-render.
    ///
    /// Returns true if the surgical update was performed. Only works when the
    /// block has a single unmarked text run and a single text node child in DOM.
    fn try_surgical_text_update(&mut self, pos: usize) -> bool {
        let resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(pos));
        let block_idx = match resolved {
            Ok(r) => r.block_index,
            Err(_) => return false,
        };
        let runs = self.editor_doc.block_inline_runs(block_idx);
        // Only handle single unmarked text run
        if runs.len() != 1 || !runs[0].marks.is_empty() {
            return false;
        }
        let new_text = &runs[0].text;
        let dom_node = match self.block_map.dom_node(block_idx) {
            Some(n) => n,
            None => return false,
        };
        // Check DOM has a single text child
        let d = self.doc.borrow();
        let children = &d.tree.nodes[dom_node].children;
        if children.len() != 1 {
            return false;
        }
        let text_node_id = children[0];
        let is_text = d
            .tree
            .get(text_node_id)
            .and_then(|n| n.text_content())
            .is_some();
        if !is_text {
            return false;
        }
        drop(d);

        // Surgical update: just change the text content
        let mut d = self.doc.borrow_mut();
        d.set_text_content(rinch_core::dom::NodeId(text_node_id), new_text);
        true
    }

    /// Rebuild the BlockMap from the current DOM state.
    pub(crate) fn rebuild_block_map(&mut self) {
        let d = self.doc.borrow();
        self.block_map.rebuild(&d.tree, self.ce_node_id);
    }

    /// Re-render all blocks after a block split at the given position.
    pub(crate) fn render_block_split(&mut self, pos: usize) {
        // After split_block in EditorDocument, there's a new block at orig_idx+1.
        // The BlockMap still has the old state (one fewer block).
        // Strategy: re-render the original block, then INSERT a new DOM node
        // for the new block (don't overwrite the next block).
        let resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(pos));
        let orig_idx = match resolved {
            Ok(r) => r.block_index,
            Err(_) => self.editor_doc.block_count().saturating_sub(2),
        };

        // Re-render the original block (which now has less content)
        self.render_block_by_index(orig_idx);

        // Insert the new block (orig_idx + 1) into the DOM
        let new_idx = orig_idx + 1;
        if new_idx < self.editor_doc.block_count() {
            let block_data = self.editor_doc_block_data(new_idx);
            if let Some(bd) = block_data {
                let after = self.block_map.dom_node(orig_idx);
                let mut d = self.doc.borrow_mut();
                let new_id =
                    crate::ce_render::render_block_insert(&mut d, self.ce_node_id, &bd, after);
                drop(d);
                // Insert into BlockMap at the new position
                self.block_map.insert(new_idx, new_id);
            }
        }
    }

    /// Re-render blocks affected by a delete that may have merged blocks.
    pub(crate) fn render_blocks_after_delete(
        &mut self,
        start_pos: usize,
        pre_block_count: usize,
        post_block_count: usize,
    ) {
        let blocks_removed = pre_block_count.saturating_sub(post_block_count);

        // Find which block contains the start position now
        let resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(start_pos));
        let surviving_idx = match resolved {
            Ok(r) => r.block_index,
            Err(_) => self.editor_doc.block_count().saturating_sub(1),
        };

        // Remove the merged blocks from the DOM (in reverse order to keep indices stable)
        for i in (0..blocks_removed).rev() {
            let remove_idx = surviving_idx + 1 + i;
            if let Some(dom_node) = self.block_map.dom_node(remove_idx) {
                let mut d = self.doc.borrow_mut();
                crate::ce_render::remove_block(&mut d, dom_node);
                drop(d);
                self.block_map.remove(remove_idx);
            }
        }

        // Re-render the surviving block
        self.render_block_by_index(surviving_idx);
    }

    /// Re-render all blocks in a range of EditorPositions.
    pub(crate) fn render_blocks_in_range(&mut self, start: usize, end: usize) {
        let start_resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(start));
        let end_resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(
                end.saturating_sub(1).max(start),
            ));
        let start_idx = start_resolved.map(|r| r.block_index).unwrap_or(0);
        let end_idx = end_resolved
            .map(|r| r.block_index)
            .unwrap_or(self.editor_doc.block_count().saturating_sub(1));

        for idx in start_idx..=end_idx {
            self.render_block_by_index(idx);
        }
    }

    /// Re-render a contiguous group of list blocks around `block_idx`.
    ///
    /// When an indent level changes, the DOM nesting structure changes for the
    /// entire contiguous list group. This method finds the group boundaries,
    /// removes all DOM nodes in the group, and re-renders them with correct nesting.
    pub(crate) fn rerender_list_group(&mut self, block_idx: usize) {
        let block_count = self.editor_doc.block_count();

        // Find the start of the contiguous list group
        let mut group_start = block_idx;
        while group_start > 0 {
            let prev_type = self
                .editor_doc
                .block_type(group_start - 1)
                .unwrap_or_default();
            if prev_type == "bullet_list" || prev_type == "ordered_list" {
                group_start -= 1;
            } else {
                break;
            }
        }

        // Find the end of the contiguous list group
        let mut group_end = block_idx;
        while group_end + 1 < block_count {
            let next_type = self
                .editor_doc
                .block_type(group_end + 1)
                .unwrap_or_default();
            if next_type == "bullet_list" || next_type == "ordered_list" {
                group_end += 1;
            } else {
                break;
            }
        }

        // Collect BlockData for the group
        let mut group_blocks = Vec::new();
        for idx in group_start..=group_end {
            if let Some(bd) = self.editor_doc_block_data(idx) {
                group_blocks.push(bd);
            }
        }

        // Remove old DOM nodes for the group
        // First, find the parent list container(s) and the insertion point
        let insert_before_dom = self.block_map.dom_node(group_end + 1);

        // Remove all DOM nodes in the group (and their parent list containers)
        {
            let mut d = self.doc.borrow_mut();
            let mut removed_lists = std::collections::HashSet::new();
            for idx in group_start..=group_end {
                if let Some(dom_node) = self.block_map.dom_node(idx) {
                    // Get the <li>'s parent list container
                    if let Some(parent_id) = d.tree.get(dom_node).and_then(|n| n.parent) {
                        let parent_tag = d.tree.get(parent_id).and_then(|n| n.tag()).unwrap_or("");
                        if (parent_tag == "ul" || parent_tag == "ol")
                            && !removed_lists.contains(&parent_id)
                        {
                            // Remove the entire list container
                            d.remove_node(rinch_core::dom::NodeId(parent_id));
                            removed_lists.insert(parent_id);
                        }
                    }
                    // If li wasn't inside a list (edge case), remove it directly
                    if d.tree.get(dom_node).is_some() {
                        d.remove_node(rinch_core::dom::NodeId(dom_node));
                    }
                }
            }

            // Re-render the group blocks with correct nesting
            // We need to insert before a reference node or at the end
            // Create a temporary wrapper to collect the rendered nodes
            let ce_root = self.ce_node_id;

            if let Some(ref_id) = insert_before_dom {
                // Insert a temporary marker before ref_id, render after it, then remove marker
                // Actually, simpler: render into ce_root, then move before ref_id
                // For now, just use load_blocks which appends to ce_root
                // The blocks will be at the end — we need to move them
                let pre_child_count = d.tree.nodes[ce_root].children.len();
                crate::ce_render::load_blocks(&mut d, ce_root, &group_blocks);
                // Move newly created nodes before ref_id
                let new_children: Vec<usize> =
                    d.tree.nodes[ce_root].children[pre_child_count..].to_vec();
                for &new_child in &new_children {
                    d.remove_node(rinch_core::dom::NodeId(new_child));
                    d.insert_before(
                        rinch_core::dom::NodeId(ce_root),
                        rinch_core::dom::NodeId(new_child),
                        rinch_core::dom::NodeId(ref_id),
                    );
                }
            } else {
                crate::ce_render::load_blocks(&mut d, ce_root, &group_blocks);
            }
        }

        // Rebuild BlockMap to pick up new DOM node IDs
        self.rebuild_block_map();
    }

    /// Update cursor/anchor from app.rs after it handles input directly.
    ///
    /// Called by app.rs to keep CeOps in sync after operations that
    /// app.rs handles inline (e.g., text insertion, deletion, mouse clicks).
    pub fn sync_cursor(&mut self, cursor: DomCursor, anchor: DomCursor) {
        self.cursor = cursor;
        self.anchor = anchor;
    }

    /// Notify the virtual window that the block structure changed.
    /// Call after split_block, join_block, delete_selection, or any operation
    /// that adds/removes direct children of the CE root.
    pub fn notify_blocks_changed(&mut self) {
        if let Some(vw) = &mut self.virtual_window {
            let mut d = self.doc.borrow_mut();
            vw.on_blocks_changed(&mut d);
        }
    }

    // ── Position helpers ──────────────────────────────────────────────

    // ── Undo infrastructure ───────────────────────────────────────────

    /// Record an inverse operation for undo. No-op when replaying or applying remote.
    fn push_undo_op(&mut self, op: UndoOp) {
        if !self.suppress_undo_recording && !self.suppress_crdt_writes {
            self.pending_undo_ops.push(op);
        }
    }

    /// Start a new undo group: saves cursor position before mutation.
    /// Called at the start of each `ContentEditableApi` method.
    pub(crate) fn begin_undo_group(&mut self) {
        if self.suppress_undo_recording || self.suppress_crdt_writes {
            return;
        }
        // Consume skip_next_sync on first mutation — it was only meant to
        // prevent the initial sync after pre-loading an EditorDocument.
        {
            self.skip_next_sync = false;
        }
        self.pending_undo_ops.clear();
        // Always record cursor restore as first op (replayed last during undo)
        self.pending_undo_ops.push(UndoOp::RestoreCursor {
            cursor: self.cursor,
            anchor: self.anchor,
        });
    }

    /// Flush the pending undo ops as a group onto the undo stack.
    /// Called at the end of each `ContentEditableApi` method.
    pub(crate) fn commit_undo_group(&mut self) {
        if self.suppress_undo_recording
            || self.suppress_crdt_writes
            || self.pending_undo_ops.len() <= 1
        {
            // Only the RestoreCursor — nothing actually happened
            self.pending_undo_ops.clear();
            return;
        }
        let ops = std::mem::take(&mut self.pending_undo_ops);
        self.undo_stack.push_back(UndoGroup { ops });
        if self.undo_stack.len() > 100 {
            self.undo_stack.pop_front();
        }
        self.redo_stack.clear();
    }

    /// Replay an undo group. Executes ops in reverse order.
    ///
    /// Returns the cursor/anchor to restore (from the `RestoreCursor` op).
    pub(crate) fn replay_undo(&mut self) -> Option<(DomCursor, DomCursor)> {
        let group = self.undo_stack.pop_back()?;

        // Build the redo group: snapshot current state before replaying
        let mut redo_ops = Vec::new();
        redo_ops.push(UndoOp::RestoreCursor {
            cursor: self.cursor,
            anchor: self.anchor,
        });
        // Capture current content for redo's inverse
        self.capture_inverse_ops(&group.ops, &mut redo_ops);

        let mut restore_cursor = None;

        self.suppress_undo_recording = true;
        let mut had_snapshot = false;
        // Replay in reverse order (most recent change undone first)
        for op in group.ops.iter().rev() {
            match op {
                UndoOp::RestoreCursor { cursor, anchor } => {
                    // Only restore cursor if no Snapshot was applied.
                    // After Snapshot restore, the cursor from the old DOM is
                    // invalid (node IDs changed). restore_from_snapshot already
                    // set the cursor to a valid position.
                    if !had_snapshot {
                        restore_cursor = Some((*cursor, *anchor));
                    }
                }
                UndoOp::Snapshot { .. } => {
                    had_snapshot = true;
                    self.apply_undo_op(op);
                }
                _ => self.apply_undo_op(op),
            }
        }
        self.suppress_undo_recording = false;

        self.redo_stack.push_back(UndoGroup { ops: redo_ops });

        restore_cursor
    }

    /// Replay a redo group. Executes ops in reverse order.
    pub(crate) fn replay_redo(&mut self) -> Option<(DomCursor, DomCursor)> {
        let group = self.redo_stack.pop_back()?;

        // Build the undo group from current state
        let mut undo_ops = Vec::new();
        undo_ops.push(UndoOp::RestoreCursor {
            cursor: self.cursor,
            anchor: self.anchor,
        });
        self.capture_inverse_ops(&group.ops, &mut undo_ops);

        let mut restore_cursor = None;

        self.suppress_undo_recording = true;
        let mut had_snapshot = false;
        for op in group.ops.iter().rev() {
            match op {
                UndoOp::RestoreCursor { cursor, anchor } => {
                    if !had_snapshot {
                        restore_cursor = Some((*cursor, *anchor));
                    }
                }
                UndoOp::Snapshot { .. } => {
                    had_snapshot = true;
                    self.apply_undo_op(op);
                }
                _ => self.apply_undo_op(op),
            }
        }
        self.suppress_undo_recording = false;

        self.undo_stack.push_back(UndoGroup { ops: undo_ops });

        restore_cursor
    }

    /// Apply a single undo op via EditorDocument + re-render.
    fn apply_undo_op(&mut self, op: &UndoOp) {
        match op {
            UndoOp::InsertText { pos, text } => {
                self.set_cursor_from_editor_pos(*pos);
                ContentEditableApi::insert_text(self, text);
            }
            UndoOp::DeleteRange { start, end } => {
                // Set selection to the range, then delete
                self.set_cursor_from_editor_pos(*end);
                let anchor = {
                    let d = self.doc.borrow();
                    crate::ce_render::editor_pos_to_dom_cursor(&d.tree, &self.block_map, *start)
                };
                if let Some(a) = anchor {
                    self.anchor = a;
                }
                ContentEditableApi::delete_selection(self);
            }
            UndoOp::SplitBlock { pos } => {
                self.set_cursor_from_editor_pos(*pos);
                ContentEditableApi::split_block(self);
            }
            UndoOp::JoinBlock { pos } => {
                // Join = delete the block separator at `pos`
                self.set_cursor_from_editor_pos(*pos + 1);
                let anchor = {
                    let d = self.doc.borrow();
                    crate::ce_render::editor_pos_to_dom_cursor(&d.tree, &self.block_map, *pos)
                };
                if let Some(a) = anchor {
                    self.anchor = a;
                }
                ContentEditableApi::delete_selection(self);
            }
            UndoOp::SetBlockType {
                block_idx,
                block_type,
                attrs,
            } => {
                let attrs_opt = if attrs.is_empty() {
                    None
                } else {
                    Some(attrs.clone())
                };
                let _ = self
                    .editor_doc
                    .set_block_type(*block_idx, block_type, attrs_opt);
                self.rebuild_block_map();
                self.render_block_by_index(*block_idx);
                self.rebuild_block_map();
            }
            UndoOp::AddMark {
                start,
                end,
                mark_type,
            } => {
                let mark = EditorMarkData::new(mark_type.as_str());
                let _ = self
                    .editor_doc
                    .add_mark(EditorRange::new(*start, *end), mark);
                self.render_blocks_in_range(*start, *end);
            }
            UndoOp::RemoveMark {
                start,
                end,
                mark_type,
            } => {
                let _ = self
                    .editor_doc
                    .remove_mark(EditorRange::new(*start, *end), mark_type);
                self.render_blocks_in_range(*start, *end);
            }
            UndoOp::Snapshot { blocks } => {
                self.restore_from_snapshot(blocks);
            }
            UndoOp::RestoreCursor { .. } => {
                // Handled by caller
            }
        }
    }

    /// Capture inverse operations for a group of ops.
    /// Used to build redo from undo (and vice versa).
    fn capture_inverse_ops(&self, ops: &[UndoOp], out: &mut Vec<UndoOp>) {
        for op in ops {
            match op {
                UndoOp::InsertText { pos, text } => {
                    out.push(UndoOp::DeleteRange {
                        start: *pos,
                        end: *pos + text.len(),
                    });
                }
                UndoOp::DeleteRange { start, end } => {
                    // We need the text that will be deleted. Extract it from DOM.
                    let text = self.extract_flat_text(*start, *end);
                    out.push(UndoOp::InsertText { pos: *start, text });
                }
                UndoOp::SplitBlock { pos } => {
                    out.push(UndoOp::JoinBlock { pos: *pos });
                }
                UndoOp::JoinBlock { pos } => {
                    out.push(UndoOp::SplitBlock { pos: *pos });
                }
                UndoOp::SetBlockType {
                    block_idx,
                    block_type: _,
                    attrs: _,
                } => {
                    // Capture current block type before it changes
                    let blocks = self.extract_content();
                    if let Some(block) = blocks.get(*block_idx) {
                        out.push(UndoOp::SetBlockType {
                            block_idx: *block_idx,
                            block_type: block.block_type.clone(),
                            attrs: block.attrs.clone(),
                        });
                    }
                }
                UndoOp::AddMark {
                    start,
                    end,
                    mark_type,
                } => {
                    out.push(UndoOp::RemoveMark {
                        start: *start,
                        end: *end,
                        mark_type: mark_type.clone(),
                    });
                }
                UndoOp::RemoveMark {
                    start,
                    end,
                    mark_type,
                } => {
                    out.push(UndoOp::AddMark {
                        start: *start,
                        end: *end,
                        mark_type: mark_type.clone(),
                    });
                }
                UndoOp::Snapshot { .. } => {
                    // Snapshot redo = snapshot of current state
                    out.push(UndoOp::Snapshot {
                        blocks: self.extract_content(),
                    });
                }
                UndoOp::RestoreCursor { .. } => {
                    // Handled separately
                }
            }
        }
    }

    /// Extract text from a flat offset range in the EditorDocument.
    fn extract_flat_text(&self, start: usize, end: usize) -> String {
        let full_text = self.editor_doc.to_text();
        let len = full_text.len();
        let start = start.min(len);
        let end = end.min(len);
        if start >= end {
            return String::new();
        }
        // Find valid char boundaries
        let mut s = start;
        while s < len && !full_text.is_char_boundary(s) {
            s += 1;
        }
        let mut e = end;
        while e < len && !full_text.is_char_boundary(e) {
            e += 1;
        }
        if s >= e {
            return String::new();
        }
        full_text[s..e].to_string()
    }

    /// Restore content from a snapshot (for complex undo/redo).
    fn restore_from_snapshot(&mut self, blocks: &[BlockData]) {
        // Update EditorDocument from snapshot
        self.editor_doc = EditorDocument::from_block_data(blocks);

        let ce_root = self.ce_node_id;
        {
            let mut d = self.doc.borrow_mut();
            let children: Vec<usize> = d.tree.nodes[ce_root].children.clone();
            for &child_id in &children {
                d.remove_node(rinch_core::dom::NodeId(child_id));
            }
            if blocks.is_empty() {
                // Ensure at least one block
                let p = d.create_element("p");
                d.append_child(rinch_core::dom::NodeId(ce_root), p);
                let text = d.create_text("");
                d.append_child(p, text);
            } else {
                crate::ce_render::load_blocks(&mut d, ce_root, blocks);
            }
        }

        self.rebuild_block_map();

        // Set cursor to first text node
        self.set_cursor_from_editor_pos(0);
        self.notify_blocks_changed();
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
        self.begin_undo_group();

        // 1. If there's a selection, delete it first
        if self.cursor != self.anchor {
            self.delete_selection();
            // delete_selection commits its own undo group, so start a new one
            // Actually, we want insert_text to be atomic, so just record the
            // position after deletion for the insert undo.
        }

        // 2. Compute cursor position in EditorDocument space
        let pos = self.cursor_editor_pos();

        // 3. Record undo: inverse of insert is delete
        self.push_undo_op(UndoOp::DeleteRange {
            start: pos,
            end: pos + text.len(),
        });

        // 4. Mutate EditorDocument (source of truth)
        let _ = self.editor_doc.insert_text(EditorPosition(pos), text);

        // 5. Update DOM to match EditorDocument.
        // Fast path: for a single unmarked text run, surgically update the
        // existing text node via set_text_content (O(1), triggers only IFC
        // text root rebuild). This avoids clearing + re-creating all children
        // (O(N) with N style resolution passes per child).
        let new_pos = pos + text.len();
        let did_surgical = self.try_surgical_text_update(pos);
        if !did_surgical {
            // Full block re-render (marks changed, multiple runs, etc.)
            self.render_block_containing(pos);
        }

        // 6. Update cursor to after inserted text
        self.set_cursor_from_editor_pos(new_pos);

        // 7. Dispatch event and commit
        dispatch_ce_event(&CeEvent::TextInserted {
            node_id: self.cursor.node_id,
            offset: self.cursor.offset.saturating_sub(text.len()),
            text: text.to_string(),
        });
        self.commit_undo_group();
    }

    fn delete_backward(&mut self) {
        if self.cursor != self.anchor {
            return self.delete_selection();
        }

        let pos = self.cursor_editor_pos();
        if pos == 0 {
            return;
        }

        self.begin_undo_group();

        // Find what to delete: one character or block separator
        // In EditorDocument, block separators are newlines (\n).
        let full_text = self.editor_doc.to_text();
        let delete_start = full_text[..pos]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);

        // Record undo: re-insert the deleted content
        let deleted: String = full_text[delete_start..pos].to_string();
        self.push_undo_op(UndoOp::InsertText {
            pos: delete_start,
            text: deleted,
        });

        // Mutate EditorDocument
        let pre_blocks = self.editor_doc.block_count();
        let _ = self
            .editor_doc
            .delete_range(EditorRange::new(delete_start, pos));
        let post_blocks = self.editor_doc.block_count();

        // Re-render
        self.render_blocks_after_delete(delete_start, pre_blocks, post_blocks);

        // Update cursor
        self.set_cursor_from_editor_pos(delete_start);

        dispatch_ce_event(&CeEvent::TextDeleted {
            node_id: self.cursor.node_id,
            offset: self.cursor.offset,
            length: pos - delete_start,
        });
        self.commit_undo_group();
    }

    fn delete_forward(&mut self) {
        if self.cursor != self.anchor {
            return self.delete_selection();
        }

        let pos = self.cursor_editor_pos();
        let total_len = self.editor_doc.text_length();
        if pos >= total_len {
            return;
        }

        self.begin_undo_group();

        // Find what to delete: one character forward
        let full_text = self.editor_doc.to_text();
        let delete_end = full_text[pos..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| pos + i)
            .unwrap_or(total_len);

        // Record undo: re-insert the deleted content
        let deleted: String = full_text[pos..delete_end].to_string();
        self.push_undo_op(UndoOp::InsertText { pos, text: deleted });

        // Mutate EditorDocument
        let pre_blocks = self.editor_doc.block_count();
        let _ = self
            .editor_doc
            .delete_range(EditorRange::new(pos, delete_end));
        let post_blocks = self.editor_doc.block_count();

        // Re-render
        self.render_blocks_after_delete(pos, pre_blocks, post_blocks);

        // Cursor stays at same position, but DOM nodes may have changed
        self.set_cursor_from_editor_pos(pos);

        dispatch_ce_event(&CeEvent::TextDeleted {
            node_id: self.cursor.node_id,
            offset: self.cursor.offset,
            length: delete_end - pos,
        });
        self.commit_undo_group();
    }

    fn delete_selection(&mut self) {
        let (start, end) = self.ordered_editor_selection();
        if start == end {
            return;
        }
        self.begin_undo_group();

        // Record undo: re-insert the deleted text
        let deleted_text = self.editor_doc.to_text();
        let deleted_range: String = deleted_text.chars().skip(start).take(end - start).collect();
        self.push_undo_op(UndoOp::InsertText {
            pos: start,
            text: deleted_range,
        });

        // Mutate EditorDocument
        let pre_blocks = self.editor_doc.block_count();
        let _ = self.editor_doc.delete_range(EditorRange::new(start, end));
        let post_blocks = self.editor_doc.block_count();

        // Re-render affected blocks
        self.render_blocks_after_delete(start, pre_blocks, post_blocks);

        // Update cursor
        self.set_cursor_from_editor_pos(start);

        dispatch_ce_event(&CeEvent::SelectionChanged {
            selection: self.get_selection(),
        });
        self.commit_undo_group();
    }

    // ── Block Structure ──────────────────────────────────────────────

    fn split_block(&mut self) {
        // Delete selection first if present
        if self.cursor != self.anchor {
            self.delete_selection();
        }

        self.begin_undo_group();

        let pos = self.cursor_editor_pos();

        // Check for list exit: if cursor is in an empty list item block,
        // convert it to a paragraph instead of splitting.
        let resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(pos));
        let is_list_exit = if let Ok(ref r) = resolved {
            let block_idx = r.block_index;
            let block_type = self.editor_doc.block_type(block_idx).unwrap_or_default();
            let block_text = self.editor_doc.block_text(block_idx).unwrap_or_default();
            (block_type == "bullet_list" || block_type == "ordered_list") && block_text.is_empty()
        } else {
            false
        };

        if is_list_exit {
            // Convert empty list item to paragraph
            let block_idx = resolved.unwrap().block_index;
            let _ = self.editor_doc.set_block_type(block_idx, "paragraph", None);

            // Record undo
            let old_type = "bullet_list".to_string(); // approximate
            self.push_undo_op(UndoOp::SetBlockType {
                block_idx,
                block_type: old_type,
                attrs: HashMap::new(),
            });

            // Re-render this block and rebuild BlockMap since list structure changed
            self.rebuild_block_map();
            self.render_block_by_index(block_idx);
            // After changing from list to paragraph, the DOM structure changes
            // (li inside ul → p), so we need to rebuild the block map again
            self.rebuild_block_map();

            self.set_cursor_from_editor_pos(pos);

            dispatch_ce_event(&CeEvent::BlockTypeChanged {
                old_node_id: 0,
                new_node_id: self.cursor.node_id,
                old_tag: "li".to_string(),
                new_tag: "p".to_string(),
            });
        } else {
            // Normal split: split block at cursor position
            self.push_undo_op(UndoOp::JoinBlock { pos });

            let _ = self.editor_doc.split_block(EditorPosition(pos));

            // Re-render: the original block and the new block
            self.render_block_split(pos);

            // Cursor moves to start of new block (past the separator)
            let new_pos = pos + 1;
            self.set_cursor_from_editor_pos(new_pos);

            dispatch_ce_event(&CeEvent::BlockSplit {
                original_block_id: 0,
                new_block_id: 0,
                split_offset: pos,
            });
        }

        self.notify_blocks_changed();
        self.commit_undo_group();
    }

    fn set_block_type(&mut self, tag: &str) {
        self.begin_undo_group();

        // Convert HTML tag to EditorDocument block type
        let (block_type, attrs) = tag_to_block_type(tag);
        let attrs_opt = if attrs.is_empty() {
            None
        } else {
            Some(attrs.clone())
        };

        // Determine which blocks are affected
        let (start_idx, end_idx) = if self.cursor == self.anchor {
            // Single block
            let pos = self.cursor_editor_pos();
            let resolved = self
                .editor_doc
                .resolve_position(rinch_editor::Position::new(pos));
            let idx = resolved.map(|r| r.block_index).unwrap_or(0);
            (idx, idx)
        } else {
            // Multiple blocks spanned by selection
            let (start, end) = self.ordered_editor_selection();
            let start_resolved = self
                .editor_doc
                .resolve_position(rinch_editor::Position::new(start));
            let end_resolved = self
                .editor_doc
                .resolve_position(rinch_editor::Position::new(
                    end.saturating_sub(1).max(start),
                ));
            let s = start_resolved.map(|r| r.block_index).unwrap_or(0);
            let e = end_resolved
                .map(|r| r.block_index)
                .unwrap_or(self.editor_doc.block_count().saturating_sub(1));
            (s, e)
        };

        // Record undo for each block
        for idx in start_idx..=end_idx {
            if let Some(old_type) = self.editor_doc.block_type(idx) {
                let old_attrs = self.editor_doc.block_attrs(idx).unwrap_or_default();
                self.push_undo_op(UndoOp::SetBlockType {
                    block_idx: idx,
                    block_type: old_type,
                    attrs: old_attrs,
                });
            }
        }

        // Mutate EditorDocument
        for idx in start_idx..=end_idx {
            let _ = self
                .editor_doc
                .set_block_type(idx, block_type, attrs_opt.clone());
        }

        // Re-render affected blocks and rebuild block map
        // (block type change may alter list structure, so rebuild map)
        self.rebuild_block_map();
        for idx in start_idx..=end_idx {
            self.render_block_by_index(idx);
        }
        self.rebuild_block_map();

        // Update cursor
        let pos = self.cursor_editor_pos();
        self.set_cursor_from_editor_pos(pos);

        dispatch_ce_event(&CeEvent::BlockTypeChanged {
            old_node_id: 0,
            new_node_id: self.cursor.node_id,
            old_tag: String::new(),
            new_tag: tag.to_string(),
        });
        self.notify_blocks_changed();
        self.commit_undo_group();
    }

    // ── Inline Formatting ────────────────────────────────────────────

    fn wrap_selection(&mut self, tag: &str) {
        if self.cursor == self.anchor {
            return;
        }
        self.begin_undo_group();

        let (start, end) = self.ordered_editor_selection();
        if start == end {
            self.commit_undo_group();
            return;
        }

        let Some(mark_type) = tag_to_mark_type(tag) else {
            self.commit_undo_group();
            return;
        };

        // Record undo
        self.push_undo_op(UndoOp::RemoveMark {
            start,
            end,
            mark_type: mark_type.to_string(),
        });

        // Mutate EditorDocument
        let mark = EditorMarkData::new(mark_type);
        let _ = self.editor_doc.add_mark(EditorRange::new(start, end), mark);

        // Re-render affected blocks
        self.render_blocks_in_range(start, end);

        // Restore selection
        self.set_cursor_from_editor_pos(end);
        let anchor_cursor = {
            let d = self.doc.borrow();
            crate::ce_render::editor_pos_to_dom_cursor(&d.tree, &self.block_map, start)
        };
        if let Some(ac) = anchor_cursor {
            self.anchor = ac;
        }

        dispatch_ce_event(&CeEvent::SelectionWrapped {
            tag: tag.to_string(),
            wrapper_node_id: 0,
            wrapped_node_ids: vec![],
        });
        self.commit_undo_group();
    }

    fn unwrap_selection(&mut self, tag: &str) {
        if self.cursor == self.anchor {
            return;
        }
        self.begin_undo_group();

        let (start, end) = self.ordered_editor_selection();
        if start == end {
            self.commit_undo_group();
            return;
        }

        let Some(mark_type) = tag_to_mark_type(tag) else {
            self.commit_undo_group();
            return;
        };

        // Record undo
        self.push_undo_op(UndoOp::AddMark {
            start,
            end,
            mark_type: mark_type.to_string(),
        });

        // Mutate EditorDocument
        let _ = self
            .editor_doc
            .remove_mark(EditorRange::new(start, end), mark_type);

        // Re-render affected blocks
        self.render_blocks_in_range(start, end);

        // Restore selection
        self.set_cursor_from_editor_pos(end);
        let anchor_cursor = {
            let d = self.doc.borrow();
            crate::ce_render::editor_pos_to_dom_cursor(&d.tree, &self.block_map, start)
        };
        if let Some(ac) = anchor_cursor {
            self.anchor = ac;
        }

        dispatch_ce_event(&CeEvent::SelectionUnwrapped {
            tag: tag.to_string(),
            unwrapped_node_ids: vec![],
        });
        self.commit_undo_group();
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
                // Place cursor after the formatting element, but preserve any
                // inner formatting tags between the cursor and the escaped element.
                // E.g. inside <strong><em>text|</em></strong>, escaping "strong"
                // should create <em>ZWS</em> after <strong> so italic is preserved.
                let parent_id = d
                    .tree
                    .get(fmt_id)
                    .and_then(|n| n.parent)
                    .unwrap_or(self.ce_node_id);
                let next_sib = next_sibling(&d.tree, parent_id, fmt_id);

                // Collect inner formatting tags (innermost-first) between cursor and fmt_id
                let inner_tags =
                    collect_inner_formatting_tags(&d.tree, self.cursor.node_id, fmt_id);

                let cursor_target = if inner_tags.is_empty() {
                    // No inner formatting — just place a ZWS text node after fmt_id
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
                    }
                } else {
                    // Build nested wrappers for inner formatting, e.g. <em><u>ZWS</u></em>
                    let zws = d.create_text("\u{200B}");
                    let mut current_node = zws;
                    // Iterate innermost-first: each wrap becomes the new outermost
                    for inner_tag in &inner_tags {
                        let wrapper = d.create_element(inner_tag);
                        d.append_child(wrapper, current_node);
                        current_node = wrapper;
                    }
                    // Insert outermost wrapper after fmt_id
                    if let Some(next_id) = next_sib {
                        d.insert_before(
                            rinch_core::dom::NodeId(parent_id),
                            current_node,
                            rinch_core::dom::NodeId(next_id),
                        );
                    } else {
                        d.append_child(rinch_core::dom::NodeId(parent_id), current_node);
                    }
                    zws.0 // Cursor goes into the innermost ZWS
                };
                self.cursor = DomCursor::new(cursor_target, 0);
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
        // Note: sync already happens in wrap_selection/unwrap_selection for
        // selection paths, and for cursor-only paths the ZWS nodes are
        // temporary and don't affect CRDT content.
    }

    // ── List Operations ──────────────────────────────────────────────

    fn indent(&mut self) {
        let pos = self.cursor_editor_pos();
        let resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(pos));
        let block_idx = match resolved {
            Ok(r) => r.block_index,
            Err(_) => return,
        };
        let block_type = match self.editor_doc.block_type(block_idx) {
            Some(t) => t,
            None => return,
        };

        match block_type.as_str() {
            "bullet_list" | "ordered_list" => {
                // Already a list item — increase indent
                let current_indent: usize = self
                    .editor_doc
                    .block_attrs(block_idx)
                    .and_then(|a| a.get("indent").and_then(|s| s.parse().ok()))
                    .unwrap_or(0);

                // Can't indent first item in a list group (no previous sibling to nest under)
                if block_idx == 0 {
                    return;
                }
                let prev_type = self
                    .editor_doc
                    .block_type(block_idx - 1)
                    .unwrap_or_default();
                if prev_type != block_type {
                    return; // Previous block isn't the same list type
                }
                let prev_indent: usize = self
                    .editor_doc
                    .block_attrs(block_idx - 1)
                    .and_then(|a| a.get("indent").and_then(|s| s.parse().ok()))
                    .unwrap_or(0);
                if current_indent > prev_indent {
                    return; // Already deeper than previous — can't indent further
                }

                self.begin_undo_group();
                let old_attrs = self.editor_doc.block_attrs(block_idx).unwrap_or_default();
                self.push_undo_op(UndoOp::SetBlockType {
                    block_idx,
                    block_type: block_type.clone(),
                    attrs: old_attrs,
                });

                let new_indent = current_indent + 1;
                let mut new_attrs = HashMap::new();
                new_attrs.insert("indent".into(), new_indent.to_string());
                let _ = self
                    .editor_doc
                    .set_block_type(block_idx, &block_type, Some(new_attrs));

                // Re-render the list group
                self.rerender_list_group(block_idx);
                self.set_cursor_from_editor_pos(pos);

                dispatch_ce_event(&CeEvent::BlockIndented {
                    old_block_id: 0,
                    new_li_id: 0,
                    list_id: 0,
                });
                self.notify_blocks_changed();
                self.commit_undo_group();
            }
            _ => {
                // Not a list item — convert to bullet list
                self.begin_undo_group();
                let old_attrs = self.editor_doc.block_attrs(block_idx).unwrap_or_default();
                self.push_undo_op(UndoOp::SetBlockType {
                    block_idx,
                    block_type: block_type.clone(),
                    attrs: old_attrs,
                });

                let _ = self
                    .editor_doc
                    .set_block_type(block_idx, "bullet_list", None);

                self.rerender_list_group(block_idx);
                self.set_cursor_from_editor_pos(pos);

                dispatch_ce_event(&CeEvent::BlockIndented {
                    old_block_id: 0,
                    new_li_id: 0,
                    list_id: 0,
                });
                self.notify_blocks_changed();
                self.commit_undo_group();
            }
        }
    }

    fn outdent(&mut self) {
        let pos = self.cursor_editor_pos();
        let resolved = self
            .editor_doc
            .resolve_position(rinch_editor::Position::new(pos));
        let block_idx = match resolved {
            Ok(r) => r.block_index,
            Err(_) => return,
        };
        let block_type = match self.editor_doc.block_type(block_idx) {
            Some(t) => t,
            None => return,
        };

        match block_type.as_str() {
            "bullet_list" | "ordered_list" => {
                let current_indent: usize = self
                    .editor_doc
                    .block_attrs(block_idx)
                    .and_then(|a| a.get("indent").and_then(|s| s.parse().ok()))
                    .unwrap_or(0);

                self.begin_undo_group();
                let old_attrs = self.editor_doc.block_attrs(block_idx).unwrap_or_default();
                self.push_undo_op(UndoOp::SetBlockType {
                    block_idx,
                    block_type: block_type.clone(),
                    attrs: old_attrs,
                });

                if current_indent == 0 {
                    // At top level — convert to paragraph
                    let _ = self.editor_doc.set_block_type(block_idx, "paragraph", None);
                } else {
                    // Decrease indent
                    let new_indent = current_indent - 1;
                    let mut new_attrs = HashMap::new();
                    if new_indent > 0 {
                        new_attrs.insert("indent".into(), new_indent.to_string());
                    }
                    let attrs_opt = if new_attrs.is_empty() {
                        None
                    } else {
                        Some(new_attrs)
                    };
                    let _ = self
                        .editor_doc
                        .set_block_type(block_idx, &block_type, attrs_opt);
                }

                self.rerender_list_group(block_idx);
                self.set_cursor_from_editor_pos(pos);

                dispatch_ce_event(&CeEvent::ListItemOutdented {
                    old_li_id: 0,
                    new_block_id: 0,
                });
                self.notify_blocks_changed();
                self.commit_undo_group();
            }
            _ => {
                // Not a list item — nothing to outdent
            }
        }
    }

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
        if let Some((cursor, anchor)) = self.replay_undo() {
            self.cursor = cursor;
            self.anchor = anchor;
        }
        dispatch_ce_event(&CeEvent::UndoApplied);
    }

    fn redo(&mut self) {
        if let Some((cursor, anchor)) = self.replay_redo() {
            self.cursor = cursor;
            self.anchor = anchor;
        }
        dispatch_ce_event(&CeEvent::RedoApplied);
    }

    // ── Event Access ─────────────────────────────────────────────────

    fn event_dispatcher(&self) -> &CeEventDispatcher {
        &self.dispatcher
    }

    fn event_dispatcher_mut(&mut self) -> &mut CeEventDispatcher {
        &mut self.dispatcher
    }

    fn has_active_mark(&self, tag: &str) -> bool {
        let d = self.doc.borrow();
        find_formatting_ancestor(&d.tree, self.cursor.node_id, tag, self.ce_node_id).is_some()
    }

    fn cursor_block_tag(&self) -> Option<String> {
        let d = self.doc.borrow();
        let block_id = find_ce_root_child(&d.tree, self.cursor.node_id, self.ce_node_id)?;
        d.tree
            .get(block_id)
            .and_then(|n| n.tag())
            .map(|s| s.to_string())
    }

    fn extract_content(&self) -> Vec<rinch_core::ce::BlockData> {
        let d = self.doc.borrow();
        let children = d.tree.nodes[self.ce_node_id].children.clone();
        let mut blocks = Vec::new();
        for &child_id in &children {
            extract_block(&d.tree, child_id, &mut blocks);
        }
        blocks
    }

    fn load_content(&mut self, blocks: &[rinch_core::ce::BlockData]) {
        // Load into EditorDocument first
        self.editor_doc = EditorDocument::from_block_data(blocks);
        self.skip_next_sync = true;

        // Re-render entire DOM from EditorDocument
        let ce_root = self.ce_node_id;
        {
            let mut d = self.doc.borrow_mut();
            let children: Vec<usize> = d.tree.nodes[ce_root].children.clone();
            for &child_id in &children {
                d.remove_node(rinch_core::dom::NodeId(child_id));
            }

            if blocks.is_empty() {
                let p = d.create_element("p");
                d.append_child(rinch_core::dom::NodeId(ce_root), p);
                let text = d.create_text("");
                d.append_child(p, text);
                self.cursor = DomCursor::new(text.0, 0);
                self.anchor = self.cursor;
            } else {
                crate::ce_render::load_blocks(&mut d, ce_root, blocks);
            }

            // Initialize block virtualization for large documents.
            self.virtual_window = Some(
                crate::app::contenteditable::ce_virtualization::CeVirtualWindow::new(
                    ce_root, &mut d,
                ),
            );
        }

        self.rebuild_block_map();

        // Set cursor to start
        self.set_cursor_from_editor_pos(0);
    }

    fn load_html(&mut self, html: &str) {
        let ce_root = self.ce_node_id;
        {
            let mut d = self.doc.borrow_mut();
            d.set_inner_html(rinch_core::dom::NodeId(ce_root), html);

            if d.tree.nodes[ce_root].children.is_empty() {
                let p = d.create_element("p");
                d.append_child(rinch_core::dom::NodeId(ce_root), p);
                let text = d.create_text("");
                d.append_child(p, text);
                self.cursor = DomCursor::new(text.0, 0);
                self.anchor = self.cursor;
                self.virtual_window = None;
            } else {
                self.virtual_window = Some(
                    crate::app::contenteditable::ce_virtualization::CeVirtualWindow::new(
                        ce_root, &mut d,
                    ),
                );
            }
        }

        // Sync EditorDocument from DOM (since HTML parsing produced the DOM)
        let blocks = self.extract_content();
        self.editor_doc = EditorDocument::from_block_data(&blocks);
        self.skip_next_sync = true;
        self.rebuild_block_map();
        self.set_cursor_from_editor_pos(0);
    }

    fn clear_formatting(&mut self) {
        if self.cursor == self.anchor {
            return;
        }
        self.begin_undo_group();

        let (start, end) = self.ordered_editor_selection();
        if start == end {
            self.commit_undo_group();
            return;
        }

        // Record undo — use snapshot since we're removing many marks
        let pre_snapshot = self.extract_content();
        self.push_undo_op(UndoOp::Snapshot {
            blocks: pre_snapshot,
        });

        // Remove all mark types from EditorDocument
        let mark_types = [
            "bold",
            "italic",
            "underline",
            "strike",
            "code",
            "highlight",
            "subscript",
            "superscript",
        ];
        for mark_type in mark_types {
            let _ = self
                .editor_doc
                .remove_mark(EditorRange::new(start, end), mark_type);
        }

        // Re-render affected blocks
        self.render_blocks_in_range(start, end);

        // Restore selection
        self.set_cursor_from_editor_pos(end);
        let anchor_cursor = {
            let d = self.doc.borrow();
            crate::ce_render::editor_pos_to_dom_cursor(&d.tree, &self.block_map, start)
        };
        if let Some(ac) = anchor_cursor {
            self.anchor = ac;
        }

        dispatch_ce_event(&CeEvent::SelectionUnwrapped {
            tag: "all".to_string(),
            unwrapped_node_ids: vec![],
        });
        self.commit_undo_group();
    }

    // ── Downcasting ────────────────────────────────────────────────────

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
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
