//! A click on a field component's label focuses its field, in a browser
//! (issue #2034).
//!
//! The field components rendered their `<label>` beside the field with no
//! `for`, so it labelled nothing and a click on it focused nothing. Each now
//! names its field by id (`rinch-components/tests/field_label_for_2034.rs`
//! pins the markup); here the browser's own label activation runs on it. The
//! click is `HTMLElement.click()`, which runs a label's activation behaviour
//! as a real click does. The control: a bare `<label>` beside an `<input>`
//! (the shape these components had) focuses nothing under the same click.
//!
//! `Select`'s trigger is a `div`, which no `for` can reach: its label
//! forwards the press through its own `data-rid` handler.
//!
//! **Mount through `rinch_web::mount_into`** — without the owner `mount_tree`
//! pushes, component effects are disposed at once.
#![cfg(target_arch = "wasm32")]

use rinch::components::{
    ColorInput, NumberInput, PasswordInput, Select, SelectOption, TextInput, Textarea,
};
use rinch_core::Component;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-2034";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
}

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
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            // The control: the shape the components had before #2034.
            let bare = scope.create_element("div");
            let label = scope.create_element("label");
            label.set_attribute("id", "bare-label");
            label.append_child(&scope.create_text("Bare"));
            let input = scope.create_element("input");
            input.set_attribute("id", "bare-input");
            bare.append_child(&label);
            bare.append_child(&input);
            root.append_child(&bare);

            let parts = [
                TextInput {
                    label: "Text".into(),
                    ..Default::default()
                }
                .render(scope, &[]),
                Textarea {
                    label: "Area".into(),
                    ..Default::default()
                }
                .render(scope, &[]),
                NumberInput {
                    label: "Number".into(),
                    ..Default::default()
                }
                .render(scope, &[]),
                PasswordInput {
                    label: "Password".into(),
                    ..Default::default()
                }
                .render(scope, &[]),
                ColorInput {
                    label: "Colour".into(),
                    ..Default::default()
                }
                .render(scope, &[]),
                Select {
                    label: "Fruit".into(),
                    data: vec![SelectOption::new("apple", "Apple")],
                    ..Default::default()
                }
                .render(scope, &[]),
            ];
            for part in &parts {
                root.append_child(part);
            }
            root
        },
    );
    Fixture { root, host }
}

impl Fixture {
    fn part(&self, class: &str) -> web_sys::HtmlElement {
        self.host
            .query_selector(&format!(".{class}"))
            .unwrap()
            .unwrap_or_else(|| panic!("no .{class}"))
            .dyn_into()
            .unwrap()
    }

    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn blur_all() {
    if let Some(active) = document().active_element()
        && let Ok(el) = active.dyn_into::<web_sys::HtmlElement>()
    {
        el.blur().unwrap();
    }
}

fn is_active(el: &web_sys::Element) -> bool {
    document().active_element().as_ref() == Some(el)
}

#[wasm_bindgen_test]
fn a_bare_label_beside_a_field_focuses_nothing() {
    let f = mount();
    blur_all();
    let label: web_sys::HtmlElement = document()
        .get_element_by_id("bare-label")
        .unwrap()
        .dyn_into()
        .unwrap();
    label.click();
    let input = document().get_element_by_id("bare-input").unwrap();
    assert!(!is_active(&input), "control: no `for`, no focus");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_click_on_each_field_label_focuses_its_field() {
    let f = mount();
    for (label, field) in [
        ("rinch-text-input__label", "rinch-text-input__input"),
        ("rinch-textarea__label", "rinch-textarea__input"),
        ("rinch-number-input__label", "rinch-number-input__input"),
        ("rinch-password-input__label", "rinch-password-input__input"),
        ("rinch-color-input__label", "rinch-color-input__input"),
    ] {
        blur_all();
        let field = f.part(field);
        assert!(!is_active(&field), "{label}: starts unfocused");
        f.part(label).click();
        assert!(is_active(&field), "{label}: the click focused its field");
    }
    f.teardown();
}

#[wasm_bindgen_test]
fn a_press_on_the_select_label_focuses_the_trigger() {
    let f = mount();
    blur_all();
    let trigger = f.part("rinch-select__input");
    let label = f.part("rinch-select__label");
    let r = label.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(1);
    init.set_client_x((r.x() + 4.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let down = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    label.dispatch_event(&down).unwrap();
    init.set_buttons(0);
    let up = web_sys::PointerEvent::new_with_event_init_dict("pointerup", &init).unwrap();
    label.dispatch_event(&up).unwrap();
    assert!(is_active(&trigger), "the label's press focused the trigger");
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("false"),
        "and left the list shut"
    );
    f.teardown();
}
