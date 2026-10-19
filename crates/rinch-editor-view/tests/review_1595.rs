//! Review of #1595: a claimed image's mount must never be handed another picture
//! (decision (d)). Each mount records the `data-ref` it was rendered for, and an
//! effect records what its `ctx.attrs` says now; a live mount whose `data-ref`
//! changed under it is app state moved onto another picture.
#![cfg(feature = "collaboration")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::on_cleanup;
use rinch_editor_core::{AttrValue, Attrs, Fragment, NodeSelection, Pos, Selection, Slice};
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};

#[derive(Default)]
struct Seen {
    renders: Cell<usize>,
    /// Per mount: (ref at render, ref now, alive).
    mounts: RefCell<Vec<(String, String, bool)>>,
}

fn spec(seen: &Rc<Seen>) -> NodeViewSpec {
    let s = seen.clone();
    NodeViewSpec::new("image", move |scope: &mut RenderScope, ctx: NodeViewCtx| {
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
    .matches(|attrs| attrs.get_str("data-badge").is_some())
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

/// `ab` 1..3, picture 1 at 3, `cd` 4..6, picture 2 at 6, `ef` 7..9. Same `src`.
const TWO: &str = r#"<p>ab<img src="a.png" data-badge="new" data-ref="one">cd<img src="a.png" data-badge="hot" data-ref="two">ef</p>"#;

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
    let a = page(html);
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
    a.handle.register_node_view(spec(&seen_a)).keep();
    let seen_b = Rc::new(Seen::default());
    b.handle.register_node_view(spec(&seen_b)).keep();
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

fn image(h: &EditorHandle, src: &str, badge: &str, r: &str) -> rinch_editor_core::Node {
    h.state()
        .schema()
        .create_node(
            "image",
            Attrs::from_iter([
                ("src", AttrValue::from(src)),
                ("data-badge", AttrValue::from(badge)),
                ("data-ref", AttrValue::from(r)),
            ]),
            Fragment::empty(),
        )
        .unwrap()
}

/// R1: A replaces picture "one" with a new picture of the same `src` (an app's
/// own "replace picture" with a fresh node). B's mount for "one" must not now
/// show "three".
#[test]
fn r1_a_fresh_same_src_picture_put_over_a_claimed_one_is_another_mount_on_the_peer() {
    let p = pair(TWO);
    let fresh = image(&p.a.handle, "a.png", "new", "three");
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(3, 4, Slice::new(Fragment::from_node(fresh.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    assert!(moved(&p.seen_a).is_empty(), "local: {:?}", moved(&p.seen_a));
    let renders_b = p.seen_b.renders.get();
    p.send();
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(
        moved(&p.seen_b).is_empty(),
        "peer: a mount drawn for one picture now shows another: {:?} (renders {})",
        moved(&p.seen_b),
        p.seen_b.renders.get()
    );
    // Decision (f): the pasted picture is a new char, so the peer's mount drawn for
    // "one" goes and a new one is drawn for what was pasted — remounted, never handed
    // it. (The remote path replaces the block holding a replaced char, so the other
    // picture of the paragraph is remounted too: `renders` grows by two.)
    assert!(
        p.seen_b.renders.get() > renders_b,
        "the peer remounts the view for the pasted picture"
    );
    assert!(
        !live(&p.seen_b).iter().any(|r| r == "one"),
        "no live mount on the peer is the one drawn for the picture pasted over: {:?}",
        live(&p.seen_b)
    );
}

/// R1b: the same through the real copy and paste: picture "two" copied and
/// pasted over node-selected picture "one".
#[test]
fn r1b_a_copied_same_src_picture_pasted_over_a_claimed_one() {
    let p = pair(TWO);
    let a = &p.a.handle;
    a.set_selection(Selection::Node(NodeSelection {
        anchor: Pos(6),
        head: Pos(7),
    }));
    let (copied, _text) = a.selection_clipboard().expect("copy");
    a.set_selection(Selection::Node(NodeSelection {
        anchor: Pos(3),
        head: Pos(4),
    }));
    assert!(a.replace_selection_with_html(&copied), "pasted");
    assert!(moved(&p.seen_a).is_empty(), "local: {:?}", moved(&p.seen_a));
    let renders_b = p.seen_b.renders.get();
    p.send();
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(
        moved(&p.seen_b).is_empty(),
        "peer: {:?} (renders {})\n{}",
        moved(&p.seen_b),
        p.seen_b.renders.get(),
        html(&p.b.handle)
    );
    // Decision (f): the pasted picture is a new char, so the peer's mount drawn for
    // "one" goes and a new one is drawn for what was pasted — remounted, never handed
    // it. (The remote path replaces the block holding a replaced char, so the other
    // picture of the paragraph is remounted too: `renders` grows by two.)
    assert!(
        p.seen_b.renders.get() > renders_b,
        "the peer remounts the view for the pasted picture"
    );
    assert!(
        !live(&p.seen_b).iter().any(|r| r == "one"),
        "no live mount on the peer is the one drawn for the picture pasted over: {:?}",
        live(&p.seen_b)
    );
}

/// R2: A swaps the two pictures (same `src`, different attrs) in one transaction,
/// each the same node moved.
#[test]
fn r2_two_same_src_pictures_swapped_in_one_transaction() {
    let p = pair(TWO);
    let doc = p.a.handle.doc();
    let (one, two) = (
        doc.node_at(3).unwrap().clone(),
        doc.node_at(6).unwrap().clone(),
    );
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(6, 7, Slice::new(Fragment::from_node(one.clone()), 0, 0))
            .ok()?;
        tr.replace(3, 4, Slice::new(Fragment::from_node(two.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    assert!(moved(&p.seen_a).is_empty(), "local: {:?}", moved(&p.seen_a));
    p.send();
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(
        moved(&p.seen_b).is_empty(),
        "peer: {:?} (renders {})",
        moved(&p.seen_b),
        p.seen_b.renders.get()
    );
}

/// R3: A changes the claimed picture's `src` (a SetNodeAttrStep): #1503 makes a
/// `src` change a new picture. The data-ref is the same, so no "moved" — but
/// record whether the peer remounts (informational).
#[test]
fn r3_a_src_change_on_the_peer() {
    let p = pair(TWO);
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.step(Box::new(rinch_editor_core::SetNodeAttrStep::new(
            3,
            "src",
            AttrValue::from("z.png"),
        )))
        .ok()?;
        Some(tr)
    }));
    p.send();
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    eprintln!(
        "R3 renders on b: {} (2 = kept, 3 = remount)",
        p.seen_b.renders.get()
    );
    assert!(moved(&p.seen_b).is_empty());
}

/// R4: cut picture one, paste it after "ef" (two transactions), then concurrent:
/// B sets data-badge on what it holds.
#[test]
fn r4_cut_and_paste_then_a_concurrent_attr_change() {
    let p = pair(TWO);
    let a = &p.a.handle;
    a.set_selection(Selection::Node(NodeSelection {
        anchor: Pos(3),
        head: Pos(4),
    }));
    let (cut, _) = a.selection_clipboard().unwrap();
    assert!(a.command("deleteSelection"));
    a.set_selection(Selection::cursor(Pos(7)));
    assert!(a.replace_selection_with_html(&cut));
    // B, concurrently, changes picture one's badge.
    let ctx_changed = p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.step(Box::new(rinch_editor_core::SetNodeAttrStep::new(
            3,
            "data-badge",
            AttrValue::from("x"),
        )))
        .ok()?;
        Some(tr)
    });
    assert!(ctx_changed);
    p.send();
    p.back();
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(moved(&p.seen_a).is_empty(), "a: {:?}", moved(&p.seen_a));
    assert!(moved(&p.seen_b).is_empty(), "b: {:?}", moved(&p.seen_b));
}

/// R5: two identical pictures (same src and attrs but data-ref); A deletes the
/// first. B's remaining mount must be the second's.
#[test]
fn r5_the_first_of_two_pictures_deleted() {
    let p = pair(TWO);
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(3, 4).ok()?;
        Some(tr)
    }));
    p.send();
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(moved(&p.seen_b).is_empty(), "b: {:?}", moved(&p.seen_b));
}

/// R6: A undoes after replacing one with a fresh same-src picture; B follows both.
#[test]
fn r6_replace_then_undo() {
    let p = pair(TWO);
    let fresh = image(&p.a.handle, "a.png", "new", "three");
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(3, 4, Slice::new(Fragment::from_node(fresh.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    p.send();
    assert!(p.a.handle.command("undo"));
    p.send();
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(moved(&p.seen_a).is_empty(), "a: {:?}", moved(&p.seen_a));
    assert!(moved(&p.seen_b).is_empty(), "b: {:?}", moved(&p.seen_b));
}

/// R7: A replaces picture "one" with a picture of ANOTHER `src` (a fresh node):
/// a new char by #1503, another identity — the peer must remount.
#[test]
fn r7_a_different_src_picture_put_over_a_claimed_one() {
    let p = pair(TWO);
    let fresh = image(&p.a.handle, "z.png", "new", "three");
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(3, 4, Slice::new(Fragment::from_node(fresh.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    p.send();
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(moved(&p.seen_b).is_empty(), "peer: {:?}", moved(&p.seen_b));
}
