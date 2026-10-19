//! Node views (PR B): an app's component drawn in place of an `embed`, on the
//! renderer-agnostic view over `MockDomDocument`. What the platforms add (events,
//! focus, layout) is pinned in `rinch` (desktop) and `rinch-web` (Chrome).
//!
//! - the view mounts with the host, once, and survives every unrelated edit with
//!   its own state;
//! - an attribute change of its node — local, undone, redone, or a collaborator's
//!   — reaches its `attrs` memo without a remount;
//! - any other node put in its node's place, however alike, is remounted for
//!   (decision (d): a view is never handed another node);
//! - read-only refuses `set_attrs` and `delete`; the memos follow read-only and
//!   dark mode and the node selection;
//! - an Escape the view leaves returns the keyboard with the caret after the
//!   node; an arrow onto it offers the caret;
//! - unmounting the editor disposes every view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::reactive::{Effect, Signal, on_cleanup};
use rinch_core::show_dom;
use rinch_editor_core::{
    AttrValue, Attrs, Fragment, Node, NodeSelection, Pos, Schema, Selection, Slice,
};
use rinch_editor_view::{
    CaretFrom, EditorHandle, EditorKey, NODE_VIEW_ATTR, NodeViewCtx, NodeViewSpec, create_editor,
};

/// What the test can see of the views a spec drew.
#[derive(Default)]
struct Seen {
    renders: Cell<usize>,
    cleanups: Cell<usize>,
    /// Each mount's own state: a signal it made.
    state: RefCell<Vec<Signal<i32>>>,
    /// Each mount's context.
    ctx: RefCell<Vec<NodeViewCtx>>,
    /// The `data-sheet` each mount's `attrs` memo showed, every time it ran.
    sheets: RefCell<Vec<String>>,
    /// Each mount's root.
    roots: RefCell<Vec<NodeHandle>>,
    keys: RefCell<Vec<String>>,
    entered: RefCell<Vec<CaretFrom>>,
}

fn spec(seen: &Rc<Seen>) -> NodeViewSpec {
    let s = seen.clone();
    let k = seen.clone();
    let e = seen.clone();
    NodeViewSpec::new("embed", move |scope: &mut RenderScope, ctx: NodeViewCtx| {
        s.renders.set(s.renders.get() + 1);
        let root = scope.create_element("div");
        root.set_attribute("class", "sheet");
        let input = scope.create_element("input");
        root.append_child(&input);
        s.state.borrow_mut().push(Signal::new(0));
        s.ctx.borrow_mut().push(ctx.clone());
        s.roots.borrow_mut().push(root.clone());
        let attrs = ctx.attrs;
        let sheets = s.clone();
        scope.create_effect(move || {
            let sheet = attrs.get().get_str("data-sheet").unwrap_or("").to_string();
            sheets.sheets.borrow_mut().push(sheet);
        });
        let c = s.clone();
        on_cleanup(move || c.cleanups.set(c.cleanups.get() + 1));
        root
    })
    .on_key(move |key: &EditorKey<'_>| {
        k.keys.borrow_mut().push(key.key.to_string());
        key.key == "Enter"
    })
    .on_caret_enter(move |from| {
        e.entered.borrow_mut().push(from);
        false
    })
}

struct Page {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
    container: NodeHandle,
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
        doc,
        _scope: scope,
        handle,
        container,
    }
}

fn children(p: &Page, id: NodeId) -> Vec<NodeId> {
    p.doc.borrow().get_children(id)
}

/// The host elements of the document's embeds, in order.
fn hosts(p: &Page) -> Vec<NodeId> {
    let d = p.doc.borrow();
    d.get_children(p.container.node_id())
        .into_iter()
        .filter(|&c| d.get_attribute(c, "data-pm-type").as_deref() == Some("embed"))
        .collect()
}

const DOC: &str = r#"<p>ab</p><div data-pm-type="embed" data-sheet="s1"></div><p>cd</p>"#;

/// `ab` 0..4, the embed 4..5, `cd` 5..9.
const EMBED: usize = 4;

fn register(p: &Page) -> Rc<Seen> {
    let seen = Rc::new(Seen::default());
    p.handle.register_node_view(spec(&seen)).keep();
    seen
}

#[test]
fn a_view_mounts_once_and_survives_typing_elsewhere_with_its_state() {
    let p = page(DOC);
    let seen = register(&p);
    assert_eq!(
        seen.renders.get(),
        1,
        "rendered for the embed already there"
    );
    let host = hosts(&p)[0];
    let root = seen.roots.borrow()[0].node_id();
    assert_eq!(
        children(&p, host),
        vec![root],
        "the component is in the host"
    );
    assert_eq!(
        p.doc
            .borrow()
            .get_attribute(host, NODE_VIEW_ATTR)
            .as_deref(),
        Some("")
    );
    seen.state.borrow()[0].set(5);

    // Typing before it, after it, Enter above it, a paragraph deleted above it.
    p.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(p.handle.insert_text("x"));
    p.handle.set_selection(Selection::cursor(Pos(8)));
    assert!(p.handle.insert_text("y"));
    p.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(p.handle.command("splitBlock"));
    p.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(p.handle.command("deleteCharForward"));

    assert_eq!(seen.renders.get(), 1, "never remounted");
    assert_eq!(seen.cleanups.get(), 0);
    assert_eq!(hosts(&p), vec![host], "the same host");
    assert_eq!(children(&p, host), vec![root], "the same component");
    assert_eq!(seen.state.borrow()[0].get(), 5, "its own state is kept");
}

/// One transaction that changes the blocks on both sides of the view and adds a
/// block before it: the view's node only moved within the changed stretch, and
/// keeps its mount (the stretch is matched by node identity, not position).
#[test]
fn a_view_moved_within_one_changed_stretch_keeps_its_mount() {
    let p = page(DOC);
    let seen = register(&p);
    assert!(p.handle.update(|st| {
        let mut tr = st.tr();
        // Split "ab" (adds a block before the embed), then type in "cd".
        tr.split(2, 1, None).ok()?;
        let x = st.schema().text("x").ok()?;
        let end = tr.doc().content_size() - 1;
        tr.replace(end, end, Slice::new(Fragment::from_node(x), 0, 0))
            .ok()?;
        Some(tr)
    }));
    assert_eq!(p.handle.doc().child_count(), 4);
    assert_eq!(seen.renders.get(), 1, "not remounted");
    assert_eq!(seen.ctx.borrow()[0].pos(), Some(Pos(EMBED + 2)));
}

#[test]
fn set_attrs_reaches_the_memo_without_a_remount_and_undo_redo_too() {
    let p = page(DOC);
    let seen = register(&p);
    let ctx = seen.ctx.borrow()[0].clone();
    assert_eq!(ctx.pos(), Some(Pos(EMBED)));
    assert!(ctx.set_attrs(|a| *a = a.with("data-sheet", AttrValue::from("s2"))));
    assert_eq!(*seen.sheets.borrow(), ["s1", "s2"]);
    let host = hosts(&p)[0];
    assert_eq!(
        p.doc.borrow().get_attribute(host, "data-sheet").as_deref(),
        Some("s2"),
        "the host follows"
    );
    assert!(p.handle.command("undo"));
    assert!(p.handle.command("redo"));
    assert_eq!(*seen.sheets.borrow(), ["s1", "s2", "s1", "s2"]);
    assert_eq!(seen.renders.get(), 1, "one mount throughout");
    assert!(!ctx.set_attrs(|_| {}), "no change, no transaction");
}

/// Decision (d): a node put where the view's node stood is another node, even
/// with the very same attributes — the view is remounted, never re-pointed.
#[test]
fn a_node_that_takes_the_views_place_is_remounted_for_not_handed_over() {
    let p = page(DOC);
    let seen = register(&p);
    seen.state.borrow()[0].set(7);
    let s = Schema::starter_kit();
    let twin = s
        .create_node(
            "embed",
            Attrs::from_iter([("data-sheet", AttrValue::from("s1"))]),
            Fragment::empty(),
        )
        .unwrap();
    // Replace the embed with an identical one, in one step.
    assert!(p.handle.update(|st| {
        let twin = twin.rebind(st.schema()).ok()?;
        let mut tr = st.tr();
        tr.replace(
            EMBED,
            EMBED + 1,
            Slice::new(Fragment::from_node(twin), 0, 0),
        )
        .ok()?;
        Some(tr)
    }));
    assert_eq!(seen.renders.get(), 2, "a new mount for the new node");
    assert_eq!(seen.cleanups.get(), 1, "the old one disposed");
    assert_eq!(seen.state.borrow()[1].get(), 0, "fresh state, not the old");

    // An attribute change made by replacing the node (no SetNodeAttrStep) is a
    // replace too.
    let other = s
        .create_node(
            "embed",
            Attrs::from_iter([("data-sheet", AttrValue::from("s9"))]),
            Fragment::empty(),
        )
        .unwrap();
    assert!(p.handle.update(|st| {
        let other = other.rebind(st.schema()).ok()?;
        let mut tr = st.tr();
        tr.replace(
            EMBED,
            EMBED + 1,
            Slice::new(Fragment::from_node(other), 0, 0),
        )
        .ok()?;
        Some(tr)
    }));
    assert_eq!(seen.renders.get(), 3);
    assert_eq!(seen.cleanups.get(), 2);
}

/// Two identical embeds side by side: deleting the first leaves the second's
/// view where it was — it is not shifted onto the first's mount.
#[test]
fn deleting_the_first_of_two_identical_embeds_keeps_the_seconds_mount() {
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let p = page(&format!("<p>ab</p>{e}{e}<p>cd</p>"));
    let seen = register(&p);
    assert_eq!(seen.renders.get(), 2);
    seen.state.borrow()[0].set(1);
    seen.state.borrow()[1].set(2);
    let second = seen.ctx.borrow()[1].clone();
    assert!(seen.ctx.borrow()[0].delete());
    assert_eq!(seen.cleanups.get(), 1);
    assert_eq!(seen.renders.get(), 2, "the second is not remounted");
    assert_eq!(second.pos(), Some(Pos(EMBED)));
    assert_eq!(seen.state.borrow()[1].get(), 2);
    assert!(seen.ctx.borrow()[0].pos().is_none(), "the first is gone");
}

#[test]
fn read_only_refuses_set_attrs_and_delete_and_the_memos_follow() {
    let p = page(DOC);
    let seen = register(&p);
    let ctx = seen.ctx.borrow()[0].clone();
    let ro = Rc::new(RefCell::new(Vec::new()));
    let dark = Rc::new(RefCell::new(Vec::new()));
    let selected = Rc::new(RefCell::new(Vec::new()));
    {
        let (ro, dark, selected, c) = (ro.clone(), dark.clone(), selected.clone(), ctx.clone());
        let _e1 = Effect::new(move || ro.borrow_mut().push(c.read_only.get()));
        let c = ctx.clone();
        let _e2 = Effect::new(move || dark.borrow_mut().push(c.dark_mode.get()));
        let c = ctx.clone();
        let _e3 = Effect::new(move || selected.borrow_mut().push(c.selected.get()));
        std::mem::forget((_e1, _e2, _e3));
    }
    p.handle.set_read_only(true);
    assert!(!ctx.set_attrs(|a| *a = a.with("data-sheet", AttrValue::from("x"))));
    assert!(!ctx.delete());
    assert_eq!(ctx.pos(), Some(Pos(EMBED)), "still there");
    p.handle.set_dark_mode(true);
    p.handle.set_selection(Selection::Node(NodeSelection {
        anchor: Pos(EMBED),
        head: Pos(EMBED + 1),
    }));
    p.handle.set_selection(Selection::cursor(Pos(1)));
    assert_eq!(*ro.borrow(), [false, true]);
    assert_eq!(*dark.borrow(), [false, true]);
    assert_eq!(*selected.borrow(), [false, true, false]);
    p.handle.set_read_only(false);
    assert!(ctx.delete());
    assert_eq!(seen.cleanups.get(), 1);
}

#[test]
fn an_unclaimed_escape_returns_the_caret_after_the_node() {
    let p = page(DOC);
    let seen = register(&p);
    let host = hosts(&p)[0].0;
    let key = |k: &'static str| EditorKey {
        key: k,
        primary: false,
        ctrl: false,
        meta: false,
        shift: false,
        alt: false,
        repeat: false,
    };
    p.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(p.handle.offer_node_view_key(host, &key("Enter")), "claimed");
    assert!(!p.handle.offer_node_view_key(host, &key("a")), "left alone");
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(1)));
    assert!(p.handle.offer_node_view_key(host, &key("Escape")));
    assert_eq!(*seen.keys.borrow(), ["Enter", "a", "Escape"]);
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(EMBED + 2)),
        "the caret starts the paragraph after it"
    );
    // A host that is not a view's is not offered anything.
    assert!(
        !p.handle
            .offer_node_view_key(p.container.node_id().0, &key("Escape"))
    );
}

#[test]
fn an_arrow_onto_the_node_selects_it_and_offers_the_caret() {
    let p = page(DOC);
    let seen = register(&p);
    p.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(
        p.handle
            .move_cursor(rinch_editor_core::CursorMotion::CharRight, false)
    );
    let node = Selection::Node(NodeSelection {
        anchor: Pos(EMBED),
        head: Pos(EMBED + 1),
    });
    assert_eq!(p.handle.selection(), node);
    p.handle.set_selection(Selection::cursor(Pos(6)));
    assert!(
        p.handle
            .move_cursor(rinch_editor_core::CursorMotion::CharLeft, false)
    );
    assert_eq!(*seen.entered.borrow(), [CaretFrom::Left, CaretFrom::Right]);
    // Up and Down do not step over a node with a view.
    p.handle.set_selection(Selection::cursor(Pos(2)));
    assert_eq!(p.handle.vertical_node_view(true), Some(node.clone()));
    p.handle.set_selection(Selection::cursor(Pos(7)));
    assert_eq!(p.handle.vertical_node_view(false), Some(node.clone()));
    p.handle.set_selection(Selection::cursor(Pos(7)));
    assert_eq!(p.handle.vertical_node_view(true), None);
}

#[test]
fn backspace_at_the_paragraph_after_selects_then_removes_it() {
    let p = page(DOC);
    let seen = register(&p);
    p.handle.set_selection(Selection::cursor(Pos(6)));
    assert!(p.handle.command("deleteCharBackward"));
    assert_eq!(
        p.handle.selection(),
        Selection::Node(NodeSelection {
            anchor: Pos(EMBED),
            head: Pos(EMBED + 1),
        })
    );
    assert!(seen.ctx.borrow()[0].selected.get());
    assert!(p.handle.command("deleteCharBackward"));
    assert!(hosts(&p).is_empty());
    assert_eq!(seen.cleanups.get(), 1);
}

#[test]
fn insert_embed_mounts_a_view_and_is_refused_read_only() {
    let p = page("<p>ab</p>");
    let seen = register(&p);
    p.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(
        p.handle
            .insert_embed(Attrs::from_iter([("data-sheet", AttrValue::from("new"))]))
    );
    assert_eq!(seen.renders.get(), 1);
    assert_eq!(*seen.sheets.borrow(), ["new"]);
    p.handle.set_read_only(true);
    assert!(!p.handle.insert_embed(Attrs::default()));
}

#[test]
fn matches_picks_the_first_spec_and_dropping_a_registration_unmounts() {
    let p = page(DOC);
    let a = Rc::new(Seen::default());
    let b = Rc::new(Seen::default());
    let reg_a = p
        .handle
        .register_node_view(spec(&a).matches(|attrs| attrs.get_str("data-sheet") == Some("zz")));
    let reg_b = p.handle.register_node_view(spec(&b));
    assert_eq!((a.renders.get(), b.renders.get()), (0, 1), "b matches");
    // An attr change that makes `a` match moves the node to `a`.
    assert!(b.ctx.borrow()[0].set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("zz"))));
    assert_eq!((a.renders.get(), b.renders.get()), (1, 1));
    assert_eq!(b.cleanups.get(), 1);
    drop(reg_a);
    assert_eq!(a.cleanups.get(), 1, "released: unmounted");
    assert_eq!(b.renders.get(), 2, "the next spec that matches takes over");
    drop(reg_b);
    assert_eq!(b.cleanups.get(), 2);
    let host = hosts(&p)[0];
    assert!(children(&p, host).is_empty(), "an empty host again");
    assert_eq!(
        p.doc.borrow().get_attribute(host, NODE_VIEW_ATTR),
        None,
        "no longer marked"
    );
}

/// Unmounting the editor disposes every view, and leaves nothing behind.
#[test]
fn unmounting_the_editor_disposes_every_view() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body_id = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut sc = RenderScope::new(dyn_doc.clone(), body_id);
    let body = NodeHandle::new(body_id, Rc::downgrade(&dyn_doc));
    let visible = Signal::new(false);
    let seen = Rc::new(Seen::default());
    let handle = create_editor();
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    assert!(handle.load_html(&format!("<p>ab</p>{e}<p>x</p>{e}")));
    handle.register_node_view(spec(&seen)).keep();
    let h = handle.clone();
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| h.mount(s),
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    assert_eq!(seen.renders.get(), 2);
    visible.set(false);
    assert_eq!(seen.cleanups.get(), 2, "both views disposed");
    let base = doc.borrow().__node_count() as isize;
    for _ in 0..20 {
        visible.set(true);
        visible.set(false);
    }
    assert_eq!(seen.renders.get(), 42);
    assert_eq!(seen.cleanups.get(), 42);
    assert_eq!(
        doc.borrow().__node_count() as isize - base,
        0,
        "no node leaks"
    );
}

/// A view keyed to `Node` identity: the nodes the test built are bound to the
/// editor's schema before use.
#[allow(dead_code)]
fn _bind(n: &Node, h: &EditorHandle) -> Node {
    n.rebind(h.state().schema()).unwrap()
}

#[cfg(feature = "collaboration")]
mod collab {
    use super::*;

    /// A collaborator's attribute change reaches the view's memo; no remount.
    #[test]
    fn a_collaborators_attr_change_updates_the_view_without_a_remount() {
        let a = page(DOC);
        let b_page = page("<p></p>");
        let to_b: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let sink = to_b.clone();
        let snapshot = a
            .handle
            .start_collaboration_host(move |d| sink.borrow_mut().push(d))
            .unwrap();
        b_page
            .handle
            .start_collaboration_guest(&snapshot, |_| {})
            .unwrap();
        let seen_a = register(&a);
        let seen_b = register(&b_page);
        assert_eq!(seen_b.renders.get(), 1);
        seen_b.state.borrow()[0].set(3);

        assert!(
            seen_a.ctx.borrow()[0].set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2")))
        );
        for d in to_b.borrow_mut().drain(..) {
            assert!(b_page.handle.collab_receive(&d));
        }
        assert_eq!(*seen_b.sheets.borrow(), ["s1", "s2"]);
        assert_eq!(
            seen_b.renders.get(),
            1,
            "the collaborator's view stays mounted"
        );
        assert_eq!(seen_b.state.borrow()[0].get(), 3);

        // A peer moving the embed (delete + insert a copy) remounts it.
        let first = a.handle.doc().child(1).clone();
        assert!(a.handle.update(|st| {
            let mut tr = st.tr();
            tr.delete(EMBED, EMBED + 1).ok()?;
            let end = tr.doc().content_size();
            tr.replace(
                end,
                end,
                Slice::new(Fragment::from_node(first.clone()), 0, 0),
            )
            .ok()?;
            Some(tr)
        }));
        for d in to_b.borrow_mut().drain(..) {
            assert!(b_page.handle.collab_receive(&d));
        }
        assert_eq!(seen_b.renders.get(), 2, "a moved node is another mount");
        assert_eq!(seen_b.cleanups.get(), 1);
    }
}
