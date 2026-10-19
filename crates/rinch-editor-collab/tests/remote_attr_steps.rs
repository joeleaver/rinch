//! A peer's change to the **attributes** of a leaf atom arrives as
//! `SetNodeAttrStep`s, not as a replace of its block (node views, PR B).
//!
//! An editor that draws an app component inside a document (a node view) keeps
//! that component mounted only while its node is provably the same node: the same
//! `Rc`, or one a `SetNodeAttrStep` in the transaction changed. A remote change
//! used to arrive as one block-level `ReplaceStep` over the changed top-level
//! blocks — a new node, which the view must take for a different one and remount,
//! losing the component's own state for every keystroke a collaborator typed into
//! one of its attributes. Now a changed run whose blocks differ from the old ones
//! only in the attributes of leaf atoms (at any depth) becomes one
//! `SetNodeAttrStep` per changed key; anything else is still the block replace.

use std::rc::Rc;

use rinch_editor_collab::{RemoteCarry, build_remote_transaction};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, NodeSelection, Pos, Schema, Selection,
    SetNodeAttrStep, Step, default_plugins,
};

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

fn state(schema: &Rc<Schema>, d: Node, selection: Selection) -> EditorState {
    let mut st = EditorState::create(schema.clone(), d, default_plugins());
    st.selection = selection;
    st
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
fn an_attr_change_of_a_top_level_embed_is_set_node_attr_steps() {
    let schema = Rc::new(Schema::starter_kit());
    let old = doc(
        &schema,
        vec![
            para(&schema, "ab"),
            embed(&schema, &[("data-sheet", "s1"), ("data-zoom", "1")]),
            para(&schema, "cd"),
        ],
    );
    let target = doc(
        &schema,
        vec![
            para(&schema, "ab"),
            embed(&schema, &[("data-sheet", "s2"), ("data-mode", "edit")]),
            para(&schema, "cd"),
        ],
    );
    let sel = Selection::Node(NodeSelection {
        anchor: Pos(4),
        head: Pos(5),
    });
    let st = state(&schema, old, sel.clone());
    let tr = build_remote_transaction(&st, &target).unwrap().unwrap();
    let mut steps = attr_steps(tr.steps()).expect("only attr steps");
    steps.sort();
    assert_eq!(
        steps,
        vec![
            (4, "data-mode".into(), Some("edit".into())),
            (4, "data-sheet".into(), Some("s2".into())),
            (4, "data-zoom".into(), None),
        ]
    );
    let next = st.apply(tr);
    assert_eq!(next.doc, target, "the same document as the converged one");
    assert_eq!(next.selection, sel, "the node selection stays on it");
}

#[test]
fn an_attr_change_of_a_nested_embed_is_an_attr_step_too() {
    let schema = Rc::new(Schema::starter_kit());
    let quote = |e: Node| {
        schema
            .branch(
                "blockquote",
                Fragment::from_children(vec![para(&schema, "q"), e]),
            )
            .unwrap()
    };
    let old = doc(
        &schema,
        vec![
            para(&schema, "ab"),
            quote(embed(&schema, &[("data-k", "1")])),
        ],
    );
    let target = doc(
        &schema,
        vec![
            para(&schema, "ab"),
            quote(embed(&schema, &[("data-k", "2")])),
        ],
    );
    let st = state(&schema, old, Selection::cursor(Pos(2)));
    let tr = build_remote_transaction(&st, &target).unwrap().unwrap();
    // blockquote opens at 4, its paragraph spans 5..8, the embed starts at 8.
    assert_eq!(
        attr_steps(tr.steps()),
        Some(vec![(8, "data-k".into(), Some("2".into()))])
    );
    assert_eq!(st.apply(tr).doc, target);
}

/// A change that is more than attributes is still the block replace.
#[test]
fn a_text_change_beside_the_attr_change_is_still_a_replace() {
    let schema = Rc::new(Schema::starter_kit());
    let old = doc(
        &schema,
        vec![para(&schema, "ab"), embed(&schema, &[("data-k", "1")])],
    );
    let target = doc(
        &schema,
        vec![para(&schema, "abc"), embed(&schema, &[("data-k", "2")])],
    );
    let st = state(&schema, old, Selection::cursor(Pos(2)));
    let tr = build_remote_transaction(&st, &target).unwrap().unwrap();
    assert!(attr_steps(tr.steps()).is_none(), "{:?}", tr.steps());
    assert_eq!(st.apply(tr).doc, target);
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
    let st = state(&schema, old, Selection::cursor(Pos(2)));
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
