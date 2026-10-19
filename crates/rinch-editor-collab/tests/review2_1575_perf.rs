//! Round-2 review of #1575: what a LOCAL keystroke costs in a collaborating
//! document with no embed, before and after the session has integrated a peer's
//! change (which leaves an `atom_cache` entry that every later local edit reads
//! and rewrites). Prints only; run in release.
use std::rc::Rc;
use std::time::Instant;

use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn state(s: &Rc<Schema>, n: usize) -> EditorState {
    let blocks: Vec<Node> = (0..n)
        .map(|i| {
            s.branch(
                "paragraph",
                Fragment::from_node(
                    s.text(&format!("paragraph number {i} with some text"))
                        .unwrap(),
                ),
            )
            .unwrap()
        })
        .collect();
    EditorState::create(
        s.clone(),
        s.branch("doc", Fragment::from_children(blocks)).unwrap(),
        plugins(),
    )
}

fn type_mid(st: &EditorState) -> EditorState {
    let n = st.doc.child_count();
    let at: usize = (0..n / 2)
        .map(|i| st.doc.child(i).node_size())
        .sum::<usize>()
        + 3;
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("x").unwrap();
    st.apply(tr)
}

fn keystrokes(
    s: &Rc<Schema>,
    session: &mut CollabSession,
    st: EditorState,
    k: usize,
) -> (EditorState, f64) {
    let mut chain = vec![st];
    for _ in 0..k {
        let n = type_mid(chain.last().unwrap());
        chain.push(n);
    }
    let mut total = 0.0;
    for w in chain.windows(2) {
        let t = Instant::now();
        session.record_local(s, &w[0].doc, &w[1].doc).unwrap();
        total += t.elapsed().as_secs_f64();
        session.save_incremental().unwrap();
    }
    (chain.pop().unwrap(), total * 1e6 / k as f64)
}

#[test]
#[ignore]
fn r2_local_keystroke_cost_with_and_without_a_cache_entry() {
    let s = Rc::new(Schema::starter_kit());
    for n in [2000usize, 8000] {
        for round in 0..5 {
            // cold: a host that never integrated
            let st = state(&s, n);
            let mut a = CollabSession::new(&st).unwrap();
            let (_, cold) = keystrokes(&s, &mut a, st, 300);
            // warm: a host that integrated one remote keystroke first
            let st = state(&s, n);
            let mut a = CollabSession::new(&st).unwrap();
            let mut b = CollabSession::from_bytes(&a.snapshot()).unwrap();
            let bst = EditorState::create(s.clone(), b.projected_doc(&s).unwrap(), plugins());
            let bnext = type_mid(&bst);
            b.record_local(&s, &bst.doc, &bnext.doc).unwrap();
            let d = b.save_incremental().unwrap();
            let st = a.integrate_incremental(&st, &d).unwrap().unwrap();
            let (_, warm) = keystrokes(&s, &mut a, st, 300);
            eprintln!(
                "n={n} round {round}: cold {cold:.2} us/keystroke, warm {warm:.2} us/keystroke ({:+.1}%)",
                (warm / cold - 1.0) * 100.0
            );
        }
    }
}
