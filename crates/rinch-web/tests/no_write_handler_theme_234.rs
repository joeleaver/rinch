//! Issue #234, item 2 (PR #1134 review, F1): a click handler that calls
//! `update_theme` directly and writes no signal still restyles the page. The
//! web rewrites the page-global theme `<style>` only from its signal-change
//! callback, so the batch the dispatch runs in must still call it for a theme
//! change. The signal-write control proves the delegated click fired.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn theme_text() -> String {
    document()
        .query_selector("[data-rinch-theme]")
        .unwrap()
        .map(|e| e.text_content().unwrap_or_default())
        .unwrap_or_default()
}

fn run(write_signal: bool) -> (u32, bool) {
    let ran = Rc::new(Cell::new(0u32));
    let r = ran.clone();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| -> NodeHandle {
            let sig = rinch_core::Signal::new(0);
            let b = scope.create_element("button");
            b.set_attribute("id", "theme-234");
            let id = scope.register_handler(move || {
                r.set(r.get() + 1);
                rinch::update_theme(&ThemeProviderProps {
                    dark_mode: true,
                    ..Default::default()
                });
                if write_signal {
                    sig.set(1);
                }
            });
            b.set_attribute("data-rid", &id.0.to_string());
            b
        },
    );
    let before = theme_text();
    document()
        .get_element_by_id("theme-234")
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    let changed = theme_text() != before;
    root.unmount();
    host.remove();
    rinch::update_theme(&ThemeProviderProps::default());
    (ran.get(), changed)
}

#[wasm_bindgen_test]
fn a_no_write_handler_that_updates_the_theme_restyles_the_page() {
    let (ran, changed) = run(false);
    assert_eq!(ran, 1, "the handler ran");
    assert!(changed, "the theme <style> was rewritten");
}

#[wasm_bindgen_test]
fn control_with_a_signal_write() {
    let (ran, changed) = run(true);
    assert_eq!(ran, 1);
    assert!(changed);
}
