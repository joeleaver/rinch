//! Browser-driven tests for the page scroll lock (`lock_scroll`, #474).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! Web cannot gate the browser's own wheel, so its half of the prop is
//! `overflow: hidden` on the real `<html>`. Three things have to be true and
//! none of them is visible from the desktop fixtures:
//!
//! - it is the **`<html>` element**, not `WebDocument::body()` — that is
//!   `<div id="rinch-body">`, a descendant of the real `<body>`, and hiding its
//!   overflow does not stop the page scrolling;
//! - the previous inline `overflow` is **restored**, not blanked. A page that
//!   set one itself must get its own value back;
//! - the lock is **counted and page-global**. One `<html>` serves however many
//!   island roots share the page, so the count cannot live on a `WebDocument`.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// `<html>`'s inline style, which is what the lock writes to.
fn html_style() -> web_sys::CssStyleDeclaration {
    let html: web_sys::HtmlElement = document()
        .document_element()
        .expect("every document has a root element")
        .dyn_into()
        .expect("<html> is an HtmlElement");
    html.style()
}

fn html_overflow() -> String {
    html_style()
        .get_property_value("overflow")
        .unwrap_or_default()
}

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own teardown).
const HOST_MARKER: &str = "data-scroll-lock-test-host";

fn fresh_host() -> web_sys::Element {
    rinch_web::__reset_scroll_lock();
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(node) = stale.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

// ── 1. The primitive ─────────────────────────────────────────────────────────

/// A lock hides `<html>`'s overflow and an unlock takes the declaration away
/// again — **away**, not to the empty string: an inline `overflow: ` would still
/// be an inline declaration, and the point of restoring is that a stylesheet's
/// own value applies again.
#[wasm_bindgen_test]
fn a_lock_hides_the_root_elements_overflow_and_an_unlock_removes_it() {
    let host = fresh_host();
    let captured: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let captured_in = captured.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let el = scope.create_element("div");
            *captured_in.borrow_mut() = Some(el.clone());
            el
        },
    );
    let node = captured.borrow().clone().expect("mounted once");

    assert_eq!(
        html_overflow(),
        "",
        "precondition: nothing has locked the page yet"
    );

    node.set_scroll_locked(true);
    assert_eq!(html_overflow(), "hidden", "the page is locked");

    node.set_scroll_locked(false);
    assert_eq!(
        html_overflow(),
        "",
        "and the declaration is gone, not emptied"
    );
    assert!(
        !html_style().css_text().contains("overflow"),
        "an inline `overflow: ` would still shadow the stylesheet: {}",
        html_style().css_text()
    );

    rinch_web::__reset_scroll_lock();
    root.unmount();
    host.remove();
}

/// **The lock is counted.** Two overlays hold it; the first release leaves the
/// page locked; the second frees it.
///
/// A `bool` passes the test above and unlocks the page out from under an open
/// dialog here.
#[wasm_bindgen_test]
fn two_locks_need_two_unlocks() {
    let host = fresh_host();
    let captured: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let captured_in = captured.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let el = scope.create_element("div");
            *captured_in.borrow_mut() = Some(el.clone());
            el
        },
    );
    let node = captured.borrow().clone().expect("mounted once");

    node.set_scroll_locked(true);
    node.set_scroll_locked(true);
    assert_eq!(rinch_web::__scroll_lock_depth(), 2);

    node.set_scroll_locked(false);
    assert_eq!(
        html_overflow(),
        "hidden",
        "one of the two released; the other still holds the page"
    );

    node.set_scroll_locked(false);
    assert_eq!(html_overflow(), "", "both released");
    assert_eq!(rinch_web::__scroll_lock_depth(), 0);

    rinch_web::__reset_scroll_lock();
    root.unmount();
    host.remove();
}

/// A page that declared its own inline `overflow` gets **that value** back, not
/// an empty one.
///
/// The fixture deliberately uses `scroll` rather than `visible`: `visible` is
/// the initial value, so restoring to nothing and restoring correctly would
/// agree, and the mutant would live.
#[wasm_bindgen_test]
fn an_existing_inline_overflow_is_restored_rather_than_blanked() {
    let host = fresh_host();
    let captured: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let captured_in = captured.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let el = scope.create_element("div");
            *captured_in.borrow_mut() = Some(el.clone());
            el
        },
    );
    let node = captured.borrow().clone().expect("mounted once");

    html_style().set_property("overflow", "scroll").unwrap();

    node.set_scroll_locked(true);
    assert_eq!(html_overflow(), "hidden");

    node.set_scroll_locked(false);
    assert_eq!(
        html_overflow(),
        "scroll",
        "the page's own declaration must come back"
    );

    rinch_web::__reset_scroll_lock();
    root.unmount();
    host.remove();
}

// ── 2. Through the component ─────────────────────────────────────────────────

/// `Modal { lock_scroll }` reaches all of the above on web, opening and closing.
#[wasm_bindgen_test]
fn an_open_lock_scroll_modal_locks_the_page_and_closing_it_unlocks() {
    let host = fresh_host();
    let open = Signal::new(false);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            rinch::components::Modal {
                opened_fn: Some(Rc::new(move || open.get())),
                lock_scroll: true,
                ..Default::default()
            }
            .render(scope, &[])
        },
    );

    assert_eq!(
        html_overflow(),
        "",
        "a closed modal locks nothing, though it is mounted"
    );

    open.set(true);
    assert_eq!(html_overflow(), "hidden", "opening takes the lock");

    open.set(false);
    assert_eq!(html_overflow(), "", "closing releases it");

    rinch_web::__reset_scroll_lock();
    root.unmount();
    host.remove();
}

/// **Unmounting while open releases the lock.**
///
/// The component's effect is the only thing that would ever unlock, and an
/// unmounted scope's effect never runs again — so without the `on_cleanup` the
/// page is unscrollable for the rest of the session with nothing on screen to
/// explain it.
#[wasm_bindgen_test]
fn unmounting_an_open_modal_releases_the_lock() {
    let host = fresh_host();
    let open = Signal::new(true);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            rinch::components::Modal {
                opened_fn: Some(Rc::new(move || open.get())),
                lock_scroll: true,
                ..Default::default()
            }
            .render(scope, &[])
        },
    );
    assert_eq!(html_overflow(), "hidden", "precondition: open and locked");

    root.unmount();

    assert_eq!(
        html_overflow(),
        "",
        "the lock went with the component, still open"
    );
    assert_eq!(rinch_web::__scroll_lock_depth(), 0);

    rinch_web::__reset_scroll_lock();
    host.remove();
}
