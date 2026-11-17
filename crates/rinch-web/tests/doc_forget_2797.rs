//! An unmounted root's document is swept from the registries keyed by
//! document: what its code registered there, and what its cleanups registered
//! as it was unmounted, goes with it; another root's stays (issue #2797).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test doc_forget_2797
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::element::ThemeProviderProps;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// What document `doc` holds in the registries this test fills.
fn held(doc: u64) -> Vec<(&'static str, usize)> {
    let mut held: Vec<_> = rinch_core::doc_entries(doc)
        .into_iter()
        .filter(|(name, n)| ["STACK", "PENDING_RELEASES"].contains(name) && *n > 0)
        .collect();
    held.sort_unstable();
    held
}

/// A root whose code registers a dismiss handler (its handle kept for the
/// page's life) and a parked focus release for its document, and does the
/// same again in its `on_cleanup`. Answers the root, its host and its
/// document's key.
fn root() -> (rinch_web::RootHandle, web_sys::Element, u64) {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let doc = Rc::new(Cell::new(0));
    let seen = doc.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let el = scope.create_element("div");
        let key = el.doc_key();
        seen.set(key);
        let register = move || {
            std::mem::forget(rinch_core::events::push_dismiss_handler(key, || false));
            rinch_core::events::post_release_request(key, 1);
        };
        register();
        scope.on_cleanup(register);
        el
    });
    (root, host, doc.get())
}

#[wasm_bindgen_test]
fn an_unmounted_roots_entries_are_forgotten_and_anothers_stay() {
    let (gone, gone_host, gone_doc) = root();
    let (kept, kept_host, kept_doc) = root();
    assert_ne!(gone_doc, kept_doc, "control: a document per root");
    assert_eq!(
        held(gone_doc),
        [("PENDING_RELEASES", 1), ("STACK", 1)],
        "control: registered"
    );
    let before = held(kept_doc);
    gone.unmount();
    assert_eq!(
        held(gone_doc),
        [],
        "the unmounted root's, its cleanup's included"
    );
    assert_eq!(held(kept_doc), before, "the other root's");
    kept.unmount();
    assert_eq!(held(kept_doc), []);
    gone_host.remove();
    kept_host.remove();
}
