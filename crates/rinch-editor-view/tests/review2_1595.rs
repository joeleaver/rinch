//! Round-2 review of #1595: a claimed image's mount must never be handed another picture
//! (decision (d)). Each mount records the `data-ref` it was rendered for, and an
//! effect records what its `ctx.attrs` says now; a live mount whose `data-ref`
//! changed under it is app state moved onto another picture.
#![cfg(feature = "collaboration")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::on_cleanup;
use rinch_editor_core::{AttrValue, Attrs, Fragment, Slice};
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

fn embed_spec(seen: &Rc<Seen>) -> NodeViewSpec {
    spec_for("embed", seen)
}

/// Two same-`src` pictures side by side: "ab" 1..3, one at 3, two at 4, "ef" 5..7.
const ADJ: &str = r#"<p>ab<img src="a.png" data-badge="new" data-ref="one"><img src="a.png" data-badge="hot" data-ref="two">ef</p>"#;

/// Q1 (round 2): one transaction proves an attr-only change of picture "two" and
/// deletes picture "one" right before it. The proof names "two" (after), but the
/// splice pairs it with the char of "one" (prefix run): B's mount drawn for "one"
/// must not be handed "two".
#[test]
fn q1_delete_one_and_set_an_attr_of_its_neighbour_in_one_transaction() {
    let p = pair(ADJ);
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(4, "alt", AttrValue::from("renumbered"))
            .ok()?;
        tr.delete(3, 4).ok()?;
        Some(tr)
    }));
    assert!(moved(&p.seen_a).is_empty(), "local: {:?}", moved(&p.seen_a));
    let before = p.seen_b.renders.get();
    p.send();
    eprintln!(
        "Q renders_b {} -> {} mounts {:?}",
        before,
        p.seen_b.renders.get(),
        p.seen_b.mounts.borrow()
    );
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(
        moved(&p.seen_b).is_empty(),
        "peer: {:?} live {:?}",
        moved(&p.seen_b),
        live(&p.seen_b)
    );
    assert_eq!(live(&p.seen_b), vec!["two".to_string()], "peer live mounts");
}

/// Q2: the same shape for two embeds (part B's block path, `changed_atom_identity`).
#[test]
fn q2_delete_one_embed_and_set_an_attr_of_the_next_in_one_transaction() {
    let p = pair_with(
        r#"<div data-pm-type="embed" data-ref="one"></div><div data-pm-type="embed" data-ref="two"></div><p>x</p>"#,
        embed_spec,
    );
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(1, "data-n", AttrValue::from("1")).ok()?;
        tr.delete(0, 1).ok()?;
        Some(tr)
    }));
    assert!(moved(&p.seen_a).is_empty(), "local: {:?}", moved(&p.seen_a));
    let before = p.seen_b.renders.get();
    p.send();
    eprintln!(
        "Q renders_b {} -> {} mounts {:?}",
        before,
        p.seen_b.renders.get(),
        p.seen_b.mounts.borrow()
    );
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(
        moved(&p.seen_b).is_empty(),
        "peer: {:?} live {:?}",
        moved(&p.seen_b),
        live(&p.seen_b)
    );
    assert_eq!(live(&p.seen_b), vec!["two".to_string()], "peer live mounts");
}

/// Q3: Q1 with a concurrent attr change of picture "one" on B. "one" is deleted
/// by A, so B's change is lost — it must never show on "two" (#1503's invariant,
/// decision (f)).
#[test]
fn q3_a_concurrent_attr_change_of_the_deleted_neighbour_never_lands_on_the_kept_one() {
    let p = pair(ADJ);
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(4, "alt", AttrValue::from("renumbered"))
            .ok()?;
        tr.delete(3, 4).ok()?;
        Some(tr)
    }));
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(3, "data-note", AttrValue::from("for-one"))
            .ok()?;
        Some(tr)
    }));
    p.send();
    p.back();
    let (ha, hb) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!("Q3 a={ha}\nQ3 b={hb}");
    assert_eq!(ha, hb);
    assert!(
        !ha.contains("for-one"),
        "B's note for one landed on two: {ha}"
    );
}

/// Q4: Q2's embed shape with a concurrent attr change of embed "one" on B.
#[test]
fn q4_embed_a_concurrent_attr_change_of_the_deleted_one_never_lands_on_the_next() {
    let p = pair_with(
        r#"<div data-pm-type="embed" data-ref="one"></div><div data-pm-type="embed" data-ref="two"></div><p>x</p>"#,
        embed_spec,
    );
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(1, "data-n", AttrValue::from("1")).ok()?;
        tr.delete(0, 1).ok()?;
        Some(tr)
    }));
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(0, "data-note", AttrValue::from("for-one"))
            .ok()?;
        Some(tr)
    }));
    p.send();
    p.back();
    let (ha, hb) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!("Q4 a={ha}\nQ4 b={hb}");
    assert_eq!(ha, hb);
    assert!(
        !ha.contains("for-one"),
        "B's note for one landed on two: {ha}"
    );
}

fn q5_shape(html_in: &str, pos: usize) {
    let p = pair(html_in);
    let fresh = image(&p.a.handle, "a.png", "new", "one");
    let renders_b = p.seen_b.renders.get();
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(
            pos,
            pos + 1,
            Slice::new(Fragment::from_node(fresh.clone()), 0, 0),
        )
        .ok()?;
        Some(tr)
    }));
    let local = p.seen_a.renders.get();
    p.send();
    eprintln!(
        "Q5 {html_in}: a renders {local}, b renders {} -> {}",
        renders_b,
        p.seen_b.renders.get()
    );
    assert_eq!(html(&p.b.handle), html(&p.a.handle));
    assert!(
        p.seen_b.renders.get() > renders_b,
        "decision (f): an equal-valued fresh picture is a new picture on the peer too"
    );
}

/// Q5: the only content of the document is the picture; it is replaced by a fresh
/// node of EQUAL value (a copy of itself pasted over it). Nothing is shared, so the
/// projection matches by value.
#[test]
fn q5_an_equal_fresh_picture_over_the_only_one() {
    q5_shape(
        r#"<p><img src="a.png" data-badge="new" data-ref="one"></p>"#,
        1,
    );
}

#[test]
fn q5b_an_equal_fresh_picture_over_the_only_one_in_a_list() {
    q5_shape(
        r#"<ul><li><p><img src="a.png" data-badge="new" data-ref="one"></p></li></ul>"#,
        3,
    );
}

/// Q5t (the round-3 review of #1595, F5): the same inside a one-cell table, whose
/// reconcile matched the cell by value and kept the old char.
#[test]
fn q5t_an_equal_fresh_picture_over_the_only_one_in_a_table_cell() {
    q5_shape(
        r#"<table><tr><td><p><img src="a.png" data-badge="new" data-ref="one"></p></td></tr></table>"#,
        4,
    );
}

#[test]
fn q5c_an_equal_fresh_picture_with_another_paragraph() {
    q5_shape(
        r#"<p><img src="a.png" data-badge="new" data-ref="one"></p><p>x</p>"#,
        1,
    );
}

/// Q6: the 2 "structural doubles" of the identical-pictures differential, as an
/// editor sees them. A holds one picture `Rc` twice (an in-editor copy); A sets an
/// attribute of the first copy (a proven attr-only change) while B pastes a
/// same-`src` picture over that same picture. The user had one picture there and
/// pasted over it: two pictures are not what anybody did.
#[test]
fn q6_attr_change_of_a_shared_copy_and_a_concurrent_paste_over_make_one_picture() {
    let p = pair(r#"<p>ab<img src="a.png" data-badge="new" data-ref="one">cd</p>"#);
    assert!(p.a.handle.update(|st| {
        let node = st.doc.node_at(3)?;
        let mut tr = st.tr();
        tr.replace(6, 6, Slice::new(Fragment::from_node(node), 0, 0))
            .ok()?;
        Some(tr)
    }));
    p.send();
    let count = |h: &EditorHandle| html(h).matches("<img").count();
    assert_eq!(count(&p.b.handle), 2);
    assert!(p.a.handle.update(|st| {
        let mut tr = st.tr();
        tr.set_node_attr(3, "alt", AttrValue::from("edited")).ok()?;
        Some(tr)
    }));
    let fresh = image(&p.b.handle, "a.png", "new", "pasted");
    assert!(p.b.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(3, 4, Slice::new(Fragment::from_node(fresh.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    p.send();
    p.back();
    let (ha, hb) = (html(&p.a.handle), html(&p.b.handle));
    eprintln!("Q6 a={ha}");
    assert_eq!(ha, hb);
    assert_eq!(
        count(&p.a.handle),
        2,
        "one picture pasted over, one copy: {ha}"
    );
}

/// Q5d: the shape `shares_any_inline_node` exists for — one paragraph, text kept
/// around the picture, a fresh picture of EQUAL value put over it.
#[test]
fn q5d_an_equal_fresh_picture_in_a_one_paragraph_document_with_text() {
    q5_shape(
        r#"<p>ab<img src="a.png" data-badge="new" data-ref="one">cd</p>"#,
        3,
    );
}
