//! Round-4 review of #1595: one picture `Rc` at two places (an in-editor copy); A
//! deletes the FIRST copy while B notes the first copy. Plain `record_local`, so it
//! compiles on main too.
use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::serialize::html::{node_to_html, slice_from_html};
use rinch_editor_core::{AttrValue, EditorState, Fragment, Plugin, Schema, Slice, default_plugins};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn run(a_id: u64, b_id: u64) -> (String, String) {
    let s = Rc::new(Schema::starter_kit());
    let sl = slice_from_html(
        &s,
        r#"<p>ab<img src="a.png" data-ref="one"><img src="a.png" data-ref="two">ef</p>"#,
    )
    .unwrap();
    let d = s.branch("doc", sl.content.clone()).unwrap();
    let mut a_state = EditorState::create(s.clone(), d, plugins());
    // Make the second picture the same `Rc` as the first (an in-editor copy).
    let mut a: CollabSession = session_with_client_id(&a_state, a_id).unwrap();
    let mut tr = a_state.tr();
    let n = a_state.doc.node_at(3).unwrap();
    tr.replace(4, 5, Slice::new(Fragment::from_node(n), 0, 0))
        .unwrap();
    let next = a_state.apply(tr);
    a.record_local(&s, &a_state.doc, &next.doc).unwrap();
    a_state = next;
    assert!(
        a_state
            .doc
            .node_at(3)
            .unwrap()
            .same_ref(&a_state.doc.node_at(4).unwrap())
    );
    let mut b = session_from_bytes_with_client_id(&a.snapshot(), b_id).unwrap();
    let mut b_state = EditorState::create(s.clone(), b.projected_doc(&s).unwrap(), plugins());

    // A deletes the first copy.
    let mut tr = a_state.tr();
    tr.delete(3, 4).unwrap();
    let next = a_state.apply(tr);
    a.record_local(&s, &a_state.doc, &next.doc).unwrap();
    a_state = next;
    // B notes the first copy.
    let mut tr = b_state.tr();
    tr.set_node_attr(3, "data-note", AttrValue::from("for-first"))
        .unwrap();
    let next = b_state.apply(tr);
    b.record_local(&s, &b_state.doc, &next.doc).unwrap();
    b_state = next;

    let to_b = a.sync_diff(&b.state_vector()).unwrap();
    let to_a = b.sync_diff(&a.state_vector()).unwrap();
    if let Some(n) = b.integrate_incremental(&b_state, &to_b).unwrap() {
        b_state = n;
    }
    if let Some(n) = a.integrate_incremental(&a_state, &to_a).unwrap() {
        a_state = n;
    }
    (node_to_html(&a_state.doc), node_to_html(&b_state.doc))
}

#[test]
#[ignore = "G3 of the round-4 review of #1595: deleting one copy of a shared Rc keeps the other copy's identity (pre-existing on main; issue draft review4-1595-1)"]
fn p1p_delete_first_copy_peer_notes_first() {
    for (x, y) in [(1, 2), (2, 1)] {
        let (ha, hb) = run(x, y);
        eprintln!("P1p ids {x},{y}: {ha} | {hb}");
        assert_eq!(ha, hb);
        assert!(
            !ha.contains("for-first"),
            "note for the deleted copy shows: {ha}"
        );
    }
}
