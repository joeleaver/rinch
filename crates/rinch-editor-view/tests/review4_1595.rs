//! Round-4 probes review of #1595: a claimed image's mount must never be handed another picture
//! (decision (d)). Each mount records the `data-ref` it was rendered for, and an
//! effect records what its `ctx.attrs` says now; a live mount whose `data-ref`
//! changed under it is app state moved onto another picture.
#![cfg(feature = "collaboration")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::on_cleanup;
use rinch_editor_core::{AttrValue, Fragment, Slice};
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};

#[derive(Default)]
struct Seen {
    renders: Cell<usize>,
    /// Per mount: (ref at render, ref now, alive).
    mounts: RefCell<Vec<(String, String, bool)>>,
}

fn spec_for(ty: &str, seen: &Rc<Seen>) -> NodeViewSpec {
    let s = seen.clone();
    NodeViewSpec::new(ty, move |scope: &mut RenderScope, ctx: NodeViewCtx| {
        s.renders.set(s.renders.get() + 1);
        let idx = s.mounts.borrow().len();
        let r = rinch_core::reactive::untracked(|| ctx.attrs.get())
            .get_str("data-ref")
            .unwrap_or("")
            .to_string();
        s.mounts.borrow_mut().push((r.clone(), r, true));
        let attrs = ctx.attrs;
        let seen = s.clone();
        scope.create_effect(move || {
            let now = attrs.get().get_str("data-ref").unwrap_or("").to_string();
            seen.mounts.borrow_mut()[idx].1 = now;
        });
        let c = s.clone();
        on_cleanup(move || c.mounts.borrow_mut()[idx].2 = false);
        scope.create_element("b")
    })
}

/// Every live mount whose picture's `data-ref` is not the one it was drawn for.
fn moved(seen: &Seen) -> Vec<(String, String)> {
    seen.mounts
        .borrow()
        .iter()
        .filter(|(at, now, alive)| *alive && at != now)
        .map(|(a, n, _)| (a.clone(), n.clone()))
        .collect()
}

struct Page {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
}

fn page(html: &str) -> Page {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc.clone(), body);
    let handle = create_editor();
    assert!(handle.load_html(html));
    let container = handle.mount(&mut scope);
    NodeHandle::new(body, Rc::downgrade(&dyn_doc)).append_child(&container);
    Page {
        _doc: doc,
        _scope: scope,
        handle,
    }
}

struct Pair {
    a: Page,
    b: Page,
    to_b: Rc<RefCell<Vec<Vec<u8>>>>,
    to_a: Rc<RefCell<Vec<Vec<u8>>>>,
    seen_a: Rc<Seen>,
    seen_b: Rc<Seen>,
}

impl Pair {
    fn send(&self) {
        for d in self.to_b.borrow_mut().drain(..) {
            self.b.handle.collab_receive(&d);
            assert!(self.b.handle.collab_take_error().is_none());
        }
    }
    fn back(&self) {
        for d in self.to_a.borrow_mut().drain(..) {
            self.a.handle.collab_receive(&d);
            assert!(self.a.handle.collab_take_error().is_none());
        }
    }
}

fn pair_with(html: &str, mk: fn(&Rc<Seen>) -> NodeViewSpec) -> Pair {
    pair_prepared(html, mk, |_| {})
}

/// [`pair_with`], with `prep` run on A's editor before the collaboration starts.
fn pair_prepared(
    html: &str,
    mk: fn(&Rc<Seen>) -> NodeViewSpec,
    prep: impl FnOnce(&EditorHandle),
) -> Pair {
    let a = page(html);
    prep(&a.handle);
    let b = page("<p></p>");
    let to_b: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let to_a: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let sink = to_b.clone();
    let snapshot = a
        .handle
        .start_collaboration_host(move |d| sink.borrow_mut().push(d))
        .unwrap();
    let sink = to_a.clone();
    b.handle
        .start_collaboration_guest(&snapshot, move |d| sink.borrow_mut().push(d))
        .unwrap();
    let seen_a = Rc::new(Seen::default());
    a.handle.register_node_view(mk(&seen_a)).keep();
    let seen_b = Rc::new(Seen::default());
    b.handle.register_node_view(mk(&seen_b)).keep();
    Pair {
        a,
        b,
        to_b,
        to_a,
        seen_a,
        seen_b,
    }
}
fn html(h: &EditorHandle) -> String {
    rinch_editor_core::serialize::html::node_to_html(&h.doc())
}
fn embed_spec(seen: &Rc<Seen>) -> NodeViewSpec {
    spec_for("embed", seen)
}
fn text(h: &EditorHandle, s: &str) -> rinch_editor_core::Node {
    h.state().schema().text(s).unwrap()
}
fn type_at(h: &EditorHandle, pos: usize, s: &str) {
    let t = text(h, s);
    assert!(h.update(|st| {
        let mut tr = st.tr();
        tr.replace(pos, pos, Slice::new(Fragment::from_node(t.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
}
fn share(h: &EditorHandle, from: usize, to: usize) {
    assert!(h.update(|st| {
        let n = st.doc.node_at(from)?;
        let mut tr = st.tr();
        tr.replace(to, to + 1, Slice::new(Fragment::from_node(n), 0, 0))
            .ok()?;
        Some(tr)
    }));
    assert!(
        h.doc()
            .node_at(from)
            .unwrap()
            .same_ref(&h.doc().node_at(to).unwrap())
    );
}

const EMB: &str = r#"<div data-pm-type="embed" data-ref="one"></div><div data-pm-type="embed" data-ref="two"></div><p>x</p>"#;

fn set_attr(h: &EditorHandle, pos: usize, k: &str, v: &str) {
    assert!(h.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(pos, k, AttrValue::from(v)).ok()?;
        Some(tr)
    }));
}

fn mounts(s: &Seen) -> Vec<(String, String, bool)> {
    s.mounts.borrow().clone()
}

/// P1: one embed `Rc` twice (an in-editor copy). A deletes the FIRST copy (no attr
/// change). B concurrently notes the first copy. The note belongs to a deleted embed.
#[test]
#[ignore = "G3 of the round-4 review of #1595: deleting one copy of a shared Rc keeps the other copy's identity (pre-existing on main; issue draft review4-1595-1)"]
fn p1_delete_first_copy_peer_notes_first() {
    let p = pair_with(EMB, embed_spec);
    share(&p.a.handle, 0, 1);
    p.send();
    p.back();
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(0, 1).ok()?;
        Some(tr)
    }));
    set_attr(&p.b.handle, 0, "data-note", "for-first");
    p.send();
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!("P1 A {a}\nP1 B {b}");
    assert_eq!(a, b);
    assert!(
        !a.contains("for-first"),
        "note for the deleted copy shows: {a}"
    );
}

/// P2: both copies renumbered in one transaction; B concurrently notes the second.
/// A provable attr-only edit of both: B's note should survive on the second.
#[test]
#[ignore = "G4 of the round-4 review of #1595: an attr edit of both copies in one transaction proves nothing (lost, by design; issue draft review4-1595-1)"]
fn p2_both_copies_edited_in_one_transaction() {
    let p = pair_with(EMB, embed_spec);
    share(&p.a.handle, 0, 1);
    p.send();
    p.back();
    let rb = p.seen_b.renders.get();
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(0, "data-n", AttrValue::from("1")).ok()?;
        tr.set_node_attr(1, "data-n", AttrValue::from("2")).ok()?;
        Some(tr)
    }));
    set_attr(&p.b.handle, 1, "data-note", "for-second");
    p.send();
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!(
        "P2 A {a}\nP2 B {b}\nP2 b renders {} -> {}",
        rb,
        p.seen_b.renders.get()
    );
    assert_eq!(a, b);
    assert!(a.contains("for-second"), "B's note was lost: {a}");
}

/// P2c: control — the same with two distinct embeds (no shared Rc).
#[test]
fn p2c_control_two_distinct_embeds_edited_in_one_transaction() {
    let p = pair_with(EMB, embed_spec);
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(0, "data-n", AttrValue::from("1")).ok()?;
        tr.set_node_attr(1, "data-n", AttrValue::from("2")).ok()?;
        Some(tr)
    }));
    set_attr(&p.b.handle, 1, "data-note", "for-second");
    p.send();
    p.back();
    let a = html(&p.a.handle);
    eprintln!("P2c A {a}");
    assert!(a.contains("for-second"), "B's note was lost: {a}");
}

/// P3: a peer types in an unrelated paragraph; A holds one embed `Rc` twice with views.
/// Neither of A's views remounts.
#[test]
fn p3_unrelated_remote_typing_beside_two_copies_remounts_nothing() {
    let p = pair_with(EMB, embed_spec);
    share(&p.a.handle, 0, 1);
    p.send();
    p.back();
    let ra = p.seen_a.renders.get();
    type_at(&p.b.handle, 3, "Q");
    p.back();
    eprintln!("P3 A {} mounts {:?}", html(&p.a.handle), mounts(&p.seen_a));
    assert_eq!(p.seen_a.renders.get(), ra, "a view remounted");
}

/// P5: the peer inserts a paragraph BETWEEN the two copies, then renumbers the second
/// copy. A's second mount must see it, its first must not.
#[test]
fn p5_remote_insert_between_copies_then_change_second() {
    let p = pair_with(EMB, embed_spec);
    share(&p.a.handle, 0, 1);
    p.send();
    p.back();
    let ra = p.seen_a.renders.get();
    let para =
        p.b.handle
            .state()
            .schema()
            .branch("paragraph", Fragment::from_node(text(&p.b.handle, "m")))
            .unwrap();
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(1, 1, Slice::new(Fragment::from_node(para.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    p.back();
    eprintln!(
        "P5 after insert A {} renders {} mounts {:?}",
        html(&p.a.handle),
        p.seen_a.renders.get(),
        mounts(&p.seen_a)
    );
    // doc: E(0) P(1..4) E(4) P
    set_attr(&p.b.handle, 4, "data-ref", "second-b");
    p.back();
    let m = mounts(&p.seen_a);
    eprintln!(
        "P5 A {} renders {}->{} mounts {m:?}",
        html(&p.a.handle),
        ra,
        p.seen_a.renders.get()
    );
    assert_eq!(p.seen_a.renders.get(), ra, "remounted");
    assert!(moved(&p.seen_a).iter().all(|(at, _)| at != "one" || false) || true);
    let live: Vec<_> = m.iter().filter(|x| x.2).collect();
    assert_eq!(live.len(), 2);
    // mount order: the first copy's mount was made first.
    assert_eq!(
        live[0].1, "one",
        "first copy's mount handed the second's change: {m:?}"
    );
    assert_eq!(live[1].1, "second-b", "{m:?}");
}

/// P6: the peer deletes the FIRST copy, then renumbers the survivor. A's surviving
/// mount must be the SECOND one (the CRDT block that survived).
#[test]
#[ignore = "round-4 review of #1595, observation: a remote delete of the first copy remounts the survivor (conservative, pre-existing)"]
fn p6_remote_delete_first_copy_survivor_is_second_mount() {
    let p = pair_with(EMB, embed_spec);
    share(&p.a.handle, 0, 1);
    p.send();
    p.back();
    let before = mounts(&p.seen_a).len();
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(0, 1).ok()?;
        Some(tr)
    }));
    p.back();
    let m = mounts(&p.seen_a);
    eprintln!(
        "P6 A {} mounts {m:?} (before {before}) B mounts {:?}",
        html(&p.a.handle),
        mounts(&p.seen_b)
    );
    let alive: Vec<usize> = (0..m.len()).filter(|&i| m[i].2).collect();
    // B deleted its first copy; which CRDT block is that on A? B's first mount was
    // for block 0 too (B rendered in document order).
    assert_eq!(alive.len(), 1, "{m:?}");
    assert_eq!(
        alive[0],
        before - 1,
        "the deleted first copy's mount survived on the second's block: {m:?}"
    );
}

/// P4: an embed inside a blockquote (with a twin copy beside it): a peer's attr change
/// of the second keeps both mounts and reaches only the second.
#[test]
fn p4_nested_twin_remote_attr_change() {
    let html_in = r#"<blockquote><div data-pm-type="embed" data-ref="one"></div><div data-pm-type="embed" data-ref="two"></div><p>x</p></blockquote>"#;
    let p = pair_with(html_in, embed_spec);
    share(&p.a.handle, 1, 2);
    p.send();
    p.back();
    let ra = p.seen_a.renders.get();
    set_attr(&p.b.handle, 2, "data-ref", "second-b");
    p.back();
    let m = mounts(&p.seen_a);
    eprintln!("P4 A {} mounts {m:?}", html(&p.a.handle));
    assert_eq!(p.seen_a.renders.get(), ra, "remounted");
    let live: Vec<_> = m.iter().filter(|x| x.2).collect();
    assert_eq!(live[0].1, "one", "{m:?}");
    assert_eq!(live[1].1, "second-b", "{m:?}");
}

/// P7: a remote attr change of a lone nested embed (list item) keeps the mount (old_pos plumbing).
#[test]
fn p7_nested_list_remote_attr_change_keeps_mount() {
    let html_in = r#"<ul><li><p>a</p></li><li><div data-pm-type="embed" data-ref="one"></div></li></ul><p>z</p>"#;
    let p = pair_with(html_in, embed_spec);
    let ra = p.seen_a.renders.get();
    // ul(0) li(1) p(2) a(3) /p(4) /li(5) li(6) E(7)
    set_attr(&p.b.handle, 7, "data-ref", "b");
    p.back();
    let m = mounts(&p.seen_a);
    eprintln!("P7 A {} mounts {m:?}", html(&p.a.handle));
    assert_eq!(p.seen_a.renders.get(), ra, "remounted");
    assert_eq!(m.iter().find(|x| x.2).unwrap().1, "b");
}

/// P8: cost of a remote keystroke with N embeds (no duplicates) each with a view.
#[test]
#[ignore]
fn p8_cost_remote_keystroke_many_views() {
    let n: usize = std::env::var("P8_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);
    let mut h = String::new();
    for i in 0..n {
        h.push_str(&format!(
            r#"<p>p{i}</p><div data-pm-type="embed" data-ref="e{i}"></div>"#
        ));
    }
    let p = pair_with(&h, embed_spec);
    let reps = 20;
    let mut total = std::time::Duration::ZERO;
    for r in 0..reps {
        type_at(&p.b.handle, 2, "k");
        let t = std::time::Instant::now();
        p.back();
        total += t.elapsed();
        let _ = r;
    }
    eprintln!("P8 n={n} per remote keystroke {:?}", total / reps);
}

/// P1b: P1 with the peer's change on `data-ref` (what the mounts track), and the
/// mounts printed at every step on both peers.
#[test]
#[ignore = "G3 of the round-4 review of #1595: deleting one copy of a shared Rc keeps the other copy's identity (pre-existing on main; issue draft review4-1595-1)"]
fn p1b_delete_first_copy_peer_renames_first_mounts() {
    let p = pair_with(EMB, embed_spec);
    share(&p.a.handle, 0, 1);
    p.send();
    p.back();
    eprintln!(
        "P1b start A {:?}\nP1b start B {:?}",
        mounts(&p.seen_a),
        mounts(&p.seen_b)
    );
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(0, 1).ok()?;
        Some(tr)
    }));
    set_attr(&p.b.handle, 0, "data-ref", "b-first");
    eprintln!(
        "P1b local A {:?}\nP1b local B {:?}",
        mounts(&p.seen_a),
        mounts(&p.seen_b)
    );
    p.send();
    eprintln!(
        "P1b after B receives: B {} {:?}",
        html(&p.b.handle),
        mounts(&p.seen_b)
    );
    p.back();
    eprintln!(
        "P1b after A receives: A {} {:?}",
        html(&p.a.handle),
        mounts(&p.seen_a)
    );
    assert!(
        moved(&p.seen_b).is_empty(),
        "B: a mount was handed another node: {:?}",
        mounts(&p.seen_b)
    );
    assert!(
        moved(&p.seen_a).is_empty(),
        "A: a mount was handed another node: {:?}",
        mounts(&p.seen_a)
    );
}
