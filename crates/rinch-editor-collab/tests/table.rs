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
use rinch_editor_collab::{CollabPlugin, CollabSession};
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

/// Equal rows take each other's edits: the table matches rows and columns before and
/// after a local edit by value, so with three empty rows A's `deleteRow` on the first
/// tombstones the **last** row, and B's typing there goes with it though nobody deleted
/// that row. Pinned as the documented loss (guide, `src/table.rs`); a matching by
/// identity would make this keep "keep" and fail here.
#[test]
fn deleting_one_of_several_equal_rows_can_take_a_peers_typing_with_it() {
    for g in concurrently(
        |s| {
            let empty = branch(
                s,
                "table",
                (0..3)
                    .map(|_| branch(s, "table_row", vec![cell(s, ""), cell(s, "")]))
                    .collect(),
            );
            vec![empty, para(s, "tail")]
        },
        |a| a.in_cell(0, 0, "deleteRow"),
        |b| b.type_in_cell(2, 0, "keep"),
    ) {
        assert_eq!(g, "_ | _\n_ | _", "today the wrong row is deleted");
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

/// The review's case: 2000 rows and 2000 columns appended to a 1×1 table (90 KB), which
/// read as four million filler cells — 2.3 GB on every replica before the budget. It
/// must be refused, quickly, and poison (the CRDT is healthy only once the table goes).
#[test]
fn a_small_update_that_would_read_as_millions_of_fillers_is_refused_loud() {
    let s = schema();
    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 2000, false);
    assert!(update.len() < 100_000, "{} bytes", update.len());
    let t = std::time::Instant::now();
    let res = b.session.integrate_incremental(&b.state, &update);
    let took = t.elapsed();
    let err = res.expect_err("four million fillers must not be built");
    assert!(err.to_string().contains("filler"), "{err}");
    assert!(
        b.session.is_poisoned(),
        "a refused read poisons, like any unreadable CRDT"
    );
    assert!(took < std::time::Duration::from_secs(3), "took {took:?}");
}

/// The filler budget is exact, and off the fixed point: a 1×1 table grown by 255 lines
/// each way has 256² − 1 = 65,535 fillers, inside the 2^16 floor, and reads; one line
/// more each way is 66,048, past it.
#[test]
fn the_filler_budget_admits_the_floor_and_refuses_past_it() {
    let s = schema();
    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 255, false);
    let next = b
        .session
        .integrate_incremental(&b.state, &update)
        .expect("65,535 fillers are inside the floor")
        .expect("the table grew");
    let (table, _) = first_table(&next.doc).unwrap();
    assert_eq!(table.child_count(), 256);
    assert_eq!(table.child(255).child_count(), 256);

    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 256, false);
    let err = b
        .session
        .integrate_incremental(&b.state, &update)
        .expect_err("66,048 fillers are past the floor");
    assert!(err.to_string().contains("filler"), "{err}");
}

/// The slot budget is checked before anything is allocated, and is the model's own
/// (`grid_slot_budget`): one stored cell stretched over 2100×2100 slots (more than
/// 2^22, and more than twice its one cell) has no filler at all, and is still refused,
/// as the model would refuse that table for any local edit.
#[test]
fn a_span_over_more_slots_than_the_model_allows_is_refused_before_allocating() {
    let s = schema();
    let (_a, mut b) = pair(&s, table_and_tail(&s, 1, 1), (1, 2));
    let update = grown_table_update(&b, 2100, true);
    let err = b
        .session
        .integrate_incremental(&b.state, &update)
        .expect_err("4.4 million slots for one cell");
    assert!(err.to_string().contains("slots"), "{err}");
    assert!(b.session.is_poisoned());
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
