//! A second identity differential (review 2 of #1503), with **identical** pictures:
//! no `title` tells them apart, so equal `@atom` values meet and yrs's formatting
//! inheritance between them is exercised. Three peers (one goes offline for a stretch
//! and reconciles through `sync_diff`), and a history of typing, Enter, joins, pictures
//! inserted next to identical ones, hard breaks beside pictures, cut and paste, copy and
//! paste, copy-then-move, drags, deletes with and without undo, undo, undo-redo, alt
//! changes, same-`src` paste-over and data-refs. Then A sets `data-ref = TARGET` on one picture
//! while B makes one more edit.
//!
//! The oracle is node identity in B's model: the target is the node B's edit kept
//! (`Node::same_ref`), or, when B's edit rebuilt it in place (an attr change, an undo
//! of one, a paste-over with its `src`), the picture its position maps to through B's
//! transactions, if it has the target's `src` ([`target_flags`]).
//! **Wrong** (asserted none): TARGET on any other picture. **Lost**: TARGET on none
//! (counted; cut then paste and an undo of a delete re-insert the picture as a new one
//! in a later transaction, so they lose it by design). `R2_SEEDS` (default 100, both
//! client-id orders), `R2_ONLY` one seed with its histories printed.
#![allow(dead_code)]
use std::collections::HashMap;
use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::{
    AttrSpec, AttrValue, Attrs, EditorState, Fragment, Mapping, Node, NodeSpec, Plugin, Pos,
    Schema, Selection, SetNodeAttrStep, Slice, Transaction, default_plugins,
};

const SRC: &str = "app-blob:6f1c2a0e/b3-9f86d081884c7d65";
const ID_ORDERS: [(u64, u64); 2] = [(11, 22), (22, 11)];

/// `doc > paragraph+ > (text | image | hard_break)*`, with `image` carrying `src`,
/// `alt`, `title` and, when `app` is set, `data-ref` and `width`.
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
            spec.data_attrs = true;
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
                image(s, "app-blob:two", "second"),
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

// --- the differential ---------------------------------------------------------

struct R(u64);

thread_local! {
    /// The position mapping of every local transaction made since it was last taken
    /// ([`take_mappings`]): the oracle maps the target through B's edit with it.
    static MAPPINGS: std::cell::RefCell<Vec<Mapping>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn take_mappings() -> Vec<Mapping> {
    MAPPINGS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Which of `after`'s pictures is the target, the node at `pos` before a peer's edit
/// whose transactions' mappings are `maps`: the node `same_ref` to it (an edit that
/// kept it, wherever it moved), else the picture its position maps to, when the edit
/// kept that one position's worth of content there and the picture has the target's
/// `src` (an edit that rebuilt it in place: an attr change, an undo of one, a paste-over
/// with its `src`). Mapping the position, not taking "the picture at its index", is
/// what keeps a reorder in the same edit from naming another picture (review 3).
fn target_flags(target: &Node, pos: usize, after: &Node, maps: &[Mapping]) -> Vec<bool> {
    let pics = imgs(after);
    let mut flags: Vec<bool> = pics.iter().map(|(_, n)| n.same_ref(target)).collect();
    if flags.iter().any(|f| *f) {
        return flags;
    }
    let (mut start, mut end) = (pos, pos + 1);
    for m in maps {
        start = m.map(start, 1);
        end = m.map(end, -1);
    }
    if end == start + 1
        && let Some(i) = pics
            .iter()
            .position(|(p, n)| *p == start && n.attrs().get("src") == target.attrs().get("src"))
    {
        flags[i] = true;
    }
    flags
}

impl Peer {
    fn try_local(&mut self, f: impl FnOnce(&mut Transaction)) -> bool {
        let mut tr = self.state.tr();
        f(&mut tr);
        MAPPINGS.with(|m| m.borrow_mut().push(tr.mapping().clone()));
        let before = self.state.doc.clone();
        let after = self.state.apply(tr);
        let ok = self
            .session
            .record_local(self.state.schema(), &before, &after.doc)
            .is_ok();
        self.state = after;
        ok
    }
    fn cmd(&mut self, name: &str) -> bool {
        let Some(c) = self.state.command(name) else {
            return false;
        };
        let Some((next, mapping)) = self.state.run_command_mapped(&c) else {
            return false;
        };
        MAPPINGS.with(|m| m.borrow_mut().push(mapping));
        let before = self.state.doc.clone();
        let ok = self
            .session
            .record_local(self.state.schema(), &before, &next.doc)
            .is_ok();
        self.state = next;
        ok
    }
}

/// (pos, node) of every image.
fn imgs(doc: &Node) -> Vec<(usize, Node)> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
        if n.type_name() == "image" {
            out.push((pos, n.clone()));
        }
        true
    });
    out
}
fn ipos(r: &mut R, doc: &Node) -> usize {
    inline_pos(r, doc)
}
fn same_src(s: &Schema, src: &str) -> Node {
    image(s, src, "")
}

const OPS2: &[&str] = &[
    "type",
    "enter",
    "join",
    "ins-adj-identical",
    "ins-identical",
    "del-other",
    "cut-paste-target",
    "cut-paste-other",
    "copy-paste-target",
    "copy-then-move-target",
    "del-target-undo",
    "del-other-undo",
    "undo",
    "undo-redo",
    "alt-target",
    "alt-other",
    "drag-target",
    "paste-over-other-same-src",
    "paste-over-target-same-src",
    "hard-break-adj",
];

/// Returns the op name. `target`: the node to track (may be None in history).
fn op2(r: &mut R, s: &Schema, p: &mut Peer, target: Option<&Node>) -> &'static str {
    let doc = p.state.doc.clone();
    let all = imgs(&doc);
    let tgt = target.and_then(|t| all.iter().find(|(_, n)| n.same_ref(t)).cloned());
    let others: Vec<(usize, Node)> = all
        .iter()
        .filter(|(_, n)| target.is_none_or(|t| !n.same_ref(t)))
        .cloned()
        .collect();
    let pick_other = |r: &mut R| -> Option<(usize, Node)> {
        if others.is_empty() {
            None
        } else {
            Some(others[r.below(others.len())].clone())
        }
    };
    let pick_t = |r: &mut R| -> Option<(usize, Node)> {
        tgt.clone().or_else(|| {
            if all.is_empty() {
                None
            } else {
                Some(all[r.below(all.len())].clone())
            }
        })
    };
    let op = OPS2[r.below(OPS2.len())];
    match op {
        "type" => {
            let at = ipos(r, &doc);
            p.try_local(|tr| {
                tr.set_selection(Selection::cursor(Pos(at)));
                tr.insert_text("x").unwrap();
            });
        }
        "enter" => {
            let at = ipos(r, &doc);
            p.try_local(|tr| {
                tr.split(at, 1, None).unwrap();
            });
        }
        "join" => {
            let ps = paras(&doc);
            if ps.len() < 2 {
                return "noop";
            }
            let k = 1 + r.below(ps.len() - 1);
            let open = ps[k].0 - 1;
            p.try_local(|tr| {
                tr.delete(open - 1, open + 1).unwrap();
            });
        }
        "ins-adj-identical" | "hard-break-adj" => {
            let Some((at, n)) = pick_t(r) else {
                return "noop";
            };
            let at = at + r.below(2);
            let new = if op == "hard-break-adj" {
                s.branch("hard_break", Fragment::empty()).unwrap()
            } else {
                s.create_node("image", n.attrs().without("data-ref"), Fragment::empty())
                    .unwrap()
            };
            p.try_local(|tr| {
                tr.replace(at, at, Slice::new(Fragment::from_node(new), 0, 0))
                    .unwrap();
            });
        }
        "ins-identical" => {
            let at = ipos(r, &doc);
            let n = same_src(s, "s1");
            p.try_local(|tr| {
                tr.replace(at, at, Slice::new(Fragment::from_node(n), 0, 0))
                    .unwrap();
            });
        }
        "del-other" | "del-other-undo" => {
            let Some((at, _)) = pick_other(r) else {
                return "noop";
            };
            p.try_local(|tr| {
                tr.delete(at, at + 1).unwrap();
            });
            if op == "del-other-undo" {
                p.cmd("undo");
            }
        }
        "del-target-undo" => {
            let Some((at, _)) = pick_t(r) else {
                return "noop";
            };
            p.try_local(|tr| {
                tr.delete(at, at + 1).unwrap();
            });
            p.cmd("undo");
        }
        "cut-paste-target" | "cut-paste-other" => {
            let Some((at, n)) = (if op == "cut-paste-target" {
                pick_t(r)
            } else {
                pick_other(r)
            }) else {
                return "noop";
            };
            p.try_local(|tr| {
                tr.delete(at, at + 1).unwrap();
            });
            let d = p.state.doc.clone();
            let to = ipos(r, &d);
            p.try_local(|tr| {
                tr.replace(to, to, Slice::new(Fragment::from_node(n), 0, 0))
                    .unwrap();
            });
        }
        "copy-paste-target" => {
            let Some((_, n)) = pick_t(r) else {
                return "noop";
            };
            let to = ipos(r, &doc);
            p.try_local(|tr| {
                tr.replace(to, to, Slice::new(Fragment::from_node(n), 0, 0))
                    .unwrap();
            });
        }
        "copy-then-move-target" => {
            let Some((_, n)) = pick_t(r) else {
                return "noop";
            };
            let to = ipos(r, &doc);
            p.try_local(|tr| {
                tr.replace(to, to, Slice::new(Fragment::from_node(n.clone()), 0, 0))
                    .unwrap();
            });
            // now drag the first of the two copies elsewhere
            let d = p.state.doc.clone();
            let Some((from, m)) = imgs(&d).into_iter().find(|(_, x)| x.same_ref(&n)) else {
                return op;
            };
            p.try_local(|tr| {
                tr.delete(from, from + 1).unwrap();
                let d2 = tr.doc().clone();
                let at = inline_pos(r, &d2);
                tr.replace(at, at, Slice::new(Fragment::from_node(m), 0, 0))
                    .unwrap();
            });
        }
        "drag-target" => {
            let Some((from, n)) = pick_t(r) else {
                return "noop";
            };
            p.try_local(|tr| {
                tr.delete(from, from + 1).unwrap();
                let d2 = tr.doc().clone();
                let at = inline_pos(r, &d2);
                tr.replace(at, at, Slice::new(Fragment::from_node(n), 0, 0))
                    .unwrap();
            });
        }
        "undo" => {
            p.cmd("undo");
        }
        "undo-redo" => {
            p.cmd("undo");
            p.cmd("redo");
        }
        "alt-target" | "alt-other" => {
            let Some((at, _)) = (if op == "alt-target" {
                pick_t(r)
            } else {
                pick_other(r)
            }) else {
                return "noop";
            };
            let v = AttrValue::from(format!("alt{}", r.below(3)));
            p.try_local(|tr| {
                tr.step(Box::new(SetNodeAttrStep::new(at, "alt", v)))
                    .unwrap();
            });
        }
        "paste-over-other-same-src" | "paste-over-target-same-src" => {
            let Some((at, n)) = (if op == "paste-over-other-same-src" {
                pick_other(r)
            } else {
                pick_t(r)
            }) else {
                return "noop";
            };
            let new = same_src(s, n.attrs().get_str("src").unwrap_or("s1"));
            p.try_local(|tr| {
                tr.replace(at, at + 1, Slice::new(Fragment::from_node(new), 0, 0))
                    .unwrap();
            });
        }
        _ => unreachable!(),
    }
    op
}

fn sync3(a: &mut Peer, b: &mut Peer, c: &mut Peer) {
    sync(a, b);
    sync(b, c);
    sync(a, c);
    sync(a, b);
}

#[derive(Default, Debug)]
struct T2 {
    checks: usize,
    wrong: Vec<String>,
    lost: Vec<String>,
    structural: Vec<String>,
    copies_both: usize,
    stall: usize,
}

fn run2(seed: u64, ids: (u64, u64), v: bool, t: &mut T2) {
    let s = schema(true);
    let mut r = R(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let doc = s
        .branch(
            "doc",
            Fragment::from_children(vec![
                para(
                    &s,
                    vec![
                        s.text("ab").unwrap(),
                        same_src(&s, "s1"),
                        same_src(&s, "s1"),
                        s.text("cd").unwrap(),
                    ],
                ),
                para(
                    &s,
                    vec![
                        s.text("ef").unwrap(),
                        same_src(&s, "s1"),
                        s.text("gh").unwrap(),
                        same_src(&s, "s2"),
                    ],
                ),
            ]),
        )
        .unwrap();
    let (mut a, mut b) = two_peers(&s, doc, ids);
    let mut c = peer_from_bytes(&s, &a.session.snapshot(), 33);
    // history; C sometimes goes offline for a stretch and reconciles via sync_diff
    let mut c_off = 0;
    for _ in 0..r.below(14) {
        let who = r.below(3);
        let o = match who {
            0 => op2(&mut r, &s, &mut a, None),
            1 => op2(&mut r, &s, &mut b, None),
            _ => op2(&mut r, &s, &mut c, None),
        };
        if r.chance(25) {
            // a data-ref in history
            let p = match r.below(3) {
                0 => &mut a,
                1 => &mut b,
                _ => &mut c,
            };
            let all = imgs(&p.state.doc);
            if !all.is_empty() {
                let (at, _) = all[r.below(all.len())].clone();
                let v = AttrValue::from(format!("h{}", r.below(5)));
                p.try_local(|tr| {
                    tr.step(Box::new(SetNodeAttrStep::new(at, "data-ref", v)))
                        .unwrap();
                });
            }
        }
        if v {
            eprintln!("  hist {who} {o}");
        }
        if c_off == 0 && r.chance(20) {
            c_off = 1 + r.below(4);
        }
        if c_off > 0 {
            sync(&mut a, &mut b);
            c_off -= 1;
        } else {
            sync3(&mut a, &mut b, &mut c);
        }
    }
    sync3(&mut a, &mut b, &mut c);
    if a.state.doc != b.state.doc || a.state.doc != c.state.doc {
        t.structural
            .push(format!("{seed} {ids:?}: history diverged"));
        return;
    }
    let all = imgs(&b.state.doc);
    if all.is_empty() {
        return;
    }
    let k = r.below(all.len());
    let target_b = all[k].1.clone();
    let at_a = imgs(&a.state.doc)[k].0;
    a.try_local(|tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            at_a,
            "data-ref",
            AttrValue::from("TARGET"),
        )))
        .unwrap();
    });
    if v {
        eprintln!(
            "BEFORE target #{k}: A doc {:?}\n imgs {:?}",
            a.state.doc,
            imgs(&a.state.doc)
                .iter()
                .map(|x| x.1.attrs().clone())
                .collect::<Vec<_>>()
        );
    }
    let target_pos = all[k].0;
    take_mappings();
    let op = op2(&mut r, &s, &mut b, Some(&target_b));
    let maps = take_mappings();
    if v {
        eprintln!("B after {op}: {:?}", b.state.doc);
    }
    if v {
        eprintln!(
            "RAW A {:?}\n ENT A {:?}\n RAW B {:?}\n ENT B {:?}",
            raw_atoms(&a.session.snapshot(), 0),
            raw_atom_entries(&a.session.snapshot()),
            raw_atoms(&b.session.snapshot(), 0),
            raw_atom_entries(&b.session.snapshot())
        );
    }
    let expected: Vec<(Node, bool)> = imgs(&b.state.doc)
        .into_iter()
        .map(|(_, n)| n)
        .zip(target_flags(&target_b, target_pos, &b.state.doc, &maps))
        .collect();
    let n_t = expected.iter().filter(|x| x.1).count();
    sync3(&mut a, &mut b, &mut c);
    t.checks += 1;
    if v {
        eprintln!(
            "RAW AFTER {:?}\n ENT {:?}",
            raw_atoms(&a.session.snapshot(), 0),
            raw_atom_entries(&a.session.snapshot())
        );
    }
    let tag = format!("seed {seed} {ids:?} {op}");
    if a.state.doc != b.state.doc || a.state.doc != c.state.doc {
        t.structural.push(format!("{tag}: diverged"));
        return;
    }
    for p in [&a, &b, &c] {
        if p.state.doc != p.session.projected_doc(&s).unwrap() {
            t.structural.push(format!("{tag}: model != crdt"));
            return;
        }
    }
    let got = imgs(&a.state.doc);
    if got.len() != expected.len() {
        t.structural.push(format!(
            "{tag}: {} images, B's model had {}",
            got.len(),
            expected.len()
        ));
        return;
    }
    let mut on_target = 0;
    for (i, (_, n)) in got.iter().enumerate() {
        let has = n.attrs().get_str("data-ref") == Some("TARGET");
        if has && !expected[i].1 {
            t.wrong.push(format!(
                "{tag}: TARGET on image #{i} (target at {:?})",
                expected
                    .iter()
                    .enumerate()
                    .filter(|x| x.1.1)
                    .map(|x| x.0)
                    .collect::<Vec<_>>()
            ));
        }
        if has && expected[i].1 {
            on_target += 1;
        }
    }
    if n_t >= 1 && on_target == 0 {
        t.lost.push(tag.clone());
    }
    if n_t == 2 && on_target == 2 {
        t.copies_both += 1;
    }
    if v {
        eprintln!(
            "{tag}: imgs {:?}",
            got.iter().map(|x| x.1.attrs().clone()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn a_data_ref_never_shows_on_another_identical_picture() {
    let seeds: u64 = std::env::var("R2_SEEDS")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(100);
    let only: Option<u64> = std::env::var("R2_ONLY").ok().and_then(|x| x.parse().ok());
    let mut t = T2::default();
    for seed in 1..=seeds {
        if only.is_some_and(|o| o != seed) {
            continue;
        }
        for ids in ID_ORDERS {
            run2(seed, ids, only.is_some(), &mut t);
        }
    }
    eprintln!(
        "R2 checks {} wrong {} lost {} structural {} copies_both {}",
        t.checks,
        t.wrong.len(),
        t.lost.len(),
        t.structural.len(),
        t.copies_both
    );
    for w in &t.wrong {
        eprintln!("  WRONG {w}");
    }
    for w in t.structural.iter().take(30) {
        eprintln!("  STRUCT {w}");
    }
    let mut lost: HashMap<String, usize> = HashMap::new();
    for l in &t.lost {
        *lost
            .entry(l.split(' ').nth(4).unwrap_or("").to_string())
            .or_default() += 1;
    }
    eprintln!("  lost by op {lost:?}");
    assert!(
        only.is_some() || t.checks > seeds as usize,
        "positive control: {}",
        t.checks
    );
    assert!(t.wrong.is_empty());
}
impl R {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

fn titled(s: &Schema, title: &str, src: &str) -> Node {
    s.create_node(
        "image",
        Attrs::new()
            .with("src", AttrValue::from(src))
            .with("alt", AttrValue::from("a"))
            .with("title", AttrValue::from(title)),
        Fragment::empty(),
    )
    .unwrap()
}

/// (start of content, end of content) of every paragraph.
fn paras(doc: &Node) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
        if n.type_name() == "paragraph" {
            out.push((pos + 1, pos + 1 + n.content_size()));
        }
        true
    });
    out
}

fn inline_pos(r: &mut R, doc: &Node) -> usize {
    let ps = paras(doc);
    let (a, b) = ps[r.below(ps.len())];
    a + r.below(b - a + 1)
}

fn refs_after(
    blocks: Vec<Vec<&str>>,
    target: usize,
    ids: (u64, u64),
    b_op: &dyn Fn(&Schema, &mut Peer),
) -> Vec<String> {
    let s = schema(true);
    let ps: Vec<Node> = blocks
        .iter()
        .map(|items| {
            para(
                &s,
                items
                    .iter()
                    .map(|x| match *x {
                        "img1" => same_src(&s, "s1"),
                        "img2" => same_src(&s, "s2"),
                        t => s.text(t).unwrap(),
                    })
                    .collect(),
            )
        })
        .collect();
    let doc = s.branch("doc", Fragment::from_children(ps)).unwrap();
    let (mut a, mut b) = two_peers(&s, doc, ids);
    let at = imgs(&a.state.doc)[target].0;
    a.try_local(|tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            at,
            "data-ref",
            AttrValue::from("TARGET"),
        )))
        .unwrap();
    });
    b_op(&s, &mut b);
    sync(&mut a, &mut b);
    converged(&a, &b, &s);
    imgs(&a.state.doc)
        .iter()
        .map(|x| x.1.attrs().get_str("data-ref").unwrap_or("-").to_string())
        .collect()
}

#[test]
fn a_hard_break_between_two_identical_pictures_keeps_the_data_ref_on_its_own() {
    for ids in ID_ORDERS {
        let got = refs_after(vec![vec!["ab", "img1", "img1", "cd"]], 0, ids, &|s, b| {
            b.try_local(|tr| {
                tr.replace(
                    4,
                    4,
                    Slice::new(
                        Fragment::from_node(s.branch("hard_break", Fragment::empty()).unwrap()),
                        0,
                        0,
                    ),
                )
                .unwrap();
            });
        });
        eprintln!("M1 {ids:?} {got:?}");
        assert_eq!(got, vec!["TARGET", "-"], "{ids:?}");
    }
}

/// An undo of a delete re-inserts the picture as a new char in a later transaction, so
/// nothing carries its identity: a peer's concurrent data-ref is lost, and never shows on
/// the identical picture beside it.
#[test]
fn a_delete_undone_beside_an_identical_picture_loses_the_data_ref_on_neither() {
    for (layout, t) in [
        (vec!["ab", "img1", "img1", "cd"], 0usize),
        (vec!["ab", "img1", "img1", "cd"], 1),
        (vec!["ab", "img1", "cd", "img1", "ef"], 0),
    ] {
        for ids in ID_ORDERS {
            let got = refs_after(vec![layout.clone()], t, ids, &|_, b| {
                let at = imgs(&b.state.doc)[t].0;
                b.try_local(|tr| {
                    tr.delete(at, at + 1).unwrap();
                });
                assert!(b.cmd("undo"));
            });
            assert_eq!(got, vec!["-"; 2], "{layout:?} {t} {ids:?}");
        }
    }
}

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

/// M3: an `@id` written by one peer leaks onto a picture another peer inserts right
/// after that char at the same time; separating the two later shows the first's
/// attrs on the second.
#[test]
fn an_identity_never_leaks_onto_a_concurrently_inserted_neighbour() {
    let mut fails = Vec::new();
    for ids in ID_ORDERS {
        let s = schema(true);
        let doc = s
            .branch(
                "doc",
                Fragment::from_children(vec![para(
                    &s,
                    vec![
                        s.text("ab").unwrap(),
                        same_src(&s, "s1"),
                        s.text("cd").unwrap(),
                    ],
                )]),
            )
            .unwrap();
        let (mut a, mut b) = two_peers(&s, doc, ids);
        // both press Enter before the picture at once: two copies carrying one identity
        a.try_local(|tr| {
            tr.split(3, 1, None).unwrap();
        });
        b.try_local(|tr| {
            tr.split(3, 1, None).unwrap();
        });
        sync(&mut a, &mut b);
        let all = imgs(&a.state.doc);
        eprintln!("M3 {ids:?} after concurrent splits: {} pictures", all.len());
        // A changes the first copy (fresh identity written into its value); B at once
        // inserts an identical picture right after that copy.
        let (p0, n0) = all[0].clone();
        a.try_local(|tr| {
            tr.step(Box::new(SetNodeAttrStep::new(
                p0,
                "data-ref",
                AttrValue::from("TARGET"),
            )))
            .unwrap();
        });
        let ins = same_src(&s, n0.attrs().get_str("src").unwrap());
        b.try_local(|tr| {
            tr.replace(p0 + 1, p0 + 1, Slice::new(Fragment::from_node(ins), 0, 0))
                .unwrap();
        });
        sync(&mut a, &mut b);
        eprintln!("M3 {ids:?} RAW {:?}", raw_atoms(&a.session.snapshot(), 1));
        let refs1: Vec<String> = imgs(&a.state.doc)
            .iter()
            .map(|x| x.1.attrs().get_str("data-ref").unwrap_or("-").to_string())
            .collect();
        // B types a char between the two pictures.
        let p1 = imgs(&b.state.doc)[1].0;
        b.try_local(|tr| {
            tr.set_selection(Selection::cursor(Pos(p1)));
            tr.insert_text("z").unwrap();
        });
        sync(&mut a, &mut b);
        converged(&a, &b, &s);
        let refs2: Vec<String> = imgs(&a.state.doc)
            .iter()
            .map(|x| x.1.attrs().get_str("data-ref").unwrap_or("-").to_string())
            .collect();
        eprintln!("M3 {ids:?} data-refs after insert {refs1:?}, after typing between {refs2:?}");
        if refs2.iter().filter(|x| *x == "TARGET").count() > 1 {
            fails.push(ids);
        }
    }
    assert!(fails.is_empty(), "{fails:?}");
}

/// B changes the target's `alt` (rebuilding it in place) and, in the same transaction,
/// drags an identical picture in front of it. The oracle maps the target through B's
/// transaction and names the rebuilt picture, never the dragged one (review 3: "the
/// picture at its index" named the dragged one); and A's concurrent data-ref shows on no
/// other picture.
#[test]
fn a_rebuild_and_a_reorder_in_one_transaction_never_move_the_data_ref() {
    for ids in ID_ORDERS {
        let s = schema(true);
        let doc = s
            .branch(
                "doc",
                Fragment::from_children(vec![para(
                    &s,
                    vec![
                        s.text("ab").unwrap(),
                        same_src(&s, "s1"),
                        s.text("cd").unwrap(),
                        same_src(&s, "s1"),
                        s.text("ef").unwrap(),
                    ],
                )]),
            )
            .unwrap();
        let (mut a, mut b) = two_peers(&s, doc, ids);
        let all = imgs(&b.state.doc);
        let (target_pos, target) = all[0].clone();
        let at_a = imgs(&a.state.doc)[0].0;
        a.try_local(|tr| {
            tr.step(Box::new(SetNodeAttrStep::new(
                at_a,
                "data-ref",
                AttrValue::from("TARGET"),
            )))
            .unwrap();
        });
        let (p1, n1) = all[1].clone();
        take_mappings();
        b.try_local(|tr| {
            tr.step(Box::new(SetNodeAttrStep::new(
                target_pos,
                "alt",
                AttrValue::from("changed"),
            )))
            .unwrap();
            tr.delete(p1, p1 + 1).unwrap();
            tr.replace(1, 1, Slice::new(Fragment::from_node(n1.clone()), 0, 0))
                .unwrap();
        });
        let flags = target_flags(&target, target_pos, &b.state.doc, &take_mappings());
        let after: Vec<String> = imgs(&b.state.doc)
            .iter()
            .map(|(_, n)| n.attrs().get_str("alt").unwrap_or("").to_string())
            .collect();
        assert_eq!(after, vec!["", "changed"], "{ids:?}");
        assert_eq!(
            flags,
            vec![false, true],
            "{ids:?}: the oracle names the rebuilt picture"
        );
        sync(&mut a, &mut b);
        converged(&a, &b, &s);
        let got: Vec<(String, String)> = imgs(&a.state.doc)
            .iter()
            .map(|(_, n)| {
                (
                    n.attrs().get_str("alt").unwrap_or("").to_string(),
                    n.attrs().get_str("data-ref").unwrap_or("-").to_string(),
                )
            })
            .collect();
        // The data-ref is on the rebuilt picture or on none, never on the dragged one.
        assert_ne!(got[0].1, "TARGET", "{ids:?}: {got:?}");
        assert_eq!(got[1].0, "changed", "{ids:?}");
    }
}

/// The `atoms` map's read is kept between operations: it must never be served stale.
/// Three peers fill theirs; a `data-ref` change reaches one by an incremental delta and one
/// by a state-vector diff; a keystroke, an attr removal and a late joiner follow
/// (review 3).
#[test]
fn the_atom_map_read_kept_between_operations_is_never_stale() {
    let s = schema(true);
    let doc = s
        .branch(
            "doc",
            Fragment::from_children(vec![para(
                &s,
                vec![
                    s.text("ab").unwrap(),
                    same_src(&s, "s1"),
                    s.text("cd").unwrap(),
                ],
            )]),
        )
        .unwrap();
    let (mut a, mut b) = two_peers(&s, doc, (11, 22));
    let at = imgs(&a.state.doc)[0].0;
    a.try_local(|tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            at,
            "data-ref",
            AttrValue::from("one"),
        )))
        .unwrap();
    });
    sync(&mut a, &mut b);
    let mut c = peer_from_bytes(&s, &a.session.snapshot(), 33);
    for p in [&mut a, &mut b, &mut c] {
        let _ = p.session.projected_doc(&s).unwrap();
        p.try_local(|tr| {
            tr.set_selection(Selection::cursor(Pos(2)));
            tr.insert_text("x").unwrap();
        });
    }
    sync(&mut a, &mut b);
    sync(&mut a, &mut c);
    sync(&mut b, &mut c);
    let at = imgs(&b.state.doc)[0].0;
    b.try_local(|tr| {
        tr.step(Box::new(SetNodeAttrStep::new(
            at,
            "data-ref",
            AttrValue::from("two"),
        )))
        .unwrap();
    });
    let delta = b.session.sync_diff(&a.session.state_vector()).unwrap();
    if let Some(ns) = a.session.integrate_incremental(&a.state, &delta).unwrap() {
        a.state = ns;
    }
    let to_c = b.session.sync_diff(&c.session.state_vector()).unwrap();
    if let Some(ns) = c.session.integrate_incremental(&c.state, &to_c).unwrap() {
        c.state = ns;
    }
    for (n, p) in [("A", &a), ("B", &b), ("C", &c)] {
        let model = imgs(&p.state.doc)[0]
            .1
            .attrs()
            .get_str("data-ref")
            .map(str::to_string);
        let crdt = imgs(&p.session.projected_doc(&s).unwrap())[0]
            .1
            .attrs()
            .get_str("data-ref")
            .map(str::to_string);
        assert_eq!(model.as_deref(), Some("two"), "{n}");
        assert_eq!(crdt.as_deref(), Some("two"), "{n}");
    }
    a.try_local(|tr| {
        tr.set_selection(Selection::cursor(Pos(2)));
        tr.insert_text("y").unwrap();
    });
    let at = imgs(&c.state.doc)[0].0;
    c.try_local(|tr| {
        tr.step(Box::new(SetNodeAttrStep {
            pos: at,
            attr: "data-ref".into(),
            value: None,
        }))
        .unwrap();
    });
    sync(&mut a, &mut c);
    sync(&mut a, &mut b);
    converged(&a, &b, &s);
    let late = peer_from_bytes(&s, &a.session.snapshot(), 44);
    assert_eq!(imgs(&a.state.doc)[0].1.attrs().get("data-ref"), None);
    assert_eq!(imgs(&late.state.doc)[0].1.attrs().get("data-ref"), None);
}
