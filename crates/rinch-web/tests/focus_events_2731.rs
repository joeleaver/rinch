//! Browser-driven tests for issue #2731 on the web backend: an element's
//! `onfocus` / `onblur` / `onfocusin` / `onfocusout`, and `autofocus` in a
//! mounted tree.
//!
//! The browser fires the events itself, so these pin that rinch delivers them
//! in its order — `blur`, `focusout`, `focus`, `focusin` — with `focus` and
//! `blur` on the target alone and the `…in` / `…out` pair on every ancestor,
//! the same sequence the desktop runtime's tests pin
//! (`crates/rinch/src/app/focus_events_2731_tests.rs`).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test focus_events_2731
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-focus-events-2731-host";

type Log = Rc<RefCell<Vec<String>>>;

fn host() -> web_sys::Element {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(node) = stale.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
    if let Some(active) = document()
        .active_element()
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        active.blur().ok();
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

fn element(id: &str) -> web_sys::HtmlElement {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("#{id}"))
        .dyn_into()
        .unwrap()
}

fn mount(log: &Log) -> RootHandle {
    let host = host();
    let l = log.clone();
    rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let entry = |s: &'static str| {
                let l = l.clone();
                move || l.borrow_mut().push(s.to_string())
            };
            rsx! {
                div {
                    id: "fe-form",
                    onfocus: entry("focus:form"),
                    onblur: entry("blur:form"),
                    onfocusin: entry("focusin:form"),
                    onfocusout: entry("focusout:form"),
                    input {
                        id: "fe-a",
                        onfocus: entry("focus:a"),
                        onblur: entry("blur:a"),
                        onfocusin: entry("focusin:a"),
                        onfocusout: entry("focusout:a"),
                    }
                    input {
                        id: "fe-b",
                        onfocus: entry("focus:b"),
                        onblur: entry("blur:b"),
                        onfocusin: entry("focusin:b"),
                        onfocusout: entry("focusout:b"),
                    }
                }
            }
        },
    )
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// `focus()` fires `focus` on the element and `focusin` up the tree; moving
/// on fires the old element's `blur` / `focusout` first. `focus` and `blur`
/// do not reach the form.
#[wasm_bindgen_test]
fn focus_and_blur_fire_in_the_browsers_order() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let root = mount(&log);
    take(&log);
    element("fe-a").focus().unwrap();
    assert_eq!(take(&log), ["focus:a", "focusin:a", "focusin:form"]);
    element("fe-b").focus().unwrap();
    assert_eq!(
        take(&log),
        [
            "blur:a",
            "focusout:a",
            "focusout:form",
            "focus:b",
            "focusin:b",
            "focusin:form"
        ]
    );
    element("fe-b").blur().unwrap();
    assert_eq!(take(&log), ["blur:b", "focusout:b", "focusout:form"]);
    root.unmount();
}

/// `NodeHandle::focus()` is the browser's `focus()` on the web, and fires the
/// same events.
#[wasm_bindgen_test]
fn node_handle_focus_fires_them_too() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let field: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let host = host();
    let (l, out) = (log.clone(), field.clone());
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let l = l.clone();
            let node = rsx! {
                input { onfocus: move || l.borrow_mut().push("focus".into()) }
            };
            *out.borrow_mut() = Some(node.clone());
            node
        },
    );
    take(&log);
    field.borrow().as_ref().unwrap().focus();
    assert_eq!(take(&log), ["focus"]);
    root.unmount();
}

/// The first `autofocus` element of a mounted tree takes the focus, as in a
/// page Chrome loads — and as the desktop runtime does at mount.
#[wasm_bindgen_test]
async fn the_first_autofocus_element_is_focused_at_mount() {
    let host = host();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    input { id: "af-0" }
                    input { id: "af-1", autofocus: true }
                    input { id: "af-2", autofocus: true }
                }
            }
        },
    );
    // A browser runs its autofocus steps at its next rendering update.
    next_frame().await;
    next_frame().await;
    let active = document().active_element().map(|e| e.id());
    assert_eq!(active.as_deref(), Some("af-1"));
    root.unmount();
}

/// Resolve on the next animation frame.
async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(resolve.unchecked_ref())
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}
