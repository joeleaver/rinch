//! Adversarial review of PR #1575 (node views part B). Each test states what the
//! design requires; a red one is a finding.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::reactive::{Signal, on_cleanup};
use rinch_editor_core::{
    AttrValue, Attrs, Fragment, NodeSelection, Pos, Selection, SetNodeAttrStep, Slice,
};
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};

#[derive(Default)]
struct Seen {
    renders: Cell<usize>,
    cleanups: Cell<usize>,
    /// The `data-sheet` each mount was rendered for.
    mounted_for: RefCell<Vec<String>>,
    state: RefCell<Vec<Signal<i32>>>,
    ctx: RefCell<Vec<NodeViewCtx>>,
}

fn spec(seen: &Rc<Seen>) -> NodeViewSpec {
    let s = seen.clone();
    NodeViewSpec::new("embed", move |scope: &mut RenderScope, ctx: NodeViewCtx| {
        s.renders.set(s.renders.get() + 1);
        s.mounted_for.borrow_mut().push(
            ctx.attrs
                .get()
                .get_str("data-sheet")
                .unwrap_or("")
                .to_string(),
        );
        s.state.borrow_mut().push(Signal::new(0));
        s.ctx.borrow_mut().push(ctx.clone());
        let c = s.clone();
        on_cleanup(move || c.cleanups.set(c.cleanups.get() + 1));
        scope.create_element("div")
    })
}

struct Page {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
    #[allow(dead_code)]
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

fn register(p: &Page) -> Rc<Seen> {
    let seen = Rc::new(Seen::default());
    p.handle.register_node_view(spec(&seen)).keep();
    seen
}

fn embed_attrs(sheet: &str) -> Attrs {
    Attrs::from_iter([("data-sheet", AttrValue::from(sheet))])
}

const DOC: &str = r#"<p>ab</p><div data-pm-type="embed" data-sheet="s1"></div><p>cd</p><p>ef</p>"#;
/// `ab` 0..4, the embed 4..5, `cd` 5..9, `ef` 9..13.
const EMBED: usize = 4;

/// R1 — decision (d). One transaction replaces the embed with a *different*
/// embed and then sets an attribute on the new one. The `SetNodeAttrStep` puts
/// the position in the kept set, so the old view is patched in place and handed
/// the new node.
#[test]
fn r1_replace_then_set_attr_must_not_hand_the_view_to_the_new_node() {
    let p = page(DOC);
    let seen = register(&p);
    seen.state.borrow()[0].set(42);
    assert!(p.handle.update(|st| {
        let other = st
            .schema()
            .create_node("embed", embed_attrs("OTHER"), Fragment::empty())
            .ok()?;
        let mut tr = st.tr();
        tr.replace(
            EMBED,
            EMBED + 1,
            Slice::new(Fragment::from_node(other), 0, 0),
        )
        .ok()?;
        tr.step(Box::new(SetNodeAttrStep::new(
            EMBED,
            "data-rev",
            AttrValue::from("1"),
        )))
        .ok()?;
        Some(tr)
    }));
    assert_eq!(
        seen.renders.get(),
        2,
        "a different node took the view's place: the view must be remounted \
         (it was handed node OTHER with the old mount's state {})",
        seen.state.borrow()[0].get()
    );
}

/// R1b — the reverse order: set an attribute, then replace the node with
/// another embed. The PR remounts (its `deleted_after` check); pins that check.
#[test]
fn r1b_set_attr_then_replace_remounts() {
    let p = page(DOC);
    let seen = register(&p);
    assert!(p.handle.update(|st| {
        let other = st
            .schema()
            .create_node("embed", embed_attrs("OTHER"), Fragment::empty())
            .ok()?;
        let mut tr = st.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            EMBED,
            "data-rev",
            AttrValue::from("1"),
        )))
        .ok()?;
        tr.replace(
            EMBED,
            EMBED + 1,
            Slice::new(Fragment::from_node(other), 0, 0),
        )
        .ok()?;
        Some(tr)
    }));
    assert_eq!(seen.renders.get(), 2, "another node: remounted");
}

/// R14 — a refused `set_attrs` (read-only) leaves its attribute proof behind,
/// and the next commit that sets none — here a load of another document — uses
/// it: the loaded embed, a different node at the same position, is handed the
/// old view.
#[test]
fn r14_a_refused_set_attrs_leaves_no_stale_proof_for_a_later_load() {
    let p = page(DOC);
    let seen = register(&p);
    seen.state.borrow()[0].set(5);
    p.handle.set_read_only(true);
    assert!(!seen.ctx.borrow()[0].set_attrs(|a| *a = a.with("data-x", AttrValue::from("1"))));
    p.handle.set_read_only(false);
    assert!(p.handle.load_html(
        r#"<p>ab</p><div data-pm-type="embed" data-sheet="LOADED"></div><p>cd</p><p>ef</p>"#
    ));
    assert_eq!(
        seen.renders.get(),
        2,
        "a loaded document's embed is another node (the old view now shows {:?})",
        seen.ctx.borrow()[0].attrs.get().get_str("data-sheet")
    );
}

/// R14's control: the same load with no refused edit before it remounts.
#[test]
fn r14_control_a_load_remounts() {
    let p = page(DOC);
    let seen = register(&p);
    assert!(p.handle.load_html(
        r#"<p>ab</p><div data-pm-type="embed" data-sheet="LOADED"></div><p>cd</p><p>ef</p>"#
    ));
    assert_eq!(seen.renders.get(), 2);
}

/// R16 — an embed nested in a blockquote and in a list item survives typing and
/// Enter beside it, and a set_attrs on it.
#[test]
fn r16_nested_embeds_survive_edits_beside_them() {
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let p = page(&format!(
        "<blockquote><p>ab</p>{e}<p>cd</p></blockquote><ul><li><p>x</p>{e}</li></ul><p>z</p>"
    ));
    let seen = register(&p);
    assert_eq!(seen.renders.get(), 2);
    let pos0 = seen.ctx.borrow()[0].pos().unwrap().0;
    p.handle.set_selection(Selection::cursor(Pos(pos0 - 1)));
    assert!(p.handle.insert_text("k"));
    assert!(p.handle.command("splitBlock"));
    let pos1 = seen.ctx.borrow()[1].pos().unwrap().0;
    p.handle.set_selection(Selection::cursor(Pos(pos1 - 1)));
    assert!(p.handle.insert_text("k"));
    assert!(seen.ctx.borrow()[1].set_attrs(|a| *a = a.with("data-sheet", AttrValue::from("s2"))));
    assert_eq!(seen.renders.get(), 2, "no remount");
}

/// R15 — Down from the last paragraph of a list item onto a viewed embed right
/// after the list: the guide says Up/Down never step over a view.
#[test]
fn r15_down_from_a_list_item_onto_a_view_after_the_list() {
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let p = page(&format!("<ul><li><p>ab</p></li></ul>{e}<p>cd</p>"));
    let _seen = register(&p);
    p.handle.set_selection(Selection::cursor(Pos(4)));
    assert!(
        p.handle.vertical_node_view(true).is_some(),
        "Down from the list's last line selects the view below it"
    );
}

/// R2 — requirement: an attribute change of the same node never remounts. An
/// app's transaction that adds a paragraph above the embed and sets one of its
/// attributes.
#[test]
fn r2_insert_above_and_set_attrs_in_one_transaction_keeps_the_mount() {
    let p = page(DOC);
    let seen = register(&p);
    assert!(p.handle.update(|st| {
        let para = st
            .schema()
            .create_node(
                "paragraph",
                Attrs::default(),
                Fragment::from_node(st.schema().text("new").ok()?),
            )
            .ok()?;
        let mut tr = st.tr();
        tr.replace(EMBED, EMBED, Slice::new(Fragment::from_node(para), 0, 0))
            .ok()?;
        let at = tr.mapping().map(EMBED, 1);
        tr.step(Box::new(SetNodeAttrStep::new(
            at,
            "data-rev",
            AttrValue::from("1"),
        )))
        .ok()?;
        Some(tr)
    }));
    assert_eq!(
        seen.renders.get(),
        1,
        "same node, other attributes: no remount"
    );
}

/// Same, with the text edit in the paragraph above (no index shift).
#[test]
fn r2b_type_above_and_set_attrs_in_one_transaction_keeps_the_mount() {
    let p = page(DOC);
    let seen = register(&p);
    assert!(p.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(
            2,
            2,
            Slice::new(Fragment::from_node(st.schema().text("x").ok()?), 0, 0),
        )
        .ok()?;
        tr.step(Box::new(SetNodeAttrStep::new(
            EMBED + 1,
            "data-rev",
            AttrValue::from("1"),
        )))
        .ok()?;
        Some(tr)
    }));
    assert_eq!(seen.renders.get(), 1);
}

/// R7 — `render` re-enters the editor: reads and dispatches.
#[test]
fn r7_a_render_that_calls_the_handle_does_not_hit_a_borrow_error() {
    let p = page(DOC);
    let calls = Rc::new(Cell::new(0));
    let c = calls.clone();
    p.handle
        .register_node_view(NodeViewSpec::new(
            "embed",
            move |scope: &mut RenderScope, ctx: NodeViewCtx| {
                let ed = ctx.editor().unwrap();
                let _ = ed.doc();
                let _ = ed.selection();
                let _ = ctx.pos();
                ed.set_selection(Selection::cursor(Pos(1)));
                let _ = ed.insert_text("z");
                let _ = ed.command("toggleBold");
                c.set(c.get() + 1);
                scope.create_element("div")
            },
        ))
        .keep();
    assert!(calls.get() >= 1);
    // Typing elsewhere afterwards still works.
    assert!(p.handle.insert_text("q"));
}

/// A render that sets its own node's attributes (a default) mounts once.
#[test]
fn r7b_a_render_that_sets_its_own_attrs_mounts_once() {
    let p = page(DOC);
    let renders = Rc::new(Cell::new(0));
    let cleanups = Rc::new(Cell::new(0));
    let (r, cl) = (renders.clone(), cleanups.clone());
    p.handle
        .register_node_view(NodeViewSpec::new(
            "embed",
            move |scope: &mut RenderScope, ctx: NodeViewCtx| {
                r.set(r.get() + 1);
                ctx.set_attrs(|a| *a = a.with("data-init", AttrValue::from("1")));
                let cl = cl.clone();
                on_cleanup(move || cl.set(cl.get() + 1));
                scope.create_element("div")
            },
        ))
        .keep();
    assert_eq!(
        (renders.get(), cleanups.get()),
        (1, 0),
        "renders/cleanups for one node"
    );
}

/// R8 — typing right beside the embed: Enter, Backspace, splits and joins.
#[test]
fn r8_edits_right_beside_the_embed_never_remount() {
    let p = page(DOC);
    let seen = register(&p);
    seen.state.borrow()[0].set(9);
    let h = &p.handle;
    // Enter at the end of the paragraph before.
    h.set_selection(Selection::cursor(Pos(3)));
    assert!(h.command("splitBlock"));
    // Backspace that empty paragraph away again (joins backward).
    assert!(h.command("deleteCharBackward"));
    let pos = seen.ctx.borrow()[0].pos().unwrap().0;
    // Enter at the start of the paragraph after.
    h.set_selection(Selection::cursor(Pos(pos + 2)));
    assert!(h.command("splitBlock"));
    let pos = seen.ctx.borrow()[0].pos().unwrap().0;
    // Type in the (now empty) paragraph right after, then backspace in it.
    h.set_selection(Selection::cursor(Pos(pos + 2)));
    assert!(h.insert_text("k"));
    assert!(h.command("deleteCharBackward"));
    // Join the paragraph after with the one after it (Delete at its end).
    let pos = seen.ctx.borrow()[0].pos().unwrap().0;
    h.set_selection(Selection::cursor(Pos(pos + 2)));
    assert!(h.command("deleteCharForward"));
    // Split the paragraph before in its middle, then join it.
    h.set_selection(Selection::cursor(Pos(2)));
    assert!(h.command("splitBlock"));
    assert!(h.command("deleteCharBackward"));
    assert_eq!(seen.renders.get(), 1, "never remounted");
    assert_eq!(seen.cleanups.get(), 0);
    assert_eq!(seen.state.borrow()[0].get(), 9);
}

/// R9 — delete and undo the embed many times: nothing leaks.
#[test]
fn r9_delete_and_undo_repeatedly_leaks_no_node_and_no_scope() {
    let p = page(DOC);
    let seen = register(&p);
    let base = p.doc.borrow().__node_count() as isize;
    for _ in 0..20 {
        assert!(seen.ctx.borrow().last().unwrap().delete());
        assert!(p.handle.command("undo"));
    }
    assert_eq!(seen.renders.get(), 21);
    assert_eq!(seen.cleanups.get(), 20);
    assert_eq!(
        p.doc.borrow().__node_count() as isize - base,
        0,
        "node leak"
    );
}

/// R13 — Escape / `return_focus` with nothing after the node: it is selected.
#[test]
fn r13_return_focus_on_the_last_node_selects_it() {
    let p = page(r#"<p>ab</p><div data-pm-type="embed" data-sheet="s1"></div>"#);
    let seen = register(&p);
    p.handle.set_selection(Selection::cursor(Pos(1)));
    seen.ctx.borrow()[0].return_focus();
    assert_eq!(
        p.handle.selection(),
        Selection::Node(NodeSelection {
            anchor: Pos(EMBED),
            head: Pos(EMBED + 1)
        })
    );
}

/// Two adjacent embeds; `return_focus` of the first must not land on the second
/// or before the first.
#[test]
fn r13b_return_focus_between_two_embeds() {
    let e1 = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let e2 = r#"<div data-pm-type="embed" data-sheet="s2"></div>"#;
    let p = page(&format!("<p>ab</p>{e1}{e2}<p>cd</p>"));
    let seen = register(&p);
    seen.ctx.borrow()[0].return_focus();
    let sel = p.handle.selection();
    assert!(
        sel.head().0 > EMBED,
        "caret is after the first node, got {sel:?}"
    );
}

#[allow(dead_code)]
fn ids(p: &Page) -> Vec<NodeId> {
    p.doc.borrow().get_children(p.container.node_id())
}

#[cfg(feature = "collaboration")]
mod collab {
    use super::*;
    use rinch_editor_collab::CollabSession;
    use rinch_editor_core::{EditorState, Node, Schema};

    struct Pair {
        a: Page,
        b: Page,
        to_b: Rc<RefCell<Vec<Vec<u8>>>>,
        to_a: Rc<RefCell<Vec<Vec<u8>>>>,
    }

    fn pair(html: &str) -> Pair {
        let a = page(html);
        let b = page("<p></p>");
        let to_b: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let to_a: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let sb = to_b.clone();
        let snapshot = a
            .handle
            .start_collaboration_host(move |d| sb.borrow_mut().push(d))
            .unwrap();
        let sa = to_a.clone();
        b.handle
            .start_collaboration_guest(&snapshot, move |d| sa.borrow_mut().push(d))
            .unwrap();
        Pair { a, b, to_b, to_a }
    }

    fn flush(from: &Rc<RefCell<Vec<Vec<u8>>>>, to: &Page) {
        let ds: Vec<_> = from.borrow_mut().drain(..).collect();
        for d in ds {
            let ok = to.handle.collab_receive(&d);
            assert!(ok, "receive failed: {:?}", to.handle.collab_take_error());
        }
    }

    /// R3 — a peer types in a paragraph and changes the embed's attributes; the
    /// receiver catches up in one go (a reconnect: `collab_sync_diff`).
    #[test]
    fn r3_a_batched_text_edit_plus_attr_change_keeps_the_mount() {
        let pr = pair(DOC);
        let seen_b = register(&pr.b);
        let seen_a = register(&pr.a);
        assert_eq!(seen_b.renders.get(), 1);
        let sv = pr.b.handle.collab_state_vector().unwrap();
        pr.a.handle.set_selection(Selection::cursor(Pos(2)));
        assert!(pr.a.handle.insert_text("x"));
        assert!(
            seen_a.ctx.borrow()[0].set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2")))
        );
        pr.to_b.borrow_mut().clear();
        let diff = pr.a.handle.collab_sync_diff(&sv).unwrap();
        assert!(pr.b.handle.collab_receive(&diff));
        assert_eq!(
            pr.b.handle.doc().child(1).attrs().get_str("data-sheet"),
            Some("s2")
        );
        assert_eq!(
            seen_b.renders.get(),
            1,
            "a collaborator's attribute change on the same node never remounts"
        );
    }

    /// R4 — an embed with no attributes yet, given its first by a peer.
    #[test]
    fn r4_a_peer_giving_an_attrless_embed_its_first_attr_keeps_the_mount() {
        let pr = pair(r#"<p>ab</p><div data-pm-type="embed"></div><p>cd</p>"#);
        let seen_b = register(&pr.b);
        let seen_a = register(&pr.a);
        assert_eq!(seen_b.renders.get(), 1);
        assert!(
            seen_a.ctx.borrow()[0].set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2")))
        );
        flush(&pr.to_b, &pr.b);
        assert_eq!(
            pr.b.handle.doc().child(1).attrs().get_str("data-sheet"),
            Some("s2")
        );
        assert_eq!(seen_b.renders.get(), 1, "no remount");
    }

    /// R5 — the same document also holds a table.
    #[test]
    fn r5_a_peers_attr_change_keeps_the_mount_in_a_document_with_a_table() {
        let pr = pair(&format!(
            "{DOC}<table><tr><td>a</td><td>b</td></tr></table>"
        ));
        let seen_b = register(&pr.b);
        let seen_a = register(&pr.a);
        assert_eq!(seen_b.renders.get(), 1);
        assert!(
            seen_a.ctx.borrow()[0].set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2")))
        );
        flush(&pr.to_b, &pr.b);
        assert_eq!(seen_b.renders.get(), 1, "no remount");
    }

    /// R10 — two identical embeds; peer A swaps them (moves the first after
    /// the second) while peer B changes the first's attributes. Decision (d): no
    /// view on either side ends up showing the other node's data under the old
    /// mount.
    #[test]
    fn r10_swap_identical_embeds_while_a_peer_edits_one() {
        let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
        let pr = pair(&format!("<p>ab</p>{e}{e}<p>cd</p>"));
        let seen_a = register(&pr.a);
        let seen_b = register(&pr.b);
        seen_b.state.borrow()[0].set(1);
        seen_b.state.borrow()[1].set(2);
        // A: move the first embed after the second (one transaction).
        let first = pr.a.handle.doc().child(1).clone();
        assert!(pr.a.handle.update(|st| {
            let mut tr = st.tr();
            tr.delete(EMBED, EMBED + 1).ok()?;
            tr.replace(
                EMBED + 1,
                EMBED + 1,
                Slice::new(Fragment::from_node(first.clone()), 0, 0),
            )
            .ok()?;
            Some(tr)
        }));
        assert_eq!(seen_a.renders.get(), 2, "A keeps both (same Rc moved)");
        // B, concurrently: tag the first.
        assert!(
            seen_b.ctx.borrow()[0].set_attrs(|x| *x = x.with("data-tag", AttrValue::from("B")))
        );
        let rb: Vec<bool> = pr
            .to_b
            .borrow_mut()
            .drain(..)
            .collect::<Vec<_>>()
            .iter()
            .map(|d| pr.b.handle.collab_receive(d))
            .collect();
        let ra: Vec<bool> = pr
            .to_a
            .borrow_mut()
            .drain(..)
            .collect::<Vec<_>>()
            .iter()
            .map(|d| pr.a.handle.collab_receive(d))
            .collect();
        eprintln!(
            "receive b {rb:?} a {ra:?}; B err {:?}",
            pr.b.handle.collab_take_error()
        );
        let attrs = |p: &Page| {
            (0..p.handle.doc().child_count())
                .map(|i| p.handle.doc().child(i).attrs().clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(attrs(&pr.a), attrs(&pr.b), "converged");
        // On B: every surviving mount must be for a node with the identity it was
        // mounted for. A mount kept is a mount whose node kept its CRDT block.
        let live_b: Vec<_> = seen_b
            .ctx
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.pos().map(|p| (i, p)))
            .collect();
        eprintln!(
            "B renders {} cleanups {} live {:?} doc {:?}",
            seen_b.renders.get(),
            seen_b.cleanups.get(),
            live_b,
            pr.b.handle.doc()
        );
        // The first mount (B's view of the embed A moved away) must not survive
        // at the second's place holding data-tag=B (B's lost edit) or the second's
        // state.
        for (i, _) in &live_b {
            if *i == 0 {
                panic!("B's view of the moved embed survived the move");
            }
        }
        // B's view of E2 (mount 1), if it survived, must stand where A's view of
        // E2 (A's mount 1) stands: on the same node.
        if let Some((_, pb)) = live_b.iter().find(|(i, _)| *i == 1) {
            let pa = seen_a.ctx.borrow()[1].pos();
            assert_eq!(
                Some(*pb),
                pa,
                "B's view of E2 stands on A's E1 (A's E2 is at {pa:?})"
            );
        }
    }

    /// R10b — the same swap of two identical embeds, applied in order: B
    /// receives A's move (the document is equal by value, so B's model and views
    /// do not change), then B tags its first view. Decision (d) across peers:
    /// B's first view was mounted for E1, so the tag must reach A's view of E1
    /// (which A moved to second place), not A's view of E2.
    #[test]
    fn r10b_after_an_identical_swap_a_peers_edit_reaches_the_same_node() {
        let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
        let pr = pair(&format!("<p>ab</p>{e}{e}<p>cd</p>"));
        let seen_a = register(&pr.a);
        let seen_b = register(&pr.b);
        let first = pr.a.handle.doc().child(1).clone();
        assert!(pr.a.handle.update(|st| {
            let mut tr = st.tr();
            tr.delete(EMBED, EMBED + 1).ok()?;
            tr.replace(
                EMBED + 1,
                EMBED + 1,
                Slice::new(Fragment::from_node(first.clone()), 0, 0),
            )
            .ok()?;
            Some(tr)
        }));
        assert_eq!(seen_a.renders.get(), 2, "A keeps both mounts (same Rcs)");
        assert_eq!(
            seen_a.ctx.borrow()[0].pos(),
            Some(Pos(EMBED + 1)),
            "A's E1 is second"
        );
        let ds: Vec<_> = pr.to_b.borrow_mut().drain(..).collect();
        let changed: Vec<bool> = ds.iter().map(|d| pr.b.handle.collab_receive(d)).collect();
        eprintln!(
            "B receive results {changed:?}, B renders {}",
            seen_b.renders.get()
        );
        // The CRDT holds no move: A wrote the embeds it put in each other's
        // place as new blocks, so B cannot know which is which and must not keep a
        // view on a block that is gone. Its view of E1 is not live any more (the
        // fix-round's reading of this fixture: the review asked for it to survive,
        // which the concurrent R10 rules out — there it must not).
        assert_eq!(
            seen_b.ctx.borrow()[0].pos(),
            None,
            "B's view of E1 stands on a deleted CRDT block"
        );
        // Every view B still has edits, on A, the node A's view at the same place
        // is mounted for.
        let live: Vec<(usize, Pos)> = seen_b
            .ctx
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.pos().map(|p| (i, p)))
            .collect();
        assert_eq!(live.len(), 2, "B shows both embeds");
        for (i, _) in &live {
            let tag = format!("B{i}");
            assert!(
                seen_b.ctx.borrow()[*i]
                    .set_attrs(|x| *x = x.with("data-tag", AttrValue::from(tag.as_str())))
            );
        }
        flush(&pr.to_a, &pr.a);
        for c in seen_a.ctx.borrow().iter() {
            let pa = c.pos().expect("A keeps both mounts");
            let (i, _) = live.iter().find(|(_, pb)| *pb == pa).unwrap();
            assert_eq!(
                c.attrs.get().get_str("data-tag"),
                Some(format!("B{i}").as_str()),
                "B's view at {pa:?} edits A's view at the same node"
            );
        }
    }

    /// R11 — an older-build peer (plain `record_local`, the positional rule)
    /// swaps two *different* embeds. The newer peer B sees the two CRDT blocks
    /// keep their ids with exchanged attributes, and keeps each view mounted on
    /// the other node's data: what the guide's coordinated-upgrade sentence warns
    /// about. Asserts the warned behaviour, so it documents the limitation.
    #[test]
    fn r11_an_older_peer_swapping_two_embeds_hands_views_the_other_nodes_data() {
        let e1 = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
        let e2 = r#"<div data-pm-type="embed" data-sheet="s2"></div>"#;
        let a = page(&format!("<p>ab</p>{e1}{e2}<p>cd</p>"));
        let snapshot = a.handle.start_collaboration_host(|_| {}).unwrap();
        let b = page("<p></p>");
        b.handle
            .start_collaboration_guest(&snapshot, |_| {})
            .unwrap();
        let seen_b = register(&b);
        assert_eq!(*seen_b.mounted_for.borrow(), ["s1", "s2"]);

        // The older peer.
        let schema: Schema = b.handle.state().schema().clone();
        let mut old = CollabSession::from_bytes(&snapshot).unwrap();
        let before: Node = old.projected_doc(&schema).unwrap();
        let after = {
            let kids: Vec<Node> = (0..before.child_count())
                .map(|i| before.child(i).clone())
                .collect();
            let swapped = vec![
                kids[0].clone(),
                kids[2].clone(),
                kids[1].clone(),
                kids[3].clone(),
            ];
            before.copy_with_content(Fragment::from_children(swapped))
        };
        old.record_local(&schema, &before, &after).unwrap();
        let delta = old.save_incremental().unwrap();
        assert!(b.handle.collab_receive(&delta));
        let sheets: Vec<String> = seen_b
            .ctx
            .borrow()
            .iter()
            .map(|c| {
                c.attrs
                    .get()
                    .get_str("data-sheet")
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        eprintln!(
            "B renders {} mounted_for {:?} now {:?}",
            seen_b.renders.get(),
            seen_b.mounted_for.borrow(),
            sheets
        );
        assert_eq!(seen_b.renders.get(), 2, "(limitation) no remount");
        assert_eq!(
            sheets,
            ["s2", "s1"],
            "(limitation) each view handed the other's data"
        );
        let _ = EditorState::create;
    }

    /// R12 — undo of A's own attribute change after a peer changed another key.
    #[test]
    fn r12_undo_after_a_remote_attr_change_keeps_both_mounts() {
        let pr = pair(DOC);
        let seen_a = register(&pr.a);
        let seen_b = register(&pr.b);
        assert!(seen_a.ctx.borrow()[0].set_attrs(|x| *x = x.with("data-k1", AttrValue::from("a"))));
        flush(&pr.to_b, &pr.b);
        assert!(seen_b.ctx.borrow()[0].set_attrs(|x| *x = x.with("data-k2", AttrValue::from("b"))));
        flush(&pr.to_a, &pr.a);
        assert!(pr.a.handle.command("undo"));
        flush(&pr.to_b, &pr.b);
        assert_eq!(
            pr.a.handle.doc().child(1).attrs(),
            pr.b.handle.doc().child(1).attrs()
        );
        let at = pr.a.handle.doc().child(1).attrs().clone();
        assert_eq!(at.get_str("data-k1"), None, "A's change undone");
        assert_eq!(at.get_str("data-k2"), Some("b"), "B's change kept");
        assert_eq!(seen_a.renders.get(), 1);
        assert_eq!(seen_b.renders.get(), 1);
    }
}
