//! An `Editor` inside an `if` branch (issue #732). Its view mints its block
//! nodes by raw backend access, not through a `RenderScope`; they have no
//! ownership record and must still be discarded with the branch.
use rinch_core::Component;
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_core::show_dom;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn an_editor_inside_a_branch_does_not_grow_the_document() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body_id = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut sc = RenderScope::new(dyn_doc.clone(), body_id);
    let body = NodeHandle::new(body_id, Rc::downgrade(&dyn_doc));
    let visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let ed = rinch_editor_view::Editor {
                content: "<p>one</p><p>two <strong>bold</strong></p><p>three</p>".into(),
                ..Default::default()
            };
            let node = ed.render(s, &[]);
            wrap.append_child(&node);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    visible.set(false);
    let base = doc.borrow().__node_count() as isize;
    for _ in 0..100 {
        visible.set(true);
        visible.set(false);
    }
    let delta = doc.borrow().__node_count() as isize - base;
    eprintln!("editor-in-branch delta over 100 toggles: {delta}");
    assert_eq!(delta, 0, "editor view nodes leak per hide");
}
