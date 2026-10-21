//! Issue #1032 (review of #1663, F2): the DOM menu bar invokes an item
//! straight from the `Menu`, so it has to hand the item's own frame to
//! `invoke_menu_callback`. Made in root 9 / document 7, the bar rendered in
//! root 3 / document 5 (an embedded context's mount), the click dispatched
//! from root 4 / document 2: the callback sees the item's frame, not the
//! renderer's or the click's.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::menu::{Menu, MenuItem};
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::{
    current_context_root, current_dispatching_doc, push_context_root, push_dispatching_doc,
};

fn walk(n: &NodeHandle, out: &mut Vec<NodeHandle>) {
    out.push(n.clone());
    for c in n.children() {
        walk(&c, out);
    }
}

fn label_of(n: &NodeHandle) -> String {
    let mut all = Vec::new();
    walk(n, &mut all);
    all.iter()
        .filter_map(|c| c.text_content())
        .collect::<Vec<_>>()
        .join("")
}

fn click(id: &str) {
    let id: u64 = id.parse().unwrap();
    rinch_core::events::dispatch_event(rinch_core::events::EventHandlerId(id as _));
}

type Seen = Rc<RefCell<Vec<(u64, Option<u64>)>>>;

#[test]
fn a_dom_bar_item_runs_under_the_document_and_root_it_was_made_in() {
    let seen: Seen = Rc::default();
    let s = seen.clone();
    let menu = {
        let _root = push_context_root(9);
        let _doc = push_dispatching_doc(7);
        Menu::new().item(MenuItem::new("ProbeItem").on_click(move || {
            s.borrow_mut()
                .push((current_context_root(), current_dispatching_doc()))
        }))
    };
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow_mut().create_element("body");
    let mut scope = {
        let _root = push_context_root(3);
        let _doc = push_dispatching_doc(5);
        RenderScope::new(doc.clone(), body)
    };
    let bar = {
        let _root = push_context_root(3);
        let _doc = push_dispatching_doc(5);
        rinch::menu::render_menu_bar_standalone(&mut scope, &[("File", &menu)], 0)
    };

    let _root = push_context_root(4);
    let _doc = push_dispatching_doc(2);
    // Open the menu, then click the item: up to three rounds, since opening
    // renders the entries.
    for _round in 0..3 {
        let mut all = Vec::new();
        walk(&bar, &mut all);
        let item = all.iter().find(|n| {
            n.get_attribute("data-rid").is_some()
                && n.get_attribute("class")
                    .unwrap_or_default()
                    .contains("entry")
                && label_of(n).contains("ProbeItem")
        });
        if let Some(item) = item {
            click(&item.get_attribute("data-rid").unwrap());
            assert_eq!(
                (current_context_root(), current_dispatching_doc()),
                (4, Some(2))
            );
            assert_eq!(*seen.borrow(), [(9, Some(7))]);
            drop(scope);
            return;
        }
        for n in &all {
            if let Some(rid) = n.get_attribute("data-rid")
                && label_of(n).contains("File")
            {
                click(&rid);
            }
        }
    }
    panic!("control: the item never rendered");
}
