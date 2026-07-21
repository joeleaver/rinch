//! End-to-end collaboration tests (design M9): two `EditorState`s + two adapters,
//! concurrent insert/format, convergence after sync, rebase of local steps over a
//! remote mapping, and the fail-loud boundary on unsupported content.
//!
//! Convergence is asserted on `projected_doc()` — the canonical read-back of each
//! peer's converged CRDT — which both peers reconstruct identically, so equal CRDTs
//! give structurally-equal model docs.

use std::rc::Rc;

use rinch_editor_collab::{CollabPlugin, CollabSession, RemoteOp, rebase_steps};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Mark, Node, Plugin, Pos, Schema, Selection, Slice,
    Step, Transaction, default_plugins,
};

// --- harness -------------------------------------------------------------------

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn para(schema: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(schema.text(text).unwrap())
    };
    schema.branch("paragraph", content).unwrap()
}

fn doc_of(schema: &Schema, blocks: Vec<Node>) -> Node {
    schema
        .branch("doc", Fragment::from_children(blocks))
        .unwrap()
}

fn list_item(schema: &Schema, blocks: Vec<Node>) -> Node {
    schema
        .branch("list_item", Fragment::from_children(blocks))
        .unwrap()
}

fn bullet_list(schema: &Schema, items: Vec<Node>) -> Node {
    schema
        .branch("bullet_list", Fragment::from_children(items))
        .unwrap()
}

fn ordered_list(schema: &Schema, start: i64, items: Vec<Node>) -> Node {
    schema
        .create_node(
            "ordered_list",
            Attrs::new().with("start", start),
            Fragment::from_children(items),
        )
        .unwrap()
}

/// A fully recursive canonical string form of a document (nesting-aware), so two
/// structurally-equal trees — however deeply nested — compare equal regardless of how
/// text/marks/children were assembled. `norm` above is the flat-only sibling the
/// original tests use.
fn tree(n: &Node) -> String {
    if let Some(t) = n.text() {
        return format!("«{}|{}»", t, canon_marks(n).join("+"));
    }
    let mut s = format!("<{} {}>", n.type_name(), norm_attrs(n));
    for i in 0..n.child_count() {
        s.push_str(&tree(n.child(i)));
    }
    s.push_str("</>");
    s
}

/// One collaborating editor: its model state plus its CRDT session.
struct Peer {
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    /// Apply a local transaction (built by `f`) and project it onto the CRDT.
    fn local(&mut self, f: impl FnOnce(&mut Transaction)) {
        let mut tr = self.state.tr();
        f(&mut tr);
        let before = self.state.doc.clone();
        let after = self.state.apply(tr);
        self.session
            .record_local(&before, &after.doc)
            .expect("project local");
        self.state = after;
    }

    fn type_at(&mut self, pos: usize, text: &str) {
        self.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(pos)));
            tr.insert_text(text).unwrap();
        });
    }

    fn bold(&mut self, from: usize, to: usize) {
        let bold = Mark::simple(self.state.schema().mark_type("bold").unwrap().clone());
        self.local(|tr| {
            tr.add_mark(from, to, bold).unwrap();
        });
    }
}

/// Build two peers sharing a schema, both starting from `blocks`. Peer B joins from
/// peer A's CRDT snapshot.
fn two_peers(blocks: Vec<Node>) -> (Rc<Schema>, Peer, Peer) {
    let schema = Rc::new(Schema::starter_kit());
    let a_state = EditorState::create(schema.clone(), doc_of(&schema, blocks), plugins());
    let session_a = CollabSession::new(&a_state).expect("session A");
    let mut a = Peer {
        state: a_state,
        session: session_a,
    };

    let snap = a.session.snapshot();
    let session_b = CollabSession::from_bytes(&snap).expect("session B");
    let b_doc = session_b.projected_doc(&schema).expect("project B");
    let b_state = EditorState::create(schema.clone(), b_doc, plugins());
    let b = Peer {
        state: b_state,
        session: session_b,
    };
    (schema, a, b)
}

/// Run the full Automerge sync protocol between two peers until convergence.
fn sync(a: &mut Peer, b: &mut Peer) {
    use rinch_editor_collab::SyncState;
    let mut sa = SyncState::new();
    let mut sb = SyncState::new();
    for _ in 0..100 {
        let ma = a.session.generate_sync_message(&mut sa);
        let mb = b.session.generate_sync_message(&mut sb);
        let done = ma.is_none() && mb.is_none();
        if let Some(m) = ma
            && let Some(ns) = b
                .session
                .integrate_sync_message(&b.state, &mut sb, m)
                .expect("b integrate")
        {
            b.state = ns;
        }
        if let Some(m) = mb
            && let Some(ns) = a
                .session
                .integrate_sync_message(&a.state, &mut sa, m)
                .expect("a integrate")
        {
            a.state = ns;
        }
        if done {
            break;
        }
    }
}

/// A canonical, mark-order-independent, run-coalesced string form of a flat doc, so
/// two equal documents compare equal regardless of how text/marks were assembled.
fn norm(doc: &Node) -> String {
    let mut s = String::new();
    for bi in 0..doc.child_count() {
        let b = doc.child(bi);
        s.push_str(&format!("<{} {}>", b.type_name(), norm_attrs(b)));
        let mut runs: Vec<(Vec<String>, String)> = Vec::new();
        for ci in 0..b.child_count() {
            let c = b.child(ci);
            let text = c.text().unwrap_or("").to_string();
            let marks = canon_marks(c);
            match runs.last_mut() {
                Some(last) if last.0 == marks => last.1.push_str(&text),
                _ => runs.push((marks, text)),
            }
        }
        for (marks, text) in runs {
            s.push_str(&format!("«{}|{}»", text, marks.join("+")));
        }
        s.push_str("</>");
    }
    s
}

fn norm_attrs(n: &Node) -> String {
    let mut a: Vec<String> = n
        .attrs()
        .iter()
        .map(|(k, v)| format!("{k}={v:?}"))
        .collect();
    a.sort();
    a.join(",")
}

fn canon_marks(n: &Node) -> Vec<String> {
    let mut m: Vec<String> = n
        .marks()
        .iter()
        .map(|mk| {
            let mut a: Vec<String> = mk.attrs.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
            a.sort();
            format!("{}[{}]", mk.type_name(), a.join(","))
        })
        .collect();
    m.sort();
    m
}

fn assert_converged(a: &Peer, b: &Peer, schema: &Schema) {
    let pa = a.session.projected_doc(schema).unwrap();
    let pb = b.session.projected_doc(schema).unwrap();
    assert_eq!(norm(&pa), norm(&pb), "CRDT projections must converge");
    // The live models must also match their own converged projection (the
    // `model ≡ project(model)` invariant), and therefore each other.
    assert_eq!(norm(&a.state.doc), norm(&pa), "peer A model ≡ projection");
    assert_eq!(norm(&b.state.doc), norm(&pb), "peer B model ≡ projection");
}

// --- tests ---------------------------------------------------------------------

#[test]
fn projection_round_trips_text_and_marks() {
    let schema = Rc::new(Schema::starter_kit());
    // a heading + a paragraph with a bold run
    let h = schema
        .branch(
            "heading",
            Fragment::from_node(schema.text("Title").unwrap()),
        )
        .unwrap();
    let bold = Mark::simple(schema.mark_type("bold").unwrap().clone());
    let p = schema
        .create_node(
            "paragraph",
            Default::default(),
            Fragment::from_children(vec![
                schema.text("plain ").unwrap(),
                schema.text_with_marks("bold", vec![bold]).unwrap(),
            ]),
        )
        .unwrap();
    let doc = doc_of(&schema, vec![h, p]);

    let cdoc = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap();
    let back = cdoc.to_doc(&schema).unwrap();
    assert_eq!(norm(&doc), norm(&back));
}

#[test]
fn concurrent_text_inserts_converge() {
    let schema = Rc::new(Schema::starter_kit());
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![para(&schema, "Hello")])
    };
    // pos 1 = start of paragraph content, pos 6 = end of "Hello".
    a.type_at(1, "A"); // -> "AHello"
    b.type_at(6, "B"); // -> "HelloB"
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    // both edits survived
    let text = norm(&a.session.projected_doc(&schema).unwrap());
    assert!(
        text.contains('A') && text.contains('B'),
        "both inserts kept: {text}"
    );
}

#[test]
fn concurrent_insert_and_format_converge() {
    // The design's headline test: one peer inserts text, the other formats — converge.
    let schema = Rc::new(Schema::starter_kit());
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![para(&schema, "Hello world")])
    };
    a.type_at(1, "XXX"); // "XXXHello world"
    b.bold(7, 12); // bold "world" (chars 6..11 -> model 7..12)
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    let conv = a.session.projected_doc(&schema).unwrap();
    let t = norm(&conv);
    assert!(t.contains("XXX"), "insert survived: {t}");
    assert!(t.contains("bold["), "bold survived: {t}");
}

#[test]
fn concurrent_block_split_and_text_edit_converge() {
    let schema = Rc::new(Schema::starter_kit());
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![para(&schema, "Hello world")])
    };
    // A splits the paragraph after "Hello" (model pos 6).
    a.local(|tr| {
        tr.set_selection(Selection::cursor(Pos(6)));
        tr.split(6, 1, None).unwrap();
    });
    // B edits the (still single) paragraph concurrently.
    b.type_at(1, "Z"); // "ZHello world"
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    let conv = a.session.projected_doc(&schema).unwrap();
    assert_eq!(
        conv.child_count(),
        2,
        "split produced two blocks after merge"
    );
}

#[test]
fn incremental_broadcast_converges() {
    let schema = Rc::new(Schema::starter_kit());
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![para(&schema, "start")])
    };
    a.type_at(6, " more");
    // broadcast delta from A to B
    let delta = a.session.save_incremental();
    if let Some(ns) = b.session.integrate_incremental(&b.state, &delta).unwrap() {
        b.state = ns;
    }
    assert_converged(&a, &b, &schema);
    assert!(norm(&b.state.doc).contains("start more"));
}

#[test]
fn remote_ops_translation_describes_a_remote_insert() {
    // patches → RemoteOp translation (design: remote patches translated to steps).
    let schema = Rc::new(Schema::starter_kit());
    let (_schema, mut a, mut b) = {
        let s = &schema;
        two_peers(vec![para(s, "Hello")])
    };
    let before = b.session.heads();
    a.type_at(6, " world"); // " world" at end
    let delta = a.session.save_incremental();
    b.session.integrate_incremental(&b.state, &delta).unwrap();
    let ops = b.session.remote_ops_since(&before).unwrap();
    assert!(
        ops.iter()
            .any(|o| matches!(o, RemoteOp::InsertText { text, .. } if text == " world")),
        "expected an InsertText op for the remote insert, got {ops:?}"
    );
}

#[test]
fn rebase_local_steps_over_a_remote_mapping() {
    // The design's rebase requirement: a local (unconfirmed) step rebased over a remote
    // change's mapping lands in the right place.
    let schema = Rc::new(Schema::starter_kit());
    let state = EditorState::create(
        schema.clone(),
        doc_of(&schema, vec![para(&schema, "Hello")]),
        plugins(),
    );

    // Local step: insert "Z" at the end of "Hello" (model pos 6).
    let local_steps: Vec<Box<dyn Step>> = {
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(6)));
        tr.insert_text("Z").unwrap();
        tr.steps().iter().map(|s| s.clone_box()).collect()
    };

    // Remote change: insert "XX" at the start (model pos 1). Capture its mapping
    // before applying (apply consumes the transaction).
    let mut remote_tr = state.tr();
    remote_tr.set_selection(Selection::cursor(Pos(1)));
    remote_tr.insert_text("XX").unwrap();
    let mapping = remote_tr.mapping().clone();
    let remote_state = state.apply(remote_tr);

    // Rebase the local step over the remote mapping, then apply it to the remote doc.
    let rebased = rebase_steps(&local_steps, &mapping);
    assert_eq!(rebased.len(), 1, "the step still applies after rebase");
    let mut tr = remote_state.tr();
    for s in rebased {
        tr.step(s).unwrap();
    }
    let final_doc = tr.doc();
    // "XXHello" + "Z" inserted after "Hello" (which is now at the end) -> "XXHelloZ".
    assert_eq!(all_text(final_doc), "XXHelloZ");
}

#[test]
fn concurrent_edits_in_different_blocks_converge() {
    let schema = Rc::new(Schema::starter_kit());
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![para(&schema, "one"), para(&schema, "two")])
    };
    // block 0 content: pos 1..4 ("one"); block 1 content starts at pos 6.
    a.type_at(4, "A"); // "oneA" in block 0
    b.type_at(9, "B"); // "twoB" in block 1 (6 +1 open... 6 is before block1 open; 7 inside; "two" 7..10, end 10) -> pos 9 = before 'o' end
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    let t = norm(&a.session.projected_doc(&schema).unwrap());
    assert!(
        t.contains('A') && t.contains('B'),
        "both block edits kept: {t}"
    );
}

#[test]
fn concurrent_mark_removal_and_typing_converge() {
    let schema = Rc::new(Schema::starter_kit());
    let bold = Mark::simple(schema.mark_type("bold").unwrap().clone());
    let bolded = schema
        .create_node(
            "paragraph",
            Default::default(),
            Fragment::from_node(schema.text_with_marks("word", vec![bold]).unwrap()),
        )
        .unwrap();
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![bolded])
    };
    // A removes the bold over "word" (model 1..5); B types at the end.
    let bold_a = Mark::simple(a.state.schema().mark_type("bold").unwrap().clone());
    a.local(|tr| {
        tr.remove_mark(1, 5, bold_a).unwrap();
    });
    b.type_at(5, "!");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
}

#[test]
fn concurrent_heading_level_change_and_typing_converge() {
    // Exercises the attrs-changed reconcile guard: a same-block attr edit and a text
    // edit converge, and the attr change is NOT clobbered by the concurrent typing.
    let schema = Rc::new(Schema::starter_kit());
    let h = schema
        .create_node(
            "heading",
            Attrs::new().with("level", 1i64),
            Fragment::from_node(schema.text("Title").unwrap()),
        )
        .unwrap();
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![h])
    };
    // A changes the heading level to 2 (attr edit on block 0, before its open token).
    a.local(|tr| {
        tr.set_node_attr(0, "level", AttrValue::Int(2)).unwrap();
    });
    // B types in the same heading concurrently.
    b.type_at(1, "X"); // "XTitle"
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    let conv = a.session.projected_doc(&schema).unwrap();
    assert_eq!(
        conv.child(0).attrs().get_int("level"),
        Some(2),
        "level change survived concurrent typing: {}",
        norm(&conv)
    );
    assert!(norm(&conv).contains("XTitle"), "typing survived");
}

#[test]
fn multi_block_with_code_and_heading_round_trips() {
    let schema = Rc::new(Schema::starter_kit());
    let h = schema
        .branch(
            "heading",
            Fragment::from_node(schema.text("Heading").unwrap()),
        )
        .unwrap();
    let code = schema
        .branch(
            "code_block",
            Fragment::from_node(schema.text("let x = 1;").unwrap()),
        )
        .unwrap();
    let p = para(&schema, "body");
    let doc = doc_of(&schema, vec![h, code, p]);
    let cdoc = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap();
    let back = cdoc.to_doc(&schema).unwrap();
    assert_eq!(norm(&doc), norm(&back));
}

#[test]
fn nested_content_fails_loud() {
    // design A22: anything outside flat text-blocks is a loud Unsupported error.
    let schema = Rc::new(Schema::starter_kit());
    let inner = schema
        .branch(
            "paragraph",
            Fragment::from_node(schema.text("quote").unwrap()),
        )
        .unwrap();
    let bq = schema
        .branch("blockquote", Fragment::from_node(inner))
        .unwrap();
    let doc = doc_of(&schema, vec![bq]);
    let err = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap_err();
    assert!(
        matches!(err, rinch_editor_collab::CollabError::Unsupported(_)),
        "nested block must fail loud, got {err:?}"
    );
}

#[test]
fn unsupported_change_does_not_partially_mutate() {
    // design A22 fail-loud must be all-or-nothing: a mixed flat/non-flat change
    // (the kind a paste / load-html produces while collaborating) must leave the
    // CRDT EXACTLY at the prior converged state, not half-projected.
    let schema = Rc::new(Schema::starter_kit());
    let before = doc_of(&schema, vec![para(&schema, "alpha"), para(&schema, "beta")]);
    let mut cdoc = rinch_editor_collab::CollabDoc::from_doc(&before).unwrap();
    let original = norm(&cdoc.to_doc(&schema).unwrap());

    // `after` mutates the FIRST (reconcilable) block AND appends a non-flat block —
    // so a naive in-order projection would write block 0 before failing on the
    // blockquote, leaving the CRDT half-mutated.
    let inner = schema
        .branch("paragraph", Fragment::from_node(schema.text("q").unwrap()))
        .unwrap();
    let bq = schema
        .branch("blockquote", Fragment::from_node(inner))
        .unwrap();
    let after = doc_of(
        &schema,
        vec![para(&schema, "ALPHA"), para(&schema, "beta"), bq],
    );

    let err = cdoc.project_change(&before, &after).unwrap_err();
    assert!(
        matches!(err, rinch_editor_collab::CollabError::Unsupported(_)),
        "the blockquote must fail loud, got {err:?}"
    );
    assert_eq!(
        original,
        norm(&cdoc.to_doc(&schema).unwrap()),
        "an unsupported change must not partially mutate the CRDT (block 0 stays \"alpha\")"
    );
}

// --- lists (bullet / ordered / nested list items) ------------------------------

#[test]
fn projection_round_trips_bullet_and_ordered_lists_with_nesting_and_marks() {
    // The headline list test: a document whose content is a bullet list (with a
    // bold+italic run and a nested bullet list inside a list item) and an ordered list
    // (start=3) survives the CRDT round-trip byte-for-byte — structure, order, nesting
    // depth, list attrs, and inline marks all preserved.
    let schema = Rc::new(Schema::starter_kit());
    let bold = Mark::simple(schema.mark_type("bold").unwrap().clone());
    let italic = Mark::simple(schema.mark_type("italic").unwrap().clone());

    // Item 1: a paragraph with a plain run + a bold+italic run.
    let item1 = list_item(
        &schema,
        vec![
            schema
                .create_node(
                    "paragraph",
                    Default::default(),
                    Fragment::from_children(vec![
                        schema.text("plain ").unwrap(),
                        schema
                            .text_with_marks("strong", vec![bold.clone(), italic.clone()])
                            .unwrap(),
                    ]),
                )
                .unwrap(),
        ],
    );
    // Item 2: a paragraph plus a *nested* bullet list (depth 2).
    let nested = bullet_list(
        &schema,
        vec![list_item(&schema, vec![para(&schema, "nested item")])],
    );
    let item2 = list_item(&schema, vec![para(&schema, "outer"), nested]);
    let bullet = bullet_list(&schema, vec![item1, item2]);

    let ordered = ordered_list(
        &schema,
        3,
        vec![
            list_item(&schema, vec![para(&schema, "first")]),
            list_item(&schema, vec![para(&schema, "second")]),
        ],
    );

    let doc = doc_of(&schema, vec![para(&schema, "intro"), bullet, ordered]);

    let cdoc = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap();
    let back = cdoc.to_doc(&schema).unwrap();
    assert_eq!(tree(&doc), tree(&back), "list round-trip must be lossless");
    // Structural equality is the strongest form of the same claim.
    assert_eq!(doc, back);
}

#[test]
fn concurrent_edits_in_different_list_items_converge() {
    // Two peers edit *different* items of the same bullet list. Because every text-block
    // keeps its own Text object however deeply nested, the two edits touch different
    // CRDT objects and must merge without loss.
    let schema = Rc::new(Schema::starter_kit());
    let list = bullet_list(
        &schema,
        vec![
            list_item(&schema, vec![para(&schema, "one")]),
            list_item(&schema, vec![para(&schema, "two")]),
        ],
    );
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![list])
    };
    // Positions: doc>ul>li>p>text. "one" ends at model pos 6; "two" ends at pos 13.
    a.type_at(6, "A"); // item 0 -> "oneA"
    b.type_at(13, "B"); // item 1 -> "twoB"
    sync(&mut a, &mut b);

    let pa = a.session.projected_doc(&schema).unwrap();
    let pb = b.session.projected_doc(&schema).unwrap();
    assert_eq!(tree(&pa), tree(&pb), "list projections must converge");
    assert_eq!(tree(&a.state.doc), tree(&pa), "peer A model ≡ projection");
    assert_eq!(tree(&b.state.doc), tree(&pb), "peer B model ≡ projection");
    let t = tree(&pa);
    assert!(
        t.contains("oneA") && t.contains("twoB"),
        "both list-item edits kept: {t}"
    );
}

#[test]
fn concurrent_list_item_edit_and_appended_item_converge() {
    // One peer edits an existing item's text; the other appends a whole new list item.
    // Both the text edit and the structural insert must survive the merge.
    let schema = Rc::new(Schema::starter_kit());
    let list = bullet_list(
        &schema,
        vec![
            list_item(&schema, vec![para(&schema, "alpha")]),
            list_item(&schema, vec![para(&schema, "beta")]),
        ],
    );
    let (schema, mut a, mut b) = {
        let _ = &schema;
        two_peers(vec![list])
    };
    // A edits item 0: "alpha" content is pos 3..8, so type at pos 8 -> "alphaX".
    a.type_at(8, "X");
    // B appends a third list item at the end of the bullet list. The list content ends
    // just before the bullet_list close token; a block-level replace inserts a new item.
    let li = list_item(b.state.schema(), vec![para(b.state.schema(), "gamma")]);
    let insert_at = b.state.doc.child(0).node_size() - 1; // just inside the ul close token
    b.local(|tr| {
        tr.replace(
            insert_at,
            insert_at,
            Slice::new(Fragment::from_node(li), 0, 0),
        )
        .unwrap();
    });
    sync(&mut a, &mut b);

    let pa = a.session.projected_doc(&schema).unwrap();
    let pb = b.session.projected_doc(&schema).unwrap();
    assert_eq!(tree(&pa), tree(&pb), "must converge");
    assert_eq!(tree(&a.state.doc), tree(&pa), "peer A model ≡ projection");
    assert_eq!(tree(&b.state.doc), tree(&pb), "peer B model ≡ projection");
    let t = tree(&pa);
    assert!(t.contains("alphaX"), "text edit kept: {t}");
    assert!(t.contains("gamma"), "appended item kept: {t}");
}

#[test]
fn table_still_fails_loud() {
    // The scope is narrowed, not removed: lists project, but a table (and its rows/cells)
    // is still out of scope and must fail loud rather than be silently mangled.
    let schema = Rc::new(Schema::starter_kit());
    let cell = schema
        .branch("table_cell", Fragment::from_node(para(&schema, "x")))
        .unwrap();
    let row = schema
        .branch("table_row", Fragment::from_node(cell))
        .unwrap();
    let table = schema.branch("table", Fragment::from_node(row)).unwrap();
    let doc = doc_of(&schema, vec![table]);
    let err = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap_err();
    assert!(
        matches!(err, rinch_editor_collab::CollabError::Unsupported(_)),
        "a table must still fail loud, got {err:?}"
    );
}

#[test]
fn unsupported_block_inside_a_list_item_fails_loud() {
    // A supported container (list_item) holding an *unsupported* child (blockquote) must
    // fail loud on the descendant — the recursion doesn't relax the scope for children.
    let schema = Rc::new(Schema::starter_kit());
    let bq = schema
        .branch("blockquote", Fragment::from_node(para(&schema, "q")))
        .unwrap();
    let item = list_item(&schema, vec![bq]);
    let list = bullet_list(&schema, vec![item]);
    let doc = doc_of(&schema, vec![list]);
    let err = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap_err();
    assert!(
        matches!(err, rinch_editor_collab::CollabError::Unsupported(_)),
        "an unsupported block nested in a list item must fail loud, got {err:?}"
    );
}

fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    (0..node.child_count())
        .map(|i| all_text(node.child(i)))
        .collect()
}
