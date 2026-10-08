//! A focus move rinch makes does not run the browser's focus listeners with the
//! document borrowed.
//!
//! `NodeHandle::focus`, `focus_into` and `restore_focus` reach the browser's
//! `focus()` / `blur()`, which dispatches `focus`, `blur`, `focusin` and
//! `focusout` synchronously. rinch's own listeners touch the document: the
//! document's `focusin` tells every key entry (an open `Select` list) that
//! focus moved, which asks its owner for `active_element()` and flushes pending
//! effects (a modal's scroll lock). Run under the `borrow_mut` the handle method
//! held, each panicked with "RefCell already borrowed" and left the document
//! borrowed for good (wasm does not unwind). The browser work now runs once the
//! borrow is released (`DomDocument::take_after_borrow`), before the handle
//! method returns.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test focus_move_listener_reentry
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::components::{Modal, Select, SelectOption};
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn host() -> web_sys::Element {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

fn active_id() -> String {
    document()
        .active_element()
        .map(|el| el.id())
        .unwrap_or_default()
}

fn modal(s: &mut RenderScope, open: Signal<bool>, input_id: &str) -> NodeHandle {
    let input = s.create_element("input");
    input.set_attribute("id", input_id);
    Modal {
        opened_fn: Some(Rc::new(move || open.get())),
        trap_focus: true,
        with_close_button: false,
        ..Default::default()
    }
    .render(s, &[input])
}

/// The reported shape: a `Select` list is open (its key entry is live) when a
/// `trap_focus` modal opens, e.g. from a menu chord. The modal's `focus_into`
/// fires `focusin`; telling the entry that focus left its owner runs the
/// modal's queued scroll-lock effect.
#[wasm_bindgen_test]
fn a_modal_opening_while_a_select_list_is_open_takes_the_keyboard() {
    let host = host();
    let open = Signal::new(false);
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let sel = Select {
            value: "a".into(),
            data: vec![SelectOption::new("a", "A"), SelectOption::new("b", "B")],
            ..Default::default()
        }
        .render(s, &[]);
        page.append_child(&sel);
        page.append_child(&modal(s, open, "in-modal-select"));
        page
    });
    let trigger: web_sys::HtmlElement = document()
        .query_selector(".rinch-select__input")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    trigger.focus().unwrap();
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Enter");
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    trigger.dispatch_event(&ev).unwrap();
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("true"),
        "positive control: the key reached rinch and opened the list"
    );
    open.set(true);
    assert_eq!(
        active_id(),
        "in-modal-select",
        "the modal took the keyboard"
    );
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("false"),
        "focus leaving the trigger closed the list"
    );
    root.unmount();
    host.remove();
}

struct KeyEntry {
    root: rinch_web::RootHandle,
    host: web_sys::Element,
    open: Signal<bool>,
    left: Rc<Cell<u32>>,
    other: NodeHandle,
    _keep: Rc<RefCell<Vec<rinch_core::DismissHandle>>>,
}

/// A focused `#trigger` owning a live key entry (the `Select` shape, bare), a
/// plain `#other` button, and a closed `trap_focus` modal.
fn key_entry() -> KeyEntry {
    let host = host();
    let open = Signal::new(false);
    let left = Rc::new(Cell::new(0u32));
    let keep: Rc<RefCell<Vec<rinch_core::DismissHandle>>> = Default::default();
    let other: Rc<RefCell<Option<NodeHandle>>> = Default::default();
    let (l, k, o) = (left.clone(), keep.clone(), other.clone());
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let trigger = s.create_element("button");
        trigger.set_attribute("id", "trigger");
        page.append_child(&trigger);
        let b = s.create_element("button");
        b.set_attribute("id", "other");
        page.append_child(&b);
        *o.borrow_mut() = Some(b);
        page.append_child(&modal(s, open, "in-modal-entry"));
        let l = l.clone();
        k.borrow_mut().push(rinch_core::push_key_handler(
            &trigger,
            |_| false,
            move || l.set(l.get() + 1),
        ));
        page
    });
    let trigger: web_sys::HtmlElement = document()
        .get_element_by_id("trigger")
        .unwrap()
        .dyn_into()
        .unwrap();
    trigger.focus().unwrap();
    assert_eq!(active_id(), "trigger", "positive control");
    assert_eq!(left.get(), 0);
    let other = other.borrow_mut().take().unwrap();
    KeyEntry {
        root,
        host,
        open,
        left,
        other,
        _keep: keep,
    }
}

/// `focus_into` and then `restore_focus`, each with a live key entry: both
/// moves fire `focusin`, whose listener reads the document.
#[wasm_bindgen_test]
fn a_modal_opening_and_closing_over_a_live_key_entry_moves_focus_both_ways() {
    let f = key_entry();
    f.open.set(true);
    assert_eq!(active_id(), "in-modal-entry", "the modal took the keyboard");
    assert_eq!(f.left.get(), 1, "the entry heard focus leave its owner");
    f.open.set(false);
    assert_eq!(active_id(), "trigger", "closing handed the keyboard back");
    f.root.unmount();
    f.host.remove();
}

/// The plain verb, `NodeHandle::focus`, from app code.
#[wasm_bindgen_test]
fn a_handle_focus_over_a_live_key_entry_moves_focus() {
    let f = key_entry();
    f.other.focus();
    assert_eq!(
        active_id(),
        "other",
        "focus moved before `focus()` returned"
    );
    assert_eq!(f.left.get(), 1, "the entry heard focus leave its owner");
    // The document is free again: a later write does not panic.
    f.other.set_attribute("data-after", "1");
    f.root.unmount();
    f.host.remove();
}

/// A modified `<input>` fires `change` when the modal's `focus_into` blurs it:
/// the app's `onchange` writes a signal whose effect writes the document.
#[wasm_bindgen_test]
fn a_modal_opening_over_a_modified_input_runs_onchange() {
    let host = host();
    let open = Signal::new(false);
    let committed = Signal::new(String::new());
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let m = modal(__scope, open, "in-modal-change");
            let page = rsx! {
                div {
                    input { id: "field", onchange: move |v: String| committed.set(v) }
                    p { id: "echo", {|| committed.get()} }
                }
            };
            page.append_child(&m);
            page
        },
    );
    let field: web_sys::HtmlInputElement = document()
        .get_element_by_id("field")
        .unwrap()
        .dyn_into()
        .unwrap();
    field.focus().unwrap();
    // A `.value` write does not arm `change`; typing does.
    let exec: js_sys::Function = js_sys::Reflect::get(&document(), &"execCommand".into())
        .unwrap()
        .dyn_into()
        .unwrap();
    let ok = exec
        .call3(
            &document(),
            &"insertText".into(),
            &false.into(),
            &"typed".into(),
        )
        .unwrap();
    assert!(ok.as_bool().unwrap_or(false), "positive control: typed");
    open.set(true);
    assert_eq!(
        active_id(),
        "in-modal-change",
        "the modal took the keyboard"
    );
    assert_eq!(
        committed.get(),
        "typed",
        "onchange ran with the field's value"
    );
    assert_eq!(
        document()
            .get_element_by_id("echo")
            .unwrap()
            .text_content()
            .as_deref(),
        Some("typed"),
        "and its effect wrote the document"
    );
    root.unmount();
    host.remove();
}
