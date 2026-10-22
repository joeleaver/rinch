//! `rinch::window` in the browser (issue #1770): the title is the document's,
//! the size and scale are the page's, and `window_focused` follows the
//! window's own `focus` / `blur` — reactively, through a `{|| …}` mounted with
//! `rinch_web::mount_into`.
//!
//! The browser cannot be resized or moved to another display from a test, so
//! the `resize` and resolution listeners are pinned only at their reading
//! (the values equal the page's); the focus fixtures pin the publishing path
//! they share, with synthetic window events.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn window() -> web_sys::Window {
    web_sys::window().unwrap()
}

fn document() -> web_sys::Document {
    window().document().unwrap()
}

/// Mount `p#id` showing the three facts, and return the root and host.
fn mount(id: &'static str) -> (rinch_web::RootHandle, web_sys::Element) {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| -> NodeHandle {
            let p = scope.create_element("p");
            p.set_attribute("id", id);
            let text = scope.create_text("");
            p.append_child(&text);
            let t = text.clone();
            scope.create_effect(move || {
                t.set_text(&format!(
                    "{}|{:?}|{}",
                    rinch::scale_factor(),
                    rinch::window_size(),
                    rinch::window_focused()
                ));
            });
            p
        },
    );
    (root, host)
}

fn shown(id: &str) -> String {
    document()
        .get_element_by_id(id)
        .unwrap()
        .text_content()
        .unwrap()
}

fn fire_on_window(kind: &str) {
    let event = web_sys::Event::new(kind).unwrap();
    window().dispatch_event(&event).unwrap();
}

#[wasm_bindgen_test]
fn the_size_and_scale_are_the_pages() {
    let (root, host) = mount("w1770-read");
    let w = window();
    let width = w.inner_width().unwrap().as_f64().unwrap();
    let height = w.inner_height().unwrap().as_f64().unwrap();
    assert_eq!(rinch::scale_factor(), w.device_pixel_ratio());
    assert_eq!(rinch::window_size(), (width, height));
    let text = shown("w1770-read");
    assert!(
        text.starts_with(&format!(
            "{}|({width:?}, {height:?})|",
            w.device_pixel_ratio()
        )),
        "{text}"
    );
    // A `resize` with nothing changed publishes equal values.
    fire_on_window("resize");
    assert_eq!(shown("w1770-read"), text);
    root.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn window_focus_and_blur_reach_a_reader() {
    let (root, host) = mount("w1770-focus");
    fire_on_window("focus");
    assert!(rinch::window_focused());
    assert!(
        shown("w1770-focus").ends_with("|true"),
        "{}",
        shown("w1770-focus")
    );

    fire_on_window("blur");
    assert!(!rinch::window_focused());
    assert!(
        shown("w1770-focus").ends_with("|false"),
        "{}",
        shown("w1770-focus")
    );

    fire_on_window("focus");
    assert!(shown("w1770-focus").ends_with("|true"));

    // An element's blur is not the window's, even one that bubbles up to it.
    let field = document().create_element("input").unwrap();
    host.append_child(&field).unwrap();
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    let blur = web_sys::Event::new_with_event_init_dict("blur", &init).unwrap();
    field.dispatch_event(&blur).unwrap();
    assert!(
        rinch::window_focused(),
        "an element's bubbling blur was read as the window's"
    );
    root.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn the_title_is_the_documents() {
    let before = document().title();
    rinch::set_window_title("Rinch #1770 — ünïcode");
    assert_eq!(document().title(), "Rinch #1770 — ünïcode");
    document().set_title(&before);
}

#[wasm_bindgen_test]
fn a_string_that_is_not_a_url_opens_nothing() {
    for bad in ["example.com", "-x:y", "java script:x", ""] {
        assert_eq!(
            rinch::open_url(bad),
            Err(rinch::OpenUrlError::Invalid),
            "{bad:?}"
        );
    }
}

/// `window.open("javascript:…")` runs the script with this page's origin,
/// `opener` cleared or not: refused before the browser sees it.
#[wasm_bindgen_test]
fn script_urls_are_refused() {
    for bad in [
        "javascript:void(0)",
        "JAVASCRIPT:void(0)",
        "data:text/html,x",
        "vbscript:x",
    ] {
        assert_eq!(
            rinch::open_url(bad),
            Err(rinch::OpenUrlError::ScriptScheme),
            "{bad:?}"
        );
    }
}
