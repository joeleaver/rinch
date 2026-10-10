//! Per-attribute merge of an inline atom's attrs, with the attributes an app adds to
//! an image: `board` (an id the app keeps what it draws over the picture under) and
//! `width`. An app that stores something under `board` must never see the id lost to a
//! peer's concurrent edit of the same image, or of the line around it.
//!
//! The schema here is the starter kit's own shape cut to what the tests need, with an
//! `image` that also carries `board` and `width`; and, for documents written before an
//! app added them, the same image without them.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrSpec, AttrValue, Attrs, EditorState, Fragment, Node, NodeSpec, Plugin, Pos, Schema,
    Selection, SetNodeAttrStep, Slice, Transaction, default_plugins,
};

const SRC: &str = "pimble-blob:6f1c2a0e/b3-9f86d081884c7d65";
const ID_ORDERS: [(u64, u64); 2] = [(11, 22), (22, 11)];

/// `doc > paragraph+ > (text | image | hard_break)*`, with `image` carrying `src`,
/// `alt`, `title` and, when `app` is set, `board` and `width`.
fn schema(app: bool) -> Rc<Schema> {
    let mut b = Schema::builder();
    b = b.node("doc", NodeSpec::builder("doc").content("block+").build());
    b = b.node(
        "paragraph",
        NodeSpec::builder("paragraph")
            .content("inline*")
            .group("block")
            .build(),
    );
    b = b.node(
        "text",
        NodeSpec::builder("text").group("inline").inline().build(),
    );
    b = b.node("hard_break", {
        let mut spec = NodeSpec::atom("hard_break");
        spec.group = Some("inline".into());
        spec.inline = true;
        spec
    });
    b = b.node("image", {
        let mut spec = NodeSpec::atom("image");
        spec.group = Some("inline".into());
        spec.inline = true;
        spec.attrs.insert("src".into(), AttrSpec::required());
        spec.attrs.insert("alt".into(), AttrSpec::optional(""));
        spec.attrs.insert("title".into(), AttrSpec::optional(""));
        if app {
            spec.attrs.insert("board".into(), AttrSpec::optional(""));
            spec.attrs.insert("width".into(), AttrSpec::optional(0i64));
        }
        spec
    });
    Rc::new(b.build())
}

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn image(s: &Schema, src: &str, alt: &str) -> Node {
    s.create_node(
        "image",
        Attrs::new()
            .with("src", AttrValue::from(src))
            .with("alt", AttrValue::from(alt)),
        Fragment::empty(),
    )
    .unwrap()
}

fn para(s: &Schema, children: Vec<Node>) -> Node {
    s.branch("paragraph", Fragment::from_children(children))
        .unwrap()
}

/// `ab` + an image + `cd` (the image at model position 3), and, when `second` is set,
/// a second line `ef` + another image + `gh` (that image at position 10).
fn document(s: &Schema, second: bool) -> Node {
    let mut blocks = vec![para(
        s,
        vec![
            s.text("ab").unwrap(),
            image(s, SRC, "old alt"),
            s.text("cd").unwrap(),
        ],
    )];
    if second {
        blocks.push(para(
            s,
            vec![
                s.text("ef").unwrap(),
                image(s, "pimble-blob:two", "second"),
                s.text("gh").unwrap(),
            ],
        ));
    }
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

struct Peer {
    state: EditorState,
    session: CollabSession,
}

impl Peer {
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

    /// Set attr `name` of the `nth` image in the document, wherever it now is.
    fn set_nth(&mut self, nth: usize, name: &str, value: AttrValue) {
        let at = image_positions(&self.state.doc)[nth];
        self.local(|tr| {
            tr.step(Box::new(SetNodeAttrStep::new(at, name, value)))
                .unwrap();
        });
    }

    fn set(&mut self, name: &str, value: impl Into<AttrValue>) {
        self.set_nth(0, name, value.into());
    }

    fn type_at(&mut self, at: usize, text: &str) {
        self.local(|tr| {
            tr.set_selection(Selection::cursor(Pos(at)));
            tr.insert_text(text).unwrap();
        });
    }

    fn enter_at(&mut self, at: usize) {
        self.local(|tr| {
            tr.split(at, 1, None).unwrap();
        });
    }
}

/// Every image's position, in document order.
fn image_positions(doc: &Node) -> Vec<usize> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
        if n.type_name() == "image" {
            out.push(pos);
        }
        true
    });
    out
}

/// Every image's attrs, in document order.
fn images(doc: &Node) -> Vec<Attrs> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, _, _| {
        if n.type_name() == "image" {
            out.push(n.attrs().clone());
        }
        true
    });
    out
}

fn attr(a: &Attrs, k: &str) -> String {
    match a.get(k) {
        Some(AttrValue::Str(s)) => s.to_string(),
        Some(AttrValue::Int(i)) => i.to_string(),
        Some(other) => format!("{other:?}"),
        // Absent: an attr the image was never given reads as nothing.
        None => String::new(),
    }
}

fn two_peers(schema: &Rc<Schema>, doc: Node, ids: (u64, u64)) -> (Peer, Peer) {
    let a_state = EditorState::create(schema.clone(), doc, plugins());
    let session_a = session_with_client_id(&a_state, ids.0).expect("session A");
    let b = peer_from_bytes(schema, &session_a.snapshot(), ids.1);
    (
        Peer {
            state: a_state,
            session: session_a,
        },
        b,
    )
}

fn peer_from_bytes(schema: &Rc<Schema>, bytes: &[u8], id: u64) -> Peer {
    let session = session_from_bytes_with_client_id(bytes, id).expect("join");
    let doc = session.projected_doc(schema).expect("project");
    Peer {
        state: EditorState::create(schema.clone(), doc, plugins()),
        session,
    }
}

/// Exchange state vectors and diffs until neither side changes.
fn sync(a: &mut Peer, b: &mut Peer) {
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
            return;
        }
    }
    panic!("the state-vector exchange did not settle");
}

/// Both peers hold one document, each what its CRDT reads as, and so does a peer
/// joining now from either's snapshot. Returns the images.
fn converged(a: &Peer, b: &Peer, schema: &Rc<Schema>) -> Vec<Attrs> {
    assert_eq!(a.state.doc, b.state.doc, "the peers diverged");
    for peer in [a, b] {
        assert_eq!(
            peer.state.doc,
            peer.session.projected_doc(schema).unwrap(),
            "a peer's document is not what its CRDT holds"
        );
    }
    let late = CollabSession::from_bytes(&b.session.snapshot()).unwrap();
    assert_eq!(late.projected_doc(schema).unwrap(), a.state.doc);
    images(&a.state.doc)
}

// --- board against every concurrent edit of the image and its line -----------------

#[test]
fn a_board_and_a_concurrent_alt_change_both_survive() {
    for ids in ID_ORDERS {
        let s = schema(true);
        let (mut a, mut b) = two_peers(&s, document(&s, false), ids);
        a.set("board", "01JA7Z3QK2V9");
        b.set("alt", "new alt");
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        assert_eq!(imgs.len(), 1);
        assert_eq!(attr(&imgs[0], "board"), "01JA7Z3QK2V9", "{ids:?}");
        assert_eq!(attr(&imgs[0], "alt"), "new alt", "{ids:?}");
        assert_eq!(attr(&imgs[0], "src"), SRC, "{ids:?}");
    }
}

#[test]
fn a_board_and_a_concurrent_width_change_both_survive() {
    for ids in ID_ORDERS {
        let s = schema(true);
        let (mut a, mut b) = two_peers(&s, document(&s, false), ids);
        a.set("board", "b1");
        b.set("width", 320i64);
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        assert_eq!(
            (attr(&imgs[0], "board"), attr(&imgs[0], "width")),
            ("b1".to_string(), "320".to_string()),
            "{ids:?}"
        );
    }
}

/// Enter inside the text before the image, right before it, right after it and at the
/// end of the line, while a peer marks the picture up: the board is kept, and there is
/// one image.
#[test]
fn a_board_survives_enter_anywhere_in_the_images_line() {
    for ids in ID_ORDERS {
        for at in [2, 3, 4, 6] {
            let s = schema(true);
            let (mut a, mut b) = two_peers(&s, document(&s, false), ids);
            a.set("board", "b1");
            b.enter_at(at);
            sync(&mut a, &mut b);
            let imgs = converged(&a, &b, &s);
            assert_eq!(imgs.len(), 1, "{ids:?} Enter at {at}");
            assert_eq!(attr(&imgs[0], "board"), "b1", "{ids:?} Enter at {at}");
            assert_eq!(attr(&imgs[0], "alt"), "old alt", "{ids:?} Enter at {at}");
        }
    }
}

/// The other way round: the peer presses Enter first and syncs, then both edit at once
/// (Backspace joining the line back, and a board): kept.
#[test]
fn a_board_survives_a_join_that_moves_the_image_into_the_line_above() {
    for ids in ID_ORDERS {
        let s = schema(true);
        let (mut a, mut b) = two_peers(&s, document(&s, true), ids);
        // The second line's image is at 10; Backspace at the start of that line joins
        // it into the first, which moves the image's char into the first line's text.
        a.set_nth(1, "board", AttrValue::from("b2"));
        b.local(|tr| {
            tr.delete(6, 8).unwrap();
        });
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        assert_eq!(a.state.doc.child_count(), 1, "{ids:?}: joined");
        assert_eq!(imgs.len(), 2, "{ids:?}");
        assert_eq!(attr(&imgs[1], "board"), "b2", "{ids:?}");
        assert_eq!(attr(&imgs[1], "alt"), "second", "{ids:?}");
        assert_eq!(
            attr(&imgs[0], "board"),
            "",
            "{ids:?}: the other image untouched"
        );
    }
}

/// A peer cuts the image and pastes it at the end of the line in one transaction (a
/// drag), while another marks it up: the board follows the picture.
#[test]
fn a_board_follows_an_image_moved_in_one_transaction() {
    for ids in ID_ORDERS {
        let s = schema(true);
        let (mut a, mut b) = two_peers(&s, document(&s, true), ids);
        a.set("board", "b1");
        b.local(|tr| {
            let img = tr.doc().child(0).child(1).clone();
            tr.delete(3, 4).unwrap();
            // The second line's end: `ef` + image + `gh` closes at 12 after the delete.
            tr.replace(12, 12, Slice::new(Fragment::from_node(img), 0, 0))
                .unwrap();
        });
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        assert_eq!(imgs.len(), 2, "{ids:?}: {:?}", a.state.doc);
        assert_eq!(
            attr(&imgs[1], "src"),
            SRC,
            "{ids:?}: moved to the second line"
        );
        assert_eq!(attr(&imgs[1], "board"), "b1", "{ids:?}");
    }
}

/// Two peers marking one picture up at once both mint a board id; the replicas
/// converge on one of them (the higher client id's, yrs's rule for one map key).
#[test]
fn two_boards_written_at_once_converge_on_the_higher_client_ids() {
    for ids in ID_ORDERS {
        let s = schema(true);
        let (mut a, mut b) = two_peers(&s, document(&s, false), ids);
        a.set("board", "from A");
        b.set("board", "from B");
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        let expected = if ids.0 > ids.1 { "from A" } else { "from B" };
        assert_eq!(attr(&imgs[0], "board"), expected, "{ids:?}");
    }
}

/// Both peers press Enter before one image at once, so each writes a copy of the line's
/// tail (yrs has no move): two images, both carrying the first one's identity. A
/// change to one of them afterwards is its own: the other copy keeps its attrs.
#[test]
fn two_copies_from_concurrent_splits_are_edited_apart() {
    for ids in ID_ORDERS {
        let s = schema(true);
        let (mut a, mut b) = two_peers(&s, document(&s, false), ids);
        a.enter_at(3);
        b.enter_at(2);
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        assert_eq!(imgs.len(), 2, "{ids:?}: the copies {:?}", a.state.doc);
        a.set_nth(0, "board", AttrValue::from("only the first"));
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        assert_eq!(
            imgs.iter().map(|i| attr(i, "board")).collect::<Vec<_>>(),
            vec!["only the first".to_string(), String::new()],
            "{ids:?}"
        );
        b.set_nth(1, "alt", AttrValue::from("only the second"));
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        assert_eq!(
            imgs.iter().map(|i| attr(i, "alt")).collect::<Vec<_>>(),
            vec!["old alt".to_string(), "only the second".to_string()],
            "{ids:?}"
        );
    }
}

/// An image replaced in place by one with fewer attrs (a paste over it) loses the ones
/// it no longer has, on every replica and for a peer joining afterwards; a peer's
/// concurrent change of another attr is kept.
#[test]
fn an_attr_the_image_no_longer_has_reads_as_absent_everywhere() {
    for ids in ID_ORDERS {
        let s = schema(true);
        let (mut a, mut b) = two_peers(&s, document(&s, false), ids);
        a.set("board", "b1");
        sync(&mut a, &mut b);
        let plain = image(&s, SRC, "old alt");
        a.local(|tr| {
            tr.replace(3, 4, Slice::new(Fragment::from_node(plain), 0, 0))
                .unwrap();
        });
        b.set("width", 120i64);
        sync(&mut a, &mut b);
        let imgs = converged(&a, &b, &s);
        assert_eq!(imgs.len(), 1, "{ids:?}");
        assert_eq!(imgs[0].get("board"), None, "{ids:?}: {:?}", imgs[0]);
        assert_eq!(attr(&imgs[0], "width"), "120", "{ids:?}");
    }
}

// --- documents written before the merge was per attribute -----------------------

/// A session snapshot written by `rinch-editor-collab` at `3c9958cc` (before this
/// change): `ab` + image(`pimble-blob:one`, alt `old alt`, title `t`) + `cd`, then
/// `ef` + image(`pimble-blob:two`, alt `second`, title `t`) + `gh`, and the first
/// image's alt then changed to `edited alt` (which that build wrote as the image's
/// whole `@atom` value again). Starter-kit image: no `board`, no `width`.
const OLD_DOCUMENT: &[&str] = &[
    "011507002801046d65746106666f726d617401771972696e63682d656469746f722d636f6c6c61622f797273",
    "2d31070107636f6e74656e740128000701047479706501770970617261677261706827000701056174747273",
    "01270007010474657874020400070402616284070603efbfbc840707026364c10706070701c1070707080187",
    "0701012800070c04747970650177097061726167726170682700070c056174747273012700070c0474657874",
    "020400070f02656684071103efbfbc840712026768c607110712054061746f6d447b227469746c65223a2274",
    "222c224074797065223a22696d616765222c22737263223a2270696d626c652d626c6f623a74776f222c2261",
    "6c74223a227365636f6e64227dc607120713054061746f6d046e756c6cc60706070a054061746f6d487b2261",
    "6c74223a2265646974656420616c74222c227469746c65223a2274222c224074797065223a22696d61676522",
    "2c22737263223a2270696d626c652d626c6f623a6f6e65227dc6070b0708054061746f6d046e756c6c010701",
    "0a02",
];

fn old_bytes() -> Vec<u8> {
    let hex: String = OLD_DOCUMENT.concat();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn a_document_written_before_loads_as_it_was() {
    for app in [false, true] {
        let s = schema(app);
        let peer = peer_from_bytes(&s, &old_bytes(), 31);
        let imgs = images(&peer.state.doc);
        assert_eq!(imgs.len(), 2);
        assert_eq!(
            (
                attr(&imgs[0], "src"),
                attr(&imgs[0], "alt"),
                attr(&imgs[0], "title")
            ),
            ("pimble-blob:one".into(), "edited alt".into(), "t".into())
        );
        assert_eq!(
            (attr(&imgs[1], "src"), attr(&imgs[1], "alt")),
            ("pimble-blob:two".into(), "second".into())
        );
        // The attrs the app added since are not there.
        assert_eq!(imgs[0].get("board"), None);
        assert_eq!(imgs[0].get("width"), None);
    }
}

/// The everyday case for an app upgrading: its existing pictures get their first
/// board while a peer edits around them. Every concurrent edit keeps the board.
#[test]
fn an_image_written_before_takes_a_board_beside_every_concurrent_edit() {
    type Edit = Box<dyn Fn(&mut Peer)>;
    let edits: Vec<(&str, Edit)> = vec![
        ("alt", Box::new(|b: &mut Peer| b.set("alt", "new alt"))),
        (
            "width",
            Box::new(|b: &mut Peer| b.set_nth(0, "width", AttrValue::Int(200))),
        ),
        ("typing", Box::new(|b: &mut Peer| b.type_at(2, "XY"))),
        (
            "typing right after",
            Box::new(|b: &mut Peer| b.type_at(4, "XY")),
        ),
        ("Enter before", Box::new(|b: &mut Peer| b.enter_at(3))),
        (
            "Enter in the text before",
            Box::new(|b: &mut Peer| b.enter_at(2)),
        ),
        (
            "join",
            Box::new(|b: &mut Peer| {
                b.local(|tr| {
                    tr.delete(6, 8).unwrap();
                })
            }),
        ),
    ];
    for ids in ID_ORDERS {
        for (name, edit) in &edits {
            let s = schema(true);
            let mut a = peer_from_bytes(&s, &old_bytes(), ids.0);
            let mut b = peer_from_bytes(&s, &old_bytes(), ids.1);
            a.set_nth(0, "board", AttrValue::from("b1"));
            a.set_nth(1, "board", AttrValue::from("b2"));
            edit(&mut b);
            sync(&mut a, &mut b);
            let imgs = converged(&a, &b, &s);
            assert_eq!(
                imgs.iter().map(|i| attr(i, "board")).collect::<Vec<_>>(),
                vec!["b1".to_string(), "b2".to_string()],
                "{name} {ids:?}: {:?}",
                a.state.doc
            );
            match *name {
                "alt" => assert_eq!(attr(&imgs[0], "alt"), "new alt", "{ids:?}"),
                "width" => assert_eq!(attr(&imgs[0], "width"), "200", "{ids:?}"),
                _ => assert_eq!(attr(&imgs[0], "alt"), "edited alt", "{name} {ids:?}"),
            }
        }
    }
}

// --- the wire ------------------------------------------------------------------

/// Block `i`'s text chunks with their `@atom` values, read with yrs alone.
fn raw_atoms(bytes: &[u8], block: u32) -> Vec<(String, Option<HashMap<String, yrs::Any>>)> {
    use yrs::updates::decoder::Decode;
    use yrs::{Array, Map, Text, Transact, Update};
    let doc = yrs::Doc::with_options(yrs::Options {
        offset_kind: yrs::OffsetKind::Utf16,
        ..Default::default()
    });
    doc.transact_mut()
        .apply_update(Update::decode_v1(bytes).unwrap())
        .unwrap();
    let content = doc.get_or_insert_array("content");
    let txn = doc.transact();
    let Some(yrs::Out::YMap(node)) = content.get(&txn, block) else {
        panic!("no block {block}");
    };
    let Some(yrs::Out::YText(text)) = node.get(&txn, "text") else {
        panic!("block {block} has no text");
    };
    text.diff(&txn, yrs::types::text::YChange::identity)
        .into_iter()
        .map(|d| {
            let yrs::Out::Any(yrs::Any::String(s)) = &d.insert else {
                panic!("a string chunk");
            };
            let atom = d.attributes.as_ref().and_then(|a| match a.get("@atom") {
                Some(yrs::Any::Map(m)) => Some(
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<HashMap<_, _>>(),
                ),
                _ => None,
            });
            (s.to_string(), atom)
        })
        .collect()
}

/// The `atoms` root map's keys, read with yrs alone.
fn raw_atom_entries(bytes: &[u8]) -> Vec<(String, yrs::Any)> {
    use yrs::updates::decoder::Decode;
    use yrs::{Map, Transact, Update};
    let doc = yrs::Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v1(bytes).unwrap())
        .unwrap();
    let map = doc.get_or_insert_map("atoms");
    let txn = doc.transact();
    let mut out: Vec<(String, yrs::Any)> = map
        .iter(&txn)
        .filter_map(|(k, v)| match v {
            yrs::Out::Any(a) => Some((k.to_string(), a)),
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A new image is written exactly as a build before this change writes it (one
/// `@atom` value, its type and attrs, nothing else), so such a build reads every
/// document whose images were only inserted. A change of one attr is one `atoms`
/// entry and leaves that value alone; a move carries `@id`.
#[test]
fn the_wire_a_new_image_a_changed_attr_and_a_moved_image() {
    let s = schema(true);
    let (mut a, _) = two_peers(&s, document(&s, false), (11, 22));
    let chunks = raw_atoms(&a.session.snapshot(), 0);
    let atom = chunks[1].1.clone().expect("the image's value");
    let mut keys: Vec<&str> = atom.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, vec!["@type", "alt", "src"]);
    assert!(raw_atom_entries(&a.session.snapshot()).is_empty());

    a.set("board", "b1");
    let after = a.session.snapshot();
    assert_eq!(
        raw_atoms(&after, 0)[1].1,
        Some(atom.clone()),
        "the value is not rewritten"
    );
    let entries = raw_atom_entries(&after);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert!(entries[0].0.ends_with("/board"), "{entries:?}");
    assert_eq!(entries[0].1, yrs::Any::String(Arc::from("b1")));

    // Enter right before the image moves it into a new block: its new char carries
    // the identity the entry is under.
    a.enter_at(3);
    let moved = raw_atoms(&a.session.snapshot(), 1);
    let value = moved[0].1.clone().expect("the moved image's value");
    let id = entries[0].0.trim_end_matches("/board");
    assert_eq!(value.get("@id"), Some(&yrs::Any::String(Arc::from(id))));
    assert_eq!(attr(&images(&a.state.doc)[0], "board"), "b1");
}

/// A `hard_break` has no attrs a peer could change, so a move carries nothing: its
/// value stays the one key `@type`, whose encoding is the same on every run (#841).
#[test]
fn a_moved_hard_break_carries_no_identity() {
    let s = schema(true);
    let line = para(
        &s,
        vec![
            s.text("ab").unwrap(),
            s.branch("hard_break", Fragment::empty()).unwrap(),
            s.text("cd").unwrap(),
        ],
    );
    let doc = s.branch("doc", Fragment::from_node(line)).unwrap();
    let (mut a, _) = two_peers(&s, doc, (11, 22));
    a.enter_at(3);
    let moved = raw_atoms(&a.session.snapshot(), 1);
    let value = moved[0].1.clone().expect("the moved break's value");
    assert_eq!(value.keys().collect::<Vec<_>>(), vec!["@type"], "{moved:?}");
}
