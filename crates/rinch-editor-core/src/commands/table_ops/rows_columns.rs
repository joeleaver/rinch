//! Row and column insertion and removal: the grid edits behind the
//! registered row/column commands in the parent module.

use super::*;

// Every command here (and the registered ones in the parent module) states
// its edits in the coordinates of the document it starts from and applies
// them as one [`crate::transform::BatchStep`] (#1200). ProseMirror's port
// took one step per row (a column insert) or per cell (a merge), each mapped
// through the ones before it; each of those steps rebuilt the table's row
// list and the transaction kept the document from before every one, which
// made a column into 16,000 rows 6.4 s and 2.1 GB. The batch maps a position
// exactly as those steps did, so a caret in a cell the command did not touch
// stays where it was.

// ===================================================================
// Column operations
// ===================================================================

/// Insert a column at grid index `col` (`0..=width`). Where an existing cell spans
/// across the insertion line its colspan grows; otherwise an empty cell is spliced
/// into each row. Port of `addColumn`.
///
/// Each new cell is modelled on the cell beside it in its row, in PM's
/// reference column: the one on the left, or at the table's left edge the one
/// it pushes right; when that is a header column, the one on the other side,
/// or none at either edge (then the cells are plain `table_cell`s). The new
/// cell takes that cell's type, so a header row gets a header cell, and its
/// alignment (#2210).
pub(super) fn add_column(info: &TableRect, col: usize, schema: &Schema) -> Option<Vec<BatchEdit>> {
    let map = &info.map;
    let width = map.width();
    let cells = Cells::new(info);
    let mut ref_col = Some(col.saturating_sub(1)).filter(|&c| c < width);
    if ref_col.is_some_and(|c| column_is_header(info, &cells, c)) {
        ref_col = (col > 0 && col < width).then_some(col);
    }
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
            let like = ref_col.and_then(|c| cells.node(info, map.map()[row * width + c]));
            let ty = like.map_or("table_cell", |cell| cell.type_name());
            edits.push(BatchEdit::insert(
                at,
                Fragment::from_node(make_empty_cell(schema, ty, 1, like)?),
            ));
            row += 1;
        }
    }
    Some(edits)
}

/// Whether every slot of grid column `col` is a `table_header_cell` (PM
/// `columnIsHeader`; a hole is not one).
fn column_is_header(info: &TableRect, cells: &Cells, col: usize) -> bool {
    let width = info.map.width();
    (0..info.map.height()).all(|row| {
        cells
            .node(info, info.map.map()[row * width + col])
            .is_some_and(|cell| cell.type_name() == "table_header_cell")
    })
}

/// Remove the grid columns `left..right` (`right <= width`). A cell that spans
/// columns outside them shrinks by the columns it loses; a cell wholly inside
/// them is deleted. Ports `removeColumn`, which ProseMirror's `deleteColumn`
/// runs once per column from the right, recomputing the map in between: a
/// column's removal leaves the columns to its left where they were, so every
/// column's cells can be read off the one map.
pub(super) fn remove_columns(
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
/// The new cells are plain `table_cell`s, as before, whatever `like` holds;
/// each takes the alignment of the cell of `like` it is cut under (#2210),
/// and a run of holes none.
/// Walks the grid row once: the map it reads is already that size.
pub(super) fn add_row(
    info: &TableRect,
    row: usize,
    like: usize,
    schema: &Schema,
) -> Option<Vec<BatchEdit>> {
    let map = &info.map;
    let cell_nodes = Cells::new(info);
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
        let like_cell = cell_nodes.node(info, owner);
        cells.push(make_empty_cell(schema, "table_cell", end - col, like_cell)?);
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

/// Remove the grid rows `top..bottom` (not all of them). A cell spanning into
/// them from above loses the rows it had there; a cell starting in them and
/// continuing below is re-created in row `bottom` with the rows it has left;
/// the rest go with their rows. Ports `removeRow`, which ProseMirror's
/// `deleteRow` runs once per row from the bottom: a row's removal moves a
/// cell that continues below it into the row below, which is row `bottom`
/// once the rows between are gone, and a copy lands before the first cell
/// of that row to its right — in column order, whichever row it came from.
pub(super) fn remove_rows(
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
pub(super) fn plain_grid(info: &TableRect, cells: &Cells) -> bool {
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
pub(super) fn has_hole(map: &TableMap, row: usize) -> bool {
    let width = map.width();
    row < map.height() && map.map()[row * width..(row + 1) * width].contains(&HOLE)
}

/// Remove grid column `col`, as `removeColumn` does, for a grid
/// [`plain_grid`] does not accept: [`delete_column`] runs it once per
/// column, on a map recomputed in between.
pub(super) fn remove_column_on(
    info: &TableRect,
    cells: &Cells,
    col: usize,
) -> Option<Vec<BatchEdit>> {
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
pub(super) fn remove_row_on(
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
pub(super) fn one_at_a_time_fits(map: &TableMap, lines: usize) -> bool {
    let budget = FALLBACK_SLOT_BUDGET_OVERRIDE
        .with(std::cell::Cell::get)
        .unwrap_or(FALLBACK_SLOT_BUDGET);
    // The last line needs no recompute after it.
    lines.saturating_sub(1).saturating_mul(map.map().len()) <= budget
}

/// Recompute the table rectangle (map + table node) from the transaction's current
/// doc — used between successive row/column removals (which invalidate the map).
pub(super) fn recompute(tr: &Transaction, table_start: usize, rect: Rect) -> Option<TableRect> {
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
