//! The insertion half (issues #716, #732): every insertion verb tells the
//! observers, nearest first, never from inside a callback, and a
//! registration goes with its container.

use super::test_support::*;
use super::*;
use crate::dom::traits::DomDocument;

#[test]
fn each_of_the_four_insertion_verbs_tells_the_observer() {
    for verb in [
        "append_child",
        "insert_before",
        "insert_after",
        "replace_with",
    ] {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);
        let anchor = element(&d, "span");
        root.append_child(&anchor);

        let (seen, sink) = recorder();
        on_child_inserted(&root, move |node| sink.borrow_mut().push(node.node_id()));

        let fresh = element(&d, "b");
        match verb {
            "append_child" => root.append_child(&fresh),
            "insert_before" => root.insert_before(&fresh, &anchor),
            "insert_after" => anchor.insert_after(&fresh),
            _ => anchor.replace_with(&fresh),
        }

        assert_eq!(
            *seen.borrow(),
            vec![fresh.node_id()],
            "`{verb}` puts a node into a tree, so it has to tell the \
             container that node landed in (issue #716)"
        );
        forget((root.doc_key(), root.node_id()), Half::Inserted);
    }
}

#[test]
fn every_registered_ancestor_is_told_nearest_first() {
    let (d, body) = doc();
    let outer = element(&d, "div");
    let inner = element(&d, "div");
    body.append_child(&outer);
    outer.append_child(&inner);

    let order = Rc::new(RefCell::new(Vec::new()));
    let sink = order.clone();
    on_child_inserted(&outer, move |_| sink.borrow_mut().push("outer"));
    let sink = order.clone();
    on_child_inserted(&inner, move |_| sink.borrow_mut().push("inner"));

    let fresh = element(&d, "b");
    inner.append_child(&fresh);

    assert_eq!(
        *order.borrow(),
        vec!["inner", "outer"],
        "nearest first. Both are told because two containers of different \
         kinds can nest — a radio group holding a list — and each decides \
         for itself whether the node is its business"
    );
    forget((outer.doc_key(), outer.node_id()), Half::Inserted);
    forget((inner.doc_key(), inner.node_id()), Half::Inserted);
}

#[test]
fn a_callbacks_own_edits_do_not_call_it_back() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);

    let runs = Rc::new(RefCell::new(0));
    let sink = runs.clone();
    let d2 = d.clone();
    on_child_inserted(&root, move |node| {
        *sink.borrow_mut() += 1;
        // Exactly what a container's patch does: put something into the
        // subtree it is watching.
        let patch = element(&d2, "i");
        node.append_child(&patch);
    });

    let fresh = element(&d, "b");
    root.append_child(&fresh);

    assert_eq!(
        *runs.borrow(),
        1,
        "an observer that edits the tree it watches must not re-enter \
         itself — the alternative is an unbounded loop, not a missed patch"
    );
    forget((root.doc_key(), root.node_id()), Half::Inserted);
}

#[test]
fn unmounting_the_container_releases_its_observer() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);

    let runs = Rc::new(RefCell::new(0));
    {
        // A scope of the shape a component renders under. Registration ties
        // itself to the ambient owner, so dropping the scope has to take the
        // observer with it.
        let scope =
            crate::dom::RenderScope::new(d.clone() as Rc<RefCell<dyn DomDocument>>, body.node_id());
        let owner = scope.push_owner();
        let sink = runs.clone();
        on_child_inserted(&root, move |_| *sink.borrow_mut() += 1);
        drop(owner);

        root.append_child(&element(&d, "i"));
        assert_eq!(
            *runs.borrow(),
            1,
            "positive control: the observer fires while its scope is alive"
        );
    }

    root.append_child(&element(&d, "b"));
    assert_eq!(
        *runs.borrow(),
        1,
        "and not after it is gone. `on_child_inserted` is public, so a \
         third-party container may capture a `Signal` its scope owns — \
         firing an observer whose scope has been disposed is the #141 PR4 \
         use-after-free, and nothing else in this module catches it: \
         `discarding_the_container_drops_its_observer` pins the other \
         release path"
    );
}

#[test]
fn discarding_the_container_drops_its_observer() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);

    let runs = Rc::new(RefCell::new(0));
    let sink = runs.clone();
    on_child_inserted(&root, move |_| *sink.borrow_mut() += 1);

    let before = count_of(Half::Inserted);
    assert!(before > 0, "positive control: the observer was registered");
    root.discard();
    assert_eq!(
        count_of(Half::Inserted),
        before - 1,
        "a discarded id may be handed to the next node the document mints, \
         and an observer left under it would fire for a container that no \
         longer exists"
    );
}

#[test]
fn two_documents_on_one_thread_do_not_collide() {
    // Node ids are per-document slab indices, so the same id exists in both.
    let (first, first_body) = doc();
    let (second, second_body) = doc();
    let a = element(&first, "div");
    let b = element(&second, "div");
    first_body.append_child(&a);
    second_body.append_child(&b);
    assert_eq!(
        a.node_id(),
        b.node_id(),
        "precondition: the two containers share an id, which is what makes \
         this a test of the doc_key half of the key (issue #134)"
    );

    let runs = Rc::new(RefCell::new(0));
    let sink = runs.clone();
    on_child_inserted(&a, move |_| *sink.borrow_mut() += 1);

    b.append_child(&element(&second, "i"));
    assert_eq!(
        *runs.borrow(),
        0,
        "the other document's insertion is not this container's business"
    );
    a.append_child(&element(&first, "i"));
    assert_eq!(*runs.borrow(), 1, "positive control: this one is");
    forget((a.doc_key(), a.node_id()), Half::Inserted);
}
