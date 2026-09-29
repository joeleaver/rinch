//! `Select`'s open list answers the keyboard in a real browser (issue #434).
//!
//! The web backend has no focus arbiter: the trigger is focused by the browser
//! (`tabindex="0"`) and every key reaches rinch through the document `keydown`
//! delegate, which calls `dispatch_keyboard_event` and `preventDefault`s a
//! consumed key. What is checked here is that path end to end with genuine
//! `KeyboardEvent`s dispatched on the focused trigger:
//!
//! - ArrowDown/ArrowUp/Home/End move the highlight, and each is
//!   `defaultPrevented` — otherwise the arrows also scroll the page;
//! - Enter commits the highlighted option (not the trigger's own Enter
//!   activation, which would merely toggle the list shut);
//! - Escape closes without committing and the trigger keeps focus;
//! - a closed `Select` leaves ArrowDown to the browser (not prevented);
//! - `aria-activedescendant` names an element that exists in the document.
//!
//! The positive control is the first assertion of every fixture: Enter on the
//! focused trigger opened the list, so a genuine key did reach rinch.
//!
//! **Mount through `rinch_web::mount_into`** — without the owner `mount_tree`
//! pushes, component effects are disposed at once.
#![cfg(target_arch = "wasm32")]

use rinch::components::{Select, SelectOption};
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, InputCallback};
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-434";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn mount(value: &str) -> (web_sys::Document, Rc<RefCell<Vec<String>>>) {
    let bdoc = browser_document();
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
    host.set_attribute("style", "width: 240px; margin: 40px")
        .unwrap();
    bdoc.body().unwrap().append_child(&host).unwrap();
    let picks = Rc::new(RefCell::new(Vec::new()));
    let sink = picks.clone();
    let value = value.to_string();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let sink = sink.clone();
        Select {
            value: value.clone(),
            data: vec![
                SelectOption::new("apple", "Apple"),
                SelectOption::new("banana", "Banana"),
                SelectOption::new("cherry", "Cherry"),
                SelectOption::new("date", "Date"),
            ],
            onchange: Some(InputCallback::new(move |v: String| {
                sink.borrow_mut().push(v)
            })),
            ..Default::default()
        }
        .render(scope, &[])
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    (bdoc, picks)
}

fn one(doc: &web_sys::Document, selector: &str) -> web_sys::Element {
    doc.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("nothing matches `{selector}`"))
}

fn keydown(el: &web_sys::Element, key: &str) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

fn is_open(trigger: &web_sys::Element) -> bool {
    trigger.get_attribute("aria-expanded").as_deref() == Some("true")
}

fn highlighted(doc: &web_sys::Document) -> Option<usize> {
    let options = doc.query_selector_all(".rinch-select__option").unwrap();
    let mut lit = Vec::new();
    for i in 0..options.length() {
        let el: web_sys::Element = options.item(i).unwrap().dyn_into().unwrap();
        if el
            .get_attribute("class")
            .is_some_and(|c| c.split_whitespace().any(|one| one == "rinch-select__option--highlighted"))
        {
            lit.push(i as usize);
        }
    }
    assert!(lit.len() <= 1, "more than one highlighted option: {lit:?}");
    lit.first().copied()
}

/// Focus the trigger and open it with Enter — the positive control.
fn open(doc: &web_sys::Document) -> web_sys::Element {
    let trigger = one(doc, ".rinch-select__input");
    trigger
        .dyn_ref::<web_sys::HtmlElement>()
        .unwrap()
        .focus()
        .unwrap();
    assert_eq!(
        doc.active_element().as_ref(),
        Some(&trigger),
        "precondition: the trigger takes focus"
    );
    keydown(&trigger, "Enter");
    assert!(is_open(&trigger), "precondition: Enter opened the list");
    trigger
}

#[wasm_bindgen_test]
fn the_arrows_move_the_highlight_and_are_default_prevented_in_chrome() {
    let (doc, picks) = mount("banana");
    let trigger = open(&doc);
    assert_eq!(highlighted(&doc), Some(1), "the highlight starts at the selection");

    let ev = keydown(&trigger, "ArrowDown");
    assert!(ev.default_prevented(), "a consumed ArrowDown must not scroll the page");
    assert_eq!(highlighted(&doc), Some(2));
    keydown(&trigger, "ArrowUp");
    assert_eq!(highlighted(&doc), Some(1));
    assert!(keydown(&trigger, "End").default_prevented());
    assert_eq!(highlighted(&doc), Some(3));
    keydown(&trigger, "Home");
    assert_eq!(highlighted(&doc), Some(0));
    keydown(&trigger, "c");
    assert_eq!(highlighted(&doc), Some(2), "c → Cherry");

    let active = trigger
        .get_attribute("aria-activedescendant")
        .expect("an open list names its active option");
    let named = doc
        .get_element_by_id(&active)
        .expect("aria-activedescendant names an element in the document");
    assert!(named.text_content().unwrap_or_default().contains("Cherry"));
    assert!(picks.borrow().is_empty(), "moving commits nothing");
}

#[wasm_bindgen_test]
fn enter_commits_the_highlighted_option_in_chrome() {
    let (doc, picks) = mount("banana");
    let trigger = open(&doc);
    keydown(&trigger, "ArrowDown");
    keydown(&trigger, "ArrowDown");
    let ev = keydown(&trigger, "Enter");
    assert!(ev.default_prevented());
    assert!(!is_open(&trigger), "Enter closes the list");
    assert_eq!(*picks.borrow(), vec!["date".to_string()]);
    assert_eq!(doc.active_element().as_ref(), Some(&trigger));
}

#[wasm_bindgen_test]
fn escape_closes_without_committing_in_chrome() {
    let (doc, picks) = mount("banana");
    let trigger = open(&doc);
    keydown(&trigger, "ArrowDown");
    let ev = keydown(&trigger, "Escape");
    assert!(ev.default_prevented());
    assert!(!is_open(&trigger), "Escape closes the list");
    assert!(picks.borrow().is_empty(), "and commits nothing");
    assert_eq!(doc.active_element().as_ref(), Some(&trigger));

    // Closed again: ArrowDown is the browser's.
    let ev = keydown(&trigger, "ArrowDown");
    assert!(
        !ev.default_prevented(),
        "a closed Select must leave ArrowDown to the browser"
    );
}
