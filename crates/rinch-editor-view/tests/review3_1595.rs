//! Round-3 review of #1595: a claimed image's mount must never be handed another picture
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

fn spec(seen: &Rc<Seen>) -> NodeViewSpec {
    spec_for("image", seen).matches(|attrs| attrs.get_str("data-badge").is_some())
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

/// The `data-ref`s the live mounts were drawn for.
fn live(seen: &Seen) -> Vec<String> {
    seen.mounts
        .borrow()
        .iter()
        .filter(|(_, _, alive)| *alive)
        .map(|(at, _, _)| at.clone())
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

fn pair(html: &str) -> Pair {
    pair_with(html, spec)
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

const ADJ: &str = r#"<p>ab<img src="a.png" data-badge="new" data-ref="one"><img src="a.png" data-badge="hot" data-ref="two">ef</p>"#;

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

/// Make the node at `to` a copy of (the same `Rc` as) the node at `from`.
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

/// R1: G1 with a shared `Rc`. Two copies of one picture side by side; one
/// transaction deletes the first and renumbers the second; B concurrently notes
/// the first. The note is for a deleted picture: it must not show on the second.
#[test]
fn r1_shared_rc_picture_delete_first_renumber_second() {
    let p = pair(ADJ);
    share(&p.a.handle, 3, 4);
    p.send();
    p.back();
    assert_eq!(html(&p.a.handle), html(&p.b.handle));
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(4, "data-ref", AttrValue::from("renumbered"))
            .ok()?;
        tr.delete(3, 4).ok()?;
        Some(tr)
    }));
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(3, "data-note", AttrValue::from("for-first"))
            .ok()?;
        Some(tr)
    }));
    p.send();
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!(
        "R1 A {a}\nR1 B {b}\nR1 moved_b {:?} live_b {:?} live_a {:?}",
        moved(&p.seen_b),
        live(&p.seen_b),
        live(&p.seen_a)
    );
    assert_eq!(a, b, "converged");
    assert!(
        !a.contains("for-first"),
        "B's note for the deleted copy shows on the kept one: {a}"
    );
}

/// R8 (fix of round 3, F2): R1 with a third copy pasted in the same transaction, so
/// the count of copies is what an edit of one would leave (two before, one after)
/// while the copy the pair could be matched with is gone. Only the transaction's
/// mapping tells (`names_one_place`); the session's count floor does not.
#[test]
fn r8_shared_rc_delete_first_renumber_second_and_paste_a_third() {
    let p = pair(ADJ);
    share(&p.a.handle, 3, 4);
    p.send();
    p.back();
    assert_eq!(html(&p.a.handle), html(&p.b.handle));
    assert!(p.a.handle.update(|st| {
        let copy = st.doc.node_at(3)?;
        let mut tr = st.tr();
        tr.set_node_attr(4, "data-ref", AttrValue::from("renumbered"))
            .ok()?;
        tr.delete(3, 4).ok()?;
        tr.replace(6, 6, Slice::new(Fragment::from_node(copy), 0, 0))
            .ok()?;
        Some(tr)
    }));
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(3, "data-note", AttrValue::from("for-first"))
            .ok()?;
        Some(tr)
    }));
    p.send();
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!("R8 A {a}\nR8 B {b}");
    assert_eq!(a, b, "converged");
    assert!(
        !a.split("<img")
            .any(|t| t.contains("renumbered") && t.contains("for-first")),
        "B's note for the deleted copy shows on the renumbered one: {a}"
    );
}

const EMB: &str = r#"<div data-pm-type="embed" data-ref="one"></div><div data-pm-type="embed" data-ref="two"></div><p>x</p>"#;

/// R2: the same with two embeds.
#[test]
fn r2_shared_rc_embed_delete_first_renumber_second() {
    let p = pair_with(EMB, embed_spec);
    share(&p.a.handle, 0, 1);
    p.send();
    p.back();
    assert_eq!(html(&p.a.handle), html(&p.b.handle));
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(1, "data-ref", AttrValue::from("renumbered"))
            .ok()?;
        tr.delete(0, 1).ok()?;
        Some(tr)
    }));
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(0, "data-note", AttrValue::from("for-first"))
            .ok()?;
        Some(tr)
    }));
    p.send();
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!(
        "R2 A {a}\nR2 B {b}\nR2 moved_b {:?} live_b {:?} live_a {:?}",
        moved(&p.seen_b),
        live(&p.seen_b),
        live(&p.seen_a)
    );
    assert_eq!(a, b, "converged");
    assert!(
        !a.contains("for-first"),
        "B's note for the deleted copy shows on the kept one: {a}"
    );
}

/// R3: Enter at the start of a lone paragraph while a peer types at its end. The
/// peer's character belongs to "abc".
#[test]
fn r3_enter_at_start_of_a_lone_paragraph_keeps_a_peers_typing_with_its_text() {
    let p = pair("<p>abc</p>");
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.split(1, 1, None).ok()?;
        Some(tr)
    }));
    type_at(&p.b.handle, 4, "X");
    p.send();
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!("R3 A {a}\nR3 B {b}");
    assert_eq!(a, b, "converged");
    assert_eq!(a, "<p></p><p>abcX</p>");
}

/// R4: the same in a lone list item (Enter splits the item).
#[test]
fn r4_enter_at_start_of_a_lone_list_item_keeps_a_peers_typing_with_its_text() {
    let p = pair("<ul><li><p>abc</p></li></ul><p>z</p>");
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.split(3, 2, None).ok()?;
        Some(tr)
    }));
    type_at(&p.b.handle, 6, "X");
    p.send();
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!("R4 A {a}\nR4 B {b}");
    assert_eq!(a, b, "converged");
    assert!(a.contains("<li><p>abcX</p></li>"), "{a}");
}

/// R5: a sticky anchor in a lone paragraph follows its character through Enter at
/// the paragraph's start (CLAUDE.md: "Enter at its start keeps them").
#[test]
fn r5_sticky_anchor_survives_enter_at_start_of_a_lone_paragraph() {
    let p = pair("<p>abc</p>");
    let sticky =
        p.a.handle
            .collab_sticky_index(rinch_editor_core::Pos(3))
            .unwrap();
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.split(1, 1, None).ok()?;
        Some(tr)
    }));
    let now = p.a.handle.collab_resolve_sticky(&sticky);
    eprintln!("R5 {now:?} doc {}", html(&p.a.handle));
    // "<p></p><p>abc</p>": 'c' is at 5.
    assert_eq!(now, Some(rinch_editor_core::Pos(5)));
}

/// R6: the identity cache (`note_kept_inline_atoms`) with a shared `Rc`. A holds one
/// picture twice and edits the SECOND copy; B then changes the second copy. A's
/// mount for the first copy must not be handed the change.
#[test]
fn r6_shared_rc_edit_second_copy_then_peer_changes_it() {
    let p = pair(r#"<p>ab<img src="a.png" data-badge="new" data-ref="one">cd</p>"#);
    assert!(p.a.handle.update(|st| {
        let node = st.doc.node_at(3)?;
        let mut tr = st.tr();
        tr.replace(6, 6, Slice::new(Fragment::from_node(node), 0, 0))
            .ok()?;
        Some(tr)
    }));
    p.send();
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(6, "alt", AttrValue::from("edited")).ok()?;
        Some(tr)
    }));
    p.send();
    assert_eq!(html(&p.a.handle), html(&p.b.handle));
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(6, "data-ref", AttrValue::from("end-b"))
            .ok()?;
        Some(tr)
    }));
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!(
        "R6 A {a}\nR6 B {b}\nR6 mounts_a {:?}\nR6 mounts_b {:?}",
        p.seen_a.mounts.borrow(),
        p.seen_b.mounts.borrow()
    );
    assert_eq!(a, b);
    let m = p.seen_a.mounts.borrow();
    let first_live = m.iter().find(|(_, _, alive)| *alive).unwrap();
    assert_eq!(
        first_live.1, "one",
        "A's first copy's mount was handed the second's change: {m:?}"
    );
}

/// R6b (fix of round 3, F2): the identity cache carries the copy that changed. A
/// holds one picture `Rc` twice when it starts hosting (an in-editor copy pasted
/// before), so its session knows both copies' chars under one `Rc`. A sets an
/// attribute of the SECOND copy, and B then changes the FIRST. A must take that as an
/// attribute change of its first copy and keep the mount: carried onto the first
/// entry for the `Rc` (the old `inline_carry`), A's first copy read as the second's
/// char and was remounted.
#[test]
fn r6b_shared_rc_edit_second_copy_then_peer_changes_the_first() {
    let p = pair_prepared(
        r#"<p>ab<img src="a.png" data-badge="new" data-ref="one">cd</p>"#,
        spec,
        |h| share_into(h, 3, 6),
    );
    assert_eq!(html(&p.a.handle), html(&p.b.handle));
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(6, "alt", AttrValue::from("edited")).ok()?;
        Some(tr)
    }));
    p.send();
    assert_eq!(html(&p.a.handle), html(&p.b.handle));
    let renders = p.seen_a.renders.get();
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(3, "data-note", AttrValue::from("b"))
            .ok()?;
        Some(tr)
    }));
    p.back();
    let (a, b) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!("R6b A {a}\nR6b mounts_a {:?}", p.seen_a.mounts.borrow());
    assert_eq!(a, b);
    assert_eq!(
        p.seen_a.renders.get(),
        renders,
        "a peer's attribute change of A's first copy remounted it: {a}"
    );
}

/// Insert a copy of the node at `from` (the same `Rc`) at `at`.
fn share_into(h: &EditorHandle, from: usize, at: usize) {
    assert!(h.update(|st| {
        let node = st.doc.node_at(from)?;
        let mut tr = st.tr();
        tr.replace(at, at, Slice::new(Fragment::from_node(node), 0, 0))
            .ok()?;
        Some(tr)
    }));
}

/// R9 (fix of round 3): a kept node is matched with the copy its `SetNodeAttrStep`
/// named (`Kept::old_pos`). One embed `Rc` at two places; one transaction renumbers
/// the SECOND copy, deletes the FIRST and appends a paragraph (so no run of the diff
/// matches the kept node, and the stretch is matched by identity). The
/// second copy's mount must be the one that lives on: matched by `Rc` alone, the
/// renumbered node took the deleted first copy's descriptor, and its view.
#[test]
fn r9_a_kept_copy_keeps_its_own_mount_not_its_twins() {
    let p = pair_with(EMB, embed_spec);
    share(&p.a.handle, 0, 1);
    let before = p.seen_a.mounts.borrow().len();
    let para =
        p.a.handle
            .state()
            .schema()
            .branch("paragraph", Fragment::from_node(text(&p.a.handle, "n")))
            .unwrap();
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(1, "data-ref", AttrValue::from("renumbered"))
            .ok()?;
        tr.delete(0, 1).ok()?;
        tr.replace(4, 4, Slice::new(Fragment::from_node(para.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    let m = p.seen_a.mounts.borrow();
    eprintln!("R9 {} mounts {m:?}", html(&p.a.handle));
    assert_eq!(m.len(), before, "nothing remounted");
    let alive: Vec<usize> = (0..m.len()).filter(|&i| m[i].2).collect();
    assert_eq!(
        alive,
        vec![before - 1],
        "the second copy's mount lives on, the deleted first copy's goes: {m:?}"
    );
}

/// R7: a load while collaborating keeps the value rule: loading the same document
/// again rewrites no picture, so the peer's view is not remounted.
#[test]
fn r7_a_load_of_the_same_document_remounts_nothing_on_the_peer() {
    let doc = r#"<p>ab<img src="a.png" data-badge="new" data-ref="one">cd</p><p>z</p>"#;
    let p = pair(doc);
    let before = p.seen_b.renders.get();
    assert!(p.a.handle.load_html(doc));
    p.send();
    assert_eq!(html(&p.a.handle), html(&p.b.handle));
    assert_eq!(
        p.seen_b.renders.get(),
        before,
        "a load rewrote the picture as a new char"
    );
}
