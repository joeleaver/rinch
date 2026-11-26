//! Structural commands: splitting blocks and list items (Enter) and inserting
//! breaks, rules, tables and images.

use crate::Fragment;
use crate::command::{Command, chain};
use crate::model::{Attrs, Node};
use crate::pos::Pos;
use crate::schema::data_attrs::without_app_data;
use crate::state::{EditorState, Transaction};

use super::data_attrs::remove_app_data;
use super::wrapping::build_lift;
use super::{command_tr, newline_in_code, table_ops};

// ===================================================================
// Structure & deletion
// ===================================================================

/// Split the current textblock at the cursor (Enter). Deletes a non-empty
/// selection first; at the end of a heading, the new block becomes a paragraph.
///
/// An empty block made by the split is made without the block's
/// [app data attributes](crate::schema::data_attrs): at its end the new block
/// below, at its start the one above (the text keeps them). A split in the
/// middle gives both halves the block's attributes.
///
/// Refuses (`None`) when the cursor, after that delete, is not in a textblock:
/// a caret at a gap between blocks (between a rule and a paragraph in a table
/// cell, #1232; inside a table row) would otherwise split the node around the
/// gap — the cell, leaving a ragged row. Enter puts a paragraph at such a gap
/// instead ([`create_paragraph_near`]). A split never cuts an isolating node in
/// any case ([`Transform::split`](crate::transform::Transform::split)).
pub fn split_block() -> Command {
    command_tr(|state| {
        let mut tr = state.tr();
        if !state.selection.is_empty() {
            tr.delete_selection().ok()?;
        }
        let pos = tr.selection().from().0;
        let r = tr.doc().resolve(Pos(pos)).ok()?;
        if r.depth() < 1 || !r.parent().is_textblock() {
            return None;
        }
        let parent = r.parent();
        let at_end = r.parent_offset() == parent.content().size();
        let at_start = !at_end && r.parent_offset() == 0;
        let types_after = if at_end && parent.type_name() == "heading" {
            let para = state.schema().node_type("paragraph")?.clone();
            Some(vec![Some((para, Attrs::new()))])
        } else if at_end {
            Some(vec![Some((
                parent.node_type().clone(),
                without_app_data(parent),
            ))])
        } else {
            None
        };
        let first = r.before(r.depth())?;
        tr.split(pos, 1, types_after).ok()?;
        if at_start {
            remove_app_data(&mut tr, first)?;
        }
        Some(tr)
    })
}

/// Split the current list item into a new one (Enter inside a list). Port of
/// ProseMirror's `splitListItem`: when the cursor is in a textblock whose parent
/// is `item_type` (e.g. `list_item`), split **two levels** so the text after the
/// cursor moves into a fresh sibling item. Returns `None` (inapplicable) when not
/// in a list item or when the item's textblock is empty — the [`split_block_or_list_item`]
/// chain then falls through to [`lift_empty_block`] (which exits the list).
pub fn split_list_item(item_type: &'static str) -> Command {
    command_tr(move |state| {
        let r_from = state.doc.resolve(state.selection.from()).ok()?;
        let r_to = state.doc.resolve(state.selection.to()).ok()?;
        // Need at least `… > item > textblock` so node(depth-1) is the item, and a
        // selection contained within one textblock.
        if r_from.depth() < 2
            || r_from.depth() != r_to.depth()
            || r_from.start(r_from.depth()) != r_to.start(r_to.depth())
        {
            return None;
        }
        if r_from.node(r_from.depth() - 1).type_name() != item_type {
            return None;
        }
        // Empty textblock → not a split; let the chain lift it out of the list.
        if r_from.parent().content().size() == 0 {
            return None;
        }
        let mut tr = state.tr();
        if !state.selection.is_empty() {
            tr.delete_selection().ok()?;
        }
        let pos = tr.selection().from().0;
        let r = tr.doc().resolve(Pos(pos)).ok()?;
        // The new sibling item keeps the item type but **resets its attrs** — splitting a
        // *checked* task item must yield a fresh *unchecked* one, never inherit the tick.
        // (list_item has no attrs, so this is a no-op there.)
        let item_ty = r.node(r.depth() - 1).node_type().clone();
        let new_item = Some((item_ty, Attrs::new()));
        // At the end of the textblock the new item starts with the list item's
        // default content (a paragraph) rather than copying a heading/code block; mid-text
        // the second half keeps its own textblock type (`None`).
        let at_end = r.parent_offset() == r.parent().content().size();
        let at_start = !at_end && r.parent_offset() == 0;
        let inner = if at_end {
            Some((state.schema().node_type("paragraph")?.clone(), Attrs::new()))
        } else {
            None
        };
        let first = r.before(r.depth())?;
        tr.split(pos, 2, Some(vec![new_item, inner])).ok()?;
        if at_start {
            remove_app_data(&mut tr, first)?;
        }
        Some(tr)
    })
}

/// Lift an empty textblock out of its wrapper (PM `liftEmptyBlock`): pressing Enter
/// in an empty list item / blockquote paragraph exits it. If the empty block is not
/// the last child of its parent, split the parent before it instead.
pub fn lift_empty_block() -> Command {
    command_tr(|state| {
        if !state.selection.is_empty() {
            return None;
        }
        let r = state.doc.resolve(state.selection.from()).ok()?;
        if !r.parent().is_textblock() || r.parent().content().size() != 0 {
            return None;
        }
        // Not the last child of its grandparent → split the grandparent before it.
        // A table cell is never split (`Transform::split` refuses an isolating
        // cut, #1232): the empty block is then split as any other, below.
        if r.depth() >= 2
            && r.after(r.depth()) != Some(r.end(r.depth() - 1))
            && let Some(before) = r.before(r.depth())
        {
            let mut tr = state.tr();
            if tr.split(before, 1, None).is_ok() && tr.doc_changed() {
                return Some(tr);
            }
        }
        // Otherwise lift the empty block out of its wrapper (exits a list, etc.).
        build_lift(state)
    })
}

/// Put an empty paragraph at a caret that is not in a textblock — a gap between
/// two blocks, such as between a horizontal rule and a paragraph in a table cell —
/// and move the caret into it. Port of ProseMirror's `createParagraphNear` for a
/// collapsed text selection (a node selection is left to the rest of the Enter
/// chain, which replaces the node). `None` when the caret is in a textblock, or
/// when a paragraph is not valid content at the gap (inside a table row).
pub fn create_paragraph_near() -> Command {
    command_tr(|state| {
        if !state.selection.is_text_cursor() {
            return None;
        }
        let pos = state.selection.from().0;
        let r = state.doc.resolve(Pos(pos)).ok()?;
        if r.parent().is_textblock() {
            return None;
        }
        let para = state
            .schema()
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .ok()?;
        let mut tr = state.tr();
        tr.replace_with(pos, pos, Fragment::from_node(para)).ok()?;
        tr.set_selection(crate::selection::Selection::cursor(Pos(pos + 1)));
        Some(tr)
    })
}

/// The Enter handler (`splitBlock` is the toolbar/programmatic name; Enter binds
/// here): in a code block type a newline ([`newline_in_code`], #2993), else
/// split a list item into a new item, else put a paragraph at a caret between
/// blocks, else lift an empty block out of its wrapper, else split the current
/// block. ProseMirror's base Enter chain with its list keymap's
/// `splitListItem` ahead of it — after `newlineInCode`, so Enter in a code
/// block inside a list item is a newline, not a new item.
pub fn split_block_or_list_item() -> Command {
    chain(vec![
        newline_in_code(),
        split_list_item("list_item"),
        split_list_item("task_item"),
        create_paragraph_near(),
        lift_empty_block(),
        split_block(),
    ])
}

/// The transaction an insert starts from: over a cell selection, the
/// selected cells cleared and the caret in the first of them, where the
/// inserted node goes (prosemirror-tables' `CellSelection.replace`, as typing
/// and paste over one do, #1847); otherwise a fresh transaction.
fn insertion_tr(state: &EditorState) -> Option<Transaction> {
    if matches!(state.selection, crate::selection::Selection::Cell(_)) {
        table_ops::clear_cells(state)
    } else {
        Some(state.tr())
    }
}

/// Insert a hard break at the cursor (Shift+Enter).
pub fn insert_hard_break() -> Command {
    command_tr(|state| {
        let hb = state
            .schema()
            .create_node("hard_break", Attrs::new(), Fragment::empty())
            .ok()?;
        let mut tr = insertion_tr(state)?;
        tr.replace_selection_with(hb).ok()?;
        Some(tr)
    })
}

/// Insert a horizontal rule, replacing the selection.
pub fn insert_horizontal_rule() -> Command {
    command_tr(|state| {
        let hr = state
            .schema()
            .create_node("horizontal_rule", Attrs::new(), Fragment::empty())
            .ok()?;
        let mut tr = insertion_tr(state)?;
        tr.insert_block(hr).ok()?;
        Some(tr)
    })
}

/// Build a `table` node with `rows`×`cols` body cells, each holding one empty
/// paragraph. `rows`/`cols` are clamped to a minimum of 1. `None` if the schema
/// lacks table types.
pub fn build_table(schema: &crate::Schema, rows: usize, cols: usize) -> Option<Node> {
    let rows = rows.max(1);
    let cols = cols.max(1);
    let make_cell = || {
        let para = schema
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .ok()?;
        schema
            .create_node("table_cell", Attrs::new(), Fragment::from_node(para))
            .ok()
    };
    let mut row_nodes = Vec::with_capacity(rows);
    for _ in 0..rows {
        let mut cells = Vec::with_capacity(cols);
        for _ in 0..cols {
            cells.push(make_cell()?);
        }
        row_nodes.push(
            schema
                .create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .ok()?,
        );
    }
    schema
        .create_node("table", Attrs::new(), Fragment::from_children(row_nodes))
        .ok()
}

/// Insert a `rows`×`cols` table, replacing the selection. A builder (the dims are
/// the caller's; the registered `insertTable` command uses a default size).
pub fn insert_table(rows: usize, cols: usize) -> Command {
    command_tr(move |state| {
        let table = build_table(state.schema(), rows, cols)?;
        let mut tr = insertion_tr(state)?;
        tr.insert_block(table).ok()?;
        Some(tr)
    })
}

/// Insert an image with `src`/`alt`, replacing the selection. A builder.
pub fn insert_image(src: String, alt: String) -> Command {
    command_tr(move |state| {
        let attrs = Attrs::from_iter([
            ("src", crate::AttrValue::from(src.clone())),
            ("alt", crate::AttrValue::from(alt.clone())),
        ]);
        let img = state
            .schema()
            .create_node("image", attrs, Fragment::empty())
            .ok()?;
        let mut tr = insertion_tr(state)?;
        tr.replace_selection_with(img).ok()?;
        Some(tr)
    })
}
