//! DOM reconciliation: EditorDocument -> contentEditable DOM.
//!
//! Renders the editor's document model into DOM elements inside a
//! contentEditable div. Uses block-level diffing with two-level hashing:
//!
//! - **Structure hash**: block type, attrs, inline run count/types/marks
//! - **Content hash**: text content only
//!
//! When only text changes (structure hash matches), text nodes are updated
//! in-place via `set_text()` — no DOM node destruction/creation needed.
//! When structure changes, the block is fully re-rendered.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Weak;

use crate::document::{EditorDocument, InlineRun, MarkData};
use crate::editor::TableCellRef;
use crate::extensions::table_model::TableModel;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};

use super::view_desc::{BlockMapping, TextNodeMapping, ViewDesc};

/// Cached state for a single rendered block.
pub struct BlockCacheEntry {
    /// The block's root DOM node.
    pub node: NodeHandle,
    /// Hash of block structure (type, attrs, run types, marks) — NOT text content.
    pub structure_hash: u64,
    /// Hash of text content only.
    pub content_hash: u64,
}

/// Reconcile the editor document into the contentEditable container.
///
/// Performs block-level diffing with two-level hashing:
/// 1. If both hashes match → skip (no change)
/// 2. If structure matches but content differs → update text nodes in-place
/// 3. If structure differs → full remove + re-render
/// 4. New blocks → create and insert
/// 5. Excess DOM nodes → remove
pub fn reconcile(
    scope: &mut RenderScope,
    container: &NodeHandle,
    doc: &EditorDocument,
    block_cache: &mut HashMap<usize, BlockCacheEntry>,
    tables: &HashMap<String, TableModel>,
    view_desc: &mut ViewDesc,
    table_selection: Option<&TableCellRef>,
) {
    let block_count = doc.block_count();

    // Remove excess block nodes
    let old_count = block_cache.len();
    for i in block_count..old_count {
        if let Some(entry) = block_cache.remove(&i) {
            entry.node.remove();
        }
    }
    view_desc.truncate_blocks(block_count);

    let doc_weak = scope.doc_weak();

    // Update or create blocks
    for i in 0..block_count {
        let structure_hash = compute_structure_hash(doc, i, tables, table_selection);
        let content_hash = compute_content_hash(doc, i);

        if let Some(entry) = block_cache.get(&i) {
            if entry.structure_hash == structure_hash && entry.content_hash == content_hash {
                // Block completely unchanged, skip
                continue;
            }

            if entry.structure_hash == structure_hash {
                // Structure same, only text changed → update text nodes in-place
                update_text_in_place(&doc_weak, doc, i, view_desc);
                // Update content hash in cache
                block_cache.get_mut(&i).unwrap().content_hash = content_hash;
                continue;
            }

            // Structure changed — remove old, will create new below
            entry.node.remove();
        }

        // Full render for new or structurally changed blocks
        let (block_node, text_mappings) = render_block(scope, doc, i, tables, table_selection);
        // Re-query children after potential removal above so index i is correct
        // (removal shifts subsequent children down; querying fresh gives the right reference).
        let children = container.children();
        if i < children.len() {
            container.insert_before(&block_node, &children[i]);
        } else {
            container.append_child(&block_node);
        }

        view_desc.add_block(BlockMapping {
            dom_node_id: block_node.node_id().0,
            block_index: i,
            text_nodes: text_mappings,
        });

        block_cache.insert(
            i,
            BlockCacheEntry {
                node: block_node,
                structure_hash,
                content_hash,
            },
        );
    }
}

/// Perform a full render of the document (initial render or after undo/redo).
///
/// Clears and rebuilds the ViewDesc completely.
pub fn full_render(
    scope: &mut RenderScope,
    container: &NodeHandle,
    doc: &EditorDocument,
    block_cache: &mut HashMap<usize, BlockCacheEntry>,
    tables: &HashMap<String, TableModel>,
    view_desc: &mut ViewDesc,
    table_selection: Option<&TableCellRef>,
) {
    // Clear existing content
    for (_i, entry) in block_cache.drain() {
        entry.node.remove();
    }
    view_desc.clear();

    let block_count = doc.block_count();
    for i in 0..block_count {
        let (block_node, text_mappings) = render_block(scope, doc, i, tables, table_selection);
        container.append_child(&block_node);

        view_desc.add_block(BlockMapping {
            dom_node_id: block_node.node_id().0,
            block_index: i,
            text_nodes: text_mappings,
        });

        block_cache.insert(
            i,
            BlockCacheEntry {
                node: block_node,
                structure_hash: compute_structure_hash(doc, i, tables, table_selection),
                content_hash: compute_content_hash(doc, i),
            },
        );
    }
}

/// Update text nodes in-place when only text content changed (structure is identical).
///
/// Iterates over inline runs and updates each text node's content via `set_text()`,
/// then refreshes the ViewDesc byte ranges. No DOM nodes are created or destroyed.
pub(crate) fn update_text_in_place(
    doc_weak: &Weak<RefCell<dyn DomDocument>>,
    doc: &EditorDocument,
    block_index: usize,
    view_desc: &mut ViewDesc,
) {
    let runs = doc.block_inline_runs(block_index);
    let existing_text_nodes = view_desc.text_nodes_for_block(block_index).to_vec();
    let block_node_id = match view_desc.block_node_id(block_index) {
        Some(id) => id,
        None => return,
    };

    let mut byte_offset = 0;
    let mut new_mappings = Vec::new();
    let mut mapping_idx = 0;

    for run in &runs {
        if run.inline_type == "text" && mapping_idx < existing_text_nodes.len() {
            let existing = &existing_text_nodes[mapping_idx];
            // Update text content in-place — no DOM node destruction
            let text_node = NodeHandle::new(NodeId(existing.dom_node_id), doc_weak.clone());
            text_node.set_text(&run.text);

            new_mappings.push(TextNodeMapping {
                dom_node_id: existing.dom_node_id,
                block_byte_start: byte_offset,
                block_byte_end: byte_offset + run.text.len(),
            });
            mapping_idx += 1;
        }
        byte_offset += run.text.len();
    }

    // Update ViewDesc with new byte ranges (same DOM node IDs)
    view_desc.add_block(BlockMapping {
        dom_node_id: block_node_id,
        block_index,
        text_nodes: new_mappings,
    });
}

/// Compute a hash of block structure for change detection.
///
/// Covers everything EXCEPT actual text content: block type, attributes,
/// inline run count, run types, and mark types/attrs. A matching structure
/// hash means only text changed — safe for in-place updates.
fn compute_structure_hash(
    doc: &EditorDocument,
    block_index: usize,
    tables: &HashMap<String, TableModel>,
    table_selection: Option<&TableCellRef>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    // Hash block type
    if let Some(bt) = doc.block_type(block_index) {
        bt.hash(&mut hasher);
    }

    // Hash block attrs — sort keys for deterministic hash regardless of HashMap iteration order
    if let Some(attrs) = doc.block_attrs(block_index) {
        let mut keys: Vec<_> = attrs.keys().collect();
        keys.sort();
        for k in &keys {
            k.hash(&mut hasher);
            attrs[*k].hash(&mut hasher);
        }

        // For table blocks, also hash the table content
        if let Some(table_id) = attrs.get("table_id")
            && let Some(table) = tables.get(table_id)
        {
            hash_table(table, &mut hasher);
        }

        // Hash table selection state so cell highlight triggers re-render
        if let Some(sel) = table_selection
            && attrs.get("table_id").map(|id| id.as_str()) == Some(&sel.table_id)
        {
            sel.row.hash(&mut hasher);
            sel.col.hash(&mut hasher);
            "selected".hash(&mut hasher);
        }
    }

    // Hash inline run structure (count, types, marks) — NOT text content
    let runs = doc.block_inline_runs(block_index);
    runs.len().hash(&mut hasher);
    for run in &runs {
        run.inline_type.hash(&mut hasher);
        run.marks.len().hash(&mut hasher);
        for mark in &run.marks {
            mark.mark_type.hash(&mut hasher);
            // Sort mark attr keys for deterministic hash regardless of HashMap iteration order
            let mut mark_keys: Vec<_> = mark.attrs.keys().collect();
            mark_keys.sort();
            for k in &mark_keys {
                k.hash(&mut hasher);
                mark.attrs[*k].hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

/// Compute a hash of block text content only.
fn compute_content_hash(doc: &EditorDocument, block_index: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    if let Some(text) = doc.block_text(block_index) {
        text.hash(&mut hasher);
    }

    hasher.finish()
}

/// Hash table content for change detection.
fn hash_table(table: &TableModel, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    table.row_count().hash(hasher);
    table.col_count().hash(hasher);
    table.header_rows.hash(hasher);
    for row in &table.rows {
        for cell in &row.cells {
            cell.content.hash(hasher);
            cell.colspan.hash(hasher);
            cell.rowspan.hash(hasher);
            cell.is_header.hash(hasher);
        }
    }
}

/// Render a single block to the appropriate HTML element.
///
/// Returns the block's DOM node and a list of text node mappings
/// recording which text nodes map to which byte ranges within the block.
fn render_block(
    scope: &mut RenderScope,
    doc: &EditorDocument,
    block_index: usize,
    tables: &HashMap<String, TableModel>,
    table_selection: Option<&TableCellRef>,
) -> (NodeHandle, Vec<TextNodeMapping>) {
    let block_type = doc
        .block_type(block_index)
        .unwrap_or_else(|| "paragraph".into());
    let attrs = doc.block_attrs(block_index).unwrap_or_default();

    // Handle table blocks specially (no text node mappings for tables)
    if block_type == "table" {
        return (
            render_table_block(scope, block_index, &attrs, tables, table_selection),
            Vec::new(),
        );
    }

    let tag = match block_type.as_str() {
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
        "bullet_list" => "li",
        "ordered_list" => "li",
        "list_item" => "li",
        "horizontal_rule" => "hr",
        _ => "p",
    };

    let element = scope.create_element(tag);
    element.set_attribute("data-block-index", &block_index.to_string());

    // Apply alignment if present
    if let Some(align) = attrs.get("align") {
        element.set_attribute("style", &format!("text-align: {};", align));
    }

    // For hr, no inline content needed
    if block_type == "horizontal_rule" {
        return (element, Vec::new());
    }

    let runs = doc.block_inline_runs(block_index);
    let mut text_mappings = Vec::new();

    // For code_block, wrap content in <code>
    if block_type == "code_block" {
        let code = scope.create_element("code");
        render_inline_runs(scope, &code, &runs, &mut text_mappings);
        element.append_child(&code);
        return (element, text_mappings);
    }

    // Render inline content
    render_inline_runs(scope, &element, &runs, &mut text_mappings);

    (element, text_mappings)
}

/// Render a table block using div-based flexbox layout.
///
/// rinch-dom uses Taffy for layout (no native `<table>` support), so we
/// render tables as nested `<div>` elements with flexbox styling:
/// - Outer div: column direction (stacks rows)
/// - Row div: row direction with equal-width cells
/// - Cell div: flex:1 with border and padding
fn render_table_block(
    scope: &mut RenderScope,
    block_index: usize,
    attrs: &HashMap<String, String>,
    tables: &HashMap<String, TableModel>,
    table_selection: Option<&TableCellRef>,
) -> NodeHandle {
    let table_el = scope.create_element("div");
    table_el.set_attribute("data-block-index", &block_index.to_string());
    table_el.set_attribute("data-table", "true");
    if let Some(tid) = attrs.get("table_id") {
        table_el.set_attribute("data-table-id", tid);
    }
    table_el.set_attribute(
        "style",
        "display: flex; flex-direction: column; width: 100%; \
         margin: 8px 0; border: 1px solid #ced4da; border-radius: 4px; \
         overflow: hidden;",
    );

    let table_id = match attrs.get("table_id") {
        Some(id) => id,
        None => {
            let placeholder = scope.create_element("div");
            placeholder.set_attribute("style", "padding: 8px 12px; color: #868e96;");
            placeholder.set_text("[Table: no data]");
            table_el.append_child(&placeholder);
            return table_el;
        }
    };

    let table = match tables.get(table_id) {
        Some(t) => t,
        None => {
            let placeholder = scope.create_element("div");
            placeholder.set_attribute("style", "padding: 8px 12px; color: #868e96;");
            placeholder.set_text("[Table: data not found]");
            table_el.append_child(&placeholder);
            return table_el;
        }
    };

    // Render table rows as flex rows
    for (row_idx, row) in table.rows.iter().enumerate() {
        let tr = scope.create_element("div");
        tr.set_attribute("data-table-row", &row_idx.to_string());

        // Row styling: horizontal flex, with bottom border between rows
        let row_border = if row_idx + 1 < table.rows.len() {
            "border-bottom: 1px solid #ced4da; "
        } else {
            ""
        };
        tr.set_attribute(
            "style",
            &format!("display: flex; flex-direction: row; {}", row_border,),
        );

        for (col_idx, cell) in row.cells.iter().enumerate() {
            let td = scope.create_element("div");
            td.set_attribute("data-row", &row_idx.to_string());
            td.set_attribute("data-col", &col_idx.to_string());
            td.set_attribute(
                "data-table-cell",
                &format!("{}:{}:{}", table_id, row_idx, col_idx),
            );

            // Cell styling: flex:1 for equal width, with right border between cells
            let col_border = if col_idx + 1 < row.cells.len() {
                "border-right: 1px solid #ced4da; "
            } else {
                ""
            };

            let bg = if cell.is_header {
                "background: #f1f3f5; font-weight: 600; "
            } else {
                ""
            };

            // Check if this cell is selected
            let selection_style = match table_selection {
                Some(sel)
                    if sel.table_id == *table_id && sel.row == row_idx && sel.col == col_idx =>
                {
                    "outline: 2px solid #228be6; outline-offset: -2px; background: #e7f5ff; "
                }
                _ => "",
            };

            td.set_attribute(
                "style",
                &format!(
                    "flex: 1; padding: 8px 12px; min-width: 50px; {}{}{}",
                    col_border, bg, selection_style,
                ),
            );

            // Cell content
            if cell.content.is_empty() {
                let text = scope.create_text("\u{200B}");
                td.append_child(&text);
            } else {
                let text = scope.create_text(&cell.content);
                td.append_child(&text);
            }

            tr.append_child(&td);
        }

        table_el.append_child(&tr);
    }

    table_el
}

/// Render inline runs into a parent element, recording text node mappings.
///
/// Each text run's DOM text node is recorded with its byte offset range
/// within the block's text content, enabling DomCursor ↔ Position translation.
fn render_inline_runs(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    runs: &[InlineRun],
    text_mappings: &mut Vec<TextNodeMapping>,
) {
    let mut byte_offset = 0;

    for run in runs {
        let (node, text_node_id) = render_inline_run_mapped(scope, run);
        parent.append_child(&node);

        if let Some(tn_id) = text_node_id {
            text_mappings.push(TextNodeMapping {
                dom_node_id: tn_id,
                block_byte_start: byte_offset,
                block_byte_end: byte_offset + run.text.len(),
            });
        }

        byte_offset += run.text.len();
    }
}

/// Render a single inline run, returning the DOM node and optional text node ID.
///
/// Returns `(dom_node, Some(text_node_dom_id))` for text runs,
/// `(dom_node, None)` for hard breaks and unknown types.
fn render_inline_run_mapped(
    scope: &mut RenderScope,
    run: &InlineRun,
) -> (NodeHandle, Option<usize>) {
    match run.inline_type.as_str() {
        "hard_break" => (scope.create_element("br"), None),
        "text" => {
            if run.marks.is_empty() {
                let text_node = scope.create_text(&run.text);
                let id = text_node.node_id().0;
                (text_node, Some(id))
            } else {
                let (wrapper, text_node_id) = wrap_in_marks_mapped(scope, &run.text, &run.marks);
                (wrapper, Some(text_node_id))
            }
        }
        _ => (scope.create_text(""), None),
    }
}

/// Wrap text content in nested mark elements, returning the outer wrapper
/// and the inner text node's DOM ID.
fn wrap_in_marks_mapped(
    scope: &mut RenderScope,
    text: &str,
    marks: &[MarkData],
) -> (NodeHandle, usize) {
    let text_node = scope.create_text(text);
    let text_node_id = text_node.node_id().0;

    if marks.is_empty() {
        return (text_node, text_node_id);
    }

    // Build from innermost to outermost
    let mut current = text_node;
    for mark in marks.iter().rev() {
        let wrapper = mark_element(scope, mark);
        wrapper.append_child(&current);
        current = wrapper;
    }

    (current, text_node_id)
}

/// Create the HTML element for a mark.
fn mark_element(scope: &mut RenderScope, mark: &MarkData) -> NodeHandle {
    match mark.mark_type.as_str() {
        "bold" => scope.create_element("strong"),
        "italic" => scope.create_element("em"),
        "underline" => scope.create_element("u"),
        "strike" | "strikethrough" => scope.create_element("s"),
        "code" => scope.create_element("code"),
        "highlight" => scope.create_element("mark"),
        "subscript" => scope.create_element("sub"),
        "superscript" => scope.create_element("sup"),
        "link" => {
            let a = scope.create_element("a");
            if let Some(href) = mark.attrs.get("href") {
                a.set_attribute("href", href);
            }
            a
        }
        "textColor" => {
            let span = scope.create_element("span");
            if let Some(color) = mark.attrs.get("color") {
                span.set_attribute("style", &format!("color: {};", color));
            }
            span
        }
        _ => {
            // Unknown mark: wrap in span
            let span = scope.create_element("span");
            span.set_attribute("data-mark", &mark.mark_type);
            span
        }
    }
}
