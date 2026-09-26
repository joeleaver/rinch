//! A right or middle press runs no `data-rid` on the web either (#1093; the
//! review of PR #1110 measured it running one in Chrome 153), and a
//! `data-backdrop` one runs for every button.
#![cfg(target_arch = "wasm32")]
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn press(el: &web_sys::Element, button: i16, buttons: u16) {
    let r = el.get_bounding_client_rect();
    for (ty, bs) in [("pointerdown", buttons), ("pointerup", 0)] {
        let init = web_sys::PointerEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_pointer_id(1);
        init.set_is_primary(true);
        init.set_pointer_type("mouse");
        init.set_button(button);
        init.set_buttons(bs);
        init.set_client_x((r.x() + r.width() / 2.0) as i32);
        init.set_client_y((r.y() + r.height() / 2.0) as i32);
        let ev = web_sys::PointerEvent::new_with_event_init_dict(ty, &init).unwrap();
        el.dispatch_event(&ev).unwrap();
    }
}

#[wasm_bindgen_test]
fn a_right_or_middle_press_runs_no_data_rid() {
    let count = Rc::new(Cell::new(0u32));
    let c = count.clone();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let b = scope.create_element("button");
            b.set_attribute("id", "r1110");
            b.set_attribute("style", "width: 100px; height: 30px");
            let c = c.clone();
            let id = scope.register_handler(move || c.set(c.get() + 1));
            b.set_attribute("data-rid", &id.0.to_string());
            b
        },
    );
    let el: web_sys::Element = document()
        .get_element_by_id("r1110")
        .unwrap()
        .dyn_into()
        .unwrap();
    press(&el, 0, 1);
    let left = count.get();
    press(&el, 2, 2);
    let right = count.get() - left;
    press(&el, 1, 4);
    let middle = count.get() - left - right;
    root.unmount();
    host.remove();
    assert_eq!(left, 1, "positive control: a left press clicks");
    assert_eq!(
        (right, middle),
        (0, 0),
        "web clicked a data-rid on right={right} middle={middle}"
    );
}

/// A backdrop (`data-backdrop`) is the one `data-rid` every button runs: an
/// outside-press dismissal is a `mousedown` of any button.
#[wasm_bindgen_test]
fn a_backdrop_runs_for_every_button() {
    let count = Rc::new(Cell::new(0u32));
    let c = count.clone();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let b = scope.create_element("div");
            b.set_attribute("id", "b1093");
            b.set_attribute("style", "width: 100px; height: 30px");
            b.set_attribute(rinch_core::events::BACKDROP_ATTRIBUTE, "");
            let c = c.clone();
            let id = scope.register_handler(move || c.set(c.get() + 1));
            b.set_attribute("data-rid", &id.0.to_string());
            b
        },
    );
    let el: web_sys::Element = document()
        .get_element_by_id("b1093")
        .unwrap()
        .dyn_into()
        .unwrap();
    press(&el, 0, 1);
    press(&el, 2, 2);
    press(&el, 1, 4);
    let n = count.get();
    root.unmount();
    host.remove();
    assert_eq!(n, 3, "left, right and middle each dismiss");
}
