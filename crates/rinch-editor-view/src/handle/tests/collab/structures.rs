//! Inline atoms, lists, quotes and tables reaching the peer, and the wiring
//! fuzz.

use super::*;

#[test]
fn inline_atom_edits_sync_to_the_peer() {
    // The handle-level pin on PlotWeb's symptom: a hard break or an image used
    // to stop the body syncing the moment it appeared, while the editor still
    // reported a clean save. Both are inside the projected scope now, so
    // neither may fail loud, and the guest must actually receive them.
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "one two")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);
    assert_eq!(doc_text(&guest), "one two");

    // Shift+Enter between the words.
    host.set_selection(Selection::cursor(Pos(5)));
    assert!(host.command("insertHardBreak"));
    assert!(
        host.collab_take_error().is_none(),
        "a hard break is supported and must not fail loud"
    );

    // An image paste, which is an inline atom carrying attrs.
    assert!(host.insert_image("data:image/png;base64,AAAA", "shot"));
    assert!(
        host.collab_take_error().is_none(),
        "an image is supported and must not fail loud"
    );

    // The guest holds the same document, atoms and all. Compared by *shape*,
    // not by `==`: the two handles are mounted with their own `Schema`
    // instances, and node/mark type equality is `Rc::ptr_eq` (issue #217).
    let line = guest.doc().child(0).clone();
    let kinds: Vec<String> = (0..line.child_count())
        .map(|i| line.child(i).type_name().to_string())
        .collect();
    assert_eq!(
        kinds,
        vec!["text", "hard_break", "image", "text"],
        "the peer received the inline atoms, not just the text around them"
    );
    assert_eq!(
        line.child(2).attrs().get_str("src"),
        Some("data:image/png;base64,AAAA"),
        "with the image's attrs intact"
    );
    assert!(
        guest.collab_take_error().is_none(),
        "and integrated them without an error"
    );

    // Ordinary typing keeps flowing afterwards — the "it went quiet" symptom.
    host.set_selection(Selection::cursor(Pos(1)));
    assert!(host.insert_text("Z"));
    assert!(
        doc_text(&guest).starts_with('Z'),
        "typing still reaches the peer"
    );
}

#[test]
fn list_edits_sync_to_the_peer() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "ok")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);
    assert_eq!(doc_text(&guest), "ok");

    // Lists are inside the projected scope, so this must sync rather than
    // fail loud (the counterpart to the task list case in `convergence.rs`).
    assert!(host.load_html("<ul><li><p>item</p></li></ul>"));
    assert!(
        host.collab_take_error().is_none(),
        "a bullet list is supported and must not fail loud"
    );
    assert_eq!(
        doc_text(&guest),
        "item",
        "the peer receives the list content"
    );

    // A nested list survives the round-trip too.
    assert!(host.load_html("<ol><li><p>a</p><ul><li><p>b</p></li></ul></li></ol>"));
    assert!(host.collab_take_error().is_none());
    assert_eq!(
        doc_text(&guest),
        "ab",
        "nested list content reaches the peer"
    );
}

#[test]
fn quote_edits_sync_to_the_peer() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "ok")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);
    assert_eq!(doc_text(&guest), "ok");

    // Quotes are inside the projected scope: a loaded one syncs.
    assert!(host.load_html("<blockquote><p>quoted</p><ul><li><p>li</p></li></ul></blockquote>"));
    assert!(
        host.collab_take_error().is_none(),
        "a blockquote is supported and must not fail loud"
    );
    assert_eq!(doc_text(&guest), "quotedli");
    // Compared by shape: each mount builds its own schema, and node types
    // compare by schema identity.
    let shape = |h: &EditorHandle| format!("{:?}", h.doc());
    assert_eq!(
        shape(&guest),
        shape(&host),
        "the guest holds the same quote"
    );

    // Typing inside it, and lifting the paragraph out, sync as well.
    host.set_selection(Selection::cursor(Pos(8)));
    assert!(host.insert_text("!"));
    assert!(host.command("liftListItem"));
    assert!(host.collab_take_error().is_none());
    assert_eq!(doc_text(&guest), "quoted!\nli", "one block per line");
    assert_eq!(shape(&guest), shape(&host));
    assert_eq!(guest.doc().child(0).type_name(), "paragraph");
}

#[test]
fn table_edits_sync_to_the_peer() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "ok")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);
    let shape = |h: &EditorHandle| format!("{:?}", h.doc());

    // A table, typing in a cell, a column and a merge: every one syncs.
    host.set_selection(Selection::cursor(Pos(3)));
    assert!(host.command("insertTable"));
    assert!(
        host.collab_take_error().is_none(),
        "a table is supported and must not fail loud"
    );
    let doc = host.doc();
    let table_at = (0..doc.child_count())
        .take_while(|&i| doc.child(i).type_name() != "table")
        .map(|i| doc.child(i).node_size())
        .sum::<usize>();
    // table, row, cell, paragraph: the first cell's text starts four in.
    host.set_selection(Selection::cursor(Pos(table_at + 4)));
    assert!(host.insert_text("cell"));
    assert!(host.command("addColumnAfter"));
    assert!(host.collab_take_error().is_none());
    assert_eq!(
        shape(&guest),
        shape(&host),
        "the guest holds the same table"
    );
    assert!(doc_text(&guest).contains("cell"));
}

/// Seeded fuzz over the real `EditorHandle` wiring: two handles relay random
/// edits (via the `outbound` sink) and integrate them (`collab_receive`) in
/// interleaved order, then must converge to the *identical* document
/// (structural — marks included, which is what the mark-order fix guarantees).
#[test]
fn fuzz_handle_wiring_converges() {
    struct Rng(u64);
    impl Rng {
        fn new(s: u64) -> Rng {
            Rng(s ^ 0x9E37_79B9_7F4A_7C15)
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
            if n == 0 {
                0
            } else {
                (self.next() % n as u64) as usize
            }
        }
    }

    for seed in 0..6u64 {
        let s = schema();
        let host = mount(doc_node(&s, vec![para(&s, "start")])).handle;
        let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
        // Each peer's outbound pushes into the OTHER peer's inbox.
        let to_host: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let to_guest: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let tg = to_guest.clone();
        let snap = host
            .start_collaboration_host(move |d| tg.borrow_mut().push(d))
            .unwrap();
        let th = to_host.clone();
        guest
            .start_collaboration_guest(&snap, move |d| th.borrow_mut().push(d))
            .unwrap();

        let peers = [&host, &guest];
        let inbox = [&to_host, &to_guest]; // inbox[p] = deltas destined for peer p
        let mut seen = [0usize, 0usize];
        let mut rng = Rng::new(seed);

        let deliver = |p: usize, seen: &mut [usize; 2]| {
            let delta = {
                let q = inbox[p].borrow();
                (seen[p] < q.len()).then(|| q[seen[p]].clone())
            };
            if let Some(d) = delta {
                seen[p] += 1;
                peers[p].collab_receive(&d);
            }
        };

        for _ in 0..140 {
            if rng.below(100) < 65 {
                let p = rng.below(2);
                let h = peers[p];
                let doc = h.doc();
                let size = doc.content().size();
                let pos = 1 + rng.below(size.max(1));
                h.set_selection(Selection::near(&doc, Pos(pos.min(size)), 1));
                match rng.below(5) {
                    0..=2 => {
                        h.insert_text(["a", "b", " ", "猫", "Z"][rng.below(5)]);
                    }
                    3 => {
                        h.command(
                            ["toggleBold", "toggleItalic", "toggleStrike", "toggleCode"]
                                [rng.below(4)],
                        );
                    }
                    _ => {
                        h.command(["setHeading1", "setParagraph", "splitBlock"][rng.below(3)]);
                    }
                }
            } else {
                deliver(rng.below(2), &mut seen);
            }
        }
        // Flush both inboxes.
        for p in 0..2 {
            while seen[p] < inbox[p].borrow().len() {
                deliver(p, &mut seen);
            }
        }

        // Compare via HTML, not `Node` equality: each handle has its OWN
        // `Schema` instance (`create_editor`/`mount` make one per editor), and
        // `Node` equality compares `NodeType` by interned-pointer identity, so
        // structurally-identical docs from different schemas compare unequal.
        // HTML is schema-independent and captures text + marks + block type, so
        // it's the faithful cross-handle convergence check. (Full structural
        // convergence under one schema is proven by the adapter fuzz.)
        use rinch_editor_core::serialize::node_to_html;
        assert_eq!(
            node_to_html(&host.doc()),
            node_to_html(&guest.doc()),
            "handles diverged (seed={seed})"
        );
    }
}

// ── Sticky positions (deep links) ────────────────────────────────────
