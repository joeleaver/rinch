//! Browser-driven test for the root mount's ambient owner (#141 PR2).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The invariant under test: `mount_into` builds the tree with the root
//! `RenderScope` as the ambient owner, and pops that owner before returning.
//! Everything after the build — theme injection, event delegation, root
//! registration — is page-global setup with app lifetime, and must not be
//! attributed to the root.
//!
//! This lives in `rinch-web` rather than `rinch-core` because the guard is at
//! the web mount site, which `rinch-core` cannot reach.
#![cfg(target_arch = "wasm32")]

use rinch_core::element::ThemeProviderProps;
use rinch_core::{Owner, Signal, current_owner, dom::RenderScope};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// A detached host element, so each test mounts into a clean subtree.
fn host() -> web_sys::Element {
    let el = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&el).unwrap();
    el
}

#[wasm_bindgen_test]
fn the_web_root_build_runs_under_an_owner_that_does_not_outlive_it() {
    let seen: Rc<RefCell<Option<Owner>>> = Rc::new(RefCell::new(None));
    let log = seen.clone();

    let host = host();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            *log.borrow_mut() = current_owner();
            Signal::new(0i32);
            let div = scope.create_element("div");
            let text = scope.create_text("owned");
            div.append_child(&text);
            div
        },
    );

    assert!(
        current_owner().is_none(),
        "the mount guard must be popped before mount_into returns"
    );

    let owner = seen
        .borrow()
        .clone()
        .expect("the build must run under an ambient owner");
    assert!(owner.is_alive(), "the root scope outlives the mount");
    assert_eq!(
        owner.owned_counts().map(|c| c.signals),
        Some(1),
        "a signal created by the root build belongs to the root scope"
    );

    // Page-global setup that runs after the build is deliberately unowned.
    let after = owner.owned_counts().expect("root scope is live");
    Signal::new(0i32);
    assert_eq!(
        owner.owned_counts(),
        Some(after),
        "allocations after the build must not be attributed to the root"
    );

    root.unmount();
    host.remove();
}
