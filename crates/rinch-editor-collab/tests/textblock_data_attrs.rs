//! A paragraph's and a heading's app `data-*` attributes through collaboration:
//! each is a key of the block's attribute map, so concurrent changes merge per
//! attribute.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::commands::{
    delete_char_backward, set_block_data_attr, set_block_type, set_paragraph, split_block,
};
use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{
    AttrValue, Attrs, Command, EditorState, Plugin, Pos, Schema, Selection, Transaction,
    default_plugins,
};

struct Peer {
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn apply(&mut self, tr: Transaction) {
        let before = self.state.doc.clone();
        let after = self.state.apply(tr);
        self.session
            .record_local(self.state.schema(), &before, &after.doc)
            .expect("project local");
        self.state = after;
    }

    /// Run `cmd` with the caret at `at`.
    fn run(&mut self, at: usize, cmd: Command) {
        let mut tr = self.state.tr();
        tr.set_selection(Selection::text(Pos(at), Pos(at)));
        self.state = self.state.apply(tr);
        let mut out = None;
        let mut dispatch = |tr| out = Some(tr);
        assert!(cmd(&self.state, Some(&mut dispatch)), "the command applies");
        self.apply(out.expect("a transaction"));
    }

    fn type_text(&mut self, at: usize, text: &str) {
        let mut tr = self.state.tr();
        tr.set_selection(Selection::text(Pos(at), Pos(at)));
        tr.insert_text(text).unwrap();
        self.apply(tr);
    }

    fn html(&self) -> String {
        node_to_html(&self.state.doc)
    }
}

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn two_peers(html: &str, ids: (u64, u64)) -> (Peer, Peer) {
    let schema = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content).unwrap();
    let a_state = EditorState::create(schema.clone(), doc, plugins());
    let session_a = session_with_client_id(&a_state, ids.0).expect("session A");
    let session_b =
        session_from_bytes_with_client_id(&session_a.snapshot(), ids.1).expect("session B");
    let b_doc = session_b.projected_doc(&schema).expect("project B");
    (
        Peer {
            state: a_state,
            session: session_a,
        },
        Peer {
            state: EditorState::create(schema, b_doc, plugins()),
            session: session_b,
        },
    )
}

/// Exchange diffs until neither side changes, and check both read the same
/// document from their own CRDT.
fn sync(a: &mut Peer, b: &mut Peer) -> String {
    let mut settled = false;
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
            settled = true;
            break;
        }
    }
    assert!(settled, "the state-vector exchange did not settle");
    for peer in [&*a, &*b] {
        assert_eq!(
            peer.state.doc,
            peer.session.projected_doc(peer.state.schema()).unwrap()
        );
    }
    assert_eq!(a.html(), b.html());
    a.html()
}

const ID_ORDERS: [(u64, u64); 2] = [(11, 22), (22, 11)];

fn set(name: &str, value: &str) -> Command {
    set_block_data_attr(name, Some(value))
}

#[test]
fn they_round_trip_through_a_snapshot_and_a_live_update() {
    let (mut a, mut b) = two_peers(r#"<h1 data-role="t">ab</h1><p>cd</p>"#, (11, 22));
    assert_eq!(b.html(), r#"<h1 data-role="t">ab</h1><p>cd</p>"#);
    a.run(5, set("data-role", "sub"));
    assert_eq!(
        sync(&mut a, &mut b),
        r#"<h1 data-role="t">ab</h1><p data-role="sub">cd</p>"#
    );
    b.run(5, set_block_data_attr("data-role", None::<String>));
    b.run(1, set("data-role", ""));
    assert_eq!(sync(&mut a, &mut b), r#"<h1 data-role="">ab</h1><p>cd</p>"#);
}

#[test]
fn concurrent_changes_of_different_attributes_both_survive() {
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers(r#"<p data-x="1">ab</p>"#, ids);
        a.run(1, set("data-role", "sub"));
        b.run(1, set("data-x", "2"));
        assert_eq!(
            sync(&mut a, &mut b),
            r#"<p data-role="sub" data-x="2">ab</p>"#
        );
    }
}

#[test]
fn concurrent_changes_of_one_attribute_converge_on_one_of_them() {
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers("<p>ab</p>", ids);
        a.run(1, set("data-role", "a"));
        b.run(1, set("data-role", "b"));
        let html = sync(&mut a, &mut b);
        assert!(
            html == r#"<p data-role="a">ab</p>"# || html == r#"<p data-role="b">ab</p>"#,
            "{html}"
        );
    }
}

#[test]
fn an_attribute_survives_a_peers_concurrent_typing() {
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers("<p>ab</p>", ids);
        a.run(1, set("data-role", "sub"));
        b.type_text(3, "cd");
        assert_eq!(sync(&mut a, &mut b), r#"<p data-role="sub">abcd</p>"#);
    }
}

#[test]
fn splits_joins_and_type_changes_reach_the_peer() {
    let (mut a, mut b) = two_peers(r#"<p data-role="sub">abcd</p>"#, (11, 22));
    a.run(3, split_block());
    assert_eq!(
        sync(&mut a, &mut b),
        r#"<p data-role="sub">ab</p><p data-role="sub">cd</p>"#
    );
    b.run(5, delete_char_backward());
    assert_eq!(sync(&mut a, &mut b), r#"<p data-role="sub">abcd</p>"#);
    a.run(5, split_block());
    assert_eq!(
        sync(&mut a, &mut b),
        r#"<p data-role="sub">abcd</p><p data-role="sub"></p>"#
    );
    // At the start: both again.
    b.run(1, split_block());
    assert_eq!(
        sync(&mut a, &mut b),
        r#"<p data-role="sub"></p><p data-role="sub">abcd</p><p data-role="sub"></p>"#
    );
    // Backspace joins the empty one away; the text's block stays.
    a.run(3, delete_char_backward());
    assert_eq!(
        sync(&mut a, &mut b),
        r#"<p data-role="sub">abcd</p><p data-role="sub"></p>"#
    );
    a.run(2, set_block_type("code_block", Attrs::new()));
    assert_eq!(
        sync(&mut a, &mut b),
        r#"<pre>abcd</pre><p data-role="sub"></p>"#
    );
    b.run(2, set_block_type("paragraph", Attrs::new()));
    b.run(2, set("data-role", "x"));
    // A paragraph reset is a paragraph still.
    assert!(!set_paragraph()(&b.state, None));
    b.run(
        2,
        set_block_type("heading", Attrs::from_iter([("level", AttrValue::Int(2))])),
    );
    assert_eq!(
        sync(&mut a, &mut b),
        r#"<h2>abcd</h2><p data-role="sub"></p>"#
    );
}

/// A join that keeps only the second block's text has its attributes, on both
/// peers.
#[test]
fn a_join_after_an_empty_block_reaches_the_peer() {
    let start = r#"<p data-role="a"></p><p data-role="b">cd</p>"#;
    let (mut a, mut b) = two_peers(start, (11, 22));
    a.run(3, delete_char_backward());
    assert_eq!(sync(&mut a, &mut b), r#"<p data-role="b">cd</p>"#);
}

#[test]
fn a_concurrent_split_and_set_converge() {
    for ids in ID_ORDERS {
        let (mut a, mut b) = two_peers("<p>abcd</p>", ids);
        a.run(3, split_block());
        b.run(1, set("data-role", "sub"));
        let html = sync(&mut a, &mut b);
        // The first half is the block that was there, so the set lands on it.
        assert_eq!(html, r#"<p data-role="sub">ab</p><p>cd</p>"#);
    }
}
