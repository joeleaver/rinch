//! Review of #1518: `#[ignore]` growth and phase probes for an image with many
//! data attributes (`P_N`, `P_LEN`, `P_K`), kept as the measurement behind
//! `image_attrs::an_images_attributes_cost_linear_copies_in_collaboration`.
#![allow(dead_code)]

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
fn set_any(p: &mut Peer, attr: &str, v: AttrValue) {
    p.local(|tr| {
        tr.step(Box::new(SetNodeAttrStep::new(3, attr, v))).unwrap();
    });
}

/// model ≡ project after a step stores a name/value validation would drop.
#[test]
fn probe_reserved_or_nonstring_attr_through_a_step() {
    for (name, v) in [
        ("data-rid", AttrValue::from("7")),
        ("data-n", AttrValue::Int(3)),
        ("data-Ref", AttrValue::from("u")),
        ("caption", AttrValue::from("c")),
    ] {
        let schema = Rc::new(Schema::starter_kit());
        let (mut a, mut b) = two_peers(&schema, line(&schema, SRC, "", ""), (11, 22));
        set_any(&mut a, name, v.clone());
        sync(&mut a, &mut b);
        let pa = a.session.projected_doc(&schema).unwrap();
        let pb = b.session.projected_doc(&schema).unwrap();
        let img = |d: &Node| d.child(0).child(1).attrs().get(name).cloned();
        eprintln!(
            "PROBE {name}: model_a={:?} proj_a={:?} model_b={:?} proj_b={:?} a_model==proj {} a==b {}",
            img(&a.state.doc),
            img(&pa),
            img(&b.state.doc),
            img(&pb),
            a.state.doc == pa,
            a.state.doc == b.state.doc
        );
    }
}

fn line_with(schema: &Schema, n: usize, len: usize) -> Node {
    let mut attrs = Attrs::new().with("src", AttrValue::from(SRC));
    let v = "x".repeat(len);
    for i in 0..n {
        attrs = attrs.with(format!("data-a{i}"), AttrValue::from(v.as_str()));
    }
    let image = schema
        .create_node("image", attrs, Fragment::empty())
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

/// CRDT growth: one image with `n` data attrs of `len` bytes, then `k` Enters
/// before it and after it (each moves the picture to a new block).
#[test]
#[ignore]
fn probe_growth() {
    let n: usize = std::env::var("P_N").unwrap().parse().unwrap();
    let len: usize = std::env::var("P_LEN").unwrap().parse().unwrap();
    let k: usize = std::env::var("P_K").unwrap_or("0".into()).parse().unwrap();
    let schema = Rc::new(Schema::starter_kit());
    let t = std::time::Instant::now();
    let (mut a, mut b) = two_peers(&schema, line_with(&schema, n, len), (11, 22));
    let snap0 = a.session.snapshot().len();
    let t0 = t.elapsed();
    let t = std::time::Instant::now();
    for _ in 0..k {
        // Enter right before the picture, then Backspace it back.
        a.local(|tr| {
            tr.split(3, 1, None).unwrap();
        });
        a.local(|tr| {
            tr.delete(3, 5).unwrap();
        });
    }
    sync(&mut a, &mut b);
    eprintln!(
        "PROBE n={n} len={len} k={k}: raw attrs {} B, snapshot0 {} B, snapshot after {} B, build {:?}, edits {:?}, a==b {}",
        n * len,
        snap0,
        a.session.snapshot().len(),
        t0,
        t.elapsed(),
        a.state.doc == b.state.doc
    );
}

#[test]
#[ignore]
fn probe_phases() {
    let n: usize = std::env::var("P_N").unwrap().parse().unwrap();
    let schema = Rc::new(Schema::starter_kit());
    let t = std::time::Instant::now();
    let doc = line_with(&schema, n, 8);
    let t_doc = t.elapsed();
    let st = EditorState::create(schema.clone(), doc, plugins());
    let t = std::time::Instant::now();
    let s = session_with_client_id(&st, 11).unwrap();
    let t_sess = t.elapsed();
    let t = std::time::Instant::now();
    let snap = s.snapshot();
    let t_snap = t.elapsed();
    let t = std::time::Instant::now();
    let s2 = session_from_bytes_with_client_id(&snap, 22).unwrap();
    let t_load = t.elapsed();
    let t = std::time::Instant::now();
    let _ = s2.projected_doc(&schema).unwrap();
    let t_proj = t.elapsed();
    // HTML paste of the same image
    let html = {
        let mut h = String::from("<p><img src=\"a.png\"");
        for i in 0..n {
            h.push_str(&format!(" data-a{i}=\"xxxxxxxx\""));
        }
        h.push_str("></p>");
        h
    };
    let t = std::time::Instant::now();
    let sl = rinch_editor_core::serialize::slice_from_html(&schema, &html).unwrap();
    let t_paste = t.elapsed();
    eprintln!(
        "PROBE n={n}: doc {t_doc:?} session {t_sess:?} snapshot {t_snap:?} load {t_load:?} project {t_proj:?} paste {t_paste:?} kept {}",
        sl.content.child(0).child(0).attrs().iter().count()
    );
}
