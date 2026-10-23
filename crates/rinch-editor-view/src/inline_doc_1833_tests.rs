//! #1833 (review of PR #1862): a schema whose top node is itself a textblock
//! (`doc: text*`). The root's inline children are placed in their wrappers
//! like any textblock's, at mount and on every update.
use super::*;
use rinch_core::dom::NodeId;
use rinch_core::dom::mock::MockDomDocument;
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Attrs, EditorState, Pos, Schema, Selection, default_plugins};
use std::cell::RefCell;
use std::rc::Rc;

fn shape(d: &dyn DomDocument, id: NodeId) -> String {
    let mut out = String::new();
    for c in d.get_children(id) {
        match d.tag_name(c) {
            Some(tag) => out.push_str(&format!("<{tag}>{}</{tag}>", shape(d, c))),
            None => out.push_str(&d.text_content(c).unwrap_or_default()),
        }
    }
    out
}

#[test]
fn a_textblock_root_places_its_runs_in_their_wrappers() {
    let s = Rc::new(
        Schema::builder()
            .node(
                "doc",
                rinch_editor_core::NodeSpec::builder("doc")
                    .content("text*")
                    .build(),
            )
            .node(
                "text",
                rinch_editor_core::NodeSpec::builder("text")
                    .group("inline")
                    .inline()
                    .build(),
            )
            .mark("link", rinch_editor_core::MarkSpec::simple("link"))
            .build(),
    );
    let link = Mark::new(s.mark_type("link").unwrap().clone(), Attrs::default());
    let doc = s
        .branch(
            "doc",
            Fragment::from_children(vec![
                s.text("ab").unwrap(),
                s.text_with_marks("cd", vec![link.clone()]).unwrap(),
            ]),
        )
        .unwrap();
    let st = EditorState::create(s.clone(), doc, default_plugins());
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let ddoc: Rc<RefCell<dyn DomDocument>> = mock.clone();
    let cid = ddoc.borrow_mut().create_element("div");
    let container = NodeHandle::new(cid, Rc::downgrade(&ddoc));
    let mut view = RinchDomEditorView::new(container, Rc::downgrade(&ddoc), &st);
    let mounted = shape(&*ddoc.borrow(), cid);
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(4)));
    tr.set_stored_marks(Some(vec![]));
    tr.insert_text("Z").unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    let after = shape(&*ddoc.borrow(), cid);
    assert_eq!(mounted, "ab<a>cd</a>");
    assert_eq!(after, "ab<a>cd</a>Z");
}
