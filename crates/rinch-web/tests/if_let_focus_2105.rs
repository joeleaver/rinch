//! An rsx `if let` around a form field, in a real browser (issue #2105).
//!
//! A branch whose pattern binds a value is rebuilt when that value changes,
//! so an `<input>` inside it whose own `oninput` writes the scrutinee is
//! replaced on every keystroke, and the browser drops its focus. `#[once]`
//! keeps the branch built once per selection: the same `<input>` stays, and
//! so does the focus.
//!
//! The positive control in each test is that the dispatched `input` event
//! reached rinch's delegation: the signal holds what was typed.
//!
//! **Mount through `rinch_web::mount_into`** — without the owner `mount_tree`
//! pushes, component effects are disposed at once.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-2105";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

#[component]
fn follow_form(draft: Signal<Option<String>>) -> NodeHandle {
    rsx! {
        div {
            if let Some(text) = draft.get() {
                input {
                    id: "field-2105",
                    value: text.clone(),
                    oninput: move |v: String| draft.set(Some(v)),
                }
            }
        }
    }
}

#[component]
fn once_form(draft: Signal<Option<String>>) -> NodeHandle {
    rsx! {
        div {
            #[once]
            if let Some(text) = draft.get() {
                input {
                    id: "field-2105",
                    value: text.clone(),
                    oninput: move |v: String| draft.set(Some(v)),
                }
            }
        }
    }
}

fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) {
    let bdoc = document();
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = bdoc.query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    let host = bdoc.create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    bdoc.body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
}

fn field() -> web_sys::HtmlInputElement {
    document()
        .get_element_by_id("field-2105")
        .expect("the field is mounted")
        .dyn_into()
        .unwrap()
}

/// Focus the field, then type one character the way the browser reports it:
/// the value changes and an `input` event bubbles.
fn focus_and_type(text: &str) -> web_sys::HtmlInputElement {
    let el = field();
    el.focus().unwrap();
    assert_eq!(
        document().active_element().map(|a| a.id()),
        Some("field-2105".to_string()),
        "the field took focus"
    );
    el.set_value(text);
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    let ev = web_sys::Event::new_with_event_init_dict("input", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    el
}

#[wasm_bindgen_test]
fn a_rebuilt_field_loses_focus() {
    let draft = Signal::new(Some("h".to_string()));
    mount(move |s| follow_form(s, draft));
    let before = focus_and_type("he");
    assert_eq!(draft.get().as_deref(), Some("he"), "the input reached rinch");
    let after = field();
    assert!(
        !before.is_same_node(Some(&after)),
        "the default rebuild replaced the <input>"
    );
    assert_ne!(
        document().active_element().map(|a| a.id()),
        Some("field-2105".to_string()),
        "a replaced <input> does not keep the focus"
    );
}

#[wasm_bindgen_test]
fn a_once_field_keeps_focus() {
    let draft = Signal::new(Some("h".to_string()));
    mount(move |s| once_form(s, draft));
    let before = focus_and_type("he");
    assert_eq!(draft.get().as_deref(), Some("he"), "the input reached rinch");
    assert!(
        before.is_same_node(Some(&field())),
        "`#[once]` keeps the same <input>"
    );
    assert_eq!(
        document().active_element().map(|a| a.id()),
        Some("field-2105".to_string()),
        "and its focus"
    );
}
