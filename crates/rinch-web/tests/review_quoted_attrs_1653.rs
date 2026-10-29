//! Review of PR #2003 (#1653): a quoted attribute on a component lands on the
//! root DOM element in a real browser and follows its signal surgically.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use std::cell::Cell;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

thread_local! { static RENDERS: Cell<u32> = const { Cell::new(0) }; }

#[component]
fn Counted(label: String) -> NodeHandle {
    RENDERS.with(|r| r.set(r.get() + 1));
    let root = __scope.create_element("section");
    root.set_attribute("id", "counted");
    root.set_attribute("title", "own");
    root.append_child(&__scope.create_text(&label));
    root
}

fn el(id: &str) -> web_sys::Element {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("no #{id}"))
}

#[wasm_bindgen_test]
fn quoted_title_on_a_component_root_is_reactive() {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let tip = Signal::new("Save".to_string());
    let before = RENDERS.with(Cell::get);
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |__scope| {
        rsx! {
            div {
                Counted { label: "x", "title": {move || tip.get()} }
                Button { "title": {move || format!("btn:{}", tip.get())}, "id": "btn", "Go" }
                ActionIcon { title: "Close it", "id": "ai" }
                ActionIcon { "id": "ai2" }
                CloseButton { title: "Dismiss", "id": "cb" }
                div { "id": "upper", "Title": "folded" }
            }
        }
    });

    // Positive control: the mount reached the DOM.
    let first = el("counted");
    assert_eq!(
        first.get_attribute("title").as_deref(),
        Some("Save"),
        "caller wins over own"
    );
    assert_eq!(el("btn").tag_name(), "BUTTON");
    assert_eq!(
        el("btn").get_attribute("title").as_deref(),
        Some("btn:Save")
    );
    assert_eq!(el("ai").tag_name(), "BUTTON");
    assert_eq!(el("ai").get_attribute("title").as_deref(), Some("Close it"));
    assert!(!el("ai2").has_attribute("title"), "empty title writes none");
    assert_eq!(el("cb").get_attribute("title").as_deref(), Some("Dismiss"));
    assert_eq!(
        el("cb").get_attribute("aria-label").as_deref(),
        Some("Close")
    );
    assert_eq!(
        el("upper").get_attribute("title").as_deref(),
        Some("folded")
    );
    assert_eq!(RENDERS.with(Cell::get) - before, 1);

    tip.set("Saving".to_string());
    let second = el("counted");
    assert!(
        first.is_same_node(Some(second.unchecked_ref())),
        "same root element"
    );
    assert_eq!(second.get_attribute("title").as_deref(), Some("Saving"));
    assert_eq!(
        el("btn").get_attribute("title").as_deref(),
        Some("btn:Saving")
    );
    assert_eq!(RENDERS.with(Cell::get) - before, 1, "no re-render");

    root.unmount();
    host.remove();
}
