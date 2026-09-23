//! Sticky positions (`CollabSession::sticky_index` / `resolve_sticky`): an editor
//! position turned into a plain yrs `StickyIndex` and back, for deep links.
//!
//! Two properties matter to an app. The index follows its character through local
//! edits, a peer's merged edits and its own deletion, and ends (`None`) with its block.
//! And the bytes are exactly what raw yrs produces for the same text and offset, so an
//! app can resolve them without an editor (the last group of tests does that with yrs
//! alone).

use std::rc::Rc;

use rinch_editor_collab::CollabSession;
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Pos, Schema, Selection, Slice, Transaction,
    default_plugins,
};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Array, ArrayRef, Assoc, Doc, GetString, IndexedSequence, Map, MapRef, OffsetKind, Options, Out,
    ReadTxn, StickyIndex, Text, TextRef, Transact, Update,
};

// --- harness -------------------------------------------------------------------

fn schema() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}

fn text(s: &Schema, t: &str) -> Node {
    s.text(t).unwrap()
}

fn para_of(s: &Schema, children: Vec<Node>) -> Node {
    s.branch("paragraph", Fragment::from_children(children))
        .unwrap()
}

fn para(s: &Schema, t: &str) -> Node {
    if t.is_empty() {
        para_of(s, vec![])
    } else {
        para_of(s, vec![text(s, t)])
    }
}

fn heading(s: &Schema, t: &str) -> Node {
    s.create_node(
        "heading",
        Attrs::new().with("level", 2i64),
        Fragment::from_node(text(s, t)),
    )
    .unwrap()
}

fn image(s: &Schema) -> Node {
    s.create_node(
        "image",
        Attrs::new().with("src", AttrValue::from("cat.png")),
        Fragment::empty(),
    )
    .unwrap()
}

fn hard_break(s: &Schema) -> Node {
    s.branch("hard_break", Fragment::empty()).unwrap()
}

fn list_item(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("list_item", Fragment::from_children(blocks))
        .unwrap()
}

fn bullet_list(s: &Schema, items: Vec<Node>) -> Node {
    s.branch("bullet_list", Fragment::from_children(items))
        .unwrap()
}

fn ordered_list(s: &Schema, items: Vec<Node>) -> Node {
    s.create_node(
        "ordered_list",
        Attrs::new().with("start", 1i64),
        Fragment::from_children(items),
    )
    .unwrap()
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

/// One collaborating editor: its model state plus its CRDT session.
struct Peer {
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn host(s: &Rc<Schema>, blocks: Vec<Node>) -> Peer {
        let state = EditorState::create(s.clone(), doc_of(s, blocks), default_plugins());
        let session = CollabSession::new(&state).unwrap();
        Peer { state, session }
    }

    fn join(s: &Rc<Schema>, host: &Peer) -> Peer {
        let session = CollabSession::from_bytes(&host.session.snapshot()).unwrap();
        let doc = session.projected_doc(s).unwrap();
        Peer {
            state: EditorState::create(s.clone(), doc, default_plugins()),
            session,
        }
    }

    fn local(&mut self, f: impl FnOnce(&mut Transaction)) {
        let mut tr = self.state.tr();
        f(&mut tr);
        let before = self.state.doc.clone();
        let after = self.state.apply(tr);
        self.session
            .record_local(self.state.schema(), &before, &after.doc)
            .expect("project local");
        self.state = after;
    }

    fn type_at(&mut self, pos: usize, t: &str) {
        self.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(pos)));
            tr.insert_text(t).unwrap();
        });
    }

    fn delete(&mut self, from: usize, to: usize) {
        self.local(|tr| {
            tr.replace(from, to, Slice::empty()).unwrap();
        });
    }

    fn sticky(&self, pos: usize) -> Option<Vec<u8>> {
        self.session.sticky_index(&self.state.doc, Pos(pos))
    }

    fn resolve(&self, bytes: &[u8]) -> Option<usize> {
        self.session
            .resolve_sticky(&self.state.doc, bytes)
            .map(|p| p.0)
    }

    /// Deliver everything `self` has not broadcast yet to `to`.
    fn send_to(&mut self, to: &mut Peer) {
        let delta = self.session.save_incremental().unwrap();
        if let Some(next) = to.session.integrate_incremental(&to.state, &delta).unwrap() {
            to.state = next;
        }
    }
}

/// The char that starts at model position `pos` (an inline atom reads as U+FFFC), or
/// `None` at the end of a textblock.
fn char_at(doc: &Node, pos: usize) -> Option<char> {
    let rp = doc.resolve(Pos(pos)).unwrap();
    let block = rp.parent();
    let mut offset = rp.parent_offset();
    for i in 0..block.child_count() {
        let child = block.child(i);
        let len = child.text().map_or(1, |t| t.chars().count());
        if offset < len {
            return Some(
                child
                    .text()
                    .map_or('\u{FFFC}', |t| t.chars().nth(offset).unwrap()),
            );
        }
        offset -= len;
    }
    None
}

/// Every position of `peer`'s document: a position in a textblock round-trips, every
/// other position has no sticky index. Returns how many round-tripped.
fn assert_every_position_round_trips(peer: &Peer) -> usize {
    let doc = &peer.state.doc;
    let mut in_text = 0;
    for pos in 0..=doc.content_size() {
        let textblock = doc.resolve(Pos(pos)).unwrap().parent().is_textblock();
        match peer.sticky(pos) {
            Some(bytes) => {
                assert!(textblock, "position {pos} is not in a textblock");
                assert_eq!(peer.resolve(&bytes), Some(pos), "round trip at {pos}");
                in_text += 1;
            }
            None => assert!(!textblock, "no sticky index at {pos}, in a textblock"),
        }
    }
    in_text
}

// --- round trips ---------------------------------------------------------------

#[test]
fn every_position_in_flat_blocks_round_trips() {
    let s = schema();
    // Start, middle and end of a paragraph, an empty paragraph, a heading with a
    // non-ASCII letter.
    let peer = Peer::host(
        &s,
        vec![para(&s, "hello"), para(&s, ""), heading(&s, "wörld")],
    );
    // 6 + 1 + 6 positions inside the three textblocks.
    assert_eq!(assert_every_position_round_trips(&peer), 13);
    // Before the first block and between blocks: not inside a textblock.
    assert_eq!(peer.sticky(0), None);
    assert_eq!(peer.sticky(7), None);
}

#[test]
fn every_position_in_nested_list_items_round_trips() {
    let s = schema();
    let peer = Peer::host(
        &s,
        vec![
            para(&s, "intro"),
            bullet_list(
                &s,
                vec![
                    list_item(
                        &s,
                        vec![
                            para(&s, "one"),
                            ordered_list(&s, vec![list_item(&s, vec![para(&s, "deep")])]),
                        ],
                    ),
                    list_item(&s, vec![para(&s, "")]),
                ],
            ),
        ],
    );
    // "intro" 6 + "one" 4 + "deep" 5 + "" 1.
    assert_eq!(assert_every_position_round_trips(&peer), 16);
}

#[test]
fn positions_around_inline_atoms_round_trip() {
    let s = schema();
    let peer = Peer::host(
        &s,
        vec![para_of(
            &s,
            vec![
                text(&s, "ab"),
                image(&s),
                text(&s, "cd"),
                hard_break(&s),
                text(&s, "e"),
            ],
        )],
    );
    // 7 inline positions (each atom is one), so 8 caret positions.
    assert_eq!(assert_every_position_round_trips(&peer), 8);
    // The index on the char after the image is on `c`, and stays there when a char is
    // typed before the image.
    let mut peer = peer;
    let c = peer.sticky(4).unwrap();
    peer.type_at(1, "X");
    assert_eq!(peer.resolve(&c), Some(5));
    assert_eq!(char_at(&peer.state.doc, 5), Some('c'));
}

#[test]
fn an_emoji_before_the_position_counts_as_one_position() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "😀😀ab")]);
    // `a` is model position 3 (content starts at 1), UTF-16 offset 4.
    let a = peer.sticky(3).unwrap();
    assert_eq!(peer.resolve(&a), Some(3));
    peer.type_at(1, "🎉");
    assert_eq!(peer.resolve(&a), Some(4));
    assert_eq!(char_at(&peer.state.doc, 4), Some('a'));
}

// --- following the text --------------------------------------------------------

#[test]
fn a_sticky_index_follows_its_character_through_local_edits_before_it() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "first"), para(&s, "hello world")]);
    // `w` of "world": block 1 starts at 7, its content at 8, `w` is the 7th char.
    let w = peer.sticky(14).unwrap();
    assert_eq!(char_at(&peer.state.doc, 14), Some('w'));

    peer.type_at(8, "Oh, "); // before it in its own block
    peer.type_at(1, "the "); // in the block before
    peer.local(|tr| {
        // A whole new block before both.
        tr.replace(0, 0, Slice::new(Fragment::from_node(para(&s, "new")), 0, 0))
            .unwrap();
    });
    peer.delete(1, 3); // and a deletion before it ("ne" of "new")

    let now = peer.resolve(&w).unwrap();
    assert_eq!(now, 14 + 4 + 4 + 5 - 2);
    assert_eq!(char_at(&peer.state.doc, now), Some('w'));
}

#[test]
fn a_sticky_index_follows_its_character_through_a_peers_merged_edits() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "hello world")]);
    let mut b = Peer::join(&s, &a);
    let w = a.sticky(7).unwrap();

    b.type_at(1, "well, ");
    b.send_to(&mut a);
    assert_eq!(a.resolve(&w), Some(13));
    assert_eq!(char_at(&a.state.doc, 13), Some('w'));
    // The same bytes on the other replica name the same character.
    assert_eq!(b.resolve(&w), Some(13));

    // A concurrent edit on each side, then both merged.
    a.type_at(1, "A");
    b.type_at(13, "B");
    a.send_to(&mut b);
    b.send_to(&mut a);
    for peer in [&a, &b] {
        let now = peer.resolve(&w).unwrap();
        assert_eq!(char_at(&peer.state.doc, now), Some('w'));
    }
    assert_eq!(a.resolve(&w), b.resolve(&w));
}

#[test]
fn a_sticky_index_on_a_deleted_character_resolves_to_where_it_was() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "hello world")]);
    let w = peer.sticky(7).unwrap();
    peer.delete(7, 8); // the `w` itself
    assert_eq!(peer.resolve(&w), Some(7));
    assert_eq!(char_at(&peer.state.doc, 7), Some('o'));
    // And it keeps following the text from there.
    peer.type_at(1, "ah ");
    assert_eq!(peer.resolve(&w), Some(10));
}

#[test]
fn a_sticky_index_at_the_end_of_a_text_stays_at_its_end() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "abc")]);
    let end = peer.sticky(4).unwrap();
    peer.type_at(1, "x");
    assert_eq!(peer.resolve(&end), Some(5));
    assert_eq!(char_at(&peer.state.doc, 5), None, "still at the end");
}

#[test]
fn a_sticky_index_in_an_empty_block_resolves_to_its_start() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "a"), para(&s, "")]);
    let empty = peer.sticky(4).unwrap();
    peer.type_at(1, "bc");
    assert_eq!(peer.resolve(&empty), Some(6));
    peer.type_at(6, "typed");
    assert_eq!(peer.resolve(&empty), Some(6), "the start of what was typed");
}

#[test]
fn a_sticky_index_ends_with_its_block() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "keep"), para(&s, "gone"), para(&s, "x")]);
    let mut b = Peer::join(&s, &a);
    let in_gone = a.sticky(8).unwrap();
    let in_keep = a.sticky(2).unwrap();

    // The peer deletes the whole block; A merges it.
    b.delete(6, 12);
    b.send_to(&mut a);
    assert_eq!(a.resolve(&in_gone), None);
    assert_eq!(b.resolve(&in_gone), None);
    assert_eq!(a.resolve(&in_keep), Some(2), "the others are unaffected");
}

/// Splitting a block moves the tail's characters into a new block's text (the
/// projection has no "move"), so an index on them lands on the split point. Pinned
/// because the module docs promise it.
#[test]
fn a_split_before_the_position_leaves_the_index_at_the_split_point() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "hello world")]);
    let w = peer.sticky(7).unwrap();
    peer.local(|tr| {
        tr.set_selection(Selection::cursor(Pos(6)));
        tr.split(6, 1, None).unwrap();
    });
    assert_eq!(peer.state.doc.child_count(), 2);
    assert_eq!(peer.resolve(&w), Some(6), "the end of \"hello\"");
}

/// Joining a block into the one before it (Backspace at its start) writes its text
/// into the first block as a new insert and deletes the second block, so an index on
/// the joined text ends. Pinned because the docs promise it.
#[test]
fn a_join_ends_an_index_on_the_joined_text() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "ab"), para(&s, "cd")]);
    let d = peer.sticky(6).unwrap();
    let b = peer.sticky(2).unwrap();
    peer.delete(3, 5);
    assert_eq!(peer.state.doc.child_count(), 1);
    assert_eq!(peer.resolve(&d), None);
    assert_eq!(
        peer.resolve(&b),
        Some(2),
        "the first block's own text is kept"
    );
}

// --- when there is nothing to name ---------------------------------------------

#[test]
fn garbage_and_foreign_bytes_resolve_to_nothing() {
    let s = schema();
    let peer = Peer::host(&s, vec![para(&s, "abc")]);
    assert_eq!(peer.resolve(&[]), None);
    assert_eq!(peer.resolve(&[0xff, 0xff, 0xff]), None);
    // A sticky index from an unrelated document names an item this one never saw.
    let other = Peer::host(&s, vec![para(&s, "abc")]);
    let foreign = other.sticky(2).unwrap();
    assert_eq!(peer.resolve(&foreign), None);
    // A sticky index on the root `content` array is not a textblock's text.
    let doc = raw_replica(&peer);
    let content = doc.get_or_insert_array("content");
    let txn = doc.transact();
    let on_root = content
        .sticky_index(&txn, 0, Assoc::After)
        .unwrap()
        .encode_v1();
    assert_eq!(peer.resolve(&on_root), None);
}

#[test]
fn a_crdt_with_zero_blocks_has_no_sticky_index_for_the_starter_paragraph() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "one"), para(&s, "two")]);
    let mut b = Peer::join(&s, &a);
    // Each deletes a different block: together they delete both (issue #192).
    a.delete(0, 5);
    b.delete(5, 10);
    a.send_to(&mut b);
    b.send_to(&mut a);
    assert_eq!(a.state.doc.child_count(), 1, "the starter paragraph");
    assert_eq!(a.sticky(1), None);
}

#[test]
fn a_model_that_is_not_the_projected_one_gets_nothing() {
    let s = schema();
    let peer = Peer::host(&s, vec![para(&s, "abc")]);
    let bytes = peer.sticky(2).unwrap();
    let other = doc_of(&s, vec![para(&s, "xyz")]);
    assert_eq!(peer.session.sticky_index(&other, Pos(2)), None);
    assert_eq!(peer.session.resolve_sticky(&other, &bytes), None);
    let heading_doc = doc_of(&s, vec![heading(&s, "abc")]);
    assert_eq!(peer.session.sticky_index(&heading_doc, Pos(2)), None);
}

#[test]
fn a_stalled_session_gives_no_sticky_index() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "abc")]);
    let bytes = peer.sticky(2).unwrap();
    // A blockquote is outside the projection's scope: outbound stalls (#220). It goes
    // *after* the paragraph, so the paragraph's own path and text still match the
    // CRDT: only the stall itself can say no.
    let quote = s
        .branch("blockquote", Fragment::from_node(para(&s, "q")))
        .unwrap();
    let mut tr = peer.state.tr();
    tr.replace(5, 5, Slice::new(Fragment::from_node(quote), 0, 0))
        .unwrap();
    let before = peer.state.doc.clone();
    let after = peer.state.apply(tr);
    assert!(
        peer.session
            .record_local(after.schema(), &before, &after.doc)
            .is_err()
    );
    peer.state = after;
    assert!(peer.session.outbound_stall().is_some());
    assert_eq!(peer.sticky(2), None);
    assert_eq!(peer.resolve(&bytes), None);
}

#[test]
fn a_poisoned_session_gives_no_sticky_index() {
    // Interior damage with shared lineage (the #196 shape from `tests/poison.rs`): a
    // peer parks an embed inside block 0's text, which poisons the session. Block 1 is
    // untouched, so its path and text still match the CRDT: only the poison guard can
    // say no.
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "alpha"), para(&s, "beta")]);
    let _ = peer.session.save_incremental().unwrap();
    assert_eq!(char_at(&peer.state.doc, 9), Some('e'));
    let bytes = peer.sticky(9).unwrap();

    let foreign = Doc::new();
    foreign
        .transact_mut()
        .apply_update(Update::decode_v1(&peer.session.snapshot()).unwrap())
        .unwrap();
    {
        let content = foreign.get_or_insert_array("content");
        let mut txn = foreign.transact_mut();
        let text = raw_text(&txn, &content, 0);
        text.insert_embed(&mut txn, 2, yrs::Any::Bool(true));
    }
    let delta = {
        let sv = yrs::StateVector::decode_v1(&peer.session.state_vector()).unwrap();
        foreign.transact().encode_diff_v1(&sv)
    };
    assert!(
        peer.session
            .integrate_incremental(&peer.state, &delta)
            .is_err()
    );
    assert!(peer.session.is_poisoned());
    assert_eq!(peer.sticky(9), None);
    assert_eq!(peer.resolve(&bytes), None);
}

// --- the bytes are plain yrs ---------------------------------------------------

/// A raw yrs replica of `peer`'s CRDT, built the way the adapter builds its own.
fn raw_replica(peer: &Peer) -> Doc {
    let doc = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Default::default()
    });
    let update = Update::decode_v1(&peer.session.snapshot()).unwrap();
    doc.transact_mut().apply_update(update).unwrap();
    doc
}

/// The `text` of the `index`-th block under the `content` root, with yrs alone.
fn raw_text<T: ReadTxn>(txn: &T, content: &ArrayRef, index: u32) -> TextRef {
    let Some(Out::YMap(node)) = content.get(txn, index) else {
        panic!("block {index}");
    };
    let node: MapRef = node;
    let Some(Out::YText(text)) = node.get(txn, "text") else {
        panic!("text of block {index}");
    };
    text
}

#[test]
fn the_bytes_are_what_raw_yrs_encodes_for_the_same_text_and_offset() {
    let s = schema();
    let peer = Peer::host(&s, vec![para(&s, "😀ab"), para(&s, "")]);
    let doc = raw_replica(&peer);
    let content = doc.get_or_insert_array("content");
    let txn = doc.transact();
    let text = raw_text(&txn, &content, 0);
    assert_eq!(text.get_string(&txn), "😀ab");
    let empty = raw_text(&txn, &content, 1);
    let raw =
        |t: &TextRef, at: u32, assoc: Assoc| t.sticky_index(&txn, at, assoc).unwrap().encode_v1();

    // Model positions 1..=4 are UTF-16 offsets 0, 2, 3, 4 of the first text.
    assert_eq!(peer.sticky(1).unwrap(), raw(&text, 0, Assoc::After));
    assert_eq!(peer.sticky(2).unwrap(), raw(&text, 2, Assoc::After));
    assert_eq!(peer.sticky(3).unwrap(), raw(&text, 3, Assoc::After));
    // The end of a text sticks to its last char.
    assert_eq!(peer.sticky(4).unwrap(), raw(&text, 4, Assoc::Before));
    // An empty text names the text itself.
    let in_empty = peer.sticky(6).unwrap();
    assert_eq!(in_empty, raw(&empty, 0, Assoc::Before));
    assert!(StickyIndex::decode_v1(&in_empty).unwrap().is_nested());
}

/// What an app does with the bytes and yrs alone: decode, `get_offset`, check the
/// branch is a textblock's text. It must agree with the editor.
#[test]
fn an_external_resolver_with_yrs_alone_agrees_with_the_editor() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "x"), para(&s, "😀 hello")]);
    let h = peer.sticky(6).unwrap(); // `h`: block 1's content starts at 4
    assert_eq!(char_at(&peer.state.doc, 6), Some('h'));
    peer.type_at(4, "🎉");

    let doc = raw_replica(&peer);
    let content = doc.get_or_insert_array("content");
    let txn = doc.transact();
    let offset = StickyIndex::decode_v1(&h)
        .unwrap()
        .get_offset(&txn)
        .unwrap();
    let text = raw_text(&txn, &content, 1);
    assert!(
        offset.branch == yrs::branch::BranchPtr::from(AsRef::<yrs::branch::Branch>::as_ref(&text))
    );
    // "🎉😀 " is 5 UTF-16 units; the editor says model position 7 (4 + 3 chars).
    assert_eq!(offset.index, 5);
    assert_eq!(peer.resolve(&h), Some(7));
    assert_eq!(char_at(&peer.state.doc, 7), Some('h'));
}

/// The resolver's `Doc` must be built with `OffsetKind::Utf16`: the offset kind is per
/// `Doc`, not in the bytes, and a default `Doc::new()` (`OffsetKind::Bytes`) answers
/// `get_offset` in another unit once there is an edited item to the left. This is the
/// construction the guide and the rustdoc tell an outside resolver to use.
#[test]
fn an_external_resolver_needs_a_utf16_doc() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "hello")]);
    let h = peer.sticky(1).unwrap();
    peer.type_at(1, "😀ö ");
    assert_eq!(peer.resolve(&h), Some(4)); // 1 + three chars
    assert_eq!(char_at(&peer.state.doc, 4), Some('h'));

    let offset_in = |doc: &Doc| {
        let txn = doc.transact();
        StickyIndex::decode_v1(&h)
            .unwrap()
            .get_offset(&txn)
            .unwrap()
            .index
    };
    // "😀ö " is 4 UTF-16 code units.
    assert_eq!(offset_in(&raw_replica(&peer)), 4);

    let default_doc = Doc::new();
    default_doc
        .transact_mut()
        .apply_update(Update::decode_v1(&peer.session.snapshot()).unwrap())
        .unwrap();
    assert_ne!(
        offset_in(&default_doc),
        4,
        "a default Doc must not be mistaken for a UTF-16 resolver"
    );
}
