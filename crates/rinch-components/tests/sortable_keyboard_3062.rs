//! `SortableList::keyboard` (#3062): the keyboard drag is the app's to turn
//! on. Off — the default — a list takes no key and its items are no Tab stops;
//! on, each item is a Tab stop with a description of the keys, the items that
//! arrive later included, and what the app wrote on an item stays.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::{SortableItem, SortableList, sortable_drag_group};
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{self, EventHandlerId, KeyEventData};

fn doc_scope() -> RenderScope {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    // The scope holds the document weakly; the test keeps it for good.
    std::mem::forget(doc.clone());
    RenderScope::new(doc, body)
}

fn item(scope: &mut RenderScope, key: &str, children: &[NodeHandle]) -> NodeHandle {
    SortableItem {
        value: key.into(),
        ..Default::default()
    }
    .render(scope, children)
}

fn list(scope: &mut RenderScope, keyboard: bool, items: &[NodeHandle]) -> NodeHandle {
    let list = SortableList {
        group: "cards".into(),
        keyboard,
        ..Default::default()
    }
    .render(scope, items);
    scope.body_handle().append_child(&list);
    list
}

fn key_on(node: &NodeHandle, key: &str, code: &str) -> bool {
    let id = node
        .get_attribute("data-onkeydown")
        .unwrap()
        .parse()
        .unwrap();
    events::dispatch_key_handler(
        EventHandlerId(id),
        &KeyEventData::new(key, code).with_target(Some(node.node_id())),
    )
}

const DESCRIPTION: &str =
    "Press Space to lift, the arrow keys to move, Space to drop, Escape to cancel.";

#[test]
fn by_default_a_list_takes_no_key_and_its_items_are_no_tab_stops() {
    let mut scope = doc_scope();
    let a = item(&mut scope, "a", &[]);
    let b = item(&mut scope, "b", &[]);
    let _list = list(&mut scope, false, &[a.clone(), b.clone()]);
    for n in [&a, &b] {
        assert_eq!(n.get_attribute("tabindex"), None);
        assert_eq!(n.get_attribute("aria-description"), None);
    }
    assert!(!key_on(&a, " ", "Space"), "Space is the app's");
    assert!(!key_on(&a, "Enter", "Enter"), "Enter is the app's");
    assert!(!key_on(&a, "ArrowDown", "ArrowDown"));
    assert_eq!(sortable_drag_group(), None);
}

#[test]
fn a_keyboard_list_makes_its_items_tab_stops_and_lifts_by_key() {
    let mut scope = doc_scope();
    let a = item(&mut scope, "a", &[]);
    let _list = list(&mut scope, true, std::slice::from_ref(&a));
    assert_eq!(a.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(
        a.get_attribute("aria-description").as_deref(),
        Some(DESCRIPTION)
    );
    assert!(key_on(&a, " ", "Space"));
    assert_eq!(sortable_drag_group().as_deref(), Some("cards"));
    assert!(key_on(&a, "Escape", "Escape"));
    assert_eq!(sortable_drag_group(), None);
}

#[test]
fn an_item_appended_to_a_keyboard_list_later_is_a_tab_stop_too() {
    let mut scope = doc_scope();
    let a = item(&mut scope, "a", &[]);
    let list = list(&mut scope, true, std::slice::from_ref(&a));
    let late = item(&mut scope, "late", &[]);
    list.append_child(&late);
    assert_eq!(late.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(
        late.get_attribute("aria-description").as_deref(),
        Some(DESCRIPTION)
    );
}

#[test]
fn what_the_app_wrote_on_an_item_stays() {
    let mut scope = doc_scope();
    let a = item(&mut scope, "a", &[]);
    a.set_attribute("tabindex", "-1");
    a.set_attribute("aria-description", "The app's words");
    let _list = list(&mut scope, true, std::slice::from_ref(&a));
    assert_eq!(a.get_attribute("tabindex").as_deref(), Some("-1"));
    assert_eq!(
        a.get_attribute("aria-description").as_deref(),
        Some("The app's words")
    );
}

#[test]
fn a_disabled_item_gets_no_description_of_keys_it_does_not_take() {
    let mut scope = doc_scope();
    let a = SortableItem {
        value: "a".into(),
        disabled: true,
        ..Default::default()
    }
    .render(&mut scope, &[]);
    let _list = list(&mut scope, true, std::slice::from_ref(&a));
    assert_eq!(a.get_attribute("aria-description"), None);
}

#[test]
fn a_keyboard_list_leaves_a_nested_lists_items_to_that_list() {
    let mut scope = doc_scope();
    let card = item(&mut scope, "card", &[]);
    let inner = SortableList {
        group: "inner".into(),
        ..Default::default()
    }
    .render(&mut scope, std::slice::from_ref(&card));
    let wrapper = scope.create_element("div");
    wrapper.append_child(&inner);
    let column = item(&mut scope, "column", &[]);
    let _outer = list(&mut scope, true, &[column.clone(), wrapper.clone()]);
    assert_eq!(column.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(card.get_attribute("tabindex"), None, "the inner list's");
    assert_eq!(card.get_attribute("aria-description"), None);
    // A card that lands in the inner list later is the inner list's too.
    let late = item(&mut scope, "late", &[]);
    inner.append_child(&late);
    assert_eq!(late.get_attribute("tabindex"), None);
    assert!(!key_on(&card, " ", "Space"));
    assert_eq!(sortable_drag_group(), None);
}
