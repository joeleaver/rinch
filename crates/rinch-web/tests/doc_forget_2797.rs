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

const STACK: &str = "rinch_core::events::dismiss::STACK";
const RELEASES: &str = "rinch_core::events::selection::PENDING_RELEASES";

/// What document `doc` holds in the registries this test fills.
fn held(doc: u64) -> Vec<(&'static str, usize)> {
    let mut held: Vec<_> = rinch_core::doc_entries(doc)
        .into_iter()
        .filter(|(name, n)| [STACK, RELEASES].contains(name) && *n > 0)
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
        [(STACK, 1), (RELEASES, 1)],
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

/// An app key a root's `on_cleanup` registers as the root is unmounted: the
/// disposal runs as the root's document's code, so the key is the
/// document's and the sweep takes it. Counted over every listener on the
/// thread, whatever its document.
#[wasm_bindgen_test]
fn an_app_key_an_unmounted_roots_cleanup_registers() {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let before = rinch_core::events::app_key_listener_count();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), |scope| {
        scope.on_cleanup(|| {
            let _listener = rinch_core::on_app_key(|_| false);
        });
        scope.create_element("div")
    });
    root.unmount();
    let after = rinch_core::events::app_key_listener_count();
    host.remove();
    assert_eq!(after, before, "the cleanup's app key goes with the unmount");
}

/// A host element appended to the page's body.
fn host() -> web_sys::Element {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

/// A root that stays mounted: its ordinary code registers an app key and a
/// paste interceptor, both its document's (a root's code runs as its
/// document's, #2811), owned by its scope. Answers the root and its
/// document's key.
fn staying_root(host: &web_sys::Element) -> (rinch_web::RootHandle, u64) {
    let doc = Rc::new(Cell::new(0));
    let seen = doc.clone();
    let root = rinch_web::mount_into(host, ThemeProviderProps::default(), move |scope| {
        let _listener = rinch_core::on_app_key(|_| false);
        rinch_core::events::set_paste_interceptor(|_| true);
        let el = scope.create_element("div");
        seen.set(el.doc_key());
        el
    });
    (root, doc.get())
}

/// Whether document `doc` is served a paste interceptor.
fn has_paste_interceptor_in(doc: u64) -> bool {
    let _doc = rinch_core::push_dispatching_doc(doc);
    rinch_core::events::has_paste_interceptor()
}

/// Another root's unmount leaves a staying root's app key and paste
/// interceptor, marked disposal or not: the sweep takes the unmounted root's
/// document only.
#[wasm_bindgen_test]
fn a_staying_roots_registrations_survive_anothers_unmount() {
    let (gone_host, kept_host) = (host(), host());
    let keys_before = rinch_core::events::app_key_listener_count();
    let (kept, kept_doc) = staying_root(&kept_host);
    let gone = rinch_web::mount_into(&gone_host, ThemeProviderProps::default(), |scope| {
        scope.create_element("div")
    });
    gone.unmount();
    let keys_after = rinch_core::events::app_key_listener_count();
    let interceptor_after = has_paste_interceptor_in(kept_doc);
    kept.unmount();
    gone_host.remove();
    kept_host.remove();
    assert_eq!(keys_after, keys_before + 1, "the staying root's app key");
    assert!(interceptor_after, "the staying root's paste interceptor");
}

/// A clear in an unmounting root's cleanup clears that root's document's
/// slot, not another root's install (#2811; before it, every entry on the web
/// was the page's one ownerless entry, and the clear took the staying root's
/// interceptor too).
#[wasm_bindgen_test]
fn a_clear_in_an_unmounting_roots_cleanup_leaves_anothers_slot() {
    let (gone_host, kept_host) = (host(), host());
    let keys_before = rinch_core::events::app_key_listener_count();
    let (kept, kept_doc) = staying_root(&kept_host);
    let gone = rinch_web::mount_into(&gone_host, ThemeProviderProps::default(), |scope| {
        scope.on_cleanup(rinch_core::events::clear_paste_interceptor);
        scope.create_element("div")
    });
    assert!(has_paste_interceptor_in(kept_doc), "control: installed");
    gone.unmount();
    let keys_after = rinch_core::events::app_key_listener_count();
    let interceptor_after = has_paste_interceptor_in(kept_doc);
    kept.unmount();
    gone_host.remove();
    kept_host.remove();
    assert_eq!(keys_after, keys_before + 1, "the staying root's app key");
    assert!(interceptor_after, "the staying root's paste interceptor");
}
