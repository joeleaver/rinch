//! The poisoned-session contract (issue #196): a live session that integrates bytes
//! leaving the shared CRDT **unprojectable** must go loud in **both** directions —
//! sticky [`CollabError::SessionPoisoned`] on inbound *and* outbound — instead of
//! one-way partitioning (inbound dead forever while `record_local` keeps `Ok`-ing and
//! `save_incremental` keeps broadcasting).
//!
//! The probe matrix these tests pin (mid-session integrate of foreign whole-doc
//! bytes, by the foreign `content` root's type):
//!
//! * **Map root** — entries land in the array branch's invisible map component;
//!   harmless, must stay accepted-and-ignored.
//! * **Text root** — the characters read back as non-map array entries; the rebuild
//!   fails forever → poison.
//! * **Array of scalars** — same unprojectable read-back → poison (the same sticky
//!   kind, converged deliberately).
//! * **Array holding a valid projected block** — a legitimate merge; the transport
//!   chose the peers, not a defect. Must stay adopted.
//!
//! Plus the boundary cases: undecodable bytes never touch the CRDT (transient, no
//! poison); **interior damage** — a shared-lineage delta parking an embed inside one
//! block's text, which changes no block count and so slipped every #218 gate: on
//! unfixed code that was the last remaining one-way partition (integrate `Err`
//! forever, `record_local` of *another* block `Ok`, a delta broadcast); a rebuild
//! failure with updates parked on **missing dependencies**, which must stay
//! transient because the missing delta cures it (review finding F1); and the
//! **heal**: poison clears when an inbound update makes the document rebuildable
//! again, while outbound stays refused for the whole poisoned window.

use std::rc::Rc;

use rinch_editor_collab::{CollabError, CollabSession};
use rinch_editor_core::{
    Attrs, EditorState, Fragment, Node, Pos, Schema, Selection, default_plugins,
};
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Array, Doc, Map, MapPrelim, ReadTxn, StateVector, Text, TextPrelim, Transact, Update,
};

// --- harness -------------------------------------------------------------------

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

fn heading(schema: &Schema, level: i64, text: &str) -> Node {
    schema
        .create_node(
            "heading",
            Attrs::new().with("level", level),
            Fragment::from_node(schema.text(text).unwrap()),
        )
        .unwrap()
}

fn code_block(schema: &Schema, text: &str) -> Node {
    schema
        .branch(
            "code_block",
            Fragment::from_node(schema.text(text).unwrap()),
        )
        .unwrap()
}

/// One live editor+session, its initial projection already drained (mid-session).
struct Live {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

fn live(texts: &[&str]) -> Live {
    let schema = Rc::new(Schema::starter_kit());
    let blocks = texts.iter().map(|t| para(&schema, t)).collect();
    let state = EditorState::create(schema.clone(), doc_of(&schema, blocks), default_plugins());
    let mut session = CollabSession::new(&state).unwrap();
    let _ = session.save_incremental().unwrap(); // drain the initial projection
    Live {
        schema,
        state,
        session,
    }
}

impl Live {
    /// Insert `text` at `pos` in the model and record it onto the CRDT, returning
    /// the projection result (the state advances regardless, as a real editor's
    /// would — the edit is already applied when projection is attempted).
    fn type_at(&mut self, pos: usize, text: &str) -> Result<(), CollabError> {
        let mut tr = self.state.tr();
        tr.set_selection(Selection::cursor(Pos(pos)));
        tr.insert_text(text).unwrap();
        let before = self.state.doc.clone();
        let next = self.state.apply(tr);
        let r = self.session.record_local(&self.schema, &before, &next.doc);
        self.state = next;
        r
    }

    /// A valid delta from a legitimate peer that joined this session's snapshot and
    /// typed one character.
    fn valid_peer_delta(&self) -> Vec<u8> {
        let mut peer = CollabSession::from_bytes(&self.session.snapshot()).unwrap();
        let doc = peer.projected_doc(&self.schema).unwrap();
        let mut state = EditorState::create(self.schema.clone(), doc, default_plugins());
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(1)));
        tr.insert_text("P").unwrap();
        let before = state.doc.clone();
        state = state.apply(tr);
        peer.record_local(&self.schema, &before, &state.doc)
            .unwrap();
        let delta = peer.save_incremental().unwrap();
        assert!(!delta.is_empty(), "the peer edit must produce a delta");
        delta
    }
}

/// Whole-document bytes of a foreign yrs doc built by `build` — bytes that decode
/// and apply fine but were never one of our projections.
fn foreign_update(build: impl FnOnce(&Doc)) -> Vec<u8> {
    let doc = Doc::new();
    build(&doc);
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}

/// Assert the full sticky contract after a poisoning integrate: every direction of
/// the session fails with `SessionPoisoned`, and no broadcast delta escapes.
/// `valid_delta` is a legitimate peer delta captured **before** the poisoning, so the
/// inbound refusal is shown on bytes that would otherwise integrate fine.
fn assert_poisoned_both_directions(l: &mut Live, valid_delta: &[u8], what: &str) {
    assert!(l.session.is_poisoned(), "{what}: is_poisoned");
    assert!(
        matches!(
            l.session.projected_doc(&l.schema),
            Err(CollabError::SessionPoisoned(_))
        ),
        "{what}: projected_doc is sticky-refused"
    );
    // Outbound: a local edit must NOT be projected-and-broadcast.
    let r = l.type_at(1, "L");
    assert!(
        matches!(r, Err(CollabError::SessionPoisoned(_))),
        "{what}: record_local must fail sticky, got {r:?}"
    );
    assert!(
        matches!(
            l.session.save_incremental(),
            Err(CollabError::SessionPoisoned(_))
        ),
        "{what}: save_incremental must fail sticky (no delta may leave)"
    );
    // Reconciliation: a poisoned document's content must not reach healthy peers.
    let sv = l.session.state_vector();
    assert!(
        matches!(
            l.session.sync_diff(&sv),
            Err(CollabError::SessionPoisoned(_))
        ),
        "{what}: sync_diff must fail sticky"
    );
    // Inbound: integration is still *attempted*, but neither a perfectly valid peer
    // delta nor the empty update removes the junk, so the rebuild keeps failing and
    // both report the sticky kind — an app polling errors sees "dead", not a mix.
    let state = l.state.clone();
    assert!(
        matches!(
            l.session.integrate_incremental(&state, valid_delta),
            Err(CollabError::SessionPoisoned(_))
        ),
        "{what}: a valid peer delta is sticky-refused once poisoned"
    );
    assert!(
        matches!(
            l.session.integrate_incremental(&state, &[0, 0]),
            Err(CollabError::SessionPoisoned(_))
        ),
        "{what}: even the empty update is sticky-refused once poisoned"
    );
    // An inbound failure that never touches the CRDT (an undecodable blob) also
    // reports the sticky kind while poisoned, not its own transient kind — the
    // shape `sticky_or` exists for (#219 review, R2-2).
    assert!(
        matches!(
            l.session
                .integrate_incremental(&state, &[0xff, 0xff, 0xff, 0xff]),
            Err(CollabError::SessionPoisoned(_))
        ),
        "{what}: an undecodable blob while poisoned reports the sticky kind"
    );
    assert!(
        l.session.is_poisoned(),
        "{what}: still poisoned after the blob"
    );
}

// --- the matrix ----------------------------------------------------------------

#[test]
fn foreign_text_root_bytes_poison_the_session_in_both_directions() {
    // The issue's headline shape: `content` was created as a Text type in some
    // unrelated yrs document. On unfixed code this one-way partitioned — integrate
    // failed forever with Schema("read_node_data: missing node") while the local
    // user kept typing and broadcasting.
    let mut l = live(&["hello"]);
    let valid_delta = l.valid_peer_delta();
    let bytes = foreign_update(|d| {
        let t = d.get_or_insert_text("content");
        t.insert(&mut d.transact_mut(), 0, "foreign");
    });
    let state = l.state.clone();
    let err = l.session.integrate_incremental(&state, &bytes).unwrap_err();
    assert!(
        matches!(err, CollabError::SessionPoisoned(_)),
        "the poisoning integrate itself returns the sticky kind, got {err:?}"
    );
    assert_poisoned_both_directions(&mut l, &valid_delta, "text root");
}

#[test]
fn foreign_scalar_array_bytes_poison_the_same_way() {
    // An array root holding scalars instead of node maps: unprojectable for the same
    // reason, and deliberately converged on the same sticky kind.
    let mut l = live(&["hello"]);
    let valid_delta = l.valid_peer_delta();
    let bytes = foreign_update(|d| {
        let a = d.get_or_insert_array("content");
        a.insert(&mut d.transact_mut(), 0, Any::String("junk".into()));
    });
    let state = l.state.clone();
    let err = l.session.integrate_incremental(&state, &bytes).unwrap_err();
    assert!(
        matches!(err, CollabError::SessionPoisoned(_)),
        "got {err:?}"
    );
    assert_poisoned_both_directions(&mut l, &valid_delta, "scalar array");
}

#[test]
fn foreign_map_root_bytes_stay_harmless() {
    // A foreign Map root named `content`: its entries land in the array branch's map
    // component, invisible to list reads. Harmless before the fix, and must stay so —
    // the rebuild succeeds, so nothing may poison.
    let mut l = live(&["hello"]);
    let bytes = foreign_update(|d| {
        let m = d.get_or_insert_map("content");
        m.insert(&mut d.transact_mut(), "k", Any::Bool(true));
    });
    let state = l.state.clone();
    assert!(
        l.session
            .integrate_incremental(&state, &bytes)
            .unwrap()
            .is_none(),
        "map-root entries are invisible to the projection"
    );
    assert!(!l.session.is_poisoned());
    assert!(l.type_at(1, "X").is_ok(), "local edits still project");
    assert!(
        !l.session.save_incremental().unwrap().is_empty(),
        "and still broadcast"
    );
    let delta = l.valid_peer_delta();
    let state = l.state.clone();
    assert!(
        l.session
            .integrate_incremental(&state, &delta)
            .unwrap()
            .is_some(),
        "a valid peer delta still integrates"
    );
}

#[test]
fn a_valid_foreign_block_is_still_adopted() {
    // An array root holding a well-formed projected node: a legitimate block merge —
    // the transport chose the peers, not a defect. Must NOT regress into poison.
    let mut l = live(&["hello"]);
    let bytes = foreign_update(|d| {
        let a = d.get_or_insert_array("content");
        let mut txn = d.transact_mut();
        let node = a.insert(&mut txn, 0, MapPrelim::default());
        node.insert(&mut txn, "type", Any::String("paragraph".into()));
        node.insert(&mut txn, "attrs", MapPrelim::default());
        node.insert(&mut txn, "text", TextPrelim::new("peer"));
    });
    let state = l.state.clone();
    let next = l
        .session
        .integrate_incremental(&state, &bytes)
        .unwrap()
        .expect("the foreign block is a real document change");
    l.state = next;
    assert!(!l.session.is_poisoned());
    let all: String = (0..l.state.doc.child_count())
        .map(|i| {
            let b = l.state.doc.child(i);
            (0..b.child_count())
                .filter_map(|j| b.child(j).text().map(str::to_string))
                .collect::<String>()
        })
        .collect();
    assert!(
        all.contains("peer") && all.contains("hello"),
        "both blocks live in the adopted document: {all}"
    );
    assert!(l.type_at(1, "X").is_ok(), "collaboration continues");
    assert!(!l.session.save_incremental().unwrap().is_empty());
}

#[test]
fn undecodable_bytes_do_not_poison() {
    // A garbage blob fails to decode, so the CRDT is untouched — a transient Engine
    // error. The next valid delta must integrate normally.
    let mut l = live(&["hello"]);
    let state = l.state.clone();
    let err = l
        .session
        .integrate_incremental(&state, &[0xff, 0xff, 0xff, 0xff])
        .unwrap_err();
    assert!(
        matches!(err, CollabError::Engine(_)),
        "a decode failure is transient, got {err:?}"
    );
    assert!(!l.session.is_poisoned());
    let delta = l.valid_peer_delta();
    assert!(
        l.session
            .integrate_incremental(&state, &delta)
            .unwrap()
            .is_some(),
        "the next valid delta integrates — no sticky state"
    );
    assert!(l.type_at(1, "X").is_ok());
}

#[test]
fn interior_damage_with_shared_lineage_poisons_and_silences_the_broadcast() {
    // The last one-way partition on unfixed code (probe-verified): a delta with
    // SHARED lineage (forked from our snapshot) parks an embed inside block 0's
    // text. The top-level block count is unchanged, so #218's gate 1 cannot see it,
    // and a local edit to block 1 skips damaged block 0 by Rc identity, so gates 2–3
    // cannot either. Unfixed: integrate `Err(Unsupported)` forever, record_local of
    // block 1 `Ok`, save_incremental a 24-byte broadcast — inbound-dead,
    // outbound-alive. Fixed: sticky poison, silent in no direction.
    let mut l = live(&["alpha", "beta"]);

    let peer_doc = Doc::new();
    {
        let update = Update::decode_v1(&l.session.snapshot()).unwrap();
        peer_doc.transact_mut().apply_update(update).unwrap();
    }
    {
        let content = peer_doc.get_or_insert_array("content");
        let mut txn = peer_doc.transact_mut();
        let Some(yrs::Out::YMap(node)) = content.get(&txn, 0) else {
            panic!("block 0 must be a node map");
        };
        let Some(yrs::Out::YText(text)) = node.get(&txn, "text") else {
            panic!("block 0 must carry a text");
        };
        text.insert_embed(&mut txn, 2, Any::Bool(true));
    }
    let delta = {
        let sv = StateVector::decode_v1(&l.session.state_vector()).unwrap();
        peer_doc.transact().encode_diff_v1(&sv)
    };

    let state = l.state.clone();
    let err = l.session.integrate_incremental(&state, &delta).unwrap_err();
    assert!(
        matches!(err, CollabError::SessionPoisoned(_)),
        "interior damage poisons on the integrate that applied it, got {err:?}"
    );
    // The headline: an edit to the UNDAMAGED block must now refuse loudly instead of
    // projecting and broadcasting from an inbound-dead replica.
    let r = l.type_at(8, "L"); // inside "beta"
    assert!(
        matches!(r, Err(CollabError::SessionPoisoned(_))),
        "record_local of the other block must fail sticky, got {r:?}"
    );
    assert!(
        matches!(
            l.session.save_incremental(),
            Err(CollabError::SessionPoisoned(_))
        ),
        "no delta may leave the poisoned replica"
    );
    assert!(l.session.is_poisoned());
}

#[test]
fn a_misrouted_reconciliation_diff_with_missing_dependencies_is_transient_and_heals() {
    // PR #219 review finding F1. A rebuild failure while yrs holds updates parked on
    // **missing dependencies** is NOT the permanent class: the missing bytes cure it.
    //
    // The reachable shape: C rewrites block 0's `type` (delete old item + insert
    // C-item); B — who has seen C — rewrites it again (delete C-item + insert
    // B-item). B answers *C's* state vector with a reconciliation diff, which a hub
    // fans to A as opaque bytes (diffs and broadcasts are indistinguishable on the
    // wire). That diff carries B's insertion and the COMPLETE delete set
    // (`encode_diff_v1` always writes it) but not C's insertion — so at A the
    // original `type` item is deleted while B's successor parks on the missing
    // C-dependency, and the node reads back with no type. Poisoning here would turn
    // a self-healing state into a dead one AND refuse the very bytes that heal it.
    let schema = Rc::new(Schema::starter_kit());
    let doc0 = doc_of(&schema, vec![para(&schema, "hello")]);
    let a_state = EditorState::create(schema.clone(), doc0.clone(), default_plugins());
    let mut a = CollabSession::new(&a_state).unwrap();
    let _ = a.save_incremental().unwrap();

    let mut b = CollabSession::from_bytes(&a.snapshot()).unwrap();
    let mut c = CollabSession::from_bytes(&a.snapshot()).unwrap();

    // C rewrites the block's type: paragraph -> heading.
    let heading_doc = doc_of(&schema, vec![heading(&schema, 1, "hello")]);
    c.record_local(&schema, &doc0, &heading_doc).unwrap();
    let delta_c = c.save_incremental().unwrap();
    assert!(!delta_c.is_empty());

    // B integrates C, then rewrites the type again: heading -> code_block.
    let b_state = EditorState::create(schema.clone(), doc0.clone(), default_plugins());
    let b_state = b
        .integrate_incremental(&b_state, &delta_c)
        .unwrap()
        .expect("B adopts C's rewrite");
    let code_doc = doc_of(&schema, vec![code_block(&schema, "hello")]);
    b.record_local(&schema, &b_state.doc, &code_doc).unwrap();
    let _ = b.save_incremental().unwrap();

    // The misroute: B's reconciliation answer FOR C, delivered to A.
    let misrouted = b.sync_diff(&c.state_vector()).unwrap();
    let err = a.integrate_incremental(&a_state, &misrouted).unwrap_err();
    assert!(
        !matches!(err, CollabError::SessionPoisoned(_)),
        "a dependency-pending rebuild failure must stay transient, got {err:?}"
    );
    assert!(
        !a.is_poisoned(),
        "the session must not poison while updates are parked on missing dependencies"
    );

    // The missing dependency arrives (C's ordinary broadcast); the session heals.
    let healed = a
        .integrate_incremental(&a_state, &delta_c)
        .unwrap()
        .expect("the healing delta is a real document change");
    assert!(!a.is_poisoned());
    assert_eq!(
        healed.doc.child(0).type_name(),
        "code_block",
        "A converged on B's rewrite once the dependency arrived"
    );
    let pa = a.projected_doc(&schema).unwrap();
    let pb = b.projected_doc(&schema).unwrap();
    assert_eq!(pa, pb, "A and B project the same converged document");

    // And A keeps collaborating: a local edit projects and broadcasts.
    let edited = doc_of(&schema, vec![code_block(&schema, "hello!")]);
    a.record_local(&schema, &healed.doc, &edited).unwrap();
    assert!(
        !a.save_incremental().unwrap().is_empty(),
        "the healed session broadcasts again"
    );
}

#[test]
fn poison_clears_when_inbound_bytes_make_the_document_projectable_again() {
    // Poison is sticky, not fatal: inbound integration keeps being *attempted*, and
    // an update that leaves the document rebuildable again — here the damaged-lineage
    // peer deleting its own embedded range — clears the poison. Outbound stays
    // refused for the whole poisoned window.
    let mut l = live(&["alpha", "beta"]);

    let peer_doc = Doc::new();
    {
        let update = Update::decode_v1(&l.session.snapshot()).unwrap();
        peer_doc.transact_mut().apply_update(update).unwrap();
    }
    // The peer parks an embed inside block 0's text (interior damage)...
    {
        let content = peer_doc.get_or_insert_array("content");
        let mut txn = peer_doc.transact_mut();
        let Some(yrs::Out::YMap(node)) = content.get(&txn, 0) else {
            panic!("block 0 must be a node map");
        };
        let Some(yrs::Out::YText(text)) = node.get(&txn, "text") else {
            panic!("block 0 must carry a text");
        };
        text.insert_embed(&mut txn, 2, Any::Bool(true));
    }
    let damage = {
        let sv = StateVector::decode_v1(&l.session.state_vector()).unwrap();
        peer_doc.transact().encode_diff_v1(&sv)
    };
    let state = l.state.clone();
    let err = l
        .session
        .integrate_incremental(&state, &damage)
        .unwrap_err();
    assert!(
        matches!(err, CollabError::SessionPoisoned(_)),
        "got {err:?}"
    );
    assert!(l.session.is_poisoned());
    assert!(
        matches!(l.type_at(8, "L"), Err(CollabError::SessionPoisoned(_))),
        "outbound is refused while poisoned"
    );

    // ...then deletes the embedded range and sends that delta.
    {
        let content = peer_doc.get_or_insert_array("content");
        let mut txn = peer_doc.transact_mut();
        let Some(yrs::Out::YMap(node)) = content.get(&txn, 0) else {
            panic!("block 0 must be a node map");
        };
        let Some(yrs::Out::YText(text)) = node.get(&txn, "text") else {
            panic!("block 0 must carry a text");
        };
        text.remove_range(&mut txn, 2, 1); // the embed occupies one unit
    }
    let heal = {
        let sv = StateVector::decode_v1(&l.session.state_vector()).unwrap();
        peer_doc.transact().encode_diff_v1(&sv)
    };
    let state = l.state.clone();
    let healed = l.session.integrate_incremental(&state, &heal).unwrap();
    assert!(
        !l.session.is_poisoned(),
        "a successful converged rebuild clears the poison"
    );
    // The healed CRDT matches the model A already held, so integrating may report
    // "no document change" — what matters is that the session works again.
    let _ = healed;
    assert!(l.type_at(8, "L").is_ok(), "outbound projects again");
    assert!(
        !l.session.save_incremental().unwrap().is_empty(),
        "and broadcasts again"
    );
    let pd = l.session.projected_doc(&l.schema).unwrap();
    assert_eq!(pd, l.state.doc, "model ≡ project(model) is restored");
}

#[test]
fn merge_refuses_to_touch_or_spread_poison() {
    // PR #219 review finding F2: pin `merge`'s refusal surface in BOTH directions —
    // a poisoned session must not merge, and a healthy session must not pull a
    // poisoned peer's document in.
    let mut poisoned = live(&["hello"]);
    let bytes = foreign_update(|d| {
        let t = d.get_or_insert_text("content");
        t.insert(&mut d.transact_mut(), 0, "foreign");
    });
    let state = poisoned.state.clone();
    let _ = poisoned
        .session
        .integrate_incremental(&state, &bytes)
        .unwrap_err();
    assert!(poisoned.session.is_poisoned());

    let mut healthy = live(&["clean"]);
    assert!(
        matches!(
            poisoned.session.merge(&healthy.session),
            Err(CollabError::SessionPoisoned(_))
        ),
        "a poisoned session refuses to merge (its own guard)"
    );
    assert!(
        matches!(
            healthy.session.merge(&poisoned.session),
            Err(CollabError::SessionPoisoned(_))
        ),
        "a healthy session refuses to merge FROM a poisoned peer (the other-side guard)"
    );
    assert!(
        !healthy.session.is_poisoned(),
        "the refusal protected the healthy session"
    );
    assert!(
        healthy.type_at(1, "X").is_ok() && !healthy.session.save_incremental().unwrap().is_empty(),
        "the healthy session keeps collaborating"
    );
}

#[test]
fn recovery_is_a_fresh_session_from_a_healthy_snapshot() {
    // The documented recovery path: drop the poisoned session, rejoin from a healthy
    // peer's snapshot, collaborate on.
    let mut l = live(&["hello"]);
    let healthy_snapshot = l.session.snapshot();
    let bytes = foreign_update(|d| {
        let t = d.get_or_insert_text("content");
        t.insert(&mut d.transact_mut(), 0, "foreign");
    });
    let state = l.state.clone();
    let _ = l.session.integrate_incremental(&state, &bytes).unwrap_err();
    assert!(l.session.is_poisoned());

    let mut fresh = CollabSession::from_bytes(&healthy_snapshot).unwrap();
    assert!(!fresh.is_poisoned());
    let doc = fresh.projected_doc(&l.schema).unwrap();
    let mut state = EditorState::create(l.schema.clone(), doc, default_plugins());
    let mut tr = state.tr();
    tr.set_selection(Selection::cursor(Pos(1)));
    tr.insert_text("R").unwrap();
    let before = state.doc.clone();
    state = state.apply(tr);
    fresh.record_local(&l.schema, &before, &state.doc).unwrap();
    assert!(
        !fresh.save_incremental().unwrap().is_empty(),
        "the fresh session collaborates normally"
    );
}
