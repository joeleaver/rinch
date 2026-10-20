//! The starter kit's `embed` through collaboration.
//!
//! An embed is a **leaf block atom**, projected as `horizontal_rule` is: a block of
//! its own with empty text. What it adds is attributes — an app's `data-*` keys, the
//! state a node view keys its own data to — and those live in the block's `attrs`
//! map, one yrs map entry per key:
//!
//! - every key round-trips, through a snapshot and through a live update;
//! - two peers concurrently changing **different** keys of one embed both keep
//!   their change, under either client-id order; the **same** key converges on one
//!   value;
//! - a key removed by one peer while another changes a different key stays removed;
//! - a peer's update holding more keys than the model keeps is bounded on receive;
//! - a peer typing around the embed, and a peer deleting it, merge as for an hr.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, SetNodeAttrStep,
    Transaction, default_plugins,
};

struct Peer {
    state: EditorState,
    session: CollabSession,
}

/// The embed sits at model position 4: `doc`, `paragraph("ab")` (size 4), then it.
const EMBED_AT: usize = 4;

impl Peer {
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

    fn set(&mut self, key: &str, value: AttrValue) {
        self.local(|tr| {
            tr.step(Box::new(SetNodeAttrStep::new(EMBED_AT, key, value)))
                .unwrap();
        });
    }

    /// The one embed's data attributes.
    fn embed(&self) -> Vec<(String, String)> {
        let embeds: Vec<&Node> = (0..self.state.doc.child_count())
            .map(|i| self.state.doc.child(i))
            .filter(|n| n.type_name() == "embed")
            .collect();
        assert_eq!(embeds.len(), 1, "exactly one embed: {:?}", self.state.doc);
        embeds[0]
            .attrs()
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("").to_string()))
            .collect()
    }
}

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn para(schema: &Schema, text: &str) -> Node {
    schema
        .branch("paragraph", Fragment::from_node(schema.text(text).unwrap()))
        .unwrap()
}

/// `ab` / an embed with `attrs` / `cd`.
fn three(schema: &Schema, attrs: &[(&str, &str)]) -> Node {
    let embed = schema
        .create_node(
            "embed",
            Attrs::from_iter(attrs.iter().map(|(k, v)| (*k, AttrValue::from(*v)))),
            Fragment::empty(),
        )
        .unwrap();
    schema
        .branch(
            "doc",
            Fragment::from_children(vec![para(schema, "ab"), embed, para(schema, "cd")]),
        )
        .unwrap()
}

fn two_peers(schema: &Rc<Schema>, doc: Node, ids: (u64, u64)) -> (Peer, Peer) {
    let a_state = EditorState::create(schema.clone(), doc, plugins());
    let session_a = session_with_client_id(&a_state, ids.0).expect("session A");
    let session_b =
        session_from_bytes_with_client_id(&session_a.snapshot(), ids.1).expect("session B");
    let b_doc = session_b.projected_doc(schema).expect("project B");
    (
        Peer {
            state: a_state,
            session: session_a,
        },
        Peer {
            state: EditorState::create(schema.clone(), b_doc, plugins()),
            session: session_b,
        },
    )
}

fn sync(a: &mut Peer, b: &mut Peer) {
    for _ in 0..8 {
        let to_b = a.session.sync_diff(&b.session.state_vector()).unwrap();
        let to_a = b.session.sync_diff(&a.session.state_vector()).unwrap();
        let mut changed = false;
        if let Some(ns) = b.session.integrate_incremental(&b.state, &to_b).unwrap() {
            b.state = ns;
            changed = true;
        }
        if let Some(ns) = a.session.integrate_incremental(&a.state, &to_a).unwrap() {
            a.state = ns;
            changed = true;
        }
        if !changed {
            return;
        }
    }
    panic!("the state-vector exchange did not settle");
}

/// Both peers hold one document, which is what each one's CRDT reads back as.
fn converged(a: &Peer, b: &Peer, schema: &Schema) {
    for peer in [a, b] {
        assert_eq!(
            peer.state.doc,
            peer.session.projected_doc(schema).unwrap(),
            "a peer's document is not what its CRDT holds"
        );
    }
    assert_eq!(
        a.state.doc, b.state.doc,
        "the peers hold different documents"
    );
}

fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

const ID_ORDERS: [(u64, u64); 2] = [(11, 22), (22, 11)];

#[test]
fn an_embeds_data_attrs_round_trip_through_a_snapshot_and_a_live_update() {
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(
        &schema,
        three(&schema, &[("data-sheet", "s1"), ("data-view", "grid")]),
        (11, 22),
    );
    assert_eq!(
        b.embed(),
        kv(&[("data-sheet", "s1"), ("data-view", "grid")])
    );

    a.set("data-view", AttrValue::from("chart"));
    sync(&mut a, &mut b);
    b.set("data-zoom", AttrValue::from("2"));
    sync(&mut a, &mut b);
    a.set("data-sheet", AttrValue::Null);
    sync(&mut a, &mut b);
    converged(&a, &b, &schema);
    assert_eq!(a.embed(), kv(&[("data-view", "chart"), ("data-zoom", "2")]));

    // A peer joining now reads the same embed.
    let joiner = CollabSession::from_bytes(&a.session.snapshot()).unwrap();
    assert_eq!(joiner.projected_doc(&schema).unwrap(), a.state.doc);
}

#[test]
fn concurrent_changes_of_different_keys_of_one_embed_both_survive() {
    let schema = Rc::new(Schema::starter_kit());
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers(
            &schema,
            three(&schema, &[("data-sheet", "s1"), ("data-view", "grid")]),
            ids,
        );
        a.set("data-view", AttrValue::from("chart"));
        b.set("data-zoom", AttrValue::from("2"));
        sync(&mut a, &mut b);
        converged(&a, &b, &schema);
        assert_eq!(
            a.embed(),
            kv(&[
                ("data-sheet", "s1"),
                ("data-view", "chart"),
                ("data-zoom", "2")
            ]),
            "ids {ids:?}"
        );
    }
}

#[test]
fn a_key_removed_while_a_peer_changes_another_stays_removed() {
    let schema = Rc::new(Schema::starter_kit());
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers(
            &schema,
            three(&schema, &[("data-sheet", "s1"), ("data-view", "grid")]),
            ids,
        );
        a.set("data-sheet", AttrValue::Null);
        b.set("data-view", AttrValue::from("chart"));
        sync(&mut a, &mut b);
        converged(&a, &b, &schema);
        assert_eq!(a.embed(), kv(&[("data-view", "chart")]), "ids {ids:?}");
    }
}

#[test]
fn concurrent_changes_of_the_same_key_converge_on_one_value() {
    let schema = Rc::new(Schema::starter_kit());
    let mut winners = Vec::new();
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers(&schema, three(&schema, &[("data-view", "grid")]), ids);
        a.set("data-view", AttrValue::from("chart"));
        b.set("data-view", AttrValue::from("table"));
        sync(&mut a, &mut b);
        converged(&a, &b, &schema);
        let got = a.embed();
        assert!(
            got == kv(&[("data-view", "chart")]) || got == kv(&[("data-view", "table")]),
            "{got:?}"
        );
        winners.push(got);
    }
    // yrs breaks the tie by client id, so swapping the ids swaps the winner.
    assert_ne!(winners[0], winners[1]);
}

/// Typing in the paragraphs around the embed, and changing its attrs, merge.
#[test]
fn typing_around_an_embed_merges_with_a_change_of_its_attrs() {
    let schema = Rc::new(Schema::starter_kit());
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers(&schema, three(&schema, &[("data-view", "grid")]), ids);
        a.set("data-view", AttrValue::from("chart"));
        b.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(3)));
            tr.insert_text("X").unwrap();
        });
        b.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(7)));
            tr.insert_text("Y").unwrap();
        });
        sync(&mut a, &mut b);
        converged(&a, &b, &schema);
        assert_eq!(a.embed(), kv(&[("data-view", "chart")]));
        assert_eq!(a.state.doc.child(0).child(0).text(), Some("abX"));
        assert_eq!(a.state.doc.child(2).child(0).text(), Some("Ycd"));
    }
}

/// A peer deleting the embed while another changes it: the deletion wins (there
/// is nothing left to change), and both peers agree.
#[test]
fn a_deleted_embed_stays_deleted_under_a_concurrent_attr_change() {
    let schema = Rc::new(Schema::starter_kit());
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers(&schema, three(&schema, &[("data-view", "grid")]), ids);
        a.set("data-view", AttrValue::from("chart"));
        b.local(|tr| {
            tr.delete(EMBED_AT, EMBED_AT + 1).unwrap();
        });
        sync(&mut a, &mut b);
        converged(&a, &b, &schema);
        assert!(
            (0..a.state.doc.child_count()).all(|i| a.state.doc.child(i).type_name() != "embed"),
            "{:?}",
            a.state.doc
        );
    }
}

/// A peer whose schema keeps no data attributes on its `embed` writes as many as
/// it likes; the receiver keeps what its model keeps.
#[test]
fn a_peers_data_attrs_past_the_bounds_are_bounded_on_receive() {
    use rinch_editor_core::schema::{MAX_DATA_ATTR_VALUE, MAX_DATA_ATTRS};
    let kit = Rc::new(Schema::starter_kit());
    // The sender's schema: the starter kit, but its embed declares 41 attrs of its
    // own, which the model then keeps however many there are.
    let mut b = Schema::builder();
    for (name, spec) in kit.nodes.iter() {
        let mut spec = spec.clone();
        if name == "embed" {
            spec.data_attrs = false;
            for i in 0..40 {
                spec.attrs.insert(
                    format!("data-k{i:02}"),
                    rinch_editor_core::schema::AttrSpec::optional(""),
                );
            }
            spec.attrs.insert(
                "data-a-big".into(),
                rinch_editor_core::schema::AttrSpec::optional(""),
            );
        }
        b = b.node(name, spec);
    }
    for (name, spec) in kit.marks.iter() {
        b = b.mark(name, spec.clone());
    }
    let wide = Rc::new(b.build());
    let a_state = EditorState::create(wide.clone(), three(&wide, &[]), plugins());
    let session_a = session_with_client_id(&a_state, 11).unwrap();
    let session_b = session_from_bytes_with_client_id(&session_a.snapshot(), 22).unwrap();
    let b_doc = session_b.projected_doc(&kit).unwrap();
    let mut a = Peer {
        state: a_state,
        session: session_a,
    };
    let mut b = Peer {
        state: EditorState::create(kit.clone(), b_doc, plugins()),
        session: session_b,
    };
    let big = "x".repeat(MAX_DATA_ATTR_VALUE * 4);
    a.local(|tr| {
        for i in 0..40 {
            tr.step(Box::new(SetNodeAttrStep::new(
                EMBED_AT,
                format!("data-k{i:02}"),
                AttrValue::from("v"),
            )))
            .unwrap();
        }
        tr.step(Box::new(SetNodeAttrStep::new(
            EMBED_AT,
            "data-a-big",
            AttrValue::from(big.as_str()),
        )))
        .unwrap();
    });
    let sent = a.state.doc.child(1).clone();
    assert_eq!(
        sent.attrs()
            .iter()
            .filter(|(k, v)| k.starts_with("data-") && v.as_str() != Some(""))
            .count(),
        41,
        "the sender kept every one: {sent:?}"
    );
    let delta = a.session.save_incremental().unwrap();
    b.state = b
        .session
        .integrate_incremental(&b.state, &delta)
        .unwrap()
        .expect("a change");
    let got = b.state.doc.child(1).clone();
    assert_eq!(got.type_name(), "embed");
    let names: Vec<&str> = got.attrs().iter().map(|(k, _)| k).collect();
    assert_eq!(names.len(), MAX_DATA_ATTRS, "{names:?}");
    assert_eq!(got.attrs().get("data-a-big"), None);
    assert!(got.attrs().get("data-k31").is_some());
    assert_eq!(got.attrs().get("data-k32"), None);
    assert_eq!(b.state.doc, b.session.projected_doc(&kit).unwrap());
}

/// An older peer — one whose schema has no `embed` — cannot read one: its join
/// from a snapshot holding one fails, and a live update inserting one is an error
/// it surfaces rather than a silent drop. (Why every peer must be upgraded before
/// any of them inserts an embed.)
#[test]
fn a_peer_without_embed_in_its_schema_fails_loud_on_one() {
    let kit = Rc::new(Schema::starter_kit());
    let mut b = Schema::builder();
    for (name, spec) in kit.nodes.iter().filter(|(n, _)| n.as_str() != "embed") {
        b = b.node(name, spec.clone());
    }
    for (name, spec) in kit.marks.iter() {
        b = b.mark(name, spec.clone());
    }
    let old = Rc::new(b.build());
    assert!(!old.has_node("embed"));

    // A join from a snapshot that holds an embed fails.
    let with = EditorState::create(kit.clone(), three(&kit, &[("data-k", "v")]), plugins());
    let host = session_with_client_id(&with, 11).unwrap();
    let joiner = session_from_bytes_with_client_id(&host.snapshot(), 22).unwrap();
    assert!(joiner.projected_doc(&old).is_err());

    // A live peer that integrates an inserted embed reports an error.
    let plain = kit
        .branch(
            "doc",
            Fragment::from_children(vec![para(&kit, "ab"), para(&kit, "cd")]),
        )
        .unwrap();
    let mut new_peer = Peer {
        state: EditorState::create(kit.clone(), plain.clone(), plugins()),
        session: session_with_client_id(
            &EditorState::create(kit.clone(), plain.clone(), plugins()),
            11,
        )
        .unwrap(),
    };
    let snapshot = new_peer.session.snapshot();
    let mut old_session = session_from_bytes_with_client_id(&snapshot, 22).unwrap();
    let old_state = EditorState::create(
        old.clone(),
        old_session.projected_doc(&old).unwrap(),
        plugins(),
    );
    let embed = kit
        .create_node("embed", Attrs::new(), Fragment::empty())
        .unwrap();
    new_peer.local(|tr| {
        tr.replace(
            4,
            4,
            rinch_editor_core::Slice::new(Fragment::from_node(embed), 0, 0),
        )
        .unwrap();
    });
    let delta = new_peer.session.save_incremental().unwrap();
    assert!(
        old_session
            .integrate_incremental(&old_state, &delta)
            .is_err()
    );
}
