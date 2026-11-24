//! Convergence: local, remote and concurrent edits, and the sync protocol.

use super::*;

#[test]
fn guest_adopts_host_document_on_join() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "shared title")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "stale local")])).handle;
    loopback(&host, &guest);
    assert_eq!(
        doc_text(&guest),
        "shared title",
        "the guest adopts the host's converged document"
    );
    assert!(host.is_collaborating() && guest.is_collaborating());
}

#[test]
fn local_edits_converge_both_directions() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);

    // Type in the host → the guest converges.
    host.set_selection(Selection::cursor(Pos(6))); // end of "hello"
    assert!(host.insert_text(" world"));
    assert_eq!(doc_text(&host), "hello world");
    assert_eq!(
        doc_text(&guest),
        "hello world",
        "host edit reached the guest"
    );

    // Type in the guest → the host converges.
    guest.set_selection(Selection::cursor(Pos(1))); // start of the block
    assert!(guest.insert_text("X"));
    assert_eq!(doc_text(&guest), "Xhello world");
    assert_eq!(
        doc_text(&host),
        "Xhello world",
        "guest edit reached the host"
    );
}

#[test]
fn marks_converge() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "abcd")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);

    host.set_selection(Selection::text(Pos(1), Pos(5)));
    assert!(host.command("toggleBold"));
    // The guest's projected document carries the bold mark on the run.
    let guest_doc = guest.doc();
    let run = guest_doc.child(0).child(0);
    assert_eq!(run.text(), Some("abcd"));
    assert!(
        !run.marks().is_empty(),
        "the bold mark projected through the CRDT to the guest"
    );
}

#[test]
fn concurrent_edits_converge() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

    // Buffer deltas instead of delivering them, so both peers edit against
    // the same base — a genuine concurrent edit.
    let to_guest: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let tg = to_guest.clone();
    let snapshot = host
        .start_collaboration_host(move |d| tg.borrow_mut().push(d))
        .unwrap();
    let th = to_host.clone();
    guest
        .start_collaboration_guest(&snapshot, move |d| th.borrow_mut().push(d))
        .unwrap();

    // Concurrent: host appends, guest prepends — neither has seen the other.
    host.set_selection(Selection::cursor(Pos(6)));
    assert!(host.insert_text("H"));
    guest.set_selection(Selection::cursor(Pos(1)));
    assert!(guest.insert_text("G"));

    // Exchange both deltas.
    for d in to_guest.borrow_mut().drain(..) {
        guest.collab_receive(&d);
    }
    for d in to_host.borrow_mut().drain(..) {
        host.collab_receive(&d);
    }

    // CRDT convergence: identical documents on both peers.
    let h = doc_text(&host);
    let g = doc_text(&guest);
    assert_eq!(h, g, "concurrent edits converge to one document");
    assert!(
        h.contains('H') && h.contains('G'),
        "both edits survived: {h}"
    );
}

/// Reconcile two handles by exchanging state vectors and the diffs they imply —
/// the shape a networked caller uses when it cannot trust delta delivery.
///
/// The exchange is unconditional in both directions: equal state vectors would
/// not prove convergence (they summarise insertions only), so settling is
/// detected by both sides integrating without a document change.
fn sync_until_quiet(a: &EditorHandle, b: &EditorHandle) {
    for _ in 0..20 {
        let a_sv = a.collab_state_vector().expect("a is collaborating");
        let b_sv = b.collab_state_vector().expect("b is collaborating");
        let to_b = a.collab_sync_diff(&b_sv).expect("diff for b");
        let to_a = b.collab_sync_diff(&a_sv).expect("diff for a");
        let b_changed = b.collab_receive(&to_b);
        let a_changed = a.collab_receive(&to_a);
        if !a_changed && !b_changed {
            return;
        }
    }
    panic!("the state-vector exchange did not settle");
}

#[test]
fn the_sync_protocol_converges_two_peers_whose_deltas_never_arrived() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

    // Deltas go nowhere: this models the transport reconciliation exists for —
    // an HTTP poll, a dropped socket, a peer that was offline. The broadcast
    // path alone would leave these two permanently diverged.
    let snapshot = host.start_collaboration_host(|_d| {}).unwrap();
    guest.start_collaboration_guest(&snapshot, |_d| {}).unwrap();

    host.set_selection(Selection::cursor(Pos(6)));
    assert!(host.insert_text(" world"));
    guest.set_selection(Selection::cursor(Pos(1)));
    assert!(guest.insert_text("G"));
    assert_ne!(
        doc_text(&host),
        doc_text(&guest),
        "precondition: no delta was delivered, so the peers diverged"
    );

    sync_until_quiet(&host, &guest);

    let h = doc_text(&host);
    assert_eq!(h, doc_text(&guest), "reconciliation converged the peers");
    assert!(
        h.contains("world") && h.contains('G'),
        "both offline edits survived: {h}"
    );
    assert_eq!(
        host.collab_state_vector(),
        guest.collab_state_vector(),
        "converged peers have seen the same changes"
    );
}

/// Issue #220, end to end through the handle. A horizontal rule is outside the
/// staged A22 scope, so inserting one while collaborating cannot be projected.
/// The editor still applies it — the model is the source of truth — so from that
/// point the local document holds a block the CRDT does not, and outbound
/// stalls.
///
/// What used to happen: every later edit failed a block-count check whose
/// message named neither the cause nor the cure ("the model document holds 2
/// block(s) but the CRDT holds 1"), *including the deletion that was supposed
/// to be the cure*; and when the counts finally realigned, the diff skipped
/// every block it believed unchanged, so text typed during the stall stayed
/// local forever while `record_local` answered `Ok`.
///
/// What must happen now: the stall is visible and self-describing, nothing
/// reaches the guest while it holds, and deleting the rule ships the whole
/// backlog in one delta.
#[test]
fn an_out_of_scope_edit_stalls_outbound_and_removing_it_ships_the_backlog() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);
    assert_eq!(doc_text(&guest), "hello");
    assert!(host.collab_outbound_stall().is_none(), "healthy to start");

    // Append a task list — applied locally, refused by the projection. (A
    // `horizontal_rule` and then a `blockquote` used to stand here; both are
    // inside the projected scope now, so the stall needs content that is still
    // outside it.)
    assert!(
        host.update(|state| {
            let s = state.schema().clone();
            let inner = s
                .branch("paragraph", Fragment::from_node(s.text("q").ok()?))
                .ok()?;
            let item = s.branch("task_item", Fragment::from_node(inner)).ok()?;
            let bq = s.branch("task_list", Fragment::from_node(item)).ok()?;
            let at = state.doc.content_size();
            let mut tr = state.tr();
            tr.replace(at, at, Slice::new(Fragment::from_node(bq), 0, 0))
                .ok()?;
            Some(tr)
        }),
        "the editor applies it"
    );
    let stall = host
        .collab_outbound_stall()
        .expect("outbound must report itself stalled");
    assert!(
        stall.to_string().contains("task_list"),
        "the stall must name the content to remove, got: {stall}"
    );
    assert!(
        !host.is_collaboration_poisoned(),
        "an out-of-scope LOCAL edit is not poison — the shared doc is healthy"
    );

    // Typing while stalled stays local — this is the text that used to be lost.
    host.set_selection(Selection::cursor(Pos(6)));
    assert!(host.insert_text("!!"));
    assert!(
        host.collab_outbound_stall().is_some(),
        "still stalled while the task list is there"
    );
    assert_eq!(
        doc_text(&guest),
        "hello",
        "nothing reached the guest during the stall"
    );

    // Delete the task list — selecting it and pressing Delete, as an app would. Note
    // this is NOT `undo`: the text typed during the stall stays, which is the
    // half that must survive.
    assert!(
        host.update(|state| {
            let doc = &state.doc;
            let last = doc.child_count() - 1;
            let from: usize = (0..last).map(|i| doc.child(i).node_size()).sum();
            let to = from + doc.child(last).node_size();
            let mut tr = state.tr();
            tr.delete(from, to).ok()?;
            Some(tr)
        }),
        "the task list is deleted"
    );
    assert!(
        host.collab_outbound_stall().is_none(),
        "removing the offending content resumes outbound: {:?}",
        host.collab_outbound_stall().map(|e| e.to_string())
    );
    assert_eq!(
        doc_text(&host),
        "hello!!",
        "the local document kept what was typed during the stall"
    );
    assert_eq!(
        doc_text(&guest),
        doc_text(&host),
        "and the guest caught up on all of it in one delta"
    );
}

#[test]
fn sync_methods_are_inert_without_a_session() {
    let s = schema();
    let solo = mount(doc_node(&s, vec![para(&s, "local only")])).handle;
    assert!(solo.collab_state_vector().is_none());
    assert!(solo.collab_sync_diff(&[0]).is_none());
    assert_eq!(doc_text(&solo), "local only", "the document is untouched");
}

#[test]
fn integrating_a_reconciliation_diff_does_not_echo() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

    let snapshot = host.start_collaboration_host(|_d| {}).unwrap();
    // A change that arrives by reconciliation must not fire the broadcast sink
    // either — a caller running both paths would otherwise loop.
    let guest_emits = Rc::new(Cell::new(0usize));
    let ge = guest_emits.clone();
    guest
        .start_collaboration_guest(&snapshot, move |_d| ge.set(ge.get() + 1))
        .unwrap();

    host.set_selection(Selection::cursor(Pos(3)));
    assert!(host.insert_text("c"));
    sync_until_quiet(&host, &guest);

    assert_eq!(doc_text(&guest), "abc", "host edit reached the guest");
    assert_eq!(
        guest_emits.get(),
        0,
        "integrating a reconciliation diff must not broadcast a delta back"
    );
}

#[test]
fn reconciliation_carries_a_deletion_that_the_state_vector_cannot_express() {
    // A state vector counts insertions, so a peer that only *deleted* leaves it
    // unchanged. If any part of the path short-circuits on state-vector equality
    // the deletion is silently lost — which is exactly the class of bug the fuzz
    // suites caught during the engine swap, so it is pinned here at the handle
    // level too.
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "abcdef")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

    let snapshot = host.start_collaboration_host(|_d| {}).unwrap();
    guest.start_collaboration_guest(&snapshot, |_d| {}).unwrap();
    assert_eq!(doc_text(&guest), "abcdef", "guest joined on the host's doc");

    let sv_before = host.collab_state_vector().unwrap();
    host.update(|st| {
        let mut tr = st.tr();
        tr.delete(3, 5).ok()?; // drop "cd"
        Some(tr)
    });
    assert_eq!(doc_text(&host), "abef", "the host really deleted");
    assert_eq!(
        host.collab_state_vector().unwrap(),
        sv_before,
        "precondition: a delete-only change leaves the state vector untouched"
    );

    sync_until_quiet(&host, &guest);
    assert_eq!(doc_text(&guest), "abef", "the deletion reached the guest");
}

#[test]
fn integrating_a_remote_delta_does_not_echo() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;

    let guest_in = guest.clone();
    let snapshot = host
        .start_collaboration_host(move |d| {
            guest_in.collab_receive(&d);
        })
        .unwrap();
    // Count the guest's outbound emissions: integrating the host's delta
    // must NOT produce one (no echo / infinite loop).
    let guest_emits = Rc::new(Cell::new(0usize));
    let ge = guest_emits.clone();
    guest
        .start_collaboration_guest(&snapshot, move |_d| ge.set(ge.get() + 1))
        .unwrap();

    host.set_selection(Selection::cursor(Pos(3)));
    assert!(host.insert_text("c"));
    assert_eq!(doc_text(&guest), "abc", "host edit applied on the guest");
    assert_eq!(
        guest_emits.get(),
        0,
        "integrating a remote delta must not broadcast one back"
    );
}

#[test]
fn selection_only_change_broadcasts_nothing() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "abc")])).handle;
    let emits = Rc::new(Cell::new(0usize));
    let e = emits.clone();
    host.start_collaboration_host(move |_d| e.set(e.get() + 1))
        .unwrap();

    host.set_selection(Selection::cursor(Pos(2)));
    assert_eq!(emits.get(), 0, "moving the cursor broadcasts nothing");
    assert!(host.insert_text("X"));
    assert_eq!(emits.get(), 1, "a text edit broadcasts exactly one delta");
}

#[test]
fn stop_collaboration_silences_broadcasts() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
    let emits = Rc::new(Cell::new(0usize));
    let e = emits.clone();
    host.start_collaboration_host(move |_d| e.set(e.get() + 1))
        .unwrap();
    assert!(host.is_collaborating());

    host.stop_collaboration();
    assert!(!host.is_collaborating());
    host.set_selection(Selection::cursor(Pos(3)));
    assert!(host.insert_text("c"));
    assert_eq!(emits.get(), 0, "a detached editor broadcasts nothing");
}

#[test]
fn late_joiner_adopts_current_content_via_snapshot() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);

    // Edit AFTER the original join snapshot.
    host.set_selection(Selection::cursor(Pos(6)));
    assert!(host.insert_text(" world"));
    assert_eq!(doc_text(&guest), "hello world");

    // A third peer joins late from the host's CURRENT snapshot — it must
    // adopt the edited content, not the host's original document.
    let late = mount(doc_node(&s, vec![para(&s, "stale")])).handle;
    let snapshot = host.collab_snapshot().expect("host is collaborating");
    late.start_collaboration_guest(&snapshot, |_d| {}).unwrap();
    assert_eq!(
        doc_text(&late),
        "hello world",
        "a late joiner adopts the current shared document"
    );
}

// ── The poisoned session (issue #196) ────────────────────────────────
//
// Foreign bytes whose `content` root was created as the wrong yrs type leave
// the shared CRDT unprojectable once integrated, with nothing pending that
// could cure it (yrs has no rollback). The session must then go loud in BOTH
// directions — sticky `SessionPoisoned` on inbound and outbound — instead of
// one-way partitioning: keeping `record_local` Ok and broadcasting while
// every receive fails was the dangerous silent half. (Inbound stays
// *attempted*: an update that makes the document rebuildable again clears
// the poison — pinned at the session level in tests/poison.rs.)
