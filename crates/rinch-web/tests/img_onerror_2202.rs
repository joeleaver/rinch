//! `onerror` on an `<img>` (issues #2202, #2203) in the browser.
//!
//! The browser fires `error` at an `<img>` whose picture failed; it does not
//! bubble, so rinch-web listens for it in the capture phase and runs the
//! element's `data-onerror`. A source only the app understands, answered
//! "not yet", gives the browser no `src` at all, so it fires nothing; there
//! rinch-web fires `onerror` itself, as the desktop does for a loader's miss.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test img_onerror_2202
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::Cell;
use std::rc::Rc;

use rinch::image::{ImageLoadResult, register_image_scheme};
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Not a PNG: the browser fails to decode it.
const BAD_DATA_URI: &str = "data:image/png;base64,bm90IGEgcGljdHVyZQ==";
/// A 3x2 PNG.
const GOOD_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR42mP4z8DwH4YZkDkAm34L9Q9fJiIAAAAASUVORK5CYII=";

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

/// Mount one `<img src>` with an `onerror` counting its calls.
fn mount_img(src: &'static str) -> (rinch_web::RootHandle, Rc<Cell<u32>>) {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let fired = Rc::new(Cell::new(0));
    let f = fired.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let root = scope.create_element("div");
        let img = scope.create_element("img");
        let f = f.clone();
        let id = scope.register_handler(move || f.set(f.get() + 1));
        img.set_attribute("data-onerror", &id.0.to_string());
        img.set_attribute("src", src);
        root.append_child(&img);
        root
    });
    (root, fired)
}

async fn wait_for(fired: &Cell<u32>) {
    for _ in 0..100 {
        if fired.get() > 0 {
            break;
        }
        sleep(20).await;
    }
}

/// The browser's `error` reaches the element's `onerror`, once.
#[wasm_bindgen_test]
async fn a_picture_that_fails_runs_onerror() {
    let (_root, fired) = mount_img(BAD_DATA_URI);
    wait_for(&fired).await;
    sleep(100).await;
    assert_eq!(fired.get(), 1);
}

/// The control: a picture that loads runs nothing (and the listener is live:
/// the test above fails without it).
#[wasm_bindgen_test]
async fn a_picture_that_loads_runs_no_onerror() {
    let (_root, fired) = mount_img(GOOD_DATA_URI);
    sleep(300).await;
    assert_eq!(fired.get(), 0);
}

/// An app source answered "not yet" runs `onerror`, as the desktop's loader
/// miss does.
#[wasm_bindgen_test]
async fn an_app_source_answered_not_yet_runs_onerror() {
    register_image_scheme("web-2202-missing", |src: &str| {
        ImageLoadResult::Failed(format!("{src} is not here"))
    });
    let (_root, fired) = mount_img("web-2202-missing:portrait");
    wait_for(&fired).await;
    sleep(100).await;
    assert_eq!(fired.get(), 1);
}

/// `src=""` fails in the browser (the desktop now matches it).
#[wasm_bindgen_test]
async fn an_empty_src_runs_onerror() {
    let (_root, fired) = mount_img("");
    wait_for(&fired).await;
    sleep(100).await;
    assert_eq!(fired.get(), 1);
}

/// `Anchor { onclick }` with no `href` (#2206 review) is a keyboard stop the
/// browser focuses, and Enter on it runs the handler: the browser makes no
/// click for an `<a>` without `href`, so rinch's `tabindex` activation does.
#[wasm_bindgen_test]
fn an_anchor_with_onclick_and_no_href_runs_on_enter() {
    use rinch::prelude::Anchor;
    use rinch_core::{Callback, Component};
    use wasm_bindgen::JsCast;
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let clicks = Rc::new(Cell::new(0));
    let c = clicks.clone();
    let _root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let c = c.clone();
        let text = scope.create_text("Settings");
        let a = Anchor {
            onclick: Some(Callback::new(move || c.set(c.get() + 1))),
            ..Default::default()
        }
        .render(scope, &[text]);
        a.set_attribute("id", "anchor-2202");
        a
    });
    let a: web_sys::HtmlElement = document()
        .get_element_by_id("anchor-2202")
        .unwrap()
        .dyn_into()
        .unwrap();
    assert_eq!(a.tab_index(), 0, "the browser takes it as a stop");
    a.focus().unwrap();
    assert_eq!(
        document().active_element().map(|e| e.id()),
        Some("anchor-2202".to_string()),
        "it takes the focus"
    );
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Enter");
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    a.dispatch_event(&ev).unwrap();
    assert_eq!(clicks.get(), 1, "Enter ran its onclick");
}
