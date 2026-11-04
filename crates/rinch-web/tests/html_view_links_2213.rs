//! A link in an `HtmlView` is the app's to follow (#2213 review): the
//! browser never navigates the app's own tab to it, and `onlink` is told
//! which link was clicked, by a pointer or by `element.click()` (keyboard,
//! assistive technology).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test html_view_links_2213
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, InputCallback};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-2213";

thread_local! {
    static PREVIOUS: RefCell<Option<rinch_web::RootHandle>> = const { RefCell::new(None) };
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount an `HtmlView` holding one link (`#link`, around `#inner`), and
/// return what its `onlink` is given.
fn mount() -> Rc<RefCell<Vec<String>>> {
    rinch_web::__reset_activation_state();
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = document().query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 0px; top: 0px; width: 400px",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let links: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = links.clone();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let l = l.clone();
        HtmlView {
            html: r#"<p><a href="https://evil.example/p2213"><span style="display: inline-block; width: 200px; height: 20px">go</span></a></p>"#.into(),
            onlink: Some(InputCallback::new(move |href: String| l.borrow_mut().push(href))),
            ..Default::default()
        }
        .render(scope, &[])
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    links
}

fn link() -> web_sys::HtmlElement {
    document()
        .query_selector(&format!("[{HOST_ATTR}] a[href]"))
        .unwrap()
        .expect("the link")
        .dyn_into()
        .unwrap()
}

/// Whether the next `click` is cancelled by the time it reaches `window`
/// (after rinch's own listener on the document): what decides whether the
/// browser follows the link.
fn watch_click() -> Rc<RefCell<Vec<bool>>> {
    let seen: Rc<RefCell<Vec<bool>>> = Rc::default();
    let s = seen.clone();
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
        s.borrow_mut().push(e.default_prevented());
        // Never leave the test page, whatever the answer.
        e.prevent_default();
    });
    let opts = web_sys::AddEventListenerOptions::new();
    opts.set_once(true);
    web_sys::window()
        .unwrap()
        .add_event_listener_with_callback_and_add_event_listener_options(
            "click",
            closure.as_ref().unchecked_ref(),
            &opts,
        )
        .unwrap();
    closure.forget();
    seen
}

fn pointer(el: &web_sys::Element, name: &str) {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(if name == "pointerup" { 0 } else { 1 });
    init.set_client_x((r.x() + 4.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

#[wasm_bindgen_test]
fn a_pointer_click_on_a_link_does_not_navigate_and_calls_onlink() {
    let links = mount();
    let a = link();
    let inner = a.first_element_child().unwrap();
    rinch_web::__force_trusted_clicks(true);
    let prevented = watch_click();
    pointer(&inner, "pointerdown");
    pointer(&inner, "pointerup");
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_detail(1);
    let click = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    inner.dispatch_event(&click).unwrap();
    rinch_web::__force_trusted_clicks(false);
    assert_eq!(
        *prevented.borrow(),
        vec![true],
        "the browser does not follow it"
    );
    assert_eq!(
        *links.borrow(),
        vec!["https://evil.example/p2213".to_owned()]
    );
}

#[wasm_bindgen_test]
fn element_click_on_a_link_does_not_navigate_and_calls_onlink() {
    let links = mount();
    let prevented = watch_click();
    link().click();
    assert_eq!(
        *prevented.borrow(),
        vec![true],
        "the browser does not follow it"
    );
    assert_eq!(
        *links.borrow(),
        vec!["https://evil.example/p2213".to_owned()]
    );
}
