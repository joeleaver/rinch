//! The freeze over a table too large to read, through `EditorHandle` (#1233 review
//! round 4): a peer's update grows a table past the read's budget; the guest shows a
//! placeholder, reports the freeze, keeps its edits local, and
//! `collab_delete_oversized_table` deletes the table for everybody and ships them.
#![cfg(feature = "collaboration")]

use rinch_editor_core::*;
use rinch_editor_view::{CollabError, create_editor};
use std::cell::RefCell;
use std::rc::Rc;
use yrs::updates::decoder::Decode;
use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};

/// A foreign writer's update on top of `snapshot`: 2000 rows and 2000 columns appended
/// to the first top-level table (90 KB; four million slots no stored cell covers).
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

#[test]
fn an_over_budget_table_freezes_the_guest_until_it_is_deleted_by_id() {
    let host = create_editor();
    assert!(host.update(|st| {
        let s = st.schema();
        let p = |t: &str| {
            s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap()
        };
        let cell = s
            .create_node("table_cell", Attrs::new(), Fragment::from_node(p("c")))
            .unwrap();
        let row = s
            .create_node("table_row", Attrs::new(), Fragment::from_node(cell))
            .unwrap();
        let table = s
            .create_node("table", Attrs::new(), Fragment::from_node(row))
            .unwrap();
        let mut tr = st.tr();
        tr.replace_with(
            0,
            st.doc.content_size(),
            Fragment::from_children(vec![table, p("tail")]),
        )
        .ok()?;
        Some(tr)
    }));
    let to_guest: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let sink = to_guest.clone();
    let snap = host
        .start_collaboration_host(move |d| sink.borrow_mut().push(d))
        .unwrap();
    let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let sink = to_host.clone();
    let guest = create_editor();
    guest
        .start_collaboration_guest(&snap, move |d| sink.borrow_mut().push(d))
        .unwrap();

    let update = grown(&guest.collab_snapshot().unwrap());
    assert!(guest.collab_receive(&update));
    assert!(host.collab_receive(&update));
    let tables = guest.collab_oversized_tables();
    assert_eq!(tables.len(), 1);
    assert!(matches!(
        guest.collab_outbound_stall(),
        Some(CollabError::OversizedTable(_))
    ));

    // An edit while frozen stays local: nothing is sent.
    let end = guest.doc().content_size() - 1;
    guest.set_selection(Selection::cursor(Pos(end)));
    guest.insert_text("!");
    assert!(to_host.borrow().is_empty(), "frozen: nothing broadcast");

    // The cure: one delta carries the deletion and the edit.
    assert_eq!(guest.collab_delete_oversized_table(&tables[0].id), Ok(true));
    assert!(guest.collab_outbound_stall().is_none());
    assert!(guest.collab_oversized_tables().is_empty());
    let sent: Vec<Vec<u8>> = to_host.borrow_mut().drain(..).collect();
    assert!(!sent.is_empty());
    for d in sent {
        host.collab_receive(&d);
    }
    let html =
        |h: &rinch_editor_view::EditorHandle| rinch_editor_core::serialize::node_to_html(&h.doc());
    assert_eq!(html(&host), html(&guest));
    assert!(!html(&host).contains("<table"), "{}", html(&host));
    assert!(html(&host).contains("tail!"), "{}", html(&host));
    assert!(host.collab_oversized_tables().is_empty());
    // Deleting again finds nothing.
    assert_eq!(
        guest.collab_delete_oversized_table(&tables[0].id),
        Ok(false)
    );
}
