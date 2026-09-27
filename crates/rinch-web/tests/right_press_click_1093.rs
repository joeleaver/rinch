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

fn contextmenu(el: &web_sys::Element) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(2);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// A `Popover` open inside an element whose `data-oncontextmenu` is `ctx`:
/// `(host, root, closes, contextmenus)`.
fn popover_in_context_target(
    live_ctx: bool,
) -> (
    web_sys::Element,
    rinch_web::RootHandle,
    Rc<Cell<u32>>,
    Rc<Cell<u32>>,
) {
    use rinch::components::Popover;
    use rinch_core::Component;
    let closes = Rc::new(Cell::new(0u32));
    let menus = Rc::new(Cell::new(0u32));
    let (cl, mn) = (closes.clone(), menus.clone());
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let wrap = scope.create_element("div");
            let ctx = scope.register_handler(move || mn.set(mn.get() + 1));
            if !live_ctx {
                rinch_core::events::unregister_handler(ctx);
            }
            wrap.set_attribute("data-oncontextmenu", &ctx.0.to_string());
            let pop = Popover {
                opened_fn: Some(Rc::new(|| true)),
                onclose: Some(rinch_core::Callback::new(move || cl.set(cl.get() + 1))),
                ..Default::default()
            }
            .render(scope, &[]);
            wrap.append_child(&pop);
            wrap
        },
    );
    (host, root, closes, menus)
}

fn popover_backdrop() -> web_sys::Element {
    document()
        .query_selector(".rinch-popover__backdrop")
        .unwrap()
        .expect("the popover mounted its backdrop")
}

/// A live `data-oncontextmenu` above a backdrop takes the right press: the
/// contextmenu handler runs and the backdrop does not dismiss (as on desktop,
/// where the contextmenu claim is offered first). A left press there dismisses.
#[wasm_bindgen_test]
fn a_contextmenu_handler_above_a_backdrop_takes_the_right_press() {
    let (host, root, closes, menus) = popover_in_context_target(true);
    let backdrop = popover_backdrop();
    press(&backdrop, 2, 2);
    contextmenu(&backdrop);
    let after_right = (closes.get(), menus.get());
    press(&backdrop, 0, 1);
    let after_left = closes.get();
    root.unmount();
    host.remove();
    assert_eq!(
        after_right,
        (0, 1),
        "(closes, contextmenus) after the right press"
    );
    assert_eq!(after_left, 1, "positive control: a left press dismisses");
}

/// A **stale** `data-oncontextmenu` (its handler freed, #141) is no handler, so
/// it does not hold a right press back from the backdrop — desktop's rule.
#[wasm_bindgen_test]
fn a_stale_contextmenu_attribute_does_not_block_a_backdrop() {
    let (host, root, closes, _) = popover_in_context_target(false);
    press(&popover_backdrop(), 2, 2);
    let n = closes.get();
    root.unmount();
    host.remove();
    assert_eq!(n, 1, "the right press on the backdrop dismissed");
}

/// `data-backdrop="false"` is rinch's `data-` escape: not a backdrop. `"0"` is.
#[wasm_bindgen_test]
fn data_backdrop_false_is_not_a_backdrop() {
    for (value, expected) in [("false", 0u32), ("FALSE", 0), ("0", 1)] {
        let count = Rc::new(Cell::new(0u32));
        let c = count.clone();
        let host = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let b = scope.create_element("div");
                b.set_attribute("id", "f1093");
                b.set_attribute("style", "width: 100px; height: 30px");
                b.set_attribute(rinch_core::events::BACKDROP_ATTRIBUTE, value);
                let c = c.clone();
                let id = scope.register_handler(move || c.set(c.get() + 1));
                b.set_attribute("data-rid", &id.0.to_string());
                b
            },
        );
        let el = document().get_element_by_id("f1093").unwrap();
        press(&el, 2, 2);
        let n = count.get();
        root.unmount();
        host.remove();
        assert_eq!(n, expected, "data-backdrop={value:?}");
    }
}

/// A draggable's click is deferred to the release; a right press and release
/// that never became a drag is not a click either. Left is the control.
#[wasm_bindgen_test]
fn a_right_press_on_a_draggable_does_not_click_on_release() {
    let count = Rc::new(Cell::new(0u32));
    let c = count.clone();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let b = scope.create_element("div");
            b.set_attribute("id", "d1093");
            b.set_attribute("draggable", "true");
            b.set_attribute("style", "width: 100px; height: 30px");
            let c = c.clone();
            let id = scope.register_handler(move || c.set(c.get() + 1));
            b.set_attribute("data-rid", &id.0.to_string());
            b
        },
    );
    let el = document().get_element_by_id("d1093").unwrap();
    press(&el, 2, 2);
    let right = count.get();
    press(&el, 0, 1);
    let left = count.get() - right;
    root.unmount();
    host.remove();
    assert_eq!((right, left), (0, 1), "(right, left) clicks on a draggable");
}
