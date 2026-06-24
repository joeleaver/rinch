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
    pub fn compute(table: &Node, table_start: usize) -> TableMap {
        let height = table.child_count();
        let width = find_width(table);
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
                        break; // overlong rowspan — clamp rather than write OOB
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
pub fn column_count(table: &Node) -> usize {
    find_width(table)
}

/// Read a span attribute (`colspan`/`rowspan`), clamped to a minimum of 1.
fn span_attr(cell: &Node, name: &str) -> usize {
    cell.attrs().get_int(name).unwrap_or(1).max(1) as usize
}

/// The number of columns in `table` — the maximum row width once colspans are
/// summed and rowspans from earlier rows are carried down. Port of `findWidth`.
fn find_width(table: &Node) -> usize {
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
