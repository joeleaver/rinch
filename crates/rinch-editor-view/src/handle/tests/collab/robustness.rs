//! Poisoning deltas, undecodable blobs, emptied documents and unsupported
//! edits.

use super::*;

/// Whole-document bytes of a foreign yrs doc whose `content` root is a
/// **Text** type — the issue's headline poison shape. Decodes and applies
/// fine; the read-back is what can never succeed.
fn foreign_text_root_bytes() -> Vec<u8> {
    use yrs::{ReadTxn, Text, Transact};
    let doc = yrs::Doc::new();
    let t = doc.get_or_insert_text("content");
    t.insert(&mut doc.transact_mut(), 0, "foreign");
    doc.transact()
        .encode_state_as_update_v1(&yrs::StateVector::default())
}

#[test]
fn a_poisoning_delta_turns_the_session_loud_in_both_directions() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);

    // Foreign bytes reach the HOST mid-session.
    assert!(!host.collab_receive(&foreign_text_root_bytes()));
    assert!(
        host.is_collaboration_poisoned(),
        "the host session is poisoned"
    );
    assert!(
        !guest.is_collaboration_poisoned(),
        "the guest never saw the bytes and stays healthy"
    );
    assert!(
        matches!(
            host.collab_take_error(),
            Some(CollabError::SessionPoisoned(_))
        ),
        "the parked error is the sticky kind, distinguishable from a transient one"
    );

    // The host keeps editing locally, but nothing may leave the poisoned
    // replica — on unfixed code this edit was projected AND broadcast.
    host.set_selection(Selection::cursor(Pos(6)));
    assert!(host.insert_text("X"));
    assert_eq!(doc_text(&host), "helloX", "the local model still edits");
    assert_eq!(
        doc_text(&guest),
        "hello",
        "no delta left the poisoned replica"
    );
    assert!(
        matches!(
            host.collab_take_error(),
            Some(CollabError::SessionPoisoned(_))
        ),
        "each refused edit re-reports the sticky error"
    );

    // Inbound stays refused for perfectly valid peer deltas too.
    guest.set_selection(Selection::cursor(Pos(1)));
    assert!(guest.insert_text("G"));
    assert_eq!(doc_text(&guest), "Ghello");
    assert_eq!(
        doc_text(&host),
        "helloX",
        "the poisoned host can no longer receive"
    );

    // Recovery: stop, rejoin from the healthy peer's snapshot.
    host.stop_collaboration();
    assert!(
        !host.is_collaboration_poisoned(),
        "no session, no poison to report"
    );
    let snapshot = guest.collab_snapshot().expect("guest is collaborating");
    let g_in = guest.clone();
    host.start_collaboration_guest(&snapshot, move |d| {
        g_in.collab_receive(&d);
    })
    .expect("rejoining from a healthy snapshot works");
    assert_eq!(
        doc_text(&host),
        "Ghello",
        "the rejoined host adopts the healthy shared document"
    );
    host.set_selection(Selection::cursor(Pos(1)));
    assert!(host.insert_text("R"));
    assert_eq!(
        doc_text(&guest),
        "RGhello",
        "collaboration flows again after the rejoin"
    );
}

#[test]
fn an_undecodable_blob_is_transient_not_poison() {
    // A garbage blob never touches the CRDT, so it must NOT trip the sticky
    // state — the session keeps collaborating in both directions.
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);

    assert!(!host.collab_receive(&[0xff, 0xff, 0xff, 0xff]));
    assert!(
        matches!(host.collab_take_error(), Some(CollabError::Engine(_))),
        "a decode failure surfaces as a transient engine error"
    );
    assert!(!host.is_collaboration_poisoned());

    guest.set_selection(Selection::cursor(Pos(1)));
    assert!(guest.insert_text("G"));
    assert_eq!(doc_text(&host), "Ghello", "inbound still works");
    host.set_selection(Selection::cursor(Pos(7)));
    assert!(host.insert_text("X"));
    assert_eq!(doc_text(&guest), "GhelloX", "outbound still works");
}

// ── The zero-block state (issue #192) ────────────────────────────────
//
// Two peers deleting *different* blocks concurrently deletes every block, so
// the shared CRDT converges to an empty content list. No model can mirror that
// (the schema requires a block), so both peers show the starter paragraph while
// the CRDT holds nothing. That state used to wedge the session permanently: the
// next local edit tried to reconcile a block the CRDT did not have, failed with
// `Schema("reconcile_node: missing node")`, broadcast nothing, and never
// recovered.

/// Delete a whole block from `h` by its index, tokens included.
fn delete_block(h: &EditorHandle, index: usize) {
    let doc = h.doc();
    let start: usize = (0..index).map(|i| doc.child(i).node_size()).sum();
    let end = start + doc.child(index).node_size();
    h.update(|st| {
        let mut tr = st.tr();
        tr.delete(start, end).ok()?;
        Some(tr)
    });
}

/// Wire `host` and `guest` through a **buffered** loopback: deltas pile up
/// instead of being delivered, so edits made between exchanges are genuinely
/// concurrent. The returned closure drains both directions until quiet.
fn buffered_loopback(host: &EditorHandle, guest: &EditorHandle) -> impl Fn() {
    let to_guest: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let tg = to_guest.clone();
    let snapshot = host
        .start_collaboration_host(move |d| tg.borrow_mut().push(d))
        .expect("host projects its document");
    let th = to_host.clone();
    guest
        .start_collaboration_guest(&snapshot, move |d| th.borrow_mut().push(d))
        .expect("guest joins from the snapshot");
    let h = host.clone();
    let g = guest.clone();
    move || {
        // An integrated delta is never re-broadcast, so this settles in one
        // pass; the loop keeps that from being an assumption.
        for _ in 0..4 {
            let out_g: Vec<Vec<u8>> = to_guest.borrow_mut().drain(..).collect();
            let out_h: Vec<Vec<u8>> = to_host.borrow_mut().drain(..).collect();
            if out_g.is_empty() && out_h.is_empty() {
                return;
            }
            for d in out_g {
                g.collab_receive(&d);
            }
            for d in out_h {
                h.collab_receive(&d);
            }
        }
        panic!("the buffered loopback did not settle");
    }
}

/// Drive two peers to the converged zero-block state: each deletes a different
/// block of a two-block document, then the deltas are exchanged.
fn empty_the_shared_document(host: &EditorHandle, guest: &EditorHandle, exchange: &impl Fn()) {
    assert_eq!(doc_text(host), "one\ntwo");
    assert_eq!(
        doc_text(guest),
        "one\ntwo",
        "the guest joined on the host's doc"
    );
    delete_block(host, 1); // host drops "two"
    delete_block(guest, 0); // guest drops "one"
    assert_eq!(doc_text(host), "one");
    assert_eq!(doc_text(guest), "two");
    exchange();
    assert_eq!(doc_text(host), "", "every block was deleted");
    assert_eq!(doc_text(guest), "");
    assert_eq!(
        host.doc().child_count(),
        1,
        "the starter paragraph stands in"
    );
    assert_eq!(guest.doc().child_count(), 1);
}

#[test]
fn concurrent_block_deletions_do_not_wedge_the_session() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "one"), para(&s, "two")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    let exchange = buffered_loopback(&host, &guest);
    empty_the_shared_document(&host, &guest, &exchange);
    assert!(
        host.collab_take_error().is_none() && guest.collab_take_error().is_none(),
        "converging to an empty document is not an error"
    );

    // The host types: the edit must project, broadcast, and reach the guest.
    host.set_selection(Selection::cursor(Pos(1)));
    assert!(host.insert_text("Z"));
    assert_eq!(doc_text(&host), "Z");
    assert!(
        host.collab_take_error().is_none(),
        "the first edit after the empty state must project cleanly"
    );
    exchange();
    assert_eq!(
        doc_text(&guest),
        "Z",
        "the recovered edit reached the guest"
    );

    // And so must the guest's, in the other direction.
    guest.set_selection(Selection::cursor(Pos(1)));
    assert!(guest.insert_text("Y"));
    assert_eq!(doc_text(&guest), "YZ");
    assert!(guest.collab_take_error().is_none());
    exchange();
    assert_eq!(doc_text(&host), "YZ", "the guest's edit reached the host");
    assert!(
        host.collab_take_error().is_none() && guest.collab_take_error().is_none(),
        "no collaboration error surfaced anywhere in the recovery"
    );
}

#[test]
fn concurrent_first_edits_after_the_empty_state_converge() {
    // Both peers type before seeing the other, so both project their starter
    // paragraph into the CRDT. Two concurrent inserts into an empty array is
    // ordinary CRDT behaviour: two blocks, identical on both peers.
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "one"), para(&s, "two")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    let exchange = buffered_loopback(&host, &guest);
    empty_the_shared_document(&host, &guest, &exchange);

    host.set_selection(Selection::cursor(Pos(1)));
    assert!(host.insert_text("H"));
    guest.set_selection(Selection::cursor(Pos(1)));
    assert!(guest.insert_text("G"));
    exchange();

    use rinch_editor_core::serialize::node_to_html;
    assert_eq!(
        node_to_html(&host.doc()),
        node_to_html(&guest.doc()),
        "concurrent first edits out of the empty state converge"
    );
    assert_eq!(host.doc().child_count(), 2, "one block per peer");
    let t = doc_text(&host);
    assert!(
        t.contains('H') && t.contains('G'),
        "both edits survived: {t}"
    );
    assert!(host.collab_take_error().is_none() && guest.collab_take_error().is_none());
}

#[test]
fn a_late_joiner_can_join_a_session_with_no_blocks_left() {
    // The second symptom of #192: `load` used to reject a zero-block projection
    // as "not a rinch editor projection", locking a late joiner out of exactly
    // the state above.
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "one"), para(&s, "two")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

    // Manual buffers rather than the helper: the host's deltas have to fan out to
    // two peers once the late joiner arrives.
    let to_peers: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let tp = to_peers.clone();
    let snapshot = host
        .start_collaboration_host(move |d| tp.borrow_mut().push(d))
        .unwrap();
    let th = to_host.clone();
    guest
        .start_collaboration_guest(&snapshot, move |d| th.borrow_mut().push(d))
        .unwrap();

    delete_block(&host, 1);
    delete_block(&guest, 0);
    for d in to_peers.borrow_mut().drain(..) {
        guest.collab_receive(&d);
    }
    for d in to_host.borrow_mut().drain(..) {
        host.collab_receive(&d);
    }
    assert_eq!(
        doc_text(&host),
        "",
        "precondition: the CRDT holds no blocks"
    );
    assert_eq!(doc_text(&guest), "");

    // A third peer joins from the host's CURRENT (zero-block) snapshot.
    let late = mount(doc_node(&s, vec![para(&s, "stale")])).handle;
    let late_out: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let lo = late_out.clone();
    let snapshot = host.collab_snapshot().expect("host is collaborating");
    late.start_collaboration_guest(&snapshot, move |d| lo.borrow_mut().push(d))
        .expect("a zero-block snapshot is joinable");
    assert_eq!(
        doc_text(&late),
        "",
        "the late joiner adopts the starter paragraph"
    );
    assert_eq!(late.doc().child_count(), 1);

    // The joiner types: the host converges.
    late.set_selection(Selection::cursor(Pos(1)));
    assert!(late.insert_text("L"));
    for d in late_out.borrow_mut().drain(..) {
        host.collab_receive(&d);
        guest.collab_receive(&d);
    }
    assert_eq!(doc_text(&host), "L", "the joiner's edit reached the host");
    assert!(late.collab_take_error().is_none());

    // The host types: the joiner converges.
    host.set_selection(Selection::cursor(Pos(2)));
    assert!(host.insert_text("H"));
    for d in to_peers.borrow_mut().drain(..) {
        late.collab_receive(&d);
        guest.collab_receive(&d);
    }
    assert_eq!(doc_text(&late), "LH", "the host's edit reached the joiner");
    assert!(
        host.collab_take_error().is_none() && late.collab_take_error().is_none(),
        "no collaboration error surfaced in either direction"
    );
}

#[test]
fn unsupported_local_edit_fails_loud_without_touching_the_peer() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "ok")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);
    assert_eq!(doc_text(&guest), "ok");

    // A task list is still outside the projected scope (lists, quotes and
    // inline atoms are supported now; tables and task lists are not). HTML has
    // no task list, so it is inserted directly.
    assert!(host.update(|state| {
        let s = state.schema().clone();
        let inner = s
            .branch("paragraph", Fragment::from_node(s.text("todo").ok()?))
            .ok()?;
        let item = s.branch("task_item", Fragment::from_node(inner)).ok()?;
        let list = s.branch("task_list", Fragment::from_node(item)).ok()?;
        let at = state.doc.content_size();
        let mut tr = state.tr();
        tr.replace(at, at, Slice::new(Fragment::from_node(list), 0, 0))
            .ok()?;
        Some(tr)
    }));

    // The host's model changed locally, but the projection failed loud (the
    // CRDT was left untouched, all-or-nothing) so the peer received nothing.
    assert!(
        host.collab_take_error().is_some(),
        "an unsupported edit surfaces a fail-loud error"
    );
    assert_eq!(
        doc_text(&guest),
        "ok",
        "the peer is untouched by an unsupported local edit (no partial sync)"
    );
}
