//! The pending focus request and the lent `tabindex` (#3062 review 3), on the
//! mock document: what a list registers, which list a request reaches, when
//! the request ends, and a lend that survives a detach.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::dom::insert_observer_count;
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope};

use super::*;

fn doc_scope() -> RenderScope {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    // The scope holds the document weakly; the test keeps it for good.
    std::mem::forget(doc.clone());
    RenderScope::new(doc, body)
}

fn item(scope: &mut RenderScope, key: &str) -> NodeHandle {
    SortableItem {
        value: key.into(),
        ..Default::default()
    }
    .render(scope, &[])
}

fn list(
    scope: &mut RenderScope,
    group: &str,
    id: &str,
    keyboard: bool,
    keys: &[&str],
) -> NodeHandle {
    let items: Vec<NodeHandle> = keys.iter().map(|k| item(scope, k)).collect();
    let list = SortableList {
        group: group.into(),
        id: id.into(),
        keyboard,
        ..Default::default()
    }
    .render(scope, &items);
    scope.body_handle().append_child(&list);
    list
}

fn pending() -> Option<(String, Option<String>)> {
    PENDING_FOCUS.with(|p| p.0.as_ref().map(|r| (r.key.clone(), r.list.clone())))
}

fn focused(node: &NodeHandle) -> bool {
    node.active_element()
        .is_some_and(|f| f.node_id() == node.node_id())
}

/// Finding 1: a list without `keyboard` registers no insertion observer, so
/// an app's plain lists cost no insertion anywhere a walk of its ancestors.
/// A `keyboard` list registers one (the control): it gives late items their
/// `tabindex`.
#[test]
fn a_plain_list_registers_no_insertion_observer() {
    let mut scope = doc_scope();
    let base = insert_observer_count();
    let _plain = list(&mut scope, "cards", "a", false, &["x", "y"]);
    assert_eq!(
        insert_observer_count(),
        base,
        "a plain list observes insertions"
    );
    let _keyed = list(&mut scope, "cards", "b", true, &["z"]);
    assert_eq!(insert_observer_count(), base + 1);
}

/// Findings 1–3: a request watches the plain list it names, and only that
/// list; the item arriving in another list of the group (a keyboard one,
/// which sees every arrival) or in a list of the same id in another group
/// does not take it; arriving where it was asked for, it takes the focus,
/// and the request and its watch end.
#[test]
fn a_request_reaches_only_its_list_and_ends_when_satisfied() {
    let mut scope = doc_scope();
    let doc = scope.body_handle().doc_key();
    let keyed = list(&mut scope, "cards", "a", true, &["x"]);
    let plain = list(&mut scope, "cards", "b", false, &["y"]);
    let other = list(&mut scope, "lanes", "b", true, &[]);
    let base = insert_observer_count();
    request_focus(doc, "cards".into(), "z".into(), Some("b".into()));
    assert_eq!(
        insert_observer_count(),
        base + 1,
        "the plain list it names watches"
    );

    for (at, what) in [
        (&keyed, "another list of the group"),
        (&other, "another group"),
    ] {
        let z = item(&mut scope, "z");
        at.append_child(&z);
        assert!(!focused(&z), "{what} took the request");
        assert_eq!(pending(), Some(("z".into(), Some("b".into()))), "{what}");
    }

    let z = item(&mut scope, "z");
    plain.append_child(&z);
    assert!(focused(&z));
    assert_eq!(z.get_attribute("tabindex").as_deref(), Some("-1"), "lent");
    assert_eq!(pending(), None, "a satisfied request lingers");
    assert_eq!(insert_observer_count(), base, "its watch outlived it");
}

/// Finding 3: a new request replaces the last one, its watches with it, and
/// a drag that starts drops it.
#[test]
fn a_request_is_replaced_and_dropped_at_the_next_drag() {
    let mut scope = doc_scope();
    let doc = scope.body_handle().doc_key();
    let keyed = list(&mut scope, "cards", "a", true, &["x"]);
    let _b = list(&mut scope, "cards", "b", false, &[]);
    let _c = list(&mut scope, "cards", "c", false, &[]);
    let base = insert_observer_count();
    request_focus(doc, "cards".into(), "z".into(), None);
    assert_eq!(insert_observer_count(), base + 2, "both plain lists watch");
    request_focus(doc, "cards".into(), "w".into(), Some("c".into()));
    assert_eq!(
        insert_observer_count(),
        base + 1,
        "the replaced request's watches stayed"
    );
    assert_eq!(pending(), Some(("w".into(), Some("c".into()))));

    let x = items_of(&keyed)[0].0.clone();
    assert!(begin(&x, "x", true));
    assert_eq!(pending(), None, "a drag left the request");
    assert_eq!(insert_observer_count(), base);
    end_active();
}

/// Finding 3: a plain list named by the request watches while it is drawn,
/// and its watch goes with it.
#[test]
fn a_named_list_drops_its_watch_when_it_goes() {
    let scope = doc_scope();
    let doc = scope.body_handle().doc_key();
    let base = insert_observer_count();
    request_focus(doc, "cards".into(), "z".into(), Some("b".into()));
    let body = scope.body_handle();
    let mut inner =
        RenderScope::with_parent(scope.doc_weak().upgrade().unwrap(), body.node_id(), None);
    let _b = list(&mut inner, "cards", "b", false, &[]);
    assert_eq!(
        insert_observer_count(),
        base + 1,
        "a list drawn under a request watches"
    );
    inner.dispose();
    assert_eq!(insert_observer_count(), base, "the watch outlived its list");
    cancel_pending_focus();
}

/// Finding 4: a lent `tabindex` survives its item's being taken out and put
/// back (another lend sweeps the records in between), and is taken back
/// when the focus leaves it.
#[test]
fn a_lend_survives_a_detach_and_goes_with_the_focus() {
    let mut scope = doc_scope();
    let doc = scope.body_handle().doc_key();
    let plain = list(&mut scope, "cards", "b", false, &["a", "b"]);
    let a = items_of(&plain)[0].0.clone();
    let b = items_of(&plain)[1].0.clone();
    focus_item(&a);
    assert_eq!(a.get_attribute("tabindex").as_deref(), Some("-1"));
    a.remove();
    focus_item(&b);
    plain.insert_before(&a, &b);
    assert_eq!(lent_in(doc), 2, "the detached item's lend was swept");
    rinch_core::notify_focus_moved(doc);
    assert_eq!(
        a.get_attribute("tabindex"),
        None,
        "the lend outlived the focus"
    );
    assert_eq!(
        b.get_attribute("tabindex").as_deref(),
        Some("-1"),
        "b holds the focus"
    );
}

/// The control for finding 4: an item the document let go of (discarded) is
/// swept at the next lend.
#[test]
fn a_discarded_items_lend_is_swept() {
    let mut scope = doc_scope();
    let plain = list(&mut scope, "cards", "b", false, &["a", "b"]);
    let a = items_of(&plain)[0].0.clone();
    let b = items_of(&plain)[1].0.clone();
    focus_item(&a);
    a.discard();
    focus_item(&b);
    let doc = b.doc_key();
    assert_eq!(lent_in(doc), 1, "a discarded item's lend outlived it");
}

/// A drop's request for `a` of the plain list `b` [a b c d], moved after
/// `b` and not applied yet, held by `a`'s node.
fn drop_request(scope: &mut RenderScope) -> (NodeHandle, NodeHandle) {
    let doc = scope.body_handle().doc_key();
    let plain = list(scope, "cards", "b", false, &["a", "b", "c", "d"]);
    let a = items_of(&plain)[0].0.clone();
    a.set_attribute("tabindex", "0");
    a.focus();
    let mv = SortMove {
        group: "cards".into(),
        keys: vec!["a".into()],
        from_list: "b".into(),
        to_list: "b".into(),
        place: SortPlace {
            after: Some("b".into()),
            before: Some("c".into()),
            index: 1,
        },
    };
    let landing = Landing::at_drop(&plain, &mv, "a");
    request_drop_focus(
        doc,
        "cards".into(),
        "a".into(),
        Some("b".into()),
        landing,
        &a,
    );
    assert_eq!(pending(), Some(("a".into(), Some("b".into()))), "control");
    (plain, a)
}

/// #3071 review 5, finding 3: the focus falling to the document (a click on
/// blank space, which a browser answers with `<body>`) while the lifted item
/// is still drawn drops a drop's request, so the drop applied after that
/// does not take the focus back.
#[test]
fn the_focus_falling_to_the_body_drops_a_drop_request() {
    let mut scope = doc_scope();
    let doc = scope.body_handle().doc_key();
    let _ = drop_request(&mut scope);
    scope.body_handle().focus();
    rinch_core::notify_focus_moved(doc);
    // Decided once the tree has settled (the scheduled check).
    try_pending_focus();
    assert_eq!(pending(), None, "the request outlived the blur");
}

/// The control: the focus falling to the document because a redraw took the
/// lifted item away keeps the request, for the node drawn in its place. In
/// both orders: desktop lets the focus go once the item is detached; the web
/// blurs it first, while it is still in place (#3071 review 6), and the
/// browser's `focusout` reaches the request before the redraw detaches it.
#[test]
fn the_focus_falling_with_its_item_keeps_a_drop_request() {
    for web_order in [false, true] {
        let mut scope = doc_scope();
        let doc = scope.body_handle().doc_key();
        let (_plain, a) = drop_request(&mut scope);
        if web_order {
            scope.body_handle().focus();
            rinch_core::notify_focus_moved(doc);
            a.remove();
        } else {
            a.remove();
            scope.body_handle().focus();
            rinch_core::notify_focus_moved(doc);
        }
        try_pending_focus();
        assert_eq!(
            pending(),
            Some(("a".into(), Some("b".into()))),
            "web order: {web_order}"
        );
        cancel_pending_focus();
    }
}

/// The web's order for a keyed redraw that applies the drop (#3071 review
/// 6): the lifted item is blurred before it is moved, and is still drawn,
/// now where the drop put it. The settled check gives it the focus back.
#[test]
fn a_keyed_move_after_the_web_blur_focuses_the_item() {
    let mut scope = doc_scope();
    let doc = scope.body_handle().doc_key();
    let (plain, a) = drop_request(&mut scope);
    scope.body_handle().focus();
    rinch_core::notify_focus_moved(doc);
    let c = items_of(&plain)[2].0.clone();
    plain.insert_before(&a, &c);
    assert_eq!(keys_of(&plain), ["b", "a", "c", "d"], "control: applied");
    try_pending_focus();
    assert_eq!(pending(), None, "the request outlived the keyed move");
    assert!(focused(&a), "the moved item lost the focus");
}

/// How many lent `tabindex` records document `doc` has.
fn lent_in(doc: u64) -> usize {
    LENT_TABINDEX.with(|l| l.iter().filter(|(d, _, _)| *d == doc).count())
}
