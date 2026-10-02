//! Tables in the projection: rows and columns with identities, cells keyed by them,
//! and a deterministic read that always yields a rectangular grid.
//!
//! ## Why not an array of rows of cells
//!
//! The model's table is `table > table_row+ > cell*`, each cell `block+`, with
//! `colspan`/`rowspan`. Projected as nested arrays (rows, each an array of cells) the
//! obvious concurrent edits break the grid: one peer adds a row while another adds a
//! column, and the new row has no cell in the new column; two peers each add a column
//! and every row grows by two cells, but the rows each peer added grow by one. The
//! CRDT converges, on a table whose rows are ragged, and a cell's column is just its
//! index in a row, so one missing cell shifts every cell after it into the wrong
//! column.
//!
//! ## The wire shape
//!
//! A table node carries neither `text` nor `content`: it carries two arrays naming its
//! columns and its rows, and every cell is keyed by the column it starts in.
//!
//! ```text
//! Node (Map):
//!   "type"  -> "table"
//!   "attrs" -> Map
//!   "cols"  -> Array<Map>          // the column order
//!     "id"      -> string         // unique, minted by the peer that added it
//!     "deleted" -> true           // present once the column is deleted (a tombstone)
//!   "rows"  -> Array<Map>          // the row order
//!     "id"      -> string
//!     "deleted" -> true
//!     "attrs"   -> Map            // the table_row's attrs
//!     "cells"   -> Map<column id, Cell>
//!   Cell (Map):                   // the cell whose top-left slot is (this row, that column)
//!     "type"    -> "table_cell" | "table_header_cell"
//!     "attrs"   -> Map            // as the model has them, colspan/rowspan included
//!     "content" -> Array<Node>    // the cell's blocks, any projected node
//!     "col_end" -> column id      // the last column it spans, when more than one
//!     "row_end" -> row id         // the last row it spans, when more than one
//! ```
//!
//! A cell's place is its `(row id, column id)` key, never an index, so a column
//! inserted by one peer and a row inserted by another meet at a key no one wrote, and
//! that slot is simply empty (below). A span names its last row and column by id, so a
//! column inserted inside a merged cell's span is inside it on every replica, which is
//! what `addColumn` does to a span locally too. Deleted rows and columns stay as
//! tombstones so that a span ending on one still has an order to resolve against.
//!
//! ## The read is total and rectangular ([`read_table`])
//!
//! The live columns, in order, and the live rows, in order, make a `height × width`
//! grid. Cells are placed in row-major order of their anchors; a cell's rectangle runs
//! from its anchor to the last live row and column at or before its `row_end` and
//! `col_end`. Every rule is a pure function of the converged CRDT, so every replica
//! reads the same table:
//!
//! 1. **The earlier anchor wins.** A cell whose anchor slot an earlier cell already
//!    covers is hidden (it stays in the CRDT, inert). A cell whose rectangle would run
//!    into an earlier one is clipped along its row to the slots still free; the rows
//!    below need no clip, because cells are placed in row-major order and so a slot
//!    below a free one is never taken yet. Two concurrent merges that overlap
//!    therefore converge on the earlier one whole and what is left of the other.
//! 2. **Every slot left uncovered gets a filler**: a 1×1 `table_cell` holding one empty
//!    paragraph, which exists in the model and not in the CRDT. That is the cell of a
//!    row one peer added in a column another peer added at the same time. A local edit
//!    that changes a filler writes it into the CRDT; one that leaves it alone leaves it
//!    virtual, so a filler never races a peer's edit of the same cell by accident.
//! 3. **A cell with no blocks reads as one empty paragraph** (two peers each deleting
//!    one of its two paragraphs). Typing into it inserts the paragraph.
//! 4. **A table with no live row or no live column is void** and reads as absent, like
//!    any emptied container (`projection::is_void`).
//! 5. **A table too large for what the CRDT stores is not built.** A row or column
//!    line costs about 20 bytes on the wire and a filler none, so `n` rows and `n`
//!    columns appended by a foreign writer would read as `n²` fillers (90 KB of
//!    update, 2.3 GB on every replica, measured). The read refuses more rows plus
//!    columns than `line_budget` (`max(2 × stored cells, LINE_FLOOR = 2^16)`, #1248)
//!    and more slots than the model's own `grid_slot_budget` allows (both checked
//!    before the grid is allocated), and more fillers than `max(stored cells, FILLER_FLOOR)`
//!    (2^16 — room for the fillers honest concurrent row and column adds make: a
//!    256 × 256 block of them; honest peers reach it by adding hundreds of rows on
//!    one replica and hundreds of columns on another at once). The model sees such a
//!    table as a **placeholder** — one empty cell, the table's attrs plus
//!    [`PLACEHOLDER_ATTR`] holding its yrs item id, so two are never equal — built only
//!    by the model's reads (`CollabDoc::to_doc` and the join gate, under
//!    [`placeholder_reads`], which also record it as an [`OversizedTable`]); every CRDT
//!    read a write depends on is strict.
//!
//!    **No diff ever runs against a placeholder.** While the shared document holds a
//!    table too large to read, `CollabSession::record_local` refuses every local
//!    change (`CollabError::OversizedTable`), and an editor refuses the edit itself, as
//!    read-only does, so the model never runs ahead of the CRDT: outbound is frozen,
//!    inbound keeps integrating. Every narrower rule was a route the next review found around — a
//!    position-paired middle, two equal placeholders matched by value, an app export
//!    (HTML, markdown, `DocNode`) loaded back, which cannot carry the mark — each of
//!    which deleted the real table for every peer with `Ok`. The cure is
//!    `CollabSession::delete_oversized_table` by id ([`CollabDoc::delete_table`] removes
//!    the map wherever it is, without reading it; an id not listed now is refused),
//!    or a peer deleting or shrinking the table. No backlog of frozen edits exists, so
//!    nothing is re-based and nothing is lost when it lifts (review round 5: a kept
//!    backlog was wiped by any inbound change, and its re-base turned a peer's delete
//!    into the delete of another block — #1263, #1264 for the #220 stall's own). [`read_model_table`] still refuses a marked table (an
//!    undo, a paste of a copy), so a placeholder is never written either. It was a
//!    poison (#196) at first, which no peer that had seen the table could cure: a
//!    poisoned session sends nothing.
//!
//! The model side is held to the same shape: a table whose cells do not tile a
//! rectangle exactly (a ragged row, overlapping spans, a span past the edge), or whose
//! spans declare more rows plus columns than `line_budget` allows for its cells (one
//! `colspan = 3_000_000` cell would write three million column lines, #1248), is
//! [`CollabError::Unsupported`]. The editor's own table commands never make one; a
//! pasted ragged table can, and stalls outbound until it is fixed or removed.
//!
//! ## Local edits ([`reconcile_table`])
//!
//! The editor hands the projection the table after a command, not the command. The
//! rows and columns of the table before (the CRDT's read, with ids) and after (the
//! model) are matched the way a child list is: a common leading and trailing run of
//! **unchanged** rows (columns) keeps its ids, the changed middle is paired by index,
//! and the rest is inserted (fresh ids) or deleted (tombstoned).
//!
//! Unchanged means the model edit kept it, which is a question of **identity**, not
//! value (#1240): a row is unchanged when the model's row node before and after is the
//! same `Rc`, and a column when, in every row kept on both sides, the same model cell
//! covers it at the same offset from its anchor. By value, equal rows were
//! interchangeable: in a fresh `insertTable`, whose rows are all empty, a `deleteRow`
//! on the first tombstoned the *last*, and a peer's concurrent typing there went with
//! it. A column added or deleted changes every row node, so the rows fall in the
//! middle and pair by index; a merge changes the master cell, so the columns it spans
//! fall in the middle and pair by index too. A row also matches a row that shares one
//! of its cells: deleting a row a rowspan starts in moves that cell one row down,
//! which rebuilds the row below, and that row is still the same row. In the column
//! check, a row whose cell there changed abstains rather than vetoes, for the same
//! reason with a colspan.
//!
//! Identity is used only where the before and after are **related** — they share a
//! row or a cell. A load while collaborating (`load_doc`/`load_html`) and the re-base
//! of a stalled outbound on the CRDT's read-back hand the projection two documents that
//! share nothing; there the match falls back to values — the rows' and columns'
//! **slots** with spans left out (the anchored cell's type, attrs and content, or
//! "covered") — and the model is still passed down, since a level below may be related.
//!
//! Then every anchor of the new grid is written at its key: unchanged cells are left
//! alone, changed ones reconciled in place (their blocks diffed like any child list,
//! by identity too, so a peer's typing in another paragraph of the cell merges), new
//! ones inserted, and cells that are no longer anchors (merged away) deleted.
//!
//! ## What concurrency can still lose
//!
//! Convergence holds for every interleaving; these are the edits that lose to another:
//!
//! * Typing in a row or column a peer deletes, or in a cell a peer merges away.
//! * Two peers changing the same **filler** at once: both write a cell at one key, and
//!   one of the two cells wins whole (a yrs map key is last-writer-wins).
//! * Two overlapping merges: the content the later one moved is in a cell the earlier
//!   one deleted.
//! * Deleting the row or column a merged cell is anchored in: the editor moves the cell
//!   to the next row or column, which the projection writes as a new cell there, so a
//!   peer's concurrent typing in it lands in the old one.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, Map, MapPrelim, MapRef, Out, ReadTxn, Transact,
    TransactionMut,
};

use rinch_editor_core::{AttrValue, Attrs, Node};

use rinch_editor_core::tables::GRID_SLOT_FLOOR;

use crate::error::{CollabError, Result};
use crate::projection::{
    ATTRS, BlockData, CONTENT, CollabDoc, Model, NodeData, TYPE, common_runs, insert_node,
    node_attrs, node_content, node_type, read_children, read_node, reconcile_attrs,
    reconcile_child_list, write_attrs,
};

/// The table node type, and its row type.
pub(crate) const TABLE: &str = "table";
const ROW: &str = "table_row";
/// The cell types a row may hold.
const CELL_TYPES: [&str; 2] = ["table_cell", "table_header_cell"];
/// The type of a filler cell, and of the paragraph in it.
const FILLER_CELL: &str = "table_cell";
const FILLER_BLOCK: &str = "paragraph";

const COLS: &str = "cols";
const ROWS: &str = "rows";
const CELLS: &str = "cells";
const ID: &str = "id";
const DELETED: &str = "deleted";
const COL_END: &str = "col_end";
const ROW_END: &str = "row_end";
const COLSPAN: &str = "colspan";
const ROWSPAN: &str = "rowspan";

/// A projectable table: its attrs, its grid width, and its rows of cells in model order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableData {
    pub attrs: Attrs,
    /// The number of grid columns (the width every row's slots add up to).
    pub width: usize,
    pub rows: Vec<RowData>,
}

/// One table row: its attrs and the cells anchored in it, left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowData {
    pub attrs: Attrs,
    pub cells: Vec<CellData>,
}

/// One cell: its type, its attrs (colspan/rowspan included, as the model holds them)
/// and its blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellData {
    pub type_name: String,
    pub attrs: Attrs,
    pub children: Vec<NodeData>,
}

/// A cell's place in the grid: the column of its anchor and its extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    col: usize,
    cs: usize,
    rs: usize,
}

/// Where every cell of a table sits: per row, per cell in model order.
struct Layout {
    width: usize,
    height: usize,
    cells: Vec<Vec<Placed>>,
}

fn span(attrs: &Attrs, key: &str) -> usize {
    usize::try_from(attrs.get_int(key).unwrap_or(1).max(1)).unwrap_or(usize::MAX)
}

/// How many rows plus columns a table may have for `cells` cells, on both sides
/// (#1248): `max(2 × cells, LINE_FLOOR)`. A rectangle of 1×1 cells has `w + h ≤
/// cells + 1`, so only spans can pass it; a line costs a CRDT map on the wire.
fn line_budget(cells: usize) -> usize {
    cells.saturating_mul(2).max(LINE_FLOOR)
}

/// The floor of [`line_budget`].
/// 2^16 lines is about 1.3 MB of row and column maps on the wire: it admits a cell
/// spanning thousands of columns that an app's own transaction made (a host holding
/// `colspan = 5000` must reach its guests whole, `colspan_cap_collab_guest`, #1214),
/// and refuses one of `colspan = 3_000_000`.
pub(crate) const LINE_FLOOR: usize = 1 << 16;

/// The attr that marks a [`placeholder`] in the model, holding the id of the CRDT
/// table it stands for ([`OversizedTable::id`]), so two placeholders are never equal
/// and the model's one can be found by it. No HTML, markdown or table command
/// produces it; the projection refuses to write a table carrying it.
pub(crate) const PLACEHOLDER_ATTR: &str = "rinch-collab-unreadable-table";

/// A table in the shared document too large to read (rule 5): the model holds a
/// [`placeholder`] for it, and while any exists, a session refuses every local change
/// (see `CollabSession::record_local`). The cure is deleting it by its `id`
/// (`CollabSession::delete_oversized_table`), or a peer shrinking or deleting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OversizedTable {
    /// The table's identity in the CRDT (its yrs item id, `"{client}:{clock}"`), the
    /// same on every replica; also the value of [`PLACEHOLDER_ATTR`] on its placeholder.
    pub id: String,
    /// What made it too large, for a message.
    pub detail: String,
}

/// The id [`OversizedTable::id`] names a CRDT node map by.
pub(crate) fn map_id(map: &MapRef) -> String {
    match AsRef::<yrs::branch::Branch>::as_ref(map).id() {
        yrs::branch::BranchID::Nested(id) => format!("{}:{}", id.client.get(), id.clock),
        yrs::branch::BranchID::Root(name) => name.to_string(),
    }
}

/// Whether `node` is a [`placeholder`].
pub(crate) fn is_placeholder(node: &Node) -> bool {
    node.type_name() == TABLE && node.attrs().get(PLACEHOLDER_ATTR).is_some()
}

/// How many filler cells a read may make for a table whose CRDT stores fewer cells
/// than that (see [`read_table`]): 2^16, about 36 MB of read and model at their cost
/// today, and a 256 × 256 table's worth of concurrently added rows and columns.
pub(crate) const FILLER_FLOOR: usize = 1 << 16;

/// A remote table too large to read (rule 5): the `Unsupported` error every strict read
/// of it fails with. The model's own read sees a [`placeholder`] instead.
fn over_budget(detail: String) -> CollabError {
    CollabError::unsupported(format!(
        "a table of {detail} is past the read's budget and is not built \
         (a filler cell costs no bytes on the wire, so the read bounds them)"
    ))
}

fn not_a_tiling(detail: &str) -> CollabError {
    CollabError::unsupported(format!(
        "a table whose cells do not tile a rectangle ({detail}) cannot be projected; \
         the editor's table commands never make one, a pasted ragged table can"
    ))
}

/// Lay out a table's cells on its grid, as `TableMap` does, and check they **tile** it
/// exactly: every slot covered by one cell, no span past an edge. The projection only
/// carries tables of that shape (see the module docs).
fn layout(t: &TableData) -> Result<Layout> {
    let (width, height) = (t.width, t.rows.len());
    if width == 0 || height == 0 {
        return Err(not_a_tiling("no columns or no rows"));
    }
    let mut taken = vec![false; width * height];
    let mut cells = Vec::with_capacity(height);
    for (r, row) in t.rows.iter().enumerate() {
        let mut col = 0usize;
        let mut placed = Vec::with_capacity(row.cells.len());
        for cell in &row.cells {
            while col < width && taken[r * width + col] {
                col += 1;
            }
            let (cs, rs) = (span(&cell.attrs, COLSPAN), span(&cell.attrs, ROWSPAN));
            if col >= width || cs > width - col || rs > height - r {
                return Err(not_a_tiling("a cell runs past the table's edge"));
            }
            for rr in r..r + rs {
                for c in col..col + cs {
                    let slot = &mut taken[rr * width + c];
                    if *slot {
                        return Err(not_a_tiling("two cells overlap"));
                    }
                    *slot = true;
                }
            }
            placed.push(Placed { col, cs, rs });
            col += cs;
        }
        cells.push(placed);
    }
    if taken.iter().any(|t| !t) {
        return Err(not_a_tiling("a row leaves a slot with no cell"));
    }
    Ok(Layout {
        width,
        height,
        cells,
    })
}

/// A copy of `attrs` without the span keys.
fn without_spans(attrs: &Attrs) -> Attrs {
    attrs.without(COLSPAN).without(ROWSPAN)
}

/// The empty paragraph a filler cell holds, and that an emptied cell reads as.
fn empty_block() -> NodeData {
    NodeData::Block(BlockData {
        type_name: FILLER_BLOCK.into(),
        attrs: Attrs::new(),
        text: String::new(),
        marks: Vec::new(),
    })
}

/// The filler cell for an uncovered slot.
fn filler() -> CellData {
    CellData {
        type_name: FILLER_CELL.into(),
        attrs: Attrs::new(),
        children: vec![empty_block()],
    }
}

// --- model → TableData ---------------------------------------------------------

/// Read a model `table` node into [`TableData`], validating that it is one the
/// projection carries: rows of `table_row`, cells of the two cell types, every cell
/// holding projectable blocks, and the cells tiling the grid exactly.
pub(crate) fn read_model_table(node: &Node) -> Result<TableData> {
    // A placeholder stands for a table this replica could not read (rule 5): writing
    // it would put one empty cell where the real table is. Only its deletion is a
    // change the projection can make, and a deletion never reads the node.
    if is_placeholder(node) {
        return Err(CollabError::unsupported(
            "a table too large to read from the shared document cannot be written \
             while collaborating",
        ));
    }
    // The grid a hostile paste can declare is bounded the way `TableMap`'s is: in
    // lines (#1248 — the projection writes one per row and column, so a single
    // `colspan = 3_000_000` cell would write three million) and in slots.
    let height = node.child_count();
    let width = rinch_editor_core::tables::column_count(node);
    let cells: usize = (0..height).map(|r| node.child(r).child_count()).sum();
    if width.saturating_add(height) > line_budget(cells) {
        return Err(not_a_tiling(
            "its spans declare more rows and columns than it has cells for",
        ));
    }
    if (width as u128) * (height as u128)
        > rinch_editor_core::tables::grid_slot_budget(node) as u128
    {
        return Err(not_a_tiling(
            "its spans declare more slots than it has cells for",
        ));
    }
    let mut rows = Vec::with_capacity(height);
    for i in 0..height {
        let row = node.child(i);
        if row.type_name() != ROW {
            return Err(CollabError::unsupported(format!(
                "a table holding a `{}` where a `{ROW}` belongs",
                row.type_name()
            )));
        }
        let mut cells = Vec::with_capacity(row.child_count());
        for k in 0..row.child_count() {
            let cell = row.child(k);
            if !CELL_TYPES.contains(&cell.type_name()) {
                return Err(CollabError::unsupported(format!(
                    "a table row holding a `{}`, which is not a table cell",
                    cell.type_name()
                )));
            }
            if cell.child_count() == 0 {
                return Err(CollabError::schema(format!(
                    "a `{}` with no blocks; a cell holds at least one",
                    cell.type_name()
                )));
            }
            let mut children = Vec::with_capacity(cell.child_count());
            for b in 0..cell.child_count() {
                children.push(read_node(cell.child(b))?);
            }
            cells.push(CellData {
                type_name: cell.type_name().to_string(),
                attrs: cell.attrs().clone(),
                children,
            });
        }
        rows.push(RowData {
            attrs: row.attrs().clone(),
            cells,
        });
    }
    let data = TableData {
        attrs: node.attrs().clone(),
        width,
        rows,
    };
    layout(&data)?;
    Ok(data)
}

// --- CRDT → TableData ------------------------------------------------------------

/// Whether a node map is a projected table (it carries the `rows` array).
pub(crate) fn is_table_map<T: ReadTxn>(txn: &T, node: &MapRef) -> bool {
    matches!(node.get(txn, ROWS), Some(Out::YArray(_)))
}

/// One row or column of the CRDT table, in its array.
#[derive(Debug, Clone)]
struct Line {
    raw: u32,
    id: String,
    deleted: bool,
    map: MapRef,
}

fn read_lines<T: ReadTxn>(txn: &T, list: &ArrayRef, what: &str) -> Result<Vec<Line>> {
    let mut out = Vec::with_capacity(list.len(txn) as usize);
    let mut seen = HashSet::new();
    // One sequential pass: `ArrayRef::get` walks the array from its start, so reading
    // each line by index cost O(lines²) — 70,000 lines took minutes.
    for (raw, line) in list.iter(txn).enumerate() {
        let raw = raw as u32;
        let Out::YMap(map) = line else {
            return Err(CollabError::schema(format!(
                "a table {what} that is not a map"
            )));
        };
        let Some(Out::Any(Any::String(id))) = map.get(txn, ID) else {
            return Err(CollabError::schema(format!("a table {what} with no id")));
        };
        // Ids are minted unique (client id and clock); a repeat could only come from a
        // foreign writer, and the later one is read as deleted so the read stays total.
        let deleted = matches!(map.get(txn, DELETED), Some(Out::Any(Any::Bool(true))))
            || !seen.insert(id.to_string());
        out.push(Line {
            raw,
            id: id.to_string(),
            deleted,
            map,
        });
    }
    Ok(out)
}

/// The live lines in order, and for every line (live or not) the index of the last
/// live line at or before it (`None` when no live line comes that early), keyed by id.
fn resolve_index(lines: &[Line]) -> (Vec<Line>, HashMap<String, Option<usize>>) {
    let mut live = Vec::new();
    let mut at_or_before = HashMap::with_capacity(lines.len());
    for line in lines {
        if !line.deleted {
            live.push(line.clone());
        }
        at_or_before
            .entry(line.id.clone())
            .or_insert(live.len().checked_sub(1));
    }
    (live, at_or_before)
}

/// A cell of the read grid: its place, and the CRDT map behind it (`None` for a
/// filler, which has none).
#[derive(Debug, Clone)]
pub(crate) struct ReadCell {
    col: usize,
    cs: usize,
    rs: usize,
    pub map: Option<MapRef>,
}

/// A table read back from the CRDT: the model's view of it ([`TableData`]) and the
/// ids and maps behind each row, column and cell, which the writes and the sticky-index
/// walk address.
pub(crate) struct TableRead {
    pub data: TableData,
    rows: Vec<Line>,
    cols: Vec<Line>,
    /// Per live row, its cells in model order.
    pub cells: Vec<Vec<ReadCell>>,
    rows_array: ArrayRef,
    cols_array: ArrayRef,
}

/// Read a table node map back as the model sees it — see the module docs for the
/// rules that make the grid rectangular whatever concurrent edits left. A table past
/// the read's budget (rule 5) is an `Unsupported` error.
pub(crate) fn read_table<T: ReadTxn>(txn: &T, node: &MapRef) -> Result<TableRead> {
    read_table_or_budget(txn, node)?.map_err(over_budget)
}

thread_local! {
    /// The tables read as a [`placeholder`] so far, while [`placeholder_reads`] is
    /// held (`None` outside it: a table past the read's budget is then an error).
    static PLACEHOLDER_READS: RefCell<Option<Vec<OversizedTable>>> = const { RefCell::new(None) };
}

/// While the returned guard lives, a table past the read's budget reads as a
/// [`placeholder`] (rule 5), and is recorded. Held by the reads that build the model
/// (`CollabDoc::to_doc`, the join gate); every read a write depends on stays strict.
pub(crate) fn placeholder_reads() -> PlaceholderReads {
    PlaceholderReads(PLACEHOLDER_READS.with(|c| c.replace(Some(Vec::new()))))
}

/// The guard [`placeholder_reads`] returns; restores the previous setting on drop.
pub(crate) struct PlaceholderReads(Option<Vec<OversizedTable>>);

impl PlaceholderReads {
    /// End the scope and return the tables it read as placeholders, in read order.
    pub(crate) fn finish(mut self) -> Vec<OversizedTable> {
        let prev = self.0.take();
        PLACEHOLDER_READS
            .with(|c| c.replace(prev))
            .unwrap_or_default()
    }
}

impl Drop for PlaceholderReads {
    fn drop(&mut self) {
        let prev = self.0.take();
        PLACEHOLDER_READS.with(|c| *c.borrow_mut() = prev);
    }
}

/// What the model sees of a table past the read's budget: one empty cell, with the
/// table's own attrs and [`PLACEHOLDER_ATTR`]. A pure function of the CRDT, so every
/// replica reads the same, and marked, so no write path can put it in the CRDT.
fn placeholder(attrs: Attrs, id: &str) -> TableData {
    TableData {
        attrs: attrs.with(PLACEHOLDER_ATTR, AttrValue::Str(id.into())),
        width: 1,
        rows: vec![RowData {
            attrs: Attrs::new(),
            cells: vec![filler()],
        }],
    }
}

/// [`read_table`]'s data, or — inside [`placeholder_reads`] — a [`placeholder`] for a
/// table past the read's budget.
pub(crate) fn read_table_data<T: ReadTxn>(txn: &T, node: &MapRef) -> Result<TableData> {
    match read_table_or_budget(txn, node)? {
        Ok(read) => Ok(read.data),
        Err(detail) => PLACEHOLDER_READS.with(|c| match c.borrow_mut().as_mut() {
            Some(read) => {
                let id = map_id(node);
                let data = placeholder(node_attrs(txn, node), &id);
                read.push(OversizedTable { id, detail });
                Ok(data)
            }
            None => Err(over_budget(detail)),
        }),
    }
}

/// [`read_table`], with a table past the read's budget as `Ok(Err(detail))`.
fn read_table_or_budget<T: ReadTxn>(
    txn: &T,
    node: &MapRef,
) -> Result<std::result::Result<TableRead, String>> {
    let (Some(Out::YArray(rows_array)), Some(Out::YArray(cols_array))) =
        (node.get(txn, ROWS), node.get(txn, COLS))
    else {
        return Err(CollabError::schema(
            "a table without its `rows` and `cols` arrays",
        ));
    };
    let attrs = node_attrs(txn, node);
    let (cols, col_index) = resolve_index(&read_lines(txn, &cols_array, "column")?);
    let (rows, row_index) = resolve_index(&read_lines(txn, &rows_array, "row")?);
    let (width, height) = (cols.len(), rows.len());

    // Every live row's cell map, and how many cells the CRDT stores in them: what the
    // budgets below measure against.
    let row_maps: Vec<Option<MapRef>> = rows
        .iter()
        .map(|row| match row.map.get(txn, CELLS) {
            Some(Out::YMap(m)) => Some(m),
            _ => None,
        })
        .collect();
    let stored: usize = row_maps.iter().flatten().map(|m| m.len(txn) as usize).sum();
    // The line budget (#1248) and the slot budget, before anything is allocated: the
    // model's own (`line_budget`, `grid_slot_budget`), so the read never yields a table
    // the model would refuse to project an edit of.
    if width.saturating_add(height) > line_budget(stored) {
        return Ok(Err(format!(
            "{width} columns and {height} rows for {stored} stored cells"
        )));
    }
    let slots = width as u128 * height as u128;
    if slots > stored.saturating_mul(2).max(GRID_SLOT_FLOOR) as u128 {
        return Ok(Err(format!(
            "{width}×{height} slots for {stored} stored cells"
        )));
    }
    let slots = slots as usize;
    let col_pos: HashMap<&str, usize> = cols
        .iter()
        .enumerate()
        .map(|(j, c)| (c.id.as_str(), j))
        .collect();

    let mut cells: Vec<Vec<ReadCell>> = vec![Vec::new(); height];
    let mut row_cells_data: Vec<Vec<(usize, CellData)>> = vec![Vec::new(); height];
    let mut taken = vec![false; slots];
    let mut covered = 0usize;
    for (i, cell_map) in row_maps.iter().enumerate() {
        let Some(cell_map) = cell_map else {
            continue;
        };
        // This row's anchors in a live column, left to right: walking the stored cells
        // rather than every column keeps the read proportional to what the CRDT holds.
        let mut anchors: Vec<(usize, MapRef)> = cell_map
            .iter(txn)
            .filter_map(|(key, value)| match (col_pos.get(key), value) {
                (Some(&j), Out::YMap(cell)) => Some((j, cell)),
                _ => None,
            })
            .collect();
        anchors.sort_by_key(|(j, _)| *j);
        for (j, cell) in anchors {
            if taken[i * width + j] {
                continue;
            }
            let end = |key: &str, index: &HashMap<String, Option<usize>>, from: usize| match cell
                .get(txn, key)
            {
                Some(Out::Any(Any::String(id))) => match index.get(&*id) {
                    Some(Some(e)) => (*e).max(from),
                    _ => from,
                },
                _ => from,
            };
            let (je, ie) = (end(COL_END, &col_index, j), end(ROW_END, &row_index, i));
            // Clip along the row to the slots still free. No clip downwards is needed:
            // cells are placed in row-major order, so a slot below one of these that an
            // earlier cell covers is in a rectangle that also covers the slot above it
            // in this row, where the clip has already stopped.
            let mut jx = j;
            while jx < je && !taken[i * width + jx + 1] {
                jx += 1;
            }
            for r in i..=ie {
                for c in j..=jx {
                    debug_assert!(!taken[r * width + c], "a slot below the row clip is free");
                    taken[r * width + c] = true;
                }
            }
            let (cs, rs) = (jx - j + 1, ie - i + 1);
            covered += cs * rs;
            row_cells_data[i].push((j, read_cell(txn, &cell, cs, rs)?));
            cells[i].push(ReadCell {
                col: j,
                cs,
                rs,
                map: Some(cell),
            });
        }
    }
    // The filler budget: a filler costs no wire bytes (a line costs about 20, and
    // `n` rows and `n` columns make `n²` slots), so it is bounded by what the CRDT
    // stores, with a floor for small tables that honest concurrent row and column
    // adds can fill (each pair of a new row and a new column makes one filler).
    let fillers = slots - covered;
    if fillers > stored.max(FILLER_FLOOR) {
        return Ok(Err(format!(
            "{fillers} filler cells for {stored} stored cells in a {width}×{height} grid"
        )));
    }
    // Fillers for the slots no cell covers.
    for i in 0..height {
        for j in 0..width {
            if !taken[i * width + j] {
                row_cells_data[i].push((j, filler()));
                cells[i].push(ReadCell {
                    col: j,
                    cs: 1,
                    rs: 1,
                    map: None,
                });
            }
        }
        row_cells_data[i].sort_by_key(|(j, _)| *j);
        cells[i].sort_by_key(|c| c.col);
    }
    let data = TableData {
        attrs,
        width,
        rows: rows
            .iter()
            .zip(row_cells_data)
            .map(|(row, cells)| RowData {
                attrs: node_attrs(txn, &row.map),
                cells: cells.into_iter().map(|(_, c)| c).collect(),
            })
            .collect(),
    };
    Ok(Ok(TableRead {
        data,
        rows,
        cols,
        cells,
        rows_array,
        cols_array,
    }))
}

/// The CRDT map behind every cell of a table node, per row and per cell in model
/// order (`None` for a filler, which has none): how the sticky-index walk crosses a
/// table, whose rows and cells are not content arrays.
pub(crate) fn cell_maps<T: ReadTxn>(txn: &T, node: &MapRef) -> Result<Vec<Vec<Option<MapRef>>>> {
    Ok(read_table(txn, node)?
        .cells
        .into_iter()
        .map(|row| row.into_iter().map(|c| c.map).collect())
        .collect())
}

/// One cell map read back, with the extent the read gave it written into its span
/// attrs wherever that differs from what the attrs say.
fn read_cell<T: ReadTxn>(txn: &T, cell: &MapRef, cs: usize, rs: usize) -> Result<CellData> {
    let type_name =
        node_type(txn, cell).ok_or_else(|| CollabError::schema("a cell with no type"))?;
    if !CELL_TYPES.contains(&type_name.as_str()) {
        return Err(CollabError::unsupported(format!(
            "a `{type_name}` keyed as a table cell in the CRDT"
        )));
    }
    let mut attrs = node_attrs(txn, cell);
    for (key, extent) in [(COLSPAN, cs), (ROWSPAN, rs)] {
        if span(&attrs, key) != extent {
            attrs = attrs.with(key, AttrValue::Int(extent as i64));
        }
    }
    let mut children: Vec<NodeData> = match node_content(txn, cell) {
        Some(content) => read_children(txn, &content)?
            .into_iter()
            .map(|(_, nd)| nd)
            .collect(),
        None => return Err(CollabError::schema("a cell with no `content` array")),
    };
    if children.is_empty() {
        children.push(empty_block());
    }
    Ok(CellData {
        type_name,
        attrs,
        children,
    })
}

/// Whether a table node map is void: no live row, or no live column. Read off the
/// tombstones alone.
pub(crate) fn table_map_is_void<T: ReadTxn>(txn: &T, node: &MapRef) -> bool {
    let live = |key: &str| match node.get(txn, key) {
        Some(Out::YArray(list)) => list.iter(txn).any(|line| match line {
            Out::YMap(m) => !matches!(m.get(txn, DELETED), Some(Out::Any(Any::Bool(true)))),
            _ => false,
        }),
        _ => false,
    };
    !(live(ROWS) && live(COLS))
}

// --- writes ----------------------------------------------------------------------

/// A row or column id no other peer can mint: this replica's client id and its clock
/// at the moment, which every insert advances (so mint one, then insert, then mint the
/// next).
fn fresh_id(txn: &TransactionMut) -> String {
    let client = txn.doc().client_id();
    format!("{:x}.{}", client.get(), txn.state_vector().get(&client))
}

fn insert_line(
    txn: &mut TransactionMut,
    list: &ArrayRef,
    at: u32,
    row: Option<&Attrs>,
) -> (String, MapRef) {
    let id = fresh_id(txn);
    let map = list.insert(txn, at, MapPrelim::default());
    map.insert(txn, ID, Any::String(id.as_str().into()));
    if let Some(attrs) = row {
        let attrs_obj = map.insert(txn, ATTRS, MapPrelim::default());
        write_attrs(txn, &attrs_obj, attrs);
        map.insert(txn, CELLS, MapPrelim::default());
    }
    (id, map)
}

fn write_cell(
    txn: &mut TransactionMut,
    cells: &MapRef,
    key: &str,
    cell: &CellData,
    ends: (Option<&str>, Option<&str>),
) -> Result<()> {
    let map = cells.insert(txn, key, MapPrelim::default());
    map.insert(txn, TYPE, Any::String(cell.type_name.as_str().into()));
    let attrs_obj = map.insert(txn, ATTRS, MapPrelim::default());
    write_attrs(txn, &attrs_obj, &cell.attrs);
    let content = map.insert(txn, CONTENT, ArrayPrelim::default());
    for (i, child) in cell.children.iter().enumerate() {
        insert_node(txn, &content, i as u32, child)?;
    }
    set_ends(txn, &map, ends);
    Ok(())
}

/// Write a cell's `col_end`/`row_end` where they differ from what it holds.
fn set_ends(
    txn: &mut TransactionMut,
    map: &MapRef,
    (col_end, row_end): (Option<&str>, Option<&str>),
) {
    for (key, want) in [(COL_END, col_end), (ROW_END, row_end)] {
        let have = match map.get(txn, key) {
            Some(Out::Any(Any::String(s))) => Some(s.to_string()),
            _ => None,
        };
        match want {
            Some(id) if have.as_deref() != Some(id) => {
                map.insert(txn, key, Any::String(id.into()));
            }
            None if have.is_some() => {
                map.remove(txn, key);
            }
            _ => {}
        }
    }
}

/// Write a fresh table into its (already-inserted, typed) node map: new ids for every
/// row and column, a cell at every anchor.
pub(crate) fn write_table(txn: &mut TransactionMut, node: &MapRef, t: &TableData) -> Result<()> {
    let grid = layout(t)?;
    let cols_array = node.insert(txn, COLS, ArrayPrelim::default());
    let rows_array = node.insert(txn, ROWS, ArrayPrelim::default());
    let mut col_ids = Vec::with_capacity(grid.width);
    for j in 0..grid.width {
        col_ids.push(insert_line(txn, &cols_array, j as u32, None).0);
    }
    let mut row_ids = Vec::with_capacity(grid.height);
    let mut row_maps = Vec::with_capacity(grid.height);
    for (i, row) in t.rows.iter().enumerate() {
        let (id, map) = insert_line(txn, &rows_array, i as u32, Some(&row.attrs));
        row_ids.push(id);
        row_maps.push(map);
    }
    for (i, row) in t.rows.iter().enumerate() {
        let Some(Out::YMap(cells)) = row_maps[i].get(txn, CELLS) else {
            return Err(CollabError::schema("a fresh row without its cells map"));
        };
        for (cell, p) in row.cells.iter().zip(&grid.cells[i]) {
            let ends = ends_of(*p, i, &col_ids, &row_ids);
            write_cell(
                txn,
                &cells,
                &col_ids[p.col],
                cell,
                (ends.0.as_deref(), ends.1.as_deref()),
            )?;
        }
    }
    Ok(())
}

/// The `col_end`/`row_end` ids a cell placed at `p` in row `i` names: its last column
/// and row, when it spans more than one.
fn ends_of(
    p: Placed,
    i: usize,
    col_ids: &[String],
    row_ids: &[String],
) -> (Option<String>, Option<String>) {
    (
        (p.cs > 1).then(|| col_ids[p.col + p.cs - 1].clone()),
        (p.rs > 1).then(|| row_ids[i + p.rs - 1].clone()),
    )
}

/// A slot's signature for matching rows and columns: the anchored cell without its
/// spans, or "covered".
#[derive(PartialEq)]
enum Slot<'a> {
    Anchor(&'a str, Attrs, &'a [NodeData]),
    Covered,
}

fn slots<'a>(t: &'a TableData, grid: &Layout) -> Vec<Vec<Slot<'a>>> {
    let mut out: Vec<Vec<Slot<'a>>> = (0..grid.height)
        .map(|_| (0..grid.width).map(|_| Slot::Covered).collect())
        .collect();
    for (r, row) in t.rows.iter().enumerate() {
        for (cell, p) in row.cells.iter().zip(&grid.cells[r]) {
            out[r][p.col] =
                Slot::Anchor(&cell.type_name, without_spans(&cell.attrs), &cell.children);
        }
    }
    out
}

/// Match a sequence before and after an edit, the way a child list is diffed: a
/// common leading and trailing run under `same`, the changed middle paired by index.
/// Returns, for every `after` index, the `before` index it keeps (`None` for a new
/// one), and the `before` indices no `after` index keeps.
fn match_lines(
    bn: usize,
    an: usize,
    same: impl Fn(usize, usize) -> bool,
) -> (Vec<Option<usize>>, Vec<usize>) {
    let (prefix, suffix) = common_runs(bn, an, same);
    let (bm, am) = (bn - prefix - suffix, an - prefix - suffix);
    let common = bm.min(am);
    let mut keep = Vec::with_capacity(an);
    for a in 0..an {
        keep.push(if a < prefix + common {
            Some(a)
        } else if a < prefix + am {
            None
        } else {
            Some(a + bn - an)
        });
    }
    (keep, (prefix + common..prefix + bm).collect())
}

/// For every slot of a grid, the model cell covering it and the column that cell is
/// anchored in: a column's identity, slot by slot ([`reconcile_table`]).
fn cover<'a>(
    table: &'a Node,
    cells: &[Vec<Placed>],
    width: usize,
) -> Vec<Vec<Option<(&'a Node, usize)>>> {
    let mut out = vec![vec![None; width]; cells.len()];
    for (r, row) in cells.iter().enumerate() {
        for (k, p) in row.iter().enumerate() {
            let cell = table.child(r).child(k);
            for slots in out.iter_mut().skip(r).take(p.rs) {
                for slot in &mut slots[p.col..p.col + p.cs] {
                    *slot = Some((cell, p.col));
                }
            }
        }
    }
    out
}

/// Whether the edit kept any cell of the table: a before and after with nothing in
/// common (a load, a re-base on the CRDT's read-back) carries no identity.
fn shares_a_cell(b: &Node, a: &Node) -> bool {
    (0..a.child_count()).any(|i| {
        let ar = a.child(i);
        (0..ar.child_count()).any(|j| {
            let c = ar.child(j);
            (0..b.child_count()).any(|k| {
                let br = b.child(k);
                br.same_ref(ar) || (0..br.child_count()).any(|l| br.child(l).same_ref(c))
            })
        })
    })
}

/// Whether a model table's rows and cells line up with a grid's placed cells.
fn lines_up(table: &Node, cells: &[Vec<Placed>]) -> bool {
    table.child_count() == cells.len()
        && cells
            .iter()
            .enumerate()
            .all(|(r, row)| table.child(r).child_count() == row.len())
}

/// Bring the CRDT's lines (rows or columns) in step with `keep`: tombstone the dropped
/// ones and insert the new ones right before the next kept line (after any tombstones
/// in between, so a span ending on one of those does not grow over the new line).
/// Returns every after line's id and map.
fn write_lines(
    txn: &mut TransactionMut,
    list: &ArrayRef,
    before: &[Line],
    keep: &[Option<usize>],
    dropped: &[usize],
    row_attrs: Option<&[Attrs]>,
) -> Vec<(String, MapRef)> {
    for &b in dropped {
        before[b].map.insert(txn, DELETED, Any::Bool(true));
    }
    let mut out: Vec<Option<(String, MapRef)>> = keep
        .iter()
        .map(|k| k.map(|b| (before[b].id.clone(), before[b].map.clone())))
        .collect();
    // Insert from the last new line backwards, so the raw index of every kept line
    // still to be used is unchanged by the inserts after it, and new lines that share
    // a place land in order (each goes in before the one inserted just now). The end
    // of the array is measured once, before any insert, for the same reason.
    let end = list.len(txn);
    let mut a = keep.len();
    while a > 0 {
        a -= 1;
        if keep[a].is_some() {
            continue;
        }
        let at = keep[a + 1..]
            .iter()
            .find_map(|k| k.map(|b| before[b].raw))
            .unwrap_or(end);
        out[a] = Some(insert_line(txn, list, at, row_attrs.map(|r| &r[a])));
    }
    out.into_iter()
        .map(|o| o.expect("every line is kept or new"))
        .collect()
}

/// Reconcile a table node map to `target` (see the module docs).
pub(crate) fn reconcile_table(
    txn: &mut TransactionMut,
    node: &MapRef,
    target: &TableData,
    model: Model<'_>,
    per_char: &BTreeSet<String>,
) -> Result<()> {
    let before = read_table(txn, node)?;
    if before.data == *target {
        return Ok(());
    }
    if before.data.attrs != target.attrs {
        reconcile_attrs(txn, node, &before.data.attrs, &target.attrs);
    }
    let grid = layout(target)?;
    let before_grid = Layout {
        width: before.data.width,
        height: before.data.rows.len(),
        cells: before
            .cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|c| Placed {
                        col: c.col,
                        cs: c.cs,
                        rs: c.rs,
                    })
                    .collect()
            })
            .collect(),
    };
    // Rows and columns are matched by model identity when the model's before and
    // after line up with the two grids (they always do: the model is the projection),
    // and by their slots' values otherwise (see the module docs).
    let model = model.filter(|(b, a)| lines_up(b, &before_grid.cells) && lines_up(a, &grid.cells));
    let identity = model.filter(|(b, a)| shares_a_cell(b, a));
    let (bn, an) = (before_grid.height, grid.height);
    let ((row_keep, row_drop), (col_keep, col_drop)) = match identity {
        Some((old, new)) => {
            // A row is kept when the edit kept its `Rc`.
            let rows = match_lines(bn, an, |b, a| {
                let (x, y) = (old.child(b), new.child(a));
                x.same_ref(y)
                    || (0..x.child_count())
                        .any(|i| (0..y.child_count()).any(|j| x.child(i).same_ref(y.child(j))))
            });
            // A column is kept when, in every row kept on both sides, the same model
            // cell covers it at the same offset from its anchor. (Every row of the
            // shorter side is paired, so some row always vouches. The offset is what
            // "the same column" means; no case is known where it decides a match the
            // cell alone would not — a kept cell keeps its span, so runs walked from
            // one end meet it at one offset — and dropping it fails no test.)
            let (bc, ac) = (
                cover(old, &before_grid.cells, before_grid.width),
                cover(new, &grid.cells, grid.width),
            );
            let kept: Vec<(usize, usize)> = rows
                .0
                .iter()
                .enumerate()
                .filter_map(|(i, k)| k.map(|k| (k, i)))
                .collect();
            let cols = match_lines(before_grid.width, grid.width, |b, a| {
                let mut yes = false;
                for &(k, i) in &kept {
                    // A row whose cell here changed abstains: it cannot say.
                    if let (Some((x, xc)), Some((y, yc))) = (bc[k][b], ac[i][a])
                        && x.same_ref(y)
                    {
                        if b - xc != a - yc {
                            return false;
                        }
                        yes = true;
                    }
                }
                yes
            });
            (rows, cols)
        }
        None => {
            let (b_slots, a_slots) = (slots(&before.data, &before_grid), slots(target, &grid));
            let rows = match_lines(bn, an, |b, a| b_slots[b] == a_slots[a]);
            let cols = match_lines(before_grid.width, grid.width, |b, a| {
                b_slots
                    .iter()
                    .map(|row| &row[b])
                    .eq(a_slots.iter().map(|row| &row[a]))
            });
            (rows, cols)
        }
    };

    let row_attrs: Vec<Attrs> = target.rows.iter().map(|r| r.attrs.clone()).collect();
    let cols = write_lines(
        txn,
        &before.cols_array,
        &before.cols,
        &col_keep,
        &col_drop,
        None,
    );
    let rows = write_lines(
        txn,
        &before.rows_array,
        &before.rows,
        &row_keep,
        &row_drop,
        Some(&row_attrs),
    );
    let col_ids: Vec<String> = cols.iter().map(|(id, _)| id.clone()).collect();
    let row_ids: Vec<String> = rows.iter().map(|(id, _)| id.clone()).collect();

    // The before anchors, by (row id, column id): what each one was, so an unchanged
    // cell is left alone and one that is no longer an anchor is deleted.
    let mut was: HashMap<(String, String), (&CellData, &ReadCell, Option<&Node>)> = HashMap::new();
    for (b, row) in before.cells.iter().enumerate() {
        for (k, cell) in row.iter().enumerate() {
            was.insert(
                (before.rows[b].id.clone(), before.cols[cell.col].id.clone()),
                (
                    &before.data.rows[b].cells[k],
                    cell,
                    model.map(|(old, _)| old.child(b).child(k)),
                ),
            );
        }
    }
    let mut anchors = HashSet::new();
    for (i, row) in target.rows.iter().enumerate() {
        // A kept row's attrs are reconciled; a new one was written with them.
        if row_keep[i].is_some() {
            let current = node_attrs(txn, &rows[i].1);
            if current != row.attrs {
                reconcile_attrs(txn, &rows[i].1, &current, &row.attrs);
            }
        }
        let cells_map = match rows[i].1.get(txn, CELLS) {
            Some(Out::YMap(m)) => m,
            _ => rows[i].1.insert(txn, CELLS, MapPrelim::default()),
        };
        for (k, (cell, p)) in row.cells.iter().zip(&grid.cells[i]).enumerate() {
            let key = (row_ids[i].clone(), col_ids[p.col].clone());
            let ends = ends_of(*p, i, &col_ids, &row_ids);
            let ends = (ends.0.as_deref(), ends.1.as_deref());
            let unchanged = match was.get(&key) {
                // A filler that is still exactly a filler stays virtual.
                Some((data, ReadCell { map: None, .. }, _)) => {
                    *data == cell && p.cs == 1 && p.rs == 1
                }
                Some((data, ReadCell { map: Some(m), .. }, _)) => {
                    *data == cell
                        && stored_ends(txn, m)
                            == (ends.0.map(str::to_string), ends.1.map(str::to_string))
                }
                None => false,
            };
            anchors.insert(key.clone());
            if unchanged {
                continue;
            }
            // The cell's blocks are matched by identity too, when this key held it before.
            let pair = match (was.get(&key), model) {
                (Some((_, _, Some(old))), Some((_, new))) => Some((*old, new.child(i).child(k))),
                _ => None,
            };
            match cells_map.get(txn, &key.1) {
                Some(Out::YMap(existing)) => {
                    reconcile_cell(txn, &existing, cell, ends, pair, per_char)?
                }
                _ => write_cell(txn, &cells_map, &key.1, cell, ends)?,
            }
        }
    }
    // Cells that were anchors and are not any more (merged away), in a row and a
    // column that are still there: delete them. One in a deleted row or column goes
    // with its tombstone.
    let live_rows: HashSet<&String> = row_ids.iter().collect();
    let live_cols: HashSet<&String> = col_ids.iter().collect();
    for (b, row) in before.cells.iter().enumerate() {
        let row_id = &before.rows[b].id;
        if !live_rows.contains(row_id) {
            continue;
        }
        for cell in row {
            let col_id = &before.cols[cell.col].id;
            if cell.map.is_none()
                || !live_cols.contains(col_id)
                || anchors.contains(&(row_id.clone(), col_id.clone()))
            {
                continue;
            }
            if let Some(Out::YMap(cells_map)) = before.rows[b].map.get(txn, CELLS) {
                cells_map.remove(txn, col_id);
            }
        }
    }
    Ok(())
}

fn stored_ends<T: ReadTxn>(txn: &T, map: &MapRef) -> (Option<String>, Option<String>) {
    let get = |key: &str| match map.get(txn, key) {
        Some(Out::Any(Any::String(s))) => Some(s.to_string()),
        _ => None,
    };
    (get(COL_END), get(ROW_END))
}

/// Reconcile one existing cell map to `target`: its type, its attrs per key, its blocks
/// as a child list (so a peer's edit in another of its paragraphs merges), its ends.
fn reconcile_cell(
    txn: &mut TransactionMut,
    map: &MapRef,
    target: &CellData,
    ends: (Option<&str>, Option<&str>),
    model: Model<'_>,
    per_char: &BTreeSet<String>,
) -> Result<()> {
    if node_type(txn, map).as_deref() != Some(target.type_name.as_str()) {
        map.insert(txn, TYPE, Any::String(target.type_name.as_str().into()));
    }
    let current = node_attrs(txn, map);
    if current != target.attrs {
        reconcile_attrs(txn, map, &current, &target.attrs);
    }
    let content = match node_content(txn, map) {
        Some(c) => c,
        None => map.insert(txn, CONTENT, ArrayPrelim::default()),
    };
    reconcile_child_list(txn, &content, &target.children, model, per_char)?;
    set_ends(txn, map, ends);
    Ok(())
}

// --- tables too large to read: the freeze and its cure -------------------------------

/// Lock the oversized-table list, recovering a poisoned lock (the list has no
/// invariant a panic could break).
pub(crate) fn lock_oversized(
    list: &std::sync::Mutex<Vec<OversizedTable>>,
) -> std::sync::MutexGuard<'_, Vec<OversizedTable>> {
    list.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The content array holding the table map named `id`, and its raw index there,
/// searching `list` and every container, table and cell below it. Walks the CRDT's
/// own shapes (rows, cell maps, content arrays) rather than the table read, so it
/// reaches a table inside a table too large to read too.
fn find_table<T: ReadTxn>(txn: &T, list: &ArrayRef, id: &str) -> Option<(ArrayRef, u32)> {
    for (raw, child) in list.iter(txn).enumerate() {
        let Out::YMap(map) = child else { continue };
        if is_table_map(txn, &map) {
            if map_id(&map) == id {
                return Some((list.clone(), raw as u32));
            }
            let Some(Out::YArray(rows)) = map.get(txn, ROWS) else {
                continue;
            };
            for row in rows.iter(txn) {
                let Out::YMap(row) = row else { continue };
                let Some(Out::YMap(cells)) = row.get(txn, CELLS) else {
                    continue;
                };
                for (_, cell) in cells.iter(txn) {
                    if let Out::YMap(cell) = cell
                        && let Some(content) = node_content(txn, &cell)
                        && let Some(found) = find_table(txn, &content, id)
                    {
                        return Some(found);
                    }
                }
            }
        } else if let Some(content) = node_content(txn, &map)
            && let Some(found) = find_table(txn, &content, id)
        {
            return Some(found);
        }
    }
    None
}

impl CollabDoc {
    /// The tables in the shared document too large to read, as the last model read
    /// found them (rule 5).
    pub fn oversized_tables(&self) -> Vec<OversizedTable> {
        lock_oversized(&self.oversized).clone()
    }

    /// Delete the table map named `id` (an [`OversizedTable::id`]) from wherever it is
    /// in the CRDT, without reading it. `false` when there is none (a peer deleted it
    /// first). The delete is a local change: it lands in the outbox for peers. The
    /// caller re-reads the model ([`CollabDoc::to_doc`]), which also refreshes
    /// [`CollabDoc::oversized_tables`].
    pub fn delete_table(&mut self, id: &str) -> bool {
        let found = {
            let txn = self.doc.transact();
            find_table(&txn, &self.content, id)
        };
        let Some((list, raw)) = found else {
            return false;
        };
        {
            let mut txn = self.doc.transact_mut();
            list.remove(&mut txn, raw);
        }
        // A container whose only child was the table is void now; at the top level the
        // diff must know (see `top_void`).
        self.top_void = crate::projection::holds_void(&self.doc.transact(), &self.content);
        true
    }
}
