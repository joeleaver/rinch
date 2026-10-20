//! #940 on the web: the history's typing-merge window is armed in a browser.
//!
//! `EditorHandle` stamps each local transaction with a `web_time` clock
//! (`performance.now()` in a browser, where `std::time::Instant::now` panics);
//! before #940 nothing stamped it, so all typing was one undo step. Typed here
//! through the capture textarea's `beforeinput`, the web glue's own channel.
//!
//! ```text
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_history_time_940
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::create_editor;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn sleep(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
}

fn capture() -> web_sys::HtmlTextAreaElement {
    document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .expect("the capture textarea exists once the editor was focused")
        .dyn_into()
        .unwrap()
}

fn before_input(input_type: &str, data: Option<&str>) {
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_input_type(input_type);
    init.set_data(data);
    let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
    capture().dispatch_event(&ev).unwrap();
}

fn text(editor: &web_sys::Element) -> String {
    editor.query_selector("p").unwrap().unwrap().text_content().unwrap_or_default()
}

#[wasm_bindgen_test]
async fn a_pause_past_the_window_starts_a_new_undo_step_in_a_browser() {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html("<p>x</p>"));
    let mounted = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| mounted.mount(scope),
    );
    let editor = document().query_selector("[data-pm-editor]").unwrap().unwrap();
    handle.set_selection(Selection::cursor(Pos(2)));
    handle.focus();
    assert!(
        document().active_element().as_deref() == Some(capture().as_ref()),
        "positive control: focus() puts the keyboard in the capture textarea"
    );

    before_input("insertText", Some("a"));
    before_input("insertText", Some("b"));
    assert_eq!(text(&editor), "xab", "positive control: beforeinput types");
    sleep(650).await;
    before_input("insertText", Some("c"));
    before_input("insertText", Some("d"));
    assert_eq!(text(&editor), "xabcd");

    before_input("historyUndo", None);
    assert_eq!(text(&editor), "xab", "undo takes back only `cd`");
    before_input("historyUndo", None);
    assert_eq!(text(&editor), "x", "then `ab`");

    root.unmount();
    host.remove();
}
