//! Round-3 review of #1595: Enter at the start of a lone paragraph, recorded with
//! no proof (`record_local`), while a peer types at its end. Compiles on main too.
use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::serialize::html::node_to_html;
use rinch_editor_core::{EditorState, Fragment, Plugin, Schema, Slice, default_plugins};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

#[test]
fn r3_plain_enter_at_start_of_a_lone_paragraph() {
    let s = Rc::new(Schema::starter_kit());
    let para = s
        .branch("paragraph", Fragment::from_node(s.text("abc").unwrap()))
        .unwrap();
    let d = s.branch("doc", Fragment::from_node(para)).unwrap();
    let mut a_state = EditorState::create(s.clone(), d, plugins());
    let mut a: CollabSession = session_with_client_id(&a_state, 1).unwrap();
    let mut b = session_from_bytes_with_client_id(&a.snapshot(), 2).unwrap();
    let mut b_state = EditorState::create(s.clone(), b.projected_doc(&s).unwrap(), plugins());

    let mut tr = a_state.tr();
    tr.split(1, 1, None).unwrap();
    let next = a_state.apply(tr);
    a.record_local(&s, &a_state.doc, &next.doc).unwrap();
    a_state = next;

    let mut tr = b_state.tr();
    tr.replace(
        4,
        4,
        Slice::new(Fragment::from_node(s.text("X").unwrap()), 0, 0),
    )
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
    let (ha, hb) = (node_to_html(&a_state.doc), node_to_html(&b_state.doc));
    eprintln!("R3p {ha} | {hb}");
    assert_eq!(ha, hb);
    assert_eq!(ha, "<p></p><p>abcX</p>");
}

#[test]
fn r5_plain_sticky_through_enter_at_start_of_a_lone_paragraph() {
    let s = Rc::new(Schema::starter_kit());
    let para = s
        .branch("paragraph", Fragment::from_node(s.text("abc").unwrap()))
        .unwrap();
    let d = s.branch("doc", Fragment::from_node(para)).unwrap();
    let mut a_state = EditorState::create(s.clone(), d, plugins());
    let mut a: CollabSession = session_with_client_id(&a_state, 1).unwrap();
    let sticky = a
        .sticky_index(&a_state.doc, rinch_editor_core::Pos(3))
        .unwrap();
    let mut tr = a_state.tr();
    tr.split(1, 1, None).unwrap();
    let next = a_state.apply(tr);
    a.record_local(&s, &a_state.doc, &next.doc).unwrap();
    a_state = next;
    let now = a.resolve_sticky(&a_state.doc, &sticky);
    eprintln!("R5p {now:?}");
    assert_eq!(now, Some(rinch_editor_core::Pos(5)));
}

/// R1p (fix of round 3, F2): the session's own floor for a caller that hands
/// `record_local_edit` a pair whose old `Rc` the document held twice — one picture
/// node at two places (an in-editor copy). A deletes the first copy and changes an
/// attribute of the second in one transaction; B, concurrently, notes the first. A's
/// pair names no one place (two copies before, none left), so it proves nothing and
/// the renumbered picture is a new char: B's note goes with the deleted copy, never
/// onto it (`one_place_pairs`).
#[test]
fn r1p_a_pair_whose_old_node_stood_twice_proves_nothing_once_both_copies_went() {
    use rinch_editor_collab::testing::kept_atom_pairs;
    use rinch_editor_core::{AttrValue, Attrs};
    let s = Rc::new(Schema::starter_kit());
    let img = s
        .create_node(
            "image",
            Attrs::from_iter([
                ("src", AttrValue::from("a.png")),
                ("data-ref", AttrValue::from("one")),
            ]),
            Fragment::empty(),
        )
        .unwrap();
    let para = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![
                s.text("ab").unwrap(),
                img.clone(),
                img,
                s.text("ef").unwrap(),
            ]),
        )
        .unwrap();
    let d = s.branch("doc", Fragment::from_node(para)).unwrap();
    let mut a_state = EditorState::create(s.clone(), d, plugins());
    let mut a: CollabSession = session_with_client_id(&a_state, 1).unwrap();
    let mut b = session_from_bytes_with_client_id(&a.snapshot(), 2).unwrap();
    let mut b_state = EditorState::create(s.clone(), b.projected_doc(&s).unwrap(), plugins());

    let mut tr = a_state.tr();
    tr.set_node_attr(4, "data-ref", AttrValue::from("renumbered"))
        .unwrap();
    tr.delete(3, 4).unwrap();
    let kept = kept_atom_pairs(&tr);
    assert_eq!(
        kept.len(),
        1,
        "the transaction proves the renumbered picture"
    );
    let next = a_state.apply(tr);
    a.record_local_edit(&s, &a_state.doc, &next.doc, &kept)
        .unwrap();
    a_state = next;

    let mut tr = b_state.tr();
    tr.set_node_attr(3, "data-note", AttrValue::from("for-first"))
        .unwrap();
    let kept = kept_atom_pairs(&tr);
    let next = b_state.apply(tr);
    b.record_local_edit(&s, &b_state.doc, &next.doc, &kept)
        .unwrap();
    b_state = next;

    let to_b = a.sync_diff(&b.state_vector()).unwrap();
    let to_a = b.sync_diff(&a.state_vector()).unwrap();
    if let Some(n) = b.integrate_incremental(&b_state, &to_b).unwrap() {
        b_state = n;
    }
    if let Some(n) = a.integrate_incremental(&a_state, &to_a).unwrap() {
        a_state = n;
    }
    let (ha, hb) = (node_to_html(&a_state.doc), node_to_html(&b_state.doc));
    eprintln!("R1p {ha} | {hb}");
    assert_eq!(ha, hb);
    assert!(
        !ha.split("<img")
            .any(|t| t.contains("renumbered") && t.contains("for-first")),
        "B's note for the deleted copy shows on the renumbered one: {ha}"
    );
}
