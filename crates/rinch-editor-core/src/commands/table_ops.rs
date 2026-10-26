//! Table structure commands — add/remove rows and columns, merge and split cells,
//! delete the table. A faithful port of the structural half of ProseMirror's
//! `prosemirror-tables/src/commands.ts`, operating over the absolute-position
//! [`TableMap`](crate::tables::TableMap).
//!
//! Two simplifications versus PM, both safe for the starter-kit schema (documented
//! at each site): inserted cells are always plain `table_cell` (no header-type
//! inference), and there is no `colwidth` attribute to carry.
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

/// An empty `table_cell` holding one empty paragraph.
fn make_empty_cell(schema: &Schema) -> Option<Node> {
    let para = schema
        .create_node("paragraph", Attrs::new(), Fragment::empty())
        .ok()?;
    schema
        .create_node("table_cell", Attrs::new(), Fragment::from_node(para))
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

// Every command below states its edits in the coordinates of the document it
// starts from and applies them as one [`crate::transform::BatchStep`] (#1200).
// ProseMirror's port took one step per row (a column insert) or per cell (a
// merge), each mapped through the ones before it; each of those steps rebuilt
// the table's row list and the transaction kept the document from before
// every one, which made a column into 16,000 rows 6.4 s and 2.1 GB. The batch
// maps a position exactly as those steps did, so a caret in a cell the
// command did not touch stays where it was.

// ===================================================================
// Column operations
// ===================================================================

/// Insert a column at grid index `col` (`0..=width`). Where an existing cell spans
/// across the insertion line its colspan grows; otherwise an empty cell is spliced
/// into each row. Port of `addColumn`.
fn add_column(info: &TableRect, col: usize, schema: &Schema) -> Option<Vec<BatchEdit>> {
    let map = &info.map;
    let width = map.width();
    let mut edits = Vec::new();
    let mut row = 0;
    while row < map.height() {
        let index = row * width + col;
        // Two holes side by side are no cell spanning the line.
        let inside_span = col > 0
            && col < width
            && map.map()[index] != HOLE
            && map.map()[index - 1] == map.map()[index];
        if inside_span {
            // Inside a horizontally-spanning cell → widen it.
            let pos = map.map()[index];
            let rect = map.cell_rect(index)?;
            edits.push(BatchEdit::set_attr(
                pos,
                "colspan",
                span_value(rect.right - rect.left + 1),
            ));
            row = rect.bottom.max(row + 1);
        } else {
            let at = map.position_at(row, col);
            edits.push(BatchEdit::insert(
                at,
                Fragment::from_node(make_empty_cell(schema)?),
            ));
            row += 1;
        }
    }
    Some(edits)
}

/// Remove the grid columns `left..right` (`right <= width`). A cell that spans
/// columns outside them shrinks by the columns it loses; a cell wholly inside
/// them is deleted. Ports `removeColumn`, which ProseMirror's `deleteColumn`
/// runs once per column from the right, recomputing the map in between: a
/// column's removal leaves the columns to its left where they were, so every
/// column's cells can be read off the one map.
fn remove_columns(
    info: &TableRect,
    cells: &Cells,
    left: usize,
    right: usize,
) -> Option<Vec<BatchEdit>> {
    let map = &info.map;
    let width = map.width();
    // Each cell once, with its rectangle and the columns of `left..right` it
    // was found in (each column's walk steps over a cell's rows, as
    // `removeColumn` does).
    let mut found: HashMap<usize, (Rect, usize)> = HashMap::new();
    let mut order = Vec::new();
    for col in left..right.min(width) {
        let mut row = 0;
        while row < map.height() {
            let index = row * width + col;
            let pos = map.map()[index];
            // A cell met in an earlier column: its rectangle is known. Asking
            // `cell_rect` again walks back to the cell's left edge, so a row
            // of 1000-wide cells cost 1000 steps a column (#1874).
            if let Some(e) = found.get_mut(&pos) {
                e.1 += 1;
                row = e.0.bottom.max(row + 1);
                continue;
            }
            // A hole: this row has nothing in the column.
            let Some(rect) = map.cell_rect(index) else {
                row += 1;
                continue;
            };
            order.push(pos);
            found.insert(pos, (rect, 1));
            row = rect.bottom.max(row + 1);
        }
    }
    let mut edits = Vec::with_capacity(order.len());
    for pos in order {
        let (rect, lost) = found[&pos];
        let cell = cells.node(info, pos)?;
        // A column whose removal finds the cell spanning a column the
        // command keeps shrinks it; the last one it covers deletes it.
        let kept = rect.left < left || rect.right > right;
        if kept {
            let width = (rect.right - rect.left).saturating_sub(lost);
            edits.push(BatchEdit::set_attr(pos, "colspan", span_value(width)));
        } else {
            edits.push(BatchEdit::delete(pos, pos + cell.node_size()));
        }
    }
    Some(edits)
}

// ===================================================================
// Row operations
// ===================================================================

/// Insert a row at grid index `row` (`0..=height`), shaped like the grid row
/// `like` beside it (`row - 1` for `addRowAfter`, `row` for `addRowBefore`).
///
/// A cell that spans across the insertion line grows its rowspan, as in
/// ProseMirror's `addRow`. The columns it leaves free get empty cells with
/// `like`'s cell boundaries, cut where a free run starts or ends, so a cell of
/// `like` that is 3 columns wide gives a 3-wide cell (#1244). ProseMirror
/// makes one cell per column instead; on a table with no merged cells the two
/// are the same row. A row of 2,100 cells of `colspan = 1000` was a
/// 2,097,152-column grid and `addRowAfter` built a cell per column there: 2 M
/// cells, 32 s and 5 GB. A new row now has at most as many cells as `like`
/// covers, plus its holes.
///
/// A hole in `like` (a ragged row's tail) has no boundary to copy. The holes
/// get one cell per column as before while they are no more than the
/// table's cells or [`HOLE_CELLS_FLOOR`], whichever is more; past that (a
/// short row beside rows of hundreds of thousands of columns) each run of
/// holes gets cells [`tables::MAX_COLSPAN`] columns wide, so the cost stays
/// in proportion to the document. The floor keeps every table of ordinary
/// size on the old rule: without it a title cell of `colspan = 10` over rows
/// of 2 cells made a new row of `[1, 1, colspan 8]`.
///
/// The new cells are plain `table_cell`s, as before, whatever `like` holds.
/// Walks the grid row once: the map it reads is already that size.
fn add_row(info: &TableRect, row: usize, like: usize, schema: &Schema) -> Option<Vec<BatchEdit>> {
    let map = &info.map;
    let width = map.width();
    let slots = map.map();
    let row_pos = map.row_start(row)?;
    let like = like.min(map.height().saturating_sub(1));
    let like_slot = |col: usize| slots[like * width + col];
    // Two holes one above the other are no cell spanning the line.
    let from_above = |col: usize| {
        let index = row * width + col;
        row > 0
            && row < map.height()
            && slots[index] != HOLE
            && slots[index] == slots[index - width]
    };
    // Holes on the free columns: one cell each unless that outnumbers both
    // the table's cells and the floor.
    let holes = (0..width)
        .filter(|&col| like_slot(col) == HOLE && !from_above(col))
        .count();
    let table_cells: usize = (0..info.table.child_count())
        .map(|r| info.table.child(r).child_count())
        .sum();
    let hole_run_cap = if holes <= table_cells.max(HOLE_CELLS_FLOOR) {
        1
    } else {
        usize::try_from(tables::MAX_COLSPAN).unwrap_or(1)
    };
    let mut edits = Vec::new();
    let mut cells: Vec<Node> = Vec::new();
    let mut col = 0;
    while col < width {
        if from_above(col) {
            let pos = slots[row * width + col];
            let rect = map.cell_rect(row * width + col)?;
            edits.push(BatchEdit::set_attr(
                pos,
                "rowspan",
                span_value(rect.bottom - rect.top + 1),
            ));
            col = rect.right.max(col + 1);
            continue;
        }
        // The free run's piece under one cell of `like` (or one run of
        // holes, cut at `hole_run_cap`).
        let owner = like_slot(col);
        let cap = if owner == HOLE { hole_run_cap } else { width };
        let mut end = col + 1;
        while end < width && end - col < cap && like_slot(end) == owner && !from_above(end) {
            end += 1;
        }
        cells.push(make_empty_cell_spanning(schema, end - col)?);
        col = end;
    }
    let new_row = schema
        .create_node("table_row", Attrs::new(), Fragment::from_children(cells))
        .ok()?;
    edits.push(BatchEdit::insert(row_pos, Fragment::from_node(new_row)));
    Some(edits)
}

/// The most holes [`add_row`] always fills with one cell per column, however
/// few cells the table has (see there).
const HOLE_CELLS_FLOOR: usize = 1000;

/// An empty cell `cols` columns wide: [`make_empty_cell`] for one column (the
/// very cell `addRow` always made), else the same built with that `colspan`.
fn make_empty_cell_spanning(schema: &Schema, cols: usize) -> Option<Node> {
    if cols <= 1 {
        return make_empty_cell(schema);
    }
    let para = schema
        .create_node("paragraph", Attrs::new(), Fragment::empty())
        .ok()?;
    let attrs = Attrs::from_iter([("colspan", span_value(cols))]);
    schema
        .create_node("table_cell", attrs, Fragment::from_node(para))
        .ok()
}

/// Remove the grid rows `top..bottom` (not all of them). A cell spanning into
/// them from above loses the rows it had there; a cell starting in them and
/// continuing below is re-created in row `bottom` with the rows it has left;
/// the rest go with their rows. Ports `removeRow`, which ProseMirror's
/// `deleteRow` runs once per row from the bottom: a row's removal moves a
/// cell that continues below it into the row below, which is row `bottom`
/// once the rows between are gone, and a copy lands before the first cell
/// of that row to its right — in column order, whichever row it came from.
fn remove_rows(
    info: &TableRect,
    cells: &Cells,
    top: usize,
    bottom: usize,
    schema: &Schema,
) -> Option<Vec<BatchEdit>> {
    let map = &info.map;
    let width = map.width();
    let mut edits = vec![BatchEdit::delete(
        map.row_start(top)?,
        map.row_start(bottom)?,
    )];
    // Each cell once, with its rectangle, in the order the walk meets them:
    // row `bottom - 1` first, left to right. A cell that continues below
    // `bottom` covers that row, so the cells to move come in column order,
    // which is the order their copies land in.
    let mut found: HashMap<usize, Rect> = HashMap::new();
    let mut order = Vec::new();
    for row in (top..bottom).rev() {
        let mut col = 0;
        while col < width {
            let index = row * width + col;
            let pos = map.map()[index];
            // A cell met in a row below: its rectangle is known. Asking
            // `cell_rect` again walks up to the cell's top, once per row it
            // covers (#1874).
            if let Some(rect) = found.get(&pos) {
                col = rect.right.max(col + 1);
                continue;
            }
            // A hole: this row has nothing in the column.
            let Some(rect) = map.cell_rect(index) else {
                col += 1;
                continue;
            };
            order.push(pos);
            found.insert(pos, rect);
            col = rect.right.max(col + 1);
        }
    }
    for pos in order {
        let rect = found[&pos];
        if rect.top < top {
            // Spans into the rows from above → it loses those rows.
            let lost = rect.bottom.min(bottom) - top;
            let height = (rect.bottom - rect.top).saturating_sub(lost);
            edits.push(BatchEdit::set_attr(pos, "rowspan", span_value(height)));
        } else if bottom < map.height() && rect.bottom > bottom {
            // Starts in them and continues below → re-create it in row
            // `bottom`, with the rows it has there.
            let cell = cells.node(info, pos)?;
            let new_attrs = cell
                .attrs()
                .with("rowspan", span_value(rect.bottom - bottom));
            let copy = schema
                .create_node(cell.type_name(), new_attrs, cell.content().clone())
                .ok()?;
            edits.push(BatchEdit::insert(
                map.position_at(bottom, rect.left),
                Fragment::from_node(copy),
            ));
        }
    }
    Some(edits)
}

/// Whether the grid is the table's cells as they say they are: every cell has
/// slots, they form a rectangle, and its spans are that rectangle's, cut only
/// at the grid's edges. Holes are allowed; cells that overlap (a span the map
/// resolved by giving the slots to the cell that claimed them first) and cells
/// in no slot are not. Only on such a grid do the one-pass [`remove_columns`]
/// and [`remove_rows`] do what ProseMirror's one-at-a-time removal does,
/// recomputing the map in between: overlapping cells re-resolve differently
/// once a column or row is gone.
fn plain_grid(info: &TableRect, cells: &Cells) -> bool {
    let map = &info.map;
    let (width, height) = (map.width(), map.height());
    let m = map.map();
    // Each cell's first slot in row-major order, and its slot count.
    let mut slots: HashMap<usize, (usize, usize)> = HashMap::new();
    for (index, &pos) in m.iter().enumerate() {
        if pos != HOLE {
            slots.entry(pos).or_insert((index, 0)).1 += 1;
        }
    }
    if slots.len() != cells.starts.len() {
        return false;
    }
    cells.starts.iter().all(|pos| {
        let Some(&(index, count)) = slots.get(pos) else {
            return false;
        };
        let Some(rect) = map.cell_rect(index) else {
            return false;
        };
        let cell = cells.node(info, *pos).expect("a cell of the table");
        let span = |name: &str| usize::try_from(cell.attrs().get_int(name).unwrap_or(1).max(1));
        let (cols, rows) = (rect.right - rect.left, rect.bottom - rect.top);
        // `cols * rows == count` is defensive: no grid the differential in
        // `table_commands_one_step.rs` has met fails it alone.
        (rect.top, rect.left) == (index / width, index % width)
            && cols * rows == count
            && span("colspan").map_or(cols == width - rect.left, |c| {
                c.min(width - rect.left) == cols
            })
            && span("rowspan").map_or(rows == height - rect.top, |r| {
                r.min(height - rect.top) == rows
            })
    })
}

/// Whether grid row `row` has a slot no cell covers (`false` past the last
/// row). A cell [`remove_rows`] moves down lands before the first cell of row
/// `bottom` right of it, which puts the moved cells in column order — unless
/// that row has a hole, where `positionAt` answers the row's end and the
/// cells moved one row at a time land in the order they were moved.
fn has_hole(map: &TableMap, row: usize) -> bool {
    let width = map.width();
    row < map.height() && map.map()[row * width..(row + 1) * width].contains(&HOLE)
}

/// Remove grid column `col`, as `removeColumn` does, for a grid
/// [`plain_grid`] does not accept: [`delete_column`] runs it once per
/// column, on a map recomputed in between.
fn remove_column_on(info: &TableRect, cells: &Cells, col: usize) -> Option<Vec<BatchEdit>> {
    let map = &info.map;
    let width = map.width();
    let mut edits = Vec::new();
    let mut row = 0;
    while row < map.height() {
        let index = row * width + col;
        let pos = map.map()[index];
        // A hole: this row has nothing in the column.
        let Some(rect) = map.cell_rect(index) else {
            row += 1;
            continue;
        };
        let cell = cells.node(info, pos)?;
        let spans_more = (col > 0 && map.map()[index - 1] == pos)
            || (col < width - 1 && map.map()[index + 1] == pos);
        if spans_more {
            edits.push(BatchEdit::set_attr(
                pos,
                "colspan",
                span_value(rect.right - rect.left - 1),
            ));
        } else {
            edits.push(BatchEdit::delete(pos, pos + cell.node_size()));
        }
        row = rect.bottom.max(row + 1);
    }
    Some(edits)
}

/// Remove grid row `row`, as `removeRow` does, for a grid [`plain_grid`]
/// does not accept (see [`remove_column_on`]).
fn remove_row_on(
    info: &TableRect,
    cells: &Cells,
    row: usize,
    schema: &Schema,
) -> Option<Vec<BatchEdit>> {
    let map = &info.map;
    let width = map.width();
    let row_pos = map.row_start(row)?;
    let next_row = map.row_start(row + 1)?;
    let mut edits = vec![BatchEdit::delete(row_pos, next_row)];
    let mut seen: HashSet<usize> = HashSet::new();
    let mut col = 0;
    while col < width {
        let index = row * width + col;
        let pos = map.map()[index];
        if seen.contains(&pos) {
            col += 1;
            continue;
        }
        // A hole: this row has nothing in the column.
        let Some(rect) = map.cell_rect(index) else {
            col += 1;
            continue;
        };
        seen.insert(pos);
        let cell = cells.node(info, pos)?;
        let next_col = rect.right.max(col + 1);
        let shorter = span_value(rect.bottom - rect.top - 1);
        if row > 0 && pos == map.map()[index - width] {
            // Spans into this row from above → reduce its rowspan.
            edits.push(BatchEdit::set_attr(pos, "rowspan", shorter));
        } else if row + 1 < map.height() && pos == map.map()[index + width] {
            // Starts here and continues below → recreate it one row down.
            let new_attrs = cell.attrs().with("rowspan", shorter);
            let copy = schema
                .create_node(cell.type_name(), new_attrs, cell.content().clone())
                .ok()?;
            edits.push(BatchEdit::insert(
                map.position_at(row + 1, col),
                Fragment::from_node(copy),
            ));
        }
        col = next_col;
    }
    Some(edits)
}

/// The most grid slots the one-line-at-a-time fallbacks of [`delete_row`] and
/// [`delete_column`] may map in all: they recompute the whole map after each
/// row or column, so their work is the lines removed times the map's slots.
/// Past this the command removes the lines in one pass over the map it
/// already has (#1874), as it does on a [`plain_grid`].
///
/// The fallback exists for grids whose cells overlap (and cells in no slot,
/// past #1176's slot budget), where ProseMirror's one-at-a-time removal can
/// re-resolve the cells differently between lines. On a table past the slot
/// budget the map is 2^22 slots and a recompute about 10 ms in a release
/// build: a 1000-column cell took 10 s to delete, and a whole row of a
/// 2,097,152-column table hours. 2^22 slots in all is one such map, or a few
/// milliseconds; every table small enough for the difference to be seen by
/// hand keeps the exact one-at-a-time result.
const FALLBACK_SLOT_BUDGET: usize = 1 << 22;

thread_local! {
    /// A test's replacement for [`FALLBACK_SLOT_BUDGET`] on this thread.
    static FALLBACK_SLOT_BUDGET_OVERRIDE: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

/// Replace [`FALLBACK_SLOT_BUDGET`] on this thread (`None` restores it), so
/// a test can force the one-pass removal onto small tables whose cells
/// overlap — the path otherwise reached only on tables of millions of
/// slots. Not for apps.
#[doc(hidden)]
pub fn set_fallback_slot_budget_for_tests(budget: Option<usize>) {
    FALLBACK_SLOT_BUDGET_OVERRIDE.with(|b| b.set(budget));
}

/// Whether removing `lines` rows or columns one at a time, recomputing `map`
/// after each, stays within [`FALLBACK_SLOT_BUDGET`].
fn one_at_a_time_fits(map: &TableMap, lines: usize) -> bool {
    let budget = FALLBACK_SLOT_BUDGET_OVERRIDE
        .with(std::cell::Cell::get)
        .unwrap_or(FALLBACK_SLOT_BUDGET);
    // The last line needs no recompute after it.
    lines.saturating_sub(1).saturating_mul(map.map().len()) <= budget
}

/// Recompute the table rectangle (map + table node) from the transaction's current
/// doc — used between successive row/column removals (which invalidate the map).
fn recompute(tr: &Transaction, table_start: usize, rect: Rect) -> Option<TableRect> {
    let table = tr.doc().node_at(table_start - 1)?;
    if !tables::is_table(&table) {
        return None;
    }
    let map = TableMap::compute(&table, table_start);
    Some(TableRect {
        map,
        table,
        table_start,
        rect,
    })
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

// ===================================================================
// Merge / split
// ===================================================================

/// True if `cell` holds a single empty textblock (one paragraph with no content).
fn cell_is_empty(cell: &Node) -> bool {
    cell.child_count() == 1 && {
        let only = cell.content().child(0);
        only.is_textblock() && only.content().size() == 0
    }
}

/// True if any selected cell pokes out of `rect` on an edge — merging would create
/// an L-shaped (non-rectangular) cell, which is disallowed. Port of
/// `cellsOverlapRectangle`.
fn cells_overlap_rectangle(map: &TableMap, rect: Rect) -> bool {
    let width = map.width();
    let height = map.height();
    let m = map.map();
    // Left/right edges.
    for r in rect.top..rect.bottom {
        let il = r * width + rect.left;
        let ir = r * width + (rect.right - 1);
        // Two holes side by side are no cell bleeding across the edge.
        let bleed_left = rect.left > 0 && m[il] != HOLE && m[il] == m[il - 1];
        let bleed_right = rect.right < width && m[ir] != HOLE && m[ir] == m[ir + 1];
        if bleed_left || bleed_right {
            return true;
        }
    }
    // Top/bottom edges.
    for c in rect.left..rect.right {
        let it = rect.top * width + c;
        let ib = (rect.bottom - 1) * width + c;
        let bleed_top = rect.top > 0 && m[it] != HOLE && m[it] == m[it - width];
        let bleed_bottom = rect.bottom < height && m[ib] != HOLE && m[ib] == m[ib + width];
        if bleed_top || bleed_bottom {
            return true;
        }
    }
    false
}

/// Clear every cell in the current cell selection — replace each selected cell's
/// content with a single empty paragraph and collapse the cursor to the start of the
/// top-left cell (ProseMirror `deleteCellSelection`). The edit path routes Backspace/
/// Delete/typing over a cell selection here so they blank the cells instead of
/// splicing the coarse `from()..to()` range (which would collapse the table).
/// `None` unless a cell selection is active.
pub fn clear_cells(state: &EditorState) -> Option<Transaction> {
    if !matches!(state.selection, Selection::Cell(_)) {
        return None;
    }
    let info = selected_rect(state)?;
    let schema = state.schema();
    let cells = Cells::new(&info);
    let mut edits = Vec::new();
    for cell_pos in info.map.cells_in_rect(info.rect) {
        let Some(cell) = cells.node(&info, cell_pos) else {
            continue;
        };
        if cell_is_empty(cell) {
            continue;
        }
        let para = schema
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .ok()?;
        let from = cell_pos + 1;
        let to = from + cell.content_size();
        edits.push(BatchEdit::replace(from, to, Fragment::from_node(para)));
    }
    let mut tr = state.tr();
    tr.batch(edits).ok()?;
    // Collapse into the top-left cell (its position is unchanged — every edit
    // is inside a cell, and none is before it).
    let top_left = info.map.cell_at(info.rect.top, info.rect.left)?;
    let sel = Selection::near(tr.doc(), Pos(top_left + 1), 1);
    tr.set_selection(sel);
    Some(tr)
}

/// The current cell selection's cells as a table of their own: what copying a
/// cell selection puts on the clipboard. A merged cell that pokes out of the
/// rectangle keeps only its span inside it. `None` unless a cell selection is
/// active. Port of prosemirror-tables' `CellSelection.content`, as a whole
/// table rather than its rows.
pub fn cell_selection_slice(state: &EditorState) -> Option<Slice> {
    if !matches!(state.selection, Selection::Cell(_)) {
        return None;
    }
    let info = selected_rect(state)?;
    let (map, rect) = (&info.map, info.rect);
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for row in rect.top..rect.bottom {
        let mut cells = Vec::new();
        for col in rect.left..rect.right {
            let index = row * map.width() + col;
            let Some(cell_rect) = map.cell_rect(index) else {
                continue;
            };
            let pos = map.map()[index];
            if !seen.insert(pos) {
                continue;
            }
            let Some(cell) = info.table.node_at(pos - info.table_start) else {
                continue;
            };
            let cols = cell_rect.right.min(rect.right) - cell_rect.left.max(rect.left);
            let rows_in = cell_rect.bottom.min(rect.bottom) - cell_rect.top.max(rect.top);
            let mut attrs = cell.attrs().clone();
            if cols != cell_rect.right - cell_rect.left {
                attrs = attrs.with("colspan", span_value(cols));
            }
            if rows_in != cell_rect.bottom - cell_rect.top {
                attrs = attrs.with("rowspan", span_value(rows_in));
            }
            cells.push(cell.with_attrs(attrs));
        }
        rows.push(
            info.table
                .child(row)
                .copy_with_content(Fragment::from_children(cells)),
        );
    }
    let table = info.table.copy_with_content(Fragment::from_children(rows));
    Some(Slice::from_fragment(Fragment::from_node(table)))
}

/// `deleteCellSelection` — clear the selected cells' content (see [`clear_cells`]).
pub fn delete_cell_selection() -> Command {
    command_tr(clear_cells)
}

/// `mergeCells` — collapse a rectangular [`Selection::Cell`] into its top-left cell,
/// pulling the other cells' content in and growing the master's colspan/rowspan to
/// cover the rectangle. No-op unless a multi-cell rectangle is selected and it is
/// rectangular (no cell straddles an edge). Port of `mergeCells`.
pub fn merge_cells() -> Command {
    command_tr(|state| {
        let Selection::Cell(c) = &state.selection else {
            return None;
        };
        if c.anchor_cell == c.head_cell {
            return None;
        }
        let info = selected_rect(state)?;
        if cells_overlap_rectangle(&info.map, info.rect) {
            return None;
        }
        let map = &info.map;
        let width = map.width();
        let rect = info.rect;
        let cells = Cells::new(&info);
        // The master is the rectangle's top-left cell, which grows over the rest,
        // holes included: they are no cell to delete. The top-left slot is never
        // a hole: a row's own cells all lie left of its holes (a hole can be
        // followed by a slot a rowspan from above covers, never by one of the
        // row's own cells), and the rectangle's top row holds a selected cell at
        // or right of its left edge. Every other cell comes after it in the
        // document.
        let mut seen: HashSet<usize> = HashSet::new();
        let mut edits = Vec::new();
        let mut content = Fragment::empty();
        let mut master: Option<(usize, &Node)> = None;
        for row in rect.top..rect.bottom {
            for col in rect.left..rect.right {
                let pos = map.map()[row * width + col];
                if pos == HOLE || !seen.insert(pos) {
                    continue;
                }
                let cell = cells.node(&info, pos)?;
                match &master {
                    None => master = Some((pos, cell)),
                    Some(_) => {
                        if !cell_is_empty(cell) {
                            content = content.append(cell.content());
                        }
                        edits.push(BatchEdit::delete(pos, pos + cell.node_size()));
                    }
                }
            }
        }
        let (master_pos, master_cell) = master?;
        // Grow the master to cover the rectangle.
        edits.push(BatchEdit::set_attr(
            master_pos,
            "colspan",
            AttrValue::Int((rect.right - rect.left) as i64),
        ));
        edits.push(BatchEdit::set_attr(
            master_pos,
            "rowspan",
            AttrValue::Int((rect.bottom - rect.top) as i64),
        ));
        if content.size() > 0 {
            let content_end = master_pos + 1 + master_cell.content_size();
            let start = if cell_is_empty(master_cell) {
                master_pos + 1
            } else {
                content_end
            };
            edits.push(BatchEdit::replace(start, content_end, content));
        }
        let mut tr = state.tr();
        tr.batch(edits).ok()?;
        tr.set_selection(Selection::cell(Pos(master_pos), Pos(master_pos)));
        Some(tr)
    })
}

/// `splitCell` — split the merged cell under the cursor (or a single selected cell)
/// back into 1×1 cells, filling the freed grid slots with empty cells. No-op on a
/// 1×1 cell or a multi-cell selection. Port of `splitCell` (plain-cell type).
///
/// **Cost**: one step (#1200), holding one insert per row the cell spans (#1185
/// made it one step per row, where ProseMirror inserts a cell per slot). The
/// cells it creates are as many as the slots it vacates, and there is no cap
/// (PM has none): the slots come from the [`TableMap`], which holds at most
/// [`tables::grid_slot_budget`] of them, so a split creates at most
/// `grid_slot_budget` cells: 2^22 (about 4.2 M) for a table of fewer than
/// 2^21 cells, twice its cell count beyond that. Measured in a release build:
/// a 62 × 16,000 cell is 1.0 M cells in 0.29 s and 1.06 GB peak (8.1 s and
/// 2.4 GB as a step per row).
pub fn split_cell() -> Command {
    command_tr(|state| {
        // The single target cell: a 1-cell cell selection, or the cell at the cursor.
        let (cell_pos, info) = match &state.selection {
            Selection::Cell(c) if c.anchor_cell == c.head_cell => {
                (c.anchor_cell.0, selected_rect(state)?)
            }
            Selection::Cell(_) => return None,
            _ => {
                let r = state.doc.resolve(state.selection.head()).ok()?;
                (tables::cell_around(&r)?, selected_rect(state)?)
            }
        };
        let cell = info.table.node_at(cell_pos - info.table_start)?;
        if colspan(&cell) == 1 && rowspan(&cell) == 1 {
            return None;
        }
        let rect = info.map.find_cell(cell_pos)?;
        let cell_size = cell.node_size();
        let schema = state.schema();
        // Reset the master to 1×1, and fill the vacated grid slots with empty
        // cells: one insert per row holding all of that row's new cells.
        let mut edits = vec![
            BatchEdit::set_attr(cell_pos, "colspan", AttrValue::Int(1)),
            BatchEdit::set_attr(cell_pos, "rowspan", AttrValue::Int(1)),
        ];
        let width = rect.right - rect.left;
        for row in rect.top..rect.bottom {
            let mut at = info.map.position_at(row, rect.left);
            let mut count = width;
            if row == rect.top {
                at += cell_size; // place new cells just after the master
                count -= 1; // the master itself stays
            }
            if count == 0 {
                continue;
            }
            let cells = (0..count)
                .map(|_| make_empty_cell(schema))
                .collect::<Option<Vec<_>>>()?;
            edits.push(BatchEdit::insert(at, Fragment::from_children(cells)));
        }
        let mut tr = state.tr();
        tr.batch(edits).ok()?;
        // Collapse the selection into the (now 1×1) master cell.
        let sel = Selection::near(tr.doc(), Pos(cell_pos + 1), 1);
        tr.set_selection(sel);
        tr.doc_changed().then_some(tr)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_table;
    use crate::model::Fragment;
    use crate::state::EditorState;

    fn sk() -> Schema {
        Schema::starter_kit()
    }

    /// A fresh state whose document is a single `rows×cols` table, cursor inside the
    /// top-left cell.
    fn table_state(rows: usize, cols: usize) -> EditorState {
        let s = sk();
        let table = build_table(&s, rows, cols).unwrap();
        // A trailing empty paragraph keeps the doc valid (`block+`) when a command
        // deletes the whole table — the realistic shape (an editor always has a
        // landing block after a table).
        let tail = s
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .unwrap();
        let doc = s
            .branch("doc", Fragment::from_children(vec![table, tail]))
            .unwrap();
        let plugins: Vec<std::rc::Rc<dyn crate::plugin::Plugin>> =
            vec![std::rc::Rc::new(crate::commands::BaseCommandsPlugin)];
        let mut st = EditorState::create(std::rc::Rc::new(s), doc, plugins);
        // cursor inside the first cell's paragraph (table 1, row 2, cell 3, p 4)
        st.selection = Selection::cursor(Pos(4));
        st
    }

    fn table_of(state: &EditorState) -> Node {
        state
            .doc
            .content()
            .children()
            .iter()
            .find(|n| tables::is_table(n))
            .cloned()
            .expect("a table")
    }

    fn dims(state: &EditorState) -> (usize, usize) {
        let table = table_of(state);
        let map = TableMap::compute(&table, 1);
        (map.width(), map.height())
    }

    #[test]
    fn add_row_after_grows_height() {
        let state = table_state(2, 3);
        let next = state.run("addRowAfter").expect("applies");
        assert_eq!(dims(&next), (3, 3), "one more row");
        // The table is still well-formed (every row has 3 cells).
        let table = table_of(&next);
        for r in 0..3 {
            assert_eq!(table.content().child(r).content().child_count(), 3);
        }
    }

    #[test]
    fn add_column_before_grows_width() {
        let state = table_state(2, 2);
        let next = state.run("addColumnBefore").expect("applies");
        assert_eq!(dims(&next), (3, 2), "one more column");
    }

    #[test]
    fn delete_row_shrinks_and_undoes() {
        let state = table_state(3, 2);
        let next = state.run("deleteRow").expect("applies");
        assert_eq!(dims(&next), (2, 2), "one fewer row");
    }

    #[test]
    fn delete_column_shrinks() {
        let state = table_state(2, 3);
        let next = state.run("deleteColumn").expect("applies");
        assert_eq!(dims(&next), (2, 2), "one fewer column");
    }

    #[test]
    fn delete_last_row_deletes_table() {
        // A 1-row table: deleteRow would empty it → the whole table goes.
        let state = table_state(1, 2);
        // Select all cells so the rect spans the only row.
        let next = state.run("deleteRow").expect("applies");
        assert!(
            next.doc
                .content()
                .children()
                .iter()
                .all(|n| !tables::is_table(n)),
            "table removed entirely"
        );
    }

    #[test]
    fn delete_over_cell_selection_blanks_not_collapses() {
        // A 2x2 table with text in each cell. Selecting all cells and pressing
        // Backspace must BLANK the cells, not splice the coarse range (which the old
        // path did, collapsing the table to 1x1).
        let s = sk();
        let text_cell = |t: &str| {
            let p = s
                .branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap();
            s.create_node("table_cell", Attrs::new(), Fragment::from_node(p))
                .unwrap()
        };
        let row = |a: &str, b: &str| {
            s.create_node(
                "table_row",
                Attrs::new(),
                Fragment::from_children(vec![text_cell(a), text_cell(b)]),
            )
            .unwrap()
        };
        let table = s
            .create_node(
                "table",
                Attrs::new(),
                Fragment::from_children(vec![row("a", "b"), row("c", "d")]),
            )
            .unwrap();
        let tail = s
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .unwrap();
        let doc = s
            .branch("doc", Fragment::from_children(vec![table.clone(), tail]))
            .unwrap();
        let plugins: Vec<std::rc::Rc<dyn crate::plugin::Plugin>> =
            vec![std::rc::Rc::new(crate::commands::BaseCommandsPlugin)];
        let mut state = EditorState::create(std::rc::Rc::new(s), doc, plugins);
        let map = TableMap::compute(&table, 1);
        let c00 = map.cell_at(0, 0).unwrap();
        let c11 = map.cell_at(1, 1).unwrap();
        state.selection = Selection::cell(Pos(c00), Pos(c11));

        let next = state.run("deleteCharBackward").expect("applies");
        // The table survives at full dimensions (the critical regression).
        let ntable = table_of(&next);
        let nmap = TableMap::compute(&ntable, 1);
        assert_eq!((nmap.width(), nmap.height()), (2, 2), "table preserved");
        // Every cell is now empty (its text was blanked).
        for r in 0..2 {
            for c in 0..2 {
                let cell = ntable.content().child(r).content().child(c);
                assert_eq!(cell.content().child(0).content().size(), 0, "cell blanked");
            }
        }
        // The selection collapsed to a text cursor (in the top-left cell).
        assert!(matches!(next.selection, Selection::Text(_)));
    }

    #[test]
    fn merge_and_split_round_trip() {
        // 2×2 table; select the whole grid and merge → one 2×2 cell.
        let mut state = table_state(2, 2);
        let table = table_of(&state);
        let map = TableMap::compute(&table, 1);
        let c00 = map.cell_at(0, 0).unwrap();
        let c11 = map.cell_at(1, 1).unwrap();
        state.selection = Selection::cell(Pos(c00), Pos(c11));
        let merged = state.run("mergeCells").expect("merge applies");
        let mtable = table_of(&merged);
        let mmap = TableMap::compute(&mtable, 1);
        assert_eq!(mmap.width(), 2);
        assert_eq!(mmap.height(), 2);
        // Only one distinct cell remains in the grid.
        let master = mmap.cell_at(0, 0).unwrap();
        let distinct: std::collections::BTreeSet<usize> = mmap.map().iter().copied().collect();
        assert_eq!(distinct.len(), 1, "all slots point at the merged cell");
        let cell = mtable.node_at(master - 1).unwrap();
        assert_eq!(colspan(&cell), 2);
        assert_eq!(rowspan(&cell), 2);

        // Now split it back.
        let mut split_state = merged.clone();
        split_state.selection = Selection::cursor(Pos(master + 1));
        let split = split_state.run("splitCell").expect("split applies");
        let stable = table_of(&split);
        let smap = TableMap::compute(&stable, 1);
        let distinct2: std::collections::BTreeSet<usize> = smap.map().iter().copied().collect();
        assert_eq!(distinct2.len(), 4, "back to four distinct cells");
    }
}
