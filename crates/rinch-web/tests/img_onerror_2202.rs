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
