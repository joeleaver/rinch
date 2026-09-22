//! `live_value` on the web backend: the value binding and the `<select>` arm (from the review of PR #863).
#![cfg(target_arch = "wasm32")]

use rinch::prelude::{Component, TextInput};
use rinch_core::InputCallback;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::Signal;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-live-value";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

#[allow(dead_code)]
fn js(body: &str) -> wasm_bindgen::JsValue {
    js_sys::Function::new_no_args(body)
        .call0(&wasm_bindgen::JsValue::NULL)
        .unwrap()
}

struct Mounted {
    root: rinch_web::RootHandle,
    host: web_sys::Element,
    input: web_sys::HtmlInputElement,
    heard: Rc<RefCell<Vec<String>>>,
}

fn mount(
    signal: Signal<String>,
    input_type: &'static str,
    normalise: fn(&str, &str) -> String,
) -> Mounted {
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
    let heard: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let heard_in = heard.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let heard_in = heard_in.clone();
            TextInput {
                input_type: input_type.to_string(),
                value_fn: Some(Rc::new(move || signal.get())),
                oninput: Some(InputCallback::new(move |v: String| {
                    heard_in.borrow_mut().push(v.clone());
                    let old = signal.get();
                    signal.set(normalise(&old, &v));
                })),
                ..Default::default()
            }
            .render(scope, &[])
        },
    );
    let input: web_sys::HtmlInputElement = host
        .query_selector(".rinch-text-input__input")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    Mounted {
        root,
        host,
        input,
        heard,
    }
}

fn fire_input(input: &web_sys::HtmlInputElement) {
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    let ev = web_sys::Event::new_with_event_init_dict("input", &init).unwrap();
    input.dispatch_event(&ev).unwrap();
}

fn digits_only(old: &str, new: &str) -> String {
    if new.chars().all(|c| c.is_ascii_digit()) {
        new.to_string()
    } else {
        old.to_string()
    }
}

#[allow(dead_code)]
fn as_typed(_old: &str, new: &str) -> String {
    new.to_string()
}

/// The classic reject: the handler writes back the SAME value the signal
/// already held. The live text ('12a') differs, so the field must be rewritten.
#[wasm_bindgen_test]
fn a_rejecting_handler_rewrites_the_field_back() {
    let sig = Signal::new("12".to_string());
    let m = mount(sig, "text", digits_only);
    m.input.focus().unwrap();
    m.input.set_value("12a");
    fire_input(&m.input);
    assert_eq!(
        *m.heard.borrow(),
        vec!["12a".to_string()],
        "positive control"
    );
    assert_eq!(sig.get(), "12");
    assert_eq!(m.input.value(), "12", "a rejected keystroke must be undone");
    m.root.unmount();
    m.host.remove();
}

/// `<select>`: a user's pick moves `.value` and not the attribute; live_value
/// reports the pick. Kills a `live_value` with the select arm removed.
#[wasm_bindgen_test]
fn live_value_reads_a_select_pick() {
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let slot: Rc<RefCell<Option<rinch_core::dom::NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let sel = scope.create_element("select");
            for v in ["a", "b"] {
                let o = scope.create_element("option");
                o.set_attribute("value", v);
                sel.append_child(&o);
            }
            sel.set_attribute("value", "a");
            *slot_in.borrow_mut() = Some(sel.clone());
            sel
        },
    );
    let handle = slot.borrow_mut().take().unwrap();
    let select: web_sys::HtmlSelectElement = host
        .query_selector("select")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    select.set_value("b");
    assert_eq!(select.value(), "b", "control");
    assert_eq!(
        handle.get_attribute("value").as_deref(),
        Some("a"),
        "control: attribute lags"
    );
    assert_eq!(handle.live_value().as_deref(), Some("b"));
    root.unmount();
    host.remove();
}
