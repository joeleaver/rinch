//! #1833: adjacent text runs share the wrapper of a mark that continues across
//! them. A mark that carries on from the run before keeps its open wrapper and
//! the run's other marks open inside it, so inline code split by bold, italic
//! or a link is one `<code>` element (one pill, now that an inline element's
//! padding takes room on its line, #1426) rather than one per run.

use super::*;
use rinch_core::dom::NodeId;
use rinch_core::dom::mock::MockDomDocument;
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Attrs, EditorState, Pos, Schema, Selection, default_plugins};
use std::cell::RefCell;
use std::rc::Rc;

struct H {
    doc: Rc<RefCell<dyn DomDocument>>,
    mock: Rc<RefCell<MockDomDocument>>,
    container: NodeHandle,
    container_id: NodeId,
}

fn harness() -> H {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let doc: Rc<RefCell<dyn DomDocument>> = mock.clone();
    let container_id = doc.borrow_mut().create_element("div");
    let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
    H {
        doc,
        mock,
        container,
        container_id,
    }
}

fn schema() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}

fn mark(s: &Schema, name: &str) -> Mark {
    Mark::new(s.mark_type(name).unwrap().clone(), Attrs::default())
}

fn link(s: &Schema, href: &str) -> Mark {
    Mark::new(
        s.mark_type("link").unwrap().clone(),
        Attrs::default().with("href", href),
    )
}

/// A mark set in the model's canonical (type-name) order, built directly:
/// the starter kit's `code` excludes bold, italic and links, but an app's
/// schema need not, and the view must draw what the model holds.
fn set(mut marks: Vec<Mark>) -> Vec<Mark> {
    marks.sort_by(|a, b| a.type_name().cmp(b.type_name()));
    marks
}

/// A paragraph of runs, each `(text, marks)`.
fn para(s: &Schema, runs: &[(&str, Vec<Mark>)]) -> Node {
    let nodes: Vec<Node> = runs
        .iter()
        .map(|(t, m)| s.text_with_marks(t, m.clone()).unwrap())
        .collect();
    s.branch("paragraph", Fragment::from_children(nodes))
        .unwrap()
}

fn state(s: Rc<Schema>, blocks: Vec<Node>) -> EditorState {
    let doc = s.branch("doc", Fragment::from_children(blocks)).unwrap();
    EditorState::create(s, doc, default_plugins())
}

/// The host subtree under `id` as markup: elements by tag, text verbatim.
fn shape(h: &H, id: NodeId) -> String {
    let d = h.doc.borrow();
    fn walk(d: &dyn DomDocument, id: NodeId, out: &mut String) {
        match d.tag_name(id) {
            Some(tag) => {
                out.push_str(&format!("<{tag}>"));
                for c in d.get_children(id) {
                    walk(d, c, out);
                }
                out.push_str(&format!("</{tag}>"));
            }
            None => out.push_str(&d.text_content(id).unwrap_or_default()),
        }
    }
    let mut out = String::new();
    for c in d.get_children(id) {
        walk(&*d, c, &mut out);
    }
    out
}

fn first_block(h: &H) -> NodeId {
    h.doc.borrow().get_children(h.container_id)[0]
}

fn mount(h: &H, st: &EditorState) -> RinchDomEditorView {
    RinchDomEditorView::new(h.container.clone(), Rc::downgrade(&h.doc), st)
}

#[test]
fn code_split_by_bold_italic_and_link_is_one_code_element() {
    let s = schema();
    for (inner, tag) in [
        (mark(&s, "bold"), "strong"),
        (mark(&s, "italic"), "em"),
        (link(&s, "https://x.test/"), "a"),
    ] {
        let h = harness();
        let code = mark(&s, "code");
        let st = state(
            s.clone(),
            vec![para(
                &s,
                &[
                    ("ab", vec![code.clone()]),
                    ("cd", set(vec![inner.clone(), code.clone()])),
                    ("ef", vec![code.clone()]),
                ],
            )],
        );
        let _view = mount(&h, &st);
        assert_eq!(
            shape(&h, first_block(&h)),
            format!("<code>ab<{tag}>cd</{tag}>ef</code>"),
            "inline code split by {tag} is one <code>"
        );
    }
}

#[test]
fn a_run_alone_keeps_the_canonical_nesting() {
    // No run before it: its marks open outermost-last-in-canonical-order, as a
    // run always nested (`<em><strong>`), and the copy-out HTML nests.
    let s = schema();
    let h = harness();
    let st = state(
        s.clone(),
        vec![para(
            &s,
            &[("x", vec![mark(&s, "bold"), mark(&s, "italic")])],
        )],
    );
    let _view = mount(&h, &st);
    assert_eq!(shape(&h, first_block(&h)), "<em><strong>x</strong></em>");
}

#[test]
fn a_mark_that_stops_closes_and_a_later_one_opens_again() {
    let s = schema();
    let h = harness();
    let (b, i) = (mark(&s, "bold"), mark(&s, "italic"));
    let st = state(
        s.clone(),
        vec![para(
            &s,
            &[
                ("a", vec![b.clone()]),
                ("b", vec![b.clone(), i.clone()]),
                ("c", vec![i.clone()]),
                ("d", vec![]),
                ("e", vec![b.clone()]),
            ],
        )],
    );
    let _view = mount(&h, &st);
    // italic continues from "b" into "c", but it was opened inside bold, which
    // stops: it closes with it and opens again.
    assert_eq!(
        shape(&h, first_block(&h)),
        "<strong>a<em>b</em></strong><em>c</em>d<strong>e</strong>"
    );
}

/// Bold added to the middle of a highlighted run, then removed: the view
/// restructures by structure, reusing the `<mark>` element and the first run's
/// text node.
#[test]
fn a_mark_added_and_removed_mid_run_restructures_in_place() {
    let s = schema();
    let h = harness();
    let code = mark(&s, "highlight");
    let st = state(s.clone(), vec![para(&s, &[("abcd", vec![code.clone()])])]);
    let mut view = mount(&h, &st);
    let p = first_block(&h);
    let code_el = h.doc.borrow().get_children(p)[0];
    let text_a = h.doc.borrow().get_children(code_el)[0];
    let nodes = h.mock.borrow().__node_count();

    // 0[p 1 a 2 b 3 c 4 d 5]6 — bold over "bc".
    let mut tr = st.tr();
    tr.add_mark(2, 4, mark(&s, "bold")).unwrap();
    let bolded = st.apply(tr);
    view.update_dom(&st, &bolded);
    assert_eq!(shape(&h, p), "<mark>a<strong>bc</strong>d</mark>");
    assert_eq!(h.doc.borrow().get_children(p), vec![code_el], "same <mark>");
    assert_eq!(
        h.doc.borrow().get_children(code_el)[0],
        text_a,
        "the first run keeps its text node"
    );

    let mut tr = bolded.tr();
    tr.remove_mark(2, 4, mark(&s, "bold")).unwrap();
    let plain = bolded.apply(tr);
    view.update_dom(&bolded, &plain);
    assert_eq!(shape(&h, p), "<mark>abcd</mark>");
    assert_eq!(h.doc.borrow().get_children(p), vec![code_el], "same <mark>");
    assert_eq!(
        h.mock.borrow().__node_count(),
        nodes,
        "the <strong> and the split runs' text nodes are gone"
    );
}

/// A run with no wrapper of its own any more, then one again: the wrappers
/// that lose their last run are discarded, not left empty in the host.
#[test]
fn wrappers_no_run_uses_are_discarded() {
    let s = schema();
    let h = harness();
    let (code, b) = (mark(&s, "highlight"), mark(&s, "bold"));
    let st = state(
        s.clone(),
        vec![para(
            &s,
            &[("ab", vec![code.clone()]), ("cd", vec![b.clone()])],
        )],
    );
    let mut view = mount(&h, &st);
    let p = first_block(&h);
    let nodes = h.mock.borrow().__node_count();
    // Highlight over everything: both runs share one <mark>.
    let mut tr = st.tr();
    tr.add_mark(1, 5, code.clone()).unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(shape(&h, p), "<mark>ab<strong>cd</strong></mark>");
    assert_eq!(
        h.mock.borrow().__node_count(),
        nodes,
        "one wrapper fewer than two <mark>s would make, no orphan left"
    );
}

/// Typing at the seam of two runs that share a wrapper patches one text node
/// and does nothing else to the host: no structure pass, no other write.
#[test]
fn typing_at_a_shared_seam_writes_only_the_text() {
    let s = schema();
    let h = harness();
    let code = mark(&s, "code");
    let st = state(
        s.clone(),
        vec![para(
            &s,
            &[
                ("ab", vec![code.clone()]),
                ("cd", set(vec![code.clone(), mark(&s, "bold")])),
            ],
        )],
    );
    let mut view = mount(&h, &st);
    let p = first_block(&h);
    let before = h.mock.borrow().__op_counts();
    let nodes = h.mock.borrow().__node_count();

    // The caret after "ab" carries `code` (the run before), so "Z" joins it.
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(3)));
    tr.insert_text("Z").unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);

    let ops = h.mock.borrow().__op_counts().since(before);
    assert_eq!(shape(&h, p), "<code>abZ<strong>cd</strong></code>");
    assert_eq!(h.mock.borrow().__node_count(), nodes, "no node made");

    // The control: the same keystroke in a paragraph of one unmarked run.
    let hc = harness();
    let stc = state(s.clone(), vec![para(&s, &[("abcd", vec![])])]);
    let mut viewc = mount(&hc, &stc);
    let beforec = hc.mock.borrow().__op_counts();
    let mut tr = stc.tr();
    tr.set_selection(Selection::cursor(Pos(3)));
    tr.insert_text("Z").unwrap();
    let nextc = stc.apply(tr);
    viewc.update_dom(&stc, &nextc);
    let control = hc.mock.borrow().__op_counts().since(beforec);
    assert_eq!(ops.text_writes, 1, "one text write");
    assert_eq!(ops, control, "no more host work than an unmarked paragraph");
}

/// A run typed at a seam with marks of its own is placed by structure: here
/// plain text typed between two code runs splits the shared `<code>`.
#[test]
fn a_new_run_at_a_shared_seam_splits_the_wrapper() {
    let s = schema();
    let h = harness();
    let code = mark(&s, "code");
    let st = state(
        s.clone(),
        vec![para(
            &s,
            &[
                ("ab", vec![code.clone()]),
                ("cd", set(vec![code.clone(), mark(&s, "bold")])),
            ],
        )],
    );
    let mut view = mount(&h, &st);
    let p = first_block(&h);
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(3)));
    tr.set_stored_marks(Some(Vec::new()));
    tr.insert_text("Z").unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(
        shape(&h, p),
        "<code>ab</code>Z<code><strong>cd</strong></code>"
    );
}

/// A decoration over two runs that share a wrapper splits each run inside its
/// own wrappers, and the shared one stays one element.
#[test]
fn a_decoration_across_a_shared_seam_keeps_the_wrapper() {
    let s = schema();
    let h = harness();
    let code = mark(&s, "code");
    let st = state(
        s.clone(),
        vec![para(
            &s,
            &[
                ("ab", vec![code.clone()]),
                ("cd", set(vec![code.clone(), mark(&s, "bold")])),
            ],
        )],
    );
    let mut view = mount(&h, &st);
    let p = first_block(&h);
    let decos = vec![InlineDeco {
        from: 2,
        to: 4,
        class: Rc::from("pm-spell-error"),
    }];
    let doc = Rc::downgrade(&h.doc);
    sync_inline_decos(&mut view.root, &decos, 0, &doc);
    assert_eq!(
        shape(&h, p),
        "<code><span>a<span>b</span></span><strong><span><span>c</span>d</span></strong></code>"
    );
    // And off again: the bare text nodes come back in the same wrappers.
    sync_inline_decos(&mut view.root, &[], 0, &doc);
    assert_eq!(shape(&h, p), "<code>ab<strong>cd</strong></code>");
    // A keystroke after a decoration pass still finds the runs in place.
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(2)));
    tr.insert_text("Z").unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(shape(&h, p), "<code>aZb<strong>cd</strong></code>");
}

/// A pointer hit on a shared wrapper's own box (its padding) resolves to the
/// first run inside it, as a hit on a run's own wrapper resolved to the run.
#[test]
fn a_hit_on_a_shared_wrapper_resolves_to_its_first_run() {
    let s = schema();
    let h = harness();
    let code = mark(&s, "code");
    let st = state(
        s.clone(),
        vec![para(
            &s,
            &[
                ("x", vec![]),
                ("ab", vec![code.clone()]),
                ("cd", set(vec![code.clone(), mark(&s, "bold")])),
            ],
        )],
    );
    let view = mount(&h, &st);
    let p = first_block(&h);
    let code_el = h.doc.borrow().get_children(p)[1];
    let (pos, node) = find_node_by_host(&view.root, code_el.0, 0).expect("resolves");
    assert_eq!(pos, 2, "the run starting after \"x\"");
    assert_eq!(node.text(), Some("ab"));
}

/// #1426's edge caret: at the seam of two runs sharing a `<code>`, that
/// `<code>`'s edges are not at the seam, so they are not offered.
#[test]
fn a_shared_wrapper_offers_no_edge_at_the_seam() {
    let s = schema();
    let h = harness();
    let code = mark(&s, "code");
    let st = state(
        s.clone(),
        vec![para(
            &s,
            &[
                ("ab", vec![code.clone()]),
                ("cd", set(vec![code.clone(), mark(&s, "bold")])),
            ],
        )],
    );
    let view = mount(&h, &st);
    let p = first_block(&h);
    let code_el = h.doc.borrow().get_children(p)[0].0;
    let strong = h.doc.borrow().get_children(NodeId(code_el))[1].0;
    for stored in [None, Some(set(vec![code.clone(), mark(&s, "bold")]))] {
        let edges = view.mark_edges_beside(
            &st.doc,
            Pos(3),
            stored.as_deref(),
            CaretAffinity::Downstream,
        );
        assert!(
            edges.iter().all(|&(e, _, _)| e != code_el),
            "no edge of the shared <code> at the seam: {edges:?}"
        );
        assert!(
            edges.iter().any(|&(e, _, _)| e == strong),
            "the <strong> starts here"
        );
    }
}
