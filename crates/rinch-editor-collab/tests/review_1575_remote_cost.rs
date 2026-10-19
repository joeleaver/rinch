//! Review of #1575: what a remote keystroke reads when the document holds an
//! attributed embed (a second full read of the CRDT plus a whole-document
//! compare), against the same document without one.

use std::rc::Rc;
use std::time::Instant;

use rinch_editor_collab::testing::node_reads;
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn doc(s: &Schema, n: usize, embed: bool) -> Node {
    let mut blocks: Vec<Node> = (0..n)
        .map(|i| {
            s.branch(
                "paragraph",
                Fragment::from_node(s.text(&format!("paragraph {i} with text")).unwrap()),
            )
            .unwrap()
        })
        .collect();
    if embed {
        let e = s
            .create_node(
                "embed",
                Attrs::from_iter([("data-sheet", AttrValue::from("s1"))]),
                Fragment::empty(),
            )
            .unwrap();
        blocks.insert(n / 2, e);
    }
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

fn remote_keystroke(n: usize, embed: bool) -> (u64, std::time::Duration) {
    let s = Rc::new(Schema::starter_kit());
    let a_state = EditorState::create(s.clone(), doc(&s, n, embed), plugins());
    let mut a = CollabSession::new(&a_state).unwrap();
    let mut b = CollabSession::from_bytes(&a.snapshot()).unwrap();
    let b_state = EditorState::create(s.clone(), a_state.doc.clone(), plugins());
    // A types one char at the start of the last paragraph.
    let at = a_state.doc.content_size() - 3;
    let mut tr = a_state.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    let _ = tr.insert_text("x");
    let a2 = a_state.apply(tr);
    a.record_local(&s, &a_state.doc, &a2.doc).unwrap();
    let delta = a.save_incremental().unwrap();
    let r0 = node_reads();
    let t0 = Instant::now();
    let tr = b
        .integrate_incremental_transaction(&b_state, &delta)
        .unwrap();
    let dt = t0.elapsed();
    assert!(tr.is_some());
    (node_reads() - r0, dt)
}

#[test]
fn remote_keystroke_cost_with_and_without_an_attributed_embed() {
    for n in [500, 1000, 2000] {
        let (r0, t0) = remote_keystroke(n, false);
        let (r1, t1) = remote_keystroke(n, true);
        eprintln!("n={n}: without embed {r0} reads {t0:?}; with embed {r1} reads {t1:?}");
    }
}
