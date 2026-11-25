//! Writes that replace a child list (issue #1440): `set_text` on an
//! element and `set_inner_html` tell the removal half once, with the
//! element, and only when a child left.

use super::test_support::*;
use super::*;

/// `set_text` on an element replaces its children with the text, so each
/// of them left — and the container is told, once, with the element whose
/// children went (the former parent), nearest first.
#[test]
fn text_written_over_an_elements_children_tells_the_removal_observer() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let wrapper = element(&d, "section");
    root.append_child(&wrapper);
    let (a, b) = (element(&d, "span"), element(&d, "span"));
    wrapper.append_child(&a);
    wrapper.append_child(&b);

    let seen = watch_removals(&root);
    let landed = {
        let (seen, sink) = recorder();
        on_child_inserted(&root, move |node| sink.borrow_mut().push(node.node_id()));
        seen
    };

    wrapper.set_text("gone");

    assert_eq!(
        (
            a.parent_node().map(|p| p.node_id()),
            wrapper.children().len()
        ),
        (None, 0),
        "precondition: the write orphaned the children"
    );
    assert_eq!(
        *seen.borrow(),
        vec![wrapper.node_id()],
        "#1440: two children left `wrapper`, and the container above it is \
         told once, with the node they left"
    );
    assert!(
        landed.borrow().is_empty(),
        "the text that replaced them is not reported as an insertion: it \
         has no node of its own on the mock or in a browser"
    );
    // Detached, not retired: an orphan goes back in like a removed node.
    root.append_child(&a);
    assert_eq!(a.parent_node().map(|p| p.node_id()), Some(root.node_id()));
    forget((root.doc_key(), root.node_id()), Half::Removed);
    forget((root.doc_key(), root.node_id()), Half::Inserted);
}

/// The writes that take no child out say nothing: text on a text node (what
/// every reactive `{|| …}` in `rsx!` is), and text on an element that has
/// no children to lose.
#[test]
fn text_written_where_no_child_leaves_tells_nobody() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let label = text(&d, "one");
    root.append_child(&label);
    let empty = element(&d, "p");
    root.append_child(&empty);

    let seen = watch_removals(&root);

    label.set_text("two");
    empty.set_text("first");
    empty.set_text("second");

    assert!(
        seen.borrow().is_empty(),
        "no child left anything, so there is nothing to re-derive: {:?}",
        seen.borrow()
    );
    // Positive control: the observer is live.
    label.remove();
    assert_eq!(*seen.borrow(), vec![root.node_id()]);
    forget((root.doc_key(), root.node_id()), Half::Removed);
}

/// With nothing registered for removals, `set_text` asks the document
/// nothing it did not ask before — the cost rule of the module, for the
/// verb every reactive text effect runs.
#[test]
fn set_text_reads_no_child_list_while_nobody_watches_removals() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let child = element(&d, "span");
    root.append_child(&child);
    // An insertion observer alone must not turn the read on.
    on_child_inserted(&root, |_| {});

    let before = d.borrow().__get_children_calls();
    root.set_text("x");
    assert_eq!(
        d.borrow().__get_children_calls(),
        before,
        "no removal observer on the thread: one `Cell` read, no child list"
    );
    forget((root.doc_key(), root.node_id()), Half::Inserted);
}

/// With a removal observer on the thread — in **another** document, as a
/// mounted `Stepper` beside any reactive text is — a write to a text node
/// still reads no child list: 10,000 of them, zero `get_children`. An
/// element is asked once when it has no children and twice when it had.
#[test]
fn a_text_node_write_reads_no_child_list_with_an_observer_on_the_thread() {
    let (d, body) = doc();
    let label = text(&d, "0");
    body.append_child(&label);
    let (d2, body2) = doc();
    let other = element(&d2, "div");
    body2.append_child(&other);
    on_child_removed(&other, |_| {});

    let base = d.borrow().__get_children_calls();
    for i in 0..10_000 {
        label.set_text(&i.to_string());
    }
    assert_eq!(d.borrow().__get_children_calls() - base, 0);

    // The counter does see an element: empty, then with a child to lose.
    let host = element(&d, "p");
    body.append_child(&host);
    let base = d.borrow().__get_children_calls();
    host.set_text("a");
    assert_eq!(d.borrow().__get_children_calls() - base, 1);
    host.append_child(&element(&d, "i"));
    let base = d.borrow().__get_children_calls();
    host.set_text("b");
    assert_eq!(d.borrow().__get_children_calls() - base, 2);
    forget((other.doc_key(), other.node_id()), Half::Removed);
}

/// Rewriting an element's own text loses no child on the mock: the text
/// lives on the element, as a browser's has no rinch id (`rinch-dom`
/// writes its text node in place — `child_list_replaced_1440_tests`).
#[test]
fn rewriting_an_elements_own_text_tells_nobody() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let label = element(&d, "p");
    root.append_child(&label);
    label.set_text("0");
    let seen = watch_removals(&root);
    for i in 1..=10 {
        label.set_text(&i.to_string());
    }
    assert!(seen.borrow().is_empty());
    forget((root.doc_key(), root.node_id()), Half::Removed);
}

/// `set_inner_html` replaces a child list as well; the children it frees
/// left, and an observer registered **on** one of them goes with it — the
/// backend may hand a freed id to the next node it mints (`rinch-dom`'s
/// slab does), exactly as after a discard.
#[test]
fn html_written_over_an_elements_children_tells_the_removal_observer() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let wrapper = element(&d, "section");
    root.append_child(&wrapper);
    let inner = element(&d, "div");
    wrapper.append_child(&inner);
    inner.append_child(&element(&d, "span"));

    let seen = watch_removals(&root);
    let stale = watch_removals(&inner);
    on_child_inserted(&inner, |_| {});
    assert_eq!((count_of(Half::Removed), count_of(Half::Inserted)), (2, 1));

    wrapper.set_inner_html("<b>new</b>");

    assert_eq!(
        *seen.borrow(),
        vec![wrapper.node_id()],
        "#1440: the replaced children left `wrapper`"
    );
    assert!(stale.borrow().is_empty());
    assert_eq!(
        (count_of(Half::Removed), count_of(Half::Inserted)),
        (1, 0),
        "both observers on the freed `inner` are dropped with it"
    );

    // An element with no children loses none.
    let empty = element(&d, "p");
    root.append_child(&empty);
    seen.borrow_mut().clear();
    empty.set_inner_html("<i>x</i>");
    assert!(seen.borrow().is_empty());
    forget((root.doc_key(), root.node_id()), Half::Removed);
}

/// A container's own `set_text` over children, made from inside its
/// callback, does not call it back (the re-entrancy rule).
#[test]
fn a_callbacks_own_text_write_over_children_does_not_call_it_back() {
    let (d, body) = doc();
    let root = element(&d, "div");
    body.append_child(&root);
    let scratch = element(&d, "p");
    root.append_child(&scratch);
    let doomed = element(&d, "span");
    root.append_child(&doomed);

    let calls = Rc::new(Cell::new(0));
    let (count, target, doc2) = (calls.clone(), scratch.clone(), d.clone());
    on_child_removed(&root, move |_| {
        count.set(count.get() + 1);
        target.append_child(&element(&doc2, "i"));
        target.set_text("patched");
    });

    doomed.remove();
    assert_eq!(calls.get(), 1);
    forget((root.doc_key(), root.node_id()), Half::Removed);
}
