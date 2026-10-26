//! An update that fills a gap yrs left in a client's clocks (a `Skip`: an update that
//! arrived ahead of one it follows, or a merged update with a hole) fires no event for
//! a change, in the same transaction, inside a type that client created after the gap
//! (yrs counts it as created by the transaction). No path names the change, so the
//! integrate that follows must read the whole document, and must not carry a table's
//! cell index through it (review of #1926, by its reviewer: both failed before
//! `sync::AppliedChanges` was spoiled while a gap is open — the first read back a
//! stale paragraph, the second read a cell map yrs had collected, which Valgrind and
//! ASan reported as a use after free). `fuzz.rs`'s out-of-order trials cover the class.

use std::rc::Rc;

use rinch_editor_collab::CollabSession;
use rinch_editor_collab::testing::{
    integrate_reads, session_from_bytes_with_client_id, session_with_client_id,
    set_incremental_read, set_incremental_read_check,
};
use rinch_editor_core::{EditorState, Fragment, Node, Pos, Schema, Selection, default_plugins};

fn para(s: &Schema, text: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn edit(&mut self, f: impl FnOnce(&EditorState) -> EditorState) -> Vec<u8> {
        let next = f(&self.state);
        self.session
            .record_local(&self.schema, &self.state.doc, &next.doc)
            .unwrap();
        self.state = next;
        self.session.save_incremental().unwrap()
    }
}

fn type_at(st: &EditorState, pos: usize, text: &str) -> EditorState {
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(pos)));
    tr.insert_text(text).unwrap();
    st.apply(tr)
}

fn end_of_doc(st: &EditorState) -> usize {
    st.doc.content().size()
}

/// Returns (the model R holds after the second integrate, R's CRDT read, partial reads).
fn scenario(incremental: bool) -> (Node, Node, u64) {
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch("doc", Fragment::from_children(vec![para(&s, "base")]))
        .unwrap();
    let state = EditorState::create(s.clone(), doc, default_plugins());
    let mut a = Peer {
        schema: s.clone(),
        session: session_with_client_id(&state, 1).unwrap(),
        state,
    };
    let _ = a.session.save_incremental().unwrap();
    let mut r_session = session_from_bytes_with_client_id(&a.session.snapshot(), 2).unwrap();
    let r_doc = r_session.projected_doc(&s).unwrap();
    let mut r_state = EditorState::create(s.clone(), r_doc, default_plugins());

    // A: a keystroke in the first paragraph (d1), another (d2), a new paragraph at
    // the end (d3), and a keystroke in it (d4).
    let d1 = a.edit(|st| type_at(st, 5, "1"));
    let d2 = a.edit(|st| type_at(st, 6, "2"));
    let d3 = a.edit(|st| {
        let end = end_of_doc(st);
        let mut tr = st.tr();
        tr.replace(
            end,
            end,
            rinch_editor_core::Slice::new(Fragment::from_node(para(&s, "new")), 0, 0),
        )
        .unwrap();
        st.apply(tr)
    });
    let d4 = a.edit(|st| {
        let end = end_of_doc(st);
        type_at(st, end - 1, "x")
    });

    set_incremental_read(incremental);
    // Off, so the release build shows what a release build does.
    set_incremental_read_check(false);
    let reads = integrate_reads();
    // R gets d1 and d3 merged (d2 missing: yrs stores a Skip for its clocks), then d2
    // and d4 merged: the transaction that fills the Skip also types in the paragraph
    // d3 created after it.
    for batch in [vec![&d1, &d3], vec![&d2, &d4]] {
        let bytes =
            yrs::merge_updates_v1(batch.iter().map(|d| d.as_slice()).collect::<Vec<_>>()).unwrap();
        if let Some(next) = r_session.integrate_incremental(&r_state, &bytes).unwrap() {
            r_state = next;
        }
    }
    let partial = integrate_reads().0 - reads.0;
    let read = r_session.projected_doc(&s).unwrap();
    assert_eq!(read, a.state.doc, "R's CRDT converged with A's");
    (r_state.doc, read, partial)
}

#[test]
fn whole_read_converges() {
    let (model, read, partial) = scenario(false);
    assert_eq!(partial, 0);
    assert_eq!(model, read);
}

#[test]
fn partial_read_after_a_skip_is_filled() {
    let (model, read, _) = scenario(true);
    assert_eq!(
        model, read,
        "the model R's editor holds is not its CRDT's read"
    );
}

fn branch(s: &Schema, name: &str, children: Vec<Node>) -> Node {
    s.branch(name, Fragment::from_children(children)).unwrap()
}

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

fn cell_before(doc: &Node, p: usize) -> usize {
    let r = doc.resolve(Pos(p)).unwrap();
    let d = (1..=r.depth())
        .rev()
        .find(|&d| r.node(d).type_name().ends_with("cell"))
        .unwrap();
    r.before(d).unwrap()
}

/// The same gap, but what the gap-filling transaction changes unseen is a table's
/// grid (a peer's merge deletes a cell map), so the table's cell index is carried
/// as if untouched while yrs has collected a map it holds; then R types in that cell.
#[test]
fn a_carried_index_after_a_skip_is_filled() {
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch("doc", Fragment::from_children(vec![para(&s, "base")]))
        .unwrap();
    let state = EditorState::create(s.clone(), doc, default_plugins());
    let mut a = Peer {
        schema: s.clone(),
        session: session_with_client_id(&state, 1).unwrap(),
        state,
    };
    let _ = a.session.save_incremental().unwrap();
    let mut r = Peer {
        schema: s.clone(),
        session: session_from_bytes_with_client_id(&a.session.snapshot(), 2).unwrap(),
        state: EditorState::create(s.clone(), a.state.doc.clone(), default_plugins()),
    };
    r.state = EditorState::create(
        s.clone(),
        r.session.projected_doc(&s).unwrap(),
        default_plugins(),
    );
    let cell = |t: &str| branch(&s, "table_cell", vec![para(&s, t)]);
    let table = branch(
        &s,
        "table",
        vec![
            branch(&s, "table_row", vec![cell("c00"), cell("c01"), cell("c02")]),
            branch(&s, "table_row", vec![cell("c10"), cell("c11"), cell("c12")]),
        ],
    );
    let d1 = a.edit(|st| type_at(st, 5, "1"));
    let d2 = a.edit(|st| type_at(st, 6, "2"));
    let d3 = a.edit(|st| {
        let end = end_of_doc(st);
        let mut tr = st.tr();
        tr.replace(
            end,
            end,
            rinch_editor_core::Slice::new(Fragment::from_node(table.clone()), 0, 0),
        )
        .unwrap();
        st.apply(tr)
    });
    let d4 = a.edit(|st| {
        let (x, y) = (
            cell_before(&st.doc, pos_of(&st.doc, "c00")),
            cell_before(&st.doc, pos_of(&st.doc, "c01")),
        );
        let mut tr = st.tr();
        tr.set_selection(Selection::cell(Pos(x), Pos(y)));
        st.apply(tr).run("mergeCells").expect("merge")
    });
    set_incremental_read_check(false);
    let merge = |batch: &[&Vec<u8>]| {
        yrs::merge_updates_v1(batch.iter().map(|d| d.as_slice()).collect::<Vec<_>>()).unwrap()
    };
    if let Some(next) = r
        .session
        .integrate_incremental(&r.state, &merge(&[&d1, &d3]))
        .unwrap()
    {
        r.state = next;
    }
    // R types in c01: the table's cell index is built.
    let at = pos_of(&r.state.doc, "c01") + 3;
    let _ = r.edit(|st| type_at(st, at, "r"));
    if let Some(next) = r
        .session
        .integrate_incremental(&r.state, &merge(&[&d2, &d4]))
        .unwrap()
    {
        r.state = next;
    }
    assert_eq!(
        r.state.doc,
        r.session.projected_doc(&s).unwrap(),
        "R's model after the gap is filled"
    );
    // R types in c01 again: the index must not be the one carried through the gap.
    let at = pos_of(&r.state.doc, "c01") + 3;
    let next = type_at(&r.state, at, "s");
    r.session
        .record_local(&s, &r.state.doc, &next.doc)
        .expect("the keystroke projects");
    assert_eq!(
        next.doc,
        r.session.projected_doc(&s).unwrap(),
        "R's CRDT after the keystroke"
    );
    // A receives it: both hold the merged cell with R's keystrokes in it.
    let d = r.session.save_incremental().unwrap();
    let _ = a.session.save_incremental().unwrap();
    if let Some(next) = a.session.integrate_incremental(&a.state, &d).unwrap() {
        a.state = next;
    }
    let d = r.session.sync_diff(&a.session.state_vector()).unwrap();
    if let Some(next) = a.session.integrate_incremental(&a.state, &d).unwrap() {
        a.state = next;
    }
    assert_eq!(a.state.doc, next.doc, "A converged with R");
}
