//! An `image`'s own attributes (`src`, `alt`, `title`) through the projection and
//! through a merge.
//!
//! An inline atom is one U+FFFC char carrying the formatting attribute `@atom`, whose
//! value is the atom's attrs as its char was written; every change of one attr since is
//! an entry of its own in the `atoms` root map, keyed by the atom's identity and the
//! attr (`src/atoms.rs`). These tests say what that means for an app that edits an
//! image's `alt` or `title` while a peer does too:
//!
//! - every attr round-trips, through a snapshot and through a live update;
//! - two concurrent changes of the **same** attr converge on one of the two values
//!   (the higher client id's);
//! - two concurrent changes of **different** attrs of one image both survive: the
//!   merge is per attribute, under any client ids and histories;
//! - a change survives a peer's typing, marks, retype and deletions around the image,
//!   and a peer's Enter anywhere in the image's line, before it included (the atom
//!   keeps its identity when a split moves it to a new block).
//!
//! Until #1431's follow-up these were pinned the other way: the attrs travelled as one
//! value, and of two concurrent changes to different attrs one peer's image won whole.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Node, Plugin, Schema, SetNodeAttrStep, Transaction,
    default_plugins,
};

const SRC: &str = "app-blob:6f1c2a0e-58b1-4a3e-9d57-0c1f3b0a9e11/b3-9f86d081884c7d65";

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
    a.set("src", "app-blob:other/blob");
    sync(&mut a, &mut b);
    assert_eq!(
        converged(&a, &b, &schema),
        (
            "app-blob:other/blob".to_string(),
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

/// One peer changes `alt` while the other changes `title`: both changes survive, in
/// both client-id orders.
#[test]
fn concurrent_changes_of_alt_and_title_both_survive() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", "old title"), ids);
        a.set("alt", "new alt");
        b.set("title", "new title");
        sync(&mut a, &mut b);
        assert_eq!(
            converged(&a, &b, &schema),
            (
                SRC.to_string(),
                "new alt".to_string(),
                "new title".to_string()
            ),
            "ids {ids:?}"
        );
    }
}

/// A peer replacing the picture (`src`) while another edits its description: the
/// `src` change makes a new picture, and the concurrent `alt` change, made to the old
/// one, is lost (the new `src` is kept, with the `alt` the image had). Chosen by Joe
/// (2026-10-09): a wrong attribution is worse than a lost one — the projection cannot
/// tell a `src` change from a picture pasted over another, and keeping the identity
/// through it showed a peer's app data on the pasted picture. An app that never
/// changes `src` on a live picture loses nothing to it.
#[test]
fn a_src_change_drops_a_concurrent_alt_change() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", ""), ids);
        a.set("src", "app-blob:other/blob");
        b.set("alt", "new alt");
        sync(&mut a, &mut b);
        let (src, alt, _) = converged(&a, &b, &schema);
        assert_eq!(
            (src.as_str(), alt.as_str()),
            ("app-blob:other/blob", "old alt"),
            "ids {ids:?}"
        );
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

/// The model position of the first image in the document.
fn image_pos(doc: &Node) -> usize {
    let mut at = None;
    doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
        if n.type_name() == "image" && at.is_none() {
            at = Some(pos);
        }
        true
    });
    at.expect("an image")
}

fn random_id(rng: &mut Rng) -> u64 {
    match rng.below(4) {
        0 => 1 + rng.below(1000) as u64,
        1 => rng.next() >> 32,
        2 => rng.next() >> 11, // up to 2^53
        _ => (rng.next() >> 12) | 1,
    }
}

/// Random id pairs (small, 32-bit, up to 2^53, larger), and a history before the
/// concurrent edits that differs between the peers: both peers' changes to different
/// attrs always survive, and of two concurrent changes to the **same** attr the higher
/// client id's is kept.
#[test]
fn both_changes_survive_under_random_ids_and_histories() {
    let mut rng = Rng(0x1431_1431_1431);
    let mut trials = 0usize;
    let mut shapes = [0usize; 6];
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
        assert_eq!(
            (alt.as_str(), title.as_str()),
            ("new alt", "new title"),
            "trial {trial} ids ({ia},{ib}) shape {shape}: a change was lost \
             (before: {alt0:?} {title0:?})"
        );
        shapes[shape] += 1;

        // And the same attr, changed by both at once: one value, the higher id's.
        for (peer, value) in [(&mut a, "alt from A"), (&mut b, "alt from B")] {
            let at = image_pos(&peer.state.doc);
            peer.local(|tr| {
                tr.step(Box::new(SetNodeAttrStep::new(
                    at,
                    "alt",
                    AttrValue::from(value),
                )))
                .unwrap();
            });
        }
        sync(&mut a, &mut b);
        let (_, alt, _) = converged(&a, &b, &schema);
        let expected = if ia > ib { "alt from A" } else { "alt from B" };
        assert_eq!(alt, expected, "trial {trial} ids ({ia},{ib}) shape {shape}");
    }
    assert!(trials > 200, "{trials}");
    assert!(shapes.iter().all(|&n| n > 20), "by shape {shapes:?}");
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

/// Enter anywhere in the image's line (inside the text before it, right before it,
/// right after it) while a peer changes its `alt`: the change is kept in both id
/// orders. A split before the image moves it to a new block, a delete and an insert,
/// and the new char carries the image's identity, under which the peer's change is
/// written (#861's mechanism, which lost it until then).
#[test]
fn enter_anywhere_in_the_images_line_keeps_a_concurrent_alt_change() {
    for ids in ID_ORDERS {
        for at in [2, 3, 4, 5] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", "t"), ids);
            a.set("alt", "new alt");
            b.local(|tr| {
                tr.split(at, 1, None).unwrap();
            });
            sync(&mut a, &mut b);
            assert_eq!(a.state.doc, b.state.doc, "ids {ids:?}, Enter at {at}");
            for peer in [&a, &b] {
                assert_eq!(peer.state.doc, peer.session.projected_doc(&schema).unwrap());
            }
            let imgs = images(&a.state.doc);
            assert_eq!(
                imgs,
                vec![(SRC.to_string(), "new alt".to_string(), "t".to_string())],
                "ids {ids:?}, Enter at {at}"
            );
        }
    }
}

// An app's own attributes on an image: `data-ref` (an id an app keeps for what it
// draws over the picture) and `width`. They merge per attribute like
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

    /// The image's `(data-ref, width)`, absent as `("", 0)`.
    fn ref_and_width(&self) -> (String, i64) {
        let _ = self.image(); // exactly one
        let para = self.state.doc.child(0);
        let img = (0..para.child_count())
            .map(|i| para.child(i))
            .find(|n| n.type_name() == "image")
            .unwrap();
        (
            img.attrs().get_str("data-ref").unwrap_or("").to_string(),
            img.attrs().get_int("width").unwrap_or(0),
        )
    }
}

#[test]
fn ref_and_width_round_trip_through_a_snapshot_and_a_live_update() {
    let schema = Rc::new(Schema::starter_kit());
    let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "alt", ""), (11, 22));
    a.set("data-ref", "01JA7Z3QK2V9");
    sync(&mut a, &mut b);
    assert_eq!(b.ref_and_width(), ("01JA7Z3QK2V9".to_string(), 0));
    b.set_int("width", 320);
    sync(&mut a, &mut b);
    converged(&a, &b, &schema);
    assert_eq!(a.ref_and_width(), ("01JA7Z3QK2V9".to_string(), 320));
    // The other attrs are untouched.
    assert_eq!(
        a.image(),
        (SRC.to_string(), "alt".to_string(), String::new())
    );

    // A peer joining from a snapshot builds the image with both.
    let late = CollabSession::from_bytes(&a.session.snapshot()).unwrap();
    assert_eq!(late.projected_doc(&schema).unwrap(), a.state.doc);
}

/// Marking a picture up (writing its `data-ref`) while a peer types beside it,
/// on either side: both are kept.
#[test]
fn a_data_ref_written_beside_a_peers_typing_is_kept() {
    for ids in ID_ORDERS {
        for at in [3, 4] {
            let schema = Rc::new(Schema::starter_kit());
            let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "alt", ""), ids);
            a.set("data-ref", "b1");
            b.local(|tr| {
                tr.set_selection(rinch_editor_core::Selection::cursor(
                    rinch_editor_core::Pos(at),
                ));
                tr.insert_text("XY").unwrap();
            });
            sync(&mut a, &mut b);
            converged(&a, &b, &schema);
            assert_eq!(
                a.ref_and_width().0,
                "b1",
                "ids {ids:?}, typed at {at}: {:?}",
                a.state.doc
            );
        }
    }
}

/// A first mark-up (`data-ref`) and a peer's concurrent change of another attribute
/// of the same image (here `alt`) are both kept: an image's attrs merge per
/// attribute (#1503). Before that they merged as one value, and one peer's image was
/// kept whole.
#[test]
fn a_data_ref_and_a_concurrent_alt_change_both_survive() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "old alt", ""), ids);
        a.set("data-ref", "b1");
        b.set("alt", "new alt");
        sync(&mut a, &mut b);
        let (_, alt, _) = converged(&a, &b, &schema);
        let (data_ref, _) = a.ref_and_width();
        assert_eq!(
            (data_ref.as_str(), alt.as_str()),
            ("b1", "new alt"),
            "ids {ids:?}"
        );
    }
}

/// Two peers marking the same picture up at once both mint a data-ref; they
/// converge on one of the two.
#[test]
fn two_data_refs_written_at_once_converge_on_one() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "", ""), ids);
        a.set("data-ref", "from A");
        b.set("data-ref", "from B");
        sync(&mut a, &mut b);
        converged(&a, &b, &schema);
        let expected = if ids.0 > ids.1 { "from A" } else { "from B" };
        assert_eq!(a.ref_and_width().0, expected, "ids {ids:?}");
    }
}

/// Two peers setting **different** `data-*` attributes of one starter-kit image at
/// once both keep theirs: each is its own entry in the per-attribute `atoms` map. A
/// third, set beforehand, is untouched; and a peer joining from a snapshot reads all
/// of them.
#[test]
fn two_different_data_attrs_set_at_once_both_survive() {
    for ids in ID_ORDERS {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "", ""), ids);
        a.set("data-kept", "k");
        sync(&mut a, &mut b);
        a.set("data-ref", "r1");
        b.set("data-annotation-id", "a1");
        sync(&mut a, &mut b);
        converged(&a, &b, &schema);
        let para = a.state.doc.child(0);
        let img = (0..para.child_count())
            .map(|i| para.child(i))
            .find(|n| n.type_name() == "image")
            .unwrap();
        let data: Vec<(&str, &str)> = rinch_editor_core::app_data_attrs(img.attrs()).collect();
        assert_eq!(
            data,
            [
                ("data-annotation-id", "a1"),
                ("data-kept", "k"),
                ("data-ref", "r1")
            ],
            "ids {ids:?}"
        );
        let late = CollabSession::from_bytes(&a.session.snapshot()).unwrap();
        assert_eq!(late.projected_doc(&schema).unwrap(), a.state.doc);
    }
}
