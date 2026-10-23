//! #1796: `NodeHandle::query_selector{,_all}` on the web, and the browser
//! oracle for the desktop table.
//!
//! Every row of `rinch-dom/tests/support/query_selector_table.rs` (shared with
//! the desktop test `query_selector_1796_tests.rs`) is first checked against
//! Chrome's own `Element.querySelectorAll` on the same markup — the row is the
//! browser's answer — and then through rinch-web's `NodeHandle`, which since
//! #1796 asks the handle's element (it asked `document`, whatever the handle).
//! An invalid selector is a `SyntaxError` in the browser and nothing in rinch.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test query_selector_1796
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[path = "../../rinch-dom/tests/support/query_selector_table.rs"]
mod table;

const HOST: &str = "data-test-host-1796";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount `<div id="host">MARKUP</div>` through `mount_into`; the host's handle.
fn mount() -> (rinch_web::RootHandle, NodeHandle, web_sys::Element) {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i) {
                n.dyn_into::<web_sys::Element>().unwrap().remove();
            }
        }
    }
    let mount_el = document().create_element("div").unwrap();
    mount_el.set_attribute(HOST, "").unwrap();
    document().body().unwrap().append_child(&mount_el).unwrap();
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let s2 = slot.clone();
    let root = rinch_web::mount_into(
        &mount_el,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let host = s.create_element("div");
            host.set_attribute("id", "host");
            host.set_inner_html(table::MARKUP);
            *s2.borrow_mut() = Some(host.clone());
            host
        },
    );
    let host = slot.borrow().clone().expect("the host rendered");
    (root, host, mount_el)
}

fn ids(nodes: &[NodeHandle]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| n.get_attribute("id").unwrap_or_else(|| "?".into()))
        .collect()
}

/// The browser's own answer: `#scope.querySelectorAll(selector)`.
fn browser(scope: &str, selector: &str) -> Result<Vec<String>, ()> {
    let el = document()
        .get_element_by_id(scope)
        .expect("scope in the page");
    let list = el.query_selector_all(selector).map_err(|_| ())?;
    Ok((0..list.length())
        .map(|i| {
            list.item(i)
                .unwrap()
                .dyn_into::<web_sys::Element>()
                .unwrap()
                .id()
        })
        .collect())
}

#[wasm_bindgen_test]
fn the_table_is_the_browsers_answer() {
    let (root, _host, _el) = mount();
    let mut wrong = Vec::new();
    for &(scope, selector, want) in table::CASES {
        let got = browser(scope, selector);
        if got.as_deref() != Ok(&want.iter().map(|s| s.to_string()).collect::<Vec<_>>()[..]) {
            wrong.push(format!(
                "{selector:?} in #{scope}: browser {got:?}, table {want:?}"
            ));
        }
    }
    for &sel in table::INVALID {
        if browser("host", sel).is_ok() {
            wrong.push(format!("{sel:?}: the browser accepts it"));
        }
    }
    root.unmount();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[wasm_bindgen_test]
fn a_node_handle_answers_every_row_as_the_browser() {
    let (root, host, _el) = mount();
    // Positive control: the mount is rinch's, and a handle reaches the DOM.
    assert!(document().get_element_by_id("s1").is_some());
    let mut wrong = Vec::new();
    for &(scope, selector, want) in table::CASES {
        let scope_h = if scope == "host" {
            host.clone()
        } else {
            host.query_selector(&format!("#{scope}"))
                .unwrap_or_else(|| panic!("#{scope} under the host"))
        };
        let all = ids(&scope_h.query_selector_all(selector));
        let first = scope_h
            .query_selector(selector)
            .and_then(|n| n.get_attribute("id"));
        if all != want || first.as_deref() != want.first().copied() {
            wrong.push(format!(
                "{selector:?} in #{scope}: all {all:?} first {first:?}, want {want:?}"
            ));
        }
    }
    for &sel in table::INVALID {
        if host.query_selector(sel).is_some() || !host.query_selector_all(sel).is_empty() {
            wrong.push(format!("{sel:?}: answered something"));
        }
    }
    root.unmount();
    assert!(
        wrong.is_empty(),
        "{} rows wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
