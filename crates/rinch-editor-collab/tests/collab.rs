//! End-to-end collaboration tests (design M9): two `EditorState`s + two adapters,
//! concurrent insert/format, convergence after sync, rebase of local steps over a
//! remote mapping, and the fail-loud boundary on unsupported content.
//!
//! Convergence is asserted on `projected_doc()` — the canonical read-back of each
//! peer's converged CRDT — which both peers reconstruct identically, so equal CRDTs
//! give structurally-equal model docs.

use std::rc::Rc;

use rinch_editor_collab::{CollabPlugin, CollabSession, rebase_steps};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Mark, Node, Plugin, Pos, Schema, Selection,
    SetNodeAttrStep, Slice, Step, Transaction, default_plugins,
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

/// A scene break: the leaf block *atom* of the starter kit — a block-level node with no
/// content at all, which projects as a block whose text is empty.
fn scene_break(schema: &Schema) -> Node {
    schema.branch("horizontal_rule", Fragment::empty()).unwrap()
}

/// An inline atom: an `image` with the starter kit's attrs, which lives *inside* a
/// paragraph's text rather than as a block of its own.
fn image(schema: &Schema, src: &str) -> Node {
    schema
        .create_node(
            "image",
            Attrs::new()
                .with("src", AttrValue::from(src))
                .with("alt", AttrValue::from("a cat")),
            Fragment::empty(),
        )
        .unwrap()
}

/// The other inline atom: the `hard_break` a Shift+Enter inserts.
fn hard_break(schema: &Schema) -> Node {
    schema.branch("hard_break", Fragment::empty()).unwrap()
}

/// A paragraph over arbitrary inline children (text nodes, atoms, or both).
fn para_of(schema: &Schema, children: Vec<Node>) -> Node {
    schema
        .branch("paragraph", Fragment::from_children(children))
        .unwrap()
}

/// The model position at which block `index` of `doc` starts.
fn block_start(doc: &Node, index: usize) -> usize {
    (0..index).map(|i| doc.child(i).node_size()).sum()
}

/// The model position at the *end* of block `index`'s content (just inside its close
/// token) — where typing appends to that block.
fn block_content_end(doc: &Node, index: usize) -> usize {
    block_start(doc, index) + doc.child(index).node_size() - 1
}

/// Replace block `index` of `peer`'s document with `block`, as a block-level replace —
/// the shape a remote change takes, and the local edit that turns a paragraph into a
/// scene break (or back).
fn replace_block(peer: &mut Peer, index: usize, block: Node) {
    let doc = peer.state.doc.clone();
    let start = block_start(&doc, index);
    let end = start + doc.child(index).node_size();
    peer.local(|tr| {
        tr.replace(start, end, Slice::new(Fragment::from_node(block), 0, 0))
            .unwrap();
    });
}

/// Insert `block` as a whole new block *before* block `index`.
fn insert_block_before(peer: &mut Peer, index: usize, block: Node) {
    let at = block_start(&peer.state.doc, index);
    peer.local(|tr| {
        tr.replace(at, at, Slice::new(Fragment::from_node(block), 0, 0))
            .unwrap();
    });
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
            .record_local(self.state.schema(), &before, &after.doc)
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

/// Build two peers over `schema`, both starting from `blocks`. Peer B joins from peer
/// A's CRDT snapshot.
///
/// **`schema` is the caller's, deliberately** (issue #217). This used to mint its own
/// `Schema::starter_kit()` and return it, shadowing the one the caller had already used
/// to build `blocks` — so a test's document held nodes and marks from one `Schema` while
/// `a.state.schema()` was a *different* one. `MarkType`/`NodeType` equality is
/// `Rc::ptr_eq`, so a `Mark` built from the wrong instance matches nothing:
/// `Transaction::remove_mark` removed nothing and returned `Ok`, and a test asserting
/// only convergence then passed vacuously. Worse, peer B's document came from
/// `projected_doc(&inner_schema)` while peer A's was the caller's original, so the two
/// peers disagreed about mark identity with each other. One schema per test removes the
/// whole class.
fn two_peers(schema: &Rc<Schema>, blocks: Vec<Node>) -> (Peer, Peer) {
    let a_state = EditorState::create(schema.clone(), doc_of(schema, blocks), plugins());
    let session_a = CollabSession::new(&a_state).expect("session A");
    let a = Peer {
        state: a_state,
        session: session_a,
    };

    let b = join(schema, &a.session.snapshot());
    (a, b)
}

/// Join an existing session from `snapshot`, adopting the document it projects — the
/// session-level shape of `EditorHandle::start_collaboration_guest`.
fn join(schema: &Rc<Schema>, snapshot: &[u8]) -> Peer {
    let session = CollabSession::from_bytes(snapshot).expect("join from snapshot");
    let doc = session
        .projected_doc(schema)
        .expect("project the joined document");
    Peer {
        state: EditorState::create(schema.clone(), doc, plugins()),
        session,
    }
}

/// Reconcile two peers by exchanging **state vectors** and the diffs they imply.
///
/// This replaces automerge's stateful sync-message ping-pong: each side says what it has
/// (`state_vector`), the other answers with exactly what is missing (`sync_diff`), and
/// that answer is applied through the ordinary `integrate_incremental` entry point —
/// there is no per-peer protocol state to keep.
///
/// The exchange is unconditional: equal state vectors do **not** mean "already
/// converged", because a state vector records insertions only and the peers may still
/// differ by a deletion or a mark removal. Settling is therefore detected the honest way
/// — both sides integrating without a document change — and failure to settle panics
/// rather than quietly leaving the peers apart.
fn sync(a: &mut Peer, b: &mut Peer) {
    for _ in 0..8 {
        let a_sv = a.session.state_vector();
        let b_sv = b.session.state_vector();
        let to_b = a.session.sync_diff(&b_sv).expect("diff for b");
        let to_a = b.session.sync_diff(&a_sv).expect("diff for a");
        let b_changed = match b
            .session
            .integrate_incremental(&b.state, &to_b)
            .expect("b integrate")
        {
            Some(ns) => {
                b.state = ns;
                true
            }
            None => false,
        };
        let a_changed = match a
            .session
            .integrate_incremental(&a.state, &to_a)
            .expect("a integrate")
        {
            Some(ns) => {
                a.state = ns;
                true
            }
            None => false,
        };
        if !a_changed && !b_changed {
            return;
        }
    }
    panic!("the state-vector exchange did not settle");
}

/// A canonical, mark-order-independent, run-coalesced string form of a flat doc, so
/// two equal documents compare equal regardless of how text/marks were assembled.
fn norm(doc: &Node) -> String {
    let mut s = String::new();
    for bi in 0..doc.child_count() {
        let b = doc.child(bi);
        s.push_str(&format!("<{} {}>", b.type_name(), norm_attrs(b)));
        // (marks, rendering, is_text) — an inline **atom** renders as its type and
        // attrs, and never coalesces with a neighbouring run, so a document that lost
        // one (or grew one) cannot compare equal to one that did not.
        let mut runs: Vec<(Vec<String>, String, bool)> = Vec::new();
        for ci in 0..b.child_count() {
            let c = b.child(ci);
            let marks = canon_marks(c);
            let (text, is_text) = match c.text() {
                Some(t) => (t.to_string(), true),
                None => (format!("⟦{} {}⟧", c.type_name(), norm_attrs(c)), false),
            };
            match runs.last_mut() {
                Some(last) if last.2 && is_text && last.0 == marks => last.1.push_str(&text),
                _ => runs.push((marks, text, is_text)),
            }
        }
        for (marks, text, _) in runs {
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
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "Hello")]);
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
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "Hello world")]);
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
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "Hello world")]);
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
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "start")]);
    a.type_at(6, " more");
    // broadcast delta from A to B
    let delta = a.session.save_incremental().expect("delta encodes");
    if let Some(ns) = b.session.integrate_incremental(&b.state, &delta).unwrap() {
        b.state = ns;
    }
    assert_converged(&a, &b, &schema);
    assert!(norm(&b.state.doc).contains("start more"));
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
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "one"), para(&schema, "two")]);
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
    // Issue #217: this used to pass vacuously. `two_peers` minted its own `Schema`, so
    // `a.state.schema().mark_type("bold")` was a *different* `MarkType` handle from the
    // one on the document — `MarkType` equality is `Rc::ptr_eq` — and `remove_mark`
    // matched nothing, removed nothing, and returned `Ok`. The only assertion was
    // convergence, which of course held: the peers agreed on a document that still had
    // the bold on it. One schema per test (see `two_peers`) plus the mark assertion
    // below is what makes the name true.
    let schema = Rc::new(Schema::starter_kit());
    let bold = Mark::simple(schema.mark_type("bold").unwrap().clone());
    let bolded = schema
        .create_node(
            "paragraph",
            Default::default(),
            Fragment::from_node(schema.text_with_marks("word", vec![bold.clone()]).unwrap()),
        )
        .unwrap();
    let (mut a, mut b) = two_peers(&schema, vec![bolded]);
    // A removes the bold over "word" (model 1..5); B types at the end.
    a.local(|tr| {
        tr.remove_mark(1, 5, bold).unwrap();
    });
    // The removal must land *locally* before any sync — otherwise convergence below
    // proves nothing about it.
    assert!(
        !norm(&a.state.doc).contains("bold"),
        "A's own model lost the bold: {}",
        norm(&a.state.doc)
    );
    b.type_at(5, "!");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    // And it must survive the concurrent typing, on both peers, through the CRDT.
    let t = norm(&a.session.projected_doc(&schema).unwrap());
    assert_eq!(
        t, "<paragraph >«word!|»</>",
        "the bold removal and the concurrent typing both survive"
    );
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
    let (mut a, mut b) = two_peers(&schema, vec![h]);
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
fn concurrent_bold_add_and_italic_removal_converge_without_resurrecting_the_italic() {
    // Issue #193 repro (a). One paragraph, "one two three", italic over "three". One
    // peer bolds "one"; the other concurrently REMOVES the italic. The two edits touch
    // *different* marks, so both must survive: bold on "one", italic gone.
    //
    // Before the per-span diff, `resync_marks` cleared and re-applied EVERY mark on the
    // block whenever ANY mark differed — so the bolding peer's resync re-wrote
    // `italic: true` over "three", and that fresh write outlived the other peer's
    // concurrent removal. Both peers converged (no divergence!) on a document that
    // silently resurrected the italic. Deterministic in both role-orderings, hence the
    // loop: the host owns the initial projection and yrs client ids break ties, so
    // neither ordering may be assumed to cover the other.
    for host_bolds in [true, false] {
        let schema = Rc::new(Schema::starter_kit());
        let italic = Mark::simple(schema.mark_type("italic").unwrap().clone());
        let p = schema
            .create_node(
                "paragraph",
                Default::default(),
                Fragment::from_children(vec![
                    schema.text("one two ").unwrap(),
                    schema.text_with_marks("three", vec![italic]).unwrap(),
                ]),
            )
            .unwrap();
        let (mut a, mut b) = two_peers(&schema, vec![p]);
        // Model positions: "one" = 1..4, "three" = 9..14.
        let unitalic = |peer: &mut Peer| {
            // Pull the mark off the document itself. `Mark`/`MarkType` equality is
            // Rc-pointer identity, so a mark from a different `Schema::starter_kit()`
            // can never match — which used to be a silent no-op, and which #217 turned
            // into two changes: `two_peers` now takes the caller's schema (so both
            // peers' documents share one), and `Transform::remove_mark` rejects a
            // foreign-schema mark loudly. Reading the mark off the doc is still the
            // robust way to ask for "whatever this document calls italic".
            let italic = {
                let block = peer.state.doc.child(0);
                (0..block.child_count())
                    .flat_map(|i| block.child(i).marks().iter())
                    .find(|m| m.type_name() == "italic")
                    .expect("the italic mark is on the initial doc")
                    .clone()
            };
            peer.local(|tr| {
                tr.remove_mark(9, 14, italic).unwrap();
            });
        };
        if host_bolds {
            a.bold(1, 4);
            unitalic(&mut b);
        } else {
            unitalic(&mut a);
            b.bold(1, 4);
        }
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        let t = norm(&a.session.projected_doc(&schema).unwrap());
        assert!(
            t.contains("«one|bold[]»"),
            "(host_bolds={host_bolds}) bold landed on exactly \"one\": {t}"
        );
        assert!(
            !t.contains("italic"),
            "(host_bolds={host_bolds}) the italic removal survived the concurrent \
             unrelated-mark edit: {t}"
        );
    }
}

#[test]
fn concurrent_edits_to_different_attrs_of_the_same_block_both_survive() {
    // Issue #193 repro (b). A level-1 heading; one peer sets `level: 3`, the other
    // concurrently sets `text_align: "center"`. Different keys of the same attrs map,
    // so both edits must survive: level=3 AND text_align=center on both peers.
    //
    // Before the per-key diff, the attrs branch of `reconcile_node` replaced the WHOLE
    // attrs map object whenever any attr differed. yrs merges map conflicts per key,
    // but only within one map object — two peers each installing a fresh object is a
    // conflict on the node's `attrs` key itself, one object wins wholesale, and the
    // other peer's edit vanishes (converged, silently wrong: e.g. level=1 +
    // text_align=center). Both role-orderings, as above.
    for host_sets_level in [true, false] {
        let schema = Rc::new(Schema::starter_kit());
        let h = schema
            .create_node(
                "heading",
                Attrs::new().with("level", 1i64),
                Fragment::from_node(schema.text("Title").unwrap()),
            )
            .unwrap();
        let (mut a, mut b) = two_peers(&schema, vec![h]);
        let set = |peer: &mut Peer, attr: &'static str, value: AttrValue| {
            peer.local(|tr| {
                tr.set_node_attr(0, attr, value).unwrap();
            });
        };
        if host_sets_level {
            set(&mut a, "level", AttrValue::Int(3));
            set(&mut b, "text_align", AttrValue::from("center"));
        } else {
            set(&mut a, "text_align", AttrValue::from("center"));
            set(&mut b, "level", AttrValue::Int(3));
        }
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        for (name, peer) in [("A", &a), ("B", &b)] {
            let conv = peer.session.projected_doc(&schema).unwrap();
            let attrs = conv.child(0).attrs().clone();
            assert_eq!(
                attrs.get_int("level"),
                Some(3),
                "(host_sets_level={host_sets_level}) peer {name}: the level change \
                 survived the concurrent other-attr edit: {}",
                norm(&conv)
            );
            assert_eq!(
                attrs.get_str("text_align"),
                Some("center"),
                "(host_sets_level={host_sets_level}) peer {name}: the alignment change \
                 survived the concurrent other-attr edit: {}",
                norm(&conv)
            );
        }
    }
}

#[test]
fn attr_set_vs_concurrent_attr_remove_of_different_key() {
    // The stale-key sweep in `reconcile_attrs`, pinned directly — the sibling test
    // above exercises only the insert half (mutation testing showed the sweep could
    // be disabled with every directed test still green). Removing an attr projects
    // as a per-key map REMOVE on the shared attrs object, which must coexist with a
    // peer's concurrent write to a *different* key of the same object.
    //
    // A heading starts with both `level` and `text_align`; one peer sets `level: 3`,
    // the other removes `text_align` outright (a `SetNodeAttrStep` with
    // `value: None` — the model drops the key, so the projection's only way to
    // mirror it is the sweep). Both edits must survive on both peers: level=3,
    // text_align absent. Both role-orderings, as above.
    for host_sets_level in [true, false] {
        let schema = Rc::new(Schema::starter_kit());
        let h = schema
            .create_node(
                "heading",
                Attrs::new().with("level", 1i64).with("text_align", "left"),
                Fragment::from_node(schema.text("Title").unwrap()),
            )
            .unwrap();
        let (mut a, mut b) = two_peers(&schema, vec![h]);
        let set_level = |peer: &mut Peer| {
            peer.local(|tr| {
                tr.set_node_attr(0, "level", AttrValue::Int(3)).unwrap();
            });
        };
        let remove_align = |peer: &mut Peer| {
            peer.local(|tr| {
                tr.step(Box::new(SetNodeAttrStep {
                    pos: 0,
                    attr: "text_align".into(),
                    value: None,
                }))
                .unwrap();
            });
        };
        if host_sets_level {
            set_level(&mut a);
            remove_align(&mut b);
        } else {
            remove_align(&mut a);
            set_level(&mut b);
        }
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        for (name, peer) in [("A", &a), ("B", &b)] {
            let conv = peer.session.projected_doc(&schema).unwrap();
            let attrs = conv.child(0).attrs().clone();
            assert_eq!(
                attrs.get_int("level"),
                Some(3),
                "(host_sets_level={host_sets_level}) peer {name}: the level change \
                 survived the concurrent removal of the other key: {}",
                norm(&conv)
            );
            assert!(
                attrs.get("text_align").is_none(),
                "(host_sets_level={host_sets_level}) peer {name}: the text_align \
                 removal survived the concurrent write to the other key: {}",
                norm(&conv)
            );
        }
    }
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
    let (mut a, mut b) = two_peers(&schema, vec![list]);
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
    let (mut a, mut b) = two_peers(&schema, vec![list]);
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

#[test]
fn a_delete_only_broadcast_delta_reaches_the_peer() {
    // The session-layer twin of the handle-level deletion test. A state vector counts
    // insertions, so a peer that only *deleted* leaves it unchanged — and yrs implements
    // un-formatting by deleting format markers, so a mark removal is delete-only too.
    // Any state-vector short-circuit in `save_incremental` or `integrate_incremental`
    // drops these changes; this pins the broadcast path, where the sibling test pins the
    // handle wiring and the fuzz suites pin it statistically.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "abcdef")]);
    // Drop whatever the initial projection queued; both peers already share it.
    let _ = a.session.save_incremental().expect("drain");

    let sv_before = a.session.state_vector();
    a.local(|tr| {
        tr.delete(3, 5).unwrap(); // remove "cd"
    });
    assert_eq!(
        a.session.state_vector(),
        sv_before,
        "precondition: a delete-only edit leaves the state vector untouched"
    );

    let delta = a.session.save_incremental().expect("delta encodes");
    assert!(
        !delta.is_empty(),
        "a delete-only edit must still produce something to broadcast"
    );
    if let Some(ns) = b
        .session
        .integrate_incremental(&b.state, &delta)
        .expect("b integrates")
    {
        b.state = ns;
    }
    assert_converged(&a, &b, &schema);
    assert!(
        norm(&b.state.doc).contains("abef"),
        "the deletion reached the peer: {}",
        norm(&b.state.doc)
    );
}

// --- leaf block atoms (scene breaks) -------------------------------------------

#[test]
fn projection_round_trips_a_scene_break_between_paragraphs() {
    // The headline atom test: a block that holds *nothing* survives the round trip as
    // itself. `horizontal_rule` projects as a block whose text is empty, and the
    // rebuild must hand back the identical `Node` — not a paragraph, and not a rule
    // with an empty text child in it.
    let schema = Rc::new(Schema::starter_kit());
    let doc = doc_of(
        &schema,
        vec![para(&schema, "a"), scene_break(&schema), para(&schema, "b")],
    );
    let cdoc = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap();
    let back = cdoc.to_doc(&schema).unwrap();
    assert_eq!(
        back, doc,
        "the rebuilt document is the identical model tree"
    );
    assert_eq!(tree(&doc), tree(&back));
    assert_eq!(back.child(1).child_count(), 0, "the rule holds no content");

    // And a late joiner reading the same bytes adopts it too (the snapshot path).
    let joined = rinch_editor_collab::CollabDoc::load(&cdoc.save())
        .expect("a projection holding a scene break is joinable")
        .to_doc(&schema)
        .unwrap();
    assert_eq!(joined, doc);
}

#[test]
fn a_scene_break_inserted_by_the_host_reaches_the_guest() {
    // PlotWeb's actual symptom (a scene break stopped the body syncing while the UI
    // still said Saved): the author inserts an hr between two paragraphs, and it must
    // both project and arrive — after which ordinary typing in the paragraph *after*
    // the rule keeps working and the rule stays where it is.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "one"), para(&schema, "two")]);
    insert_block_before(&mut a, 1, scene_break(&schema));
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&b.state.doc),
        norm(&doc_of(
            &schema,
            vec![
                para(&schema, "one"),
                scene_break(&schema),
                para(&schema, "two"),
            ],
        )),
        "the guest received the scene break in place"
    );

    // Typing into the paragraph *after* the rule still syncs, and the rule survives it.
    let at = block_content_end(&a.state.doc, 2);
    a.type_at(at, "!");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&b.state.doc),
        norm(&doc_of(
            &schema,
            vec![
                para(&schema, "one"),
                scene_break(&schema),
                para(&schema, "two!"),
            ],
        )),
        "the guest sees the text edit and still has the rule"
    );
}

#[test]
fn deleting_a_scene_break_on_one_side_removes_it_on_the_other() {
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(
        &schema,
        vec![
            para(&schema, "one"),
            scene_break(&schema),
            para(&schema, "two"),
        ],
    );
    assert_eq!(
        b.state.doc.child_count(),
        3,
        "the guest joined with the rule"
    );

    delete_block(&mut a, 1);
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&b.state.doc),
        norm(&doc_of(
            &schema,
            vec![para(&schema, "one"), para(&schema, "two")],
        )),
        "the deletion reached the guest"
    );
}

#[test]
fn a_remote_change_swapping_a_paragraph_for_a_scene_break_and_back_is_handled() {
    // A block changing *kind* is the case the projection must not treat as a text
    // splice: reconciled in place, the rule would keep the paragraph's `Text` object
    // and a peer's concurrent typing would land inside what is now an atom — a shape no
    // model can express. The peer here even has its caret inside the paragraph being
    // replaced, which is the caret-carry path that must decline rather than measure a
    // text offset against an atom.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(
        &schema,
        vec![
            para(&schema, "one"),
            para(&schema, "mid"),
            para(&schema, "two"),
        ],
    );
    // B's caret sits inside "mid" (content starts just after the block's open token).
    let caret = block_start(&b.state.doc, 1) + 2;
    b.local(|tr| {
        tr.set_selection(Selection::cursor(Pos(caret)));
    });

    // A replaces that paragraph with a scene break.
    replace_block(&mut a, 1, scene_break(&schema));
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    let expected = doc_of(
        &schema,
        vec![
            para(&schema, "one"),
            scene_break(&schema),
            para(&schema, "two"),
        ],
    );
    assert_eq!(
        norm(&b.state.doc),
        norm(&expected),
        "the guest has the rule"
    );
    let head = b.state.selection.head().0;
    assert!(
        head >= 1 && head <= b.state.doc.content_size(),
        "the caret is still a valid position: {head}"
    );

    // …and the other direction: B turns the rule back into a paragraph.
    replace_block(&mut b, 1, para(&schema, "back"));
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&a.state.doc),
        norm(&doc_of(
            &schema,
            vec![
                para(&schema, "one"),
                para(&schema, "back"),
                para(&schema, "two"),
            ],
        )),
        "the host has the paragraph again"
    );

    // Typing in the restored paragraph still syncs — the swap left no stale text object.
    let at = block_content_end(&b.state.doc, 1);
    b.type_at(at, "!");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert!(
        norm(&a.state.doc).contains("back!"),
        "the restored paragraph is editable: {}",
        norm(&a.state.doc)
    );
}

#[test]
fn a_peer_typing_in_a_paragraph_another_turns_into_a_scene_break_still_converges() {
    // The concurrency case the "retyped to empty is a replace" rule exists for. If the
    // projection reconciled that change in place, the rule would inherit the paragraph's
    // live `Text` object while its `type` flipped to the atom — and the peer's
    // concurrent keystroke, merged into that same text, would leave the *converged* CRDT
    // holding a horizontal_rule with text inside it. No model can express that, so every
    // read-back from then on fails and the session poisons: one scene break typed at the
    // wrong moment would take the document down for both authors. (Measured, with the
    // replace rule disabled: `SessionPoisoned("... leaf block atom `horizontal_rule`
    // carries text in the CRDT (1 char(s), 0 mark span(s)) ...")` on B's integrate.) Replacing makes the
    // conflict structural instead — the keystroke is lost with the block it was in,
    // which is the honest outcome of "you edited what I deleted".
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(
        &schema,
        vec![
            para(&schema, "one"),
            para(&schema, "mid"),
            para(&schema, "two"),
        ],
    );
    replace_block(&mut a, 1, scene_break(&schema)); // A: the paragraph becomes a rule
    b.type_at(block_content_end(&b.state.doc, 1), "X"); // B: types in that paragraph
    sync(&mut a, &mut b);

    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&a.state.doc),
        norm(&doc_of(
            &schema,
            vec![
                para(&schema, "one"),
                scene_break(&schema),
                para(&schema, "two"),
            ],
        )),
        "the rule won, and it is a rule with nothing in it"
    );
    assert!(
        !norm(&a.state.doc).contains('X'),
        "the keystroke went with the block it was in, rather than into the atom: {}",
        norm(&a.state.doc)
    );

    // The session is healthy, not poisoned: the next edit on either side still syncs.
    a.type_at(block_content_end(&a.state.doc, 2), "!");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert!(norm(&b.state.doc).contains("two!"), "editing still works");
}

// --- inline atoms (hard breaks, images) ----------------------------------------

#[test]
fn projection_round_trips_a_paragraph_holding_inline_atoms() {
    // The headline inline-atom test: an image and a hard break inside a line of text
    // survive the round trip as themselves, at the positions they were in — not as
    // stray U+FFFC characters, and not by failing loud.
    let schema = Rc::new(Schema::starter_kit());
    let line = para_of(
        &schema,
        vec![
            schema.text("look: ").unwrap(),
            image(&schema, "cat.png"),
            schema.text(" and on").unwrap(),
            hard_break(&schema),
            schema.text("a new line").unwrap(),
        ],
    );
    let doc = doc_of(&schema, vec![line, para(&schema, "after")]);
    let cdoc = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap();
    let back = cdoc.to_doc(&schema).unwrap();
    assert_eq!(
        back, doc,
        "the rebuilt document is the identical model tree"
    );
    assert_eq!(tree(&doc), tree(&back));

    // And a late joiner reading the same bytes adopts it too (the snapshot path).
    let joined = rinch_editor_collab::CollabDoc::load(&cdoc.save())
        .expect("a projection holding inline atoms is joinable")
        .to_doc(&schema)
        .unwrap();
    assert_eq!(joined, doc);
}

#[test]
fn a_hard_break_typed_by_the_host_reaches_the_guest_and_the_body_keeps_syncing() {
    // PlotWeb's actual symptom, the inline half of it: the moment the author pressed
    // Shift+Enter the body stopped syncing while the editor still said "Saved". The
    // break must project, arrive, and leave the paragraph editable on both sides.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "one two")]);

    // Shift+Enter between the words: an atom inserted into existing text.
    let at = block_start(&a.state.doc, 0) + 5; // "one |two"
    let br = hard_break(&schema);
    a.local(|tr| {
        tr.replace(at, at, Slice::new(Fragment::from_node(br), 0, 0))
            .unwrap();
    });
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&b.state.doc),
        norm(&doc_of(
            &schema,
            vec![para_of(
                &schema,
                vec![
                    schema.text("one ").unwrap(),
                    hard_break(&schema),
                    schema.text("two").unwrap(),
                ],
            )],
        )),
        "the guest received the hard break in place"
    );

    // Typing on both sides of it still syncs, and the break stays where it is.
    a.type_at(block_content_end(&a.state.doc, 0), "!");
    b.type_at(block_start(&b.state.doc, 0) + 1, "X");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert!(
        norm(&a.state.doc).contains("⟦hard_break ⟧"),
        "the break survived the edits around it: {}",
        norm(&a.state.doc)
    );
}

#[test]
fn deleting_an_inline_atom_removes_it_on_the_other_side() {
    let schema = Rc::new(Schema::starter_kit());
    let line = para_of(
        &schema,
        vec![
            schema.text("ab").unwrap(),
            image(&schema, "cat.png"),
            schema.text("cd").unwrap(),
        ],
    );
    let (mut a, mut b) = two_peers(&schema, vec![line]);
    assert!(
        norm(&b.state.doc).contains("⟦image"),
        "the guest joined with it"
    );

    // The image is one model position wide, just after "ab".
    let at = block_start(&a.state.doc, 0) + 3;
    a.local(|tr| {
        tr.delete(at, at + 1).unwrap();
    });
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&b.state.doc),
        norm(&doc_of(&schema, vec![para(&schema, "abcd")])),
        "the deletion reached the guest and left ordinary text behind"
    );
}

#[test]
fn changing_an_atoms_attrs_reconciles_it_rather_than_duplicating_it() {
    // An image whose `src` changes is the same node with a different attribute, so the
    // projection must rewrite that one char's `@atom` value — not insert a second
    // placeholder beside the first, which is what a diff that treated the atom as
    // opaque content would do.
    let schema = Rc::new(Schema::starter_kit());
    let line = para_of(
        &schema,
        vec![
            schema.text("see ").unwrap(),
            image(&schema, "old.png"),
            schema.text(" now").unwrap(),
        ],
    );
    let (mut a, mut b) = two_peers(&schema, vec![line]);
    let at = block_start(&a.state.doc, 0) + 5; // just before the image

    a.local(|tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            at,
            "src",
            AttrValue::from("new.png"),
        )))
        .unwrap();
    });
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&b.state.doc),
        norm(&doc_of(
            &schema,
            vec![para_of(
                &schema,
                vec![
                    schema.text("see ").unwrap(),
                    image(&schema, "new.png"),
                    schema.text(" now").unwrap(),
                ],
            )],
        )),
        "one image, with the new src"
    );
    assert_eq!(
        norm(&b.state.doc).matches("⟦image").count(),
        1,
        "exactly one image: {}",
        norm(&b.state.doc)
    );
}

#[test]
fn concurrent_edits_on_both_sides_of_an_atom_converge_and_keep_it() {
    // Two authors typing either side of a picture: exactly one image survives, both
    // insertions survive, and the session stays healthy. The typer right *after* the
    // image gets the image's `@atom` attribute on its new chars from yrs (an insert at
    // the end boundary of a formatted range joins it), but its own `resync_marks`
    // clears that in the same transaction, so the peer never receives a stray span
    // from this path. The stray-span rule of `is_atom_char` is pinned by
    // `a_stray_atom_attribute_from_a_peer_is_text_and_is_cleared_by_the_next_local_edit`,
    // which forges the update a non-rinch peer would send.
    let schema = Rc::new(Schema::starter_kit());
    let line = para_of(
        &schema,
        vec![
            schema.text("ab").unwrap(),
            image(&schema, "cat.png"),
            schema.text("cd").unwrap(),
        ],
    );
    let (mut a, mut b) = two_peers(&schema, vec![line]);
    let after_image = block_start(&a.state.doc, 0) + 4;
    let before_image = block_start(&b.state.doc, 0) + 3;

    a.type_at(after_image, "XY"); // directly after the atom
    b.type_at(before_image, "Z"); // directly before it
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);

    let converged = norm(&a.state.doc);
    assert_eq!(
        converged.matches("⟦image").count(),
        1,
        "exactly one image, and no char of the typing became a second one: {converged}"
    );
    for typed in ["XY", "Z"] {
        assert!(converged.contains(typed), "{typed} survived: {converged}");
    }

    // The session is healthy, not poisoned: the next edit on either side still syncs.
    a.type_at(block_content_end(&a.state.doc, 0), "!");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert!(norm(&b.state.doc).contains('!'), "editing still works");
}

#[test]
fn an_atom_carrying_a_mark_reaches_the_peer_with_it() {
    // A linked image — the atom is a char that happens to be formatted, so its own
    // marks are ordinary spans over that char and travel like any other formatting.
    let schema = Rc::new(Schema::starter_kit());
    let link = Mark::new(
        schema.mark_type("link").unwrap().clone(),
        Attrs::new().with("href", AttrValue::from("https://example.test/")),
    );
    let line = para_of(
        &schema,
        vec![
            schema.text("see ").unwrap(),
            image(&schema, "cat.png").with_marks(vec![link]),
        ],
    );
    let (_a, b) = two_peers(&schema, vec![line.clone()]);
    assert_eq!(
        tree(&b.state.doc),
        tree(&doc_of(&schema, vec![line])),
        "the guest built the linked image from the snapshot"
    );
}

#[test]
fn a_literal_object_replacement_character_stays_text() {
    // The character the projection uses as its placeholder is one a user can paste.
    // Nothing marks it as an atom, so it must round-trip — and converge — as text.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "a\u{FFFC}b")]);
    a.type_at(block_content_end(&a.state.doc, 0), "!");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        norm(&b.state.doc),
        norm(&doc_of(&schema, vec![para(&schema, "a\u{FFFC}b!")])),
        "still one paragraph of plain text"
    );
    assert!(
        !norm(&b.state.doc).contains('⟦'),
        "and no node was invented from it: {}",
        norm(&b.state.doc)
    );
}

#[test]
fn an_inline_atom_is_still_not_a_block_of_its_own() {
    // The boundary the inline atom does NOT move: an `image` is in scope inside a
    // textblock's inline content, never as a top-level block.
    let schema = Rc::new(Schema::starter_kit());
    let doc = doc_of(&schema, vec![image(&schema, "cat.png")]);
    let err = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap_err();
    assert!(
        matches!(err, rinch_editor_collab::CollabError::Unsupported(_)),
        "an inline atom standing as a block must still fail loud, got {err:?}"
    );
}

// --- the zero-block state (issue #192) -----------------------------------------

/// Delete block `index` outright (its open token through its close token).
fn delete_block(peer: &mut Peer, index: usize) {
    let doc = peer.state.doc.clone();
    let start: usize = (0..index).map(|i| doc.child(i).node_size()).sum();
    let end = start + doc.child(index).node_size();
    peer.local(|tr| {
        tr.delete(start, end).unwrap();
    });
}

#[test]
fn concurrent_deletion_of_different_blocks_empties_the_crdt_and_self_heals() {
    // The #192 repro at the session layer. Two peers concurrently delete *different*
    // blocks, so the union of the deletions is every block and the converged content
    // array is empty — a state no model can mirror, since the schema requires a block.
    // Before the fix the next local edit died with `Schema("reconcile_node: missing
    // node")`, nothing was broadcast, and the session never recovered.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "one"), para(&schema, "two")]);
    delete_block(&mut a, 1); // A drops "two"
    delete_block(&mut b, 0); // B drops "one"
    assert_eq!(a.state.doc.child_count(), 1, "each peer deleted one block");
    assert_eq!(b.state.doc.child_count(), 1);
    sync(&mut a, &mut b);

    // The invariant's one exception: with zero blocks in the CRDT the read-back is the
    // starter paragraph — equal to each peer's model, but not backed by CRDT content.
    let starter = norm(&doc_of(&schema, vec![para(&schema, "")]));
    assert_eq!(
        norm(&a.state.doc),
        starter,
        "A converged to the starter paragraph"
    );
    assert_eq!(
        norm(&b.state.doc),
        starter,
        "B converged to the starter paragraph"
    );
    assert_converged(&a, &b, &schema);

    // The next local edit on either peer must project, broadcast and converge.
    a.type_at(1, "Z");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert!(
        norm(&b.state.doc).contains('Z'),
        "A's first edit after the empty state reached B: {}",
        norm(&b.state.doc)
    );

    b.type_at(1, "Y");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    let t = norm(&a.state.doc);
    assert!(
        t.contains('Y') && t.contains('Z'),
        "both post-recovery edits survived: {t}"
    );
}

#[test]
fn a_late_joiner_can_join_a_session_with_no_blocks_left() {
    // The second symptom of #192: `load` used to reject a zero-block projection as "not a
    // rinch editor projection", locking a late joiner out of a session that had reached
    // that state. The format marker is the discriminator now, so the join succeeds and
    // the joiner adopts the starter paragraph.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "one"), para(&schema, "two")]);
    delete_block(&mut a, 1);
    delete_block(&mut b, 0);
    sync(&mut a, &mut b);

    let starter = norm(&doc_of(&schema, vec![para(&schema, "")]));
    let mut c = join(&schema, &a.session.snapshot());
    assert_eq!(
        norm(&c.state.doc),
        starter,
        "the late joiner adopts the starter paragraph"
    );

    // And edits flow both ways between the joiner and an existing peer.
    c.type_at(1, "L");
    sync(&mut a, &mut c);
    assert_converged(&a, &c, &schema);
    assert!(
        norm(&a.state.doc).contains('L'),
        "the joiner's edit reached the existing peer: {}",
        norm(&a.state.doc)
    );

    a.type_at(1, "H");
    sync(&mut a, &mut c);
    assert_converged(&a, &c, &schema);
    let t = norm(&c.state.doc);
    assert!(
        t.contains('H') && t.contains('L'),
        "both directions synced: {t}"
    );
}

#[test]
fn integrating_nothing_is_a_no_op_not_an_error() {
    // `save_incremental` returns an empty Vec when there is nothing to send, and a
    // reconciliation diff for an up-to-date peer encodes as the two-byte empty update.
    // Neither decodes as an update, so both must be recognised rather than surfaced as a
    // spurious decode error — a caller that forwards its own empty delta is not wrong.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "steady")]);
    let before = norm(&b.state.doc);
    for nothing in [Vec::new(), vec![0u8, 0u8]] {
        assert!(
            b.session
                .integrate_incremental(&b.state, &nothing)
                .expect("an empty update is a no-op, not an error")
                .is_none(),
            "nothing arrived, so nothing changed"
        );
    }
    assert_eq!(norm(&b.state.doc), before, "the document is untouched");
    // And an up-to-date peer's own reconciliation answer is exactly that empty update.
    let self_diff = a
        .session
        .sync_diff(&a.session.state_vector())
        .expect("self diff");
    assert!(
        a.session
            .integrate_incremental(&a.state, &self_diff)
            .expect("self diff integrates")
            .is_none()
    );
}

// --- astral (non-BMP) offsets --------------------------------------------------
//
// The model indexes **chars** (Unicode scalars); yrs indexes **UTF-16 code units**. Every
// index crossing into a block's text is converted, and getting it wrong is *silent*: yrs
// snaps an index landing mid surrogate pair to the nearest boundary instead of erroring.
// So each test below uses **two** astral characters — with only one, a conversion that
// simply passes the char offset through still lands on a legal boundary and the test
// would pass while the arithmetic was wrong.

#[test]
fn projection_round_trips_marks_across_astral_pairs() {
    let schema = Rc::new(Schema::starter_kit());
    let bold = Mark::simple(schema.mark_type("bold").unwrap().clone());

    // chars 0:'a' 1:'🐱' 2:'b' 3:'🐶' 4:'c' — UTF-16 units 0..7, so a mark over chars
    // 1..4 is UTF-16 1..6.
    let spanning = schema
        .create_node(
            "paragraph",
            Default::default(),
            Fragment::from_children(vec![
                schema.text("a").unwrap(),
                schema.text_with_marks("🐱b🐶", vec![bold.clone()]).unwrap(),
                schema.text("c").unwrap(),
            ]),
        )
        .unwrap();

    // The sharper case: mark ONLY the second astral char (chars 1..2 of "🐱🐶", i.e.
    // UTF-16 2..4). Passing the char offsets straight through would ask for UTF-16 1..2,
    // land inside the first surrogate pair, and get snapped onto the *first* cat — the
    // round-trip would then come back with the wrong character marked.
    let second_only = schema
        .create_node(
            "paragraph",
            Default::default(),
            Fragment::from_children(vec![
                schema.text("🐱").unwrap(),
                schema.text_with_marks("🐶", vec![bold]).unwrap(),
            ]),
        )
        .unwrap();

    let doc = doc_of(&schema, vec![spanning, second_only]);
    let cdoc = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap();
    let back = cdoc.to_doc(&schema).unwrap();
    assert_eq!(norm(&doc), norm(&back));
    assert_eq!(doc, back, "astral marks must round-trip byte-for-byte");
}

#[test]
fn astral_inserts_and_deletes_keep_model_and_projection_in_step() {
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, _b) = two_peers(&schema, vec![para(&schema, "🐱🐶")]);
    // Char positions: 1 = before 🐱, 2 = between the two, 3 = after 🐶.
    let check = |a: &Peer, want: &str| {
        let projected = a.session.projected_doc(&schema).unwrap();
        assert_eq!(
            norm(&a.state.doc),
            norm(&projected),
            "model ≡ project(model) across astral text"
        );
        assert!(
            norm(&projected).contains(want),
            "expected {want:?} in {}",
            norm(&projected)
        );
    };

    a.type_at(2, "X"); // insert *between* two surrogate pairs
    check(&a, "🐱X🐶");
    a.type_at(4, "🐭"); // insert an astral char after 🐶
    check(&a, "🐱X🐶🐭");
    a.local(|tr| {
        tr.delete(1, 2).unwrap();
    }); // delete the leading astral char
    check(&a, "X🐶🐭");
    a.bold(2, 4); // mark the two remaining astral chars
    check(&a, "🐶🐭");
    assert!(
        norm(&a.state.doc).contains("bold["),
        "the astral mark landed: {}",
        norm(&a.state.doc)
    );
}

#[test]
fn concurrent_edits_around_astral_pairs_converge() {
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, vec![para(&schema, "🐱🐶")]);
    a.type_at(2, "A"); // A types between the two surrogate pairs
    b.bold(1, 3); // B marks both astral chars
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    let t = norm(&a.session.projected_doc(&schema).unwrap());
    assert!(t.contains('A'), "insert survived: {t}");
    assert!(t.contains("bold["), "astral mark survived: {t}");
    assert!(t.contains('🐱') && t.contains('🐶'), "text intact: {t}");
}

fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    (0..node.child_count())
        .map(|i| all_text(node.child(i)))
        .collect()
}

// --- an inline atom's attrs survive a neighbour's typing (review of #838, F1) -------
//
// yrs extends a formatted range over an insert at its **end** boundary — the rule that
// continues bold when you type at the end of a bold word — so a char typed right after
// an image lands inside the image's `@atom` range at the CRDT level, while the model
// (which never extends an atom onto its neighbour) holds it as plain text. The typer's
// own `resync_marks` then clears that stray attribute. It used to clear it **per span**:
// the stray char and the image char are one coalesced span with one value, so clearing
// it cleared the image too and re-applied the image's *old* attrs as a fresh write —
// which outlived a peer's concurrent `src` change on every replica. Both peers still
// converged, on the old picture; nothing errored. A `link` mark is the same shape since
// links became non-inclusive (the typed char is plain in the model too), and it is
// resynced per char for the same reason —
// `typing_after_a_link_while_a_peer_changes_its_href_keeps_the_new_href` pins it.
//
// Every fixture runs under **both** yrs client-id orders, because which concurrent
// formatting write wins is decided by the client-id tie-break: a test run once with
// random ids passes or fails by coin toss.

/// Client-id pairs covering both tie-break orders.
const ID_ORDERS: [(u64, u64); 2] = [(11, 22), (22, 11)];

/// [`two_peers`] with both yrs client ids pinned.
fn two_peers_with_ids(schema: &Rc<Schema>, blocks: Vec<Node>, ids: (u64, u64)) -> (Peer, Peer) {
    use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
    let a_state = EditorState::create(schema.clone(), doc_of(schema, blocks), plugins());
    let session_a = session_with_client_id(&a_state, ids.0).expect("session A");
    let snapshot = session_a.snapshot();
    let a = Peer {
        state: a_state,
        session: session_a,
    };
    let session_b = session_from_bytes_with_client_id(&snapshot, ids.1).expect("session B");
    let doc = session_b.projected_doc(schema).expect("project B");
    let b = Peer {
        state: EditorState::create(schema.clone(), doc, plugins()),
        session: session_b,
    };
    (a, b)
}

/// `ab` + an image of `cat.png` + `cd`: the image sits at block offset 3, and the
/// position right after it is block offset 4.
fn line_with_image(schema: &Schema) -> Node {
    para_of(
        schema,
        vec![
            schema.text("ab").unwrap(),
            image(schema, "cat.png"),
            schema.text("cd").unwrap(),
        ],
    )
}

/// Every image's `src`, in document order.
fn image_srcs(doc: &Node) -> Vec<String> {
    let mut v = Vec::new();
    for bi in 0..doc.child_count() {
        let b = doc.child(bi);
        for ci in 0..b.child_count() {
            let c = b.child(ci);
            if c.type_name() == "image" {
                v.push(c.attrs().get_str("src").unwrap_or("").to_string());
            }
        }
    }
    v
}

fn set_src(peer: &mut Peer, pos: usize, src: &str) {
    peer.local(|tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            pos,
            "src",
            AttrValue::from(src),
        )))
        .unwrap();
    });
}

/// One peer types `typed` right after the image while the other changes its `src`;
/// run under every client-id order and with either peer as the typer. Returns the
/// failing combinations (empty = pass), so the assertion names every one at once.
fn typing_after_image_vs_src_change(typed: &str) -> Vec<String> {
    let mut failures = Vec::new();
    for ids in ID_ORDERS {
        for a_types in [true, false] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers_with_ids(&schema, vec![line_with_image(&schema)], ids);
            let s = block_start(&a.state.doc, 0);
            let (typer, changer) = if a_types {
                (&mut a, &mut b)
            } else {
                (&mut b, &mut a)
            };
            typer.type_at(s + 4, typed);
            set_src(changer, s + 3, "new.png");
            sync(&mut a, &mut b);
            assert_converged(&a, &b, &schema);
            let srcs = image_srcs(&a.state.doc);
            let text = all_text(&a.state.doc);
            if srcs != ["new.png"] || !text.contains(typed) {
                failures.push(format!(
                    "ids {ids:?}, typer {}: images {srcs:?}, doc {}",
                    if a_types { "A" } else { "B" },
                    norm(&a.state.doc)
                ));
            }
        }
    }
    failures
}

#[test]
fn typing_right_after_an_image_keeps_a_peers_concurrent_src_change() {
    let failures = typing_after_image_vs_src_change("X");
    assert!(
        failures.is_empty(),
        "the typer's resync reverted the peer's src change:\n{}",
        failures.join("\n")
    );
}

#[test]
fn a_literal_placeholder_typed_after_an_image_keeps_a_peers_concurrent_src_change() {
    // A pasted U+FFFC is text, not an image — and typing it must not revert the image
    // beside it any more than typing a letter does.
    let failures = typing_after_image_vs_src_change("\u{FFFC}");
    assert!(
        failures.is_empty(),
        "the typer's resync reverted the peer's src change:\n{}",
        failures.join("\n")
    );
}

fn link_mark(schema: &Schema, href: &str) -> Mark {
    Mark::new(
        schema.mark_type("link").unwrap().clone(),
        Attrs::new().with("href", AttrValue::from(href)),
    )
}

/// `ab` linked to `old`, then plain `cd`: the position right after the link is block
/// offset 3.
fn line_with_link(schema: &Schema) -> Node {
    para_of(
        schema,
        vec![
            schema
                .text_with_marks("ab", vec![link_mark(schema, "old")])
                .unwrap(),
            schema.text("cd").unwrap(),
        ],
    )
}

/// One peer types `X` right after the link while the other runs `change` over the
/// link; under every client-id order and with either peer as the typer. Returns the
/// failing combinations (empty = pass), judged by `ok` on the converged document.
fn typing_after_link_vs(
    change: impl Fn(&mut Peer, usize),
    ok: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    for ids in ID_ORDERS {
        for a_types in [true, false] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers_with_ids(&schema, vec![line_with_link(&schema)], ids);
            let s = block_start(&a.state.doc, 0);
            let (typer, changer) = if a_types {
                (&mut a, &mut b)
            } else {
                (&mut b, &mut a)
            };
            typer.type_at(s + 3, "X");
            change(changer, s);
            sync(&mut a, &mut b);
            assert_converged(&a, &b, &schema);
            let n = norm(&a.state.doc);
            if !ok(&n) {
                failures.push(format!(
                    "ids {ids:?}, typer {}: {n}",
                    if a_types { "A" } else { "B" }
                ));
            }
        }
    }
    failures
}

#[test]
fn typing_after_a_link_while_a_peer_changes_its_href_keeps_the_new_href() {
    // The typed char is plain in the model (a link is non-inclusive). Resynced per
    // span, the typer rewrote the whole link with the old href and reverted the peer's
    // change; the typed char is now inserted with its own (empty) link attribute, and
    // the old href never comes back.
    //
    // **Known limitation, pinned (#923):** the peer's href change deletes the link's end marker
    // and writes a new one at the same spot, and the typed char sits right after the
    // old one. yrs orders those two siblings by client id, so in one order (the typer's
    // id is the lower) the typed char ends up *inside* the re-written link. Nobody's
    // edit is lost and the replicas converge; the typed char is linked where the typer
    // meant it plain. It is Yjs's concurrent-formatting boundary semantics — a peer
    // bolding the text after the link concurrently makes the char bold the same way.
    let failures = typing_after_link_vs(
        |p, s| {
            let l = link_mark(p.state.schema(), "new");
            p.local(|tr| {
                tr.add_mark(s + 1, s + 3, l).unwrap();
            });
        },
        |n| {
            n == "<paragraph >«ab|link[href=Str(\"new\")]»«Xcd|»</>"
                || n == "<paragraph >«abX|link[href=Str(\"new\")]»«cd|»</>"
        },
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn typing_after_a_link_while_a_peer_removes_it_does_not_bring_it_back() {
    let failures = typing_after_link_vs(
        |p, s| {
            let l = link_mark(p.state.schema(), "old");
            p.local(|tr| {
                tr.remove_mark(s + 1, s + 3, l).unwrap();
            });
        },
        |n| n == "<paragraph >«abXcd|»</>",
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The next two are the review of #901's D2: a peer's concurrent link over the text
// *after* the link. The typed char's projection used to be insert-then-clear: yrs put
// it inside the link, and `format(X, link: null)` cleared it — which also deleted the
// link's own end marker as redundant. The peer's formatting relied on that very marker,
// so after the merge the typer's null governed everything after `X` and the peer's link
// over `cd` was gone (extending: both client-id orders; a different href: one). Now the
// char is inserted **with** its attributes (`insert_with_attributes`), which steps past
// the existing end marker instead of writing markers of its own.  The price is that the
// typed char can end up inside the peer's link (pinned below and on the href fixture):
// a char formatted where the typer did not mean it, not a peer's edit lost.

#[test]
fn typing_after_a_link_while_a_peer_extends_it_over_the_next_text_keeps_the_extension() {
    let failures = typing_after_link_vs(
        |p, s| {
            let l = link_mark(p.state.schema(), "old");
            p.local(|tr| {
                tr.add_mark(s + 1, s + 5, l).unwrap();
            });
        },
        |n| {
            n == "<paragraph >«abXcd|link[href=Str(\"old\")]»</>"
                || n == "<paragraph >«ab|link[href=Str(\"old\")]»«X|»«cd|link[href=Str(\"old\")]»</>"
        },
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn typing_after_a_link_while_a_peer_links_the_next_text_elsewhere_keeps_both_links() {
    let failures = typing_after_link_vs(
        |p, s| {
            let l = link_mark(p.state.schema(), "other");
            p.local(|tr| {
                tr.add_mark(s + 3, s + 5, l).unwrap();
            });
        },
        |n| {
            // **Known limitation, pinned (#923):** the peer's link starts at the old link's end
            // marker, before the typed char in every order, so the typed char joins it
            // (see the href-change fixture above for the mechanism). The peer's link is
            // what must survive.
            n == "<paragraph >«ab|link[href=Str(\"old\")]»«X|»«cd|link[href=Str(\"other\")]»</>"
                || n == "<paragraph >«ab|link[href=Str(\"old\")]»«Xcd|link[href=Str(\"other\")]»</>"
        },
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A line that is only a link (nothing follows it in the block).
fn only_link_line(schema: &Schema) -> Node {
    para_of(
        schema,
        vec![
            schema
                .text_with_marks("ab", vec![link_mark(schema, "old")])
                .unwrap(),
        ],
    )
}

#[test]
fn typing_repeatedly_after_a_link_at_the_end_of_its_block_stays_plain() {
    // Nothing follows the link, so there is no end marker to step past: the insert has
    // to write a clear of its own, and every later char typed after it must stay plain.
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers_with_ids(&schema, vec![only_link_line(&schema)], ids);
        let s = block_start(&a.state.doc, 0);
        a.type_at(s + 3, "X");
        sync(&mut a, &mut b);
        a.type_at(s + 4, "Y");
        sync(&mut a, &mut b);
        b.type_at(s + 5, "Z");
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        assert_eq!(
            norm(&a.state.doc),
            "<paragraph >«ab|link[href=Str(\"old\")]»«XYZ|»</>",
            "ids {ids:?}"
        );
        assert_eq!(
            raw_block0_attr_chunks(&raw_replica(&a.session), "link"),
            vec![("ab".to_string(), true), ("XYZ".to_string(), false)],
            "ids {ids:?}"
        );
    }
}

#[test]
fn typing_after_a_link_at_the_end_of_its_block_while_a_peer_removes_it_brings_nothing_back() {
    // With nothing after the link, the typed char's insert writes a clear before itself
    // and yrs puts the link's value back after it — an empty range at the block's end.
    // That restore must not resurrect a link the peer removed concurrently, over the
    // typed char or over anything typed after it.
    let mut failures = vec![];
    for ids in ID_ORDERS {
        for a_types in [true, false] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers_with_ids(&schema, vec![only_link_line(&schema)], ids);
            let s = block_start(&a.state.doc, 0);
            let (t, c) = if a_types {
                (&mut a, &mut b)
            } else {
                (&mut b, &mut a)
            };
            t.type_at(s + 3, "X");
            let l = link_mark(c.state.schema(), "old");
            c.local(|tr| {
                tr.remove_mark(s + 1, s + 3, l).unwrap();
            });
            sync(&mut a, &mut b);
            a.type_at(s + 4, "Y");
            b.type_at(s + 4, "Z");
            sync(&mut a, &mut b);
            assert_converged(&a, &b, &schema);
            let n = norm(&a.state.doc);
            if n != "<paragraph >«abXYZ|»</>" && n != "<paragraph >«abXZY|»</>" {
                failures.push(format!("{ids:?} typer A {a_types}: {n}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn both_peers_typing_after_a_link_at_once_both_stay_plain() {
    let mut failures = vec![];
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers_with_ids(&schema, vec![line_with_link(&schema)], ids);
        let s = block_start(&a.state.doc, 0);
        a.type_at(s + 3, "X");
        b.type_at(s + 3, "Y");
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        let n = norm(&a.state.doc);
        let plain = |t: &str| format!("<paragraph >«ab|link[href=Str(\"old\")]»«{t}|»</>");
        if n != plain("XYcd") && n != plain("YXcd") {
            failures.push(format!("{ids:?}: {n}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn typing_inside_a_link_while_a_peer_changes_its_href_keeps_the_new_href() {
    let mut failures = vec![];
    for ids in ID_ORDERS {
        for a_types in [true, false] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers_with_ids(&schema, vec![line_with_link(&schema)], ids);
            let s = block_start(&a.state.doc, 0);
            let (t, c) = if a_types {
                (&mut a, &mut b)
            } else {
                (&mut b, &mut a)
            };
            t.type_at(s + 2, "X");
            let l = link_mark(c.state.schema(), "new");
            c.local(|tr| {
                tr.add_mark(s + 1, s + 3, l).unwrap();
            });
            sync(&mut a, &mut b);
            assert_converged(&a, &b, &schema);
            let n = norm(&a.state.doc);
            if n != "<paragraph >«aXb|link[href=Str(\"new\")]»«cd|»</>" {
                failures.push(format!("{ids:?} typer A {a_types}: {n}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn typing_after_a_link_followed_by_an_image_keeps_a_peers_src_change() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let line = para_of(
            &schema,
            vec![
                schema
                    .text_with_marks("ab", vec![link_mark(&schema, "old")])
                    .unwrap(),
                image(&schema, "c.png"),
                schema.text("cd").unwrap(),
            ],
        );
        let (mut a, mut b) = two_peers_with_ids(&schema, vec![line], ids);
        let s = block_start(&a.state.doc, 0);
        a.type_at(s + 3, "X");
        set_src(&mut b, s + 3, "new.png");
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        let n = norm(&a.state.doc);
        assert!(
            n.contains("new.png") && n.contains("«ab|link[href=Str(\"old\")]»«X|»"),
            "{ids:?}: {n}"
        );
    }
}

#[test]
fn a_char_typed_after_a_link_reaches_the_peer_unlinked() {
    // The model says the typed char is not linked (`link` is non-inclusive); the
    // projection must say so in the CRDT itself — an explicit clear over that char —
    // or a peer would rebuild it inside the link.
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers_with_ids(&schema, vec![line_with_link(&schema)], (11, 22));
    let s = block_start(&a.state.doc, 0);
    a.type_at(s + 3, "X");
    assert_eq!(
        norm(&a.state.doc),
        "<paragraph >«ab|link[href=Str(\"old\")]»«Xcd|»</>"
    );
    assert_eq!(
        raw_block0_attr_chunks(&raw_replica(&a.session), "link"),
        vec![("ab".to_string(), true), ("Xcd".to_string(), false)],
        "the CRDT holds the typed char outside the link"
    );
    let delta = a.session.save_incremental().unwrap();
    b.state = b
        .session
        .integrate_incremental(&b.state, &delta)
        .unwrap()
        .expect("the delta changes B");
    assert_converged(&a, &b, &schema);
    assert_eq!(norm(&b.state.doc), norm(&a.state.doc));

    // Bold stays inclusive: a char typed after it is bold on both sides.
    let bold_line = para_of(
        &schema,
        vec![
            schema
                .text_with_marks(
                    "ab",
                    vec![Mark::simple(schema.mark_type("bold").unwrap().clone())],
                )
                .unwrap(),
            schema.text("cd").unwrap(),
        ],
    );
    let (mut a, mut b) = two_peers_with_ids(&schema, vec![bold_line], (11, 22));
    a.type_at(s + 3, "X");
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(norm(&b.state.doc), "<paragraph >«abX|bold[]»«cd|»</>");
}

#[test]
fn two_adjacent_identical_images_edited_concurrently_converge() {
    // **A known limitation, pinned for convergence only.** Two identical images side by
    // side are, at the CRDT level, one `@atom` formatting range with one value. Each
    // peer changing *one* of them writes a formatting marker at the boundary between
    // the two chars, and yrs orders two concurrent markers at one boundary by client id
    // — so one peer's edit can be overwritten by the other's restore of the neighbour
    // it did not touch. The replicas still converge (asserted), but one of the two
    // changes may be lost; which one depends on the client-id order. This is yrs/Yjs
    // concurrent-formatting semantics and cannot be fixed inside a formatting encoding.
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let line = para_of(
            &schema,
            vec![
                schema.text("a").unwrap(),
                image(&schema, "cat.png"),
                image(&schema, "cat.png"),
                schema.text("b").unwrap(),
            ],
        );
        let (mut a, mut b) = two_peers_with_ids(&schema, vec![line], ids);
        let s = block_start(&a.state.doc, 0);
        set_src(&mut a, s + 2, "one.png");
        set_src(&mut b, s + 3, "two.png");
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        assert_eq!(image_srcs(&a.state.doc).len(), 2, "both images survive");
    }
}

// --- a stray `@atom` span arriving from a peer (review of #838, F2) ------------------

/// Block 0's text in raw yrs terms: `(chunk string, carries "@atom")` per diff chunk.
fn raw_block0_chunks(doc: &yrs::Doc) -> Vec<(String, bool)> {
    raw_block0_attr_chunks(doc, "@atom")
}

/// Block 0's text in raw yrs terms: `(chunk string, carries attribute key)` per diff
/// chunk — the CRDT's own formatting, before any projection back into a model.
fn raw_block0_attr_chunks(doc: &yrs::Doc, key: &str) -> Vec<(String, bool)> {
    use yrs::{Array, Map, Text, Transact};
    let content = doc.get_or_insert_array("content");
    let txn = doc.transact();
    let Some(yrs::Out::YMap(node)) = content.get(&txn, 0) else {
        panic!("block 0 must be a node map");
    };
    let Some(yrs::Out::YText(text)) = node.get(&txn, "text") else {
        panic!("block 0 must carry a text");
    };
    text.diff(&txn, yrs::types::text::YChange::identity)
        .into_iter()
        .map(|d| {
            let yrs::Out::Any(yrs::Any::String(s)) = &d.insert else {
                panic!("block 0 holds only string chunks");
            };
            let on = d
                .attributes
                .as_ref()
                .is_some_and(|a| a.get(key).is_some_and(|v| *v != yrs::Any::Null));
            (s.to_string(), on)
        })
        .collect()
}

/// A raw yrs replica of `session`'s document, with the projection's UTF-16 offsets
/// (`Doc::new()` counts bytes, and an index into the 3-byte U+FFFC then splits it).
fn raw_replica(session: &CollabSession) -> yrs::Doc {
    use yrs::updates::decoder::Decode;
    use yrs::{Transact, Update};
    let doc = yrs::Doc::with_options(yrs::Options {
        offset_kind: yrs::OffsetKind::Utf16,
        ..Default::default()
    });
    doc.transact_mut()
        .apply_update(Update::decode_v1(&session.snapshot()).unwrap())
        .unwrap();
    doc
}

#[test]
fn a_stray_atom_attribute_from_a_peer_is_text_and_is_cleared_by_the_next_local_edit() {
    // Through the projection a typer never *sends* a stray `@atom` span: its own
    // `resync_marks` clears the one yrs gave its new chars in the same transaction. A
    // peer that is not this projection can send one, though — any Yjs client inserting
    // right after the placeholder with no formatting of its own inherits the range, and
    // so does a projection built before the per-char resync. This drives that update
    // through `integrate_incremental`, which is the only way to reach `is_atom_char`'s
    // "attribute over a char that is not the placeholder" rule from outside the crate.
    use yrs::updates::decoder::Decode;
    use yrs::{Array, Map, ReadTxn, StateVector, Text, Transact};

    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers_with_ids(&schema, vec![line_with_image(&schema)], (11, 22));

    // A raw Yjs peer types "XY" right after the placeholder (UTF-16 offset 3), with no
    // attributes of its own.
    let raw = raw_replica(&a.session);
    {
        let content = raw.get_or_insert_array("content");
        let mut txn = raw.transact_mut();
        let Some(yrs::Out::YMap(node)) = content.get(&txn, 0) else {
            panic!("block 0 must be a node map");
        };
        let Some(yrs::Out::YText(text)) = node.get(&txn, "text") else {
            panic!("block 0 must carry a text");
        };
        text.insert(&mut txn, 3, "XY");
    }
    // Positive control: the update really carries the stray span — "XY" arrives inside
    // the image's `@atom` range. Without this the rest of the test could pass on an
    // update that never exercised the rule.
    assert_eq!(
        raw_block0_chunks(&raw),
        vec![
            ("ab".to_string(), false),
            ("\u{FFFC}XY".to_string(), true),
            ("cd".to_string(), false),
        ],
        "the forged update must put `@atom` over the typed chars"
    );
    let delta = {
        let sv = StateVector::decode_v1(&a.session.state_vector()).unwrap();
        raw.transact().encode_diff_v1(&sv)
    };

    // A integrates it: one image, and "XY" is text — not two more images, and not an
    // error (which would poison the session over a formatting artifact).
    let next = a
        .session
        .integrate_incremental(&a.state, &delta)
        .expect("a stray atom attribute is not corruption")
        .expect("the document changed");
    a.state = next;
    assert_eq!(image_srcs(&a.state.doc), vec!["cat.png".to_string()]);
    assert_eq!(all_text(&a.state.doc), "abXYcd");
    assert_eq!(
        norm(&a.state.doc),
        norm(&a.session.projected_doc(&schema).unwrap()),
        "model ≡ projection"
    );

    // The stray span persists in the CRDT until this block is next reconciled locally;
    // A's next edit in it clears the stray chars — and only those.
    a.type_at(block_content_end(&a.state.doc, 0), "!");
    assert_eq!(
        raw_block0_chunks(&raw_replica(&a.session)),
        vec![
            ("ab".to_string(), false),
            ("\u{FFFC}".to_string(), true),
            ("XYcd!".to_string(), false),
        ],
        "the next local resync clears `@atom` from the stray chars and keeps the image's"
    );
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(image_srcs(&b.state.doc), vec!["cat.png".to_string()]);
}

#[test]
fn pasting_two_different_images_side_by_side_keeps_each_its_own_attrs() {
    // One transaction inserting two *different* adjacent atoms: the per-char resync must
    // write each char's own value, never one value across a run of two atoms. (A single
    // inserted atom cannot tell those apart — a run then has one char.)
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers_with_ids(&schema, vec![para(&schema, "abcd")], (11, 22));
    let two = Fragment::from_children(vec![image(&schema, "one.png"), image(&schema, "two.png")]);
    a.local(|tr| {
        tr.replace(3, 3, Slice::new(two, 0, 0)).unwrap();
    });
    assert_eq!(
        image_srcs(&a.session.projected_doc(&schema).unwrap()),
        vec!["one.png".to_string(), "two.png".to_string()],
        "model ≡ project(model) on the paste itself"
    );
    sync(&mut a, &mut b);
    assert_converged(&a, &b, &schema);
    assert_eq!(
        image_srcs(&b.state.doc),
        vec!["one.png".to_string(), "two.png".to_string()]
    );
}

// ===== Review of #901, round 2: fixtures (append to crates/rinch-editor-collab/tests/collab.rs) =====
// F1–F4 FAIL on 1f5833d6. F1/F2 passed on e4a5fc7e (insert-then-clear) and on main; F3/F4 fail
// on e4a5fc7e too and pass on main. F5/F6 pass on 1f5833d6 and pin two surviving mutants.

/// Typer types `X` right after the link at block offset 3; the other peer runs `change`.
/// Every client-id order, either peer typing. Returns the converged docs that `ok` rejects.
fn r2_vs(
    line: impl Fn(&Schema) -> Node,
    change: impl Fn(&mut Peer, usize),
    ok: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    for ids in ID_ORDERS {
        for a_types in [true, false] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers_with_ids(&schema, vec![line(&schema)], ids);
            let s = block_start(&a.state.doc, 0);
            let (typer, changer) = if a_types {
                (&mut a, &mut b)
            } else {
                (&mut b, &mut a)
            };
            typer.type_at(s + 3, "X");
            change(changer, s);
            sync(&mut a, &mut b);
            assert_converged(&a, &b, &schema);
            let n = norm(&a.state.doc);
            if !ok(&n) {
                failures.push(format!(
                    "ids {ids:?}, typer {}: {n}",
                    if a_types { "A" } else { "B" }
                ));
            }
        }
    }
    failures
}

fn r2_bold(schema: &Schema) -> Mark {
    Mark::simple(schema.mark_type("bold").unwrap().clone())
}

fn r2_unlink(href: &'static str, from: usize, to: usize) -> impl Fn(&mut Peer, usize) {
    move |p: &mut Peer, s: usize| {
        let l = link_mark(p.state.schema(), href);
        p.local(|tr| {
            tr.remove_mark(s + from, s + to, l).unwrap();
        });
    }
}

#[test]
fn typing_after_a_bold_link_while_a_peer_removes_the_link_links_nothing_else() {
    // F1. `ab` is bold AND linked. Measured on 1f5833d6: `«abX|bold»«cd|link old»` in all
    // four combinations — the removed link comes back over `cd`, which was never linked.
    let f = r2_vs(
        |sc| {
            para_of(
                sc,
                vec![
                    sc.text_with_marks("ab", vec![link_mark(sc, "old"), r2_bold(sc)])
                        .unwrap(),
                    sc.text("cd").unwrap(),
                ],
            )
        },
        r2_unlink("old", 1, 3),
        |n| n == "<paragraph >«abX|bold[]»«cd|»</>",
    );
    assert!(f.is_empty(), "{}", f.join("\n"));
}

#[test]
fn typing_after_a_link_followed_by_an_image_while_a_peer_removes_the_link_links_nothing_else() {
    // F2. Measured on 1f5833d6: the image and `cd` come back linked to `old`, all four.
    let f = r2_vs(
        |sc| {
            para_of(
                sc,
                vec![
                    sc.text_with_marks("ab", vec![link_mark(sc, "old")])
                        .unwrap(),
                    image(sc, "c.png"),
                    sc.text("cd").unwrap(),
                ],
            )
        },
        r2_unlink("old", 1, 3),
        |n| !n.contains("link["),
    );
    assert!(f.is_empty(), "{}", f.join("\n"));
}

fn r2_two_links(sc: &Schema) -> Node {
    para_of(
        sc,
        vec![
            sc.text_with_marks("ab", vec![link_mark(sc, "old")])
                .unwrap(),
            sc.text_with_marks("cd", vec![link_mark(sc, "other")])
                .unwrap(),
            sc.text("ef").unwrap(),
        ],
    )
}

#[test]
fn typing_between_two_links_while_a_peer_removes_the_second_links_nothing_else() {
    // F3. Measured on 1f5833d6: `«cdef|link old»` in 2/4 (the first link spreads over the
    // unlinked `cd` and the never-linked `ef`); on e4a5fc7e `«cdef|link other»` 4/4. main: clean.
    // X typed at the seam of two links continues the first (`ResolvedPos::marks`), so it
    // needs no formatting marker; the peer's concurrent removal of the second link may
    // leave it plain instead, depending on the client-id order. Never linked beyond `abX`.
    let f = r2_vs(r2_two_links, r2_unlink("other", 3, 5), |n| {
        n == "<paragraph >«abX|link[href=Str(\"old\")]»«cdef|»</>"
            || n == "<paragraph >«ab|link[href=Str(\"old\")]»«Xcdef|»</>"
    });
    assert!(f.is_empty(), "{}", f.join("\n"));
}

#[test]
fn typing_between_two_links_while_a_peer_removes_both_leaves_no_link() {
    // F4. Measured on 1f5833d6: `«abX|»«cdef|link old»` 4/4; e4a5fc7e: `link other` 4/4.
    let f = r2_vs(
        r2_two_links,
        |p, s| {
            r2_unlink("old", 1, 3)(p, s);
            r2_unlink("other", 3, 5)(p, s);
        },
        |n| n == "<paragraph >«abXcdef|»</>",
    );
    assert!(f.is_empty(), "{}", f.join("\n"));
}

#[test]
fn typing_after_a_bold_link_while_a_peer_unbolds_it_keeps_the_unbold() {
    // F5 — pins mutant M1 (splice_min_with_marks writes only the non-inclusive keys):
    // under it the peer's unbold of `ab` is lost in all four combinations.
    let f = r2_vs(
        |sc| {
            para_of(
                sc,
                vec![
                    sc.text_with_marks("ab", vec![link_mark(sc, "old"), r2_bold(sc)])
                        .unwrap(),
                    sc.text("cd").unwrap(),
                ],
            )
        },
        |p, s| {
            let b = r2_bold(p.state.schema());
            p.local(|tr| {
                tr.remove_mark(s + 1, s + 3, b).unwrap();
            });
        },
        |n| n.starts_with("<paragraph >«ab|link[href=Str(\"old\")]»«X"),
    );
    assert!(f.is_empty(), "{}", f.join("\n"));
}

#[test]
fn a_multi_run_insert_of_astral_text_after_a_link_lands_in_order() {
    // F6 — pins mutant M2 (`at += run.len()`, bytes instead of UTF-16 units): under it the
    // second run lands after `cd` and model ≢ projection.
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers_with_ids(&schema, vec![line_with_link(&schema)], ids);
        let s = block_start(&a.state.doc, 0);
        let bm = r2_bold(&schema);
        a.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(s + 3)));
            tr.insert_text("😀éP").unwrap();
            tr.add_mark(s + 3, s + 4, bm).unwrap();
        });
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        assert_eq!(
            norm(&a.state.doc),
            "<paragraph >«ab|link[href=Str(\"old\")]»«😀|bold[]»«éPcd|»</>",
            "ids {ids:?}"
        );
    }
}

#[test]
fn typing_after_a_bold_link_into_bold_text_while_a_peer_unbolds_the_link_keeps_the_unbold() {
    // The attribute-insert path proper: `cd` is bold too, so the typed char (bold, not
    // linked) carries exactly what `cd` carries and is inserted with those attributes.
    // Under a mutant that writes only the non-inclusive keys, yrs unsets bold on it, and
    // the per-span bold resync then rewrites bold over `abXcd`, reverting the unbold.
    let f = r2_vs(
        |sc| {
            para_of(
                sc,
                vec![
                    sc.text_with_marks("ab", vec![link_mark(sc, "old"), r2_bold(sc)])
                        .unwrap(),
                    sc.text_with_marks("cd", vec![r2_bold(sc)]).unwrap(),
                ],
            )
        },
        |p, s| {
            let b = r2_bold(p.state.schema());
            p.local(|tr| {
                tr.remove_mark(s + 1, s + 3, b).unwrap();
            });
        },
        |n| n == "<paragraph >«ab|link[href=Str(\"old\")]»«Xcd|bold[]»</>",
    );
    assert!(f.is_empty(), "{}", f.join("\n"));
}

#[test]
fn a_multi_run_insert_after_a_link_whose_first_run_is_astral_lands_in_order() {
    // The attribute-insert path with two runs: `😀` plain (what `cd` carries), then `éP`
    // bold. Under a mutant that advances by bytes instead of UTF-16 units, the second
    // run lands past `cd`.
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers_with_ids(&schema, vec![line_with_link(&schema)], ids);
        let s = block_start(&a.state.doc, 0);
        let bm = r2_bold(&schema);
        a.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(s + 3)));
            tr.insert_text("😀éP").unwrap();
            tr.add_mark(s + 4, s + 6, bm).unwrap();
        });
        sync(&mut a, &mut b);
        assert_converged(&a, &b, &schema);
        assert_eq!(
            norm(&b.state.doc),
            "<paragraph >«ab|link[href=Str(\"old\")]»«😀|»«éP|bold[]»«cd|»</>",
            "ids {ids:?}"
        );
    }
}
