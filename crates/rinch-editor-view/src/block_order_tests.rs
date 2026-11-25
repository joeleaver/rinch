//! The root's blocks come first among its host children, in the model's order:
//! a block appended at the end (Enter at the end of the last paragraph, a peer's
//! new last block) goes in before the caret and selection overlays the view keeps
//! in the root, never after them. An app's sibling selectors over the blocks
//! (`p + p` for a first-line indent, `:last-child`) then read the document, not
//! wherever the caret happened to be.
use super::*;
use rinch_core::dom::NodeId;
use rinch_core::dom::mock::MockDomDocument;
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{EditorState, Pos, Schema, Selection, default_plugins};
use std::cell::RefCell;
use std::rc::Rc;

struct Host {
    doc: Rc<RefCell<dyn DomDocument>>,
    container_id: NodeId,
}

fn para(s: &Schema, t: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
        .unwrap()
}

fn mount(blocks: &[&str]) -> (Host, Rc<Schema>, EditorState, RinchDomEditorView) {
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch(
            "doc",
            Fragment::from_children(blocks.iter().map(|t| para(&s, t)).collect()),
        )
        .unwrap();
    let st = EditorState::create(s.clone(), doc, default_plugins());
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let ddoc: Rc<RefCell<dyn DomDocument>> = mock;
    let cid = ddoc.borrow_mut().create_element("div");
    let container = NodeHandle::new(cid, Rc::downgrade(&ddoc));
    let view = RinchDomEditorView::new(container, Rc::downgrade(&ddoc), &st);
    (
        Host {
            doc: ddoc,
            container_id: cid,
        },
        s,
        st,
        view,
    )
}

/// The root's host children, each as its block's text or the overlay it is.
fn order(host: &Host) -> Vec<String> {
    let d = host.doc.borrow();
    d.get_children(host.container_id)
        .into_iter()
        .map(|id| {
            for a in ["data-pm-caret", "data-pm-selection"] {
                if d.get_attribute(id, a).is_some() {
                    return a.to_string();
                }
            }
            d.text_content(id).unwrap_or_default()
        })
        .collect()
}

/// Shows a caret and a selection rectangle, as a focused editor does.
fn show_overlays(host: &Host, view: &mut RinchDomEditorView) {
    view.position_caret(4.0, 4.0, 16.0);
    let first = host.doc.borrow().get_children(host.container_id)[0];
    view.set_selection_rects(&[((0.0, 0.0, 10.0, 16.0), first.0, None)]);
}

#[test]
fn enter_at_the_end_puts_the_new_paragraph_before_the_overlays() {
    let (host, _s, st, mut view) = mount(&["one", "two"]);
    show_overlays(&host, &mut view);
    assert_eq!(
        order(&host),
        ["one", "two", "data-pm-caret", "data-pm-selection"]
    );
    // doc(p("one"), p("two")): "two" ends at 9; split there, as Enter does.
    let mut tr = st.tr();
    tr.split(9, 1, None).unwrap();
    tr.set_selection(Selection::cursor(Pos(11)));
    tr.insert_text("three").unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(
        order(&host),
        ["one", "two", "three", "data-pm-caret", "data-pm-selection"],
        "the new last paragraph follows the last block, not the overlays"
    );
}

#[test]
fn blocks_appended_at_the_end_keep_their_order_before_the_overlays() {
    let (host, s, st, mut view) = mount(&["one"]);
    show_overlays(&host, &mut view);
    let end = st.doc.content_size();
    let mut tr = st.tr();
    tr.replace_with(
        end,
        end,
        Fragment::from_children(vec![para(&s, "two"), para(&s, "three")]),
    )
    .unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(
        order(&host),
        ["one", "two", "three", "data-pm-caret", "data-pm-selection"]
    );
    // And a block between them still lands in its place.
    let mut tr = next.tr();
    tr.replace_with(5, 5, Fragment::from_node(para(&s, "between")))
        .unwrap();
    let last = next.apply(tr);
    view.update_dom(&next, &last);
    assert_eq!(
        order(&host),
        [
            "one",
            "between",
            "two",
            "three",
            "data-pm-caret",
            "data-pm-selection"
        ]
    );
}

#[test]
fn replacing_the_last_block_keeps_it_before_the_overlays() {
    let (host, s, st, mut view) = mount(&["one", "two"]);
    show_overlays(&host, &mut view);
    // "two" (5..10) replaced by two new paragraphs: a changed stretch at the end.
    let end = st.doc.content_size();
    let mut tr = st.tr();
    tr.replace_with(
        5,
        end,
        Fragment::from_children(vec![para(&s, "deux"), para(&s, "trois")]),
    )
    .unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(
        order(&host),
        ["one", "deux", "trois", "data-pm-caret", "data-pm-selection"]
    );
}

#[test]
fn a_changed_stretch_holding_a_node_view_lands_before_the_overlays() {
    // A view of the horizontal rule makes the changed stretch below one that is
    // diffed by node identity (`ViewDesc::diff_stretch_keyed`). The view is
    // never mounted here: nothing runs the view work.
    let s = Rc::new(Schema::starter_kit());
    let views = Rc::new(crate::node_views::Registry::default());
    views.add(crate::node_views::NodeViewSpec::new(
        "horizontal_rule",
        |_, _| unreachable!("the view work is not run in this test"),
    ));
    let hr = || s.branch("horizontal_rule", Fragment::empty()).unwrap();
    let doc = s
        .branch(
            "doc",
            Fragment::from_children(vec![para(&s, "one"), hr(), para(&s, "two")]),
        )
        .unwrap();
    let st = EditorState::create(s.clone(), doc, default_plugins());
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let ddoc: Rc<RefCell<dyn DomDocument>> = mock;
    let cid = ddoc.borrow_mut().create_element("div");
    let container = NodeHandle::new(cid, Rc::downgrade(&ddoc));
    let mut view = RinchDomEditorView::with_node_views(container, Rc::downgrade(&ddoc), &st, views);
    let host = Host {
        doc: ddoc,
        container_id: cid,
    };
    show_overlays(&host, &mut view);
    // The rule and "two" (5..end) become "deux", a new rule and "trois".
    let end = st.doc.content_size();
    let mut tr = st.tr();
    tr.replace_with(
        5,
        end,
        Fragment::from_children(vec![para(&s, "deux"), hr(), para(&s, "trois")]),
    )
    .unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(
        order(&host),
        [
            "one",
            "deux",
            "",
            "trois",
            "data-pm-caret",
            "data-pm-selection"
        ]
    );
}
