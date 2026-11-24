//! #2993: Enter and Shift+Enter in a code block type a `"\n"`, and the newline
//! reaches a peer as text in the block's `Text` — the code block stays one
//! block with one text child on every replica, and concurrent typing around
//! it lands once. Mod+Enter's paragraph after the block travels too.
//!
//! Positions: `<pre>abcd</pre>` is the code block at 0, its text at 1..5.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{
    Attrs, EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    /// A host whose document is one code block holding `text`.
    fn host(text: &str, client_id: u64) -> Peer {
        let schema = Rc::new(Schema::starter_kit());
        let code = schema
            .create_node(
                "code_block",
                Attrs::new(),
                Fragment::from_node(schema.text(text).unwrap()),
            )
            .unwrap();
        let doc = schema.branch("doc", Fragment::from_node(code)).unwrap();
        let state = EditorState::create(schema.clone(), doc, plugins());
        let session = session_with_client_id(&state, client_id).unwrap();
        Peer {
            schema,
            state,
            session,
        }
    }

    fn join(&self, client_id: u64) -> Peer {
        let session =
            session_from_bytes_with_client_id(&self.session.snapshot(), client_id).unwrap();
        let doc = session.projected_doc(&self.schema).unwrap();
        Peer {
            schema: self.schema.clone(),
            state: EditorState::create(self.schema.clone(), doc, plugins()),
            session,
        }
    }

    fn commit(&mut self, next: EditorState) {
        self.session
            .record_local(&self.schema, &self.state.doc, &next.doc)
            .expect("the edit projects");
        self.state = next;
        self.assert_model_is_projection("after a local edit");
    }

    fn run(&mut self, pos: usize, name: &str) {
        let mut tr = self.state.tr();
        tr.set_selection(Selection::cursor(Pos(pos)));
        let placed = self.state.apply(tr);
        let next = placed
            .run(name)
            .unwrap_or_else(|| panic!("`{name}` applies"));
        self.commit(next);
    }

    fn type_at(&mut self, pos: usize, text: &str) {
        let mut tr = self.state.tr();
        tr.set_selection(Selection::cursor(Pos(pos)));
        tr.insert_text(text).unwrap();
        let next = self.state.apply(tr);
        self.commit(next);
    }

    fn send(&mut self) -> Vec<u8> {
        self.session.save_incremental().unwrap()
    }

    fn receive(&mut self, delta: &[u8]) {
        if let Some(next) = self
            .session
            .integrate_incremental(&self.state, delta)
            .unwrap()
        {
            self.state = next;
        }
        self.assert_model_is_projection("after a remote change");
    }

    fn assert_model_is_projection(&self, what: &str) {
        let projected = self.session.projected_doc(&self.schema).unwrap();
        assert_eq!(self.state.doc, projected, "{what}: model ≡ project(model)");
    }

    fn html(&self) -> String {
        node_to_html(&self.state.doc)
    }
}

/// The first block's children: one text node, its string.
fn only_text(doc: &Node) -> String {
    let block = doc.child(0);
    assert_eq!(block.type_name(), "code_block");
    assert_eq!(block.child_count(), 1, "one text child: {block:?}");
    block.child(0).text().expect("a text node").to_string()
}

#[test]
fn a_newline_typed_by_enter_reaches_the_peer_as_text() {
    for key in ["enter", "insertLineBreak"] {
        let mut a = Peer::host("abcd", 11);
        let mut b = a.join(22);
        let _ = a.send();
        a.run(3, key);
        let delta = a.send();
        b.receive(&delta);
        assert_eq!(only_text(&b.state.doc), "ab\ncd", "{key}");
        assert_eq!(a.state.doc, b.state.doc, "{key}");
        // A peer that joins afterwards reads it from the snapshot as well.
        let c = a.join(33);
        assert_eq!(only_text(&c.state.doc), "ab\ncd", "{key}");
    }
}

#[test]
fn concurrent_typing_around_a_newline_lands_once() {
    for ids in [(11u64, 22u64), (22, 11)] {
        for a_first in [true, false] {
            let mut a = Peer::host("abcd", ids.0);
            let mut b = a.join(ids.1);
            let _ = a.send();
            a.run(3, "enter");
            b.type_at(5, "Z");
            b.type_at(1, "Y");
            let (da, db) = (a.send(), b.send());
            if a_first {
                a.receive(&db);
                b.receive(&da);
            } else {
                b.receive(&da);
                a.receive(&db);
            }
            assert_eq!(a.state.doc, b.state.doc, "ids {ids:?}, a_first {a_first}");
            assert_eq!(only_text(&a.state.doc), "Yab\ncdZ", "ids {ids:?}");
            // The peer goes on typing on the new line.
            b.type_at(5, "X");
            let d = b.send();
            a.receive(&d);
            assert_eq!(only_text(&a.state.doc), "Yab\nXcdZ", "ids {ids:?}");
        }
    }
}

#[test]
fn exit_code_paragraph_reaches_the_peer() {
    let mut a = Peer::host("ab\ncd", 11);
    let mut b = a.join(22);
    let _ = a.send();
    a.run(3, "exitCode");
    let delta = a.send();
    b.receive(&delta);
    assert_eq!(b.html(), "<pre>ab\ncd</pre><p></p>");
    assert_eq!(a.state.doc, b.state.doc);
}
