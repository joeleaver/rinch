//! Table structure commands — add/remove rows and columns, merge and split cells,
//! delete the table. A faithful port of the structural half of ProseMirror's
//! `prosemirror-tables/src/commands.ts`, operating over the absolute-position
//! [`TableMap`].
//!
//! Two simplifications versus PM, both safe for the starter-kit schema (documented
//! at each site): a row `add_row` inserts is always plain `table_cell`s (no
//! header-row inference), and there is no `colwidth` attribute to carry.
//!
//! One addition: a cell a command makes (a new row's or column's, a split's, a
//! cleared one's paragraph) takes the `text_align` of the cell it is modelled
//! on, where PM's `createAndFill` gives a default one (#2210). An unset
//! alignment is not `left`, and a column mixing the two has no Markdown
//! delimiter, so without it `addRowAfter` in a `:---` table turned the pipe
//! table into an HTML one.
//!
//! All commands read **state** (the selection + document), never the host DOM, and
//! produce a single [`Transaction`]; structural integrity comes from the transform
//! engine's schema gate, exactly like the rest of the catalogue.

use crate::AttrValue;
use crate::command::Command;
use crate::commands::command_tr;
use crate::model::{Attrs, Fragment, Node, Slice};
use crate::pos::Pos;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::tables::{self, Rect, TableMap};
use crate::transform::BatchEdit;
use std::collections::{HashMap, HashSet};

mod cells;
mod navigation;
mod rows_columns;
#[cfg(test)]
mod tests;

pub use cells::{
    cell_selection_slice, clear_cells, delete_cell_selection, merge_cells, split_cell,
};
pub use navigation::{shift_arrow, vertical_entry};
pub use rows_columns::set_fallback_slot_budget_for_tests;
use rows_columns::*;

/// The table, its map, content-start position, and the rectangle a command should
/// operate over (the selected cells, or the single cell under a text cursor).
pub struct TableRect {
    /// The resolved grid of the table.
    pub map: TableMap,
    /// The table node.
    pub table: Node,
    /// Document position just inside the table (where row 0 begins).
    pub table_start: usize,
    /// The grid rectangle the command operates over.
    pub rect: Rect,
}

/// True if the selection is inside a table.
pub fn is_in_table(state: &EditorState) -> bool {
    selected_rect(state).is_some()
}

/// Resolve the table around the current selection and the rectangle the command
/// should act on. For a [`Selection::Cell`] that is the covering rectangle; for any
/// other selection it is the single cell containing the head. Port of
/// `selectedRect` + `selectionCell`.
pub fn selected_rect(state: &EditorState) -> Option<TableRect> {
    let doc = &state.doc;
    let probe = state.selection.head();
    let (map, table, table_start) = tables::map_around(doc, probe)?;
    let rect = match &state.selection {
        Selection::Cell(c) => map.rect_between(c.anchor_cell.0, c.head_cell.0)?,
        _ => {
            let r = doc.resolve(probe).ok()?;
            let cell_pos = tables::cell_around(&r)?;
            map.find_cell(cell_pos)?
        }
    };
    Some(TableRect {
        map,
        table,
        table_start,
        rect,
    })
}

/// An empty `ty` cell (`table_cell` or `table_header_cell`) `cols` columns
/// wide, holding one [`empty_paragraph_like`] `like`. A one-column cell has no
/// `colspan` attribute, as the cell `addRow`/`addColumn` always made.
fn make_empty_cell(schema: &Schema, ty: &str, cols: usize, like: Option<&Node>) -> Option<Node> {
    let para = empty_paragraph_like(schema, like)?;
    let attrs = if cols <= 1 {
        Attrs::new()
    } else {
        Attrs::from_iter([("colspan", span_value(cols))])
    };
    schema
        .create_node(ty, attrs, Fragment::from_node(para))
        .ok()
}

/// An empty paragraph aligned like the first block of the cell `like` (#2210):
/// its `text_align` when it has one and the schema's paragraph takes the
/// attribute, else unset. The alignment is what a Markdown pipe table keeps
/// per column, so a cell made in a column's likeness keeps the column one
/// alignment.
fn empty_paragraph_like(schema: &Schema, like: Option<&Node>) -> Option<Node> {
    let align = like
        .and_then(|cell| cell.content().maybe_child(0))
        .and_then(|block| block.attrs().get_str("text_align"))
        .filter(|_| {
            schema
                .node_type("paragraph")
                .is_some_and(|nt| nt.spec().attrs.contains_key("text_align"))
        });
    let attrs = match align {
        Some(a) => Attrs::from_iter([("text_align", AttrValue::from(a))]),
        None => Attrs::new(),
    };
    schema
        .create_node("paragraph", attrs, Fragment::empty())
        .ok()
}

/// The colspan attribute of `cell`, clamped to ≥1. The document's word, not the
/// grid's: only [`split_cell`] reads it, to tell a merged cell from a plain one.
/// Span arithmetic reads [`TableMap::cell_rect`] instead.
fn colspan(cell: &Node) -> i64 {
    cell.attrs().get_int("colspan").unwrap_or(1).max(1)
}

/// The rowspan attribute of `cell`, clamped to ≥1 (see [`colspan`]).
fn rowspan(cell: &Node) -> i64 {
    cell.attrs().get_int("rowspan").unwrap_or(1).max(1)
}

/// A grid slot no cell covers (a ragged row's tail, or a row cut by the bounded
/// grid, #1176): [`TableMap::map`]'s sentinel.
const HOLE: usize = usize::MAX;

/// A grid extent as a span attribute. An extent is at most the grid's width or
/// height, so it always fits.
fn span_value(extent: usize) -> AttrValue {
    AttrValue::Int(i64::try_from(extent.max(1)).unwrap_or(i64::MAX))
}

/// Every cell of the table by its document position, so a command finds the
/// cell a grid slot names without walking the rows (`Node::node_at` walks
/// the table's row list: one call per row was quadratic, #1200).
struct Cells {
    /// Cell positions, ascending (document order).
    starts: Vec<usize>,
    /// `(row, index in row)` of each entry of `starts`.
    at: Vec<(usize, usize)>,
}

impl Cells {
    fn new(info: &TableRect) -> Cells {
        let mut starts = Vec::new();
        let mut at = Vec::new();
        let mut pos = info.table_start;
        for r in 0..info.table.child_count() {
            let row = info.table.child(r);
            pos += 1;
            for i in 0..row.child_count() {
                starts.push(pos);
                at.push((r, i));
                pos += row.child(i).node_size();
            }
            pos += 1;
        }
        Cells { starts, at }
    }

    /// The cell node at `pos` (a cell position from the map). `None` for a
    /// hole or a position that starts no cell.
    fn node<'a>(&self, info: &'a TableRect, pos: usize) -> Option<&'a Node> {
        let k = self.starts.binary_search(&pos).ok()?;
        let (r, i) = self.at[k];
        Some(info.table.child(r).child(i))
    }
}

// ===================================================================
// Registered commands
// ===================================================================

/// One-step transaction applying `edits` (`None` when there are none).
fn batch_tr(state: &EditorState, edits: Vec<BatchEdit>) -> Option<Transaction> {
    let mut tr = state.tr();
    tr.batch(edits).ok()?;
    tr.doc_changed().then_some(tr)
}

/// `addRowBefore` — insert an empty row above the selection's top row.
pub fn add_row_before() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        let top = info.rect.top;
        batch_tr(state, add_row(&info, top, top, state.schema())?)
    })
}

/// `addRowAfter` — insert an empty row below the selection's bottom row.
pub fn add_row_after() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        let bottom = info.rect.bottom;
        batch_tr(state, add_row(&info, bottom, bottom - 1, state.schema())?)
    })
}

/// `addColumnBefore` — insert an empty column left of the selection.
pub fn add_column_before() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        batch_tr(state, add_column(&info, info.rect.left, state.schema())?)
    })
}

/// `addColumnAfter` — insert an empty column right of the selection.
pub fn add_column_after() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        batch_tr(state, add_column(&info, info.rect.right, state.schema())?)
    })
}

/// `deleteRow` — remove the selected rows. If that would remove every row, the
/// whole table is deleted instead. Port of `deleteRow`.
pub fn delete_row() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        if info.rect.top == 0 && info.rect.bottom == info.map.height() {
            return delete_table_tr(state, info.table_start);
        }
        let cells = Cells::new(&info);
        let lines = info.rect.bottom - info.rect.top;
        let exact = plain_grid(&info, &cells) && !has_hole(&info.map, info.rect.bottom);
        if exact || !one_at_a_time_fits(&info.map, lines) {
            let (top, bottom) = (info.rect.top, info.rect.bottom);
            return batch_tr(
                state,
                remove_rows(&info, &cells, top, bottom, state.schema())?,
            );
        }
        // Overlapping cells (or a hole under the rows): a row at a time from
        // the bottom, one step each.
        let mut tr = state.tr();
        let (table_start, rect) = (info.table_start, info.rect);
        let mut cur = info;
        let mut cells = cells;
        let mut i = rect.bottom;
        loop {
            i -= 1;
            tr.batch(remove_row_on(&cur, &cells, i, state.schema())?)
                .ok()?;
            if i == rect.top {
                break;
            }
            cur = recompute(&tr, table_start, rect)?;
            cells = Cells::new(&cur);
        }
        tr.doc_changed().then_some(tr)
    })
}

/// `deleteColumn` — remove the selected columns (or the whole table if that empties
/// it). Port of `deleteColumn`.
pub fn delete_column() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        if info.rect.left == 0 && info.rect.right == info.map.width() {
            return delete_table_tr(state, info.table_start);
        }
        let cells = Cells::new(&info);
        let lines = info.rect.right - info.rect.left;
        if plain_grid(&info, &cells) || !one_at_a_time_fits(&info.map, lines) {
            let (left, right) = (info.rect.left, info.rect.right);
            return batch_tr(state, remove_columns(&info, &cells, left, right)?);
        }
        // Overlapping cells: a column at a time from the right, one step each.
        let mut tr = state.tr();
        let (table_start, rect) = (info.table_start, info.rect);
        let mut cur = info;
        let mut cells = cells;
        let mut i = rect.right;
        loop {
            i -= 1;
            // A column past the recomputed grid is gone already: removing a
            // column from a ragged table can narrow the grid by more than one
            // (its widest row may be the one that lost a cell).
            if i < cur.map.width() {
                tr.batch(remove_column_on(&cur, &cells, i)?).ok()?;
            }
            if i == rect.left {
                break;
            }
            cur = recompute(&tr, table_start, rect)?;
            cells = Cells::new(&cur);
        }
        tr.doc_changed().then_some(tr)
    })
}

/// `deleteTable` — delete the entire table the selection is in.
pub fn delete_table() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        delete_table_tr(state, info.table_start)
    })
}

/// Build a transaction deleting the whole table whose content starts at
/// `table_start` (the table's open token is at `table_start - 1`).
fn delete_table_tr(state: &EditorState, table_start: usize) -> Option<Transaction> {
    let table_pos = table_start - 1;
    let table = state.doc.node_at(table_pos)?;
    let mut tr = state.tr();
    tr.delete(table_pos, table_pos + table.node_size()).ok()?;
    // Place the cursor sensibly after removing the table.
    let sel = Selection::near(tr.doc(), Pos(table_pos.min(tr.doc().content_size())), -1);
    tr.set_selection(sel);
    Some(tr)
}
