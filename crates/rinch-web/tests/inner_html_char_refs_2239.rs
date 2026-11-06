//! `set_inner_html` decodes character references (#2239) in the browser:
//! rinch-web hands the markup to the browser's own parser, so this pins that
//! the cases the desktop test checks (`rinch-dom/tests/support/
//! char_refs_2239.rs`) are what Chrome and Firefox answer, and that the two
//! backends agree.
//!
//! Run with `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`
//! and a chromedriver matching the installed Chrome (or a geckodriver).
#![cfg(target_arch = "wasm32")]

#[path = "../../rinch-dom/tests/support/char_refs_2239.rs"]
mod char_refs;
#[path = "support/engine.rs"]
mod engine;

use rinch_core::dom::DomDocument;
use rinch_web::web_document::WebDocument;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> (WebDocument, rinch_core::dom::NodeId) {
    let d = web_sys::window().unwrap().document().unwrap();
    let host = d.create_element("div").unwrap();
    d.body().unwrap().append_child(&host).unwrap();
    let mut doc = WebDocument::new_into(d, host);
    let body = doc.body();
    let el = doc.create_element("div");
    doc.append_child(body, el);
    (doc, el)
}

#[wasm_bindgen_test]
fn inner_html_decodes_character_references_as_desktop_does() {
    let (mut doc, host) = document();
    char_refs::check(&mut doc, host);
}

/// Where the engines differ: Chrome's fast `innerHTML` path decodes some
/// legacy names before `=` or an alphanumeric in an attribute as if they
/// were text, which the standard, Firefox and desktop leave alone (Chrome
/// 156 and 157: `?a=1&copy=2&amp;` reads `?a=1©=2&`).
#[wasm_bindgen_test]
fn a_legacy_name_in_an_attribute_on_the_fast_path_by_engine() {
    let (mut doc, host) = document();
    for &(value, standard) in char_refs::LEGACY_IN_ATTRIBUTES {
        doc.set_inner_html(host, &char_refs::legacy_markup(value, true));
        let span = doc.get_children(host)[0];
        let got = doc.get_attribute(span, "title");
        if engine::is_gecko() {
            assert_eq!(got.as_deref(), Some(standard), "Firefox: {value:?}");
        } else {
            let as_text = rinch_core::html_entities::decode_text(value);
            assert!(
                got.as_deref() == Some(standard) || got.as_deref() == Some(as_text.as_str()),
                "Chrome: {value:?} read {got:?}, neither {standard:?} nor {as_text:?}"
            );
        }
    }
}
