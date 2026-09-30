//! Table geometry — [`TableMap`] resolves a `table` node into a rectangular grid
//! of cell positions, honoring `colspan`/`rowspan`. It is the load-bearing piece
//! every table command, the cell selection, and the cell-navigation layer build
//! on (you cannot reason about "the cell to the right" or "the column under the
//! cursor" without it once cells can span).
//!
//! A faithful port of ProseMirror's `prosemirror-tables/src/tablemap.ts`, with one
//! deliberate change: where PM stores cell offsets **relative to the table's
//! content start** (so the map can be cached per-node), this port stores
//! **absolute document positions**. Everything else in this editor speaks absolute
//! [`Pos`], so absolute positions keep the consumers (selection, commands, the
//! desktop view) free of `table_start` bookkeeping. We recompute the map on demand
//! rather than caching it.
//!
//! Positions stored in [`TableMap::map`] are the document position **immediately
//! before** each cell (its open-token position) — the same anchor a
//! [`crate::selection::NodeSelection`] would use, and what `cell_around` returns.

use crate::model::Node;
use crate::pos::{Pos, ResolvedPos};

/// The schema type name of the table container.
pub const TABLE: &str = "table";
/// The schema type name of a table row.
pub const TABLE_ROW: &str = "table_row";
/// The schema group shared by `table_cell` and `table_header_cell`.
pub const CELL_GROUP: &str = "cell";

/// True if `node` is a table container.
pub fn is_table(node: &Node) -> bool {
    node.type_name() == TABLE
}

/// True if `node` is a table row.
pub fn is_row(node: &Node) -> bool {
    node.type_name() == TABLE_ROW
}

/// True if `node` is a table cell (`table_cell` or `table_header_cell`), i.e. a
/// member of the `cell` group.
pub fn is_cell(node: &Node) -> bool {
    node.node_type().group() == Some(CELL_GROUP)
}

/// A rectangle of grid coordinates: `[left, right) × [top, bottom)` with `right`
/// and `bottom` **exclusive**. Coordinates are 0-based column/row indices in the
/// table grid, *not* document positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Leftmost column (inclusive).
    pub left: usize,
    /// Topmost row (inclusive).
    pub top: usize,
    /// One past the rightmost column (exclusive).
    pub right: usize,
    /// One past the bottommost row (exclusive).
    pub bottom: usize,
}

/// Which way [`TableMap::next_cell`] steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Move left/right (columns).
    Horiz,
    /// Move up/down (rows).
    Vert,
}

/// Sentinel for a grid slot that no cell covers (a ragged/malformed table). A
/// well-formed table built by [`crate::commands::build_table`] never leaves one.
const UNSET: usize = usize::MAX;

/// A resolved table grid: `width × height` slots, each pointing at the document
/// position before the cell that covers it. A merged cell (colspan/rowspan > 1)
/// fills several adjacent slots, all holding its single position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableMap {
    width: usize,
    height: usize,
    /// `width * height` entries in row-major order; each is the absolute document
    /// position immediately before the covering cell (or [`UNSET`] for a hole).
    map: Vec<usize>,
    /// `height + 1` entries: `row_starts[r]` is the document position immediately
    /// before row `r`; `row_starts[height]` is the position after the last row's
    /// content (before the table's close token). Lets [`Self::position_at`] find a
    /// row's inner end without re-walking the table.
    row_starts: Vec<usize>,
}

impl TableMap {
    /// Resolve `table` (a `table` node) whose content begins at document position
    /// `table_start` (i.e. `table_pos + 1`, the open-token position of its first
    /// row). Total: a structurally-odd table yields a best-effort grid with
    /// [`UNSET`] holes rather than panicking.
    ///
    /// The grid is **bounded** (#1176): its width is [`column_count`], which
    /// caps `width × height` at [`grid_slot_budget`]. A cell is cut at the
    /// grid's right edge (and at its bottom, for a rowspan past the last row),
    /// and a cell that starts past the right edge is in no slot, so
    /// [`Self::find_cell`] answers `None` for it and a command there does
    /// nothing. A rectangular table with no spans is never cut: see
    /// [`grid_slot_budget`].
    pub fn compute(table: &Node, table_start: usize) -> TableMap {
        let height = table.child_count();
        let width = column_count(table);
        let mut map = vec![UNSET; width * height];
        let mut row_starts = Vec::with_capacity(height + 1);

        // `pos` walks the document position; `map_pos` walks the flat grid,
        // skipping slots already claimed by a rowspan from an earlier row.
        let mut pos = table_start;
        let mut map_pos = 0usize;

        for row in 0..height {
            let row_node = table.child(row);
            row_starts.push(pos);
            pos += 1; // step past the row's open token → first cell's position
            // The grid cursor never leaves this row: a cell that would start
            // past the row's end (a row wider than the capped grid) is in no
            // slot, and a colspan is cut at the row's end rather than running
            // on into the next row's slots.
            let row_end = (row + 1) * width;
            let mut i = 0usize;
            loop {
                while map_pos < row_end && map[map_pos] != UNSET {
                    map_pos += 1;
                }
                if i == row_node.child_count() {
                    break;
                }
                let cell = row_node.child(i);
                if map_pos < row_end {
                    let colspan = span_attr(cell, "colspan").min(row_end - map_pos);
                    // An overlong rowspan is cut at the last row.
                    let rowspan = span_attr(cell, "rowspan").min(height - row);
                    for h in 0..rowspan {
                        let start = map_pos + h * width;
                        for slot in &mut map[start..start + colspan] {
                            if *slot == UNSET {
                                *slot = pos;
                            }
                        }
                    }
                    map_pos += colspan;
                }
                pos += cell.node_size();
                i += 1;
            }
            // Advance the grid cursor to the end of this row (skips any trailing
            // holes a ragged row leaves) and step past the row's close token.
            map_pos = (row + 1) * width;
            pos += 1;
        }
        row_starts.push(table_start + table.content_size());

        TableMap {
            width,
            height,
            map,
            row_starts,
        }
    }

    /// The number of columns in the grid.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The number of rows in the grid.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The raw grid (row-major, `width × height`); each entry is the document
    /// position before the covering cell, or [`usize::MAX`] for a hole.
    pub fn map(&self) -> &[usize] {
        &self.map
    }

    /// The grid rectangle covered by the cell at document position `pos`, honoring
    /// its colspan/rowspan. `None` if no cell in the grid starts at `pos`.
    pub fn find_cell(&self, pos: usize) -> Option<Rect> {
        for i in 0..self.map.len() {
            if self.map[i] != pos {
                continue;
            }
            let left = i % self.width;
            let top = i / self.width;
            let mut right = left + 1;
            let mut bottom = top + 1;
            while right < self.width && self.map[i + (right - left)] == pos {
                right += 1;
            }
            while bottom < self.height && self.map[i + self.width * (bottom - top)] == pos {
                bottom += 1;
            }
            return Some(Rect {
                left,
                top,
                right,
                bottom,
            });
        }
        None
    }

    /// The grid rectangle of the cell covering slot `index`: its spans as the
    /// map resolved them, which is what every span a table command writes is
    /// computed from (#1184) and what the view places a cell by (#1182). The
    /// attributes are the document's word and may say `i64::MAX`; the map cuts
    /// a span at the grid's edge, so a rectangle is at most the grid and `± 1`
    /// on its sides cannot overflow. `None` for a hole or an index past the
    /// grid, neither of which is a cell.
    ///
    /// The walk goes up and then left to the cell's origin, then right and down
    /// from it. On a malformed table whose cells overlap, a cell's slots need
    /// not be a rectangle and the answer is an approximation; callers therefore
    /// step past a cell by at least one slot.
    pub fn cell_rect(&self, index: usize) -> Option<Rect> {
        let m = &self.map;
        let width = self.width;
        let pos = *m.get(index)?;
        if pos == UNSET {
            return None;
        }
        let (mut top, mut left) = (index / width, index % width);
        while top > 0 && m[(top - 1) * width + left] == pos {
            top -= 1;
        }
        while left > 0 && m[top * width + left - 1] == pos {
            left -= 1;
        }
        let mut right = left + 1;
        while right < width && m[top * width + right] == pos {
            right += 1;
        }
        let mut bottom = top + 1;
        while bottom < self.height && m[bottom * width + left] == pos {
            bottom += 1;
        }
        Some(Rect {
            left,
            top,
            right,
            bottom,
        })
    }

    /// The column index of the cell at document position `pos` (its leftmost
    /// column). `None` if no cell starts at `pos`.
    pub fn col_count(&self, pos: usize) -> Option<usize> {
        self.map
            .iter()
            .position(|&p| p == pos)
            .map(|i| i % self.width)
    }

    /// The row index of the cell at document position `pos` (its topmost row).
    /// `None` if no cell starts at `pos`.
    pub fn row_count(&self, pos: usize) -> Option<usize> {
        self.map
            .iter()
            .position(|&p| p == pos)
            .map(|i| i / self.width)
    }

    /// The document position before the cell covering grid slot (`row`, `col`).
    /// `None` if the coordinates are out of range or the slot is a hole.
    pub fn cell_at(&self, row: usize, col: usize) -> Option<usize> {
        if row >= self.height || col >= self.width {
            return None;
        }
        let p = self.map[row * self.width + col];
        (p != UNSET).then_some(p)
    }

    /// The cell adjacent to the one at `pos` along `axis` in direction `dir`
    /// (`-1`/`+1`). `None` at the table edge (or if `pos` isn't a cell). Port of
    /// `TableMap.nextCell`.
    pub fn next_cell(&self, pos: usize, axis: Axis, dir: i32) -> Option<usize> {
        let Rect {
            left,
            top,
            right,
            bottom,
        } = self.find_cell(pos)?;
        let next = match axis {
            Axis::Horiz => {
                if dir < 0 {
                    if left == 0 {
                        return None;
                    }
                    self.map[top * self.width + (left - 1)]
                } else {
                    if right == self.width {
                        return None;
                    }
                    self.map[top * self.width + right]
                }
            }
            Axis::Vert => {
                if dir < 0 {
                    if top == 0 {
                        return None;
                    }
                    self.map[(top - 1) * self.width + left]
                } else {
                    if bottom == self.height {
                        return None;
                    }
                    self.map[bottom * self.width + left]
                }
            }
        };
        (next != UNSET).then_some(next)
    }

    /// The smallest grid rectangle covering both the cell at `a` and the cell at
    /// `b`. `None` if either is not a cell. Port of `TableMap.rectBetween`.
    pub fn rect_between(&self, a: usize, b: usize) -> Option<Rect> {
        let ra = self.find_cell(a)?;
        let rb = self.find_cell(b)?;
        Some(Rect {
            left: ra.left.min(rb.left),
            top: ra.top.min(rb.top),
            right: ra.right.max(rb.right),
            bottom: ra.bottom.max(rb.bottom),
        })
    }

    /// The document positions of every cell whose **origin** lies inside `rect`,
    /// deduplicated. A merged cell whose top-left pokes out of the rectangle's left
    /// or top edge is excluded (its origin is elsewhere). Port of
    /// `TableMap.cellsInRect`.
    pub fn cells_in_rect(&self, rect: Rect) -> Vec<usize> {
        let mut result = Vec::new();
        let mut seen = Vec::new();
        for row in rect.top..rect.bottom.min(self.height) {
            for col in rect.left..rect.right.min(self.width) {
                let index = row * self.width + col;
                let pos = self.map[index];
                if pos == UNSET || seen.contains(&pos) {
                    continue;
                }
                seen.push(pos);
                // Skip a cell that bleeds in from the left or top edge (it belongs
                // to a column/row outside the rectangle).
                let bleeds_left = col == rect.left && col > 0 && self.map[index - 1] == pos;
                let bleeds_top = row == rect.top && row > 0 && self.map[index - self.width] == pos;
                if bleeds_left || bleeds_top {
                    continue;
                }
                result.push(pos);
            }
        }
        result
    }

    /// The document position at which to place a cell occupying grid column `col`
    /// in row `row` — the position before the existing cell there, or, if that
    /// column (and everything to its right) is covered by rowspans from above, the
    /// row's inner end (before its close token). Port of `TableMap.positionAt`;
    /// used by column insertion.
    pub fn position_at(&self, row: usize, col: usize) -> usize {
        let row = row.min(self.height.saturating_sub(1));
        let row_start = self.row_starts[row];
        let mut index = col + row * self.width;
        let row_end_index = (row + 1) * self.width;
        // Skip slots claimed by a cell that started in an earlier row.
        while index < row_end_index && self.map[index] < row_start {
            index += 1;
        }
        if index >= self.map.len() || index == row_end_index || self.map[index] == UNSET {
            // Inner end of the row = position before its close token.
            self.row_starts[row + 1].saturating_sub(1)
        } else {
            self.map[index]
        }
    }

    /// The document position immediately before row `row` (its open token). `None`
    /// if `row >= height`.
    pub fn row_start(&self, row: usize) -> Option<usize> {
        (row <= self.height).then(|| self.row_starts[row])
    }
}

/// The number of columns in `table` (its grid width), honoring colspan/rowspan —
/// the value a CSS-grid view needs for `grid-template-columns`. Computable from the
/// table node alone (no document position required).
///
/// Capped so that `width × rows` is at most [`grid_slot_budget`] (#1176): a
/// table's width grows with the colspans that rowspans carry down, so without
/// the cap 1000 pasted rows of `colspan=1000 rowspan=65534` made a
/// 1,000,000-column grid, and the [`TableMap`] over it asked for 8 GB.
pub fn column_count(table: &Node) -> usize {
    let rows = table.child_count().max(1);
    let cap = (grid_slot_budget(table) / rows).max(1);
    // The cap is a `usize`, so the narrowing cannot truncate.
    find_width(table).min(cap as u64) as usize
}

/// The most slots a [`TableMap`] over `table` may hold: twice the number of
/// cells, and never less than [`GRID_SLOT_FLOOR`]. A rectangular table with
/// no spans fills `width × rows` slots with as many cells, so it always fits;
/// only a grid past 2^22 slots and past twice its cells (spans, ragged rows)
/// can be cut.
/// The map stays linear in the document whatever its spans claim.
pub fn grid_slot_budget(table: &Node) -> usize {
    let cells: usize = (0..table.child_count())
        .map(|r| table.child(r).child_count())
        .sum();
    cells.saturating_mul(2).max(GRID_SLOT_FLOOR)
}

/// The floor of [`grid_slot_budget`]: 2^22 slots (32 MB of map on a 64-bit
/// target), which any table may use whatever its cell count. It is the most a
/// table of fewer than 2^21 cells can make a [`TableMap`] allocate, however its
/// spans or ragged rows are shaped. It is sized so that a sparse ragged table
/// of modest size keeps every cell: a 1000-cell header over 1500 one-cell rows
/// (1.5 M slots for 2500 cells) is mapped whole, as Chrome draws it.
pub const GRID_SLOT_FLOOR: usize = 1 << 22;

/// How far a pasted row's spans may reach, in grid columns, counting the
/// columns that rowspans from the rows above carry into it: the HTML import
/// cuts a colspan to what is left, never below 1, and gives a cell that finds
/// its row full a rowspan of 1 (#1176). A row is therefore at most this wide
/// plus one column per cell that found it full. 1000 is Chrome's largest
/// `colspan`; Chrome itself has no limit on a row's width.
pub const MAX_IMPORTED_ROW_WIDTH: usize = 1000;

/// Read a span attribute (`colspan`/`rowspan`), clamped to a minimum of 1.
fn span_attr(cell: &Node, name: &str) -> usize {
    usize::try_from(cell.attrs().get_int(name).unwrap_or(1).max(1)).unwrap_or(usize::MAX)
}

/// The number of columns in `table` — the maximum row width once colspans are
/// summed and rowspans from earlier rows are carried down. Port of `findWidth`,
/// linear rather than quadratic in the rows: the columns a rowspan carries are
/// added to the rows below through a difference array (`ends[r]` holds what
/// stops being carried at row `r`). A span is the document's word and the
/// document can say `i64::MAX`, so the sums are `u64` over colspans cut to
/// `u32::MAX` — more than any [`column_count`], and no sum of them overflows,
/// on a 32-bit target (wasm) as on a 64-bit one.
fn find_width(table: &Node) -> u64 {
    const SPAN_CAP: u64 = u32::MAX as u64;
    let height = table.child_count();
    let mut ends = vec![0u64; height + 1];
    let mut carried = 0u64;
    let mut width = 0u64;
    for row in 0..height {
        carried -= ends[row];
        let row_node = table.child(row);
        let mut row_width = carried;
        let mut starts = 0u64;
        for i in 0..row_node.child_count() {
            let cell = row_node.child(i);
            let colspan = (span_attr(cell, "colspan") as u64).min(SPAN_CAP);
            row_width += colspan;
            let rowspan = span_attr(cell, "rowspan");
            if rowspan > 1 && row + 1 < height {
                starts += colspan;
                ends[row.saturating_add(rowspan).min(height)] += colspan;
            }
        }
        carried += starts;
        width = width.max(row_width);
    }
    width.max(1)
}

/// The grid rectangle of every cell of `table`, row by row and cell by cell in
/// document order: [`TableMap::cell_rect`] at the cell's first slot, or `None`
/// for a cell the bounded grid has no slot for (one past a capped width, or in
/// a row that rowspans from above already fill), which no table command treats
/// as a cell either. What the view places each cell by (#1182), so the grid a
/// host lays out is the grid the commands edit.
///
/// Linear in the map's slots (a binary search per slot run) plus the cells'
/// extents; the map is bounded by [`grid_slot_budget`].
pub fn cell_rects(table: &Node) -> Vec<Vec<Option<Rect>>> {
    // The map's positions are only compared with each other, so any start
    // does; 0 keeps them clear of the hole sentinel.
    let map = TableMap::compute(table, 0);
    let mut starts = Vec::new();
    let mut pos = 0usize;
    for r in 0..table.child_count() {
        let row = table.child(r);
        pos += 1;
        for i in 0..row.child_count() {
            starts.push(pos);
            pos += row.child(i).node_size();
        }
        pos += 1;
    }
    // Each cell's first slot in row-major order is its origin.
    let mut first = vec![UNSET; starts.len()];
    let mut last = UNSET;
    for (index, &p) in map.map().iter().enumerate() {
        if p == UNSET || p == last {
            continue;
        }
        last = p;
        if let Ok(k) = starts.binary_search(&p)
            && first[k] == UNSET
        {
            first[k] = index;
        }
    }
    let mut k = 0usize;
    (0..table.child_count())
        .map(|r| {
            (0..table.child(r).child_count())
                .map(|_| {
                    let rect = match first[k] {
                        UNSET => None,
                        index => map.cell_rect(index),
                    };
                    k += 1;
                    rect
                })
                .collect()
        })
        .collect()
}

/// Walk up from `r` to the nearest enclosing `table`, returning the table node and
/// the document position just inside it (where its first row begins). `None` if
/// `r` is not inside a table.
pub fn table_around(r: &ResolvedPos) -> Option<(Node, usize)> {
    for d in (0..=r.depth()).rev() {
        if is_table(r.node(d)) {
            return Some((r.node(d).clone(), r.start(d)));
        }
    }
    None
}

/// The [`TableMap`] of the table enclosing `pos`, with the table node and its
/// content-start position. `None` if `pos` is not inside a table.
pub fn map_around(doc: &Node, pos: Pos) -> Option<(TableMap, Node, usize)> {
    let r = doc.resolve(pos).ok()?;
    let (table, table_start) = table_around(&r)?;
    let map = TableMap::compute(&table, table_start);
    Some((map, table, table_start))
}

/// The document position immediately before the cell enclosing `r` (the anchor a
/// cell selection / node selection uses). `None` if `r` is not inside a cell. Port
/// of `cellAround`.
pub fn cell_around(r: &ResolvedPos) -> Option<usize> {
    for d in (0..=r.depth()).rev() {
        if is_cell(r.node(d)) {
            return r.before(d);
        }
    }
    None
}

/// The document position before the cell enclosing `pos`, or `None` if `pos` is not
/// inside a cell. The position helper for the pointer/keyboard layers.
pub fn cell_at_pos(doc: &Node, pos: Pos) -> Option<usize> {
    cell_around(&doc.resolve(pos).ok()?)
}

/// True if `a` and `b` resolve into the same table (compared by the table's
/// content-start position, unique per table). Used to gate a cross-cell drag into a
/// cell selection.
pub fn same_table(doc: &Node, a: Pos, b: Pos) -> bool {
    let start = |p: Pos| {
        doc.resolve(p)
            .ok()
            .and_then(|r| table_around(&r).map(|(_, s)| s))
    };
    matches!((start(a), start(b)), (Some(x), Some(y)) if x == y)
}

/// The position before the **next** (`dir > 0`) or **previous** (`dir < 0`) cell in
/// document order from the cell enclosing `pos` — the Tab/Shift-Tab target. Moves to
/// the adjacent cell in the same row, else wraps to the first/last cell of the next
/// non-empty row. `None` at the table's first/last cell (or if `pos` isn't in a
/// table). A faithful port of ProseMirror's `findNextCell`.
pub fn next_cell_in_table(doc: &Node, pos: Pos, dir: i32) -> Option<usize> {
    let r = doc.resolve(pos).ok()?;
    // The cell depth (and thus its row at `cd-1`, table at `cd-2`).
    let cd = (0..=r.depth()).rev().find(|&d| is_cell(r.node(d)))?;
    if cd < 2 {
        return None;
    }
    let row = r.node(cd - 1);
    let table = r.node(cd - 2);
    let cell_index = r.index(cd - 1);
    let row_index = r.index(cd - 2);

    if dir > 0 {
        if cell_index + 1 < row.child_count() {
            // The next cell starts right where this one ends.
            return r.after(cd);
        }
        // Last cell in the row → first cell of the next non-empty row.
        let mut row_start = r.after(cd - 1)?;
        for ri in (row_index + 1)..table.child_count() {
            let rnode = table.child(ri);
            if rnode.child_count() > 0 {
                return Some(row_start + 1);
            }
            row_start += rnode.node_size();
        }
        None
    } else {
        if cell_index > 0 {
            let before = row.child(cell_index - 1);
            return Some(r.before(cd)? - before.node_size());
        }
        // First cell in the row → last cell of the previous non-empty row.
        let mut row_end = r.before(cd - 1)?;
        for ri in (0..row_index).rev() {
            let rnode = table.child(ri);
            if let Some(last) = rnode.content().children().last() {
                return Some(row_end - 1 - last.node_size());
            }
            row_end -= rnode.node_size();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_table;
    use crate::model::{Attrs, Fragment};
    use crate::{AttrValue, Schema};

    fn sk() -> Schema {
        Schema::starter_kit()
    }

    /// Wrap a table node in a doc so it has a real document position. Returns
    /// `(doc, table_start)` where `table_start` is the table's content-start pos.
    fn doc_with_table(s: &Schema, table: Node) -> (Node, usize) {
        let doc = s
            .branch("doc", Fragment::from_node(table))
            .expect("doc holds a table");
        // table is doc.child(0); doc content starts at 0, table open token at 0,
        // so the table's content starts at pos 1.
        (doc, 1)
    }

    #[test]
    fn rectangular_grid_positions() {
        let s = sk();
        let table = build_table(&s, 2, 3).expect("3x2 table");
        let (_doc, table_start) = doc_with_table(&s, table.clone());
        let map = TableMap::compute(&table, table_start);
        assert_eq!(map.width(), 3);
        assert_eq!(map.height(), 2);
        // Every slot is a distinct cell; find_cell round-trips each.
        for row in 0..2 {
            for col in 0..3 {
                let pos = map.cell_at(row, col).expect("cell present");
                let rect = map.find_cell(pos).expect("cell rect");
                assert_eq!(
                    rect,
                    Rect {
                        left: col,
                        top: row,
                        right: col + 1,
                        bottom: row + 1
                    }
                );
                assert_eq!(map.col_count(pos), Some(col));
            }
        }
    }

    #[test]
    fn next_cell_walks_and_stops_at_edges() {
        let s = sk();
        let table = build_table(&s, 2, 2).unwrap();
        let map = TableMap::compute(&table, 1);
        let c00 = map.cell_at(0, 0).unwrap();
        let c01 = map.cell_at(0, 1).unwrap();
        let c10 = map.cell_at(1, 0).unwrap();
        assert_eq!(map.next_cell(c00, Axis::Horiz, 1), Some(c01));
        assert_eq!(map.next_cell(c00, Axis::Horiz, -1), None); // left edge
        assert_eq!(map.next_cell(c00, Axis::Vert, 1), Some(c10));
        assert_eq!(map.next_cell(c00, Axis::Vert, -1), None); // top edge
        assert_eq!(map.next_cell(c01, Axis::Horiz, 1), None); // right edge
    }

    /// A row whose first cell has colspan=2: that cell fills slots (0,0) and (0,1);
    /// width is 2, and find_cell reports the span.
    #[test]
    fn colspan_fills_multiple_slots() {
        let s = sk();
        let para = || {
            s.create_node("paragraph", Attrs::new(), Fragment::empty())
                .unwrap()
        };
        let wide = s
            .create_node(
                "table_cell",
                Attrs::from_iter([("colspan", AttrValue::Int(2))]),
                Fragment::from_node(para()),
            )
            .unwrap();
        let row0 = s
            .create_node("table_row", Attrs::new(), Fragment::from_node(wide))
            .unwrap();
        let c1 = s
            .create_node("table_cell", Attrs::new(), Fragment::from_node(para()))
            .unwrap();
        let c2 = s
            .create_node("table_cell", Attrs::new(), Fragment::from_node(para()))
            .unwrap();
        let row1 = s
            .create_node(
                "table_row",
                Attrs::new(),
                Fragment::from_children(vec![c1, c2]),
            )
            .unwrap();
        let table = s
            .create_node(
                "table",
                Attrs::new(),
                Fragment::from_children(vec![row0, row1]),
            )
            .unwrap();
        let map = TableMap::compute(&table, 1);
        assert_eq!(map.width(), 2);
        assert_eq!(map.height(), 2);
        let wide_pos = map.cell_at(0, 0).unwrap();
        assert_eq!(map.cell_at(0, 1), Some(wide_pos), "colspan covers (0,1)");
        let rect = map.find_cell(wide_pos).unwrap();
        assert_eq!(
            rect,
            Rect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 1
            }
        );
    }

    /// A cell with rowspan=2 in column 0 carries down into the next row; the second
    /// row then has only one explicit cell (in column 1).
    #[test]
    fn rowspan_carries_into_next_row() {
        let s = sk();
        let para = || {
            s.create_node("paragraph", Attrs::new(), Fragment::empty())
                .unwrap()
        };
        let tall = s
            .create_node(
                "table_cell",
                Attrs::from_iter([("rowspan", AttrValue::Int(2))]),
                Fragment::from_node(para()),
            )
            .unwrap();
        let top_right = s
            .create_node("table_cell", Attrs::new(), Fragment::from_node(para()))
            .unwrap();
        let row0 = s
            .create_node(
                "table_row",
                Attrs::new(),
                Fragment::from_children(vec![tall, top_right]),
            )
            .unwrap();
        let bot_right = s
            .create_node("table_cell", Attrs::new(), Fragment::from_node(para()))
            .unwrap();
        let row1 = s
            .create_node("table_row", Attrs::new(), Fragment::from_node(bot_right))
            .unwrap();
        let table = s
            .create_node(
                "table",
                Attrs::new(),
                Fragment::from_children(vec![row0, row1]),
            )
            .unwrap();
        let map = TableMap::compute(&table, 1);
        assert_eq!(map.width(), 2);
        assert_eq!(map.height(), 2);
        let tall_pos = map.cell_at(0, 0).unwrap();
        assert_eq!(map.cell_at(1, 0), Some(tall_pos), "rowspan covers (1,0)");
        let rect = map.find_cell(tall_pos).unwrap();
        assert_eq!(
            rect,
            Rect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 2
            }
        );
        // The bottom-right cell sits at (1,1) and is distinct.
        let br = map.cell_at(1, 1).unwrap();
        assert_ne!(br, tall_pos);
    }

    #[test]
    fn rect_between_and_cells_in_rect() {
        let s = sk();
        let table = build_table(&s, 2, 2).unwrap();
        let map = TableMap::compute(&table, 1);
        let c00 = map.cell_at(0, 0).unwrap();
        let c11 = map.cell_at(1, 1).unwrap();
        let rect = map.rect_between(c00, c11).unwrap();
        assert_eq!(
            rect,
            Rect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2
            }
        );
        let cells = map.cells_in_rect(rect);
        assert_eq!(cells.len(), 4, "all four cells inside the rect");
    }

    #[test]
    fn next_cell_walks_document_order() {
        // 2x2 table in a doc. Tab from a cursor inside each cell visits the cells in
        // row-major order; Shift-Tab reverses; edges return None.
        let s = sk();
        let table = build_table(&s, 2, 2).unwrap();
        let (doc, table_start) = doc_with_table(&s, table.clone());
        let map = TableMap::compute(&table, table_start);
        let cells: Vec<usize> = (0..2)
            .flat_map(|row| (0..2).map(move |col| (row, col)))
            .map(|(r, c)| map.cell_at(r, c).unwrap())
            .collect();
        // A cursor inside a cell's paragraph is at cell_pos + 2 (cell open, p open).
        let inside = |cell_pos: usize| Pos(cell_pos + 2);
        // Forward from (0,0) → (0,1) → (1,0) → (1,1) → None.
        assert_eq!(
            next_cell_in_table(&doc, inside(cells[0]), 1),
            Some(cells[1])
        );
        assert_eq!(
            next_cell_in_table(&doc, inside(cells[1]), 1),
            Some(cells[2]),
            "wraps to next row"
        );
        assert_eq!(
            next_cell_in_table(&doc, inside(cells[2]), 1),
            Some(cells[3])
        );
        assert_eq!(
            next_cell_in_table(&doc, inside(cells[3]), 1),
            None,
            "last cell forward → None"
        );
        // Backward mirrors.
        assert_eq!(
            next_cell_in_table(&doc, inside(cells[3]), -1),
            Some(cells[2])
        );
        assert_eq!(
            next_cell_in_table(&doc, inside(cells[2]), -1),
            Some(cells[1]),
            "wraps to previous row"
        );
        assert_eq!(
            next_cell_in_table(&doc, inside(cells[0]), -1),
            None,
            "first cell backward → None"
        );
    }

    #[test]
    fn cell_and_table_around_resolve() {
        let s = sk();
        let table = build_table(&s, 1, 1).unwrap();
        let (doc, table_start) = doc_with_table(&s, table);
        // The single cell's paragraph content: doc 0[table 1[row 2[cell 3[p 4 ...
        // The cell starts at pos 2 (table_start=1, row open at 1, cell open at 2).
        let cell_pos = 2;
        // Resolve a position inside the cell's paragraph (pos 4 = inside the empty
        // paragraph's content boundary).
        let inside = doc.resolve(Pos(4)).expect("resolve inside cell");
        assert_eq!(cell_around(&inside), Some(cell_pos));
        let (t, ts) = table_around(&inside).expect("inside a table");
        assert!(is_table(&t));
        assert_eq!(ts, table_start);
    }
}

/// #1176: the grid a [`TableMap`] allocates is bounded, whatever the table's
/// `colspan`/`rowspan` attributes say. A table reaches the model by paste, by
/// `load_doc`, by an app's own transaction and by the table commands, and only
/// the paste route normalises spans, so the map has to hold the line itself.
#[cfg(test)]
mod grid_bound_tests {
    use super::*;
    use crate::model::{Attrs, Fragment};
    use crate::{AttrValue, Schema};

    /// The slot budget's floor: a map may always hold this many slots.
    const FLOOR: usize = 1 << 22;

    fn cell(s: &Schema, colspan: i64, rowspan: i64) -> Node {
        let p = s
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .unwrap();
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

    fn row(s: &Schema, cells: Vec<Node>) -> Node {
        s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
            .unwrap()
    }

    fn table(s: &Schema, rows: Vec<Node>) -> Node {
        s.create_node("table", Attrs::new(), Fragment::from_children(rows))
            .unwrap()
    }

    /// Build a table from `(colspan, rowspan)` pairs, row by row.
    fn spans_table(s: &Schema, rows: &[Vec<(i64, i64)>]) -> Node {
        let rows = rows
            .iter()
            .map(|r| row(s, r.iter().map(|&(c, h)| cell(s, c, h)).collect()))
            .collect();
        table(s, rows)
    }

    /// The document position before row `r`'s `i`th cell (the table's content
    /// starting at 1, as in every fixture here).
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

    /// The issue's staircase, built in the model rather than pasted (so the
    /// import's clamps do not apply): 1000 rows of one cell each,
    /// `colspan=1000 rowspan=65534`. Row k is 1000·(k+1) wide, so the uncapped
    /// map is 1000 × 1,000,000 slots (8 GB). The width is capped at
    /// `FLOOR / height` = 4194 and every cell keeps to its own row.
    ///
    /// Run it against an unbounded map only under `ulimit -v`.
    #[test]
    fn a_staircase_built_in_the_model_is_capped_at_the_slot_budget() {
        let s = Schema::starter_kit();
        let t = spans_table(&s, &vec![vec![(1000, 65534)]; 1000]);
        // Asked first: it allocates nothing, so an unbounded width fails here.
        assert_eq!(column_count(&t), FLOOR / 1000);
        let map = TableMap::compute(&t, 1);
        assert_eq!(map.width(), 4194);
        assert_eq!(map.height(), 1000);
        assert_eq!(map.map().len(), 4194 * 1000);
        let (r0, r3, r4, r5) = (
            cell_pos(&t, 0, 0),
            cell_pos(&t, 3, 0),
            cell_pos(&t, 4, 0),
            cell_pos(&t, 5, 0),
        );
        // Row 0's cell covers columns 0..1000 of every row.
        assert_eq!(map.cell_at(0, 999), Some(r0));
        assert_eq!(map.cell_at(999, 999), Some(r0));
        assert_eq!(map.cell_at(0, 1000), None, "row 0 has a hole past its cell");
        // Row 3's cell fits: columns 3000..4000 of rows 3.. only.
        assert_eq!(map.cell_at(2, 3000), None);
        assert_eq!(
            map.find_cell(r3),
            Some(Rect {
                left: 3000,
                top: 3,
                right: 4000,
                bottom: 1000
            })
        );
        // Row 4's cell starts at column 4000 and is cut at the grid's edge.
        assert_eq!(map.cell_at(4, 4193), Some(r4));
        assert_eq!(map.cell_at(999, 4193), Some(r4));
        assert_eq!(
            map.find_cell(r4),
            Some(Rect {
                left: 4000,
                top: 4,
                right: 4194,
                bottom: 1000
            })
        );
        // Row 5's cell would start at column 5000: it is off the grid.
        assert_eq!(map.find_cell(r5), None);
    }

    /// A row wider than the capped grid is cut at the row's end; it does not
    /// run on into the next row's slots.
    #[test]
    fn an_overwide_row_does_not_spill_into_the_next_row() {
        let s = Schema::starter_kit();
        let t = spans_table(&s, &[vec![(3_000_000, 1)], vec![(1, 1), (1, 1)]]);
        let width = FLOOR / 2;
        assert_eq!(column_count(&t), width);
        let map = TableMap::compute(&t, 1);
        assert_eq!(map.width(), width);
        assert_eq!(map.map().len(), 2 * width);
        let wide = cell_pos(&t, 0, 0);
        assert_eq!(map.cell_at(0, width - 1), Some(wide));
        assert_eq!(map.cell_at(1, 0), Some(cell_pos(&t, 1, 0)));
        assert_eq!(map.cell_at(1, 1), Some(cell_pos(&t, 1, 1)));
        assert_eq!(map.cell_at(1, 2), None);
    }

    /// An attribute no import would write (`i64::MAX`, from `load_doc` or an
    /// app's transaction) is bounded, not trusted, and does not overflow.
    #[test]
    fn a_span_of_i64_max_is_bounded_not_trusted() {
        let s = Schema::starter_kit();
        // Three in one row: their sum is past `usize::MAX`.
        let t = spans_table(
            &s,
            &[
                vec![(i64::MAX, i64::MAX), (i64::MAX, 1), (i64::MAX, 1)],
                vec![(1, 1)],
            ],
        );
        let width = FLOOR / 2;
        assert_eq!(column_count(&t), width);
        let map = TableMap::compute(&t, 1);
        assert_eq!(map.map().len(), 2 * width);
        let big = cell_pos(&t, 0, 0);
        assert_eq!(map.cell_at(1, 0), Some(big), "its rowspan covers row 1");
        assert_eq!(map.cell_at(1, width - 1), Some(big));
        // Row 0's other cells, and row 1's own, are pushed off the grid.
        assert_eq!(map.find_cell(cell_pos(&t, 0, 1)), None);
        assert_eq!(map.find_cell(cell_pos(&t, 1, 0)), None);
    }

    /// A plain rectangular table larger than the floor is never cut: the budget
    /// grows with the number of cells (twice it), so it is linear in the
    /// document and a table with no spans always fits.
    #[test]
    fn a_rectangular_table_past_the_floor_is_not_cut() {
        let s = Schema::starter_kit();
        let one = cell(&s, 1, 1);
        let r = row(&s, vec![one; 2000]);
        let t = table(&s, vec![r; 2100]);
        assert_eq!(column_count(&t), 2000);
        let map = TableMap::compute(&t, 1);
        assert_eq!(map.width(), 2000);
        assert_eq!(map.height(), 2100);
        assert!(map.map().len() > FLOOR, "the fixture is past the floor");
        assert!(map.map().iter().all(|&p| p != UNSET), "no hole, no cut");
    }

    /// The factor of two in the budget: 2100 rows of 1000 `colspan=2` cells
    /// are a rectangular 2000 × 2100 grid (past the floor) holding 2.1 M
    /// cells. Twice the cells is exactly the grid, so nothing may be cut.
    /// (From #1183's review: a budget of the cells alone passed every other
    /// fixture.)
    #[test]
    fn a_rectangular_table_of_colspan_2_cells_past_the_floor_is_not_cut() {
        let s = Schema::starter_kit();
        let r = row(&s, vec![cell(&s, 2, 1); 1000]);
        let t = table(&s, vec![r; 2100]);
        assert_eq!(column_count(&t), 2000);
        let map = TableMap::compute(&t, 1);
        assert_eq!(map.map().len(), 2000 * 2100);
        assert!(map.map().len() > FLOOR, "the fixture is past the floor");
        assert!(map.map().iter().all(|&p| p != UNSET), "no hole, no cut");
    }

    /// A sparse ragged table with a modest cell count — a 1000-cell header
    /// over 1499 rows of one cell — is a 1.5 M-slot grid for 2499 cells. The
    /// floor takes it whole, as the port before #1176 did and as Chrome draws
    /// it: every header cell keeps its slot (#1183's review, F3).
    #[test]
    fn a_sparse_ragged_table_under_the_floor_is_not_cut() {
        let s = Schema::starter_kit();
        let header = row(&s, vec![cell(&s, 1, 1); 1000]);
        let mut rows = vec![header];
        rows.extend(vec![row(&s, vec![cell(&s, 1, 1)]); 1499]);
        let t = table(&s, rows);
        assert_eq!(column_count(&t), 1000);
        let map = TableMap::compute(&t, 1);
        assert_eq!(map.map().len(), 1000 * 1500);
        assert_eq!(map.cell_at(0, 999), Some(cell_pos(&t, 0, 999)));
        assert!(map.find_cell(cell_pos(&t, 0, 999)).is_some());
        assert_eq!(map.cell_at(1499, 0), Some(cell_pos(&t, 1499, 0)));
    }

    // ── The port as it was before #1176, verbatim: the oracle for tables
    //    the budget does not touch. ──────────────────────────────────────────

    fn old_find_width(table: &Node) -> usize {
        let mut width = 0usize;
        let mut has_row_span = false;
        let height = table.child_count();
        for row in 0..height {
            let row_node = table.child(row);
            let mut row_width = 0usize;
            if has_row_span {
                for j in 0..row {
                    let prev = table.child(j);
                    for i in 0..prev.child_count() {
                        let cell = prev.child(i);
                        if j + span_attr(cell, "rowspan") > row {
                            row_width += span_attr(cell, "colspan");
                        }
                    }
                }
            }
            for i in 0..row_node.child_count() {
                let cell = row_node.child(i);
                row_width += span_attr(cell, "colspan");
                if span_attr(cell, "rowspan") > 1 {
                    has_row_span = true;
                }
            }
            width = width.max(row_width);
        }
        width.max(1)
    }

    fn old_map(table: &Node, table_start: usize) -> Vec<usize> {
        let height = table.child_count();
        let width = old_find_width(table);
        let mut map = vec![UNSET; width * height];
        let mut pos = table_start;
        let mut map_pos = 0usize;
        for row in 0..height {
            let row_node = table.child(row);
            pos += 1;
            let mut i = 0usize;
            loop {
                while map_pos < map.len() && map[map_pos] != UNSET {
                    map_pos += 1;
                }
                if i == row_node.child_count() {
                    break;
                }
                let cell = row_node.child(i);
                let colspan = span_attr(cell, "colspan");
                let rowspan = span_attr(cell, "rowspan");
                for h in 0..rowspan {
                    if row + h >= height {
                        break;
                    }
                    let start = map_pos + h * width;
                    for w in 0..colspan {
                        let idx = start + w;
                        if idx < map.len() && map[idx] == UNSET {
                            map[idx] = pos;
                        }
                    }
                }
                map_pos += colspan;
                pos += cell.node_size();
                i += 1;
            }
            map_pos = (row + 1) * width;
            pos += 1;
        }
        map
    }

    /// Under the budget nothing changes: width and grid agree with the old
    /// port on random ragged tables with spans (0 and negatives included,
    /// which read as 1), empty rows, and rowspans past the table's end.
    #[test]
    fn under_the_budget_the_grid_is_the_old_ports() {
        let s = Schema::starter_kit();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..3000 {
            let rows = 1 + next(7) as usize;
            let spec: Vec<Vec<(i64, i64)>> = (0..rows)
                .map(|_| {
                    (0..next(5))
                        .map(|_| (next(6) as i64 - 1, next(6) as i64 - 1))
                        .collect()
                })
                .collect();
            let t = spans_table(&s, &spec);
            let map = TableMap::compute(&t, 1);
            assert_eq!(column_count(&t), old_find_width(&t), "width of {spec:?}");
            assert_eq!(find_width(&t), old_find_width(&t) as u64, "{spec:?}");
            assert_eq!(map.width(), old_find_width(&t), "{spec:?}");
            assert_eq!(map.map(), old_map(&t, 1).as_slice(), "grid of {spec:?}");
        }
    }
}
