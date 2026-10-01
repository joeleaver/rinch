//! #1214, from the review of #1245: a collaboration guest adopts the shared
//! document **uncapped**. What the CRDT holds is its peers' edits, which are
//! not capped; a guest that capped on joining showed a different table than
//! the shared one, and its first edit in that cell wrote the cap back to the
//! peers as an edit nobody made.
//!
//! Tables enter the collab projection with #1233. Until then the host refuses
//! the table (`Unsupported`) and this test returns after saying so, so it pins
//! nothing yet; it pins the join from the moment #1233 lands.
#![cfg(feature = "collaboration")]

use rinch_editor_core::*;
use rinch_editor_view::create_editor;
use std::cell::RefCell;
use std::rc::Rc;

fn colspan(d: &Node) -> Option<i64> {
    d.child(0).child(0).child(0).attrs().get_int("colspan")
}

#[test]
fn a_guest_joins_a_wide_table_as_the_host_has_it() {
    let host = create_editor();
    let st = host.state();
    let s = st.schema();
    let p = s
        .branch("paragraph", Fragment::from_node(s.text("x").unwrap()))
        .unwrap();
    let c = s
        .create_node(
            "table_cell",
            Attrs::from_iter([("colspan", AttrValue::Int(5000))]),
            Fragment::from_node(p),
        )
        .unwrap();
    let r = s
        .create_node("table_row", Attrs::new(), Fragment::from_node(c))
        .unwrap();
    let t = s
        .create_node("table", Attrs::new(), Fragment::from_node(r))
        .unwrap();
    let tail = s
        .branch("paragraph", Fragment::from_node(s.text("tail").unwrap()))
        .unwrap();
    // An app's own transaction: not capped.
    assert!(host.update(|st| {
        let mut tr = st.tr();
        tr.replace_with(
            0,
            st.doc.content_size(),
            Fragment::from_children(vec![t.clone(), tail.clone()]),
        )
        .ok()?;
        Some(tr)
    }));
    assert_eq!(colspan(&host.doc()), Some(5000));
    let snap = match host.start_collaboration_host(|_| {}) {
        Ok(snap) => snap,
        Err(e) => {
            eprintln!("tables are not in the collab scope yet (#1233): {e:?}");
            return;
        }
    };
    let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let sink = to_host.clone();
    let guest = create_editor();
    guest
        .start_collaboration_guest(&snap, move |d| sink.borrow_mut().push(d))
        .unwrap();
    assert_eq!(
        colspan(&guest.doc()),
        Some(5000),
        "the guest adopts it uncapped"
    );
    assert!(to_host.borrow().is_empty(), "joining broadcasts nothing");
}
