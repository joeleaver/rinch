//! Read-only collaborators.

use super::*;

/// A host and a **read-only** guest on one loopback. The guest was locked
/// before it joined, so the join itself is part of what is proven. Returns
/// the guest's harness (for its mock host) and a counter of every delta the
/// guest tried to send.
fn read_only_guest(host: &EditorHandle) -> (Harness, Rc<Cell<usize>>) {
    let s = schema();
    let guest = mount(doc_node(&s, vec![para(&s, "stale local")]));
    guest.handle.set_read_only(true);
    let guest_in = guest.handle.clone();
    let snapshot = host
        .start_collaboration_host(move |delta| {
            guest_in.collab_receive(&delta);
        })
        .expect("host projects its document");
    let sent = Rc::new(Cell::new(0usize));
    let (host_in, counter) = (host.clone(), sent.clone());
    guest
        .handle
        .start_collaboration_guest(&snapshot, move |delta| {
            counter.set(counter.get() + 1);
            host_in.collab_receive(&delta);
        })
        .expect("a read-only guest joins");
    (guest, sent)
}

/// The first block of a harness's **host** — what is on screen.
fn shown(h: &Harness) -> Option<String> {
    text(h, children(h, h.container_id)[0])
}

/// Remote deltas are applied and shown exactly as in an editable editor:
/// integration never passes through the gate local edits pass through.
#[test]
fn a_read_only_guest_joins_and_keeps_receiving_remote_edits() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "shared")])).handle;
    let (guest, sent) = read_only_guest(&host);
    assert_eq!(doc_text(&guest.handle), "shared", "adopted on join");
    assert_eq!(shown(&guest).as_deref(), Some("shared"), "and shown");
    assert!(guest.handle.is_read_only() && guest.handle.is_collaborating());

    // Typing, a mark, a new block and a deletion, all from the peer.
    host.set_selection(Selection::cursor(Pos(7)));
    assert!(host.insert_text(" text"));
    assert_eq!(doc_text(&guest.handle), "shared text");
    assert_eq!(
        shown(&guest).as_deref(),
        Some("shared text"),
        "the view follows"
    );

    host.set_selection(Selection::text(Pos(1), Pos(7)));
    assert!(host.command("toggleBold"));
    assert_eq!(
        guest.handle.doc().child(0).child(0).marks().len(),
        1,
        "a remote mark arrives"
    );

    host.set_selection(Selection::cursor(Pos(12)));
    assert!(host.command("enter"));
    assert!(host.insert_text("second"));
    assert_eq!(doc_text(&guest.handle), "shared text\nsecond");
    assert_eq!(children(&guest, guest.container_id).len(), 2);

    host.set_selection(Selection::text(Pos(1), Pos(8)));
    assert!(host.command("deleteSelection"));
    assert_eq!(doc_text(&guest.handle), "text\nsecond");
    assert_eq!(shown(&guest).as_deref(), Some("text"));

    // A reconciliation diff is a remote update like any other.
    let sv = guest.handle.collab_state_vector().unwrap();
    let diff = host.collab_sync_diff(&sv).unwrap();
    guest.handle.collab_receive(&diff);
    assert_eq!(doc_text(&guest.handle), doc_text(&host));
    assert_eq!(
        sent.get(),
        0,
        "and through all of it the guest sent nothing"
    );
}

/// Local insert, delete, paste, format and undo on a read-only collaborator:
/// refused, and the CRDT is byte-for-byte what it was — same snapshot, same
/// state vector, nothing handed to `outbound`, nothing at the peer.
#[test]
fn a_read_only_guest_changes_nothing_shared() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello world")])).handle;
    let (guest, sent) = read_only_guest(&host);
    let guest = guest.handle;
    // Something to undo, had the guest been allowed: the host's edit is in
    // the guest's document, and a local history entry is made below.
    host.set_selection(Selection::cursor(Pos(12)));
    assert!(host.insert_text("!"));
    assert_eq!(doc_text(&guest), "hello world!");

    let (snapshot, sv) = (guest.collab_snapshot(), guest.collab_state_vector());
    let (doc, host_doc) = (guest.doc(), host.doc());

    guest.set_selection(Selection::text(Pos(7), Pos(12)));
    assert!(!guest.insert_text("X"), "insert");
    assert!(!guest.command("deleteSelection"), "delete");
    assert!(!guest.command("deleteCharBackward"), "backspace");
    assert!(!guest.replace_selection_with_text("pasted"), "paste");
    assert!(!guest.replace_selection_with_html("<p><i>rich</i></p>"));
    assert!(!guest.command("toggleBold"), "format");
    assert!(!guest.command("setHeading2"), "block format");
    assert!(!guest.command("undo"), "undo");
    assert!(!guest.command("redo"), "redo");
    guest.ime_commit("ね");

    assert!(
        guest.doc().same_ref(&doc),
        "the guest's document is untouched"
    );
    assert_eq!(
        guest.collab_snapshot(),
        snapshot,
        "the CRDT bytes are unchanged"
    );
    assert_eq!(
        guest.collab_state_vector(),
        sv,
        "and so is the state vector"
    );
    assert_eq!(sent.get(), 0, "outbound never fired");
    assert!(host.doc().same_ref(&host_doc), "the peer saw nothing");
    assert!(guest.collab_take_error().is_none());
}

/// The switch is a runtime one (a role changes): off, the same session
/// edits and broadcasts again; on again, it stops — with the peer's edits
/// arriving throughout.
#[test]
fn flipping_read_only_mid_session_stops_and_restores_editing() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "ab")])).handle;
    let (guest, sent) = read_only_guest(&host);
    let guest = guest.handle;

    guest.set_selection(Selection::cursor(Pos(3)));
    assert!(!guest.insert_text("c"));

    guest.set_read_only(false);
    assert!(guest.insert_text("c"), "editable again");
    assert_eq!(doc_text(&host), "abc", "and the edit reached the peer");
    assert_eq!(sent.get(), 1);
    // Its own history works: undo of its own edit is an edit, and travels.
    assert!(guest.command("undo"));
    assert_eq!(doc_text(&host), "ab");
    assert!(guest.command("redo"));
    assert_eq!(doc_text(&host), "abc");
    let sent_while_editable = sent.get();

    guest.set_read_only(true);
    assert!(!guest.insert_text("d"));
    assert!(
        !guest.command("undo"),
        "its own history is locked again too"
    );
    host.set_selection(Selection::cursor(Pos(1)));
    assert!(host.insert_text(">"));
    assert_eq!(doc_text(&guest), ">abc", "inbound never stopped");
    assert_eq!(doc_text(&host), ">abc");
    assert_eq!(
        sent.get(),
        sent_while_editable,
        "locked: nothing more went out"
    );
}

/// With a session attached a load is a write to the shared document — it
/// would be recorded and broadcast — so a read-only collaborator refuses it.
/// Detached, the same load is the app showing a document, and works.
#[test]
fn a_read_only_collaborator_refuses_a_load_until_it_detaches() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "shared")])).handle;
    let (guest, sent) = read_only_guest(&host);
    let snapshot = guest.handle.collab_snapshot();

    assert!(
        !guest.handle.load_html("<p>replaced</p>"),
        "reported as refused"
    );
    guest
        .handle
        .load_doc(doc_node(&s, vec![para(&s, "replaced")]));
    assert_eq!(doc_text(&guest.handle), "shared");
    assert_eq!(doc_text(&host), "shared", "the peer's document survives");
    assert_eq!(guest.handle.collab_snapshot(), snapshot);
    assert_eq!(sent.get(), 0);

    guest.handle.stop_collaboration();
    assert!(guest.handle.load_html("<p>next document</p>"));
    assert_eq!(shown(&guest).as_deref(), Some("next document"));
    assert_eq!(doc_text(&host), "shared");
}

/// `Editor { content, read_only: true }` mounted onto a handle that is
/// **already collaborating**: the component locks before it loads, so its
/// `content` is refused like any other write to a shared document rather
/// than being sent to the peers of a document this user was just told they
/// may not change. The order is observable nowhere else — with no session a
/// load is never refused, so an ordinary mount still fills the editor
/// (`the_editor_components_read_only_prop_only_ever_locks`).
#[test]
fn the_content_prop_does_not_write_a_collaborating_read_only_editors_shared_doc() {
    use rinch_core::Component;
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "shared")])).handle;
    let snapshot = host.start_collaboration_host(|_| {}).unwrap();

    let pane = crate::create_editor();
    let sent = Rc::new(Cell::new(0usize));
    let counter = sent.clone();
    pane.start_collaboration_guest(&snapshot, move |_| counter.set(counter.get() + 1))
        .unwrap();

    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
    let root = doc.borrow_mut().create_element("div");
    let mut scope = RenderScope::new(doc.clone(), root);
    let container = crate::Editor {
        editor: Some(pane.clone()),
        content: "<p>mine</p>".into(),
        read_only: true,
        spellcheck: None,
        ..Default::default()
    }
    .render(&mut scope, &[]);

    assert_eq!(
        doc_text(&pane),
        "shared",
        "the shared document is what shows"
    );
    assert_eq!(doc_text(&host), "shared", "and the peer's is untouched");
    assert_eq!(sent.get(), 0, "nothing was broadcast");
    assert_eq!(
        container.get_attribute("data-pm-readonly").as_deref(),
        Some("true")
    );
}

/// One editor pane moving from document to document (Pimble's shape), and
/// the app forgot `stop_collaboration` in between. The join adopts the new
/// document — read-only or not — and the *old* session's peers are not sent
/// the new document as an edit to theirs.
#[test]
fn joining_with_a_stale_session_attached_adopts_the_new_document_and_spares_the_old() {
    let s = schema();
    for read_only in [true, false] {
        let first = mount(doc_node(&s, vec![para(&s, "first document")])).handle;
        let second = mount(doc_node(&s, vec![para(&s, "second document")])).handle;
        let pane = mount(doc_node(&s, vec![para(&s, "")]));
        pane.handle.set_read_only(read_only);

        let to_first = Rc::new(Cell::new(0usize));
        let snapshot = first.start_collaboration_host(|_| {}).unwrap();
        let (first_in, counter) = (first.clone(), to_first.clone());
        pane.handle
            .start_collaboration_guest(&snapshot, move |delta| {
                counter.set(counter.get() + 1);
                first_in.collab_receive(&delta);
            })
            .unwrap();
        assert_eq!(doc_text(&pane.handle), "first document");

        // No `stop_collaboration` here.
        let snapshot = second.start_collaboration_host(|_| {}).unwrap();
        pane.handle
            .start_collaboration_guest(&snapshot, |_| {})
            .unwrap();
        assert_eq!(
            doc_text(&pane.handle),
            "second document",
            "read_only={read_only}: the join adopted the new document"
        );
        assert_eq!(shown(&pane).as_deref(), Some("second document"));
        assert_eq!(
            (to_first.get(), doc_text(&first).as_str()),
            (0, "first document"),
            "read_only={read_only}: the first document's peers were sent nothing"
        );
    }
}
