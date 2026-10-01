//! #1200: every table command is one transaction step, whatever the table's
//! height. One step per row (`addColumn*`, `deleteColumn`, `splitCell`, and a
//! step per cell for `mergeCells` and `deleteCellSelection`) rebuilt the
//! table's row list and kept a copy of it per step, and mapped each position
//! through every step before it: quadratic in time and memory
//! (`addColumnBefore` on 16,000 rows: 6.4 s and 2.1 GB).
//!
//! Timing-free. What is pinned is the step count, and — against the
//! commands as they were before, kept here verbatim as `reference` — the
//! document, the selection and the position mapping they produce, which must
//! not move: a caret in a cell the command did not touch stays where it was.

use rinch_editor_core::commands::table_ops::{TableRect, selected_rect};
use rinch_editor_core::state::Transaction;
use rinch_editor_core::tables::{self, Rect, TableMap};
use rinch_editor_core::*;
use std::rc::Rc;

/// The commands as they were before #1200 — one step per row or per cell —
/// copied from `src/commands/table_ops.rs` with the registration stripped.
#[allow(dead_code, clippy::all)]
mod reference {
    use super::*;

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

    /// `pos` mapped through the steps `tr` took since it had `start` of them — the
    /// steps of one row or column removal. A removal after the first works on a map
    /// recomputed from the transaction's current document, whose positions already
    /// reflect the earlier removals; mapping them through the whole transaction
    /// moved them a second time (PM maps through `tr.mapping.slice(mapStart)`).
    fn map_since(tr: &Transaction, start: usize, pos: usize) -> usize {
        tr.mapping().maps()[start..]
            .iter()
            .fold(pos, |pos, map| map.map(pos, 1))
    }

    /// The cell node at `pos` (an absolute cell position from the map). `None` for a
    /// hole or a position outside the table.
    fn cell_node(info: &TableRect, pos: usize) -> Option<Node> {
        let offset = pos.checked_sub(info.table_start)?;
        if pos == HOLE || offset >= info.table.content_size() {
            return None;
        }
        info.table.node_at(offset)
    }

    // ===================================================================
    // Column operations
    // ===================================================================

    /// Insert a column at grid index `col` (`0..=width`). Where an existing cell spans
    /// across the insertion line its colspan grows; otherwise an empty cell is spliced
    /// into each row. Port of `addColumn`. Multiple inserts shift later positions, so
    /// every target position is mapped through the transaction's accumulated mapping.
    fn add_column(
        tr: &mut Transaction,
        info: &TableRect,
        col: usize,
        schema: &Schema,
    ) -> Option<()> {
        let map = &info.map;
        let width = map.width();
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
                let mapped = tr.mapping().map(pos, 1);
                tr.set_node_attr(mapped, "colspan", span_value(rect.right - rect.left + 1))
                    .ok()?;
                row = rect.bottom.max(row + 1);
            } else {
                let at = map.position_at(row, col);
                let mapped = tr.mapping().map(at, 1);
                let cell = make_empty_cell(schema)?;
                tr.replace_with(mapped, mapped, Fragment::from_node(cell))
                    .ok()?;
                row += 1;
            }
        }
        Some(())
    }

    /// Remove the column at grid index `col`. A cell that spans more than this column
    /// shrinks (colspan−1); a cell wholly in the column is deleted. Port of
    /// `removeColumn`.
    fn remove_column(tr: &mut Transaction, info: &TableRect, col: usize) -> Option<()> {
        let start = tr.mapping().len();
        let map = &info.map;
        let width = map.width();
        let mut row = 0;
        while row < map.height() {
            let index = row * width + col;
            let pos = map.map()[index];
            // A hole: this row has nothing in the column.
            let Some(rect) = map.cell_rect(index) else {
                row += 1;
                continue;
            };
            let cell = cell_node(info, pos)?;
            let spans_more = (col > 0 && map.map()[index - 1] == pos)
                || (col < width - 1 && map.map()[index + 1] == pos);
            let mapped = map_since(tr, start, pos);
            if spans_more {
                tr.set_node_attr(mapped, "colspan", span_value(rect.right - rect.left - 1))
                    .ok()?;
            } else {
                tr.delete(mapped, mapped + cell.node_size()).ok()?;
            }
            row = rect.bottom.max(row + 1);
        }
        Some(())
    }

    // ===================================================================
    // Row operations
    // ===================================================================

    /// Insert a row at grid index `row` (`0..=height`). A cell that spans across the
    /// insertion line grows its rowspan; otherwise the new row gets an empty cell in
    /// that column. Port of `addRow`. The single row insert is the only position-
    /// shifting step and comes last, so the rowspan edits need no mapping.
    fn add_row(tr: &mut Transaction, info: &TableRect, row: usize, schema: &Schema) -> Option<()> {
        let map = &info.map;
        let width = map.width();
        let row_pos = map.row_start(row)?;
        let mut cells: Vec<Node> = Vec::new();
        let mut col = 0;
        while col < width {
            let index = row * width + col;
            // Two holes one above the other are no cell spanning the line.
            let from_above = row > 0
                && row < map.height()
                && map.map()[index] != HOLE
                && map.map()[index] == map.map()[index - width];
            if from_above {
                let pos = map.map()[index];
                let rect = map.cell_rect(index)?;
                tr.set_node_attr(pos, "rowspan", span_value(rect.bottom - rect.top + 1))
                    .ok()?;
                col = rect.right.max(col + 1);
            } else {
                cells.push(make_empty_cell(schema)?);
                col += 1;
            }
        }
        let new_row = schema
            .create_node("table_row", Attrs::new(), Fragment::from_children(cells))
            .ok()?;
        tr.replace_with(row_pos, row_pos, Fragment::from_node(new_row))
            .ok()?;
        Some(())
    }

    /// Remove the row at grid index `row`. Cells spanning into the row from above lose
    /// a rowspan; a cell starting in the row but continuing below is re-created one row
    /// down with rowspan−1; plain cells go with the row. Port of `removeRow`.
    fn remove_row(
        tr: &mut Transaction,
        info: &TableRect,
        row: usize,
        schema: &Schema,
    ) -> Option<()> {
        let map = &info.map;
        let width = map.width();
        let row_pos = map.row_start(row)?;
        let next_row = map.row_start(row + 1)?;
        let start = tr.mapping().len();
        // Delete the whole row first (the one position-shifting step besides the
        // move-down inserts, which are mapped below).
        tr.delete(row_pos, next_row).ok()?;
        let mut seen: Vec<usize> = Vec::new();
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
            seen.push(pos);
            let cell = cell_node(info, pos)?;
            let next_col = rect.right.max(col + 1);
            let shorter = span_value(rect.bottom - rect.top - 1);
            if row > 0 && pos == map.map()[index - width] {
                // Spans into this row from above → reduce its rowspan.
                let mapped = map_since(tr, start, pos);
                tr.set_node_attr(mapped, "rowspan", shorter).ok()?;
                col = next_col;
            } else if row + 1 < map.height() && pos == map.map()[index + width] {
                // Starts here and continues below → recreate it one row down.
                let new_attrs = cell.attrs().with("rowspan", shorter);
                let copy = schema
                    .create_node(cell.type_name(), new_attrs, cell.content().clone())
                    .ok()?;
                let new_pos = map.position_at(row + 1, col);
                let mapped = map_since(tr, start, new_pos);
                tr.replace_with(mapped, mapped, Fragment::from_node(copy))
                    .ok()?;
                col = next_col;
            } else {
                col = next_col;
            }
        }
        Some(())
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

    /// `addRowBefore` — insert an empty row above the selection's top row.
    pub fn add_row_before(state: &EditorState) -> Option<Transaction> {
        (|| {
            let info = selected_rect(state)?;
            let mut tr = state.tr();
            add_row(&mut tr, &info, info.rect.top, state.schema())?;
            tr.doc_changed().then_some(tr)
        })()
    }

    /// `addRowAfter` — insert an empty row below the selection's bottom row.
    pub fn add_row_after(state: &EditorState) -> Option<Transaction> {
        (|| {
            let info = selected_rect(state)?;
            let mut tr = state.tr();
            add_row(&mut tr, &info, info.rect.bottom, state.schema())?;
            tr.doc_changed().then_some(tr)
        })()
    }

    /// `addColumnBefore` — insert an empty column left of the selection.
    pub fn add_column_before(state: &EditorState) -> Option<Transaction> {
        (|| {
            let info = selected_rect(state)?;
            let mut tr = state.tr();
            add_column(&mut tr, &info, info.rect.left, state.schema())?;
            tr.doc_changed().then_some(tr)
        })()
    }

    /// `addColumnAfter` — insert an empty column right of the selection.
    pub fn add_column_after(state: &EditorState) -> Option<Transaction> {
        (|| {
            let info = selected_rect(state)?;
            let mut tr = state.tr();
            add_column(&mut tr, &info, info.rect.right, state.schema())?;
            tr.doc_changed().then_some(tr)
        })()
    }

    /// `deleteRow` — remove the selected rows. If that would remove every row, the
    /// whole table is deleted instead. Port of `deleteRow`.
    pub fn delete_row(state: &EditorState) -> Option<Transaction> {
        (|| {
            let info = selected_rect(state)?;
            if info.rect.top == 0 && info.rect.bottom == info.map.height() {
                return delete_table_tr(state, info.table_start);
            }
            let mut tr = state.tr();
            let table_start = info.table_start;
            let rect = info.rect;
            let mut cur = info;
            let mut i = rect.bottom;
            loop {
                i -= 1;
                remove_row(&mut tr, &cur, i, state.schema())?;
                if i == rect.top {
                    break;
                }
                cur = recompute(&tr, table_start, rect)?;
            }
            tr.doc_changed().then_some(tr)
        })()
    }

    /// `deleteColumn` — remove the selected columns (or the whole table if that empties
    /// it). Port of `deleteColumn`.
    pub fn delete_column(state: &EditorState) -> Option<Transaction> {
        (|| {
            let info = selected_rect(state)?;
            if info.rect.left == 0 && info.rect.right == info.map.width() {
                return delete_table_tr(state, info.table_start);
            }
            let mut tr = state.tr();
            let table_start = info.table_start;
            let rect = info.rect;
            let mut cur = info;
            let mut i = rect.right;
            loop {
                i -= 1;
                // A column past the recomputed grid is gone already: removing a
                // column from a ragged table can narrow the grid by more than one
                // (its widest row may be the one that lost a cell).
                if i < cur.map.width() {
                    remove_column(&mut tr, &cur, i)?;
                }
                if i == rect.left {
                    break;
                }
                cur = recompute(&tr, table_start, rect)?;
            }
            tr.doc_changed().then_some(tr)
        })()
    }

    /// `deleteTable` — delete the entire table the selection is in.
    pub fn delete_table(state: &EditorState) -> Option<Transaction> {
        (|| {
            let info = selected_rect(state)?;
            delete_table_tr(state, info.table_start)
        })()
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
        let mut tr = state.tr();
        // Blank cells in DESCENDING document order so each in-place content replace only
        // shifts positions after it (already processed) — the remaining lower cells keep
        // their original positions, so no mapping is needed.
        let mut cells = info.map.cells_in_rect(info.rect);
        cells.sort_unstable_by(|a, b| b.cmp(a));
        for cell_pos in cells {
            let Some(cell) = info.table.node_at(cell_pos - info.table_start) else {
                continue;
            };
            if cell_is_empty(&cell) {
                continue;
            }
            let para = schema
                .create_node("paragraph", Attrs::new(), Fragment::empty())
                .ok()?;
            let from = cell_pos + 1;
            let to = from + cell.content_size();
            tr.replace_with(from, to, Fragment::from_node(para)).ok()?;
        }
        // Collapse into the top-left cell (its position is unchanged — all edits were at
        // higher positions).
        let top_left = info.map.cell_at(info.rect.top, info.rect.left)?;
        let sel = Selection::near(tr.doc(), Pos(top_left + 1), 1);
        tr.set_selection(sel);
        Some(tr)
    }

    /// `mergeCells` — collapse a rectangular [`Selection::Cell`] into its top-left cell,
    /// pulling the other cells' content in and growing the master's colspan/rowspan to
    /// cover the rectangle. No-op unless a multi-cell rectangle is selected and it is
    /// rectangular (no cell straddles an edge). Port of `mergeCells`.
    pub fn merge_cells(state: &EditorState) -> Option<Transaction> {
        (|| {
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
            // The master is the rectangle's top-left cell, which grows over the rest,
            // holes included: they are no cell to delete. The top-left slot is never
            // a hole: a row's own cells all lie left of its holes (a hole can be
            // followed by a slot a rowspan from above covers, never by one of the
            // row's own cells), and the rectangle's top row holds a selected cell at
            // or right of its left edge.
            let mut tr = state.tr();
            let mut seen: Vec<usize> = Vec::new();
            let mut content = Fragment::empty();
            let mut master: Option<(usize, Node)> = None;
            for row in rect.top..rect.bottom {
                for col in rect.left..rect.right {
                    let pos = map.map()[row * width + col];
                    if pos == HOLE || seen.contains(&pos) {
                        continue;
                    }
                    seen.push(pos);
                    let cell = cell_node(&info, pos)?;
                    match &master {
                        None => master = Some((pos, cell)),
                        Some(_) => {
                            if !cell_is_empty(&cell) {
                                content = content.append(cell.content());
                            }
                            let mapped = tr.mapping().map(pos, 1);
                            tr.delete(mapped, mapped + cell.node_size()).ok()?;
                        }
                    }
                }
            }
            let (master_pos, master_cell) = master?;
            // Grow the master to cover the rectangle.
            tr.set_node_attr(
                master_pos,
                "colspan",
                AttrValue::Int((rect.right - rect.left) as i64),
            )
            .ok()?;
            tr.set_node_attr(
                master_pos,
                "rowspan",
                AttrValue::Int((rect.bottom - rect.top) as i64),
            )
            .ok()?;
            if content.size() > 0 {
                let content_end = master_pos + 1 + master_cell.content_size();
                let start = if cell_is_empty(&master_cell) {
                    master_pos + 1
                } else {
                    content_end
                };
                let from = tr.mapping().map(start, 1);
                let to = tr.mapping().map(content_end, 1);
                tr.replace_with(from, to, content).ok()?;
            }
            tr.set_selection(Selection::cell(Pos(master_pos), Pos(master_pos)));
            Some(tr)
        })()
    }

    /// `splitCell` — split the merged cell under the cursor (or a single selected cell)
    /// back into 1×1 cells, filling the freed grid slots with empty cells. No-op on a
    /// 1×1 cell or a multi-cell selection. Port of `splitCell` (plain-cell type).
    ///
    /// **Cost** (#1185): two attribute steps plus one insert per row the cell
    /// spans, not one per vacated slot, so a 1000×1000 cell is 1002 steps. The
    /// cells it creates are as many as the slots it vacates, and there is no cap
    /// (PM has none): the slots come from the [`TableMap`], which holds at most
    /// [`tables::grid_slot_budget`] of them, so a split creates at most
    /// `grid_slot_budget` cells: 2^22 (about 4.2 M) for a table of fewer than
    /// 2^21 cells, twice its cell count beyond that. Measured in a release
    /// build: 1000×1000 is 1.0 M cells in 0.23 s and 390 MB peak; a pasted
    /// 1000 × 4000 cell is 4.0 M cells in 1.1 s and 1.6 GB. A tall cell costs more per slot than a wide one, because each
    /// per-row step keeps its own copy of the table's row list: 62 × 16,000 is
    /// 1.0 M cells in 8.1 s and 2.4 GB, which is what `addColumnBefore` on a
    /// 16,000-row table costs too (16,000 steps, 6.4 s, 2.1 GB; #1200).
    pub fn split_cell(state: &EditorState) -> Option<Transaction> {
        (|| {
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
            let mut tr = state.tr();
            // Reset the master to 1×1 (attr-only: no position shift).
            tr.set_node_attr(cell_pos, "colspan", AttrValue::Int(1))
                .ok()?;
            tr.set_node_attr(cell_pos, "rowspan", AttrValue::Int(1))
                .ok()?;
            // Fill the vacated grid slots with empty cells: one insert per row
            // holding all of that row's new cells (#1185). PM inserts a cell per
            // slot, each mapped through the steps before it, which is 10^6 steps
            // for a pasted 1000×1000 cell; the document is the same, since each
            // of those inserts landed right after the one before it.
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
                let mapped = tr.mapping().map(at, 1);
                tr.replace_with(mapped, mapped, Fragment::from_children(cells))
                    .ok()?;
            }
            // Collapse the selection into the (now 1×1) master cell.
            let sel = Selection::near(tr.doc(), Pos(cell_pos + 1), 1);
            tr.set_selection(sel);
            tr.doc_changed().then_some(tr)
        })()
    }
}

fn cell(s: &Schema, colspan: i64, rowspan: i64, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    let p = s.branch("paragraph", content).unwrap();
    s.create_node(
        "table_cell",
        Attrs::from_iter([
            ("colspan", AttrValue::Int(colspan)),
            ("rowspan", AttrValue::Int(rowspan)),
        ]),
        Fragment::from_node(p),
    )
    .unwrap()
}

/// A table from rows of `(colspan, rowspan)`; every cell holds its number.
fn table(s: &Schema, rows: &[Vec<(i64, i64)>]) -> Node {
    table_texts(s, rows, |n| format!("c{n}"))
}

fn table_texts(s: &Schema, rows: &[Vec<(i64, i64)>], text: impl Fn(usize) -> String) -> Node {
    let mut n = 0;
    let rows = rows
        .iter()
        .map(|r| {
            let cells = r
                .iter()
                .map(|&(c, h)| {
                    n += 1;
                    cell(s, c, h, &text(n))
                })
                .collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    s.create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap()
}

/// The table at doc position 0, then an empty paragraph.
fn state_with(t: Node) -> EditorState {
    let s = Schema::starter_kit();
    let tail = s
        .create_node("paragraph", Attrs::new(), Fragment::empty())
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![t, tail]))
        .unwrap();
    EditorState::create(Rc::new(s), doc, default_plugins())
}

/// The position before row `r`'s cell `i` (the table is at doc position 0).
fn cell_pos(t: &Node, r: usize, i: usize) -> usize {
    let mut pos = 1;
    for j in 0..r {
        pos += t.child(j).node_size();
    }
    pos += 1;
    let row = t.child(r);
    for k in 0..i {
        pos += row.child(k).node_size();
    }
    pos
}

/// A caret inside row `r`'s cell `i`.
fn caret_in(t: &Node, r: usize, i: usize) -> Selection {
    Selection::cursor(Pos(cell_pos(t, r, i) + 2))
}

/// The registered command's transaction, not applied.
fn tr_of(st: &EditorState, name: &str) -> Option<Transaction> {
    let cmd = st.command(name).expect("a registered command");
    let mut out = None;
    let applied = cmd(st, Some(&mut |tr: Transaction| out = Some(tr)));
    assert_eq!(applied, out.is_some(), "{name}");
    out
}

fn steps(st: &EditorState, name: &str) -> usize {
    tr_of(st, name)
        .unwrap_or_else(|| panic!("{name} applies"))
        .steps()
        .len()
}

/// `h` rows of `w` plain cells.
fn grid(s: &Schema, w: usize, h: usize) -> Node {
    table(s, &vec![vec![(1, 1); w]; h])
}

// ------------------------------------------------------------------
// One step, whatever the height.
// ------------------------------------------------------------------

/// Each command on a 120-row table is one step: before #1200 a column insert
/// or delete was 120, a split of a 3 × 100 cell 102, a merge of 2 × 100
/// cells 201, and blanking 3 × 100 cells 300.
#[test]
fn every_table_command_is_one_step_on_a_tall_table() {
    let s = Schema::starter_kit();
    let t = grid(&s, 3, 120);
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 0, 0);
    for c in ["addColumnBefore", "addColumnAfter", "deleteColumn"] {
        assert_eq!(steps(&st, c), 1, "{c}");
    }
    // Two columns of three: one step, not one per row per column.
    st.selection = Selection::cell(Pos(cell_pos(&t, 0, 0)), Pos(cell_pos(&t, 119, 1)));
    assert_eq!(steps(&st, "deleteColumn"), 1, "deleteColumn x2");
    // Rows 10..100, then two columns by those rows merged, then blanked.
    st.selection = Selection::cell(Pos(cell_pos(&t, 10, 0)), Pos(cell_pos(&t, 99, 2)));
    assert_eq!(steps(&st, "deleteRow"), 1, "deleteRow x90");
    assert_eq!(steps(&st, "deleteCellSelection"), 1, "deleteCellSelection");
    st.selection = Selection::cell(Pos(cell_pos(&t, 10, 0)), Pos(cell_pos(&t, 109, 1)));
    assert_eq!(steps(&st, "mergeCells"), 1, "mergeCells 2 x 100");

    // A 3 × 100 merged cell in the middle of a 120-row table.
    let rows: Vec<Vec<(i64, i64)>> = (0..120)
        .map(|r| match r {
            10 => vec![(1, 1), (3, 100), (1, 1)],
            11..110 => vec![(1, 1), (1, 1)],
            _ => vec![(1, 1); 5],
        })
        .collect();
    let t = table(&s, &rows);
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 10, 1);
    assert_eq!(steps(&st, "splitCell"), 1, "splitCell 3 x 100");
    // A row through the tall cell: its rowspan grows in the same step.
    st.selection = caret_in(&t, 50, 0);
    assert_eq!(steps(&st, "addRowAfter"), 1, "addRowAfter");
    assert_eq!(steps(&st, "addRowBefore"), 1, "addRowBefore");
}

/// The size pin of the issue, scaled to a debug build: a column into 2000
/// rows is one step, and one undo takes it back.
#[test]
fn a_column_into_2000_rows_is_one_step() {
    let s = Schema::starter_kit();
    let t = grid(&s, 2, 2000);
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 0, 0);
    let tr = tr_of(&st, "addColumnBefore").expect("adds");
    assert_eq!(tr.steps().len(), 1);
    let next = st.apply(tr);
    let map = TableMap::compute(next.doc.child(0), 1);
    assert_eq!((map.width(), map.height()), (3, 2000));
    let back = next.run("undo").expect("undo");
    assert!(back.doc == st.doc);
}

/// A row the command does not touch is not rebuilt: it is the very node it
/// was (shared, not copied), so the command's cost is the rows it edits.
#[test]
fn rows_a_command_does_not_touch_are_shared() {
    let s = Schema::starter_kit();
    let rows: Vec<Vec<(i64, i64)>> = (0..60)
        .map(|r| match r {
            20 => vec![(1, 1), (2, 10), (1, 1)],
            21..30 => vec![(1, 1), (1, 1)],
            _ => vec![(1, 1); 4],
        })
        .collect();
    let t = table(&s, &rows);
    let mut st = state_with(t.clone());
    st.selection = caret_in(&t, 20, 1);
    let next = st.run("splitCell").expect("splits");
    let nt = next.doc.child(0);
    for r in 0..60 {
        let shared = nt.child(r).same_ref(t.child(r));
        assert_eq!(shared, !(20..30).contains(&r), "row {r}");
    }
    st.selection = caret_in(&t, 40, 0);
    let next = st.run("addRowAfter").expect("adds");
    let nt = next.doc.child(0);
    assert_eq!(nt.child_count(), 61);
    for r in 0..60 {
        let new = if r <= 40 { r } else { r + 1 };
        assert!(nt.child(new).same_ref(t.child(r)), "row {r}");
    }
}

// ------------------------------------------------------------------
// The same edit as before: document, selection and mapping.
// ------------------------------------------------------------------

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

thread_local! {
    /// How often the per-row commands' step-by-step selection mapping and the
    /// whole-transaction mapping disagreed.
    static EAGER_DIFFERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

type RefCmd = fn(&EditorState) -> Option<Transaction>;

const CMDS: &[(&str, RefCmd)] = &[
    ("addRowBefore", reference::add_row_before),
    ("addRowAfter", reference::add_row_after),
    ("addColumnBefore", reference::add_column_before),
    ("addColumnAfter", reference::add_column_after),
    ("deleteRow", reference::delete_row),
    ("deleteColumn", reference::delete_column),
    ("mergeCells", reference::merge_cells),
    ("splitCell", reference::split_cell),
    ("deleteTable", reference::delete_table),
    ("deleteCellSelection", reference::clear_cells),
];

/// Asserts `got` makes the edit `want` made: the same document and selection,
/// every position mapped to the same place, and undo taking it back.
fn assert_same_edit(st: &EditorState, got: Transaction, want: Transaction, what: &str) {
    for p in 0..=st.doc.content_size() {
        for a in [-1, 1] {
            let (g, w) = (
                got.mapping().map_result(p, a),
                want.mapping().map_result(p, a),
            );
            assert_eq!(
                (g.pos, g.deleted()),
                (w.pos, w.deleted()),
                "{what}: position {p} assoc {a}\n got {:?}\n want {:?}",
                got.mapping().maps(),
                want.mapping().maps()
            );
        }
    }
    let changed = got.doc_changed();
    assert_eq!(changed, want.doc_changed(), "{what}: changes the document");
    assert_eq!(
        got.selection_set(),
        want.selection_set(),
        "{what}: sets the selection"
    );
    let set = want.selection_set();
    let one_step = got.steps().len() <= 1;
    // The selection a command leaves where it was is mapped through the
    // whole transaction and resolved in its final document, as
    // ProseMirror's `Transaction.selection` is. The per-row commands mapped
    // it after every step, so a caret the edit deleted was placed in an
    // intermediate document and then mapped on; one step places it once.
    let lazily = st.selection.map(want.doc(), want.mapping());
    let (g, w) = (st.apply(got), st.apply(want));
    assert!(
        g.doc == w.doc,
        "{what}: the document\n got {:?}\n want {:?}",
        g.doc,
        w.doc
    );
    if set || g.selection != w.selection {
        if !set && one_step {
            EAGER_DIFFERS.with(|n| n.set(n.get() + 1));
        }
        // A command that still takes a step per row (a removal from a grid
        // whose cells overlap) maps it after each of its own steps.
        if set || one_step {
            let expected = if set { &w.selection } else { &lazily };
            assert_eq!(
                &g.selection, expected,
                "{what}: the selection (was {:?})",
                w.selection
            );
        }
    }
    if !changed {
        return;
    }
    let back = g
        .run("undo")
        .unwrap_or_else(|| panic!("{what}: undo refused"));
    assert!(back.doc == st.doc, "{what}: undo");
    let again = back.run("redo").expect("redo");
    assert!(again.doc == g.doc, "{what}: redo");
}

/// A random selection in `t`: a caret in a cell, or a cell selection.
fn random_selection(rng: &mut Rng, t: &Node) -> Option<Selection> {
    let cells: Vec<(usize, usize)> = (0..t.child_count())
        .flat_map(|r| (0..t.child(r).child_count()).map(move |i| (r, i)))
        .collect();
    if cells.is_empty() {
        return None;
    }
    let (r, i) = cells[rng.below(cells.len())];
    if rng.below(3) == 0 {
        return Some(caret_in(t, r, i));
    }
    let (r2, i2) = cells[rng.below(cells.len())];
    Some(Selection::cell(
        Pos(cell_pos(t, r, i)),
        Pos(cell_pos(t, r2, i2)),
    ))
}

/// A random well-formed `w × h` grid tiled with rectangles.
fn random_tiled(rng: &mut Rng) -> Vec<Vec<(i64, i64)>> {
    let w = 1 + rng.below(6);
    let h = 1 + rng.below(6);
    let mut owner = vec![false; w * h];
    let mut rows: Vec<Vec<(i64, i64)>> = vec![Vec::new(); h];
    for r in 0..h {
        for c in 0..w {
            if owner[r * w + c] {
                continue;
            }
            let mut cs = (1 + rng.below(3)).min(w - c);
            let rs = (1 + rng.below(3)).min(h - r);
            let mut free = 0;
            while c + free < w && !owner[r * w + c + free] && free < cs {
                free += 1;
            }
            cs = free;
            for rr in r..r + rs {
                for cc in c..c + cs {
                    owner[rr * w + cc] = true;
                }
            }
            rows[r].push((cs as i64, rs as i64));
        }
    }
    rows
}

/// Every command, on well-formed, ragged and overlapping tables with and
/// without text, from a caret and from cell selections: the edit is the one
/// the per-row commands made.
#[test]
fn every_command_makes_the_edit_the_per_row_commands_made() {
    let s = Schema::starter_kit();
    let mut rng = Rng(0x1200_1214_5eed_0001);
    const SPANS: &[i64] = &[1, 1, 1, 1, 2, 3, 0, -1, 40];
    let (mut compared, mut one_step_of_many, mut per_row) = (0, 0, 0);
    for case in 0..800 {
        let rows = if case % 2 == 0 {
            random_tiled(&mut rng)
        } else {
            let h = 1 + rng.below(6);
            (0..h)
                .map(|_| {
                    (0..rng.below(5))
                        .map(|_| (SPANS[rng.below(SPANS.len())], SPANS[rng.below(SPANS.len())]))
                        .collect()
                })
                .collect()
        };
        let texts: Vec<bool> = (0..64).map(|_| rng.below(3) != 0).collect();
        let t = table_texts(&s, &rows, |n| {
            if texts[n % 64] {
                format!("t{n}")
            } else {
                String::new()
            }
        });
        let Some(sel) = random_selection(&mut rng, &t) else {
            continue;
        };
        let mut st = state_with(t);
        st.selection = sel;
        for &(c, reference) in CMDS {
            let what = format!("case {case}: {c} at {:?} in {rows:?}", st.selection);
            match (tr_of(&st, c), reference(&st)) {
                (None, None) => {}
                (Some(g), Some(w)) => {
                    // Both paths of a removal: one step where the per-row
                    // commands took several, and the per-row fallback.
                    match (g.steps().len(), w.steps().len()) {
                        (1, n) if n > 1 => one_step_of_many += 1,
                        (n, _) if n > 1 => per_row += 1,
                        _ => {}
                    }
                    assert_same_edit(&st, g, w, &what);
                    compared += 1;
                }
                (g, w) => panic!(
                    "{what}: applies {} where it applied {}",
                    g.is_some(),
                    w.is_some()
                ),
            }
        }
    }
    let differs = EAGER_DIFFERS.with(|n| n.get());
    eprintln!(
        "compared {compared}: one step for many {one_step_of_many}, per row {per_row}; \
         selection placed once where it was placed per step: {differs}"
    );
    assert!(
        one_step_of_many > 1000 && per_row > 20,
        "{one_step_of_many} / {per_row}"
    );
    // The positive control: most cases applied, on both sides.
    assert!(compared > 4000, "only {compared} compared");
}
