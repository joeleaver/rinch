//! `rinch::dialogs::message(..).confirm()` in the browser, issue #1773.
//!
//! The same builder natively shows an rfd dialog; here it is the page's own
//! `window.confirm` / `window.alert`. A real one would block the headless
//! browser, so each test replaces `window.confirm` / `window.alert` with a
//! stub that records its message and answers, and puts the original back.
#![cfg(target_arch = "wasm32")]

use rinch::dialogs::{MessageLevel, message};
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Replace `window[name]` with a stub answering `answer` and recording each
/// message in `window.__rinch_1773_seen`. Returns the original, for `restore`.
fn stub(name: &str, answer: bool) -> JsValue {
    let window = web_sys::window().unwrap();
    let original = js_sys::Reflect::get(&window, &name.into()).unwrap();
    js_sys::Reflect::set(&window, &"__rinch_1773_seen".into(), &js_sys::Array::new()).unwrap();
    let f = js_sys::Function::new_with_args(
        "message",
        &format!("window.__rinch_1773_seen.push(message); return {answer};"),
    );
    js_sys::Reflect::set(&window, &name.into(), &f).unwrap();
    original
}

fn restore(name: &str, original: JsValue) {
    let window = web_sys::window().unwrap();
    js_sys::Reflect::set(&window, &name.into(), &original).unwrap();
}

fn seen() -> Vec<String> {
    let window = web_sys::window().unwrap();
    let arr: js_sys::Array = js_sys::Reflect::get(&window, &"__rinch_1773_seen".into())
        .unwrap()
        .into();
    arr.iter().map(|v| v.as_string().unwrap()).collect()
}

#[wasm_bindgen_test]
fn confirm_answers_what_the_browser_answers() {
    let original = stub("confirm", true);
    let yes = message("Delete the file?")
        .set_title("Confirm Delete")
        .set_level(MessageLevel::Warning)
        .confirm();
    let first = seen();
    restore("confirm", original);
    assert_eq!(
        first,
        ["Confirm Delete\n\nDelete the file?"],
        "window.confirm was asked"
    );
    assert!(yes);

    let original = stub("confirm", false);
    let no = message("Leave?").ask();
    let second = seen();
    restore("confirm", original);
    assert_eq!(second, ["Leave?"]);
    assert!(!no, "Cancel is false");
}

#[wasm_bindgen_test]
fn show_is_an_alert() {
    let original = stub("alert", true);
    message("Saved").show();
    let s = seen();
    restore("alert", original);
    assert_eq!(s, ["Saved"]);
}
