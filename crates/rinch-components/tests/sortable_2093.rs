//! Sortable lists (#2091, review of #2093) on `MockDomDocument`: a document
//! that answers no `bounding_rect` and has no runtime behind it, driven by
//! dispatching the handler attributes alone — the position of a plugin host
//! that proxies `data-*` handlers and nothing else. The desktop event path is
//! `rinch/src/app/sortable_2091_tests.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::{
    SortAxis, SortMove, SortVerdict, SortableItem, SortableList, sortable_drag_group,
};
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::element::ValueCallback;
use rinch_core::events::{self, ClickContext, EventHandlerId, KeyEventData};

type Offers = Rc<RefCell<Vec<Option<(SortMove, SortVerdict)>>>>;

struct Board {
    _scope: RenderScope,
    offers: Offers,
}

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

fn list(
    scope: &mut RenderScope,
    proto: SortableList,
    offers: &Offers,
    items: &[NodeHandle],
) -> NodeHandle {
    let o = offers.clone();
    SortableList {
        ondropchange: Some(ValueCallback::new(move |x| o.borrow_mut().push(x))),
        ..proto
    }
    .render(scope, items)
}

fn cards(group: &str, id: &str) -> SortableList {
    SortableList {
        group: group.into(),
        id: id.into(),
        ..Default::default()
    }
}

fn find(root: &NodeHandle, key: &str) -> NodeHandle {
    fn walk(n: &NodeHandle, key: &str) -> Option<NodeHandle> {
        if n.get_attribute("data-sortable-key").as_deref() == Some(key) {
            return Some(n.clone());
        }
        n.children().iter().find_map(|c| walk(c, key))
    }
    walk(root, key).unwrap_or_else(|| panic!("no item {key}"))
}

fn handler(node: &NodeHandle, attr: &str) -> EventHandlerId {
    EventHandlerId(node.get_attribute(attr).unwrap().parse().unwrap())
}

/// A pointer at (`x`, `y`) over an element whose box is `(0, 0, 10, 10)`.
fn context(x: f32, y: f32) {
    events::set_click_context(ClickContext {
        mouse_x: x,
        mouse_y: y,
        element_x: 0.0,
        element_y: 0.0,
        element_width: 10.0,
        element_height: 10.0,
        ..Default::default()
    });
}

fn key_on(node: &NodeHandle, key: &str, code: &str) -> bool {
    events::dispatch_key_handler(
        handler(node, "data-onkeydown"),
        &KeyEventData::new(key, code).with_target(Some(node.node_id())),
    )
}

fn last(offers: &Offers) -> Option<(SortMove, SortVerdict)> {
    offers.borrow().last().cloned().flatten()
}

fn end_any_drag(root: &NodeHandle, key: &str) {
    events::dispatch_event(handler(&find(root, key), "data-ondragend"));
    events::notify_element_drag_ended();
}

/// A row of columns (group "columns"), each holding a list of cards.
fn columns_board() -> (Board, NodeHandle) {
    let mut scope = doc_scope();
    let offers: Offers = Rc::default();
    let mut cols = Vec::new();
    for (col, card) in [("col1", "a"), ("col2", "b"), ("col3", "c")] {
        let card = item(&mut scope, card, &[]);
        let inner = list(
            &mut scope,
            cards("cards", &format!("{col}-cards")),
            &offers,
            &[card],
        );
        cols.push(item(&mut scope, col, &[inner]));
    }
    let row = list(
        &mut scope,
        SortableList {
            axis: SortAxis::Horizontal,
            ..cards("columns", "row")
        },
        &offers,
        &cols,
    );
    scope.body_handle().append_child(&row);
    (
        Board {
            _scope: scope,
            offers,
        },
        row,
    )
}

#[test]
fn without_boxes_a_column_dragged_over_the_next_columns_cards_goes_past_it() {
    let (board, row) = columns_board();
    events::dispatch_event(handler(&find(&row, "col1"), "data-ondragstart"));
    assert_eq!(sortable_drag_group().as_deref(), Some("columns"));
    // Over card b, in col2: delegated to col2, whose box the document cannot
    // give. The drag is moving forward past col2: after it.
    context(5.0, 5.0);
    let b = find(&row, "b");
    events::dispatch_event(handler(&b, "data-ondragover"));
    let (m, _) = last(&board.offers).expect("a place past col2");
    assert_eq!(m.place.after.as_deref(), Some("col2"));
    assert_eq!(m.place.before.as_deref(), Some("col3"));
    end_any_drag(&row, "col1");
    drop(board);
}

#[test]
fn without_boxes_keyboard_moves_between_lists_follow_document_order() {
    let mut scope = doc_scope();
    let offers: Offers = Rc::default();
    // c2's list is made (registered) before c1's, and sits after it.
    let b = item(&mut scope, "b", &[]);
    let c2 = list(&mut scope, cards("cards", "c2"), &offers, &[b]);
    let a = item(&mut scope, "a", &[]);
    let c1 = list(
        &mut scope,
        cards("cards", "c1"),
        &offers,
        std::slice::from_ref(&a),
    );
    let row = scope.create_element("div");
    row.append_child(&c1);
    row.append_child(&c2);
    scope.body_handle().append_child(&row);

    assert!(key_on(&a, " ", "Space"), "lifted");
    assert!(key_on(&a, "ArrowRight", "ArrowRight"));
    let (m, _) = last(&offers).expect("an offer in the next list");
    assert_eq!(m.to_list, "c2", "the next in document order");
    assert!(key_on(&a, "Escape", "Escape"));
    assert_eq!(sortable_drag_group(), None);
}

#[test]
fn a_key_typed_in_a_field_inside_an_item_does_not_lift_it() {
    let mut scope = doc_scope();
    let offers: Offers = Rc::default();
    let field = scope.create_element("input");
    let a = item(&mut scope, "a", std::slice::from_ref(&field));
    let c1 = list(
        &mut scope,
        cards("cards", "c1"),
        &offers,
        std::slice::from_ref(&a),
    );
    scope.body_handle().append_child(&c1);
    // The document cannot say where the focus is; the event says the key
    // went to the field.
    let consumed = events::dispatch_key_handler(
        handler(&a, "data-onkeydown"),
        &KeyEventData::new(" ", "Space").with_target(Some(field.node_id())),
    );
    assert!(!consumed, "the field's Space");
    assert_eq!(sortable_drag_group(), None);
    drop(scope);
}

#[test]
fn lift_keys_leave_enter_to_the_app() {
    let mut scope = doc_scope();
    let offers: Offers = Rc::default();
    let a = item(&mut scope, "a", &[]);
    let c1 = list(
        &mut scope,
        SortableList {
            lift_keys: vec![" ".into()],
            ..cards("cards", "c1")
        },
        &offers,
        std::slice::from_ref(&a),
    );
    scope.body_handle().append_child(&c1);
    assert!(!key_on(&a, "Enter", "Enter"), "Enter is the app's");
    assert_eq!(sortable_drag_group(), None);
    assert!(key_on(&a, " ", "Space"));
    assert_eq!(sortable_drag_group().as_deref(), Some("cards"));
    assert!(key_on(&a, "Escape", "Escape"));
}

#[test]
fn a_disabled_list_starts_no_drag_and_takes_none() {
    let mut scope = doc_scope();
    let offers: Offers = Rc::default();
    let a = item(&mut scope, "a", &[]);
    let c1 = list(
        &mut scope,
        cards("cards", "c1"),
        &offers,
        std::slice::from_ref(&a),
    );
    let b = item(&mut scope, "b", &[]);
    let c2 = list(
        &mut scope,
        SortableList {
            disabled: true,
            ..cards("cards", "c2")
        },
        &offers,
        std::slice::from_ref(&b),
    );
    scope.body_handle().append_child(&c1);
    scope.body_handle().append_child(&c2);

    // From the disabled list: nothing starts, by pointer or key.
    events::dispatch_event(handler(&b, "data-ondragstart"));
    assert_eq!(sortable_drag_group(), None);
    assert!(!key_on(&b, " ", "Space"));
    assert_eq!(sortable_drag_group(), None);

    // Into it: nothing on offer.
    events::dispatch_event(handler(&a, "data-ondragstart"));
    context(5.0, 8.0);
    events::dispatch_event(handler(&b, "data-ondragover"));
    assert_eq!(last(&offers), None);
    events::dispatch_event(handler(&a, "data-ondragend"));
}

#[test]
fn a_disabled_item_is_no_drag_source() {
    let mut scope = doc_scope();
    let offers: Offers = Rc::default();
    let a = SortableItem {
        value: "a".into(),
        disabled: true,
        ..Default::default()
    }
    .render(&mut scope, &[]);
    let c1 = list(
        &mut scope,
        cards("cards", "c1"),
        &offers,
        std::slice::from_ref(&a),
    );
    scope.body_handle().append_child(&c1);
    assert_eq!(a.get_attribute("draggable"), None);
    assert!(!key_on(&a, " ", "Space"));
    events::dispatch_event(handler(&a, "data-ondragstart"));
    assert_eq!(sortable_drag_group(), None);
}

#[test]
fn without_the_stylesheet_the_live_region_and_reasons_stay_out_of_sight() {
    let mut scope = doc_scope();
    let offers: Offers = Rc::default();
    let a = item(&mut scope, "a", &[]);
    let c1 = list(
        &mut scope,
        cards("cards", "c1"),
        &offers,
        std::slice::from_ref(&a),
    );
    let mut all = Vec::new();
    fn walk(n: &NodeHandle, out: &mut Vec<NodeHandle>) {
        out.push(n.clone());
        for c in n.children() {
            walk(&c, out);
        }
    }
    walk(&c1, &mut all);
    let class_is = |n: &NodeHandle, c: &str| {
        n.get_attribute("class")
            .is_some_and(|v| v.split_whitespace().any(|w| w == c))
    };
    let live = all
        .iter()
        .find(|n| class_is(n, "rinch-sortable__live"))
        .unwrap();
    let style = live.get_attribute("style").unwrap_or_default();
    assert!(
        style.contains("position: absolute") && style.contains("overflow: hidden"),
        "{style}"
    );
    for reason in all.iter().filter(|n| class_is(n, "rinch-sortable__reason")) {
        let style = reason.get_attribute("style").unwrap_or_default();
        assert!(
            style.contains("display: none") && style.contains("position: absolute"),
            "{style}"
        );
    }
    // A painting host reads it on the source or an ancestor: the list
    // carries it (#2154: the list's `preview` decides it).
    assert_eq!(
        c1.get_attribute("data-drag-ghost").as_deref(),
        Some("false")
    );
}
