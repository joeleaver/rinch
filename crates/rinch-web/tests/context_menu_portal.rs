//! Browser-driven tests for `ContextMenu`'s portal, in a real DOM.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The menu portals itself to `document.body` so `position: fixed` resolves
//! against the viewport. That puts it outside the subtree it was built into,
//! which is exactly why nothing used to reclaim it: when a reactive block
//! rebuilt the row the menu belonged to, the row's scope was disposed — its
//! handlers deregistered, its signals freed — and the portal stayed on `body`,
//! markup and all. A person met that by right-clicking while the tree behind
//! them was refreshing: an open menu that neither acted nor closed.
//!
//! `rinch-components` pins the ownership rule against a headless document; these
//! pin what a browser actually ends up holding, which is the thing the person
//! sees.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal, show_dom};
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-portal-test-host";

/// How many context-menu portals the page is currently holding.
fn portals() -> u32 {
    document()
        .query_selector_all(".rinch-context-menu__portal")
        .unwrap()
        .length()
}

/// Purge whatever a failed test left behind: its host, and any portal that was
/// orphaned before the fixture could tear it down.
fn purge() {
    for selector in [
        &format!("[{HOST_MARKER}]") as &str,
        ".rinch-context-menu__portal",
    ] {
        if let Ok(stale) = document().query_selector_all(selector) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
    }
}

fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
) -> (RootHandle, web_sys::Element) {
    purge();
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    (root, host)
}

/// A `ContextMenu` with an empty target and one empty dropdown.
fn context_menu(scope: &mut RenderScope) -> NodeHandle {
    let target = scope.create_element("div");
    let items = scope.create_element("div");
    rinch::components::ContextMenu::default().render(scope, &[target, items])
}

/// The defect: a reactive block rebuilding the row the menu hangs off used to
/// leave the old portal on `body`, so the page accumulated one dead menu per
/// rebuild.
#[wasm_bindgen_test]
fn a_rebuild_does_not_orphan_the_portal_on_body() {
    let generation = Signal::new(0u32);
    let (root, host) = mount(move |scope: &mut RenderScope| {
        let container = scope.create_element("div");
        // Rendered when the generation is even and torn down when it is odd, so
        // each flip is one build and one disposal — a reactive rebuild, as far
        // as ownership is concerned.
        show_dom(
            scope,
            &container,
            move || generation.get().is_multiple_of(2),
            context_menu,
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        container
    });

    assert_eq!(portals(), 1, "the menu portals itself to body");

    for round in 1..=6u32 {
        generation.set(round);
        let expected = u32::from(round.is_multiple_of(2));
        assert_eq!(
            portals(),
            expected,
            "after {round} rebuild(s) the page must hold {expected} portal(s), \
             not one per build"
        );
    }

    root.unmount();
    host.remove();
    assert_eq!(portals(), 0, "and unmounting the root takes the last one");
}

/// The same rule at the root: unmounting an island takes its portal with it,
/// rather than leaving a dead overlay behind in someone else's page.
///
/// The host is removed **after** the assertion, deliberately. An island's
/// `body_handle()` is its host element, so removing the host would take the
/// portal down whatever the component did — the test would pass on the defect.
#[wasm_bindgen_test]
fn unmounting_a_root_removes_its_portal() {
    let (root, host) = mount(|scope: &mut RenderScope| {
        let container = scope.create_element("div");
        container.append_child(&context_menu(scope));
        container
    });
    assert_eq!(portals(), 1);

    root.unmount();
    assert_eq!(
        portals(),
        0,
        "the portal must go with the root, not with the host element"
    );
    host.remove();
}
