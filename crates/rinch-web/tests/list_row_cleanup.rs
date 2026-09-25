//! Browser-driven test for issue #356: a list row's cleanup runs while its node
//! is still the live, mounted row.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test list_row_cleanup
//! ```
//!
//! `for_each_dom_typed` and `virtual_list` used to `discard` a departing row
//! during the pass and dispose its scope at the end of it. On this backend a
//! discard **retires** the id — it leaves `NODE_REGISTRY` and
//! `WebDocument::nodes` — so the row's cleanup read `None` off its own node and
//! found no element in the page, where desktop, whose slab keeps the node, read
//! the real value. The cleanup here asks both questions: the handle's
//! attribute, and whether the browser still has the element in the document.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::{Signal, on_cleanup};
use rinch_core::{for_each_dom_typed, virtual_list};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn host(tag: &str) -> web_sys::Element {
    let el = browser_document().create_element("div").unwrap();
    el.set_attribute("data-test-host-356", tag).unwrap();
    browser_document()
        .body()
        .unwrap()
        .append_child(&el)
        .unwrap();
    el
}

/// `(the handle's data-name, the element still in the page)`, per cleanup.
type Sight = Rc<RefCell<Vec<(Option<String>, bool)>>>;

/// A row that stamps a page-unique `data-row-356` and records, from its
/// cleanup, what it can still see of itself.
fn row(tag: &'static str, name: &str, s: &mut RenderScope, sight: &Sight) -> NodeHandle {
    let node = s.create_element("div");
    let value = format!("{tag}-{name}");
    node.set_attribute("data-row-356", &value);
    let (me, sight) = (node.clone(), sight.clone());
    on_cleanup(move || {
        let in_page = browser_document()
            .query_selector(&format!("[data-row-356='{value}']"))
            .ok()
            .flatten()
            .is_some_and(|el| el.is_connected());
        sight
            .borrow_mut()
            .push((me.get_attribute("data-row-356"), in_page));
    });
    node
}

fn in_page(value: &str) -> bool {
    browser_document()
        .query_selector(&format!("[data-row-356='{value}']"))
        .ok()
        .flatten()
        .is_some()
}

#[wasm_bindgen_test]
fn a_removed_for_rows_cleanup_sees_its_own_live_node() {
    let items = Signal::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    let sight: Sight = Rc::new(RefCell::new(Vec::new()));
    let log = sight.clone();
    let _root = rinch_web::mount_into(&host("for"), ThemeProviderProps::default(), move |s| {
        let list = s.create_element("div");
        for_each_dom_typed(
            s,
            &list,
            move || items.get(),
            |n: &String| n.clone(),
            move |n: String, s: &mut RenderScope| row("for", &n, s, &log),
        );
        list
    });

    // Positive control: the rows are real elements in the page, so the
    // cleanup's page query below can see one.
    assert!(in_page("for-b"), "precondition: row b is mounted in the page");

    items.set(vec!["a".to_string(), "c".to_string()]);

    assert_eq!(
        *sight.borrow(),
        vec![(Some("for-b".to_string()), true)],
        "#356: the cleanup must run before its row is discarded"
    );
    assert!(!in_page("for-b"), "and the row is gone afterwards");
    assert!(in_page("for-a") && in_page("for-c"), "its siblings stay");
}

#[wasm_bindgen_test]
fn a_departing_virtual_rows_cleanup_sees_its_own_live_node() {
    let items = Signal::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    let sight: Sight = Rc::new(RefCell::new(Vec::new()));
    let log = sight.clone();
    let _root = rinch_web::mount_into(&host("vlist"), ThemeProviderProps::default(), move |s| {
        virtual_list(
            s,
            20.0,
            move || items.get(),
            |n: &String| n.clone(),
            1,
            move |n: String, s: &mut RenderScope| row("vlist", &n, s, &log),
        )
    });

    assert!(
        in_page("vlist-b"),
        "precondition: row b is mounted in the page"
    );

    items.set(vec!["a".to_string(), "c".to_string()]);

    assert_eq!(
        *sight.borrow(),
        vec![(Some("vlist-b".to_string()), true)],
        "#356: the cleanup must run before its row is discarded"
    );
    assert!(!in_page("vlist-b"), "and the row is gone afterwards");
}
