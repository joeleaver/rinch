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
use crate::model::{Attrs, Fragment, Node};
use crate::pos::Pos;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::tables::{self, Rect, TableMap};

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

/// The colspan of `cell`, clamped to ≥1.
fn colspan(cell: &Node) -> i64 {
    cell.attrs().get_int("colspan").unwrap_or(1).max(1)
}

/// The rowspan of `cell`, clamped to ≥1.
fn rowspan(cell: &Node) -> i64 {
    cell.attrs().get_int("rowspan").unwrap_or(1).max(1)
}

/// The cell node covering grid slot `pos` (an absolute cell position from the map).
fn cell_node(info: &TableRect, pos: usize) -> Option<Node> {
    info.table.node_at(pos - info.table_start)
}

// ===================================================================
// Column operations
// ===================================================================

/// Insert a column at grid index `col` (`0..=width`). Where an existing cell spans
/// across the insertion line its colspan grows; otherwise an empty cell is spliced
/// into each row. Port of `addColumn`. Multiple inserts shift later positions, so
/// every target position is mapped through the transaction's accumulated mapping.
fn add_column(tr: &mut Transaction, info: &TableRect, col: usize, schema: &Schema) -> Option<()> {
    let map = &info.map;
    let width = map.width();
    let mut row = 0;
    while row < map.height() {
        let index = row * width + col;
        let inside_span = col > 0 && col < width && map.map()[index - 1] == map.map()[index];
        if inside_span {
            // Inside a horizontally-spanning cell → widen it.
            let pos = map.map()[index];
            let cell = cell_node(info, pos)?;
            let mapped = tr.mapping().map(pos, 1);
            tr.set_node_attr(mapped, "colspan", AttrValue::Int(colspan(&cell) + 1))
                .ok()?;
            row += rowspan(&cell).max(1) as usize;
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
    let map = &info.map;
    let width = map.width();
    let mut row = 0;
    while row < map.height() {
        let index = row * width + col;
        let pos = map.map()[index];
        let cell = cell_node(info, pos)?;
        let spans_more = (col > 0 && map.map()[index - 1] == pos)
            || (col < width - 1 && map.map()[index + 1] == pos);
        if spans_more {
            let mapped = tr.mapping().map(pos, 1);
            tr.set_node_attr(mapped, "colspan", AttrValue::Int(colspan(&cell) - 1))
                .ok()?;
        } else {
            let start = tr.mapping().map(pos, 1);
            tr.delete(start, start + cell.node_size()).ok()?;
        }
        row += rowspan(&cell).max(1) as usize;
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
        let from_above =
            row > 0 && row < map.height() && map.map()[index] == map.map()[index - width];
        if from_above {
            let pos = map.map()[index];
            let cell = cell_node(info, pos)?;
            tr.set_node_attr(pos, "rowspan", AttrValue::Int(rowspan(&cell) + 1))
                .ok()?;
            col += colspan(&cell).max(1) as usize;
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
fn remove_row(tr: &mut Transaction, info: &TableRect, row: usize, schema: &Schema) -> Option<()> {
    let map = &info.map;
    let width = map.width();
    let row_pos = map.row_start(row)?;
    let next_row = map.row_start(row + 1)?;
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
        seen.push(pos);
        let cell = cell_node(info, pos)?;
        let cspan = colspan(&cell).max(1) as usize;
        if row > 0 && pos == map.map()[index - width] {
            // Spans into this row from above → reduce its rowspan.
            let mapped = tr.mapping().map(pos, 1);
            tr.set_node_attr(mapped, "rowspan", AttrValue::Int(rowspan(&cell) - 1))
                .ok()?;
            col += cspan;
        } else if row + 1 < map.height() && pos == map.map()[index + width] {
            // Starts here and continues below → recreate it one row down.
            let new_attrs = cell
                .attrs()
                .with("rowspan", AttrValue::Int(rowspan(&cell) - 1));
            let copy = schema
                .create_node(cell.type_name(), new_attrs, cell.content().clone())
                .ok()?;
            let new_pos = map.position_at(row + 1, col);
            let mapped = tr.mapping().map(new_pos, 1);
            tr.replace_with(mapped, mapped, Fragment::from_node(copy))
                .ok()?;
            col += cspan;
        } else {
            col += cspan;
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
pub fn add_row_before() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        let mut tr = state.tr();
        add_row(&mut tr, &info, info.rect.top, state.schema())?;
        tr.doc_changed().then_some(tr)
    })
}

/// `addRowAfter` — insert an empty row below the selection's bottom row.
pub fn add_row_after() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        let mut tr = state.tr();
        add_row(&mut tr, &info, info.rect.bottom, state.schema())?;
        tr.doc_changed().then_some(tr)
    })
}

/// `addColumnBefore` — insert an empty column left of the selection.
pub fn add_column_before() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        let mut tr = state.tr();
        add_column(&mut tr, &info, info.rect.left, state.schema())?;
        tr.doc_changed().then_some(tr)
    })
}

/// `addColumnAfter` — insert an empty column right of the selection.
pub fn add_column_after() -> Command {
    command_tr(|state| {
        let info = selected_rect(state)?;
        let mut tr = state.tr();
        add_column(&mut tr, &info, info.rect.right, state.schema())?;
        tr.doc_changed().then_some(tr)
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
        let mut tr = state.tr();
        let table_start = info.table_start;
        let rect = info.rect;
        let mut cur = info;
        let mut i = rect.right;
        loop {
            i -= 1;
            remove_column(&mut tr, &cur, i)?;
            if i == rect.left {
                break;
            }
            cur = recompute(&tr, table_start, rect)?;
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
        let bleed_left = rect.left > 0 && m[il] == m[il - 1];
        let bleed_right = rect.right < width && m[ir] == m[ir + 1];
        if bleed_left || bleed_right {
            return true;
        }
    }
    // Top/bottom edges.
    for c in rect.left..rect.right {
        let it = rect.top * width + c;
        let ib = (rect.bottom - 1) * width + c;
        let bleed_top = rect.top > 0 && m[it] == m[it - width];
        let bleed_bottom = rect.bottom < height && m[ib] == m[ib + width];
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
pub(crate) fn clear_cells(state: &EditorState) -> Option<Transaction> {
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
        let mut tr = state.tr();
        let mut seen: Vec<usize> = Vec::new();
        let mut content = Fragment::empty();
        let mut master: Option<(usize, Node)> = None;
        for row in rect.top..rect.bottom {
            for col in rect.left..rect.right {
                let pos = map.map()[row * width + col];
                if seen.contains(&pos) {
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
    })
}

/// `splitCell` — split the merged cell under the cursor (or a single selected cell)
/// back into 1×1 cells, filling the freed grid slots with empty cells. No-op on a
/// 1×1 cell or a multi-cell selection. Port of `splitCell` (plain-cell type).
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
        let mut tr = state.tr();
        // Reset the master to 1×1 (attr-only: no position shift).
        tr.set_node_attr(cell_pos, "colspan", AttrValue::Int(1))
            .ok()?;
        tr.set_node_attr(cell_pos, "rowspan", AttrValue::Int(1))
            .ok()?;
        // Fill the vacated grid slots with empty cells.
        for row in rect.top..rect.bottom {
            let mut at = info.map.position_at(row, rect.left);
            if row == rect.top {
                at += cell_size; // place new cells just after the master
            }
            for col in rect.left..rect.right {
                if row == rect.top && col == rect.left {
                    continue; // the master itself stays
                }
                let mapped = tr.mapping().map(at, 1);
                let new_cell = make_empty_cell(schema)?;
                tr.replace_with(mapped, mapped, Fragment::from_node(new_cell))
                    .ok()?;
            }
        }
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
