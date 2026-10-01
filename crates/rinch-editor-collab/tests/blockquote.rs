//! Block quotes in the projected scope.
//!
//! A `blockquote` is a container (`block+`), projected exactly as a list is: a node map
//! with a `content` array of child nodes, recursively, so a quote can hold paragraphs,
//! headings, code, scene breaks, lists and other quotes. These tests pin the round trip,
//! every local edit shape that reaches into or out of a quote (wrap, lift, typing,
//! split, join, a list inside one), the remote rebuild and the caret it carries, sticky
//! positions inside a quote, and concurrent edits merged both ways.
//!
//! Two rules came with quotes and are pinned here too, for lists as well:
//!
//! * **A void container reads as absent.** Concurrent deletions can empty a container
//!   (two peers each deleting one of a quote's two paragraphs); no model can hold an
//!   empty quote or list, so the read-back skips it on every replica alike.
//! * **A container retyped into one with different children is replaced**, never
//!   retyped in place, so a peer's concurrent insert cannot leave a list item directly
//!   inside a quote.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    EditorState, Fragment, Mark, Node, Plugin, Pos, Schema, Selection, Slice, Transaction,
    default_plugins,
};

// --- harness -------------------------------------------------------------------

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn schema() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}

fn para(s: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    s.branch("paragraph", content).unwrap()
}

fn heading(s: &Schema, text: &str) -> Node {
    s.create_node(
        "heading",
        rinch_editor_core::Attrs::new().with("level", 2i64),
        Fragment::from_node(s.text(text).unwrap()),
    )
    .unwrap()
}

fn branch(s: &Schema, type_name: &str, children: Vec<Node>) -> Node {
    s.branch(type_name, Fragment::from_children(children))
        .unwrap()
}

fn quote(s: &Schema, children: Vec<Node>) -> Node {
    branch(s, "blockquote", children)
}

fn bullets(s: &Schema, items: Vec<&str>) -> Node {
    branch(
        s,
        "bullet_list",
        items
            .into_iter()
            .map(|t| branch(s, "list_item", vec![para(s, t)]))
            .collect(),
    )
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    branch(s, "doc", blocks)
}

/// The model position of the first char of `needle`, searching every text node.
fn pos_of(doc: &Node, needle: &str) -> usize {
    fn walk(node: &Node, content_start: usize, needle: &str) -> Option<usize> {
        let mut at = content_start;
        for i in 0..node.child_count() {
            let child = node.child(i);
            if let Some(text) = child.text() {
                if let Some(byte) = text.find(needle) {
                    return Some(at + text[..byte].chars().count());
                }
            } else if let Some(found) = walk(child, at + 1, needle) {
                return Some(found);
            }
            at += child.node_size();
        }
        None
    }
    walk(doc, 0, needle).unwrap_or_else(|| panic!("`{needle}` is not in the document"))
}

/// Every node's children satisfy its content expression: the converged document is one
/// a model can hold, not merely the same on every replica.
fn assert_valid(node: &Node) {
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
        assert_valid(node.child(i));
    }
}

/// One collaborating editor.
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
        let session = session_from_bytes_with_client_id(&self.session.snapshot(), client_id)
            .expect("join from snapshot");
        let doc = session.projected_doc(&self.schema).expect("project");
        Peer {
            schema: self.schema.clone(),
            state: EditorState::create(self.schema.clone(), doc, plugins()),
            session,
        }
    }

    /// Project the move from the current state to `next`, asserting the invariant.
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

    /// Put the caret at `pos` and run the editor command `name`.
    fn run(&mut self, pos: usize, name: &str) {
        let mut tr = self.state.tr();
        tr.set_selection(Selection::cursor(Pos(pos)));
        let placed = self.state.apply(tr);
        let next = placed
            .run(name)
            .unwrap_or_else(|| panic!("`{name}` applies at {pos}"));
        self.commit(next);
    }

    fn type_at(&mut self, pos: usize, text: &str) {
        self.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(pos)));
            tr.insert_text(text).unwrap();
        });
    }

    /// Type `text` right after the first occurrence of `after`.
    fn type_after(&mut self, after: &str, text: &str) {
        let at = pos_of(&self.state.doc, after) + after.chars().count();
        self.type_at(at, text);
    }

    fn delete(&mut self, from: usize, to: usize) {
        self.local(|tr| {
            tr.delete(from, to).unwrap();
        });
    }

    fn send(&mut self) -> Vec<u8> {
        self.session.save_incremental().expect("delta")
    }

    fn receive(&mut self, delta: &[u8]) {
        if let Some(next) = self
            .session
            .integrate_incremental(&self.state, delta)
            .expect("integrate")
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

/// Exchange everything both peers have not sent yet, each integrating the other's.
fn exchange(a: &mut Peer, b: &mut Peer) {
    let (da, db) = (a.send(), b.send());
    b.receive(&da);
    a.receive(&db);
}

/// Run a two-peer concurrent scenario and merge it both ways: `a_edit` and `b_edit` are
/// made concurrently on a host and a guest of `blocks`, then each integrates the
/// other's delta. Run twice, with the client ids swapped (they break yrs's concurrent
/// tie-breaks), and every run must converge on a valid document. Returns the converged
/// document of each run.
fn concurrently(
    blocks: impl Fn(&Schema) -> Vec<Node>,
    a_edit: impl Fn(&mut Peer),
    b_edit: impl Fn(&mut Peer),
) -> Vec<Node> {
    let mut out = Vec::new();
    for ids in [(11u64, 22u64), (22, 11)] {
        let s = schema();
        let mut a = Peer::host(&s, blocks(&s), ids.0);
        let mut b = a.join(ids.1);
        let _ = a.send();
        a_edit(&mut a);
        b_edit(&mut b);
        exchange(&mut a, &mut b);
        assert_eq!(
            a.state.doc, b.state.doc,
            "the peers must converge (ids {ids:?})"
        );
        assert_valid(&a.state.doc);
        out.push(a.state.doc.clone());
    }
    out
}

fn html(doc: &Node) -> String {
    rinch_editor_core::serialize::node_to_html(doc)
}

// --- the round trip --------------------------------------------------------------

#[test]
fn a_quote_round_trips_with_everything_it_can_hold() {
    let s = schema();
    let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
    let rich = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![
                s.text("plain ").unwrap(),
                s.text_with_marks("strong", vec![bold]).unwrap(),
            ]),
        )
        .unwrap();
    let inner = quote(
        &s,
        vec![para(&s, "nested"), bullets(&s, vec!["in", "list"])],
    );
    let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
    let doc = doc_of(
        &s,
        vec![
            para(&s, "before"),
            quote(&s, vec![heading(&s, "title"), rich, hr, inner]),
            branch(
                &s,
                "bullet_list",
                vec![branch(
                    &s,
                    "list_item",
                    vec![quote(&s, vec![para(&s, "q in li")])],
                )],
            ),
        ],
    );
    let cdoc = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap();
    assert_eq!(cdoc.to_doc(&s).unwrap(), doc);
    // And through the saved bytes, the way a guest joins.
    let loaded = rinch_editor_collab::CollabDoc::load(&cdoc.save()).unwrap();
    assert_eq!(loaded.to_doc(&s).unwrap(), doc);
}

#[test]
fn an_empty_quote_in_the_model_is_refused_loud() {
    // The schema forbids it and the read-back would skip it (a void container), so it
    // could not round-trip; only a node built past the schema gets here.
    let s = schema();
    let doc = doc_of(&s, vec![para(&s, "x"), quote(&s, vec![])]);
    let err = rinch_editor_collab::CollabDoc::from_doc(&doc).unwrap_err();
    assert!(
        matches!(err, rinch_editor_collab::CollabError::Schema(_)),
        "got {err:?}"
    );
}

// --- local edits reaching the peer ----------------------------------------------

#[test]
fn wrapping_a_paragraph_in_a_quote_reaches_the_peer() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "one"), para(&s, "two")], 1);
    let mut b = a.join(2);
    let _ = a.send();
    a.run(pos_of(&a.state.doc, "two"), "wrapInBlockquote");
    assert_eq!(a.state.doc.child(1).type_name(), "blockquote");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(
        html(&b.state.doc),
        "<p>one</p><blockquote><p>two</p></blockquote>"
    );
}

#[test]
fn lifting_a_paragraph_out_of_a_quote_reaches_the_peer() {
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![quote(
            &s,
            vec![para(&s, "one"), para(&s, "two"), para(&s, "three")],
        )],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    // Lifting the middle paragraph splits the quote around it.
    a.run(pos_of(&a.state.doc, "two"), "liftListItem");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(
        html(&b.state.doc),
        "<blockquote><p>one</p></blockquote><p>two</p><blockquote><p>three</p></blockquote>"
    );
}

#[test]
fn typing_splitting_and_joining_inside_a_quote_reach_the_peer() {
    let s = schema();
    let mut a = Peer::host(&s, vec![quote(&s, vec![para(&s, "hello world")])], 1);
    let mut b = a.join(2);
    let _ = a.send();

    a.type_after("hello", ",");
    exchange(&mut a, &mut b);
    assert_eq!(
        html(&b.state.doc),
        "<blockquote><p>hello, world</p></blockquote>"
    );

    // Enter in the middle: two paragraphs, both still in the quote.
    a.run(pos_of(&a.state.doc, " world"), "enter");
    exchange(&mut a, &mut b);
    assert_eq!(
        html(&b.state.doc),
        "<blockquote><p>hello,</p><p> world</p></blockquote>"
    );

    // Backspace at the start of the second paragraph joins them again.
    a.run(pos_of(&a.state.doc, " world"), "deleteCharBackward");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(
        html(&b.state.doc),
        "<blockquote><p>hello, world</p></blockquote>"
    );
}

#[test]
fn enter_in_an_empty_last_paragraph_of_a_quote_leaves_it_on_both_peers() {
    let s = schema();
    let mut a = Peer::host(&s, vec![quote(&s, vec![para(&s, "quoted")])], 1);
    let mut b = a.join(2);
    let _ = a.send();
    let end = pos_of(&a.state.doc, "quoted") + "quoted".len();
    a.run(end, "enter");
    // The caret is in the new empty paragraph; Enter there exits the quote.
    let caret = a.state.selection.head().0;
    a.run(caret, "enter");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(a.state.doc.child(0).type_name(), "blockquote");
    assert_eq!(a.state.doc.child_count(), 2, "{}", html(&a.state.doc));
}

#[test]
fn a_list_inside_a_quote_is_edited_and_reaches_the_peer() {
    let s = schema();
    let mut a = Peer::host(&s, vec![quote(&s, vec![para(&s, "item")])], 1);
    let mut b = a.join(2);
    let _ = a.send();
    a.run(pos_of(&a.state.doc, "item"), "toggleBulletList");
    exchange(&mut a, &mut b);
    assert_eq!(
        html(&b.state.doc),
        "<blockquote><ul><li><p>item</p></li></ul></blockquote>"
    );
    // A second item, by Enter at its end, then typing into it.
    let end = pos_of(&a.state.doc, "item") + 4;
    a.run(end, "enter");
    let caret = a.state.selection.head().0;
    a.type_at(caret, "next");
    // Indent it under the first.
    a.run(pos_of(&a.state.doc, "next"), "sinkListItem");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(
        html(&b.state.doc),
        "<blockquote><ul><li><p>item</p><ul><li><p>next</p></li></ul></li></ul></blockquote>"
    );
}

#[test]
fn a_quote_inside_a_quote_reaches_the_peer() {
    let s = schema();
    let mut a = Peer::host(&s, vec![quote(&s, vec![para(&s, "a"), para(&s, "b")])], 1);
    let mut b = a.join(2);
    let _ = a.send();
    a.run(pos_of(&a.state.doc, "b"), "wrapInBlockquote");
    a.type_after("b", "!");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(
        html(&b.state.doc),
        "<blockquote><p>a</p><blockquote><p>b!</p></blockquote></blockquote>"
    );
}

// --- the remote rebuild and the caret it carries -------------------------------

#[test]
fn a_caret_inside_a_quote_keeps_its_place_when_a_peer_types_before_it() {
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![quote(&s, vec![para(&s, "first"), para(&s, "hello world")])],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    // B's caret sits before "world".
    let caret = pos_of(&b.state.doc, "world");
    let mut tr = b.state.tr();
    tr.set_selection(Selection::cursor(Pos(caret)));
    b.state = b.state.apply(tr);

    // A types at the start of B's paragraph, and in the paragraph before it.
    a.type_at(pos_of(&a.state.doc, "hello"), ">> ");
    a.type_after("first", "!");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.doc, a.state.doc);
    assert_eq!(
        b.state.selection.head().0,
        pos_of(&b.state.doc, "world"),
        "the caret stays before \"world\", not at the end of the quote"
    );
}

#[test]
fn a_caret_in_an_untouched_paragraph_of_a_changed_quote_stays_put() {
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![quote(&s, vec![para(&s, "edited"), para(&s, "mine")])],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    let caret = pos_of(&b.state.doc, "ine");
    let mut tr = b.state.tr();
    tr.set_selection(Selection::cursor(Pos(caret)));
    b.state = b.state.apply(tr);

    // A adds a whole paragraph before B's: the quote's child count changes.
    a.run(pos_of(&a.state.doc, "edited") + 6, "enter");
    let at = a.state.selection.head().0;
    a.type_at(at, "inserted");
    exchange(&mut a, &mut b);
    assert_eq!(b.state.selection.head().0, pos_of(&b.state.doc, "ine"));
}

// --- sticky positions --------------------------------------------------------------

#[test]
fn a_sticky_index_inside_a_quote_follows_its_character() {
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![
            para(&s, "lead"),
            quote(
                &s,
                vec![para(&s, "one"), quote(&s, vec![para(&s, "deep text")])],
            ),
        ],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    let at = pos_of(&a.state.doc, "text");
    let sticky = a
        .session
        .sticky_index(&a.state.doc, Pos(at))
        .expect("an index");
    assert_eq!(
        a.session.resolve_sticky(&a.state.doc, &sticky),
        Some(Pos(at))
    );

    // B edits before it (inside both quotes, and above them); A resolves after merging.
    b.type_after("deep", "er");
    b.type_at(1, "the ");
    b.run(pos_of(&b.state.doc, "one"), "enter");
    exchange(&mut a, &mut b);
    assert_eq!(
        a.session.resolve_sticky(&a.state.doc, &sticky),
        Some(Pos(pos_of(&a.state.doc, "text")))
    );
    // B resolves the same bytes against its own replica.
    assert_eq!(
        b.session.resolve_sticky(&b.state.doc, &sticky),
        Some(Pos(pos_of(&b.state.doc, "text")))
    );
}

#[test]
fn a_sticky_index_after_a_void_container_counts_only_what_the_model_holds() {
    // A quote emptied by concurrent deletions stays in the CRDT but not in the model;
    // the walk from the model to the CRDT and back must skip it, or every index after
    // it would be off by one block.
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![
            quote(&s, vec![para(&s, "x"), para(&s, "y")]),
            para(&s, "after it"),
        ],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    // A deletes the quote's first paragraph, B its second.
    let (x, y) = (pos_of(&a.state.doc, "x"), pos_of(&a.state.doc, "y"));
    a.delete(x - 1, x + 2);
    b.delete(y - 1, y + 2);
    exchange(&mut a, &mut b);
    assert_eq!(html(&a.state.doc), "<p>after it</p>");

    let at = pos_of(&a.state.doc, "it");
    let sticky = a
        .session
        .sticky_index(&a.state.doc, Pos(at))
        .expect("an index");
    assert_eq!(
        b.session.resolve_sticky(&b.state.doc, &sticky),
        Some(Pos(at))
    );
}

// --- concurrency ----------------------------------------------------------------

#[test]
fn two_peers_typing_in_different_paragraphs_of_one_quote_both_land() {
    for doc in concurrently(
        |s| vec![quote(s, vec![para(s, "one"), para(s, "two")])],
        |a| a.type_after("one", "A"),
        |b| b.type_after("two", "B"),
    ) {
        assert_eq!(
            html(&doc),
            "<blockquote><p>oneA</p><p>twoB</p></blockquote>"
        );
    }
}

#[test]
fn two_peers_typing_in_one_paragraph_of_a_quote_both_land() {
    for doc in concurrently(
        |s| vec![quote(s, vec![para(s, "start end")])],
        |a| a.type_after("start", " A"),
        |b| b.type_after("end", " B"),
    ) {
        assert_eq!(html(&doc), "<blockquote><p>start A end B</p></blockquote>");
    }
}

#[test]
fn typing_inside_a_quote_and_around_it_both_land() {
    for doc in concurrently(
        |s| {
            vec![
                para(s, "above"),
                quote(s, vec![para(s, "inside")]),
                para(s, "below"),
            ]
        },
        |a| a.type_after("inside", "!"),
        |b| {
            b.type_after("above", "?");
            b.type_after("below", "?");
        },
    ) {
        assert_eq!(
            html(&doc),
            "<p>above?</p><blockquote><p>inside!</p></blockquote><p>below?</p>"
        );
    }
}

#[test]
fn splitting_inside_a_quote_while_a_peer_types_in_another_paragraph() {
    for doc in concurrently(
        |s| vec![quote(s, vec![para(s, "split here"), para(s, "other")])],
        |a| a.run(pos_of(&a.state.doc, " here"), "enter"),
        |b| b.type_after("other", "!"),
    ) {
        assert_eq!(
            html(&doc),
            "<blockquote><p>split</p><p> here</p><p>other!</p></blockquote>"
        );
    }
}

#[test]
fn wrapping_a_paragraph_while_a_peer_types_in_it_converges_to_the_quote() {
    // The wrap changes the paragraph's kind at its index (a paragraph becomes a
    // container), which the projection writes as a replace: the old paragraph is
    // deleted and a quote holding a copy of it inserted. A peer's concurrent typing
    // lands in the deleted paragraph and is lost — the same trade a list toggle makes
    // (yrs has no move between arrays). What is pinned is that both peers agree, on the
    // quote, and that typing *elsewhere* is untouched.
    for doc in concurrently(
        |s| vec![para(s, "wrap me"), para(s, "elsewhere")],
        |a| a.run(pos_of(&a.state.doc, "wrap"), "wrapInBlockquote"),
        |b| {
            b.type_after("wrap", "ped");
            b.type_after("elsewhere", "!");
        },
    ) {
        assert_eq!(
            html(&doc),
            "<blockquote><p>wrap me</p></blockquote><p>elsewhere!</p>"
        );
    }
}

#[test]
fn typing_in_a_quote_while_a_peer_lifts_its_paragraph_out_converges() {
    // The lift deletes the quote's paragraph and inserts a paragraph after the quote:
    // the same replace trade as the wrap, from the other side.
    let docs = concurrently(
        |s| vec![quote(s, vec![para(s, "keep"), para(s, "lift me")])],
        |a| a.run(pos_of(&a.state.doc, "lift"), "liftListItem"),
        |b| b.type_after("keep", "!"),
    );
    for doc in docs {
        assert_eq!(
            html(&doc),
            "<blockquote><p>keep!</p></blockquote><p>lift me</p>"
        );
    }
}

#[test]
fn wrapping_one_paragraph_in_a_quote_and_another_in_a_list_at_once() {
    for doc in concurrently(
        |s| vec![para(s, "quote me"), para(s, "list me")],
        |a| a.run(pos_of(&a.state.doc, "quote"), "wrapInBlockquote"),
        |b| b.run(pos_of(&b.state.doc, "list"), "toggleBulletList"),
    ) {
        assert_eq!(
            html(&doc),
            "<blockquote><p>quote me</p></blockquote><ul><li><p>list me</p></li></ul>"
        );
    }
}

#[test]
fn two_peers_wrapping_the_same_paragraph_converge() {
    // Each replaces the paragraph with a quote of its own: both quotes survive the merge
    // (two inserts, one delete), in an order the client ids decide. Valid, the same on
    // both peers, and nothing typed is lost.
    for doc in concurrently(
        |s| vec![para(s, "same")],
        |a| a.run(1, "wrapInBlockquote"),
        |b| b.run(1, "wrapInBlockquote"),
    ) {
        assert_eq!(
            html(&doc),
            "<blockquote><p>same</p></blockquote><blockquote><p>same</p></blockquote>"
        );
    }
}

// --- void containers -----------------------------------------------------------

#[test]
fn deleting_both_paragraphs_of_a_quote_concurrently_leaves_no_empty_quote() {
    for doc in concurrently(
        |s| {
            vec![
                para(s, "before"),
                quote(s, vec![para(s, "x"), para(s, "y")]),
                para(s, "after"),
            ]
        },
        |a| {
            let x = pos_of(&a.state.doc, "x");
            a.delete(x - 1, x + 2);
        },
        |b| {
            let y = pos_of(&b.state.doc, "y");
            b.delete(y - 1, y + 2);
        },
    ) {
        assert_eq!(html(&doc), "<p>before</p><p>after</p>");
    }
}

#[test]
fn deleting_both_items_of_a_list_concurrently_leaves_no_empty_list() {
    // The same hole, older than quotes: two peers deleting a two-item list's items one
    // each used to converge on `<ul></ul>`, which no model can hold.
    for doc in concurrently(
        |s| {
            vec![
                para(s, "before"),
                bullets(s, vec!["first", "second"]),
                para(s, "after"),
            ]
        },
        // A list item's text starts two positions into it (the item's and the
        // paragraph's open tokens) and ends two before its end.
        |a| {
            let at = pos_of(&a.state.doc, "first");
            a.delete(at - 2, at + "first".len() + 2);
        },
        |b| {
            let at = pos_of(&b.state.doc, "second");
            b.delete(at - 2, at + "second".len() + 2);
        },
    ) {
        assert_eq!(html(&doc), "<p>before</p><p>after</p>");
    }
}

#[test]
fn editing_next_to_a_void_quote_projects_around_it() {
    // After the quote is void the model has two blocks and the CRDT three; every later
    // edit must address the blocks the model sees, on both sides of the void one.
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![
            para(&s, "before"),
            quote(&s, vec![para(&s, "x"), para(&s, "y")]),
            para(&s, "after"),
        ],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    let x = pos_of(&a.state.doc, "x");
    a.delete(x - 1, x + 2);
    let y = pos_of(&b.state.doc, "y");
    b.delete(y - 1, y + 2);
    exchange(&mut a, &mut b);
    assert_eq!(html(&a.state.doc), "<p>before</p><p>after</p>");

    a.type_after("after", "!");
    a.run(pos_of(&a.state.doc, "fore"), "enter");
    b.type_at(1, "?");
    exchange(&mut a, &mut b);
    assert_eq!(a.state.doc, b.state.doc);
    assert_eq!(html(&a.state.doc), "<p>?be</p><p>fore</p><p>after!</p>");
    // A block inserted between the two, and the second deleted outright.
    let end = pos_of(&b.state.doc, "fore") + 4;
    b.run(end, "enter");
    let caret = b.state.selection.head().0;
    b.type_at(caret, "new");
    let start = pos_of(&a.state.doc, "after!") - 1;
    a.delete(start, start + "after!".len() + 2);
    exchange(&mut a, &mut b);
    assert_eq!(a.state.doc, b.state.doc);
    assert_eq!(html(&a.state.doc), "<p>?be</p><p>fore</p><p>new</p>");
    assert_valid(&a.state.doc);
}

#[test]
fn a_peers_insert_into_a_quote_others_emptied_makes_it_visible_again() {
    // Three peers. A and B each delete one of the quote's paragraphs; C, concurrently,
    // adds a third. The quote is not void after all: it holds C's paragraph alone.
    let s = schema();
    let mut a = Peer::host(&s, vec![quote(&s, vec![para(&s, "x"), para(&s, "y")])], 1);
    let mut b = a.join(2);
    let mut c = a.join(3);
    let _ = a.send();
    let x = pos_of(&a.state.doc, "x");
    a.delete(x - 1, x + 2);
    let y = pos_of(&b.state.doc, "y");
    b.delete(y - 1, y + 2);
    c.run(pos_of(&c.state.doc, "y") + 1, "enter");
    let caret = c.state.selection.head().0;
    c.type_at(caret, "z");

    let (da, db, dc) = (a.send(), b.send(), c.send());
    // A and B first see each other's deletions: the quote is void for them.
    b.receive(&da);
    a.receive(&db);
    assert_eq!(
        html(&a.state.doc),
        "<p></p>",
        "the starter paragraph stands in"
    );
    // Then C's paragraph arrives.
    a.receive(&dc);
    b.receive(&dc);
    c.receive(&da);
    c.receive(&db);
    for peer in [&a, &b, &c] {
        assert_eq!(html(&peer.state.doc), "<blockquote><p>z</p></blockquote>");
    }
}

// --- retyping a container ------------------------------------------------------

/// Replace top-level block 0 of `peer`'s document with `block`, in one transaction.
fn replace_first_block(peer: &mut Peer, block: Node) {
    let end = peer.state.doc.child(0).node_size();
    peer.local(|tr| {
        tr.replace(0, end, Slice::new(Fragment::from_node(block), 0, 0))
            .unwrap();
    });
}

#[test]
fn a_list_turned_into_a_quote_while_a_peer_adds_an_item_stays_valid() {
    // A's list becomes a quote holding the same paragraph. Retyped in place, the list's
    // `content` array would survive under the new type, and B's concurrent `list_item`
    // would land directly inside the quote. Replaced instead, B's item lands in a list
    // that is gone.
    for doc in concurrently(
        |s| vec![bullets(s, vec!["one"])],
        |a| {
            let s = a.schema.clone();
            replace_first_block(a, quote(&s, vec![para(&s, "one")]));
        },
        |b| {
            let end = pos_of(&b.state.doc, "one") + 3;
            b.run(end, "enter");
            let caret = b.state.selection.head().0;
            b.type_at(caret, "two");
        },
    ) {
        assert_eq!(html(&doc), "<blockquote><p>one</p></blockquote>");
    }
}

#[test]
fn a_bullet_list_toggled_to_ordered_keeps_a_peers_concurrent_typing() {
    // The one container retype that stays in place: the two list types take the same
    // children, so the list keeps its identity and a peer's typing in an item merges.
    for doc in concurrently(
        |s| vec![bullets(s, vec!["one", "two"])],
        |a| {
            let s = a.schema.clone();
            let items = (0..2)
                .map(|i| a.state.doc.child(0).child(i).clone())
                .collect();
            replace_first_block(a, branch(&s, "ordered_list", items));
        },
        |b| b.type_after("two", "!"),
    ) {
        assert_eq!(
            html(&doc),
            "<ol><li><p>one</p></li><li><p>two!</p></li></ol>"
        );
    }
}

// --- review of #1229: the void-container paths the first round did not reach -----

#[test]
fn nested_void_list_inside_a_quote() {
    // quote(p0, ul(li a, li b)): A deletes item a, B item b. The list goes void inside
    // the quote; the quote keeps p0. Then edits inside the quote around the void list.
    for ids in [(11u64, 22u64), (22, 11)] {
        let s = schema();
        let mut a = Peer::host(
            &s,
            vec![
                para(&s, "top"),
                quote(
                    &s,
                    vec![
                        para(&s, "p0"),
                        bullets(&s, vec!["aa", "bb"]),
                        para(&s, "p9"),
                    ],
                ),
                para(&s, "end"),
            ],
            ids.0,
        );
        let mut b = a.join(ids.1);
        let mut c = a.join(33);
        let _ = a.send();
        let at = pos_of(&a.state.doc, "aa");
        a.delete(at - 2, at + 2 + 2);
        let at = pos_of(&b.state.doc, "bb");
        b.delete(at - 2, at + 2 + 2);
        let (da, db) = (a.send(), b.send());
        b.receive(&da);
        a.receive(&db);
        c.receive(&da);
        c.receive(&db);
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(
            html(&a.state.doc),
            "<p>top</p><blockquote><p>p0</p><p>p9</p></blockquote><p>end</p>"
        );
        // A inserts a paragraph between p0 and p9 (where the void list sits), B types in p9,
        // C deletes p0.
        let end_p0 = pos_of(&a.state.doc, "p0") + 2;
        a.run(end_p0, "enter");
        let caret = a.state.selection.head().0;
        a.type_at(caret, "mid");
        b.type_after("p9", "!");
        let p0 = pos_of(&c.state.doc, "p0");
        c.delete(p0 - 1, p0 + 3);
        let (da, db, dc) = (a.send(), b.send(), c.send());
        for (p, ds) in [
            (&mut a, [&db, &dc]),
            (&mut b, [&da, &dc]),
            (&mut c, [&da, &db]),
        ] {
            for d in ds {
                p.receive(d);
            }
        }
        assert_eq!(a.state.doc, b.state.doc);
        assert_eq!(a.state.doc, c.state.doc);
        assert_valid(&a.state.doc);
        // sticky inside the quote after the void list
        let at = pos_of(&a.state.doc, "p9");
        let st = a
            .session
            .sticky_index(&a.state.doc, Pos(at + 1))
            .expect("sticky");
        assert_eq!(
            b.session.resolve_sticky(&b.state.doc, &st),
            Some(Pos(at + 1))
        );
        assert_eq!(
            c.session.resolve_sticky(&c.state.doc, &st),
            Some(Pos(at + 1))
        );
        // a late joiner
        let d = a.join(44);
        assert_eq!(d.state.doc, a.state.doc);
    }
}

#[test]
fn cascading_void_quote_holding_only_a_list() {
    // quote(ul(li a, li b)): both items deleted concurrently -> list void -> quote void.
    for doc in concurrently(
        |s| {
            vec![
                para(s, "x"),
                quote(s, vec![bullets(s, vec!["aa", "bb"])]),
                para(s, "y"),
            ]
        },
        |a| {
            let at = pos_of(&a.state.doc, "aa");
            a.delete(at - 2, at + 4);
        },
        |b| {
            let at = pos_of(&b.state.doc, "bb");
            b.delete(at - 2, at + 4);
        },
    ) {
        assert_eq!(html(&doc), "<p>x</p><p>y</p>");
    }
}

#[test]
fn void_list_item_holding_only_a_nested_list() {
    // ul(li(ul(li a, li b))) : list_item whose only child is a list, both nested items
    // deleted concurrently: nested list void -> li void -> outer ul void.
    for doc in concurrently(
        |s| {
            let inner = bullets(s, vec!["aa", "bb"]);
            vec![
                para(s, "x"),
                branch(
                    s,
                    "bullet_list",
                    vec![branch(s, "list_item", vec![para(s, "head"), inner])],
                ),
            ]
        },
        |a| {
            let at = pos_of(&a.state.doc, "aa");
            a.delete(at - 2, at + 4);
        },
        |b| {
            let at = pos_of(&b.state.doc, "bb");
            b.delete(at - 2, at + 4);
        },
    ) {
        assert_eq!(html(&doc), "<p>x</p><ul><li><p>head</p></li></ul>");
    }
}

#[test]
fn insert_between_blocks_after_a_leading_void_container() {
    // CRDT [void quote, p1, p2] / model [p1, p2]: a block inserted between p1 and p2
    // must land between them, not before p1 (kills "insert at the visible index").
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![
            quote(&s, vec![para(&s, "x"), para(&s, "y")]),
            para(&s, "one"),
            para(&s, "two"),
        ],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    let x = pos_of(&a.state.doc, "x");
    a.delete(x - 1, x + 2);
    let y = pos_of(&b.state.doc, "y");
    b.delete(y - 1, y + 2);
    exchange(&mut a, &mut b);
    assert_eq!(html(&a.state.doc), "<p>one</p><p>two</p>");
    let end = pos_of(&a.state.doc, "one") + 3;
    a.run(end, "enter");
    let caret = a.state.selection.head().0;
    a.type_at(caret, "mid");
    exchange(&mut a, &mut b);
    assert_eq!(html(&b.state.doc), "<p>one</p><p>mid</p><p>two</p>");
    assert_eq!(a.state.doc, b.state.doc);
}

#[test]
fn sticky_after_a_cascading_void_container() {
    // quote(ul(a, b)) emptied concurrently: the list is void, so the quote is void too.
    // A sticky index after it must skip the quote (kills a shallow `map_is_void`). Two
    // and three levels deep, so a check that looks only one level down is caught too.
    for depth in [1, 2] {
        let s = schema();
        let mut wrapped = bullets(&s, vec!["aa", "bb"]);
        for _ in 0..depth {
            wrapped = quote(&s, vec![wrapped]);
        }
        let mut a = Peer::host(&s, vec![wrapped, para(&s, "after it")], 1);
        let mut b = a.join(2);
        let _ = a.send();
        let at = pos_of(&a.state.doc, "aa");
        a.delete(at - 2, at + 4);
        let at = pos_of(&b.state.doc, "bb");
        b.delete(at - 2, at + 4);
        exchange(&mut a, &mut b);
        assert_eq!(html(&a.state.doc), "<p>after it</p>", "depth {depth}");
        let at = pos_of(&a.state.doc, "it");
        let sticky = a
            .session
            .sticky_index(&a.state.doc, Pos(at))
            .expect("an index");
        assert_eq!(
            b.session.resolve_sticky(&b.state.doc, &sticky),
            Some(Pos(at)),
            "depth {depth}"
        );
    }
}

#[test]
fn a_peer_joining_after_a_quote_went_void_edits_around_it() {
    // The void quote arrives in the joiner's snapshot rather than in a delta: loading it
    // must find it too, or the joiner's first edit addresses the blocks by the wrong
    // index (its count gate refuses it, and its typing stays local).
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![
            para(&s, "p0"),
            quote(&s, vec![para(&s, "x"), para(&s, "y")]),
            para(&s, "p1"),
        ],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    let x = pos_of(&a.state.doc, "x");
    a.delete(x - 1, x + 2);
    let y = pos_of(&b.state.doc, "y");
    b.delete(y - 1, y + 2);
    exchange(&mut a, &mut b);
    let mut c = a.join(3);
    assert_eq!(html(&c.state.doc), "<p>p0</p><p>p1</p>");
    c.type_after("p1", "!");
    let end_p0 = pos_of(&c.state.doc, "p0") + 2;
    c.run(end_p0, "enter");
    let caret = c.state.selection.head().0;
    c.type_at(caret, "mid");
    let dc = c.send();
    a.receive(&dc);
    b.receive(&dc);
    assert_eq!(html(&a.state.doc), "<p>p0</p><p>mid</p><p>p1!</p>");
    assert_eq!(a.state.doc, b.state.doc);
    assert_eq!(a.state.doc, c.state.doc);
}

#[test]
fn stall_and_heal_with_a_void_container_present() {
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![
            para(&s, "p0"),
            quote(&s, vec![para(&s, "x"), para(&s, "y")]),
            para(&s, "p1"),
        ],
        1,
    );
    let mut b = a.join(2);
    let _ = a.send();
    let x = pos_of(&a.state.doc, "x");
    a.delete(x - 1, x + 2);
    let y = pos_of(&b.state.doc, "y");
    b.delete(y - 1, y + 2);
    exchange(&mut a, &mut b);
    // A appends a task list: refused, stalls.
    let item = s
        .branch("task_item", Fragment::from_node(para(&s, "todo")))
        .unwrap();
    let tl = s.branch("task_list", Fragment::from_node(item)).unwrap();
    let mut tr = a.state.tr();
    let at = a.state.doc.content_size();
    tr.replace(at, at, Slice::new(Fragment::from_node(tl), 0, 0))
        .unwrap();
    let next = a.state.apply(tr);
    assert!(a.session.record_local(&s, &a.state.doc, &next.doc).is_err());
    a.state = next;
    assert!(a.session.outbound_stall().is_some());
    // Type during the stall, in p0 and p1.
    let type_raw = |a: &mut Peer, after: &str, t: &str| {
        let at = pos_of(&a.state.doc, after) + after.len();
        let mut tr = a.state.tr();
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.insert_text(t).unwrap();
        let next = a.state.apply(tr);
        let _ = a.session.record_local(&s, &a.state.doc, &next.doc);
        a.state = next;
    };
    type_raw(&mut a, "p0", "A");
    type_raw(&mut a, "p1", "B");
    assert!(a.session.outbound_stall().is_some());
    // Delete the task list: heals.
    let start: usize = (0..a.state.doc.child_count() - 1)
        .map(|i| a.state.doc.child(i).node_size())
        .sum();
    let end = a.state.doc.content_size();
    let mut tr = a.state.tr();
    tr.delete(start, end).unwrap();
    let next = a.state.apply(tr);
    a.session
        .record_local(&s, &a.state.doc, &next.doc)
        .expect("heals");
    a.state = next;
    assert!(a.session.outbound_stall().is_none());
    a.assert_model_is_projection("after heal");
    let d = a.send();
    b.receive(&d);
    assert_eq!(html(&b.state.doc), "<p>p0A</p><p>p1B</p>");
    assert_eq!(a.state.doc, b.state.doc);
}
