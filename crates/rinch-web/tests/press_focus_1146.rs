//! A press on a `data-rid` element focuses what the browser would have
//! focused (issue #1146).
//!
//! rinch dispatches a `data-rid` click on the `pointerdown` and then
//! `preventDefault`s it (text selection during a slider drag, `<label>`
//! re-dispatch). In a browser that suppresses the compatibility `mousedown`,
//! and focus is `mousedown`'s default action, so before #1146 a press never
//! moved focus: a `Button` took no focus and the field that had it was not
//! blurred, and a text field inside a clickable (`ColorInput`'s, inside its
//! input group) was never focused at all — typing reached nothing. Measured in
//! Chrome 155 with trusted CDP input.
//!
//! Now the press is resolved the way a browser resolves it, as desktop's
//! `claim_press_focus` does:
//!
//! - nearest focusable is a form control or editable content: the default is
//!   **not** prevented, so the browser focuses it and places the caret where
//!   the press landed;
//! - nearest focusable is anything else (a `<button>`, a `tabindex` row, an
//!   `<a href>`): rinch focuses it, **before** the handler runs, so the field
//!   that had the focus is blurred first and a focus move the handler makes
//!   itself wins;
//! - `data-nofocus` on the chain, or a default already prevented: no focus
//!   change, as before;
//! - nothing focusable on the chain: no focus change, as before.
//!
//! A synthetic `pointerdown` makes the browser focus nothing whether or not it
//! is prevented, so every focus asserted here is one rinch made itself, and a
//! form-control press is pinned at its mechanism (`defaultPrevented`). The
//! positive control of each fixture is that the handler ran.
//!
//! **Mount through `rinch_web::mount_into`** — without the owner `mount_tree`
//! pushes, component effects are disposed at once.
#![cfg(target_arch = "wasm32")]

use rinch::components::ColorInput;
use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-1146";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    log: Log,
    _blur: Closure<dyn FnMut()>,
}

/// What every fixture holds: a text field `#field` (focused first), a
/// `<button>` `#btn`, a `tabindex` row `#row` holding a non-focusable `#chev`,
/// a `<button>` `#outer` holding a `#inner` span, a non-focusable `#plain`, a
/// `data-nofocus` toolbar `#bar` holding `#guarded`, a `<button>` `#mover`
/// whose handler focuses `#other`, `#other` itself, a text field `#inner-field`
/// inside a clickable `#group`, and a `ColorInput`. Every handler logs its
/// element's id.
fn mount() -> Fixture {
    rinch_web::__reset_activation_state();
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i)
                && let Ok(el) = n.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute("style", "width: 300px; margin: 20px")
        .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let clickable = |scope: &mut RenderScope, tag: &str, id: &str| -> NodeHandle {
                let el = scope.create_element(tag);
                el.set_attribute("id", id);
                el.append_child(&scope.create_text(id));
                let sink = sink.clone();
                let name = id.to_string();
                let rid = scope.register_handler(move || sink.borrow_mut().push(name.clone()));
                el.set_attribute("data-rid", &rid.0.to_string());
                el
            };
            let field = scope.create_element("input");
            field.set_attribute("id", "field");
            root.append_child(&field);

            root.append_child(&clickable(scope, "button", "btn"));

            let row = scope.create_element("div");
            row.set_attribute("id", "row");
            row.set_attribute("tabindex", "0");
            row.append_child(&clickable(scope, "span", "chev"));
            root.append_child(&row);

            let outer = scope.create_element("button");
            outer.set_attribute("id", "outer");
            outer.append_child(&clickable(scope, "span", "inner"));
            root.append_child(&outer);

            root.append_child(&clickable(scope, "div", "plain"));

            let bar = scope.create_element("div");
            bar.set_attribute("data-nofocus", "");
            bar.append_child(&clickable(scope, "button", "guarded"));
            root.append_child(&bar);

            let other = scope.create_element("button");
            other.set_attribute("id", "other");
            other.append_child(&scope.create_text("other"));
            let mover = scope.create_element("button");
            mover.set_attribute("id", "mover");
            mover.append_child(&scope.create_text("mover"));
            {
                let other = other.clone();
                let sink = sink.clone();
                let rid = scope.register_handler(move || {
                    sink.borrow_mut().push("mover".into());
                    other.focus();
                });
                mover.set_attribute("data-rid", &rid.0.to_string());
            }
            root.append_child(&mover);
            root.append_child(&other);

            let group = clickable(scope, "div", "group");
            let inner_field = scope.create_element("input");
            inner_field.set_attribute("id", "inner-field");
            group.append_child(&inner_field);
            root.append_child(&group);

            let color = ColorInput {
                value: "#123456".to_string(),
                ..Default::default()
            }
            .render(scope, &[]);
            root.append_child(&color);
            root
        },
    );
    // The field's blur is logged too, so the order of a blur and a click is
    // visible.
    let blur_sink = log.clone();
    let blur = Closure::<dyn FnMut()>::new(move || blur_sink.borrow_mut().push("blur".into()));
    by_id("field")
        .add_event_listener_with_callback("blur", blur.as_ref().unchecked_ref())
        .unwrap();
    Fixture {
        root,
        host,
        log,
        _blur: blur,
    }
}

impl Fixture {
    fn focus_field(&self) {
        by_id("field").focus().unwrap();
        assert_eq!(
            active_id(),
            "field",
            "positive control: the field is focused"
        );
        self.log.borrow_mut().clear();
    }

    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn by_id(id: &str) -> web_sys::HtmlElement {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("no element #{id}"))
        .dyn_into()
        .unwrap()
}

fn active_id() -> String {
    document()
        .active_element()
        .map(|a| {
            if a.id().is_empty() {
                a.tag_name().to_lowercase()
            } else {
                a.id()
            }
        })
        .unwrap_or_default()
}

/// A primary mouse `pointerdown` (and its `pointerup`) at the element's centre.
fn press(el: &web_sys::Element) -> web_sys::PointerEvent {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(1);
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let down = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    el.dispatch_event(&down).unwrap();
    init.set_buttons(0);
    let up = web_sys::PointerEvent::new_with_event_init_dict("pointerup", &init).unwrap();
    el.dispatch_event(&up).unwrap();
    down
}

#[wasm_bindgen_test]
fn a_press_on_a_button_focuses_it_and_blurs_the_field_before_the_click() {
    let f = mount();
    f.focus_field();
    let ev = press(&by_id("btn"));
    assert_eq!(
        f.log(),
        ["blur", "btn"],
        "the field lets go before the click"
    );
    assert_eq!(active_id(), "btn", "the button takes the focus");
    assert!(ev.default_prevented(), "the press is still prevented");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_press_inside_a_tabindex_row_focuses_the_row() {
    let f = mount();
    f.focus_field();
    press(&by_id("chev"));
    assert_eq!(f.log(), ["blur", "chev"]);
    assert_eq!(active_id(), "row");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_press_on_a_handler_inside_a_button_focuses_the_button() {
    let f = mount();
    f.focus_field();
    press(&by_id("inner"));
    assert_eq!(f.log(), ["blur", "inner"]);
    assert_eq!(active_id(), "outer");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_focus_move_made_by_the_handler_wins() {
    let f = mount();
    f.focus_field();
    press(&by_id("mover"));
    assert_eq!(f.log(), ["blur", "mover"]);
    assert_eq!(
        active_id(),
        "other",
        "the handler's own focus is not undone"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_press_in_a_nofocus_region_moves_no_focus() {
    let f = mount();
    f.focus_field();
    press(&by_id("guarded"));
    assert_eq!(
        f.log(),
        ["guarded"],
        "the click runs, the field is not blurred"
    );
    assert_eq!(active_id(), "field");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_press_with_nothing_focusable_on_its_chain_moves_no_focus() {
    let f = mount();
    f.focus_field();
    let ev = press(&by_id("plain"));
    assert_eq!(f.log(), ["plain"]);
    assert_eq!(active_id(), "field");
    assert!(ev.default_prevented());
    f.teardown();
}

#[wasm_bindgen_test]
fn a_press_on_a_text_field_inside_a_clickable_is_left_to_the_browser() {
    let f = mount();
    f.focus_field();
    let ev = press(&by_id("inner-field"));
    assert_eq!(f.log(), ["group"], "the clickable's handler still runs");
    assert!(
        !ev.default_prevented(),
        "the browser focuses the field and puts the caret where the press is"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_press_on_the_color_input_field_is_left_to_the_browser() {
    let f = mount();
    let input = document()
        .query_selector(".rinch-color-input input")
        .unwrap()
        .expect("the ColorInput's text field");
    let opened = || {
        document()
            .query_selector(".rinch-color-input--opened")
            .unwrap()
            .is_some()
    };
    assert!(!opened(), "control: closed");
    let ev = press(&input);
    assert!(opened(), "positive control: the input group's handler ran");
    assert!(
        !ev.default_prevented(),
        "the browser focuses the field and places its caret"
    );
    f.teardown();
}
