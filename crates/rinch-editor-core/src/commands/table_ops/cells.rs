//! Cell-level commands: clearing a cell selection, copying it, merging and
//! splitting cells.

use super::*;

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
        // The cleared cell keeps its alignment (#2210).
        let para = empty_paragraph_like(schema, Some(cell))?;
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
            // Cells like the master: its type (PM `splitCell`'s default
            // `getCellType`) and its alignment (#2210).
            let cells = (0..count)
                .map(|_| make_empty_cell(schema, cell.type_name(), 1, Some(&cell)))
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
