//! Issue #1227: a textblock retyped into one that allows less, while a peer formats it.
//!
//! `reconcile_node` retypes a textblock in place (its `type` key rewritten, its `Text`
//! kept), so a peer's concurrent typing survives a paragraph → code block change. But
//! formatting and inline atoms merge into that `Text` independently of the `type` key:
//! peer A runs `setCodeBlock` while peer B bolds a character and inserts a hard break,
//! and both used to converge on `<pre>a<strong>b</strong>c<br></pre>` — content the
//! starter kit's `code_block` (`text*`, no marks) forbids, which no local edit can make.
//!
//! The read normalises (ProseMirror's `setBlockType` / `clearIncompatible` rule): a mark
//! the block type does not allow is dropped, a `hard_break` in a code block is the line
//! break `"\n"`, and any other inline node the block does not accept is dropped. Every
//! replica reads the same CRDT, so the normalisation converges by construction; the
//! CRDT keeps the stray formatting until a local edit of that block writes the block
//! back as the model holds it.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, Slice,
    Transaction, default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn schema() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}

fn para(s: &Schema, text: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

/// Every node's children satisfy its content expression **and** every mark is one its
/// parent allows: the document is one a model can hold.
fn assert_valid(schema: &Schema, node: &Node) {
    if node.is_text() {
        return;
    }
    let names: Vec<&str> = (0..node.child_count())
        .map(|i| node.child(i).type_name())
        .collect();
    assert!(
        node.node_type().content_match().matches(&names),
        "<{}> holds {names:?}, which its schema does not allow",
        node.type_name()
    );
    for i in 0..node.child_count() {
        let child = node.child(i);
        for m in child.marks() {
            assert!(
                schema.allows_mark(node.type_name(), m.type_name()),
                "<{}> holds a `{}` mark, which its schema does not allow",
                node.type_name(),
                m.type_name()
            );
        }
        assert_valid(schema, child);
    }
}

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn host(schema: &Rc<Schema>, blocks: Vec<Node>, client_id: u64) -> Peer {
        let state = EditorState::create(schema.clone(), doc_of(schema, blocks), plugins());
        let session = session_with_client_id(&state, client_id).unwrap();
        Peer {
            schema: schema.clone(),
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

    fn local(&mut self, f: impl FnOnce(&mut Transaction)) {
        let mut tr = self.state.tr();
        f(&mut tr);
        let next = self.state.apply(tr);
        self.commit(next);
    }

    fn run_at(&mut self, sel: Selection, name: &str) {
        let mut tr = self.state.tr();
        tr.set_selection(sel);
        let placed = self.state.apply(tr);
        let next = placed
            .run(name)
            .unwrap_or_else(|| panic!("`{name}` applies"));
        self.commit(next);
    }

    fn run(&mut self, pos: usize, name: &str) {
        self.run_at(Selection::cursor(Pos(pos)), name);
    }

    fn insert_node(&mut self, pos: usize, node: Node) {
        self.local(|tr| {
            tr.replace(pos, pos, Slice::new(Fragment::from_node(node), 0, 0))
                .unwrap();
        });
    }

    fn type_at(&mut self, pos: usize, text: &str) {
        self.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(pos)));
            tr.insert_text(text).unwrap();
        });
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
}

fn html(doc: &Node) -> String {
    rinch_editor_core::serialize::node_to_html(doc)
}

/// B's edit: bold `b`, then a hard break after `c` (positions in `<p>abc</p>`).
fn bold_b_and_break_after_c(b: &mut Peer) {
    b.run_at(Selection::text(Pos(2), Pos(3)), "toggleBold");
    b.run(4, "insertHardBreak");
}

/// The issue's scenario, merged in both client-id orders and both delivery orders.
/// Returns the converged peers of each run.
fn scenario(b_edit: impl Fn(&mut Peer)) -> Vec<(Peer, Peer)> {
    let mut out = Vec::new();
    for ids in [(11u64, 22u64), (22, 11)] {
        for a_first in [true, false] {
            let s = schema();
            let mut a = Peer::host(&s, vec![para(&s, "abc")], ids.0);
            let mut b = a.join(ids.1);
            let _ = a.send();
            a.run(1, "setCodeBlock");
            b_edit(&mut b);
            let (da, db) = (a.send(), b.send());
            if a_first {
                a.receive(&db);
                b.receive(&da);
            } else {
                b.receive(&da);
                a.receive(&db);
            }
            assert_eq!(
                a.state.doc, b.state.doc,
                "the peers must converge (ids {ids:?}, a_first {a_first})"
            );
            assert_valid(&s, &a.state.doc);
            out.push((a, b));
        }
    }
    out
}

#[test]
fn a_code_block_retyped_while_a_peer_bolds_and_breaks_it_reads_valid() {
    for (a, _) in scenario(bold_b_and_break_after_c) {
        assert_eq!(
            html(&a.state.doc),
            "<pre><code>abc\n</code></pre>",
            "the bold is dropped and the hard break is a line break"
        );
    }
}

#[test]
fn a_local_edit_after_the_repair_writes_the_block_back_clean() {
    for (mut a, mut b) in scenario(bold_b_and_break_after_c) {
        // A types at the end of the code block: the block is written back as the
        // model holds it, so the CRDT's stray bold and break are gone for good.
        a.type_at(5, "d");
        let da = a.send();
        b.receive(&da);
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(html(&a.state.doc), "<pre><code>abc\nd</code></pre>");
        // A send with nothing new must stay empty-handed: the read and the CRDT agree,
        // so nothing fights over the block.
        let again = a.send();
        b.receive(&again);
        assert_eq!(a.state.doc, b.state.doc);

        // Now B turns the block back into a paragraph. The bold B made concurrently
        // with the retype is not resurrected: the model never held it after the
        // repair, and the reconcile wrote the block from the model.
        b.run(1, "setParagraph");
        let db = b.send();
        a.receive(&db);
        assert_eq!(a.state.doc, b.state.doc);
        assert_valid(&a.schema, &a.state.doc);
        assert!(
            !html(&a.state.doc).contains("<strong>"),
            "a dropped mark must not come back: {}",
            html(&a.state.doc)
        );
    }
}

#[test]
fn retyping_the_untouched_block_back_does_not_resurrect_the_peers_bold() {
    for (mut a, mut b) in scenario(bold_b_and_break_after_c) {
        // No edit of the block in between: the CRDT still holds the stray formatting,
        // so this pins that the retype's own reconcile clears it.
        a.run(1, "setParagraph");
        let da = a.send();
        b.receive(&da);
        assert_eq!(a.state.doc, b.state.doc);
        assert_valid(&a.schema, &a.state.doc);
        assert!(
            !html(&a.state.doc).contains("<strong>"),
            "a dropped mark must not come back: {}",
            html(&a.state.doc)
        );
    }
}

#[test]
fn an_image_inserted_into_a_block_being_made_code_is_dropped() {
    let image = |b: &mut Peer| {
        let img = b
            .schema
            .create_node(
                "image",
                Attrs::new()
                    .with("src", AttrValue::from("a.png"))
                    .with("alt", AttrValue::from("x")),
                Fragment::empty(),
            )
            .unwrap();
        b.insert_node(3, img);
        b.run_at(Selection::text(Pos(1), Pos(2)), "toggleItalic");
    };
    for (mut a, mut b) in scenario(image) {
        assert_eq!(html(&a.state.doc), "<pre><code>abc</code></pre>");
        // Typing after the dropped image lands where the model says, on both peers.
        a.type_at(4, "X");
        let da = a.send();
        b.receive(&da);
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(html(&a.state.doc), "<pre><code>abcX</code></pre>");
    }
}
