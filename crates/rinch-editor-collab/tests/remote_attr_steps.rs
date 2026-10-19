//! A peer's change to the **attributes** of a leaf block atom (an `embed`) arrives
//! as `SetNodeAttrStep`s, not as a replace of its block (node views, PR B).
//!
//! An editor that draws an app component inside a document (a node view) keeps
//! that component mounted only while its node is provably the same node: the same
//! `Rc`, or one a `SetNodeAttrStep` in the transaction changed. A remote change
//! used to arrive as one block-level `ReplaceStep` over the changed top-level
//! blocks — a new node, which the view must take for a different one and remount,
//! losing the component's own state on every change a collaborator made to one
//! of its attributes.
//!
//! The attr steps need **identity**, not only shape: two embeds a peer swapped
//! look, position by position, exactly like two embeds whose attributes were
//! exchanged, and a view must never be handed the other node. So the steps are
//! emitted only when every atom whose attributes differ is the same CRDT block
//! on both sides; a swap stays a replace.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession, RemoteCarry};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, NodeSelection, Plugin, Pos, Schema, Selection,
    SetNodeAttrStep, Slice, Step, Transaction, default_plugins,
};

struct Peer {
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    /// A local edit, recorded as an editor records it: the leaf atoms at `kept`
    /// (positions in the new document) are the ones the edit changed the
    /// attributes of — the same atoms — and any other atom it put where another
    /// node stood is a different one.
    fn local(&mut self, kept: &[usize], f: impl FnOnce(&mut Transaction)) {
        let mut tr = self.state.tr();
        f(&mut tr);
        let before = self.state.doc.clone();
        let after = self.state.apply(tr);
        let kept: Vec<Node> = kept
            .iter()
            .map(|&p| after.doc.node_at(p).expect("a node at a kept position"))
            .collect();
        self.session
            .record_local_keeping(self.state.schema(), &before, &after.doc, &kept)
            .expect("project local");
        self.state = after;
    }

    /// Integrate `from`'s changes, answering the steps that did it.
    fn receive(&mut self, from: &Peer) -> Vec<Box<dyn Step>> {
        let delta = from
            .session
            .sync_diff(&self.session.state_vector())
            .unwrap();
        let tr = self
            .session
            .integrate_incremental_transaction(&self.state, &delta)
            .unwrap()
            .expect("a change");
        let steps = tr.steps().to_vec();
        self.state = self.state.apply(tr);
        assert_eq!(
            self.state.doc, from.state.doc,
            "the receiver holds the sender's document"
        );
        steps
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

fn embed(schema: &Schema, attrs: &[(&str, &str)]) -> Node {
    schema
        .create_node(
            "embed",
            Attrs::from_iter(attrs.iter().map(|(k, v)| (*k, AttrValue::from(*v)))),
            Fragment::empty(),
        )
        .unwrap()
}

fn doc(schema: &Schema, blocks: Vec<Node>) -> Node {
    schema
        .branch("doc", Fragment::from_children(blocks))
        .unwrap()
}

fn two_peers(schema: &Rc<Schema>, d: Node) -> (Peer, Peer) {
    let a_state = EditorState::create(schema.clone(), d, plugins());
    let session_a = session_with_client_id(&a_state, 1).unwrap();
    let session_b = session_from_bytes_with_client_id(&session_a.snapshot(), 2).unwrap();
    let b_doc = session_b.projected_doc(schema).unwrap();
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

/// The transaction's steps as `(pos, attr, value)` when every one is a
/// `SetNodeAttrStep`, else `None`.
fn attr_steps(steps: &[Box<dyn Step>]) -> Option<Vec<(usize, String, Option<String>)>> {
    steps
        .iter()
        .map(|s| {
            s.as_any().downcast_ref::<SetNodeAttrStep>().map(|a| {
                (
                    a.pos,
                    a.attr.clone(),
                    a.value
                        .as_ref()
                        .map(|v| v.as_str().unwrap_or("").to_string()),
                )
            })
        })
        .collect()
}

#[test]
fn a_peers_attr_change_of_a_top_level_embed_is_set_node_attr_steps() {
    let schema = Rc::new(Schema::starter_kit());
    let d = doc(
        &schema,
        vec![
            para(&schema, "ab"),
            embed(&schema, &[("data-sheet", "s1"), ("data-zoom", "1")]),
            para(&schema, "cd"),
        ],
    );
    let (mut a, mut b) = two_peers(&schema, d);
    let sel = Selection::Node(NodeSelection {
        anchor: Pos(4),
        head: Pos(5),
    });
    b.state.selection = sel.clone();
    a.local(&[4], |tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            4,
            "data-sheet",
            AttrValue::from("s2"),
        )))
        .unwrap();
        tr.step(Box::new(SetNodeAttrStep {
            pos: 4,
            attr: "data-zoom".into(),
            value: None,
        }))
        .unwrap();
        tr.step(Box::new(SetNodeAttrStep::new(
            4,
            "data-mode",
            AttrValue::from("edit"),
        )))
        .unwrap();
    });
    let steps = b.receive(&a);
    let mut steps = attr_steps(&steps).expect("only attr steps");
    steps.sort();
    assert_eq!(
        steps,
        vec![
            (4, "data-mode".into(), Some("edit".into())),
            (4, "data-sheet".into(), Some("s2".into())),
            (4, "data-zoom".into(), None),
        ]
    );
    assert_eq!(b.state.selection, sel, "the node selection stays on it");
}

#[test]
fn a_peers_attr_change_of_a_nested_embed_is_an_attr_step_too() {
    let schema = Rc::new(Schema::starter_kit());
    let quote = schema
        .branch(
            "blockquote",
            Fragment::from_children(vec![para(&schema, "q"), embed(&schema, &[("data-k", "1")])]),
        )
        .unwrap();
    let (mut a, mut b) = two_peers(&schema, doc(&schema, vec![para(&schema, "ab"), quote]));
    // blockquote opens at 4, its paragraph spans 5..8, the embed starts at 8.
    a.local(&[8], |tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            8,
            "data-k",
            AttrValue::from("2"),
        )))
        .unwrap();
    });
    let steps = b.receive(&a);
    assert_eq!(
        attr_steps(&steps),
        Some(vec![(8, "data-k".into(), Some("2".into()))])
    );
}

/// Two embeds a peer swapped are not two embeds whose attributes changed: each
/// stands where the other stood, and a node view keyed to one must not be
/// handed the other. The sender writes the moved atom as a new CRDT block
/// (`record_local_keeping` was told it is not the same atom), so the receiver
/// sees another block and the change arrives as a replace.
#[test]
fn a_peer_swapping_two_embeds_is_a_replace_not_attr_steps() {
    let schema = Rc::new(Schema::starter_kit());
    let d = doc(
        &schema,
        vec![
            para(&schema, "ab"),
            embed(&schema, &[("data-k", "1")]),
            embed(&schema, &[("data-k", "2")]),
            para(&schema, "cd"),
        ],
    );
    let (mut a, mut b) = two_peers(&schema, d);
    // Move the first embed after the second: delete it and insert a copy.
    a.local(&[], |tr| {
        let first = tr.doc().child(1).clone();
        tr.delete(4, 5).unwrap();
        tr.replace(5, 5, Slice::new(Fragment::from_node(first), 0, 0))
            .unwrap();
    });
    assert_eq!(a.state.doc.child(1).attrs().get_str("data-k"), Some("2"));
    let steps = b.receive(&a);
    assert!(attr_steps(&steps).is_none(), "{steps:?}");
}

/// A change that is more than attributes (a reconnect's batch: text typed in a
/// paragraph and the embed's attributes changed) replaces the paragraph and sets
/// the embed's attributes, so the embed keeps its identity and a node view stays
/// mounted (review of #1575, F3: it was one replace of both).
#[test]
fn a_text_change_beside_the_attr_change_replaces_the_text_and_sets_the_attrs() {
    let schema = Rc::new(Schema::starter_kit());
    let d = doc(
        &schema,
        vec![para(&schema, "ab"), embed(&schema, &[("data-k", "1")])],
    );
    let (mut a, mut b) = two_peers(&schema, d);
    let c = schema.text("c").unwrap();
    a.local(&[5], |tr| {
        tr.replace(3, 3, Slice::new(Fragment::from_node(c), 0, 0))
            .unwrap();
        tr.step(Box::new(SetNodeAttrStep::new(
            5,
            "data-k",
            AttrValue::from("2"),
        )))
        .unwrap();
    });
    let steps = b.receive(&a);
    let attrs: Vec<_> = steps
        .iter()
        .filter_map(|s| attr_steps(std::slice::from_ref(s)))
        .flatten()
        .collect();
    assert_eq!(
        attrs,
        [(4, "data-k".to_string(), Some("2".to_string()))],
        "{steps:?}"
    );
    assert_eq!(steps.len(), 2, "one replace of the paragraph: {steps:?}");
}

/// A selection captured elsewhere stands through an attr-only change.
#[test]
fn an_anchor_survives_an_attr_only_change() {
    let schema = Rc::new(Schema::starter_kit());
    let old = doc(
        &schema,
        vec![para(&schema, "ab"), embed(&schema, &[("data-k", "1")])],
    );
    let target = doc(
        &schema,
        vec![para(&schema, "ab"), embed(&schema, &[("data-k", "2")])],
    );
    let st = EditorState::create(schema.clone(), old, default_plugins());
    let carry = RemoteCarry::new(&st, &target).expect("a change");
    let node = Selection::Node(NodeSelection {
        anchor: Pos(4),
        head: Pos(5),
    });
    assert_eq!(carry.selection(&node), Some(node.clone()));
    assert_eq!(
        carry.selection(&Selection::cursor(Pos(1))),
        Some(Selection::cursor(Pos(1)))
    );
}

/// Review of #1575, F1: two **equal** embeds a peer swapped. The converged
/// document equals the receiver's by value, so a diff by value saw no change and
/// left the receiver's atoms — and the node views drawn for them — on CRDT blocks
/// that no longer exist. The atoms are other blocks now, so they are replaced.
#[test]
fn a_peer_swapping_two_equal_embeds_is_a_change() {
    let schema = Rc::new(Schema::starter_kit());
    let d = doc(
        &schema,
        vec![
            para(&schema, "ab"),
            embed(&schema, &[("data-k", "1")]),
            embed(&schema, &[("data-k", "1")]),
            para(&schema, "cd"),
        ],
    );
    let (mut a, mut b) = two_peers(&schema, d);
    let old: Vec<Node> = (1..3).map(|i| b.state.doc.child(i).clone()).collect();
    a.local(&[], |tr| {
        let first = tr.doc().child(1).clone();
        tr.delete(4, 5).unwrap();
        tr.replace(5, 5, Slice::new(Fragment::from_node(first), 0, 0))
            .unwrap();
    });
    let steps = b.receive(&a);
    assert!(!steps.is_empty());
    for (i, o) in old.iter().enumerate() {
        assert!(
            !b.state.doc.child(i + 1).same_ref(o),
            "embed {i} stands on a deleted block"
        );
    }
    // The paragraphs around them are untouched.
    assert_eq!(b.state.doc.child(0), a.state.doc.child(0));
}

/// Review of #1575, F3 (R4): an embed with no attributes yet, given its first by a
/// peer, is the same atom with an attribute set — not a replace.
#[test]
fn a_peer_giving_an_attrless_embed_its_first_attr_is_an_attr_step() {
    let schema = Rc::new(Schema::starter_kit());
    let d = doc(
        &schema,
        vec![para(&schema, "ab"), embed(&schema, &[]), para(&schema, "cd")],
    );
    let (mut a, mut b) = two_peers(&schema, d);
    a.local(&[4], |tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            4,
            "data-k",
            AttrValue::from("1"),
        )))
        .unwrap();
    });
    let steps = b.receive(&a);
    assert_eq!(
        attr_steps(&steps),
        Some(vec![(4, "data-k".into(), Some("1".into()))])
    );
}

/// Review of #1575, F3 (R5): a document holding a table. The atoms' CRDT blocks are
/// noted in the order the model is built, so a table (whose read places cells by
/// grid slot and adds fillers the CRDT does not hold) no longer hides them.
#[test]
fn an_attr_change_in_a_document_with_a_table_is_an_attr_step() {
    let schema = Rc::new(Schema::starter_kit());
    let cell = |t: &str| {
        schema
            .branch("table_cell", Fragment::from_node(para(&schema, t)))
            .unwrap()
    };
    let row = schema
        .branch("table_row", Fragment::from_children(vec![cell("a"), cell("b")]))
        .unwrap();
    let table = schema
        .branch("table", Fragment::from_node(row))
        .unwrap();
    let d = doc(
        &schema,
        vec![
            table,
            para(&schema, "ab"),
            embed(&schema, &[("data-k", "1")]),
        ],
    );
    let (mut a, mut b) = two_peers(&schema, d);
    let at = a.state.doc.child(0).node_size() + 4;
    a.local(&[at], |tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            at,
            "data-k",
            AttrValue::from("2"),
        )))
        .unwrap();
    });
    let steps = b.receive(&a);
    assert_eq!(
        attr_steps(&steps),
        Some(vec![(at, "data-k".into(), Some("2".into()))])
    );
}

/// After the receiver's own edits (its atoms' blocks then come from `atom_cache`,
/// carried across the edits rather than read), a peer's swap of two equal embeds
/// is still a change, and a peer's attr change still an attr step.
#[test]
fn identity_holds_across_the_receivers_own_edits() {
    let schema = Rc::new(Schema::starter_kit());
    let d = doc(
        &schema,
        vec![
            para(&schema, "ab"),
            embed(&schema, &[("data-k", "1")]),
            embed(&schema, &[("data-k", "1")]),
            para(&schema, "cd"),
            embed(&schema, &[("data-k", "3")]),
        ],
    );
    let (mut a, mut b) = two_peers(&schema, d);
    // B types before and after the embeds, and deletes the last one.
    b.local(&[], |tr| {
        let t = schema.text("x").unwrap();
        tr.replace(1, 1, Slice::new(Fragment::from_node(t), 0, 0)).unwrap();
    });
    b.local(&[], |tr| {
        let t = schema.text("y").unwrap();
        tr.replace(8, 8, Slice::new(Fragment::from_node(t), 0, 0)).unwrap();
    });
    let end = b.state.doc.content_size();
    b.local(&[], |tr| {
        tr.delete(end - 1, end).unwrap();
    });
    a.receive(&b);
    a.local(&[], |tr| {
        let first = tr.doc().child(1).clone();
        tr.delete(5, 6).unwrap();
        tr.replace(6, 6, Slice::new(Fragment::from_node(first), 0, 0))
            .unwrap();
    });
    let old: Vec<Node> = (1..3).map(|i| b.state.doc.child(i).clone()).collect();
    b.receive(&a);
    for (i, o) in old.iter().enumerate() {
        assert!(!b.state.doc.child(i + 1).same_ref(o), "embed {i}");
    }
    a.local(&[6], |tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            6,
            "data-k",
            AttrValue::from("9"),
        )))
        .unwrap();
    });
    let steps = b.receive(&a);
    assert_eq!(
        attr_steps(&steps),
        Some(vec![(6, "data-k".into(), Some("9".into()))])
    );
}
