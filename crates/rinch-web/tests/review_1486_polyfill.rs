//! Review of PR #1486: a page that polyfills `Element.prototype.moveBefore`
//! with `insertBefore` (what a polyfill for Safari does). rinch-web's feature
//! test (`typeof … === 'function'`, taken once) then believes a move keeps the
//! focus and skips #1478's release, and the polyfill's `insertBefore` fires
//! `blur` / `focusout` inside the call, under `borrow_mut`.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

#[wasm_bindgen_test]
fn a_polyfilled_move_before_runs_focus_listeners_under_the_borrow() {
    js_sys::eval(
        "Element.prototype.moveBefore = function (n, r) { return this.insertBefore(n, r); };",
    )
    .unwrap();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let left = Rc::new(Cell::new(0u32));
    type Slots = (
        NodeHandle,
        NodeHandle,
        NodeHandle,
        rinch_core::DismissHandle,
    );
    let slots: Rc<RefCell<Option<Slots>>> = Default::default();
    let (l, sl) = (left.clone(), slots.clone());
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let wrap = s.create_element("div");
        let trigger = s.create_element("button");
        trigger.set_attribute("id", "pf-trigger");
        wrap.append_child(&trigger);
        page.append_child(&wrap);
        let other = s.create_element("button");
        other.set_attribute("id", "pf-other");
        page.append_child(&other);
        let l = l.clone();
        // The entry's leave callback touches the document, as `Select`'s does.
        let o2 = other.clone();
        let keep = rinch_core::push_key_handler(
            &trigger,
            |_| false,
            move || {
                l.set(l.get() + 1);
                o2.set_attribute("data-left", "1");
            },
        );
        *sl.borrow_mut() = Some((page.clone(), wrap, other, keep));
        page
    });
    let (page, wrap, other, _keep) = slots.borrow_mut().take().unwrap();
    let trigger: web_sys::HtmlElement = document()
        .get_element_by_id("pf-trigger")
        .unwrap()
        .dyn_into()
        .unwrap();
    trigger.focus().unwrap();
    page.append_child(&wrap); // panics "already borrowed" inside the listener on the PR head
    assert_eq!(left.get(), 1, "the entry heard the focus leave");
    other.set_attribute("data-after", "1");
    assert_eq!(
        document()
            .get_element_by_id("pf-other")
            .unwrap()
            .get_attribute("data-left")
            .as_deref(),
        Some("1"),
        "and its write reached the document"
    );
    root.unmount();
    host.remove();
}
