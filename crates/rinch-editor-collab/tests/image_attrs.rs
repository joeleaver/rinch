//! An `image`'s own attributes (`src`, `alt`, `title`) through the projection and
//! through a merge.
//!
//! An inline atom is one U+FFFC char carrying one formatting attribute, `@atom`,
//! whose value is the atom's **whole** attribute map (`projection.rs`). So the three
//! attrs travel together, and yrs resolves two concurrent writes of one formatting
//! attribute over one char by keeping one of them. These tests say what that means
//! for an app that edits an image's `alt` or `title` while a peer does too:
//!
//! - every attr round-trips, through a snapshot and through a live update;
//! - two concurrent changes of the **same** attr converge on one of the two values;
//! - two concurrent changes of **different** attrs of one image also converge, and
//!   **one of the two is lost**: the image ends up exactly as one of the peers left
//!   it, never with both changes. It is last-writer-wins on the image, not on the
//!   attribute. Which peer wins follows the client-id order.
//!
//! The last is a limitation, pinned here so that a change to it is a decision.
//! Merging per attribute needs each attr to be a formatting attribute of its own
//! (`@atom.alt`, …), which is a change of the wire shape and a coordinated upgrade.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Plugin, Schema, SetNodeAttrStep, Transaction,
    default_plugins,
};

const SRC: &str = "pimble-blob:6f1c2a0e-58b1-4a3e-9d57-0c1f3b0a9e11/b3-9f86d081884c7d65";

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

    /// Set one attribute of the image (which sits at model position 3: `doc`,
    /// `paragraph`, then `"ab"`).
    fn set(&mut self, attr: &str, value: &str) {
        self.local(|tr| {
            tr.step(Box::new(SetNodeAttrStep::new(
                3,
                attr,
                AttrValue::from(value),
            )))
            .unwrap();
        });
    }

    /// The image's `(src, alt, title)`, and that there is exactly one image.
    fn image(&self) -> (String, String, String) {
        let para = self.state.doc.child(0);
        let images: Vec<&Node> = (0..para.child_count())
            .map(|i| para.child(i))
            .filter(|n| n.type_name() == "image")
            .collect();
        assert_eq!(images.len(), 1, "exactly one image: {:?}", self.state.doc);
        let attr = |name: &str| images[0].attrs().get_str(name).unwrap_or("").to_string();
        (attr("src"), attr("alt"), attr("title"))
    }
}

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

/// `ab` + an image + `cd`.
fn line(schema: &Schema, src: &str, alt: &str, title: &str) -> Node {
    let image = schema
        .create_node(
            "image",
            Attrs::new()
                .with("src", AttrValue::from(src))
                .with("alt", AttrValue::from(alt))
                .with("title", AttrValue::from(title)),
            Fragment::empty(),
        )
        .unwrap();
    let para = schema
        .branch(
            "paragraph",
            Fragment::from_children(vec![
                schema.text("ab").unwrap(),
                image,
                schema.text("cd").unwrap(),
            ]),
        )
        .unwrap();
    schema.branch("doc", Fragment::from_node(para)).unwrap()
}

/// Two peers over one document, B joined from A's snapshot, with pinned client ids
/// (yrs breaks a tie between concurrent writes by client id).
fn two_peers(schema: &Rc<Schema>, doc: Node, ids: (u64, u64)) -> (Peer, Peer) {
    let a_state = EditorState::create(schema.clone(), doc, plugins());
    let session_a = session_with_client_id(&a_state, ids.0).expect("session A");
    let session_b =
        session_from_bytes_with_client_id(&session_a.snapshot(), ids.1).expect("session B");
    let b_doc = session_b.projected_doc(schema).expect("project B");
    (
        Peer {
            state: a_state,
            session: session_a,
        },
        Peer {
            state: EditorState::create(schema.clone(), b_doc, plugins()),
            session: session_b,
        },
    )
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

/// Both peers hold the same image, and each peer's document is what its own CRDT
/// reads back as.
fn converged(a: &Peer, b: &Peer, schema: &Schema) -> (String, String, String) {
    assert_eq!(a.image(), b.image(), "the peers hold different images");
    for peer in [a, b] {
        assert_eq!(
            peer.state.doc,
            peer.session.projected_doc(schema).unwrap(),
            "a peer's document is not what its CRDT holds"
        );
    }
    assert_eq!(a.state.doc, b.state.doc);
    a.image()
}

const ID_ORDERS: [(u64, u64); 2] = [(11, 22), (22, 11)];

#[test]
fn src_alt_and_title_round_trip_through_a_snapshot_and_a_live_update() {
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(
        &schema,
        line(&schema, SRC, "Mum & \"Dad\" — 1987 📷", "the <garden>"),
        (11, 22),
    );
    // The joiner built the image from the snapshot, every attr as written.
    assert_eq!(
        b.image(),
        (
            SRC.to_string(),
            "Mum & \"Dad\" — 1987 📷".to_string(),
            "the <garden>".to_string()
        )
    );

    // Each attr changed on its own, one sync at a time, reaches the peer and
    // leaves the other two alone.
    a.set("alt", "the garden, summer");
    sync(&mut a, &mut b);
    b.set("title", "");
    sync(&mut a, &mut b);
    a.set("src", "pimble-blob:other/blob");
    sync(&mut a, &mut b);
    assert_eq!(
        converged(&a, &b, &schema),
        (
            "pimble-blob:other/blob".to_string(),
            "the garden, summer".to_string(),
            String::new()
        )
    );

    // And a peer joining now reads the same image.
    let late = CollabSession::from_bytes(&a.session.snapshot()).unwrap();
    assert_eq!(late.projected_doc(&schema).unwrap(), a.state.doc);
}

#[test]
fn an_image_with_only_a_src_keeps_its_empty_alt_and_title() {
    let schema = Rc::new(Schema::starter_kit());
    let (a, b) = two_peers(&schema, line(&schema, SRC, "", ""), (11, 22));
    assert_eq!(b.image(), (SRC.to_string(), String::new(), String::new()));
    assert_eq!(converged(&a, &b, &schema).0, SRC);
}

#[test]
fn two_peers_changing_alt_at_once_converge_on_one_of_the_two() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old", "t"), ids);
        a.set("alt", "from A");
        b.set("alt", "from B");
        sync(&mut a, &mut b);
        let (src, alt, title) = converged(&a, &b, &schema);
        let expected = if ids.0 > ids.1 { "from A" } else { "from B" };
        assert_eq!(
            alt, expected,
            "ids {ids:?}: the higher client id's write is kept"
        );
        assert_eq!((src.as_str(), title.as_str()), (SRC, "t"), "ids {ids:?}");
    }
}

/// **The limitation, pinned.** One peer changes `alt` while the other changes
/// `title`. The peers converge, and the image is exactly what **one** of them made
/// it: the other's change is gone, although the two never touched the same
/// attribute. Under yrs's tie-break the peer with the higher client id wins.
///
/// If this test starts failing because both changes survive, the encoding has
/// become per-attribute: update the module docs and `projection.rs`, and keep the
/// stronger assertion.
#[test]
fn concurrent_changes_of_alt_and_title_keep_one_peers_image_and_lose_the_others_change() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", "old title"), ids);
        a.set("alt", "new alt");
        b.set("title", "new title");
        sync(&mut a, &mut b);
        let (src, alt, title) = converged(&a, &b, &schema);
        assert_eq!(src, SRC, "ids {ids:?}: nobody touched the src");

        let as_a_left_it = ("new alt", "old title");
        let as_b_left_it = ("old alt", "new title");
        let expected = if ids.0 > ids.1 {
            as_a_left_it
        } else {
            as_b_left_it
        };
        assert_eq!(
            (alt.as_str(), title.as_str()),
            expected,
            "ids {ids:?}: the image is the higher client id's, whole"
        );
    }
}

/// The same shape with the attribute an app is least likely to expect it on: a
/// peer replacing the picture (`src`) while another edits its description. The
/// description edit is lost when the `src` writer wins, and the **old picture
/// comes back** when the `alt` writer wins.
#[test]
fn a_concurrent_src_change_and_alt_change_keep_only_one() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", ""), ids);
        a.set("src", "pimble-blob:other/blob");
        b.set("alt", "new alt");
        sync(&mut a, &mut b);
        let (src, alt, _) = converged(&a, &b, &schema);
        let expected = if ids.0 > ids.1 {
            ("pimble-blob:other/blob", "old alt")
        } else {
            (SRC, "new alt")
        };
        assert_eq!((src.as_str(), alt.as_str()), expected, "ids {ids:?}");
    }
}

/// What is **not** lost: a change to an image's attrs beside a peer's typing in the
/// same line, on either side of the image.
#[test]
fn an_alt_change_survives_a_peers_typing_on_either_side_of_the_image() {
    for ids in ID_ORDERS {
        for at in [3, 4] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old", "t"), ids);
            a.set("alt", "new alt");
            b.local(|tr| {
                tr.set_selection(rinch_editor_core::Selection::cursor(
                    rinch_editor_core::Pos(at),
                ));
                tr.insert_text("XY").unwrap();
            });
            sync(&mut a, &mut b);
            let got = converged(&a, &b, &schema);
            assert_eq!(
                got,
                (SRC.to_string(), "new alt".to_string(), "t".to_string()),
                "ids {ids:?}, typed at {at}"
            );
            let text: String = (0..a.state.doc.child(0).child_count())
                .filter_map(|i| a.state.doc.child(0).child(i).text().map(str::to_string))
                .collect();
            assert_eq!(text.len(), 6, "ids {ids:?}, typed at {at}: {text:?}");
            assert!(text.contains("XY"), "ids {ids:?}, typed at {at}: {text:?}");
        }
    }
}

// Review of #1431: the claims above under random client ids and histories,
// and an `alt` change beside each of a peer's other edits to the same line.

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Every image in the document, in order.
fn images(doc: &Node) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    fn walk(n: &Node, out: &mut Vec<(String, String, String)>) {
        for i in 0..n.child_count() {
            let c = n.child(i);
            if c.type_name() == "image" {
                let a = |k: &str| c.attrs().get_str(k).unwrap_or("").to_string();
                out.push((a("src"), a("alt"), a("title")));
            }
            walk(c, out);
        }
    }
    walk(doc, &mut out);
    out
}

fn random_id(rng: &mut Rng) -> u64 {
    match rng.below(4) {
        0 => 1 + rng.below(1000) as u64,
        1 => rng.next() >> 32,
        2 => rng.next() >> 11, // up to 2^53
        _ => (rng.next() >> 12) | 1,
    }
}

/// "The higher client id's write is kept": random id pairs (small, 32-bit, up
/// to 2^53, larger), and a history before the concurrent edits that differs
/// between the peers. The image is always exactly one peer's.
#[test]
fn the_higher_client_id_wins_under_random_ids_and_histories() {
    let mut rng = Rng(0x1431_1431_1431);
    let (mut higher, mut lower, mut trials) = (0usize, 0usize, 0usize);
    let mut shapes = [[0usize; 2]; 6];
    for trial in 0..240 {
        let (ia, ib) = (random_id(&mut rng), random_id(&mut rng));
        if ia == ib {
            continue;
        }
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(
            &schema,
            line(&schema, SRC, "old alt", "old title"),
            (ia, ib),
        );
        // A history, synced: either peer retitles, types, a few times.
        let shape = trial % 6;
        match shape {
            0 => {}
            1 => {
                for i in 0..3 {
                    a.set("alt", &format!("a{i}"));
                    sync(&mut a, &mut b);
                }
            }
            2 => {
                for i in 0..3 {
                    b.set("title", &format!("b{i}"));
                    sync(&mut a, &mut b);
                }
            }
            3 => {
                a.set("alt", "a0");
                sync(&mut a, &mut b);
                b.set("alt", "b0");
                sync(&mut a, &mut b);
                a.set("title", "a1");
                sync(&mut a, &mut b);
            }
            _ => {}
        }
        let (_, alt0, title0) = a.image();
        // The concurrent edits.
        match shape {
            // A writes twice (unsynced), B once.
            4 => {
                a.set("alt", "mid alt");
                a.set("alt", "new alt");
                b.set("title", "new title");
            }
            // A types right before the image first, then retitles.
            5 => {
                a.local(|tr| {
                    tr.set_selection(rinch_editor_core::Selection::cursor(
                        rinch_editor_core::Pos(3),
                    ));
                    tr.insert_text("Q").unwrap();
                });
                // the image is now at 4
                a.local(|tr| {
                    tr.step(Box::new(SetNodeAttrStep::new(
                        4,
                        "alt",
                        AttrValue::from("new alt"),
                    )))
                    .unwrap();
                });
                b.set("title", "new title");
            }
            _ => {
                a.set("alt", "new alt");
                b.set("title", "new title");
            }
        }
        sync(&mut a, &mut b);
        let (_, alt, title) = converged(&a, &b, &schema);
        trials += 1;
        let a_won = (alt.as_str(), title.as_str()) == ("new alt", title0.as_str());
        let b_won = (alt.as_str(), title.as_str()) == (alt0.as_str(), "new title");
        assert!(
            a_won ^ b_won,
            "trial {trial} ids ({ia},{ib}) shape {shape}: neither peer's image: {alt:?} {title:?}"
        );
        let higher_won = a_won == (ia > ib);
        if higher_won {
            higher += 1;
            shapes[shape][0] += 1;
        } else {
            lower += 1;
            shapes[shape][1] += 1;
        }
    }
    assert!(trials > 200, "{trials}");
    assert_eq!(
        (higher, lower),
        (trials, 0),
        "by shape [higher, lower] {shapes:?}"
    );
}

/// An `alt` change beside a peer's other concurrent edits to the same line:
/// kept through bold over the line, a retype to a heading and a deleted char on
/// either side; deleting the image wins over the change.
#[test]
fn an_alt_change_beside_a_peers_other_edits_to_the_line() {
    type Edit = Box<dyn Fn(&mut Peer)>;
    for ids in ID_ORDERS {
        // (what B does, the alts left)
        let cases: Vec<(&str, Edit, Vec<&str>)> = vec![
            (
                "bold over the whole line",
                Box::new(|b: &mut Peer| {
                    let bold = b.state.schema().mark_type("bold").unwrap().clone();
                    b.local(|tr| {
                        tr.add_mark(1, 6, rinch_editor_core::Mark::new(bold, Attrs::new()))
                            .unwrap();
                    })
                }),
                vec!["new alt"],
            ),
            (
                "a retype to a heading",
                Box::new(|b: &mut Peer| {
                    let h = b.state.schema().node_type("heading").unwrap().clone();
                    b.local(|tr| {
                        tr.set_block_type(
                            1,
                            6,
                            h,
                            Attrs::new().with("level", AttrValue::from(2i64)),
                        )
                        .unwrap();
                    })
                }),
                vec!["new alt"],
            ),
            (
                "the char right before deleted",
                Box::new(|b: &mut Peer| {
                    b.local(|tr| {
                        tr.delete(2, 3).unwrap();
                    })
                }),
                vec!["new alt"],
            ),
            (
                "the char right after deleted",
                Box::new(|b: &mut Peer| {
                    b.local(|tr| {
                        tr.delete(4, 5).unwrap();
                    })
                }),
                vec!["new alt"],
            ),
            (
                "the image deleted",
                Box::new(|b: &mut Peer| {
                    b.local(|tr| {
                        tr.delete(3, 4).unwrap();
                    })
                }),
                vec![],
            ),
        ];
        for (name, edit, expected) in cases {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", "t"), ids);
            a.set("alt", "new alt");
            edit(&mut b);
            sync(&mut a, &mut b);
            assert_eq!(a.state.doc, b.state.doc, "{name} {ids:?}: diverged");
            for peer in [&a, &b] {
                assert_eq!(peer.state.doc, peer.session.projected_doc(&schema).unwrap());
            }
            let imgs = images(&a.state.doc);
            let alts: Vec<&str> = imgs.iter().map(|i| i.1.as_str()).collect();
            assert_eq!(alts, expected, "{name} {ids:?}: {:?}", a.state.doc);
        }
    }
}

/// A known loss, pinned (#861's mechanism): Enter anywhere **before** the image
/// in its own paragraph (inside the text before it as well as right before it)
/// moves the image to a new block, a delete and an insert, so a peer's
/// concurrent `alt` change is lost in both id orders. Enter after it keeps the
/// change. A fix flips the first two.
#[test]
fn enter_before_the_image_in_its_line_loses_a_concurrent_alt_change() {
    for ids in ID_ORDERS {
        for (at, expected) in [(2, "old alt"), (3, "old alt"), (5, "new alt")] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", "t"), ids);
            a.set("alt", "new alt");
            b.local(|tr| {
                tr.split(at, 1, None).unwrap();
            });
            sync(&mut a, &mut b);
            assert_eq!(a.state.doc, b.state.doc, "ids {ids:?}, Enter at {at}");
            let imgs = images(&a.state.doc);
            assert_eq!(imgs.len(), 1, "ids {ids:?}, Enter at {at}");
            assert_eq!(imgs[0].1, expected, "ids {ids:?}, Enter at {at}");
        }
    }
}

// An app's own attributes on an image: `board` (an id an app keeps for what it
// draws over the picture) and `width`. They ride in the same `@atom` value as
// `src`/`alt`/`title`, so everything above holds for them too.

impl Peer {
    fn set_int(&mut self, attr: &str, value: i64) {
        self.local(|tr| {
            tr.step(Box::new(SetNodeAttrStep::new(
                3,
                attr,
                AttrValue::Int(value),
            )))
            .unwrap();
        });
    }

    /// The image's `(board, width)`, absent as `("", 0)`.
    fn board_and_width(&self) -> (String, i64) {
        let _ = self.image(); // exactly one
        let para = self.state.doc.child(0);
        let img = (0..para.child_count())
            .map(|i| para.child(i))
            .find(|n| n.type_name() == "image")
            .unwrap();
        (
            img.attrs().get_str("board").unwrap_or("").to_string(),
            img.attrs().get_int("width").unwrap_or(0),
        )
    }
}

#[test]
fn board_and_width_round_trip_through_a_snapshot_and_a_live_update() {
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "alt", ""), (11, 22));
    a.set("board", "01JA7Z3QK2V9");
    sync(&mut a, &mut b);
    assert_eq!(b.board_and_width(), ("01JA7Z3QK2V9".to_string(), 0));
    b.set_int("width", 320);
    sync(&mut a, &mut b);
    converged(&a, &b, &schema);
    assert_eq!(a.board_and_width(), ("01JA7Z3QK2V9".to_string(), 320));
    // The other attrs are untouched.
    assert_eq!(
        a.image(),
        (SRC.to_string(), "alt".to_string(), String::new())
    );

    // A peer joining from a snapshot builds the image with both.
    let late = CollabSession::from_bytes(&a.session.snapshot()).unwrap();
    assert_eq!(late.projected_doc(&schema).unwrap(), a.state.doc);
}

/// Marking a picture up (writing its `board`) while a peer types beside it,
/// on either side: both are kept.
#[test]
fn a_board_written_beside_a_peers_typing_is_kept() {
    for ids in ID_ORDERS {
        for at in [3, 4] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "alt", ""), ids);
            a.set("board", "b1");
            b.local(|tr| {
                tr.set_selection(rinch_editor_core::Selection::cursor(
                    rinch_editor_core::Pos(at),
                ));
                tr.insert_text("XY").unwrap();
            });
            sync(&mut a, &mut b);
            converged(&a, &b, &schema);
            assert_eq!(
                a.board_and_width().0,
                "b1",
                "ids {ids:?}, typed at {at}: {:?}",
                a.state.doc
            );
        }
    }
}

/// **The same limitation as `alt` and `title`, pinned for `board`:** a peer's
/// concurrent change to another attribute of the same image (here `alt`) and
/// a first mark-up converge on one peer's image, whole. An app that mints a
/// board id must expect the id it wrote to be lost this way (and the same for
/// Enter before the image in its line, above), and so must not count on the
/// attribute being there because it wrote it.
#[test]
fn a_board_and_a_concurrent_alt_change_keep_one_peers_image() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", ""), ids);
        a.set("board", "b1");
        b.set("alt", "new alt");
        sync(&mut a, &mut b);
        let (_, alt, _) = converged(&a, &b, &schema);
        let (board, _) = a.board_and_width();
        let expected = if ids.0 > ids.1 {
            ("b1", "old alt")
        } else {
            ("", "new alt")
        };
        assert_eq!((board.as_str(), alt.as_str()), expected, "ids {ids:?}");
    }
}

/// Two peers marking the same picture up at once both mint a board id; they
/// converge on one of the two.
#[test]
fn two_boards_written_at_once_converge_on_one() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "", ""), ids);
        a.set("board", "from A");
        b.set("board", "from B");
        sync(&mut a, &mut b);
        converged(&a, &b, &schema);
        let expected = if ids.0 > ids.1 { "from A" } else { "from B" };
        assert_eq!(a.board_and_width().0, expected, "ids {ids:?}");
    }
}
