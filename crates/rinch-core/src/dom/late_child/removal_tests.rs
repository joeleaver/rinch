//! The removal half (issue #745), and the two halves together: a move tells
//! both containers, a reorder only the insertion half, and each half is
//! released on its own.

use super::test_support::*;
use super::*;
use crate::dom::traits::DomDocument;

#[test]
fn each_of_the_four_removal_verbs_tells_the_observer() {
    for verb in ["remove_child", "remove", "discard", "replace_with"] {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);
        let doomed = element(&d, "span");
        root.append_child(&doomed);

        let seen = watch_removals(&root);

        match verb {
            "remove_child" => root.remove_child(&doomed),
            "remove" => doomed.remove(),
            "discard" => doomed.discard(),
            _ => doomed.replace_with(&element(&d, "b")),
        }

        assert_eq!(
            *seen.borrow(),
            vec![root.node_id()],
            "`{verb}` takes a node out of a tree, so it has to tell the \
             container that node left — and it is handed the container, not \
             the node, which is detached by then and may be retired \
             (issue #745)"
        );
        forget((root.doc_key(), root.node_id()), Half::Removed);
    }
}

#[test]
fn an_insertion_does_not_fire_the_removal_half_or_the_other_way_round() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let child = element(&d, "span");
    root.append_child(&child);

    let (inserted, sink) = recorder();
    on_child_inserted(&root, move |node| sink.borrow_mut().push(node.node_id()));
    let removed = watch_removals(&root);

    let fresh = element(&d, "b");
    root.append_child(&fresh);
    assert_eq!(
        *inserted.borrow(),
        vec![fresh.node_id()],
        "positive control: the insertion half fired"
    );
    assert!(
        removed.borrow().is_empty(),
        "an arrival is not a departure: a container that registered only \
         for removals must not be woken by growth"
    );

    child.remove();
    assert_eq!(
        *removed.borrow(),
        vec![root.node_id()],
        "positive control: the removal half fired"
    );
    assert_eq!(
        inserted.borrow().len(),
        1,
        "and the insertion half was not fired again by it"
    );

    forget((root.doc_key(), root.node_id()), Half::Inserted);
    forget((root.doc_key(), root.node_id()), Half::Removed);
}

#[test]
fn a_move_tells_the_container_it_left_as_well_as_the_one_it_landed_in() {
    let (d, body) = doc();
    let from = element(&d, "div");
    let to = element(&d, "div");
    body.append_child(&from);
    body.append_child(&to);
    let child = element(&d, "span");
    from.append_child(&child);

    let left = watch_removals(&from);
    let (arrived, sink) = recorder();
    on_child_inserted(&to, move |node| sink.borrow_mut().push(node.node_id()));

    // An insertion verb handed a node that already has a parent: the
    // implicit detach a `for` reconcile relocates a row with.
    to.append_child(&child);

    assert_eq!(
        *arrived.borrow(),
        vec![child.node_id()],
        "positive control: the receiving container was told"
    );
    assert_eq!(
        *left.borrow(),
        vec![from.node_id()],
        "#745: and so was the one it came out of — a move is a removal for \
         the parent it leaves, which no removal *verb* ever runs for"
    );

    forget((from.doc_key(), from.node_id()), Half::Removed);
    forget((to.doc_key(), to.node_id()), Half::Inserted);
}

#[test]
fn a_reorder_inside_one_parent_is_not_reported_as_a_removal() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let first = element(&d, "span");
    let second = element(&d, "b");
    root.append_child(&first);
    root.append_child(&second);

    let removed = watch_removals(&root);

    // What a keyed `for` reorder does: relocate a live node under the parent
    // it is already in.
    root.insert_before(&second, &first);

    assert!(
        removed.borrow().is_empty(),
        "a reorder leaves the container's child *set* alone, and the \
         insertion half already reports it. Firing both would make an \
         idempotent container re-derive twice per moved row (issue #745)"
    );

    forget((root.doc_key(), root.node_id()), Half::Removed);
}

#[test]
fn every_registered_ancestor_is_told_of_a_removal_nearest_first() {
    let (d, body) = doc();
    let outer = element(&d, "div");
    let inner = element(&d, "div");
    body.append_child(&outer);
    outer.append_child(&inner);
    let leaf = element(&d, "span");
    inner.append_child(&leaf);

    let order = Rc::new(RefCell::new(Vec::new()));
    let sink = order.clone();
    on_child_removed(&outer, move |_| sink.borrow_mut().push("outer"));
    let sink = order.clone();
    on_child_removed(&inner, move |_| sink.borrow_mut().push("inner"));

    leaf.remove();

    assert_eq!(
        *order.borrow(),
        vec!["inner", "outer"],
        "the removal half walks the same chain the insertion half does, so \
         a container nested in another still decides for itself"
    );
    forget((outer.doc_key(), outer.node_id()), Half::Removed);
    forget((inner.doc_key(), inner.node_id()), Half::Removed);
}

#[test]
fn a_removal_callbacks_own_edits_do_not_call_it_back() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let a = element(&d, "span");
    let b = element(&d, "span");
    root.append_child(&a);
    root.append_child(&b);

    let runs = Rc::new(RefCell::new(0));
    let sink = runs.clone();
    let b_in_callback = b.clone();
    on_child_removed(&root, move |_| {
        *sink.borrow_mut() += 1;
        // A removal observer re-deriving its container may well remove
        // something: an unguarded dispatch is an unbounded loop here as much
        // as on the insertion side.
        b_in_callback.remove();
    });

    a.remove();

    assert_eq!(
        *runs.borrow(),
        1,
        "suppression covers both halves — they share one `DISPATCHING` flag"
    );
    forget((root.doc_key(), root.node_id()), Half::Removed);
}

#[test]
fn releasing_one_half_leaves_the_other_registered() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let child = element(&d, "span");
    root.append_child(&child);

    let inserts = Rc::new(RefCell::new(0));
    let removals = Rc::new(RefCell::new(0));
    {
        let scope =
            crate::dom::RenderScope::new(d.clone() as Rc<RefCell<dyn DomDocument>>, body.node_id());
        let owner = scope.push_owner();
        let sink = removals.clone();
        on_child_removed(&root, move |_| *sink.borrow_mut() += 1);
        drop(owner);
        // The other half registered outside that scope, so it keeps app
        // lifetime: the two are separate registrations with separate owners
        // and one being released must not take the other with it.
        let sink = inserts.clone();
        on_child_inserted(&root, move |_| *sink.borrow_mut() += 1);

        child.remove();
        assert_eq!(*removals.borrow(), 1, "positive control");
    }

    root.append_child(&child);
    assert_eq!(
        *inserts.borrow(),
        1,
        "the insertion half survives its sibling's release"
    );
    child.remove();
    assert_eq!(
        *removals.borrow(),
        1,
        "and the removal half is gone with the scope that registered it"
    );

    forget((root.doc_key(), root.node_id()), Half::Inserted);
}

#[test]
fn discarding_the_container_drops_both_of_its_observers() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);

    on_child_inserted(&root, |_| {});
    on_child_removed(&root, |_| {});
    assert_eq!(
        (count_of(Half::Inserted), count_of(Half::Removed)),
        (1, 1),
        "positive control: both halves were registered"
    );

    root.discard();
    assert_eq!(
        (count_of(Half::Inserted), count_of(Half::Removed)),
        (0, 0),
        "a discarded id may be handed to the next node the document mints, \
         so `forget_node` has to release both halves and not only the one \
         it knew about before #745"
    );
}
