//! A document of one textblock, edited by a caller that names only the kept atoms'
//! new nodes (`CollabSession::record_local_keeping`, review 2 of #1595, G4).
//!
//! The edit rebuilds the only top-level block, so before and after share no block.
//! Matched by value, a picture replaced by an equal fresh copy reads as no change,
//! the old char keeps its identity, and a peer's concurrent attribute on the old
//! picture shows on the copy. What keeps the two related is the inline nodes the
//! edit left in place (`shares_any_inline_node` in `project.rs`: the text around the
//! picture). An editor's own transactions say so outright (`record_local_edit`), so
//! the view fixtures (`rinch-editor-view/tests/review2_1595.rs`, q5d) cannot see this
//! rule; this file is its pin.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Plugin, Schema, Slice, default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn image(s: &Schema, r: &str) -> Node {
    s.create_node(
        "image",
        Attrs::from_iter([
            ("src", AttrValue::from("a.png")),
            ("data-ref", AttrValue::from(r)),
        ]),
        Fragment::empty(),
    )
    .unwrap()
}

fn images_with(doc: &Node, key: &str) -> usize {
    let mut n = 0;
    doc.nodes_between(0, doc.content_size(), &mut |node: &Node, _, _| {
        if node.type_name() == "image" && node.attrs().get(key).is_some() {
            n += 1;
        }
        true
    });
    n
}

#[test]
fn an_equal_fresh_picture_in_a_one_block_document_is_a_new_char() {
    let s = Rc::new(Schema::starter_kit());
    let para = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![
                s.text("ab").unwrap(),
                image(&s, "one"),
                s.text("cd").unwrap(),
            ]),
        )
        .unwrap();
    let d = s.branch("doc", Fragment::from_node(para)).unwrap();
    let mut a_state = EditorState::create(s.clone(), d, plugins());
    let mut a: CollabSession = session_with_client_id(&a_state, 1).unwrap();
    let mut b = session_from_bytes_with_client_id(&a.snapshot(), 2).unwrap();
    let mut b_state = EditorState::create(s.clone(), b.projected_doc(&s).unwrap(), plugins());

    // A: an equal fresh copy over the picture (a paste of itself), proving nothing kept.
    let mut tr = a_state.tr();
    tr.replace(
        3,
        4,
        Slice::new(Fragment::from_node(image(&s, "one")), 0, 0),
    )
    .unwrap();
    let next = a_state.apply(tr);
    a.record_local_keeping(&s, &a_state.doc, &next.doc, &[])
        .unwrap();
    a_state = next;

    // B, concurrently: an app attribute on the old picture.
    let mut tr = b_state.tr();
    tr.set_node_attr(3, "data-note", AttrValue::from("for-old"))
        .unwrap();
    let next = b_state.apply(tr);
    b.record_local(&s, &b_state.doc, &next.doc).unwrap();
    b_state = next;

    let to_b = a.sync_diff(&b.state_vector()).unwrap();
    let to_a = b.sync_diff(&a.state_vector()).unwrap();
    if let Some(n) = b.integrate_incremental(&b_state, &to_b).unwrap() {
        b_state = n;
    }
    if let Some(n) = a.integrate_incremental(&a_state, &to_a).unwrap() {
        a_state = n;
    }
    assert_eq!(a_state.doc, b_state.doc, "converged");
    assert_eq!(
        images_with(&a_state.doc, "data-note"),
        0,
        "B's note for the old picture shows on the copy pasted over it: {:?}",
        a_state.doc
    );
}
