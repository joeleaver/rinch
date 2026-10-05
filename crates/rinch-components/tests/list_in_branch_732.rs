//! A real `List { icon }` inside a branch whose rows arrive later (issue
//! #732): the icon `List` patches into a late row is built through
//! `late_children`'s scope, and the branch's hide must reclaim it with the row.
use rinch_components::list::{List, ListItem};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::{Component, Signal, for_each_dom_typed, show_dom};
use rinch_tabler_icons::TablerIcon;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn a_list_with_a_default_icon_inside_a_branch_does_not_grow_the_document() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut sc = RenderScope::new(doc.clone(), body);
    let bh = NodeHandle::new(body, Rc::downgrade(&doc) as _);
    let visible = Signal::new(false);
    let items = Signal::new(vec![1u32]);
    show_dom(
        &mut sc,
        &bh,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let rows = s.create_element("div");
            for_each_dom_typed(
                s,
                &rows,
                move || items.get(),
                |n: &u32| n.to_string(),
                |n: u32, rs: &mut RenderScope| {
                    let t = rs.create_text(&n.to_string());
                    ListItem::default().render(rs, &[t])
                },
            );
            List {
                icon: Some(TablerIcon::Check),
                ..Default::default()
            }
            .render(s, &[rows])
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    items.set(vec![1, 2]);
    visible.set(false);
    items.set(vec![1]);
    let base = doc.borrow().__node_count() as isize;
    for i in 0..100u32 {
        visible.set(true);
        items.set(vec![1, 2 + i]);
        visible.set(false);
        items.set(vec![1]);
    }
    let d = doc.borrow().__node_count() as isize - base;
    assert_eq!(d, 0, "List late-icon nodes leak: {d} over 100 toggles");
}
