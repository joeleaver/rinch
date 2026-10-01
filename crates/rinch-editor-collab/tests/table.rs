//! Tables in the projected scope.
//!
//! A table projects with row and column identities and cells keyed by them (see
//! `src/table.rs` for the wire shape and the read rules). These tests pin the round
//! trip, every table command reaching a peer, the concurrent edits that break a naive
//! array-of-rows projection (a row and a column added at once, two columns added at
//! once, a row deleted while a peer types in it, merges that overlap), sticky
//! positions and the caret inside a cell, and a randomized convergence test over
//! several replicas with replayable seeds. Every converged table is asserted to
//! **tile** its grid: every slot covered by exactly one cell, no span past an edge.

use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabError, CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Mark, Node, Plugin, Pos, Schema, Selection, TableMap,
    Transaction, default_plugins,
};

// --- harness -------------------------------------------------------------------

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn schema() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}

fn para(s: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    s.branch("paragraph", content).unwrap()
}

fn branch(s: &Schema, type_name: &str, children: Vec<Node>) -> Node {
    s.branch(type_name, Fragment::from_children(children))
        .unwrap()
}

fn cell(s: &Schema, text: &str) -> Node {
    branch(s, "table_cell", vec![para(s, text)])
}

fn header(s: &Schema, text: &str) -> Node {
    branch(s, "table_header_cell", vec![para(s, text)])
}

fn spanning(s: &Schema, text: &str, colspan: i64, rowspan: i64) -> Node {
    s.create_node(
        "table_cell",
        Attrs::new()
            .with("colspan", AttrValue::Int(colspan))
            .with("rowspan", AttrValue::Int(rowspan)),
        Fragment::from_node(para(s, text)),
    )
    .unwrap()
}

/// A `rows × cols` table whose cells read `r{row}c{col}`.
fn grid(s: &Schema, rows: usize, cols: usize) -> Node {
    branch(
        s,
        "table",
        (0..rows)
            .map(|r| {
                branch(
                    s,
                    "table_row",
                    (0..cols).map(|c| cell(s, &format!("r{r}c{c}"))).collect(),
                )
            })
            .collect(),
    )
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    branch(s, "doc", blocks)
}

/// The model position of the first char of `needle`, searching every text node.
fn pos_of(doc: &Node, needle: &str) -> usize {
    fn walk(node: &Node, content_start: usize, needle: &str) -> Option<usize> {
        let mut at = content_start;
        for i in 0..node.child_count() {
            let child = node.child(i);
            if let Some(text) = child.text() {
                if let Some(byte) = text.find(needle) {
                    return Some(at + text[..byte].chars().count());
                }
            } else if let Some(found) = walk(child, at + 1, needle) {
                return Some(found);
            }
            at += child.node_size();
        }
        None
    }
    walk(doc, 0, needle).unwrap_or_else(|| panic!("`{needle}` is not in the document"))
}

/// The first table of `doc` and the position its content starts at.
fn first_table(doc: &Node) -> Option<(Node, usize)> {
    let mut at = 0;
    for i in 0..doc.child_count() {
        let child = doc.child(i);
        if child.type_name() == "table" {
            return Some((child.clone(), at + 1));
        }
        at += child.node_size();
    }
    None
}

/// The position just before the cell covering grid slot `(row, col)` of the first
/// table.
fn cell_pos(doc: &Node, row: usize, col: usize) -> usize {
    let (table, start) = first_table(doc).expect("a table");
    TableMap::compute(&table, start)
        .cell_at(row, col)
        .expect("a cell there")
}

/// The table's grid as text, one line per row, a cell's text in its anchor slot and
/// `·` in a slot a span covers.
fn grid_text(doc: &Node) -> String {
    let (table, _) = first_table(doc).expect("a table");
    let width = rinch_editor_core::tables::column_count(&table);
    let mut slots = vec![vec![String::new(); width]; table.child_count()];
    let mut taken = vec![vec![false; width]; table.child_count()];
    for r in 0..table.child_count() {
        let row = table.child(r);
        let mut c = 0;
        for k in 0..row.child_count() {
            while c < width && taken[r][c] {
                c += 1;
            }
            let cell = row.child(k);
            let cs = cell.attrs().get_int("colspan").unwrap_or(1) as usize;
            let rs = cell.attrs().get_int("rowspan").unwrap_or(1) as usize;
            for rr in r..r + rs {
                for cc in c..c + cs {
                    taken[rr][cc] = true;
                    slots[rr][cc] = "·".into();
                }
            }
            let mut text = String::new();
            for b in 0..cell.child_count() {
                if b > 0 {
                    text.push('/');
                }
                text.push_str(&inline_text(cell.child(b)));
            }
            slots[r][c] = if text.is_empty() { "_".into() } else { text };
            c += cs;
        }
    }
    slots
        .into_iter()
        .map(|row| row.join(" | "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn inline_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    (0..node.child_count())
        .map(|i| inline_text(node.child(i)))
        .collect()
}

/// Every node's children satisfy its content expression, and every table **tiles**
/// its grid: each slot covered by exactly one cell, no span past an edge.
fn assert_valid(node: &Node) {
    if node.is_text() {
        return;
    }
    let names: Vec<&str> = (0..node.child_count())
        .map(|i| node.child(i).type_name())
        .collect();
    assert!(
        node.is_textblock() || node.node_type().content_match().matches(&names),
        "<{}> holds {names:?}, which its schema does not allow",
        node.type_name()
    );
    if node.type_name() == "table" {
        assert_tiles(node);
    }
    for i in 0..node.child_count() {
        assert_valid(node.child(i));
    }
}

fn assert_tiles(table: &Node) {
    let width = rinch_editor_core::tables::column_count(table);
    let height = table.child_count();
    assert!(width > 0 && height > 0, "an empty table");
    let mut taken = vec![vec![false; width]; height];
    for r in 0..height {
        let row = table.child(r);
        let mut c = 0;
        for k in 0..row.child_count() {
            while c < width && taken[r][c] {
                c += 1;
            }
            let cell = row.child(k);
            let cs = cell.attrs().get_int("colspan").unwrap_or(1).max(1) as usize;
            let rs = cell.attrs().get_int("rowspan").unwrap_or(1).max(1) as usize;
            assert!(
                c + cs <= width && r + rs <= height,
                "a cell runs past the edge: {table:?}"
            );
            for row_taken in taken.iter_mut().skip(r).take(rs) {
                for slot in &mut row_taken[c..c + cs] {
                    assert!(!*slot, "two cells overlap: {table:?}");
                    *slot = true;
                }
            }
            c += cs;
        }
    }
    assert!(
        taken.iter().flatten().all(|t| *t),
        "a slot with no cell: {table:?}"
    );
}

/// One collaborating editor.
struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn host(schema: &Rc<Schema>, blocks: Vec<Node>, client_id: u64) -> Peer {
        let state = EditorState::create(schema.clone(), doc_of(schema, blocks), plugins());
        let session = session_with_client_id(&state, client_id).unwrap();
        Peer {
            schema: schema.clone(),
            state,
            session,
        }
    }

    fn join(&self, client_id: u64) -> Peer {
        let session = session_from_bytes_with_client_id(&self.session.snapshot(), client_id)
            .expect("join from snapshot");
        let doc = session.projected_doc(&self.schema).expect("project");
        Peer {
            schema: self.schema.clone(),
            state: EditorState::create(self.schema.clone(), doc, plugins()),
            session,
        }
    }

    fn commit(&mut self, next: EditorState) {
        self.session
            .record_local(&self.schema, &self.state.doc, &next.doc)
            .expect("the edit projects");
        self.state = next;
        self.assert_model_is_projection("after a local edit");
    }

    fn local(&mut self, f: impl FnOnce(&mut Transaction)) {
        let mut tr = self.state.tr();
        f(&mut tr);
        let next = self.state.apply(tr);
        self.commit(next);
    }

    /// Run `name` with `selection`; `false` when the command does not apply there.
    fn try_run(&mut self, selection: Selection, name: &str) -> bool {
        let mut tr = self.state.tr();
        tr.set_selection(selection);
        let placed = self.state.apply(tr);
        match placed.run(name) {
            Some(next) => {
                self.commit(next);
                true
            }
            None => false,
        }
    }

    /// Run `name` with the caret in the cell covering `(row, col)`.
    fn in_cell(&mut self, row: usize, col: usize, name: &str) {
        let at = cell_pos(&self.state.doc, row, col);
        let caret = Selection::near(&self.state.doc, Pos(at + 1), 1);
        assert!(
            self.try_run(caret, name),
            "`{name}` applies at ({row}, {col})"
        );
    }

    /// Select the cells from `(r0, c0)` to `(r1, c1)` and merge them.
    fn merge(&mut self, (r0, c0): (usize, usize), (r1, c1): (usize, usize)) {
        let (a, b) = (
            cell_pos(&self.state.doc, r0, c0),
            cell_pos(&self.state.doc, r1, c1),
        );
        assert!(
            self.try_run(Selection::cell(Pos(a), Pos(b)), "mergeCells"),
            "the merge applies"
        );
    }

    fn type_at(&mut self, pos: usize, text: &str) {
        self.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(pos)));
            tr.insert_text(text).unwrap();
        });
    }

    fn type_after(&mut self, after: &str, text: &str) {
        let at = pos_of(&self.state.doc, after) + after.chars().count();
        self.type_at(at, text);
    }

    /// Type `text` at the end of the first paragraph of the cell covering `(row, col)`.
    fn type_in_cell(&mut self, row: usize, col: usize, text: &str) {
        let at = cell_pos(&self.state.doc, row, col);
        let doc = self.state.doc.clone();
        let cell = doc.resolve(Pos(at + 1)).unwrap().parent().clone();
        let end = at + 2 + cell.child(0).content_size();
        self.type_at(end, text);
    }

    fn send(&mut self) -> Vec<u8> {
        self.session.save_incremental().expect("delta")
    }

    fn receive(&mut self, delta: &[u8]) {
        if let Some(next) = self
            .session
            .integrate_incremental(&self.state, delta)
            .expect("integrate")
        {
            self.state = next;
        }
        self.assert_model_is_projection("after a remote change");
    }

    fn assert_model_is_projection(&self, what: &str) {
        let projected = self.session.projected_doc(&self.schema).unwrap();
        assert_eq!(self.state.doc, projected, "{what}: model ≡ project(model)");
        assert_valid(&self.state.doc);
    }
}

fn exchange(a: &mut Peer, b: &mut Peer) {
    let (da, db) = (a.send(), b.send());
    b.receive(&da);
    a.receive(&db);
}

/// A host over `blocks` and a guest joined from it, the initial projection drained.
fn pair(s: &Rc<Schema>, blocks: Vec<Node>, ids: (u64, u64)) -> (Peer, Peer) {
    let mut a = Peer::host(s, blocks, ids.0);
    let b = a.join(ids.1);
    let _ = a.send();
    (a, b)
}

/// A two-peer concurrent scenario merged both ways, run with the client ids both ways
/// round (they break yrs's tie-breaks): every run converges on a valid table. Returns
/// each run's converged grid text.
fn concurrently(
    blocks: impl Fn(&Schema) -> Vec<Node>,
    a_edit: impl Fn(&mut Peer),
    b_edit: impl Fn(&mut Peer),
) -> Vec<String> {
    let mut out = Vec::new();
    for ids in [(11u64, 22u64), (22, 11)] {
        let s = schema();
        let (mut a, mut b) = pair(&s, blocks(&s), ids);
        a_edit(&mut a);
        b_edit(&mut b);
        exchange(&mut a, &mut b);
        assert_eq!(
            a.state.doc, b.state.doc,
            "the peers must converge (ids {ids:?})"
        );
        out.push(match first_table(&a.state.doc) {
            Some(_) => grid_text(&a.state.doc),
            None => "no table".into(),
        });
    }
    out
}

fn table_and_tail(s: &Schema, rows: usize, cols: usize) -> Vec<Node> {
    vec![grid(s, rows, cols), para(s, "tail")]
}

// --- the round trip --------------------------------------------------------------

#[test]
fn a_table_round_trips_with_headers_spans_marks_and_nested_blocks() {
    let s = schema();
    let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
    let rich = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![
                s.text("plain ").unwrap(),
                s.text_with_marks("bold", vec![bold]).unwrap(),
            ]),
        )
        .unwrap();
    let list = branch(
        &s,
        "bullet_list",
        vec![branch(&s, "list_item", vec![para(&s, "item")])],
    );
    let quote = branch(&s, "blockquote", vec![para(&s, "quoted")]);
    let table = branch(
        &s,
        "table",
        vec![
            branch(
                &s,
                "table_row",
                vec![header(&s, "h1"), header(&s, "h2"), header(&s, "h3")],
            ),
            branch(
                &s,
                "table_row",
                vec![
                    spanning(&s, "big", 2, 2),
                    branch(&s, "table_cell", vec![rich, list]),
                ],
            ),
            branch(&s, "table_row", vec![branch(&s, "table_cell", vec![quote])]),
        ],
    );
    let doc = doc_of(&s, vec![para(&s, "before"), table, para(&s, "after")]);
    let cdoc = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap();
    assert_eq!(cdoc.to_doc(&s).unwrap(), doc);
    let loaded = rinch_editor_collab::CollabDoc::load(&cdoc.save()).unwrap();
    assert_eq!(loaded.to_doc(&s).unwrap(), doc);
}

#[test]
fn a_table_whose_spans_overlap_fails_loud() {
    let s = schema();
    let table = branch(
        &s,
        "table",
        vec![
            branch(
                &s,
                "table_row",
                vec![cell(&s, "a"), spanning(&s, "tall", 1, 2)],
            ),
            branch(&s, "table_row", vec![spanning(&s, "wide", 2, 1)]),
        ],
    );
    let err = rinch_editor_collab::CollabDoc::from_doc(&doc_of(&s, vec![table])).unwrap_err();
    assert!(
        matches!(err, rinch_editor_collab::CollabError::Unsupported(_)),
        "got {err:?}"
    );
}

// --- every command reaches the peer ------------------------------------------------

#[test]
fn every_table_command_reaches_the_peer() {
    let s = schema();
    let (mut a, mut b) = pair(&s, vec![para(&s, "x")], (1, 2));
    let step = |a: &mut Peer, b: &mut Peer, what: &str| {
        exchange(a, b);
        assert_eq!(b.state.doc, a.state.doc, "after {what}");
    };
    assert!(a.try_run(Selection::cursor(Pos(2)), "insertTable"));
    step(&mut a, &mut b, "insertTable");
    a.type_in_cell(0, 0, "A");
    a.type_in_cell(1, 1, "B");
    a.type_in_cell(2, 2, "C");
    step(&mut a, &mut b, "typing in cells");
    a.in_cell(0, 0, "addRowBefore");
    step(&mut a, &mut b, "addRowBefore");
    a.in_cell(1, 1, "addRowAfter");
    step(&mut a, &mut b, "addRowAfter");
    a.in_cell(0, 0, "addColumnBefore");
    step(&mut a, &mut b, "addColumnBefore");
    a.in_cell(0, 2, "addColumnAfter");
    step(&mut a, &mut b, "addColumnAfter");
    a.merge((1, 1), (2, 2));
    step(&mut a, &mut b, "mergeCells");
    a.in_cell(1, 1, "splitCell");
    step(&mut a, &mut b, "splitCell");
    a.in_cell(0, 0, "deleteRow");
    step(&mut a, &mut b, "deleteRow");
    a.in_cell(0, 0, "deleteColumn");
    step(&mut a, &mut b, "deleteColumn");
    let (p, q) = (cell_pos(&a.state.doc, 0, 0), cell_pos(&a.state.doc, 1, 1));
    assert!(a.try_run(Selection::cell(Pos(p), Pos(q)), "deleteCellSelection"));
    step(&mut a, &mut b, "deleteCellSelection");
    // Enter inside a cell: two paragraphs in it.
    a.type_in_cell(2, 2, "split here");
    let at = pos_of(&a.state.doc, " here");
    assert!(a.try_run(Selection::cursor(Pos(at)), "enter"));
    step(&mut a, &mut b, "enter in a cell");
    assert_valid(&b.state.doc);
    a.in_cell(0, 0, "deleteTable");
    step(&mut a, &mut b, "deleteTable");
    assert!(first_table(&b.state.doc).is_none());
}

#[test]
fn a_merged_cell_and_its_split_reach_the_peer_as_a_grid() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 3, 3), (1, 2));
    a.merge((0, 0), (1, 1));
    exchange(&mut a, &mut b);
    assert_eq!(
        grid_text(&b.state.doc),
        "r0c0/r0c1/r1c0/r1c1 | · | r0c2\n· | · | r1c2\nr2c0 | r2c1 | r2c2"
    );
    a.in_cell(0, 0, "splitCell");
    exchange(&mut a, &mut b);
    assert_eq!(
        grid_text(&b.state.doc),
        "r0c0/r0c1/r1c0/r1c1 | _ | r0c2\n_ | _ | r1c2\nr2c0 | r2c1 | r2c2"
    );
}

// --- concurrency: the grid ----------------------------------------------------------

#[test]
fn a_row_and_a_column_added_at_once_make_a_full_grid() {
    // The case an array of rows of cells gets wrong: the new row has no cell in the
    // new column. Here the slot reads as an empty cell on both peers.
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.in_cell(0, 0, "addRowAfter"),
        |b| b.in_cell(0, 0, "addColumnAfter"),
    ) {
        assert_eq!(g, "r0c0 | _ | r0c1\n_ | _ | _\nr1c0 | _ | r1c1");
    }
}

#[test]
fn two_columns_added_at_once_both_land_in_every_row() {
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.in_cell(0, 0, "addColumnAfter"),
        |b| b.in_cell(0, 0, "addColumnAfter"),
    ) {
        assert_eq!(g, "r0c0 | _ | _ | r0c1\nr1c0 | _ | _ | r1c1");
    }
}

#[test]
fn two_rows_added_at_once_both_land() {
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.in_cell(1, 0, "addRowAfter"),
        |b| b.in_cell(0, 1, "addRowBefore"),
    ) {
        assert_eq!(g, "_ | _\nr0c0 | r0c1\nr1c0 | r1c1\n_ | _");
    }
}

#[test]
fn typing_in_two_cells_at_once_both_land() {
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.type_in_cell(0, 0, "A"),
        |b| b.type_in_cell(1, 1, "B"),
    ) {
        assert_eq!(g, "r0c0A | r0c1\nr1c0 | r1c1B");
    }
}

#[test]
fn typing_in_one_cell_at_once_both_land() {
    for g in concurrently(
        |s| table_and_tail(s, 1, 2),
        |a| a.type_after("r0c0", "A"),
        |b| b.type_at(pos_of(&b.state.doc, "r0c0"), "B"),
    ) {
        assert_eq!(g, "Br0c0A | r0c1");
    }
}

#[test]
fn deleting_a_row_while_a_peer_types_in_it_removes_the_row() {
    for g in concurrently(
        |s| table_and_tail(s, 3, 2),
        |a| a.in_cell(1, 0, "deleteRow"),
        |b| {
            b.type_in_cell(1, 1, "lost");
            b.type_in_cell(0, 0, "kept");
        },
    ) {
        assert_eq!(g, "r0c0kept | r0c1\nr2c0 | r2c1");
    }
}

#[test]
fn deleting_a_column_while_a_peer_types_in_it_removes_the_column() {
    for g in concurrently(
        |s| table_and_tail(s, 2, 3),
        |a| a.in_cell(0, 1, "deleteColumn"),
        |b| {
            b.type_in_cell(1, 1, "lost");
            b.type_in_cell(1, 2, "kept");
        },
    ) {
        assert_eq!(g, "r0c0 | r0c2\nr1c0 | r1c2kept");
    }
}

#[test]
fn a_row_added_while_a_peer_deletes_another_both_apply() {
    for g in concurrently(
        |s| table_and_tail(s, 3, 2),
        |a| a.in_cell(0, 0, "addRowAfter"),
        |b| b.in_cell(2, 0, "deleteRow"),
    ) {
        assert_eq!(g, "r0c0 | r0c1\n_ | _\nr1c0 | r1c1");
    }
}

#[test]
fn deleting_every_row_concurrently_leaves_no_table() {
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.in_cell(0, 0, "deleteRow"),
        |b| b.in_cell(1, 0, "deleteRow"),
    ) {
        assert_eq!(g, "no table");
    }
}

#[test]
fn deleting_the_table_while_a_peer_types_in_it_removes_it() {
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.in_cell(0, 0, "deleteTable"),
        |b| b.type_in_cell(1, 1, "lost"),
    ) {
        assert_eq!(g, "no table");
    }
}

// --- concurrency: merges and splits --------------------------------------------------

#[test]
fn a_column_added_inside_a_merge_is_inside_the_merged_cell() {
    // A merges the first row's two cells; B, not knowing, adds a column between them.
    // The merged cell names its last column, so the new column is inside its span on
    // every replica (which is what adding a column through a merged cell does locally
    // too), and B's new cell in that row is covered.
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.merge((0, 0), (0, 1)),
        |b| b.in_cell(0, 0, "addColumnAfter"),
    ) {
        assert_eq!(g, "r0c0/r0c1 | · | ·\nr1c0 | _ | r1c1");
    }
}

#[test]
fn a_row_added_inside_a_merge_is_inside_the_merged_cell() {
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.merge((0, 0), (1, 0)),
        |b| b.in_cell(0, 1, "addRowAfter"),
    ) {
        assert_eq!(g, "r0c0/r1c0 | r0c1\n· | _\n· | r1c1");
    }
}

#[test]
fn two_overlapping_merges_converge_on_a_grid() {
    // A merges the top-left 2×2, B the bottom-right 2×2: they share the centre cell.
    // The earlier anchor (A's) keeps its whole rectangle; B's is clipped to what is
    // left of it, and every slot still has exactly one cell.
    let docs = concurrently(
        |s| table_and_tail(s, 3, 3),
        |a| a.merge((0, 0), (1, 1)),
        |b| b.merge((1, 1), (2, 2)),
    );
    for g in &docs {
        let first = g.lines().next().unwrap();
        assert!(first.starts_with("r0c0/r0c1/r1c0/r1c1 | · |"), "{g}");
    }
}

#[test]
fn splitting_a_merged_cell_while_a_peer_types_in_it_keeps_the_typing() {
    let s = schema();
    let blocks = |s: &Schema| {
        vec![
            branch(
                s,
                "table",
                vec![
                    branch(s, "table_row", vec![spanning(s, "big", 2, 1), cell(s, "x")]),
                    branch(
                        s,
                        "table_row",
                        vec![cell(s, "a"), cell(s, "b"), cell(s, "c")],
                    ),
                ],
            ),
            para(s, "tail"),
        ]
    };
    let _ = s;
    for g in concurrently(
        blocks,
        |a| a.in_cell(0, 0, "splitCell"),
        |b| b.type_after("big", "!"),
    ) {
        assert_eq!(g, "big! | _ | x\na | b | c");
    }
}

#[test]
fn merging_cells_while_a_peer_types_in_the_master_keeps_the_typing() {
    for g in concurrently(
        |s| table_and_tail(s, 2, 2),
        |a| a.merge((0, 0), (0, 1)),
        |b| b.type_in_cell(0, 0, "!"),
    ) {
        assert_eq!(g, "r0c0!/r0c1 | ·\nr1c0 | r1c1");
    }
}

// --- concurrency: fillers --------------------------------------------------------------

#[test]
fn typing_into_a_filler_writes_it_and_reaches_the_peer() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 2, 2), (1, 2));
    a.in_cell(0, 0, "addRowAfter");
    b.in_cell(0, 0, "addColumnAfter");
    exchange(&mut a, &mut b);
    assert_eq!(
        grid_text(&a.state.doc),
        "r0c0 | _ | r0c1\n_ | _ | _\nr1c0 | _ | r1c1"
    );
    // (1, 1) is the filler: neither peer wrote that cell.
    a.type_in_cell(1, 1, "filled");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(
        grid_text(&b.state.doc),
        "r0c0 | _ | r0c1\n_ | filled | _\nr1c0 | _ | r1c1"
    );
    // An edit elsewhere in the table leaves the other fillers alone (no write races a
    // peer's typing there), and the table stays the same on both.
    b.type_in_cell(0, 0, "!");
    a.type_in_cell(1, 0, "row");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(
        grid_text(&b.state.doc),
        "r0c0! | _ | r0c1\nrow | filled | _\nr1c0 | _ | r1c1"
    );
}

#[test]
fn two_peers_typing_into_one_filler_converge() {
    // Both write a cell at the same key; one of the two cells wins whole (documented).
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 2, 2), (1, 2));
    a.in_cell(0, 0, "addRowAfter");
    b.in_cell(0, 0, "addColumnAfter");
    exchange(&mut a, &mut b);
    a.type_in_cell(1, 1, "A");
    b.type_in_cell(1, 1, "B");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    let g = grid_text(&a.state.doc);
    let middle = g.lines().nth(1).unwrap();
    assert!(middle == "_ | A | _" || middle == "_ | B | _", "{g}");
}

// --- the caret and sticky positions ------------------------------------------------

#[test]
fn a_caret_in_a_cell_keeps_its_place_when_a_peer_types_before_it() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 2, 2), (1, 2));
    a.type_in_cell(1, 1, " end");
    exchange(&mut a, &mut b);
    let caret = pos_of(&b.state.doc, "end");
    let mut tr = b.state.tr();
    tr.set_selection(Selection::cursor(Pos(caret)));
    b.state = b.state.apply(tr);
    a.type_at(pos_of(&a.state.doc, "r1c1"), ">> ");
    a.type_in_cell(0, 0, "more");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.selection.head().0, pos_of(&b.state.doc, "end"));
}

#[test]
fn a_sticky_index_in_a_cell_follows_its_character_through_structure_changes() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 2, 2), (1, 2));
    let at = pos_of(&a.state.doc, "r1c1");
    let sticky = a
        .session
        .sticky_index(&a.state.doc, Pos(at))
        .expect("an index");
    assert_eq!(
        a.session.resolve_sticky(&a.state.doc, &sticky),
        Some(Pos(at))
    );
    // B adds a row and a column in front of it, and types in the cell before it.
    b.in_cell(0, 0, "addRowBefore");
    b.in_cell(0, 0, "addColumnBefore");
    b.type_in_cell(2, 1, "!!");
    exchange(&mut a, &mut b);
    for peer in [&a, &b] {
        assert_eq!(
            peer.session.resolve_sticky(&peer.state.doc, &sticky),
            Some(Pos(pos_of(&peer.state.doc, "r1c1")))
        );
    }
    // A filler has no CRDT text behind it, so no index.
    let (mut c, mut d) = pair(&s, table_and_tail(&s, 2, 2), (3, 4));
    c.in_cell(0, 0, "addRowAfter");
    d.in_cell(0, 0, "addColumnAfter");
    exchange(&mut c, &mut d);
    let filler = cell_pos(&c.state.doc, 1, 1) + 2;
    assert_eq!(c.session.sticky_index(&c.state.doc, Pos(filler)), None);
}

// --- randomized convergence --------------------------------------------------------

/// A small deterministic PRNG (xorshift), so a trial replays from its seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// The grid size of the first table, if there is one.
fn dims(doc: &Node) -> Option<(usize, usize)> {
    let (table, _) = first_table(doc)?;
    Some((
        table.child_count(),
        rinch_editor_core::tables::column_count(&table),
    ))
}

/// One random edit on `peer`'s table (or a new table, when there is none). `None`
/// when the drawn command does not apply where it was drawn.
fn random_table_edit(rng: &mut Rng, peer: &mut Peer) -> bool {
    let Some((h, w)) = dims(&peer.state.doc) else {
        let end = peer.state.doc.content_size();
        let caret = Selection::near(&peer.state.doc, Pos(end - 1), -1);
        return peer.try_run(caret, "insertTable");
    };
    let (r, c) = (rng.below(h), rng.below(w));
    let at = cell_pos(&peer.state.doc, r, c);
    let caret = Selection::near(&peer.state.doc, Pos(at + 1), 1);
    let words = ["a", "bb", "ccc", "é", "🐱"];
    let op = rng.below(16);
    let applied = random_table_op(rng, peer, op, at, caret, (h, w), &words);
    if applied {
        APPLIED[op].fetch_add(1, Ordering::Relaxed);
    }
    applied
}

/// How often each arm of [`random_table_edit`] applied, across the test binary: the
/// positive control that the trials really merge, split, add and delete.
static APPLIED: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];

fn random_table_op(
    rng: &mut Rng,
    peer: &mut Peer,
    op: usize,
    at: usize,
    caret: Selection,
    (h, w): (usize, usize),
    words: &[&str],
) -> bool {
    match op {
        0..=4 => {
            // Type at a random offset of the cell's first block.
            let doc = peer.state.doc.clone();
            let cell = doc.resolve(Pos(at + 1)).unwrap().parent().clone();
            let first = cell.child(0);
            if !first.is_textblock() {
                return false;
            }
            let offset = rng.below(first.content_size() + 1);
            let word = words[rng.below(words.len())];
            peer.type_at(at + 2 + offset, word);
            true
        }
        5 => peer.try_run(caret, "addRowBefore"),
        6 => peer.try_run(caret, "addRowAfter"),
        7 => peer.try_run(caret, "addColumnBefore"),
        8 => peer.try_run(caret, "addColumnAfter"),
        9 => h > 1 && peer.try_run(caret, "deleteRow"),
        10 => w > 1 && peer.try_run(caret, "deleteColumn"),
        11 | 12 => {
            let (r2, c2) = (rng.below(h), rng.below(w));
            let b = cell_pos(&peer.state.doc, r2, c2);
            peer.try_run(Selection::cell(Pos(at), Pos(b)), "mergeCells")
        }
        13 => peer.try_run(caret, "splitCell"),
        14 => {
            let (r2, c2) = (rng.below(h), rng.below(w));
            let b = cell_pos(&peer.state.doc, r2, c2);
            peer.try_run(Selection::cell(Pos(at), Pos(b)), "deleteCellSelection")
        }
        _ => peer.try_run(caret, "enter"),
    }
}

/// One trial: `peers` replicas of a 3×3 table, `rounds` steps each of which is either a
/// random edit on a random peer or a random peer integrating a random other peer's
/// oldest undelivered delta (so deltas arrive in a different order on every replica),
/// then every peer integrates everything. Every replica must hold the same document,
/// and it must be valid with every table tiling its grid. Returns each peer's CRDT
/// bytes, which a replay must reproduce exactly.
fn table_trial(seed: u64, peers: usize, rounds: usize) -> Vec<Vec<u8>> {
    let s = schema();
    let mut rng = Rng::new(seed);
    let id = |p: usize| seed * 16 + p as u64 + 1;
    let host = Peer::host(&s, table_and_tail(&s, 3, 3), id(0));
    let mut replicas: Vec<Peer> = (1..peers).map(|p| host.join(id(p))).collect();
    replicas.insert(0, host);
    let _ = replicas[0].send();
    // outbox[p] holds p's deltas, each with the vector of how many of every peer's
    // deltas p had integrated when it made it; delivered[q][p] is how many of p's q
    // has. A delta is delivered only once its dependencies are (causal delivery), but
    // which sender a peer hears from next is random, so every replica integrates the
    // concurrent edits in an order of its own.
    let mut outbox: Vec<Vec<(Vec<u8>, Vec<usize>)>> = vec![Vec::new(); peers];
    let mut delivered = vec![vec![0usize; peers]; peers];
    let mut edits = 0;
    let deliverable = |delivered: &Vec<Vec<usize>>,
                       outbox: &Vec<Vec<(Vec<u8>, Vec<usize>)>>,
                       q: usize,
                       p: usize| {
        p != q
            && delivered[q][p] < outbox[p].len()
            && outbox[p][delivered[q][p]]
                .1
                .iter()
                .enumerate()
                .all(|(r, &need)| r == q || delivered[q][r] >= need)
    };
    for _ in 0..rounds {
        if rng.below(100) < 55 {
            let p = rng.below(peers);
            if random_table_edit(&mut rng, &mut replicas[p]) {
                edits += 1;
                let delta = replicas[p].send();
                if !delta.is_empty() {
                    let mut deps = delivered[p].clone();
                    deps[p] = outbox[p].len();
                    outbox[p].push((delta, deps));
                }
            }
        } else {
            let (q, p) = (rng.below(peers), rng.below(peers));
            if deliverable(&delivered, &outbox, q, p) {
                let delta = outbox[p][delivered[q][p]].0.clone();
                delivered[q][p] += 1;
                replicas[q].receive(&delta);
            }
        }
    }
    // Flush: keep delivering whatever is deliverable until nothing is left.
    loop {
        let mut moved = false;
        for q in 0..peers {
            for p in 0..peers {
                while deliverable(&delivered, &outbox, q, p) {
                    let delta = outbox[p][delivered[q][p]].0.clone();
                    delivered[q][p] += 1;
                    replicas[q].receive(&delta);
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
    assert!(edits > 0, "seed {seed}: the trial made no edit");
    let reference = replicas[0].state.doc.clone();
    for (q, peer) in replicas.iter().enumerate() {
        assert_eq!(
            peer.state.doc, reference,
            "seed {seed}: peer {q} diverged from peer 0"
        );
    }
    assert_valid(&reference);
    replicas.iter().map(|p| p.session.snapshot()).collect()
}

#[test]
fn random_table_edits_on_three_replicas_converge_on_a_grid() {
    let before: Vec<usize> = APPLIED.iter().map(|a| a.load(Ordering::Relaxed)).collect();
    for seed in 1..=60u64 {
        table_trial(seed, 3, 120);
    }
    let applied: Vec<usize> = APPLIED
        .iter()
        .zip(before)
        .map(|(a, b)| a.load(Ordering::Relaxed) - b)
        .collect();
    // Every structural arm must actually have run, or the trial tests less than it
    // claims: rows and columns added (5..=8) and deleted (9, 10), merges (11, 12),
    // splits (13), cleared selections (14).
    for (op, n) in applied.iter().enumerate().skip(5) {
        assert!(*n >= 10, "arm {op} applied only {n} times: {applied:?}");
    }
}

#[test]
fn random_table_edits_on_four_replicas_converge_on_a_grid() {
    for seed in 100..=130u64 {
        table_trial(seed, 4, 160);
    }
}

#[test]
fn a_table_trial_replays_from_its_seed() {
    for seed in [7u64, 42, 101] {
        assert_eq!(
            table_trial(seed, 3, 100),
            table_trial(seed, 3, 100),
            "seed {seed}"
        );
    }
}

// --- review round (#1233): rules the first suite did not pin -------------------------

#[test]
fn a_cell_emptied_by_two_concurrent_deletions_reads_as_one_empty_paragraph() {
    // Rule 3 of the read: each peer deletes one of the cell's two paragraphs, and the
    // converged cell holds no block at all in the CRDT.
    let s = schema();
    let two = branch(&s, "table_cell", vec![para(&s, "p1"), para(&s, "p2")]);
    for ids in [(11u64, 22u64), (22, 11)] {
        let blocks = vec![
            branch(
                &s,
                "table",
                vec![branch(&s, "table_row", vec![two.clone(), cell(&s, "x")])],
            ),
            para(&s, "tail"),
        ];
        let (mut a, mut b) = pair(&s, blocks, ids);
        let p1 = pos_of(&a.state.doc, "p1") - 1;
        a.local(|tr| {
            tr.delete(p1, p1 + 4).unwrap();
        });
        let p2 = pos_of(&b.state.doc, "p2") - 1;
        b.local(|tr| {
            tr.delete(p2, p2 + 4).unwrap();
        });
        exchange(&mut a, &mut b);
        assert_eq!(a.state.doc, b.state.doc, "ids {ids:?}");
        assert_eq!(grid_text(&a.state.doc), "_ | x", "ids {ids:?}");
    }
}

#[test]
fn a_column_inserted_beside_a_tombstone_is_outside_a_span_ending_on_it() {
    // `write_lines` inserts a new line after the tombstones before the next kept line,
    // so B's merge, whose span ends on the column A deleted, does not grow over A's
    // new column.
    for g in concurrently(
        |s| table_and_tail(s, 2, 3),
        |a| {
            a.in_cell(0, 1, "deleteColumn");
            a.in_cell(0, 0, "addColumnAfter");
        },
        |b| b.merge((0, 0), (0, 1)),
    ) {
        assert_eq!(g, "r0c0/r0c1 | _ | r0c2\nr1c0 | _ | r1c2");
    }
}

#[test]
fn a_sticky_index_in_the_master_cell_survives_a_peers_merge() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 2, 2), (1, 2));
    let at = pos_of(&b.state.doc, "r0c0") + 2;
    let sticky = b
        .session
        .sticky_index(&b.state.doc, Pos(at))
        .expect("an index in a cell");
    a.merge((0, 0), (1, 1));
    let d = a.send();
    b.receive(&d);
    let p = b
        .session
        .resolve_sticky(&b.state.doc, &sticky)
        .expect("resolves");
    let rp = b.state.doc.resolve(p).unwrap();
    let t = inline_text(rp.parent());
    assert_eq!(&t[rp.parent_offset()..], "c0", "parent text {t}");
}

/// An all-empty table (what `insertTable` makes).
fn empty_grid(s: &Schema, rows: usize, cols: usize) -> Node {
    branch(
        s,
        "table",
        (0..rows)
            .map(|_| branch(s, "table_row", (0..cols).map(|_| cell(s, "")).collect()))
            .collect(),
    )
}

/// Equal rows do not take each other's edits (#1240): rows and columns are matched by
/// the model's identity, not their value, so with three empty rows A's `deleteRow` on
/// the first deletes the first, and B's typing in the last survives. By value, the
/// last row was the one deleted, with B's typing.
#[test]
fn deleting_one_of_several_equal_rows_keeps_a_peers_typing_in_another() {
    for g in concurrently(
        |s| vec![empty_grid(s, 3, 2), para(s, "tail")],
        |a| a.in_cell(0, 0, "deleteRow"),
        |b| b.type_in_cell(2, 0, "keep"),
    ) {
        assert_eq!(g, "_ | _\nkeep | _");
    }
}

#[test]
fn deleting_one_of_several_equal_columns_keeps_a_peers_typing_in_another() {
    for g in concurrently(
        |s| vec![empty_grid(s, 2, 3), para(s, "tail")],
        |a| a.in_cell(0, 0, "deleteColumn"),
        |b| b.type_in_cell(0, 2, "keep"),
    ) {
        assert_eq!(g, "_ | keep\n_ | _");
    }
}

#[test]
fn a_row_added_among_equal_rows_lands_where_it_was_added() {
    // A's new row goes after row 0; B typed in old row 1, which is now row 2.
    for g in concurrently(
        |s| vec![empty_grid(s, 3, 1), para(s, "tail")],
        |a| a.in_cell(0, 0, "addRowAfter"),
        |b| b.type_in_cell(1, 0, "x"),
    ) {
        assert_eq!(g, "_\n_\nx\n_");
    }
}

#[test]
fn a_column_added_among_equal_columns_lands_where_it_was_added() {
    for g in concurrently(
        |s| vec![empty_grid(s, 1, 3), para(s, "tail")],
        |a| a.in_cell(0, 0, "addColumnAfter"),
        |b| b.type_in_cell(0, 1, "x"),
    ) {
        assert_eq!(g, "_ | _ | x | _");
    }
}

#[test]
fn deleting_one_of_several_equal_paragraphs_in_a_cell_keeps_a_peers_typing() {
    // A cell's blocks are a child list, matched by identity like the rows.
    let s = schema();
    for ids in [(11u64, 22u64), (22, 11)] {
        let three = branch(
            &s,
            "table_cell",
            vec![para(&s, ""), para(&s, ""), para(&s, "")],
        );
        let blocks = vec![
            branch(
                &s,
                "table",
                vec![branch(&s, "table_row", vec![three, cell(&s, "x")])],
            ),
            para(&s, "tail"),
        ];
        let (mut a, mut b) = pair(&s, blocks, ids);
        let at = cell_pos(&a.state.doc, 0, 0) + 1; // the cell's content
        a.local(|tr| {
            tr.delete(at, at + 2).unwrap();
        });
        b.type_at(at + 4 + 1, "keep");
        exchange(&mut a, &mut b);
        assert_eq!(a.state.doc, b.state.doc, "ids {ids:?}");
        let (table, _) = first_table(&a.state.doc).unwrap();
        let c = table.child(0).child(0);
        let texts: Vec<String> = (0..c.child_count())
            .map(|i| inline_text(c.child(i)))
            .collect();
        assert_eq!(texts, ["", "keep"], "ids {ids:?}");
    }
}

// --- the read's budget (a small remote update must not build a huge table) -----------

/// A foreign peer's update on top of `b`'s document: `n` column lines and `n` row lines
/// appended to the first table, each ~20 bytes on the wire, and optionally the first
/// cell's span stretched to the last of them. Returns the update.
fn grown_table_update(b: &Peer, n: usize, stretch: bool) -> Vec<u8> {
    use yrs::updates::decoder::Decode;
    use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};
    let doc = yrs::Doc::with_client_id(999);
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(&b.session.snapshot()).unwrap())
            .unwrap();
    }
    let sv = doc.transact().state_vector();
    {
        let content: ArrayRef = doc.get_or_insert_array("content");
        let mut txn = doc.transact_mut();
        let Some(Out::YMap(table)) = content.get(&txn, 0) else {
            panic!("no table")
        };
        let Some(Out::YArray(cols)) = table.get(&txn, "cols") else {
            panic!("no cols")
        };
        let Some(Out::YArray(rows)) = table.get(&txn, "rows") else {
            panic!("no rows")
        };
        for i in 0..n {
            let m = cols.push_back(&mut txn, MapPrelim::default());
            m.insert(&mut txn, "id", Any::String(format!("c{i}").into()));
            let r = rows.push_back(&mut txn, MapPrelim::default());
            r.insert(&mut txn, "id", Any::String(format!("r{i}").into()));
        }
        if stretch {
            let Some(Out::YMap(row0)) = rows.get(&txn, 0) else {
                panic!("no row 0")
            };
            let Some(Out::YMap(cells)) = row0.get(&txn, "cells") else {
                panic!("no cells")
            };
            let first = cells.keys(&txn).next().unwrap().to_string();
            let Some(Out::YMap(cell)) = cells.get(&txn, &first) else {
                panic!("no cell")
            };
            let last = n - 1;
            cell.insert(&mut txn, "col_end", Any::String(format!("c{last}").into()));
            cell.insert(&mut txn, "row_end", Any::String(format!("r{last}").into()));
        }
    }
    doc.transact().encode_state_as_update_v1(&sv)
}

/// The table a peer's model holds as its first block, as `rows × cols`.
fn table_dims(doc: &Node) -> (usize, usize) {
    let (table, _) = first_table(doc).expect("a table");
    (table.child_count(), table.child(0).child_count())
}

/// Integrate `update` into `peer`, which must stay healthy.
fn integrate_healthy(peer: &mut Peer, update: &[u8]) {
    if let Some(next) = peer
        .session
        .integrate_incremental(&peer.state, update)
        .expect("a table past the budget does not fail the integrate")
    {
        peer.state = next;
    }
    assert!(!peer.session.is_poisoned());
    peer.assert_model_is_projection("after an over-budget table arrived");
}

/// The review's case: 2000 rows and 2000 columns appended to a 1×1 table (90 KB), which
/// read as four million filler cells — 2.3 GB on every replica before the budget. The
/// read builds none of them: the model sees a one-cell placeholder, quickly, and the
/// session stays healthy, so the table can be deleted (below).
#[test]
fn a_small_update_that_would_read_as_millions_of_fillers_reads_as_a_placeholder() {
    let s = schema();
    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 2000, false);
    assert!(update.len() < 100_000, "{} bytes", update.len());
    let t = std::time::Instant::now();
    integrate_healthy(&mut b, &update);
    let took = t.elapsed();
    assert_eq!(table_dims(&b.state.doc), (1, 1));
    assert_eq!(
        grid_text(&b.state.doc),
        "_",
        "the placeholder is one empty cell"
    );
    assert!(took < std::time::Duration::from_secs(3), "took {took:?}");
}

/// The filler budget is exact, and off the fixed point: a 1×1 table grown by 255 lines
/// each way has 256² − 1 = 65,535 fillers, inside the 2^16 floor, and reads whole; one
/// line more each way is 66,048, past it, and reads as the placeholder.
#[test]
fn the_filler_budget_admits_the_floor_and_refuses_past_it() {
    let s = schema();
    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 255, false);
    integrate_healthy(&mut b, &update);
    assert_eq!(table_dims(&b.state.doc), (256, 256));

    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 256, false);
    integrate_healthy(&mut b, &update);
    assert_eq!(table_dims(&b.state.doc), (1, 1));
}

/// The slot budget is the model's own (`grid_slot_budget`): one stored cell stretched
/// over 2100×2100 slots (more than 2^22, and more than twice its one cell) has no
/// filler at all, and still reads as the placeholder, not as a table the model would
/// refuse any edit of.
#[test]
fn a_span_over_more_slots_than_the_model_allows_reads_as_a_placeholder() {
    let s = schema();
    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 2100, true);
    integrate_healthy(&mut b, &update);
    assert_eq!(table_dims(&b.state.doc), (1, 1));
}

/// While the shared document holds a table too large to read, outbound is frozen:
/// every local edit is refused with `OversizedTable` — typing inside the placeholder,
/// typing elsewhere, even deleting the table with the editor's own command — and
/// nothing reaches the CRDT. The cure is `delete_oversized_table`, by id: the table
/// leaves the CRDT and the model, the edits made meanwhile ship with it, and every
/// peer converges without the table.
#[test]
fn an_over_budget_table_freezes_outbound_until_it_is_deleted_by_id() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 2000, false);
    integrate_healthy(&mut a, &update);
    integrate_healthy(&mut b, &update);
    let snapshot = b.session.snapshot();
    let tables = b.session.oversized_tables();
    assert_eq!(tables.len(), 1);
    assert!(
        matches!(
            b.session.outbound_stall(),
            Some(CollabError::OversizedTable(_))
        ),
        "the freeze is reported as soon as the table arrives"
    );

    // The freeze alone leaves the model the CRDT's read: sticky indexes still work.
    let tail_pos = Pos(pos_of(&b.state.doc, "tail") + 2);
    assert!(b.session.sticky_index(&b.state.doc, tail_pos).is_some());

    // Typing inside it, typing elsewhere, deleting it with the editor: all refused.
    let at = cell_pos(&b.state.doc, 0, 0) + 2;
    let mut tr = b.state.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("x").unwrap();
    let typed = b.state.apply(tr);
    assert!(!rv4_soft_commit(&mut b, typed));
    let tail = pos_of(&b.state.doc, "tail") + 4;
    let mut tr = b.state.tr();
    tr.set_selection(Selection::cursor(Pos(tail)));
    tr.insert_text("!").unwrap();
    let typed = b.state.apply(tr);
    assert!(!rv4_soft_commit(&mut b, typed));
    let caret = Selection::near(&b.state.doc, Pos(at), 1);
    let mut tr = b.state.tr();
    tr.set_selection(caret);
    let placed = b.state.apply(tr);
    let gone = placed.run("deleteTable").expect("deleteTable applies");
    assert!(!rv4_soft_commit(&mut b, gone));
    assert!(matches!(
        b.session.outbound_stall(),
        Some(CollabError::OversizedTable(_))
    ));
    assert_eq!(b.session.snapshot(), snapshot, "nothing reached the CRDT");
    // The model is ahead of the CRDT now: no sticky index can be trusted.
    assert!(b.session.sticky_index(&b.state.doc, tail_pos).is_none());

    // Start over from the frozen model with the "!" kept, and cure by id.
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    integrate_healthy(&mut a, &update);
    integrate_healthy(&mut b, &update);
    let tail = pos_of(&b.state.doc, "tail") + 4;
    let mut tr = b.state.tr();
    tr.set_selection(Selection::cursor(Pos(tail)));
    tr.insert_text("!").unwrap();
    let typed = b.state.apply(tr);
    assert!(!rv4_soft_commit(&mut b, typed));
    let id = b.session.oversized_tables()[0].id.clone();
    let next = b
        .session
        .delete_oversized_table(&b.state, &id)
        .expect("the delete projects")
        .expect("the table was there");
    b.state = next;
    assert!(b.session.outbound_stall().is_none(), "the freeze lifted");
    assert!(b.session.oversized_tables().is_empty());
    assert!(first_table(&b.state.doc).is_none());
    b.assert_model_is_projection("after the delete");
    let delta = b.send();
    a.receive(&delta);
    assert_eq!(a.state.doc, b.state.doc);
    assert_eq!(
        inline_text(a.state.doc.child(0)),
        "tail!",
        "the frozen edit shipped"
    );
    assert!(a.session.oversized_tables().is_empty());
    assert!(a.session.outbound_stall().is_none());
    // A second delete finds nothing.
    assert!(
        b.session
            .delete_oversized_table(&b.state, &id)
            .unwrap()
            .is_none()
    );
}

/// A guest joining a document that holds a table too large to read starts frozen.
#[test]
fn a_guest_joining_a_document_with_an_over_budget_table_starts_frozen() {
    let s = schema();
    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let u = grown_table_update(&b, 2000, false);
    integrate_healthy(&mut b, &u);
    let guest = b.join(3);
    assert_eq!(guest.session.oversized_tables().len(), 1);
    assert!(matches!(
        guest.session.outbound_stall(),
        Some(CollabError::OversizedTable(_))
    ));
}

/// A peer shrinking the table back under the budget lifts the freeze on its own: the
/// placeholder becomes the real table and outbound resumes.
#[test]
fn a_peer_shrinking_the_table_lifts_the_freeze() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 2, 2), (1, 2));
    let u = grown_table_update(&b, 2000, false);
    integrate_healthy(&mut b, &u);
    integrate_healthy(&mut a, &u);
    assert!(!b.session.oversized_tables().is_empty());
    let shrink = rv4_foreign(&b, 0, 0, true, 777).expect("a shrink");
    integrate_healthy(&mut b, &shrink);
    assert!(b.session.oversized_tables().is_empty());
    assert!(b.session.outbound_stall().is_none());
    assert_eq!(table_dims(&b.state.doc).0, 3);
    b.type_after("tail", "!");
}

/// An honest concurrent edit that leaves fillers is untouched by the budget: a table
/// grown by 20 rows on one peer and 20 columns on the other at once reads with 400
/// fillers on both, and typing into one of them projects.
#[test]
fn many_rows_and_columns_added_at_once_still_read() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 2, 2), (1, 2));
    for _ in 0..20 {
        a.in_cell(0, 0, "addRowAfter");
        b.in_cell(0, 0, "addColumnAfter");
    }
    exchange(&mut a, &mut b);
    assert_eq!(a.state.doc, b.state.doc);
    let (table, _) = first_table(&a.state.doc).unwrap();
    assert_eq!(table.child_count(), 22);
    assert_eq!(table.child(0).child_count(), 22);
    a.type_in_cell(1, 1, "filled");
    exchange(&mut a, &mut b);
    assert_eq!(a.state.doc, b.state.doc);
}

// --- the identity the local diff relies on -----------------------------------------------

/// Every cell and row of `before`'s first table, with its text: `(row texts, cell)`.
fn cells_by_text(doc: &Node) -> Vec<(String, Node)> {
    let (table, _) = first_table(doc).expect("a table");
    let mut out = Vec::new();
    for r in 0..table.child_count() {
        let row = table.child(r);
        for k in 0..row.child_count() {
            out.push((inline_text(row.child(k)), row.child(k).clone()));
        }
    }
    out
}

/// Assert every row of `before`'s table not in `touched_rows` (by its first cell's
/// text) is the **same** `Rc` in `after`, and every cell whose text is not in
/// `touched_cells` and that is still there is the same `Rc` too: what the table and
/// child-list diffs match on.
fn assert_untouched_kept(
    before: &Node,
    after: &Node,
    touched_rows: &[usize],
    touched_cells: &[&str],
) {
    let (bt, _) = first_table(before).unwrap();
    let (at, _) = first_table(after).unwrap();
    let after_cells = cells_by_text(after);
    for (text, cell) in cells_by_text(before) {
        if text.is_empty() || touched_cells.contains(&text.as_str()) {
            continue;
        }
        if let Some((_, now)) = after_cells.iter().find(|(t, _)| *t == text) {
            assert!(cell.same_ref(now), "cell {text} was rebuilt");
        }
    }
    for r in 0..bt.child_count() {
        if touched_rows.contains(&r) {
            continue;
        }
        let row = bt.child(r);
        let first = inline_text(row.child(0));
        let kept = (0..at.child_count()).any(|i| {
            at.child(i).child_count() > 0
                && inline_text(at.child(i).child(0)) == first
                && at.child(i).same_ref(row)
        });
        assert!(kept, "row {r} was rebuilt");
    }
}

#[test]
fn table_commands_keep_every_row_and_cell_they_do_not_touch() {
    let s = schema();
    let fresh = || Peer::host(&s, table_and_tail(&s, 4, 4), 1);
    let check = |edit: &dyn Fn(&mut Peer), rows: &[usize], cells: &[&str]| {
        let mut p = fresh();
        let before = p.state.doc.clone();
        edit(&mut p);
        assert_untouched_kept(&before, &p.state.doc, rows, cells);
    };
    check(&|p| p.type_in_cell(1, 1, "!"), &[1], &["r1c1"]);
    check(&|p| p.in_cell(1, 1, "addRowAfter"), &[], &[]);
    check(&|p| p.in_cell(1, 1, "addRowBefore"), &[], &[]);
    check(&|p| p.in_cell(1, 1, "deleteRow"), &[1], &[]);
    check(&|p| p.in_cell(1, 1, "addColumnAfter"), &[0, 1, 2, 3], &[]);
    check(&|p| p.in_cell(1, 1, "addColumnBefore"), &[0, 1, 2, 3], &[]);
    check(&|p| p.in_cell(1, 1, "deleteColumn"), &[0, 1, 2, 3], &[]);
    check(
        &|p| p.merge((1, 1), (2, 2)),
        &[1, 2],
        &["r1c1", "r1c2", "r2c1", "r2c2"],
    );
    // A split, from the merged table.
    let mut p = fresh();
    p.merge((1, 1), (2, 2));
    let before = p.state.doc.clone();
    p.in_cell(1, 1, "splitCell");
    assert_untouched_kept(&before, &p.state.doc, &[1, 2], &["r1c1r1c2r2c1r2c2"]);
    // A row or column added inside a merged cell: the master cell's span changes.
    let mut p = fresh();
    p.merge((1, 1), (2, 2));
    let before = p.state.doc.clone();
    p.in_cell(1, 0, "addRowAfter");
    assert_untouched_kept(&before, &p.state.doc, &[1], &["r1c1r1c2r2c1r2c2"]);
    let before = p.state.doc.clone();
    p.in_cell(0, 1, "addColumnAfter");
    assert_untouched_kept(
        &before,
        &p.state.doc,
        &[0, 1, 2, 3, 4],
        &["r1c1r1c2r2c1r2c2"],
    );
}

// --- equal rows under concurrent structure: a randomized check (#1240) ---------------------

/// One peer's concurrent action in [`equal_rows_trial`].
#[derive(Debug, Clone, Copy)]
enum Act {
    Type {
        row: usize,
        col: usize,
    },
    /// `deleteRow`/`deleteColumn` (`after` unused), or an add before/after.
    Rows {
        at: usize,
        add: bool,
        after: bool,
    },
    Cols {
        at: usize,
        add: bool,
        after: bool,
    },
}

/// Where logical line `i` of one axis ends up after concurrent `deletes` and `adds`
/// (anchor, after?), or `None` when one of the peers deleted it.
fn moved(i: usize, deletes: &[usize], adds: &[(usize, bool)]) -> Option<usize> {
    if deletes.contains(&i) {
        return None;
    }
    let survivors = (0..i).filter(|k| !deletes.contains(k)).count();
    let added = adds
        .iter()
        .filter(|&&(a, after)| if after { a < i } else { a <= i })
        .count();
    Some(survivors + added)
}

/// A trial: an all-empty `rows × cols` table on `peers` synced replicas; every peer
/// makes one edit at once — typing a token of its own into a cell, or deleting or
/// adding a row or a column — and every delta reaches every peer in an order of its
/// own. All rows and columns are equal before, which is where a match by value
/// attributes a structural edit to the wrong line. The check: every token is exactly
/// where its author typed it, moved only by the rows and columns the others really
/// added or deleted, and is gone only when its own row or column was deleted.
fn equal_rows_trial(seed: u64, peers: usize) -> Vec<Act> {
    let s = schema();
    let mut rng = Rng::new(seed);
    let (rows, cols) = (4 + rng.below(2), 4 + rng.below(2));
    let id = |p: usize| seed * 16 + p as u64 + 1;
    let host = Peer::host(
        &s,
        vec![empty_grid(&s, rows, cols), para(&s, "tail")],
        id(0),
    );
    let mut replicas: Vec<Peer> = (1..peers).map(|p| host.join(id(p))).collect();
    replicas.insert(0, host);
    let _ = replicas[0].send();
    let mut acts = Vec::new();
    for (p, peer) in replicas.iter_mut().enumerate() {
        let (at_r, at_c) = (rng.below(rows), rng.below(cols));
        let (add, after) = (rng.below(2) == 0, rng.below(2) == 0);
        let act = match rng.below(if p == 0 { 1 } else { 3 }) {
            0 => Act::Type {
                row: at_r,
                col: at_c,
            },
            1 => Act::Rows {
                at: at_r,
                add,
                after,
            },
            _ => Act::Cols {
                at: at_c,
                add,
                after,
            },
        };
        let command = |add: bool, after: bool, what: &str| match (add, after) {
            (false, _) => format!("delete{what}"),
            (true, true) => format!("add{what}After"),
            (true, false) => format!("add{what}Before"),
        };
        match act {
            Act::Type { row, col } => peer.type_in_cell(row, col, &format!("t{p}x")),
            Act::Rows { at, add, after } => peer.in_cell(at, 0, &command(add, after, "Row")),
            Act::Cols { at, add, after } => peer.in_cell(0, at, &command(add, after, "Column")),
        }
        acts.push(act);
    }
    let deltas: Vec<Vec<u8>> = replicas.iter_mut().map(|p| p.send()).collect();
    for (q, replica) in replicas.iter_mut().enumerate() {
        let mut order: Vec<usize> = (0..peers).filter(|&p| p != q).collect();
        for k in (1..order.len()).rev() {
            order.swap(k, rng.below(k + 1));
        }
        for p in order {
            replica.receive(&deltas[p]);
        }
    }
    for q in 1..peers {
        assert_eq!(
            replicas[q].state.doc, replicas[0].state.doc,
            "seed {seed}: {acts:?}"
        );
    }
    let (mut row_del, mut row_add, mut col_del, mut col_add) = (vec![], vec![], vec![], vec![]);
    for act in &acts {
        match *act {
            Act::Rows { at, add: false, .. } => row_del.push(at),
            Act::Rows {
                at,
                add: true,
                after,
            } => row_add.push((at, after)),
            Act::Cols { at, add: false, .. } => col_del.push(at),
            Act::Cols {
                at,
                add: true,
                after,
            } => col_add.push((at, after)),
            Act::Type { .. } => {}
        }
    }
    let doc = &replicas[0].state.doc;
    let text = grid_text(doc);
    let grid: Vec<Vec<&str>> = text.lines().map(|l| l.split(" | ").collect()).collect();
    for (p, act) in acts.iter().enumerate() {
        let Act::Type { row, col } = *act else {
            continue;
        };
        let token = format!("t{p}x");
        let found = text.matches(&token).count();
        match (
            moved(row, &row_del, &row_add),
            moved(col, &col_del, &col_add),
        ) {
            (Some(r), Some(c)) => {
                assert_eq!(
                    found, 1,
                    "seed {seed}: {token} lost or doubled; {acts:?}\n{text}"
                );
                assert!(
                    grid[r][c].contains(&token),
                    "seed {seed}: {token} typed at ({row}, {col}) belongs at ({r}, {c}); \
                     {acts:?}\n{text}"
                );
            }
            _ => assert_eq!(
                found, 0,
                "seed {seed}: {token}'s line was deleted; {acts:?}\n{text}"
            ),
        }
    }
    acts
}

#[test]
fn typing_in_equal_rows_lands_where_it_was_typed_whatever_the_others_add_or_delete() {
    let (mut typed_beside_a_change, mut trials) = (0, 0);
    for seed in 1..=150u64 {
        for peers in [3, 4] {
            let acts = equal_rows_trial(seed * 7 + peers as u64, peers);
            trials += 1;
            let structural = acts.iter().any(|a| !matches!(a, Act::Type { .. }));
            if structural {
                typed_beside_a_change += 1;
            }
        }
    }
    // A positive control: most trials put a structural edit beside the typing.
    assert!(
        typed_beside_a_change * 2 > trials,
        "{typed_beside_a_change} of {trials}"
    );
}

// ===================== review round 2 (#1233b) =====================

/// A row deleted through a rowspan that STARTS in it: remove_row re-creates the
/// spanning cell one row down, so the row below changes Rc too. Does a peer's
/// typing in that lower row (another column) survive?
#[test]
fn rv2_delete_row_with_a_rowspan_starting_in_it_keeps_typing_below() {
    for g in concurrently(
        |s| {
            let t = branch(
                s,
                "table",
                vec![
                    branch(s, "table_row", vec![cell(s, "r0c0"), cell(s, "r0c1")]),
                    branch(
                        s,
                        "table_row",
                        vec![spanning(s, "S", 1, 2), cell(s, "r1c1")],
                    ),
                    branch(s, "table_row", vec![cell(s, "r2c1")]),
                    branch(s, "table_row", vec![cell(s, "r3c0"), cell(s, "r3c1")]),
                ],
            );
            vec![t, para(s, "tail")]
        },
        |a| a.in_cell(1, 1, "deleteRow"),
        |b| b.type_in_cell(2, 1, "X"),
    ) {
        assert_eq!(g, "r0c0 | r0c1\nS | r2c1X\nr3c0 | r3c1");
    }
}

/// A column deleted through a colspan that STARTS in it.
#[test]
fn rv2_delete_column_with_a_colspan_starting_in_it_keeps_typing_beside() {
    for g in concurrently(
        |s| {
            let t = branch(
                s,
                "table",
                vec![
                    branch(
                        s,
                        "table_row",
                        vec![spanning(s, "S", 2, 1), cell(s, "r0c2")],
                    ),
                    branch(
                        s,
                        "table_row",
                        vec![cell(s, "r1c0"), cell(s, "r1c1"), cell(s, "r1c2")],
                    ),
                ],
            );
            vec![t, para(s, "tail")]
        },
        |a| a.in_cell(1, 0, "deleteColumn"),
        |b| b.type_in_cell(1, 1, "X"),
    ) {
        assert_eq!(g, "S | r0c2\nr1c1X | r1c2");
    }
}

/// Stall recovery: the re-base diffs the CRDT's fresh read-back (no Rc shared with
/// the model) against the model. Identity then matches nothing.
#[test]
fn rv2_stall_recovery_rebase_deletes_the_row_that_was_deleted() {
    for ids in [(11u64, 22u64), (22, 11)] {
        let s = schema();
        let (mut a, mut b) = pair(&s, vec![grid(&s, 4, 1), para(&s, "tail")], ids);
        // A: an out-of-scope block → outbound stalls.
        let task = branch(
            &s,
            "task_list",
            vec![branch(&s, "task_item", vec![para(&s, "t")])],
        );
        let end = a.state.doc.content_size();
        let mut tr = a.state.tr();
        tr.replace_with(end, end, Fragment::from_node(task))
            .unwrap();
        let next = a.state.apply(tr);
        assert!(a.session.record_local(&s, &a.state.doc, &next.doc).is_err());
        a.state = next;
        // A deletes row 0 while stalled.
        let at = cell_pos(&a.state.doc, 0, 0);
        let mut tr = a.state.tr();
        tr.set_selection(Selection::near(&a.state.doc, Pos(at + 1), 1));
        let placed = a.state.apply(tr);
        let next = placed.run("deleteRow").unwrap();
        assert!(a.session.record_local(&s, &a.state.doc, &next.doc).is_err());
        a.state = next;
        // A removes the task list → re-base on the CRDT's read-back.
        let n = a.state.doc.child_count();
        let size = a.state.doc.child(n - 1).node_size();
        let end = a.state.doc.content_size();
        let mut tr = a.state.tr();
        tr.delete(end - size, end).unwrap();
        let next = a.state.apply(tr);
        a.session
            .record_local(&s, &a.state.doc, &next.doc)
            .expect("recovers");
        a.state = next;
        // B, concurrently, typed in the last row.
        b.type_in_cell(3, 0, "X");
        exchange(&mut a, &mut b);
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(grid_text(&a.state.doc), "r1c0\nr2c0\nr3c0X", "ids {ids:?}");
    }
}

/// Undo of a deleteRow among equal rows, while a peer types in a later row.
#[test]
fn rv2_undo_of_a_row_delete_among_equal_rows_keeps_typing() {
    for ids in [(11u64, 22u64), (22, 11)] {
        let s = schema();
        let (mut a, mut b) = pair(&s, vec![empty_grid(&s, 4, 1), para(&s, "tail")], ids);
        a.in_cell(0, 0, "deleteRow");
        exchange(&mut a, &mut b);
        // A undoes; B types in what is now row 2 (originally row 3).
        let next = a.state.run("undo").expect("undo");
        a.commit(next);
        b.type_in_cell(2, 0, "X");
        exchange(&mut a, &mut b);
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(grid_text(&a.state.doc), "_\n_\n_\nX", "ids {ids:?}");
    }
}

/// No two siblings anywhere in the model share an Rc, after random table edits with
/// undo/redo mixed in (identity matching needs every sibling to be its own Rc).
fn assert_no_shared_siblings(n: &Node, path: &str) {
    for i in 0..n.child_count() {
        for j in i + 1..n.child_count() {
            assert!(
                !n.child(i).same_ref(n.child(j)),
                "{path}: children {i} and {j} share an Rc"
            );
        }
        assert_no_shared_siblings(n.child(i), &format!("{path}/{i}"));
    }
}

#[test]
fn rv2_random_edits_with_undo_redo_never_share_a_sibling_rc() {
    let mut undos = 0;
    for seed in 1..=60u64 {
        let s = schema();
        let mut rng = Rng::new(seed);
        let mut p = Peer::host(&s, vec![empty_grid(&s, 3, 3), para(&s, "tail")], 1);
        for _ in 0..80 {
            match rng.below(10) {
                0 | 1 => {
                    let name = if rng.below(2) == 0 { "undo" } else { "redo" };
                    if let Some(next) = p.state.run(name) {
                        p.commit(next);
                        undos += 1;
                    }
                }
                _ => {
                    random_table_edit(&mut rng, &mut p);
                }
            }
            assert_no_shared_siblings(&p.state.doc, "doc");
        }
    }
    assert!(undos > 100, "{undos}");
}

/// Honest concurrent structure that exceeds the filler budget: A adds N rows while B
/// adds N columns (N² fillers). Measures where honest use poisons.
#[test]
#[ignore]
fn rv2_honest_concurrent_rows_and_columns_vs_filler_budget() {
    let n: usize = std::env::var("RV2_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(260);
    let s = schema();
    let (mut a, mut b) = pair(&s, vec![grid(&s, 1, 1), para(&s, "tail")], (11, 22));
    for _ in 0..n {
        a.in_cell(0, 0, "addRowAfter");
        b.in_cell(0, 0, "addColumnAfter");
    }
    let (da, db) = (a.send(), b.send());
    let t = std::time::Instant::now();
    let r = b.session.integrate_incremental(&b.state, &da);
    eprintln!(
        "n={n} delta={}B integrate={:?} ok={} poisoned={}",
        da.len(),
        t.elapsed(),
        r.is_ok(),
        b.session.is_poisoned()
    );
    // Past the floor the table reads as a placeholder; the session stays healthy.
    assert!(r.is_ok() && !b.session.is_poisoned());
    let _ = db;
}

#[test]
#[ignore]
fn rv2_perf_keystroke_big_table_and_big_list() {
    let s = schema();
    // 1000×10 table.
    let mut a = Peer::host(&s, vec![grid(&s, 1000, 10), para(&s, "tail")], 1);
    let mut b = a.join(2);
    let _ = a.send();
    let at = pos_of(&a.state.doc, "r500c5") + 2;
    let mut best = std::time::Duration::MAX;
    for i in 0..5 {
        let mut tr = a.state.tr();
        tr.set_selection(Selection::cursor(Pos(at + i)));
        tr.insert_text("x").unwrap();
        let next = a.state.apply(tr);
        let t = std::time::Instant::now();
        a.session.record_local(&s, &a.state.doc, &next.doc).unwrap();
        best = best.min(t.elapsed());
        a.state = next;
    }
    let d = a.send();
    let t = std::time::Instant::now();
    b.receive(&d);
    eprintln!(
        "table 1000x10: record_local best {best:?}; integrate {:?}",
        t.elapsed()
    );
    let t = std::time::Instant::now();
    b.in_cell(500, 5, "deleteRow");
    eprintln!("table 1000x10: deleteRow cmd+record {:?}", t.elapsed());
    // 2000-item list, each item: para + nested 1-item list.
    let item = |k: usize| {
        branch(
            &s,
            "list_item",
            vec![
                para(&s, &format!("i{k}")),
                branch(
                    &s,
                    "bullet_list",
                    vec![branch(&s, "list_item", vec![para(&s, &format!("n{k}"))])],
                ),
            ],
        )
    };
    let list = branch(&s, "bullet_list", (0..2000).map(item).collect());
    let mut a = Peer::host(&s, vec![list, para(&s, "tail")], 3);
    let at = pos_of(&a.state.doc, "n1000") + 2;
    let mut best = std::time::Duration::MAX;
    for i in 0..5 {
        let mut tr = a.state.tr();
        tr.set_selection(Selection::cursor(Pos(at + i)));
        tr.insert_text("x").unwrap();
        let next = a.state.apply(tr);
        let t = std::time::Instant::now();
        a.session.record_local(&s, &a.state.doc, &next.doc).unwrap();
        best = best.min(t.elapsed());
        a.state = next;
    }
    eprintln!("list 2000 nested: record_local best {best:?}");
}

/// A whole-document load while collaborating (EditorHandle::load_doc commits with no
/// node shared with the old document): a reload that drops row 0.
#[test]
fn rv2_a_fresh_document_load_deletes_the_row_that_was_dropped() {
    for ids in [(11u64, 22u64), (22, 11)] {
        let s = schema();
        let (mut a, mut b) = pair(&s, vec![grid(&s, 4, 1), para(&s, "tail")], ids);
        let fresh = doc_of(
            &s,
            vec![
                branch(
                    &s,
                    "table",
                    (1..4)
                        .map(|r| branch(&s, "table_row", vec![cell(&s, &format!("r{r}c0"))]))
                        .collect(),
                ),
                para(&s, "tail"),
            ],
        );
        a.session.record_local(&s, &a.state.doc, &fresh).unwrap();
        a.state = EditorState::create(s.clone(), fresh, plugins());
        b.type_in_cell(3, 0, "X");
        exchange(&mut a, &mut b);
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(grid_text(&a.state.doc), "r1c0\nr2c0\nr3c0X", "ids {ids:?}");
    }
}

/// The same for a quote's paragraphs (a nested child list).
#[test]
fn rv2_a_fresh_document_load_deletes_the_paragraph_that_was_dropped_in_a_quote() {
    for ids in [(11u64, 22u64), (22, 11)] {
        let s = schema();
        let q = |ps: &[&str]| branch(&s, "blockquote", ps.iter().map(|p| para(&s, p)).collect());
        let (mut a, mut b) = pair(
            &s,
            vec![q(&["p0", "p1", "p2", "p3"]), para(&s, "tail")],
            ids,
        );
        let fresh = doc_of(&s, vec![q(&["p1", "p2", "p3"]), para(&s, "tail")]);
        a.session.record_local(&s, &a.state.doc, &fresh).unwrap();
        a.state = EditorState::create(s.clone(), fresh, plugins());
        b.type_after("p3", "X");
        exchange(&mut a, &mut b);
        assert_eq!(a.state.doc, b.state.doc);
        let q0 = a.state.doc.child(0);
        let texts: Vec<String> = (0..q0.child_count())
            .map(|i| inline_text(q0.child(i)))
            .collect();
        assert_eq!(texts, ["p1", "p2", "p3X"], "ids {ids:?}");
    }
}

// --- review round 3 (rv3_*): the placeholder written over the real table -------------

/// [`grown_table_update`], for the table at top-level index `idx`.
fn rv3_grown_at(b: &Peer, idx: u32, n: usize) -> Vec<u8> {
    use yrs::updates::decoder::Decode;
    use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};
    let doc = yrs::Doc::with_client_id(999);
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(&b.session.snapshot()).unwrap())
            .unwrap();
    }
    let sv = doc.transact().state_vector();
    {
        let content: ArrayRef = doc.get_or_insert_array("content");
        let mut txn = doc.transact_mut();
        let Some(Out::YMap(table)) = content.get(&txn, idx) else {
            panic!("no table")
        };
        let Some(Out::YArray(cols)) = table.get(&txn, "cols") else {
            panic!("no cols")
        };
        let Some(Out::YArray(rows)) = table.get(&txn, "rows") else {
            panic!("no rows")
        };
        for i in 0..n {
            let m = cols.push_back(&mut txn, MapPrelim::default());
            m.insert(&mut txn, "id", Any::String(format!("c{i}").into()));
            let r = rows.push_back(&mut txn, MapPrelim::default());
            r.insert(&mut txn, "id", Any::String(format!("r{i}").into()));
        }
    }
    doc.transact().encode_state_as_update_v1(&sv)
}

/// Whether the peer's CRDT still holds the over-budget table: a strict read
/// (`projected_doc`... is the model read, so ask the snapshot's size instead: the real
/// table carries 2000 row and column lines).
fn rv3_real_table_alive(p: &Peer) -> bool {
    p.session.snapshot().len() > 20_000
}

/// Stalled by an edit inside the placeholder, then the paragraph above it deleted:
/// the re-base diffs the CRDT's read (placeholder) against the model by value, pairs
/// positionally, writes the model's 1-cell table over the paragraph and deletes the
/// real table — for every peer.
#[test]
fn rv3_typing_in_the_placeholder_then_deleting_the_line_above_keeps_the_real_table() {
    let s = schema();
    let blocks = vec![para(&s, "above"), grid(&s, 1, 1), para(&s, "tail")];
    let (mut a, mut b) = pair(&s, blocks, (1, 2));
    let update = rv3_grown_at(&b, 1, 2000);
    integrate_healthy(&mut a, &update);
    integrate_healthy(&mut b, &update);
    assert!(rv3_real_table_alive(&b));

    // Type in the placeholder: refused, stalled.
    let at = cell_pos(&b.state.doc, 0, 0) + 2;
    let mut tr = b.state.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("x").unwrap();
    let typed = b.state.apply(tr);
    assert!(
        b.session
            .record_local(&s, &b.state.doc, &typed.doc)
            .is_err()
    );
    b.state = typed;

    // Delete the paragraph above.
    let size = b.state.doc.child(0).node_size();
    let mut tr = b.state.tr();
    tr.delete(0, size).unwrap();
    let gone = b.state.apply(tr);
    let res = b.session.record_local(&s, &b.state.doc, &gone.doc);
    eprintln!("record_local after deleting the line above: {res:?}");
    b.state = gone;
    let delta = b.send();
    if let Some(n) = a.session.integrate_incremental(&a.state, &delta).unwrap() {
        a.state = n;
    }
    eprintln!(
        "snapshot bytes b={} a={}; a's table: {:?} {}",
        b.session.snapshot().len(),
        a.session.snapshot().len(),
        table_dims(&a.state.doc),
        grid_text(&a.state.doc)
    );
    assert!(
        rv3_real_table_alive(&b) && rv3_real_table_alive(&a),
        "the real 2000x2000 table was replaced by the placeholder (res {res:?})"
    );
}

/// The same, unstalled: one transaction deletes the two paragraphs above the
/// placeholder and edits the one after (an app's `update`, a grouped undo). Identity
/// runs are (0, 0), so the mid pairs positionally.
#[test]
fn rv3_one_transaction_moving_the_placeholder_up_keeps_the_real_table() {
    let s = schema();
    let blocks = vec![
        para(&s, "p1"),
        para(&s, "p2"),
        grid(&s, 1, 1),
        para(&s, "tail"),
    ];
    let (mut a, mut b) = pair(&s, blocks, (1, 2));
    let update = rv3_grown_at(&b, 2, 2000);
    integrate_healthy(&mut a, &update);
    integrate_healthy(&mut b, &update);
    assert!(rv3_real_table_alive(&b));

    let tail = pos_of(&b.state.doc, "tail");
    let two = b.state.doc.child(0).node_size() + b.state.doc.child(1).node_size();
    let mut tr = b.state.tr();
    tr.set_selection(Selection::cursor(Pos(tail)));
    tr.insert_text("X").unwrap();
    tr.delete(0, two).unwrap();
    let next = b.state.apply(tr);
    let res = b.session.record_local(&s, &b.state.doc, &next.doc);
    eprintln!("record_local: {res:?}");
    b.state = next;
    let delta = b.send();
    if let Some(n) = a.session.integrate_incremental(&a.state, &delta).unwrap() {
        a.state = n;
    }
    eprintln!(
        "snapshot bytes b={} a={}; a's table: {:?} {}",
        b.session.snapshot().len(),
        a.session.snapshot().len(),
        table_dims(&a.state.doc),
        grid_text(&a.state.doc)
    );
    assert!(
        rv3_real_table_alive(&b) && rv3_real_table_alive(&a),
        "the real table was replaced by the placeholder (res {res:?})"
    );
}

/// A sticky anchor in a paragraph AFTER an over-budget table: the session is healthy
/// now (no poison), so it should resolve. `find_text` walks the table with the strict
/// `cell_maps(..).ok()?`, which returns `None` for the whole search.
#[test]
fn rv3_a_sticky_after_the_placeholder_resolves() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let pos = Pos(pos_of(&b.state.doc, "tail") + 2);
    let before = b
        .session
        .sticky_index(&b.state.doc, pos)
        .expect("encode before");
    assert_eq!(b.session.resolve_sticky(&b.state.doc, &before), Some(pos));
    let update = grown_table_update(&b, 2000, false);
    integrate_healthy(&mut a, &update);
    integrate_healthy(&mut b, &update);
    let pos = Pos(pos_of(&b.state.doc, "tail") + 2);
    assert!(
        b.session.sticky_index(&b.state.doc, pos).is_some(),
        "encode after the placeholder"
    );
    assert_eq!(
        b.session.resolve_sticky(&b.state.doc, &before),
        Some(pos),
        "resolve after the placeholder"
    );
}

/// Enter at the start of a paragraph: where an anchor in it resolves. Reports, does
/// not assert (the sticky docs say: top level → the new empty paragraph; in a list or
/// quote → kept).
#[test]
fn rv3_enter_at_start_anchor_report() {
    let s = schema();
    let cases: Vec<(&str, Vec<Node>)> = vec![
        ("top, one block", vec![para(&s, "hello")]),
        (
            "top, two blocks",
            vec![para(&s, "first"), para(&s, "hello")],
        ),
        (
            "list item, one para",
            vec![branch(
                &s,
                "bullet_list",
                vec![branch(&s, "list_item", vec![para(&s, "hello")])],
            )],
        ),
        (
            "quote, two paras",
            vec![branch(
                &s,
                "blockquote",
                vec![para(&s, "first"), para(&s, "hello")],
            )],
        ),
    ];
    for (name, blocks) in cases {
        let (_a, mut b) = pair(&s, blocks, (1, 2));
        let start = pos_of(&b.state.doc, "hello");
        let anchor = b
            .session
            .sticky_index(&b.state.doc, Pos(start + 3))
            .unwrap();
        b.try_run(Selection::cursor(Pos(start)), "splitBlock")
            .then_some(())
            .or_else(|| {
                b.local(|tr| {
                    tr.set_selection(Selection::cursor(Pos(start)));
                    tr.split(start, 1, None).unwrap();
                });
                Some(())
            });
        let now = pos_of(&b.state.doc, "hello");
        let got = b.session.resolve_sticky(&b.state.doc, &anchor);
        eprintln!(
            "RV3 {name}: hello now at {now}, anchor resolves to {got:?} (kept = {:?})",
            Some(Pos(now + 3)) == got
        );
    }
}

/// Cost of the new "shares nothing?" scan: bold over every block of an n-paragraph doc
/// (every top-level block changes, none shared) — reports, does not assert.
#[test]
#[ignore]
fn rv3_select_all_bold_cost() {
    let s = schema();
    for n in [2000usize, 8000] {
        let blocks: Vec<Node> = (0..n).map(|i| para(&s, &format!("para {i}"))).collect();
        let (_a, b) = pair(&s, blocks, (1, 2));
        let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
        let mut tr = b.state.tr();
        tr.add_mark(0, b.state.doc.content_size(), bold).unwrap();
        let next = b.state.apply(tr);
        let mut session = b.session;
        let t = std::time::Instant::now();
        session.record_local(&s, &b.state.doc, &next.doc).unwrap();
        eprintln!("RV3 bold-all n={n}: {:?}", t.elapsed());
    }
}

// --- round 3: the placeholder is never written; the line budget (#1248) ---------------

/// `node` rebuilt from scratch: equal by value, sharing no `Rc` — what a load
/// (`load_doc`/`load_html`) hands the projection.
fn rebuilt(s: &Schema, node: &Node) -> Node {
    if let Some(t) = node.text() {
        return s.text(t).unwrap();
    }
    let children: Vec<Node> = (0..node.child_count())
        .map(|i| rebuilt(s, node.child(i)))
        .collect();
    s.create_node(
        node.type_name(),
        node.attrs().clone(),
        Fragment::from_children(children),
    )
    .unwrap()
}

/// [above, table, tail] with the table grown past the budget on both peers.
fn over_budget_pair(s: &Rc<Schema>) -> (Peer, Peer) {
    let blocks = vec![para(s, "above"), grid(s, 1, 1), para(s, "tail")];
    let (mut a, mut b) = pair(s, blocks, (1, 2));
    let update = rv3_grown_at(&b, 1, 2000);
    integrate_healthy(&mut a, &update);
    integrate_healthy(&mut b, &update);
    assert!(rv3_real_table_alive(&b));
    (a, b)
}

/// Commit `next` on `b`, send it to `a`, and return whether the projection took it.
fn commit_and_send(a: &mut Peer, b: &mut Peer, next: EditorState) -> bool {
    let s = b.schema.clone();
    let ok = b.session.record_local(&s, &b.state.doc, &next.doc).is_ok();
    b.state = next;
    let delta = b.send();
    if let Some(n) = a.session.integrate_incremental(&a.state, &delta).unwrap() {
        a.state = n;
    }
    ok
}

/// A load while collaborating that keeps the table and drops the paragraph above it
/// is refused like any other edit while the document is frozen: nothing is deleted.
#[test]
fn a_load_while_frozen_deletes_nothing() {
    let s = schema();
    let (mut a, mut b) = over_budget_pair(&s);
    let doc = &b.state.doc;
    let loaded = rebuilt(
        &s,
        &doc_of(&s, vec![doc.child(1).clone(), doc.child(2).clone()]),
    );
    let next = EditorState::create(s.clone(), loaded, plugins());
    assert!(!commit_and_send(&mut a, &mut b, next));
    assert!(rv3_real_table_alive(&b) && rv3_real_table_alive(&a));
    assert_eq!(
        a.state.doc.child_count(),
        3,
        "the paragraph above stays too"
    );
}

/// A load that moves the placeholder to where another block was cannot be written
/// without writing the placeholder: refused, stalled, the real table kept.
#[test]
fn a_load_that_moves_the_placeholder_is_refused() {
    let s = schema();
    let (mut a, mut b) = over_budget_pair(&s);
    let doc = &b.state.doc;
    let loaded = rebuilt(
        &s,
        &doc_of(
            &s,
            vec![doc.child(1).clone(), para(&s, "new"), para(&s, "tail!")],
        ),
    );
    let next = EditorState::create(s.clone(), loaded, plugins());
    assert!(!commit_and_send(&mut a, &mut b, next));
    assert!(b.session.outbound_stall().is_some());
    assert!(rv3_real_table_alive(&b) && rv3_real_table_alive(&a));
}

/// Select all and type over it (a paste or a replace-all) while frozen: refused, the
/// real table kept; only `delete_oversized_table` removes it.
#[test]
fn replacing_everything_while_frozen_deletes_nothing() {
    let s = schema();
    let (mut a, mut b) = over_budget_pair(&s);
    let end = b.state.doc.content_size();
    let mut tr = b.state.tr();
    tr.replace_with(0, end, Fragment::from_node(para(&s, "pasted")))
        .unwrap();
    let next = b.state.apply(tr);
    assert!(!commit_and_send(&mut a, &mut b, next));
    assert!(rv3_real_table_alive(&b) && rv3_real_table_alive(&a));
    // The cure, then the replace-all ships.
    let id = b.session.oversized_tables()[0].id.clone();
    let next = b
        .session
        .delete_oversized_table(&b.state, &id)
        .unwrap()
        .unwrap();
    b.state = next;
    let delta = b.send();
    a.receive(&delta);
    assert!(!rv3_real_table_alive(&b) && !rv3_real_table_alive(&a));
    assert_eq!(a.state.doc, b.state.doc);
    assert_eq!(inline_text(&a.state.doc), "pasted");
}

/// Pasting a copy of the placeholder elsewhere would write it: refused.
#[test]
fn pasting_a_copy_of_the_placeholder_is_refused() {
    let s = schema();
    let (mut a, mut b) = over_budget_pair(&s);
    let copy = b.state.doc.child(1).clone();
    let end = b.state.doc.content_size();
    let mut tr = b.state.tr();
    tr.replace_with(end, end, Fragment::from_node(copy))
        .unwrap();
    let next = b.state.apply(tr);
    assert!(!commit_and_send(&mut a, &mut b, next));
    assert!(rv3_real_table_alive(&b) && rv3_real_table_alive(&a));
}

/// #1248: a table's lines are bounded on write. One cell of `colspan = 3_000_000`
/// would write three million column lines; hosting it is refused, quickly, and
/// `colspan = 1000` (Chrome's largest) is not.
#[test]
fn hosting_a_table_with_a_huge_colspan_is_refused_quickly() {
    let s = schema();
    let huge = branch(
        &s,
        "table",
        vec![branch(
            &s,
            "table_row",
            vec![spanning(&s, "x", 3_000_000, 1)],
        )],
    );
    let state = EditorState::create(
        s.clone(),
        doc_of(&s, vec![huge, para(&s, "tail")]),
        plugins(),
    );
    let t = std::time::Instant::now();
    let err = session_with_client_id(&state, 1).expect_err("three million columns");
    assert!(
        t.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        t.elapsed()
    );
    assert!(err.to_string().contains("rows and columns"), "{err}");
    let wide = branch(
        &s,
        "table",
        vec![branch(&s, "table_row", vec![spanning(&s, "x", 1000, 1)])],
    );
    let state = EditorState::create(
        s.clone(),
        doc_of(&s, vec![wide, para(&s, "tail")]),
        plugins(),
    );
    assert!(session_with_client_id(&state, 1).is_ok());
}

/// #1248, an app's own transaction while collaborating: refused, stalled.
#[test]
fn an_app_transaction_adding_a_huge_colspan_is_refused() {
    let s = schema();
    let (mut a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let huge = branch(
        &s,
        "table",
        vec![branch(&s, "table_row", vec![spanning(&s, "x", 100_000, 1)])],
    );
    let end = b.state.doc.content_size();
    let mut tr = b.state.tr();
    tr.replace_with(end, end, Fragment::from_node(huge))
        .unwrap();
    let next = b.state.apply(tr);
    assert!(!commit_and_send(&mut a, &mut b, next));
    assert!(b.session.outbound_stall().is_some());
}

/// #1248, the read side and a guest join: a CRDT whose table has more lines than its
/// cells allow — here one cell stretched over 70,000 appended columns, 70,002 lines
/// and only 70,001 slots, so no other budget applies — reads as the placeholder, for a peer
/// and for a guest joining from it.
#[test]
fn a_guest_joining_past_the_line_budget_sees_the_placeholder() {
    use yrs::updates::decoder::Decode;
    use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};
    let s = schema();
    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let doc = yrs::Doc::with_client_id(999);
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(&b.session.snapshot()).unwrap())
            .unwrap();
    }
    let sv = doc.transact().state_vector();
    {
        let content: ArrayRef = doc.get_or_insert_array("content");
        let mut txn = doc.transact_mut();
        let Some(Out::YMap(table)) = content.get(&txn, 0) else {
            panic!("no table")
        };
        let Some(Out::YArray(cols)) = table.get(&txn, "cols") else {
            panic!("no cols")
        };
        let Some(Out::YArray(rows)) = table.get(&txn, "rows") else {
            panic!("no rows")
        };
        // Each new column right after the first: `push_back` walks the whole array per
        // insert, quadratic at this size. So the first one added ends up last.
        for i in 0..70_000 {
            let m = cols.insert(&mut txn, 1, MapPrelim::default());
            m.insert(&mut txn, "id", Any::String(format!("c{i}").into()));
        }
        let Some(Out::YMap(row0)) = rows.get(&txn, 0) else {
            panic!("no row 0")
        };
        let Some(Out::YMap(cells)) = row0.get(&txn, "cells") else {
            panic!("no cells")
        };
        let first = cells.keys(&txn).next().unwrap().to_string();
        let Some(Out::YMap(cell)) = cells.get(&txn, &first) else {
            panic!("no cell")
        };
        cell.insert(&mut txn, "col_end", Any::String("c0".into()));
    }
    let update = doc.transact().encode_state_as_update_v1(&sv);
    integrate_healthy(&mut b, &update);
    let placeholder = |p: &Peer| {
        let (t, _) = first_table(&p.state.doc).unwrap();
        t.attrs().get("rinch-collab-unreadable-table").is_some()
    };
    assert!(placeholder(&b), "{:?}", table_dims(&b.state.doc));
    let guest = b.join(3);
    assert!(placeholder(&guest));
    assert!(!guest.session.is_poisoned());
}

// --- rv4 fuzz: over-budget tables among random multi-peer edits --------------------

fn rv4_soft_commit(peer: &mut Peer, next: EditorState) -> bool {
    let ok = peer
        .session
        .record_local(&peer.schema, &peer.state.doc, &next.doc)
        .is_ok();
    peer.state = next;
    ok
}

/// A foreign writer (client `cid`) grows the `k`th top-level table of `p`'s CRDT by
/// `n` rows and columns, or shrinks it to 3 rows when `shrink`.
fn rv4_foreign(p: &Peer, k: usize, n: usize, shrink: bool, cid: u64) -> Option<Vec<u8>> {
    use yrs::updates::decoder::Decode;
    use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};
    let doc = yrs::Doc::with_client_id(cid);
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(&p.session.snapshot()).unwrap())
            .unwrap();
    }
    let sv = doc.transact().state_vector();
    {
        let content: ArrayRef = doc.get_or_insert_array("content");
        let mut txn = doc.transact_mut();
        let tables: Vec<yrs::MapRef> = content
            .iter(&txn)
            .filter_map(|v| match v {
                Out::YMap(m) if m.get(&txn, "cols").is_some() => Some(m),
                _ => None,
            })
            .collect();
        let table = tables.get(k)?.clone();
        let Some(Out::YArray(cols)) = table.get(&txn, "cols") else {
            return None;
        };
        let Some(Out::YArray(rows)) = table.get(&txn, "rows") else {
            return None;
        };
        if shrink {
            let len = rows.len(&txn);
            if len <= 3 {
                return None;
            }
            rows.remove_range(&mut txn, 3, len - 3);
        } else {
            for i in 0..n {
                let m = cols.insert(&mut txn, 1, MapPrelim::default());
                m.insert(&mut txn, "id", Any::String(format!("x{cid}c{i}").into()));
                let r = rows.insert(&mut txn, 1, MapPrelim::default());
                r.insert(&mut txn, "id", Any::String(format!("x{cid}r{i}").into()));
            }
        }
    }
    Some(doc.transact().encode_state_as_update_v1(&sv))
}

fn rv4_edit(rng: &mut Rng, peer: &mut Peer) -> bool {
    let doc = peer.state.doc.clone();
    let n = doc.child_count();
    let mut starts = Vec::with_capacity(n);
    let mut at = 0;
    for i in 0..n {
        starts.push(at);
        at += doc.child(i).node_size();
    }
    let i = rng.below(n);
    let block = doc.child(i);
    let mut tr = peer.state.tr();
    match rng.below(10) {
        0..=2 if block.is_textblock() => {
            let off = rng.below(block.content_size() + 1);
            tr.set_selection(Selection::cursor(Pos(starts[i] + 1 + off)));
            tr.insert_text(["a", "zz", "é"][rng.below(3)]).unwrap();
        }
        3 | 4 if n > 1 => {
            tr.delete(starts[i], starts[i] + block.node_size()).unwrap();
        }
        5 => {
            let j = rng.below(n + 1);
            let pos = if j == n { at } else { starts[j] };
            tr.replace_with(pos, pos, Fragment::from_node(para(&peer.schema, "new")))
                .unwrap();
        }
        6 if std::env::var("RV4_NOTABLE").is_ok() => return false,
        6 => {
            let pos = if rng.below(2) == 0 { 0 } else { at };
            tr.replace_with(pos, pos, Fragment::from_node(grid(&peer.schema, 2, 2)))
                .unwrap();
        }
        _ => {
            let Some((h, w)) = dims(&doc) else {
                return false;
            };
            let (r, c) = (rng.below(h), rng.below(w));
            let at = cell_pos(&doc, r, c);
            let caret = Selection::near(&doc, Pos(at + 1), 1);
            let name = [
                "addRowAfter",
                "addColumnBefore",
                "deleteRow",
                "deleteColumn",
                "splitCell",
                "enter",
            ][rng.below(6)];
            tr.set_selection(caret);
            let placed = peer.state.apply(tr);
            let Some(next) = placed.run(name) else {
                return false;
            };
            rv4_soft_commit(peer, next);
            return true;
        }
    }
    let next = peer.state.apply(tr);
    rv4_soft_commit(peer, next);
    true
}

// The review's harness, kept as it was written (index loops over shared logs).
#[allow(clippy::needless_range_loop)]
fn rv4_trial(seed: u64, peers: usize, rounds: usize) -> (usize, usize) {
    let s = schema();
    let mut rng = Rng::new(seed);
    let notable = std::env::var("RV4_NOTABLE").is_ok();
    let blocks = if notable {
        vec![
            para(&s, "a"),
            para(&s, "mid"),
            para(&s, "b"),
            para(&s, "tail"),
        ]
    } else {
        vec![
            grid(&s, 2, 2),
            para(&s, "mid"),
            grid(&s, 2, 3),
            para(&s, "tail"),
        ]
    };
    let host = Peer::host(&s, blocks, seed * 16 + 1);
    let mut reps: Vec<Peer> = (1..peers)
        .map(|p| host.join(seed * 16 + 1 + p as u64))
        .collect();
    reps.insert(0, host);
    let _ = reps[0].send();
    let initial = reps[0].session.snapshot();
    let mut seq: Vec<Vec<usize>> = vec![Vec::new(); peers];
    // Every delta (local or foreign) is broadcast to everyone, in random order per
    // receiver; yrs parks out-of-order updates.
    let mut log: Vec<Vec<u8>> = Vec::new();
    let mut deps: Vec<std::collections::HashSet<usize>> = Vec::new();
    let causal = std::env::var("RV4_CAUSAL").is_ok();
    // FIFO per sender by default: arbitrary per-sender reordering diverges even
    // with paragraphs only (out of this PR's scope; the review's issue draft 5).
    let fifo = std::env::var("RV4_ARBITRARY").is_err();
    let mut prod: Vec<usize> = Vec::new();
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); peers];
    let mut foreign_cid = 1_000_000 + seed * 1000;
    let mut grows = 0;
    for _ in 0..rounds {
        let roll = rng.below(100);
        let p = rng.below(peers);
        if roll < 50 {
            if rv4_edit(&mut rng, &mut reps[p])
                && let Ok(d) = reps[p].session.save_incremental()
                && !d.is_empty()
            {
                deps.push(order[p].iter().copied().collect());
                prod.push(p);
                log.push(d);
                order[p].push(log.len() - 1);
                seq[p].push(log.len() - 1);
            }
        } else if roll
            < 50 + std::env::var("RV4_FOREIGN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(6usize)
        {
            foreign_cid += 1;
            let k = rng.below(3);
            let shrink = rng.below(3) == 0;
            if let Some(u) = rv4_foreign(&reps[p], k, 2100, shrink, foreign_cid) {
                grows += usize::from(!shrink);
                deps.push(order[p].iter().copied().collect());
                prod.push(usize::MAX);
                log.push(u);
            }
        } else {
            // deliver a random undelivered delta to p
            let mine: std::collections::HashSet<usize> = order[p].iter().copied().collect();
            let pending: Vec<usize> = (0..log.len())
                .filter(|i| !mine.contains(i))
                .filter(|&i| !causal || deps[i].iter().all(|d| mine.contains(d)))
                .filter(|&i| {
                    !fifo
                        || prod[i] == usize::MAX
                        || (0..i).all(|j| prod[j] != prod[i] || mine.contains(&j))
                })
                .collect();
            if !pending.is_empty() {
                let i = pending[rng.below(pending.len())];
                order[p].push(i);
                seq[p].push(i);
                let st = reps[p].state.clone();
                let r = reps[p].session.integrate_incremental(&st, &log[i]);
                match r {
                    Ok(Some(n)) => reps[p].state = n,
                    Ok(None) => {}
                    Err(e) => panic!("seed {seed}: peer {p} integrate failed: {e:?}"),
                }
                assert!(!reps[p].session.is_poisoned(), "seed {seed}: poisoned");
            }
        }
    }
    // Flush.
    for p in 0..peers {
        let mine: std::collections::HashSet<usize> = order[p].iter().copied().collect();
        for i in 0..log.len() {
            if !mine.contains(&i) {
                seq[p].push(i);
                let st = reps[p].state.clone();
                if let Some(n) = reps[p]
                    .session
                    .integrate_incremental(&st, &log[i])
                    .unwrap_or_else(|e| panic!("seed {seed}: flush {e:?}"))
                {
                    reps[p].state = n;
                }
                assert!(!reps[p].session.is_poisoned(), "seed {seed}: poisoned");
            }
        }
    }
    if std::env::var("RV4_RAW").is_ok() {
        use yrs::updates::decoder::Decode;
        use yrs::{ReadTxn, Transact, Update};
        let replay = |ids: &[usize]| {
            let doc = yrs::Doc::new();
            {
                let mut t = doc.transact_mut();
                t.apply_update(Update::decode_v1(&initial).unwrap())
                    .unwrap();
            }
            for &i in ids {
                let mut t = doc.transact_mut();
                t.apply_update(Update::decode_v1(&log[i]).unwrap()).unwrap();
            }
            let t = doc.transact();
            let missing = t.has_missing_updates();
            let bytes = t.encode_state_as_update_v1(&yrs::StateVector::default());
            let sess = session_from_bytes_with_client_id(&bytes, 77).unwrap();
            (sess.projected_doc(&s).unwrap(), missing)
        };
        let all: Vec<usize> = (0..log.len()).collect();
        let reference = replay(&all);
        for q in 0..peers {
            let mine = replay(&seq[q]);
            let session_json = reps[q].session.projected_doc(&s).unwrap();
            eprintln!(
                "RAW peer {q}: raw-replay==index-order {} (missing {}), session==raw-replay {}",
                mine.0 == reference.0,
                mine.1,
                session_json == mine.0
            );
        }
    }
    // The placeholder attr never reaches the CRDT.
    for (q, r) in reps.iter().enumerate() {
        let snap = r.session.snapshot();
        let needle = b"rinch-collab-unreadable-table";
        assert!(
            !snap.windows(needle.len()).any(|w| w == needle),
            "seed {seed}: peer {q} wrote a placeholder"
        );
        assert_valid(&r.state.doc);
    }
    // Healthy (un-stalled) peers hold the CRDT's projection, and all CRDTs agree.
    let healthy: Vec<usize> = (0..peers)
        .filter(|&q| reps[q].session.outbound_stall().is_none())
        .collect();
    let proj0 = reps[0].session.projected_doc(&s).unwrap();
    if std::env::var("RV4_SEED").is_ok() {
        for (i, l) in log.iter().enumerate() {
            eprintln!(
                "log {i}: {} bytes, delivered to {:?}",
                l.len(),
                (0..peers)
                    .filter(|&q| order[q].contains(&i))
                    .collect::<Vec<_>>()
            );
        }
        if std::env::var("RV4_REDELIVER").is_ok() {
            for p in 0..peers {
                for i in 0..log.len() {
                    let before = reps[p].session.state_vector();
                    let missing_before = reps[1].session.sync_diff(&before).unwrap().len();
                    let st = reps[p].state.clone();
                    let r = reps[p].session.integrate_incremental(&st, &log[i]);
                    let after = reps[p].session.state_vector();
                    let missing_after = reps[1].session.sync_diff(&after).unwrap().len();
                    if missing_after != missing_before {
                        eprintln!(
                            "REDELIVER peer {p} log {i} ({} bytes): missing {missing_before} -> {missing_after}, r {:?}",
                            log[i].len(),
                            r.as_ref().map(|o| o.is_some())
                        );
                    }
                    if let Ok(Some(n)) = r {
                        reps[p].state = n;
                    }
                }
            }
        }
        for q in 0..peers {
            let sv0 = reps[0].session.state_vector();
            let svq = reps[q].session.state_vector();
            let d0q = reps[0].session.sync_diff(&svq).map(|d| d.len());
            let dq0 = reps[q].session.sync_diff(&sv0).map(|d| d.len());
            eprintln!(
                "peer {q}: stall {:?} diffs {d0q:?} {dq0:?}\n  proj {:?}",
                reps[q].session.outbound_stall().is_some(),
                grid_text(&reps[q].session.projected_doc(&s).unwrap())
            );
            eprintln!("  model {}", grid_text(&reps[q].state.doc));
            eprintln!("  doc {:?}", reps[q].session.projected_doc(&s).unwrap());
        }
    }
    for q in 0..peers {
        assert_eq!(
            reps[q].session.projected_doc(&s).unwrap(),
            proj0,
            "seed {seed}: CRDT of peer {q} diverged"
        );
    }
    for &q in &healthy {
        assert_eq!(
            reps[q].state.doc, proj0,
            "seed {seed}: healthy peer {q} model"
        );
    }
    (healthy.len(), grows)
}

#[test]
fn rv4_fuzz_over_budget_tables_among_multi_peer_edits() {
    let (mut healthy, mut grows, mut total) = (0, 0, 0);
    let seeds: Vec<u64> = match std::env::var("RV4_SEED") {
        Ok(v) => vec![v.parse().unwrap()],
        Err(_) => match std::env::var("RV4_SEEDS") {
            Ok(n) => (1..=n.parse::<u64>().unwrap()).collect(),
            // About 20 s a seed in a debug build; RV4_SEEDS=150 for the review's run.
            Err(_) => (1..=3).collect(),
        },
    };
    for seed in seeds {
        let (h, g) = rv4_trial(seed, 3, 160);
        healthy += h;
        grows += g;
        total += 3;
    }
    eprintln!("REPORT fuzz: healthy {healthy}/{total}, grows {grows}");
    // A positive control: the trials did grow tables past the budget.
    assert!(
        grows >= total / 3
            || std::env::var("RV4_SEED").is_ok()
            || std::env::var("RV4_FOREIGN").is_ok(),
        "grows {grows}"
    );
}

// --- round 4: the review's routes to a deleted table, all frozen now ------------------

/// The ids of the tables too large to read in `p`'s CRDT, read afresh.
fn oversized_ids(p: &Peer) -> Vec<String> {
    p.session.projected_doc(&p.schema).unwrap();
    p.session
        .oversized_tables()
        .into_iter()
        .map(|t| t.id)
        .collect()
}

/// [T1 (grown 2000), T2 (grown 2500), tail] on two peers.
fn two_over_budget(s: &Rc<Schema>) -> (Peer, Peer, Vec<String>) {
    let blocks = vec![grid(s, 1, 1), grid(s, 1, 1), para(s, "tail")];
    let (mut a, mut b) = pair(s, blocks, (1, 2));
    let first = rv3_grown_at(&b, 0, 2000);
    integrate_healthy(&mut a, &first);
    integrate_healthy(&mut b, &first);
    let second = rv3_grown_at(&b, 1, 2500);
    integrate_healthy(&mut a, &second);
    integrate_healthy(&mut b, &second);
    let ids = oversized_ids(&b);
    assert_eq!(ids.len(), 2);
    (a, b, ids)
}

/// Review round 4, F1: two placeholders are never equal (each carries its table's id),
/// and while either exists nothing is projected. The review's flow — stall on a ragged
/// paste, delete the first placeholder, cure the stall — used to re-base by value,
/// match the first placeholder to the second and delete the table the user kept.
#[test]
fn deleting_the_first_of_two_placeholders_while_frozen_deletes_nothing() {
    let s = schema();
    let (mut a, mut b, ids) = two_over_budget(&s);
    // A ragged table at the end.
    let ragged = branch(
        &s,
        "table",
        vec![
            branch(&s, "table_row", vec![cell(&s, "a"), cell(&s, "b")]),
            branch(&s, "table_row", vec![cell(&s, "c")]),
        ],
    );
    let end = b.state.doc.content_size();
    let mut tr = b.state.tr();
    tr.replace_with(end, end, Fragment::from_node(ragged))
        .unwrap();
    let next = b.state.apply(tr);
    assert!(!commit_and_send(&mut a, &mut b, next));
    // Delete the first placeholder.
    let size = b.state.doc.child(0).node_size();
    let mut tr = b.state.tr();
    tr.delete(0, size).unwrap();
    let next = b.state.apply(tr);
    assert!(!commit_and_send(&mut a, &mut b, next));
    // Delete the ragged table.
    let doc = b.state.doc.clone();
    let last = doc.child_count() - 1;
    let from: usize = (0..last).map(|i| doc.child(i).node_size()).sum();
    let mut tr = b.state.tr();
    tr.delete(from, doc.content_size()).unwrap();
    let next = b.state.apply(tr);
    assert!(!commit_and_send(&mut a, &mut b, next));
    assert_eq!(oversized_ids(&b), ids, "both real tables are still there");
    assert_eq!(oversized_ids(&a), ids);
    // The cure, by id, deletes exactly the table named.
    let next = b
        .session
        .delete_oversized_table(&b.state, &ids[0])
        .unwrap()
        .unwrap();
    b.state = next;
    let delta = b.send();
    a.receive(&delta);
    assert_eq!(oversized_ids(&b), vec![ids[1].clone()]);
    assert_eq!(oversized_ids(&a), vec![ids[1].clone()]);
    assert!(matches!(
        b.session.outbound_stall(),
        Some(CollabError::OversizedTable(_))
    ));
}

/// Review round 4, F1 through a load: a load that drops the first of two placeholders
/// deletes nothing.
#[test]
fn a_load_dropping_the_first_of_two_placeholders_deletes_nothing() {
    let s = schema();
    let (mut a, mut b, ids) = two_over_budget(&s);
    let doc = &b.state.doc;
    let loaded = rebuilt(
        &s,
        &doc_of(&s, vec![doc.child(1).clone(), doc.child(2).clone()]),
    );
    let next = EditorState::create(s.clone(), loaded, plugins());
    assert!(!commit_and_send(&mut a, &mut b, next));
    assert_eq!(oversized_ids(&b), ids);
    assert_eq!(oversized_ids(&a), ids);
}

/// Review round 4, F2: an app export carries no mark, so loading one back cannot tell
/// the placeholder from an ordinary empty table. Through HTML: the export of the
/// document without the line above, loaded back, deletes nothing.
#[test]
fn an_html_round_trip_while_frozen_deletes_nothing() {
    let s = schema();
    let (mut a, mut b) = over_budget_pair(&s);
    let doc = &b.state.doc;
    let html = rinch_editor_core::serialize::node_to_html(&doc_of(
        &s,
        vec![doc.child(1).clone(), doc.child(2).clone()],
    ));
    let slice = rinch_editor_core::serialize::slice_from_html(&s, &html).unwrap();
    let loaded = doc_of(
        &s,
        (0..slice.content.child_count())
            .map(|i| slice.content.child(i).clone())
            .collect(),
    );
    assert!(
        first_table(&loaded).is_some(),
        "the export holds a table: {html}"
    );
    let next = EditorState::create(s.clone(), loaded, plugins());
    assert!(!commit_and_send(&mut a, &mut b, next));
    assert!(rv3_real_table_alive(&b) && rv3_real_table_alive(&a));
}

/// A quote holding a table too large to read and a paragraph.
fn nested_over_budget(s: &Rc<Schema>) -> (Peer, Peer) {
    let quote = branch(s, "blockquote", vec![grid(s, 1, 1), para(s, "beside")]);
    let (mut a, mut b) = pair(s, vec![quote, para(s, "tail")], (1, 2));
    let update = {
        use yrs::updates::decoder::Decode;
        use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};
        let doc = yrs::Doc::with_client_id(999);
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(Update::decode_v1(&b.session.snapshot()).unwrap())
                .unwrap();
        }
        let sv = doc.transact().state_vector();
        {
            let content: ArrayRef = doc.get_or_insert_array("content");
            let mut txn = doc.transact_mut();
            let Some(Out::YMap(quote)) = content.get(&txn, 0) else {
                panic!("no quote")
            };
            let Some(Out::YArray(inner)) = quote.get(&txn, "content") else {
                panic!("no content")
            };
            let Some(Out::YMap(table)) = inner.get(&txn, 0) else {
                panic!("no table")
            };
            let Some(Out::YArray(cols)) = table.get(&txn, "cols") else {
                panic!("no cols")
            };
            let Some(Out::YArray(rows)) = table.get(&txn, "rows") else {
                panic!("no rows")
            };
            for i in 0..2000 {
                let m = cols.insert(&mut txn, 1, MapPrelim::default());
                m.insert(&mut txn, "id", Any::String(format!("c{i}").into()));
                let r = rows.insert(&mut txn, 1, MapPrelim::default());
                r.insert(&mut txn, "id", Any::String(format!("r{i}").into()));
            }
        }
        doc.transact().encode_state_as_update_v1(&sv)
    };
    integrate_healthy(&mut a, &update);
    integrate_healthy(&mut b, &update);
    assert_eq!(b.session.oversized_tables().len(), 1);
    (a, b)
}

/// Review round 4, F3: a placeholder nested in a quote. Typing beside it, or deleting
/// it with the editor, is frozen (nothing deleted); `delete_oversized_table` reaches
/// it inside the quote, the typing beside it ships, and the quote stays.
#[test]
fn a_nested_over_budget_table_is_deleted_by_id_and_typing_beside_it_ships() {
    let s = schema();
    let (mut a, mut b) = nested_over_budget(&s);
    let snapshot = b.session.snapshot();
    let at = pos_of(&b.state.doc, "beside") + 6;
    b.state = {
        let mut tr = b.state.tr();
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.insert_text("!").unwrap();
        let next = b.state.apply(tr);
        assert!(!rv4_soft_commit(&mut b, next.clone()));
        next
    };
    assert_eq!(b.session.snapshot(), snapshot, "frozen");
    let id = b.session.oversized_tables()[0].id.clone();
    let next = b
        .session
        .delete_oversized_table(&b.state, &id)
        .unwrap()
        .unwrap();
    b.state = next;
    assert!(b.session.outbound_stall().is_none());
    b.assert_model_is_projection("after deleting the nested table");
    let delta = b.send();
    a.receive(&delta);
    assert_eq!(a.state.doc, b.state.doc);
    let quote = a.state.doc.child(0);
    assert_eq!(quote.type_name(), "blockquote");
    assert_eq!(quote.child_count(), 1);
    assert_eq!(inline_text(quote), "beside!");
}

/// The table can be deleted by id from a cell of another table too (a nested table in
/// pasted HTML): the walk reaches inside the outer table's cells.
#[test]
fn an_over_budget_table_inside_a_cell_is_deleted_by_id() {
    let s = schema();
    let inner = grid(&s, 1, 1);
    let outer_cell = branch(&s, "table_cell", vec![inner, para(&s, "in cell")]);
    let outer = branch(
        &s,
        "table",
        vec![branch(&s, "table_row", vec![outer_cell, cell(&s, "x")])],
    );
    let (mut a, mut b) = pair(&s, vec![outer, para(&s, "tail")], (1, 2));
    let update = {
        use yrs::updates::decoder::Decode;
        use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};
        let doc = yrs::Doc::with_client_id(999);
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(Update::decode_v1(&b.session.snapshot()).unwrap())
                .unwrap();
        }
        let sv = doc.transact().state_vector();
        {
            let content: ArrayRef = doc.get_or_insert_array("content");
            let mut txn = doc.transact_mut();
            let Some(Out::YMap(outer)) = content.get(&txn, 0) else {
                panic!("no table")
            };
            let Some(Out::YArray(rows)) = outer.get(&txn, "rows") else {
                panic!("no rows")
            };
            let Some(Out::YMap(row)) = rows.get(&txn, 0) else {
                panic!("no row")
            };
            let Some(Out::YMap(cells)) = row.get(&txn, "cells") else {
                panic!("no cells")
            };
            let inner = cells
                .iter(&txn)
                .find_map(|(_, c)| match c {
                    Out::YMap(c) => match c.get(&txn, "content") {
                        Some(Out::YArray(content)) => match content.get(&txn, 0) {
                            Some(Out::YMap(t)) if t.get(&txn, "rows").is_some() => Some(t),
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                })
                .expect("the inner table");
            let Some(Out::YArray(cols)) = inner.get(&txn, "cols") else {
                panic!("no cols")
            };
            let Some(Out::YArray(rows)) = inner.get(&txn, "rows") else {
                panic!("no rows")
            };
            for i in 0..2000 {
                let m = cols.insert(&mut txn, 1, MapPrelim::default());
                m.insert(&mut txn, "id", Any::String(format!("c{i}").into()));
                let r = rows.insert(&mut txn, 1, MapPrelim::default());
                r.insert(&mut txn, "id", Any::String(format!("r{i}").into()));
            }
        }
        doc.transact().encode_state_as_update_v1(&sv)
    };
    integrate_healthy(&mut a, &update);
    integrate_healthy(&mut b, &update);
    let id = b.session.oversized_tables()[0].id.clone();
    let next = b
        .session
        .delete_oversized_table(&b.state, &id)
        .unwrap()
        .unwrap();
    b.state = next;
    b.assert_model_is_projection("after deleting the table in a cell");
    let delta = b.send();
    a.receive(&delta);
    assert_eq!(a.state.doc, b.state.doc);
    assert_eq!(grid_text(&a.state.doc), "in cell | x");
}

/// Once an edit is refused during a freeze the model is ahead of the CRDT, and a
/// sticky index can no longer be mapped: with two equal paragraphs and the first
/// deleted locally (refused), the model's first paragraph is the CRDT's second, and
/// an index taken there would name the deleted one.
#[test]
fn no_sticky_index_once_an_edit_is_refused_during_a_freeze() {
    let s = schema();
    let blocks = vec![
        para(&s, "dup"),
        para(&s, "dup"),
        grid(&s, 1, 1),
        para(&s, "tail"),
    ];
    let (_a, mut b) = pair(&s, blocks, (1, 2));
    let update = rv3_grown_at(&b, 2, 2000);
    integrate_healthy(&mut b, &update);
    assert!(b.session.sticky_index(&b.state.doc, Pos(2)).is_some());
    let size = b.state.doc.child(0).node_size();
    let mut tr = b.state.tr();
    tr.delete(0, size).unwrap();
    let next = b.state.apply(tr);
    assert!(!rv4_soft_commit(&mut b, next));
    assert!(b.session.sticky_index(&b.state.doc, Pos(2)).is_none());
}
