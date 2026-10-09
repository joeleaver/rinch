//! A write that replaces an element's child list tells the removal observer
//! above it (issue #1440) — on `rinch-dom`, where the issue was measured.
//!
//! `NodeHandle::set_text` on an **element** orphans every child
//! (`parent = None`, slab entry kept) and `NodeHandle::set_inner_html` frees
//! them; neither told an `on_child_removed` observer, so a container that
//! counts positions kept the ones it last derived. The mock's twins are
//! `dom::late_child::tests::*_written_over_an_elements_children_*` in
//! `rinch-core`; this file is the real backend, whose `set_inner_html` also
//! **re-issues** the freed ids to the nodes it parses.

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, on_child_inserted, on_child_removed};
use rinch_dom::RinchDocument;
use std::cell::RefCell;
use std::rc::Rc;

struct Fixture {
    doc: Rc<RefCell<dyn DomDocument>>,
    root: NodeHandle,
    wrapper: NodeHandle,
    kids: Vec<NodeHandle>,
}

impl Fixture {
    fn handle(&self, id: NodeId) -> NodeHandle {
        NodeHandle::new(id, Rc::downgrade(&self.doc))
    }
    fn element(&self, tag: &str) -> NodeHandle {
        let id = self.doc.borrow_mut().create_element(tag);
        self.handle(id)
    }
}

/// `body > div#root > section#wrapper > (span, span)`.
fn fixture() -> Fixture {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    let weak = Rc::downgrade(&doc);
    let (root, wrapper, kids) = {
        let mut d = doc.borrow_mut();
        let body = d.body();
        let root = d.create_element("div");
        d.append_child(body, root);
        let wrapper = d.create_element("section");
        d.append_child(root, wrapper);
        let kids: Vec<_> = (0..2)
            .map(|_| {
                let k = d.create_element("span");
                d.append_child(wrapper, k);
                k
            })
            .collect();
        (root, wrapper, kids)
    };
    Fixture {
        root: NodeHandle::new(root, weak.clone()),
        wrapper: NodeHandle::new(wrapper, weak.clone()),
        kids: kids
            .into_iter()
            .map(|k| NodeHandle::new(k, weak.clone()))
            .collect(),
        doc,
    }
}

fn watch(root: &NodeHandle) -> Rc<RefCell<Vec<NodeId>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    on_child_removed(root, move |node| sink.borrow_mut().push(node.node_id()));
    seen
}

#[test]
fn text_written_over_an_elements_children_tells_the_removal_observer() {
    let f = fixture();
    let seen = watch(&f.root);

    // Positive control: the observer hears an ordinary removal.
    let extra = f.element("i");
    f.wrapper.append_child(&extra);
    f.wrapper.remove_child(&extra);
    assert_eq!(*seen.borrow(), vec![f.wrapper.node_id()], "control");
    seen.borrow_mut().clear();

    f.wrapper.set_text("gone");

    assert!(
        f.kids.iter().all(|k| k.parent_node().is_none()),
        "precondition: `set_text_content` orphaned both spans"
    );
    assert_eq!(
        *seen.borrow(),
        vec![f.wrapper.node_id()],
        "#1440: the spans left `wrapper`, and the observer above it is told \
         once, with the node they left"
    );
}

#[test]
fn text_that_replaces_nothing_tells_nobody() {
    let f = fixture();
    f.wrapper.set_text("first");
    let seen = watch(&f.root);

    // `rinch-dom` returns before replacing anything when the element's one
    // text child already says this; the child list is the one it was.
    f.wrapper.set_text("first");
    assert!(
        seen.borrow().is_empty(),
        "an identical write moved no child"
    );

    // A different text replaces the text child with a new one: a child left.
    f.wrapper.set_text("second");
    assert_eq!(*seen.borrow(), vec![f.wrapper.node_id()]);

    // Text on a text node has no child list at all.
    seen.borrow_mut().clear();
    let text = f.wrapper.children().remove(0);
    text.set_text("third");
    assert!(seen.borrow().is_empty());
}

#[test]
fn html_written_over_an_elements_children_tells_the_removal_observer() {
    let f = fixture();
    let seen = watch(&f.root);
    // An observer on a child the write frees. `rinch-dom` re-issues the freed
    // slab ids to the nodes it parses, so one left behind would answer for a
    // node that has nothing to do with it.
    let stale = Rc::new(RefCell::new(0usize));
    {
        let (r, i) = (stale.clone(), stale.clone());
        on_child_removed(&f.kids[0], move |_| *r.borrow_mut() += 1);
        on_child_inserted(&f.kids[0], move |_| *i.borrow_mut() += 1);
    }

    f.wrapper
        .set_inner_html("<div><em>a</em></div><div><em>b</em></div>");

    assert_eq!(
        *seen.borrow(),
        vec![f.wrapper.node_id()],
        "#1440: the replaced children left `wrapper`"
    );

    // Drive both verbs through every parsed node; whichever took a freed id
    // must not reach the old child's observers.
    let mut stack = f.wrapper.children();
    let mut reused = false;
    while let Some(node) = stack.pop() {
        reused |= node.node_id() == f.kids[0].node_id();
        stack.extend(node.children());
        let probe = f.element("u");
        node.append_child(&probe);
        node.remove_child(&probe);
    }
    assert!(
        reused,
        "the fixture is only a test of this while the parse re-issues the \
         freed child's id"
    );
    assert_eq!(
        *stale.borrow(),
        0,
        "the observers registered on the freed child went with it"
    );
}

/// The list read back after `set_inner_html` cannot say whether a child left:
/// `rinch-dom` frees the old child and hands its id to the node it parses, so
/// one child replaced by one element reads back as the very same id list.
#[test]
fn html_that_re_issues_the_same_child_ids_still_tells_the_removal_observer() {
    let f = fixture();
    f.kids[1].remove();
    let before: Vec<_> = f.wrapper.children().iter().map(|c| c.node_id()).collect();
    assert_eq!(before, vec![f.kids[0].node_id()], "precondition: one child");
    let seen = watch(&f.root);

    f.wrapper.set_inner_html("<i></i>");

    let after: Vec<_> = f.wrapper.children().iter().map(|c| c.node_id()).collect();
    assert_eq!(
        after, before,
        "the fixture is only a test of this while the parsed <i> takes the \
         freed span's id"
    );
    assert_eq!(f.wrapper.children()[0].tag_name().as_deref(), Some("i"));
    assert_eq!(
        *seen.borrow(),
        vec![f.wrapper.node_id()],
        "the span left, whatever the ids say"
    );
}
