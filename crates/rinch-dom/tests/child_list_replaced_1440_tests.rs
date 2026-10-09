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

/// An element that holds nothing but its own text keeps its text node when the
/// text is rewritten: no child left, on this backend as on the mock and in a
/// browser (whose text has no rinch id at all). It used to orphan the node and
/// mint another per write, so an observer above was told a child left every
/// time (100 of 100).
#[test]
fn rewriting_an_elements_own_text_tells_nobody() {
    let f = fixture();
    f.wrapper.set_text("0");
    let text = f.wrapper.children().remove(0);
    let seen = watch(&f.root);

    for i in 1..=100 {
        f.wrapper.set_text(&i.to_string());
    }
    // An identical write, and text on the text node itself.
    f.wrapper.set_text("100");
    text.set_text("101");

    assert!(
        seen.borrow().is_empty(),
        "the element's child list never changed, told {} times",
        seen.borrow().len()
    );
    let now: Vec<_> = f.wrapper.children().iter().map(|c| c.node_id()).collect();
    assert_eq!(
        now,
        vec![text.node_id()],
        "the same text node, written in place"
    );
    assert_eq!(f.wrapper.text_content().as_deref(), Some("101"));

    // Positive control: the observer is live, and text over an element child
    // (not a lone text node) is still a child leaving.
    f.wrapper.append_child(&f.element("b"));
    f.wrapper.set_text("over");
    assert_eq!(*seen.borrow(), vec![f.wrapper.node_id()]);
    assert!(text.parent_node().is_none(), "both children were replaced");
}

/// The in-place write is a text edit like any other: the block is measured and
/// shaped again. Compared with a fresh document holding the final text, in a
/// block and in an atomic inline (sized by its own pass, #661), growing and
/// shrinking.
#[test]
fn an_elements_text_written_in_place_is_laid_out_again() {
    const CSS: &str = "
        .w { width: 100px; font-family: sans-serif; font-size: 16px; line-height: 20px; }
        .chip { display: inline-block; font-size: 16px; line-height: 20px; }
    ";
    const LONG: &str = "several words that wrap onto more lines than one";
    fn build(class: &str, text: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let outer = doc.create_element("div");
        doc.set_attribute(outer, "class", "w");
        doc.append_child(body, outer);
        let el = doc.create_element("div");
        doc.set_attribute(el, "class", class);
        doc.append_child(outer, el);
        doc.set_text_content(el, text);
        doc.resolve_layout(800.0, 600.0);
        (doc, el)
    }
    fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
        let l = &doc.tree.get(id.0).expect("live").layout;
        (l.width, l.height)
    }

    for class in ["plain", "chip"] {
        let (short_doc, short_el) = build(class, "a");
        let (long_doc, long_el) = build(class, LONG);
        let (short, long) = (size(&short_doc, short_el), size(&long_doc, long_el));
        assert_ne!(
            short, long,
            "{class}: counter-oracle, the two texts differ in size"
        );

        let (mut doc, el) = build(class, "a");
        let before = doc.get_children(el);
        doc.set_text_content(el, LONG);
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(doc.get_children(el), before, "{class}: written in place");
        assert_eq!(
            size(&doc, el),
            long,
            "{class}: grown text is measured again"
        );

        doc.set_text_content(el, "a");
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(size(&doc, el), short, "{class}: and shrunk text");
    }
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

// ---------------------------------------------------------------- review of #1489

fn find(f: &Fixture, from: NodeId, want: NodeId) -> Option<NodeHandle> {
    let kids = f.doc.borrow().get_children(from);
    for k in kids {
        if k == want {
            return Some(f.handle(k));
        }
        if let Some(found) = find(f, k, want) {
            return Some(found);
        }
    }
    None
}

/// The observed node is a **grandchild** of the node written over: every freed
/// node's observers go, not only the direct children's.
#[test]
fn an_observer_two_levels_below_the_write_is_dropped_too() {
    let f = fixture();
    let old = f.element("section");
    f.kids[0].append_child(&old);
    let calls = Rc::new(RefCell::new(0usize));
    let c = calls.clone();
    on_child_removed(&old, move |_| *c.borrow_mut() += 1);

    f.wrapper
        .set_inner_html("<a><b><i></i></b></a><u><s></s></u>");

    let heir = find(&f, f.wrapper.node_id(), old.node_id())
        .expect("the fixture needs the freed id re-issued to a parsed node");
    let probe = f.element("span");
    heir.append_child(&probe);
    probe.remove();
    assert_eq!(
        *calls.borrow(),
        0,
        "a stale observer answered for the node that inherited its id"
    );
}

/// Only an **insertion** observer exists on the thread (an app whose one
/// container is a `List`): the freed node's observer is dropped all the same.
#[test]
fn a_freed_insertion_only_observer_is_dropped_with_no_removal_observer_on_the_thread() {
    let f = fixture();
    let calls = Rc::new(RefCell::new(0usize));
    let c = calls.clone();
    on_child_inserted(&f.kids[0], move |_| *c.borrow_mut() += 1);

    f.wrapper.set_inner_html("<article></article><p></p>");

    let heir = find(&f, f.wrapper.node_id(), f.kids[0].node_id()).expect("re-issued");
    heir.append_child(&f.element("span"));
    assert_eq!(
        *calls.borrow(),
        0,
        "the freed node's observer patched an unrelated node"
    );
}

/// Markup or text written over the **container itself** tells its own removal
/// observer and leaves both of its registrations alive: only what is beneath
/// the written node is forgotten.
#[test]
fn a_write_over_the_container_itself_keeps_and_tells_its_own_observers() {
    let f = fixture();
    let (removed, inserted) = (Rc::new(RefCell::new(0usize)), Rc::new(RefCell::new(0usize)));
    let (r, i) = (removed.clone(), inserted.clone());
    on_child_removed(&f.wrapper, move |_| *r.borrow_mut() += 1);
    on_child_inserted(&f.wrapper, move |_| *i.borrow_mut() += 1);

    f.wrapper.set_inner_html("<b></b>");
    assert_eq!(*removed.borrow(), 1, "told its own children went");

    let probe = f.element("i");
    f.wrapper.append_child(&probe);
    probe.remove();
    assert_eq!(
        (*inserted.borrow(), *removed.borrow()),
        (1, 2),
        "its registrations survived the write"
    );

    f.wrapper.set_text("t");
    assert_eq!(*removed.borrow(), 3, "and the same through `set_text`");
}
