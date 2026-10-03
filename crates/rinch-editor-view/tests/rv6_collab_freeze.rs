//! Review round 6 of #1233: the selection and preedit across a cure, the freeze and
//! the read-only consumers, a read-only editor's cure, a load while frozen, honest
//! peers freezing each other — and, ignored, undo across a remote change or a cure,
//! which replays at stale positions on main too (#1249).
#![cfg(feature = "collaboration")]

use rinch_editor_core::*;
use rinch_editor_view::{EditorHandle, create_editor};
use std::cell::RefCell;
use std::rc::Rc;
use yrs::updates::decoder::Decode;
use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};

fn grown(snapshot: &[u8]) -> Vec<u8> {
    let doc = yrs::Doc::with_client_id(999);
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(snapshot).unwrap())
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
        for i in 0..2000 {
            let m = cols.insert(&mut txn, 1, MapPrelim::default());
            m.insert(&mut txn, "id", Any::String(format!("c{i}").into()));
            let r = rows.insert(&mut txn, 1, MapPrelim::default());
            r.insert(&mut txn, "id", Any::String(format!("r{i}").into()));
        }
    }
    doc.transact().encode_state_as_update_v1(&sv)
}

type Q = Rc<RefCell<Vec<Vec<u8>>>>;

/// host doc `[table(1x1 "c")?, tail, other]`; returns (host, guest, to_host, to_guest).
fn setup(with_table: bool) -> (EditorHandle, EditorHandle, Q, Q) {
    let host = create_editor();
    assert!(host.update(|st| {
        let s = st.schema();
        let p = |t: &str| {
            s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap()
        };
        let mut blocks = Vec::new();
        if with_table {
            let cell = s
                .create_node("table_cell", Attrs::new(), Fragment::from_node(p("c")))
                .unwrap();
            let row = s
                .create_node("table_row", Attrs::new(), Fragment::from_node(cell))
                .unwrap();
            blocks.push(
                s.create_node("table", Attrs::new(), Fragment::from_node(row))
                    .unwrap(),
            );
        }
        blocks.push(p("tail"));
        blocks.push(p("PEERTEXT-must-survive"));
        let mut tr = st.tr();
        tr.replace_with(0, st.doc.content_size(), Fragment::from_children(blocks))
            .ok()?;
        Some(tr)
    }));
    let to_guest: Q = Rc::default();
    let sink = to_guest.clone();
    let snap = host
        .start_collaboration_host(move |d| sink.borrow_mut().push(d))
        .unwrap();
    let to_host: Q = Rc::default();
    let sink = to_host.clone();
    let guest = create_editor();
    guest
        .start_collaboration_guest(&snap, move |d| sink.borrow_mut().push(d))
        .unwrap();
    (host, guest, to_host, to_guest)
}

fn flush(q: &Q, to: &EditorHandle) {
    let v: Vec<Vec<u8>> = q.borrow_mut().drain(..).collect();
    for d in v {
        to.collab_receive(&d);
    }
}

fn html(h: &EditorHandle) -> String {
    rinch_editor_core::serialize::node_to_html(&h.doc())
}

/// End of the paragraph whose text is `t`.
fn end_of(h: &EditorHandle, t: &str) -> Pos {
    let doc = h.doc();
    let mut at = 0;
    for i in 0..doc.child_count() {
        let b = doc.child(i);
        let txt: String = (0..b.child_count())
            .filter_map(|j| b.child(j).text().map(str::to_owned))
            .collect();
        if b.is_textblock() && txt == t {
            return Pos(at + b.node_size() - 1);
        }
        at += b.node_size();
    }
    panic!("no {t} in {}", html(h))
}

/// Undo of a local edit after a remote insertion before it: the history is not mapped
/// through non-history transactions, so the inverse step lands in the peer's text.
/// Pre-existing on main; the cure is one more such transaction.
#[test]
#[ignore = "#1249: undo replays at stale positions after a remote change"]
fn rv6_undo_after_a_remote_insert_above_undoes_my_edit_not_a_peers_text() {
    let (host, guest, to_host, to_guest) = setup(false);
    guest.set_selection(Selection::cursor(end_of(&guest, "tail")));
    assert!(guest.insert_text("XYZ"));
    flush(&to_host, &host);
    // host inserts a paragraph at the very top
    assert!(host.update(|st| {
        let s = st.schema();
        let p = s
            .branch(
                "paragraph",
                Fragment::from_node(s.text("TOPTOPTOPTOPTOP").unwrap()),
            )
            .unwrap();
        let mut tr = st.tr();
        tr.replace_with(0, 0, Fragment::from_node(p)).ok()?;
        Some(tr)
    }));
    flush(&to_guest, &guest);
    let undone = guest.command("undo");
    flush(&to_host, &host);
    eprintln!("undo {undone}: guest {}", html(&guest));
    assert!(html(&guest).contains("TOPTOP"), "{}", html(&guest));
    assert!(
        html(&guest).contains("PEERTEXT-must-survive"),
        "{}",
        html(&guest)
    );
    assert!(!html(&guest).contains("XYZ") || !undone, "{}", html(&guest));
    assert_eq!(html(&guest), html(&host));
}

/// Undo across a freeze and its cure: the same #1249 replay, through the cure.
#[test]
#[ignore = "#1249: undo replays at stale positions after a remote change"]
fn rv6_undo_after_a_cure_undoes_my_edit_not_a_peers_text() {
    let (host, guest, to_host, _to_guest) = setup(true);
    guest.set_selection(Selection::cursor(end_of(&guest, "tail")));
    assert!(guest.insert_text("XYZ"));
    flush(&to_host, &host);
    let update = grown(&guest.collab_snapshot().unwrap());
    assert!(guest.collab_receive(&update));
    assert!(host.collab_receive(&update));
    let id = guest.collab_oversized_tables()[0].id.clone();
    assert!(!guest.command("undo"), "frozen: undo refused");
    assert_eq!(guest.collab_delete_oversized_table(&id), Ok(true));
    flush(&to_host, &host);
    let undone = guest.command("undo");
    flush(&to_host, &host);
    eprintln!("undo {undone}: guest {}", html(&guest));
    assert!(
        html(&guest).contains("PEERTEXT-must-survive"),
        "{}",
        html(&guest)
    );
    assert!(html(&guest).contains("tail"), "{}", html(&guest));
    assert_eq!(html(&guest), html(&host));
}

/// The selection across a cure: a range spanning the placeholder, and a caret after it.
#[test]
fn rv6_selection_and_preedit_across_a_cure() {
    let (host, guest, _to_host, _to_guest) = setup(true);
    let update = grown(&guest.collab_snapshot().unwrap());
    assert!(guest.collab_receive(&update));
    assert!(host.collab_receive(&update));
    let id = guest.collab_oversized_tables()[0].id.clone();
    // a range from inside the placeholder cell to inside "PEERTEXT"
    let doc = guest.doc();
    let end = doc.content_size() - 3;
    guest.set_selection(Selection::text(Pos(3), Pos(end)));
    // a preedit while frozen shows nothing
    guest.ime_set_preedit("kana", None);
    assert_eq!(guest.collab_delete_oversized_table(&id), Ok(true));
    let sel = guest.selection();
    eprintln!("after cure: {:?} doc {}", sel, html(&guest));
    let size = guest.doc().content_size();
    assert!(sel.from().0 <= size && sel.to().0 <= size);
    // editing works and lands where the caret is
    guest.set_selection(Selection::cursor(end_of(&guest, "tail")));
    assert!(guest.insert_text("!"));
    assert!(html(&guest).contains("tail!"));
}

/// The freeze and the read-only consumers: `is_read_only` stays the switch, and
/// `refuses_edits` — what the IME, the context menu, the clipboard paths and the web
/// capture field ask — answers the freeze too, and stops once the cure lands.
#[test]
fn rv6_a_frozen_editor_refuses_edits_without_being_read_only() {
    let (host, guest, _to_host, _to_guest) = setup(true);
    assert!(!guest.refuses_edits(), "control: editable");
    let update = grown(&guest.collab_snapshot().unwrap());
    assert!(guest.collab_receive(&update));
    let _ = host;
    assert!(!guest.insert_text("a"));
    assert!(!guest.is_read_only());
    assert!(guest.refuses_edits());
    let id = guest.collab_oversized_tables()[0].id.clone();
    assert_eq!(guest.collab_delete_oversized_table(&id), Ok(true));
    assert!(!guest.refuses_edits());
}

/// A read-only collaborating editor (a viewer role) can delete a table for everybody?
#[test]
fn rv6_a_read_only_editor_cannot_delete_a_table_for_everybody() {
    let (host, guest, to_host, _to_guest) = setup(true);
    let update = grown(&guest.collab_snapshot().unwrap());
    assert!(guest.collab_receive(&update));
    assert!(host.collab_receive(&update));
    guest.set_read_only(true);
    let id = guest.collab_oversized_tables()[0].id.clone();
    let r = guest.collab_delete_oversized_table(&id);
    let sent = to_host.borrow().len();
    flush(&to_host, &host);
    eprintln!("read-only cure: {r:?}, sent {sent}, host {}", html(&host));
    assert_eq!(
        r,
        Ok(false),
        "a read-only editor's cure is a write to the shared doc"
    );
    assert_eq!(sent, 0);
}

/// Honest peers alone: one adds rows, another adds columns at once — the merged table
/// crosses the filler budget and freezes everybody.
#[test]
fn rv6_honest_concurrent_rows_and_columns_freeze() {
    let (host, guest, to_host, to_guest) = setup(true);
    // guest: rows; host: columns, concurrently
    guest.set_selection(Selection::cursor(Pos(3)));
    host.set_selection(Selection::cursor(Pos(3)));
    for _ in 0..300 {
        assert!(guest.command("addRowAfter"));
        assert!(host.command("addColumnAfter"));
    }
    flush(&to_host, &host);
    flush(&to_guest, &guest);
    eprintln!(
        "oversized guest {:?} host {:?}",
        guest.collab_oversized_tables().len(),
        host.collab_oversized_tables().len()
    );
    assert!(!guest.collab_oversized_tables().is_empty());
}

/// Pin for M3: a load while frozen is refused and changes nothing.
#[test]
fn rv6_a_load_while_frozen_is_refused() {
    let (host, guest, to_host, _to_guest) = setup(true);
    let update = grown(&guest.collab_snapshot().unwrap());
    assert!(guest.collab_receive(&update));
    let _ = host;
    let before = guest.doc();
    assert!(!guest.load_html("<p>replaced</p>"));
    assert!(guest.doc().same_ref(&before));
    assert!(to_host.borrow().is_empty());
}
