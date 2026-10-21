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
fn a_damaged_trailer_is_refused() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "keep"), para(&s, "gone")]);
    let mut with = peer.sticky_with(8, StickyAssoc::After);
    let cut = with.len() - 1;
    with.truncate(cut);
    // Refused even while the block lives: the bytes are not what was made.
    assert_eq!(peer.locate(&with), None);
    // `resolve_sticky` reads only the yrs index, as yrs alone would.
    assert_eq!(peer.resolve(&with), Some(8));
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

// --- review of #1679 --------------------------------------------------------------

fn blockquote(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("blockquote", Fragment::from_children(blocks))
        .unwrap()
}

impl Peer {
    fn run(&mut self, cmd: &str) {
        let before = self.state.doc.clone();
        let after = self.state.run(cmd).expect("command applies");
        self.session
            .record_local(self.state.schema(), &before, &after.doc)
            .expect("project local");
        self.state = after;
    }
}

/// R1: `locate_sticky`'s docs say `None` "when `doc` is not the document this CRDT
/// projects". A doc whose block text differs from the CRDT's (the model ahead of the
/// CRDT) gives `None` from `resolve_sticky`; does `locate_sticky` agree?
#[test]
fn r1_a_mismatched_model_is_not_reported_displaced() {
    let s = schema();
    let peer = Peer::host(&s, vec![para(&s, "keep"), para(&s, "live")]);
    let with = peer.sticky_with(8, StickyAssoc::After);
    let cdoc = CollabDoc::load(&peer.session.snapshot()).unwrap();
    // The same document with one character typed in the second block, not projected.
    let ahead = doc_of(&s, vec![para(&s, "keep"), para(&s, "liXve")]);
    assert_eq!(cdoc.resolve_sticky(&ahead, &with), None);
    let got = cdoc.locate_sticky(&ahead, &with);
    assert_eq!(got, None, "a live block reported displaced");
}

/// R2: depth. `sticky_index_with` is documented "`None` exactly when
/// `sticky_index` is `None`"; its trailer count is one byte.
#[test]
fn r2_a_deep_block_has_an_index_with_a_trailer() {
    let s = schema();
    let mut node = para(&s, "deep");
    for _ in 0..256 {
        node = blockquote(&s, vec![node]);
    }
    let peer = Peer::host(&s, vec![node]);
    let pos = 256 + 1 + 2;
    assert_eq!(char_at(&peer.state.doc, pos), Some('e'));
    assert!(peer.sticky(pos).is_some(), "plain index exists");
    assert!(
        peer.session
            .sticky_index_with(&peer.state.doc, Pos(pos), StickyAssoc::After)
            .is_some(),
        "sticky_index_with is None where sticky_index is Some"
    );
    let with = peer.sticky_with(pos, StickyAssoc::After);
    assert_eq!(peer.located(&with).0, pos);
}

/// R3: a deletion undone. The block comes back as new CRDT items, so the index stays
/// displaced (documented).
#[test]
fn r3_a_deletion_undone_reports() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "keep"), para(&s, "gone"), para(&s, "x")]);
    let with = peer.sticky_with(8, StickyAssoc::After);
    peer.delete(6, 12);
    peer.run("undo");
    assert_eq!(peer.state.doc.child_count(), 3);
    assert_eq!(peer.located(&with), (5, vec![0], true));
}

/// R4: three peers: A deletes the block, B types in it, C splits it, concurrently.
#[test]
fn r4_three_peers_agree_on_a_displaced_index() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "keep"), para(&s, "gone"), para(&s, "x")]);
    let mut b = Peer::join(&s, &a);
    let mut c = Peer::join(&s, &a);
    let with = a.sticky_with(9, StickyAssoc::After);
    a.delete(6, 12);
    b.type_at(9, "QQ");
    c.local(|tr| {
        tr.split(8, 1, None).unwrap();
    });
    fn bcast(from: &mut Peer, to: [&mut Peer; 2]) {
        let delta = from.session.save_incremental().unwrap();
        for p in to {
            if let Some(next) = p.session.integrate_incremental(&p.state, &delta).unwrap() {
                p.state = next;
            }
        }
    }
    bcast(&mut a, [&mut b, &mut c]);
    bcast(&mut b, [&mut a, &mut c]);
    bcast(&mut c, [&mut a, &mut b]);
    assert_eq!(a.state.doc, b.state.doc);
    assert_eq!(a.state.doc, c.state.doc);
    let la = a.locate(&with);
    assert_eq!(a.located(&with), (5, vec![0], true));
    assert_eq!(la, b.locate(&with));
    assert_eq!(la, c.locate(&with));
}

/// R5: a list toggled on the index's paragraph replaces the block: displaced to the end
/// of the block before (documented).
#[test]
fn r5_list_toggle_displaces() {
    let s = schema();
    let mut a = Peer::host(&s, vec![para(&s, "z"), para(&s, "tog"), para(&s, "t")]);
    let w = a.sticky_with(5, StickyAssoc::After);
    assert_eq!(char_at(&a.state.doc, 5), Some('o'));
    a.local(|tr| {
        tr.set_selection(Selection::cursor(Pos(5)));
    });
    a.run("toggleBulletList");
    assert_eq!(a.state.doc.child(1).type_name(), "bullet_list");
    assert_eq!(a.located(&w), (2, vec![0], true));
}

/// R6: fuzz the trailer: every single-byte change of the trailer, after the block is
/// deleted. Outcomes: None, the right place, or another place.
#[test]
fn r6_trailer_fuzz() {
    let s = schema();
    let mut peer = Peer::host(
        &s,
        vec![
            para(&s, "p0"),
            bullet_list(
                &s,
                vec![
                    list_item(&s, vec![para(&s, "one")]),
                    list_item(&s, vec![para(&s, "two")]),
                    list_item(&s, vec![para(&s, "three")]),
                ],
            ),
            para(&s, "p2"),
            para(&s, "p3"),
        ],
    );
    let pos = 20; // in "three"? find
    let pos = (0..peer.state.doc.content_size())
        .find(|&p| char_at(&peer.state.doc, p) == Some('h'))
        .unwrap_or(pos);
    let with = peer.sticky_with(pos, StickyAssoc::After);
    let yrs_len = StickyIndex::decode_v1(&with).unwrap().encode_v1().len();
    // Delete the list item holding "three".
    let rp = peer.state.doc.resolve(Pos(pos)).unwrap();
    let item_start = rp.before(2).unwrap();
    let item_end = rp.after(2).unwrap();
    peer.delete(item_start, item_end);
    let right = peer.locate(&with);
    assert_eq!(right.as_ref().map(|l| l.displaced), Some(true));
    let (mut none, mut same, mut other, mut panics) = (0, 0, 0, 0);
    for i in yrs_len..with.len() {
        for v in 0..=255u8 {
            if v == with[i] {
                continue;
            }
            let mut m = with.clone();
            m[i] = v;
            let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| peer.locate(&m)));
            let Ok(got) = got else {
                if panics < 3 {
                    eprintln!("R6 PANIC byte {i} = {v}");
                }
                panics += 1;
                continue;
            };
            match got {
                None => none += 1,
                r if r == right => same += 1,
                r => {
                    if other < 5 {
                        eprintln!("R6 byte {i} = {v}: {r:?}");
                    }
                    other += 1
                }
            }
        }
    }
    // Truncations at every length: the yrs index alone is a plain one, whose block
    // is gone.
    for cut in 0..with.len() {
        assert_eq!(peer.locate(&with[..cut]), None, "cut at {cut}");
    }
    // Every change of a trailer byte is refused: nothing lands on another place,
    // nothing panics (yrs asserts a client id fits 53 bits in debug builds).
    assert_eq!((same, other, panics), (0, 0, 0), "none {none}");
}

/// R8: cost. Locate 200 anchors spread over an n-paragraph document, live and
/// displaced. `RV1679_N` sets n.
#[test]
#[ignore]
fn r8_cost_of_locating_many_anchors() {
    let n: usize = std::env::var("RV1679_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let s = schema();
    let blocks: Vec<Node> = (0..n)
        .map(|i| para(&s, &format!("paragraph {i:05}")))
        .collect();
    let mut peer = Peer::host(&s, blocks);
    let size = 2 + "paragraph 00000".len();
    let anchors: Vec<(usize, Vec<u8>, Vec<u8>)> = (0..200)
        .map(|k| {
            let block = k * n / 200;
            let pos = block * size + 3;
            (
                block,
                peer.sticky_with(pos, StickyAssoc::After),
                peer.sticky(pos).unwrap(),
            )
        })
        .collect();
    let t = std::time::Instant::now();
    for (_, w, _) in &anchors {
        assert!(!peer.locate(w).unwrap().displaced);
    }
    let live = t.elapsed();
    let all: Vec<&[u8]> = anchors.iter().map(|(_, w, _)| w.as_slice()).collect();
    let t = std::time::Instant::now();
    let batch = peer.session.locate_sticky_many(&peer.state.doc, &all);
    let live_many = t.elapsed();
    assert!(
        batch
            .iter()
            .all(|l| l.as_ref().is_some_and(|l| !l.displaced))
    );
    let t = std::time::Instant::now();
    for (_, _, p) in &anchors {
        assert!(peer.resolve(p).is_some());
    }
    let plain = t.elapsed();
    // Delete every anchored block, from the end.
    for (block, _, _) in anchors.iter().rev() {
        peer.delete(block * size, (block + 1) * size);
    }
    let t = std::time::Instant::now();
    for (_, w, _) in &anchors {
        assert!(peer.locate(w).unwrap().displaced);
    }
    let gone = t.elapsed();
    let t = std::time::Instant::now();
    let batch = peer.session.locate_sticky_many(&peer.state.doc, &all);
    let many = t.elapsed();
    assert!(
        batch
            .iter()
            .all(|l| l.as_ref().is_some_and(|l| l.displaced))
    );
    eprintln!(
        "R8 n={n}: 200 live locates {live:?}, one batch of 200 live {live_many:?}, 200 plain resolves {plain:?}, 200 displaced locates {gone:?}, one batch of 200 displaced {many:?}"
    );
}

// --- the trailer is checked, and the session guard holds (#1676 review) ----------

fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x0100_0193)
    })
}

/// `bytes` with its trailing hash recomputed, as a deliberate (not accidental) edit.
fn rehash(mut bytes: Vec<u8>) -> Vec<u8> {
    let body = bytes.len() - 4;
    let hash = fnv1a(&bytes[..body]);
    bytes[body..].copy_from_slice(&hash.to_be_bytes());
    bytes
}

/// The length of the yrs index at the start of `bytes`.
fn yrs_len(bytes: &[u8]) -> usize {
    StickyIndex::decode_v1(bytes).unwrap().encode_v1().len()
}

#[test]
fn a_trailer_of_another_version_is_refused() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "keep"), para(&s, "gone")]);
    let with = peer.sticky_with(8, StickyAssoc::After);
    let version_at = yrs_len(&with) + 2;
    assert_eq!(with[version_at], 1);
    // Positive control: the same bytes rehashed unchanged still locate.
    assert_eq!(peer.located(&rehash(with.clone())), (8, vec![1], false));
    let mut v2 = with.clone();
    v2[version_at] = 2;
    let v2 = rehash(v2);
    assert_eq!(peer.locate(&v2), None);
    peer.delete(6, 12);
    assert_eq!(peer.located(&with), (5, vec![0], true));
    assert_eq!(peer.locate(&v2), None);
}

#[test]
fn a_trailer_that_is_not_its_blocks_chain_is_refused() {
    let s = schema();
    let mut peer = Peer::host(
        &s,
        vec![
            para(&s, "keep"),
            bullet_list(&s, vec![list_item(&s, vec![para(&s, "deep")])]),
            para(&s, "flat"),
        ],
    );
    let deep = peer.sticky_with(10, StickyAssoc::After); // `e` of "deep"
    let flat = peer.sticky_with(18, StickyAssoc::After); // `l` of "flat"
    assert_eq!(char_at(&peer.state.doc, 18), Some('l'));
    // `flat`'s yrs index with `deep`'s trailer, rehashed: a chain from another block.
    let mut mixed = flat[..yrs_len(&flat)].to_vec();
    mixed.extend_from_slice(&deep[yrs_len(&deep)..]);
    let mixed = rehash(mixed);
    assert_eq!(
        peer.locate(&mixed),
        None,
        "live: the text is not the chain's block"
    );
    // The batch walk checks the same chain.
    assert_eq!(
        peer.session
            .locate_sticky_many(&peer.state.doc, &[&mixed, &flat]),
        vec![None, peer.locate(&flat)]
    );
    // `deep`'s yrs index with `flat`'s (one-entry) trailer, its block then deleted.
    let mut short = deep[..yrs_len(&deep)].to_vec();
    short.extend_from_slice(&flat[yrs_len(&flat)..]);
    let short = rehash(short);
    peer.delete(6, 16);
    assert_eq!(
        peer.located(&deep),
        (5, vec![0], true),
        "the real chain locates"
    );
    assert_eq!(
        peer.locate(&short),
        None,
        "the entry is not in the root array"
    );
}

#[test]
fn a_client_id_past_53_bits_is_refused_without_a_panic() {
    let s = schema();
    let peer = Peer::host(&s, vec![para(&s, "abc")]);
    // Tag 0 (an item), client 2^53 as LEB128, clock 0, assoc 0.
    let mut bytes = vec![0u8];
    let mut v = 1u64 << 53;
    while v >= 0x80 {
        bytes.push((v as u8) | 0x80);
        v >>= 7;
    }
    bytes.extend_from_slice(&[v as u8, 0, 0]);
    assert_eq!(peer.locate(&bytes), None);
    assert_eq!(peer.resolve(&bytes), None);
}

/// The session refuses while its model is ahead of the CRDT: the paragraph's text
/// still matches the CRDT, so only the guard can say no.
#[test]
fn a_stalled_session_locates_nothing() {
    let s = schema();
    let mut peer = Peer::host(&s, vec![para(&s, "abc")]);
    let with = peer.sticky_with(2, StickyAssoc::After);
    let item = s
        .branch("task_item", Fragment::from_node(para(&s, "q")))
        .unwrap();
    let tasks = s.branch("task_list", Fragment::from_node(item)).unwrap();
    let mut tr = peer.state.tr();
    tr.replace(5, 5, Slice::new(Fragment::from_node(tasks), 0, 0))
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
    assert_eq!(peer.locate(&with), None);
    assert_eq!(
        peer.session
            .locate_sticky_many(&peer.state.doc, &[&with, &with]),
        vec![None, None]
    );
}

/// One walk for many indexes answers what one call per index answers, in order.
#[test]
fn locating_many_at_once_agrees_with_one_at_a_time() {
    let s = schema();
    let mut a = Peer::host(
        &s,
        vec![
            para(&s, "keep"),
            para(&s, "gone"),
            bullet_list(
                &s,
                vec![
                    list_item(&s, vec![para(&s, "one")]),
                    list_item(&s, vec![para(&s, "two")]),
                ],
            ),
            para(&s, "x"),
        ],
    );
    let mut b = Peer::join(&s, &a);
    let anchors: Vec<Vec<u8>> = vec![
        a.sticky_with(2, StickyAssoc::After),
        a.sticky_with(8, StickyAssoc::Before),
        a.sticky_with(16, StickyAssoc::After),
        a.sticky_with(22, StickyAssoc::After),
        a.sticky(8).unwrap(),
        vec![0xff, 0x00],
    ];
    b.delete(6, 12);
    b.type_at(1, "Z");
    b.send_to(&mut a);
    let refs: Vec<&[u8]> = anchors.iter().map(Vec::as_slice).collect();
    let many = a.session.locate_sticky_many(&a.state.doc, &refs);
    let one: Vec<_> = anchors.iter().map(|x| a.locate(x)).collect();
    assert_eq!(many, one);
    assert_eq!(
        many.iter()
            .map(|l| l.as_ref().map(|l| l.displaced))
            .collect::<Vec<_>>(),
        vec![
            Some(false),
            Some(true),
            Some(false),
            Some(false),
            None,
            None
        ]
    );
    let loaded = CollabDoc::load(&a.session.snapshot()).unwrap();
    assert_eq!(loaded.locate_sticky_many_in(&s, &refs).unwrap(), many);
}

/// The yrs index and the trailer's entries of `bytes` (each entry well under 128
/// bytes, so its length is one LEB128 byte; so is the count).
fn split_anchor(bytes: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let head = yrs_len(bytes);
    let body = &bytes[..bytes.len() - 4];
    let count = body[head + 3] as usize;
    let mut at = head + 4;
    let mut entries = Vec::new();
    for _ in 0..count {
        let len = body[at] as usize;
        entries.push(body[at + 1..at + 1 + len].to_vec());
        at += 1 + len;
    }
    (bytes[..head].to_vec(), entries)
}

fn join_anchor(head: &[u8], entries: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = head.to_vec();
    bytes.extend_from_slice(b"RS");
    bytes.push(1);
    bytes.push(entries.len() as u8);
    for e in entries {
        bytes.push(e.len() as u8);
        bytes.extend_from_slice(e);
    }
    bytes.extend_from_slice(&[0; 4]);
    rehash(bytes)
}

/// An entry that names a block's tombstone in an array that is not its own block's
/// next level is refused, even when everything else about it checks out.
#[test]
fn an_entry_from_another_container_breaks_the_chain() {
    let s = schema();
    let list = |a: &str, b: &str| {
        bullet_list(
            &s,
            vec![
                list_item(&s, vec![para(&s, a)]),
                list_item(&s, vec![para(&s, b)]),
            ],
        )
    };
    let mut peer = Peer::host(&s, vec![list("a1", "a2"), list("b1", "b2")]);
    let x = peer.sticky_with(9, StickyAssoc::After);
    let y = peer.sticky_with(23, StickyAssoc::After);
    assert_eq!(char_at(&peer.state.doc, 9), Some('a'));
    assert_eq!(char_at(&peer.state.doc, 23), Some('b'));
    let (x_head, x_entries) = split_anchor(&x);
    let (_, y_entries) = split_anchor(&y);
    assert_eq!((x_entries.len(), y_entries.len()), (3, 3));
    // Round trip of the helpers: the real bytes rebuilt are the real bytes.
    assert_eq!(join_anchor(&x_head, &x_entries), x);
    let crossed = join_anchor(
        &x_head,
        &[
            x_entries[0].clone(),
            y_entries[1].clone(),
            x_entries[2].clone(),
        ],
    );
    peer.delete(7, 13); // the item holding "a2"
    peer.delete(15, 21); // the item holding "b2"
    assert_eq!(
        peer.located(&x),
        (5, vec![0, 0, 0], true),
        "the end of \"a1\""
    );
    assert_eq!(peer.locate(&crossed), None);
}

/// A level in a void container's array must belong to the next entry's container too.
#[test]
fn an_entry_after_a_void_level_must_be_its_container() {
    let s = schema();
    let list = |a: &str, b: &str| {
        bullet_list(
            &s,
            vec![
                list_item(&s, vec![para(&s, a)]),
                list_item(&s, vec![para(&s, b)]),
            ],
        )
    };
    let mut a = Peer::host(
        &s,
        vec![
            para(&s, "z"),
            list("a1", "a2"),
            list("b1", "b2"),
            para(&s, "x"),
        ],
    );
    let mut b = Peer::join(&s, &a);
    let x = a.sticky_with(6, StickyAssoc::After);
    let y = a.sticky_with(20, StickyAssoc::After);
    assert_eq!(char_at(&a.state.doc, 6), Some('a'));
    assert_eq!(char_at(&a.state.doc, 20), Some('b'));
    // Each peer deletes one item of each list: both lists end up void.
    a.delete(4, 10);
    a.delete(12, 18);
    b.delete(10, 16);
    b.delete(18, 24);
    exchange(&mut a, &mut b);
    assert_eq!(a.state.doc.child_count(), 2, "z, x: both lists are void");
    let (x_head, x_entries) = split_anchor(&x);
    let (_, y_entries) = split_anchor(&y);
    let crossed = join_anchor(
        &x_head,
        &[
            x_entries[0].clone(),
            x_entries[1].clone(),
            y_entries[2].clone(),
        ],
    );
    assert!(a.located(&x).2, "the real chain locates, displaced");
    assert_eq!(a.locate(&crossed), None);
}
