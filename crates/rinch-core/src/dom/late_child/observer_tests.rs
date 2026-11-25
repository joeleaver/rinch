//! How an observer runs and what releases it: untracked inside an effect
//! (issue #931), and a scope cleanup that releases its own registration
//! only (issue #1490).

use super::test_support::*;
use super::*;
use crate::dom::mock::MockDomDocument;

/// An observer runs inside whatever effect made the insertion or removal
/// — every `for` reconcile, `if` branch swap and component re-render is
/// one — so its reads must subscribe nobody (issue #931). The effect is
/// created inside another effect's run, two frames deep, which is what
/// tells whole-stack suspension apart from plain `untracked` (#932).
fn assert_observer_untracked(
    register: impl FnOnce(&NodeHandle, Rc<dyn Fn()>),
    drive: fn(&Rc<RefCell<MockDomDocument>>, &NodeHandle),
) {
    use crate::reactive::{Effect, Signal};
    use std::cell::Cell;
    let (own, store) = (Signal::new(0), Signal::new(0));
    let fired = Rc::new(Cell::new(0u32));
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    register(&root, {
        let fired = fired.clone();
        Rc::new(move || {
            fired.set(fired.get() + 1);
            store.get();
        })
    });
    let (outer, inner) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(0u32)));
    let slot: Rc<RefCell<Option<Effect>>> = Rc::default();
    let _outer = Effect::new({
        let (outer, inner, slot, root) = (outer.clone(), inner.clone(), slot.clone(), root.clone());
        move || {
            outer.set(outer.get() + 1);
            let (inner, d, root) = (inner.clone(), d.clone(), root.clone());
            slot.replace(Some(Effect::new(move || {
                own.get();
                inner.set(inner.get() + 1);
                drive(&d, &root);
            })));
        }
    });
    assert_eq!(
        fired.get(),
        1,
        "control: the observer ran inside the effect"
    );
    store.set(1);
    assert_eq!(
        (outer.get(), inner.get()),
        (1, 1),
        "a signal only the observer read re-ran an effect"
    );
    own.set(1);
    assert_eq!(inner.get(), 2, "positive control: the effect is live");
    forget((root.doc_key(), root.node_id()), Half::Inserted);
    forget((root.doc_key(), root.node_id()), Half::Removed);
}

#[test]
fn an_insertion_observer_run_from_an_effect_subscribes_nobody() {
    assert_observer_untracked(
        |root, obs| on_child_inserted(root, move |_| obs()),
        |d, root| root.append_child(&element(d, "b")),
    );
}

#[test]
fn a_removal_observer_run_from_an_effect_subscribes_nobody() {
    assert_observer_untracked(
        |root, obs| on_child_removed(root, move |_| obs()),
        |d, root| {
            let row = element(d, "b");
            root.append_child(&row);
            row.remove();
        },
    );
}

/// Issue #1490. A registration's scope cleanup releases **that
/// registration**, not whatever is registered under the node's id when it
/// runs. The mock never re-issues an id, so the second registration here is
/// made on the same node (it replaces the first, as documented); on
/// `rinch-dom` the same cleanup reaches a *different* container that was
/// handed the freed id (`late_child_reissued_id_1490_tests`).
#[test]
fn a_replaced_registration_survives_the_first_scopes_cleanup() {
    for half in [Half::Inserted, Half::Removed] {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);
        let watch = |runs: &Rc<RefCell<usize>>| {
            let sink = runs.clone();
            let f = move |_: &NodeHandle| *sink.borrow_mut() += 1;
            match half {
                Half::Inserted => on_child_inserted(&root, f),
                Half::Removed => on_child_removed(&root, f),
            }
        };
        let churn = || {
            let k = element(&d, "i");
            root.append_child(&k);
            k.remove();
        };

        let (first_runs, second_runs) = (Rc::new(RefCell::new(0)), Rc::new(RefCell::new(0)));
        let first = crate::reactive::Scope::new();
        first.run(|| watch(&first_runs));
        let second = crate::reactive::Scope::new();
        second.run(|| watch(&second_runs));
        let live = count_of(half);

        churn();
        assert_eq!(
            (*first_runs.borrow(), *second_runs.borrow()),
            (0, 1),
            "control: the second registration replaced the first"
        );

        first.dispose();
        assert_eq!(
            count_of(half),
            live,
            "the first scope's registration was already replaced, so its \
             cleanup has nothing of its own to release"
        );
        churn();
        assert_eq!(
            *second_runs.borrow(),
            2,
            "#1490: the second registration is still there after the first \
             scope's cleanup"
        );

        second.dispose();
        assert_eq!(
            count_of(half),
            live - 1,
            "and its own scope's cleanup does release it"
        );
        churn();
        assert_eq!(*second_runs.borrow(), 2);
    }
}

/// The other direction of #1490's token: a discard drops the entry, and a
/// registration made afterwards under the same key is not the one the
/// discarded container's scope may release.
#[test]
fn a_cleanup_after_the_container_was_discarded_releases_nothing_newer() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let old = crate::reactive::Scope::new();
    old.run(|| on_child_inserted(&root, |_| {}));
    // What `NodeHandle::discard` does to the registry, without retiring the
    // mock's node — standing in for a backend that re-issues the id.
    forget_node(&root);

    let runs = Rc::new(RefCell::new(0));
    let sink = runs.clone();
    on_child_inserted(&root, move |_| *sink.borrow_mut() += 1);
    old.dispose();
    root.append_child(&element(&d, "i"));
    assert_eq!(*runs.borrow(), 1, "the newer registration is untouched");
}
