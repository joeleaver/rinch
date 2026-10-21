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
    // A task list is outside the projection's scope: outbound stalls (#220). It goes
    // *after* the paragraph, so the paragraph's own path and text still match the
    // CRDT: only the stall itself can say no.
    let item = s
        .branch("task_item", Fragment::from_node(para(&s, "q")))
        .unwrap();
    let quote = s.branch("task_list", Fragment::from_node(item)).unwrap();
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

// --- association, block deletion and headless use (#1676) ------------------------

use rinch_editor_collab::{CollabDoc, StickyAssoc, StickyLocation};

impl Peer {
    fn sticky_with(&self, pos: usize, assoc: StickyAssoc) -> Vec<u8> {
        self.session
            .sticky_index_with(&self.state.doc, Pos(pos), assoc)
            .unwrap()
    }

    fn locate(&self, bytes: &[u8]) -> Option<StickyLocation> {
        self.session.locate_sticky(&self.state.doc, bytes)
    }

    /// The `(pos, path, displaced)` of `bytes` here, asserting it found something.
    fn located(&self, bytes: &[u8]) -> (usize, Vec<usize>, bool) {
        let l = self.locate(bytes).expect("located");
        (l.pos.0, l.path, l.displaced)
    }
}

fn exchange(a: &mut Peer, b: &mut Peer) {
    a.send_to(b);
    b.send_to(a);
}

#[test]
fn the_association_decides_which_side_of_an_insert_the_index_keeps() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "abcd")]);
    // Position 3 is between `b` and `c`.
    let after = peer.sticky_with(3, StickyAssoc::After);
    let before = peer.sticky_with(3, StickyAssoc::Before);
    // The default is `After`, and the yrs bytes are the plain index's.
    assert!(after.starts_with(&peer.sticky(3).unwrap()));
    peer.type_at(3, "XY");
    assert_eq!(peer.located(&after), (5, vec![0], false), "before `c`");
    assert_eq!(char_at(&peer.state.doc, 5), Some('c'));
    assert_eq!(peer.located(&before), (3, vec![0], false), "after `b`");
    // `resolve_sticky` reads the same yrs index and ignores the trailer.
    assert_eq!(peer.resolve(&after), Some(5));
    assert_eq!(peer.resolve(&before), Some(3));
}

#[test]
fn before_at_the_start_of_a_text_stays_at_its_start() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "x"), para(&s, "abc")]);
    let start = peer.sticky_with(4, StickyAssoc::Before);
    let start_after = peer.sticky_with(4, StickyAssoc::After);
    peer.type_at(4, "new ");
    assert_eq!(
        peer.located(&start).0,
        4,
        "Before: what was typed is after it"
    );
    assert_eq!(peer.located(&start_after).0, 8, "After: it stays on `a`");
}

#[test]
fn an_index_whose_block_a_peer_deleted_is_displaced_to_where_the_block_stood() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "keep"), para(&s, "gone"), para(&s, "x")]);
    let mut b = Peer::join(&s, &a);
    let with = a.sticky_with(8, StickyAssoc::After);
    let plain = a.sticky(8).unwrap();

    b.delete(6, 12);
    b.send_to(&mut a);
    for peer in [&a, &b] {
        // The end of "keep", the block before the place "gone" stood.
        assert_eq!(peer.located(&with), (5, vec![0], true));
        assert_eq!(peer.resolve(&with), None, "resolve_sticky keeps its answer");
        assert_eq!(peer.locate(&plain), None, "a plain index has no trailer");
    }
    // Edits after the deletion move the place with the text around it.
    a.type_at(1, "AB");
    assert_eq!(a.located(&with), (7, vec![0], true));
}

#[test]
fn an_index_in_the_first_block_deleted_goes_to_the_start_of_the_next() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "gone"), para(&s, "next")]);
    let with = peer.sticky_with(2, StickyAssoc::Before);
    peer.delete(0, 6);
    assert_eq!(peer.located(&with), (1, vec![0], true));
}

#[test]
fn a_join_displaces_an_index_on_the_joined_text() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "ab"), para(&s, "cd"), para(&s, "e")]);
    let d = peer.sticky_with(6, StickyAssoc::After);
    let b = peer.sticky_with(2, StickyAssoc::After);
    peer.delete(3, 5);
    assert_eq!(peer.state.doc.child_count(), 2);
    // "abcd" is block 0; the place block 1 stood is after it.
    assert_eq!(peer.located(&d), (5, vec![0], true));
    assert_eq!(peer.located(&b), (2, vec![0], false));
}

#[test]
fn a_split_before_the_index_leaves_it_at_the_split_point_not_displaced() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "hello world")]);
    let w = peer.sticky_with(7, StickyAssoc::After);
    peer.local(|tr| {
        tr.set_selection(Selection::cursor(Pos(6)));
        tr.split(6, 1, None).unwrap();
    });
    // Its block lives (it is the first half), so the index is where its char was.
    assert_eq!(peer.located(&w), (6, vec![0], false));
}

#[test]
fn a_block_deleted_while_a_peer_inserts_before_it_is_displaced_alike_on_both() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "keep"), para(&s, "gone"), para(&s, "x")]);
    let mut b = Peer::join(&s, &a);
    let with = a.sticky_with(9, StickyAssoc::After);

    // B inserts a block before "gone" while A deletes "gone".
    b.local(|tr| {
        tr.replace(6, 6, Slice::new(Fragment::from_node(para(&s, "new")), 0, 0))
            .unwrap();
    });
    a.delete(6, 12);
    exchange(&mut a, &mut b);
    assert_eq!(a.state.doc, b.state.doc);
    assert_eq!(a.state.doc.child_count(), 3, "keep, new, x");
    // The inserted block went in before the deleted one: the place is after "new".
    assert_eq!(a.located(&with), (10, vec![1], true));
    assert_eq!(b.located(&with), a.located(&with));
}

#[test]
fn a_concurrent_insert_before_a_live_block_moves_the_index_with_its_char() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "keep"), para(&s, "hold")]);
    let mut b = Peer::join(&s, &a);
    let l = a.sticky_with(10, StickyAssoc::Before); // after `l` of "hold"
    b.local(|tr| {
        tr.replace(6, 6, Slice::new(Fragment::from_node(para(&s, "new")), 0, 0))
            .unwrap();
    });
    a.type_at(7, ">");
    exchange(&mut a, &mut b);
    for peer in [&a, &b] {
        let (pos, path, displaced) = peer.located(&l);
        assert_eq!((path, displaced), (vec![2], false));
        assert_eq!(char_at(&peer.state.doc, pos - 1), Some('l'));
    }
}

#[test]
fn a_deleted_list_item_falls_back_to_its_place_in_the_list() {
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![bullet_list(
            &s,
            vec![
                list_item(&s, vec![para(&s, "one")]),
                list_item(&s, vec![para(&s, "two")]),
            ],
        )],
    );
    let mut b = Peer::join(&s, &a);
    let w = a.sticky_with(11, StickyAssoc::After);
    assert_eq!(char_at(&a.state.doc, 11), Some('w'));
    // The paragraph's own entry goes with the item that held it; the item's entry
    // stays in the list.
    b.delete(8, 15);
    b.send_to(&mut a);
    assert_eq!(
        a.located(&w),
        (6, vec![0, 0, 0], true),
        "the end of \"one\""
    );
}

#[test]
fn a_deleted_container_falls_back_to_the_top_level() {
    let s = schema();
    let mut peer = Peer::host(
        &s,
        vec![
            para(&s, "intro"),
            bullet_list(&s, vec![list_item(&s, vec![para(&s, "deep")])]),
            para(&s, "outro"),
        ],
    );
    let deep = peer.sticky_with(11, StickyAssoc::After);
    let end = peer.state.doc.content_size();
    let list_end = end - 7;
    peer.delete(7, list_end);
    assert_eq!(peer.state.doc.child_count(), 2);
    assert_eq!(
        peer.located(&deep),
        (6, vec![0], true),
        "the end of \"intro\""
    );
}

#[test]
fn an_index_on_an_edit_not_merged_yet_is_not_located() {
    let s = schema();
    let a = Peer::host(&s, vec![para(&s, "abc")]);
    let mut b = Peer::join(&s, &a);
    b.type_at(2, "Z");
    let z = b.sticky_with(2, StickyAssoc::After);
    assert_eq!(
        a.locate(&z),
        None,
        "not displaced: the char is not here yet"
    );
}

#[test]
fn a_damaged_trailer_is_ignored() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "keep"), para(&s, "gone")]);
    let mut with = peer.sticky_with(8, StickyAssoc::After);
    let cut = with.len() - 1;
    with.truncate(cut);
    assert_eq!(peer.located(&with), (8, vec![1], false));
    peer.delete(6, 12);
    assert_eq!(peer.locate(&with), None);
}

#[test]
fn raw_yrs_reads_the_index_in_front_of_the_trailer() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "x"), para(&s, "hello")]);
    let h = peer.sticky_with(4, StickyAssoc::After);
    peer.type_at(4, "😀");
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
    assert_eq!(offset.index, 2);
}

// --- headless: a loaded replica, no editor ---------------------------------------

#[test]
fn an_index_made_on_a_loaded_replica_resolves_in_a_live_session_after_edits() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "intro"), para(&s, "say \"quoted\" here")]);
    // A tool with only the bytes: load, read the model, find the place, make an index.
    let loaded = CollabDoc::load(&a.session.snapshot()).unwrap();
    let doc = loaded.to_doc(&s).unwrap();
    let after_quote = 7 + 1 + "say \"quoted\"".chars().count();
    let bytes = loaded
        .sticky_index_with(&doc, Pos(after_quote), StickyAssoc::Before)
        .unwrap();
    assert_eq!(
        loaded.locate_sticky_in(&s, &bytes).unwrap(),
        a.locate(&bytes),
        "the loaded replica and the session agree before any edit"
    );

    let mut b = Peer::join(&s, &a);
    b.type_at(8, "they ");
    a.type_at(1, "An ");
    exchange(&mut a, &mut b);
    for peer in [&a, &b] {
        let (pos, path, displaced) = peer.located(&bytes);
        assert_eq!((path, displaced), (vec![1], false));
        assert_eq!(char_at(&peer.state.doc, pos - 1), Some('"'));
        assert_eq!(char_at(&peer.state.doc, pos), Some(' '));
    }
}

#[test]
fn an_index_made_in_a_session_resolves_on_a_loaded_replica() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "zero"), para(&s, "one"), para(&s, "two")]);
    let mut b = Peer::join(&s, &a);
    let t = a.sticky_with(13, StickyAssoc::After); // `w` of "two"
    let n = a.sticky_with(8, StickyAssoc::After); // `n` of "one"
    assert_eq!(char_at(&a.state.doc, 13), Some('w'));

    b.local(|tr| {
        tr.replace(0, 0, Slice::new(Fragment::from_node(para(&s, "new")), 0, 0))
            .unwrap();
    });
    b.delete(11, 16); // "one"
    b.send_to(&mut a);

    let loaded = CollabDoc::load(&a.session.snapshot()).unwrap();
    let w = loaded.locate_sticky_in(&s, &t).unwrap().unwrap();
    assert_eq!((w.path.clone(), w.displaced), (vec![2], false));
    assert_eq!(Some(w.clone()), a.locate(&t));
    let gone = loaded.locate_sticky_in(&s, &n).unwrap().unwrap();
    assert_eq!(
        (gone.pos, gone.path, gone.displaced),
        (Pos(10), vec![1], true)
    );
}

/// A table's rows and cells are not arrays and have no entry in the trailer: an index
/// whose row was deleted falls back to the table's own entry, which still lives, and
/// so to the first textblock inside the table.
#[test]
fn a_deleted_table_row_falls_back_into_its_table() {
    let s = schema();
    let cell = |t: &str| {
        s.branch("table_cell", Fragment::from_node(para(&s, t)))
            .unwrap()
    };
    let row = |t: &str| s.branch("table_row", Fragment::from_node(cell(t))).unwrap();
    let table = s
        .branch("table", Fragment::from_children(vec![row("xy"), row("zw")]))
        .unwrap();
    let mut a = Peer::host(&s, vec![para(&s, "a"), table]);
    let mut b = Peer::join(&s, &a);
    let w = a.sticky_with(16, StickyAssoc::After);
    assert_eq!(char_at(&a.state.doc, 16), Some('w'));
    assert_eq!(a.located(&w), (16, vec![1, 1, 0, 0], false));

    b.delete(12, 20);
    b.send_to(&mut a);
    assert_eq!(a.state.doc, b.state.doc);
    // The start of "xy", inside the table that is still there.
    assert_eq!(a.located(&w), (7, vec![1, 0, 0, 0], true));
}

/// A list emptied by two concurrent deletions stays in the CRDT but is not in the
/// model (it is void): the place a later-deleted block stood is counted without it.
#[test]
fn a_void_container_is_not_counted_in_the_place_a_block_stood() {
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![
            para(&s, "z"),
            bullet_list(
                &s,
                vec![
                    list_item(&s, vec![para(&s, "a")]),
                    list_item(&s, vec![para(&s, "b")]),
                ],
            ),
            para(&s, "t"),
            para(&s, "x"),
        ],
    );
    let mut b = Peer::join(&s, &a);
    let t = a.sticky_with(16, StickyAssoc::After);
    assert_eq!(char_at(&a.state.doc, 16), Some('t'));
    a.delete(4, 9);
    b.delete(9, 14);
    exchange(&mut a, &mut b);
    assert_eq!(a.state.doc.child_count(), 3, "z, t, x: the list is void");
    a.delete(3, 6);
    assert_eq!(a.located(&t), (2, vec![0], true), "the end of \"z\"");
}
