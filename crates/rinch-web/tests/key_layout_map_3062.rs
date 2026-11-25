//! #3062 on the web: the page asks the browser for its keyboard layout map
//! (`navigator.keyboard.getLayoutMap()`, Chromium) when the first root
//! mounts, so what each key types without AltGr is known before the page's
//! first key. It used to be asked at the first keydown, and that press (an
//! AltGr one on a non-US layout) was judged by the US fallback.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test key_layout_map_3062
//! ```
//!
//! Its own binary, so no other test's keys have taught or cleared the table.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/engine.rs"]
mod engine;

wasm_bindgen_test_configure!(run_in_browser);

/// Resolve after `ms` of wall clock.
async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Whether this browser has the Keyboard API's layout map.
fn has_layout_map() -> bool {
    let get = |target: &wasm_bindgen::JsValue, name: &str| {
        js_sys::Reflect::get(target, &name.into()).unwrap_or_default()
    };
    let keyboard = get(&web_sys::window().unwrap().navigator().into(), "keyboard");
    keyboard.is_object() && get(&keyboard, "getLayoutMap").is_function()
}

/// Mount, press nothing, and wait: where the browser has a layout map, the
/// page knows `KeyQ`'s character from the map alone. Where it has none
/// (Firefox, Safari), nothing is known until a press teaches it, which is
/// the control that the answer came from the map and not from a key.
#[wasm_bindgen_test]
async fn the_layout_map_is_asked_at_mount_not_at_the_first_key() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        |scope: &mut RenderScope| scope.create_element("div"),
    );
    // The map is a promise; give it time to settle.
    let mut known = None;
    for _ in 0..40 {
        known = rinch_web::__known_unmodified_key("KeyQ");
        if known.is_some() {
            break;
        }
        sleep_ms(25).await;
    }
    root.unmount();
    host.remove();
    // The engines' own answer, so the test cannot pass on the branch
    // without the map by accident: Chromium has it, Gecko does not.
    assert_eq!(
        has_layout_map(),
        !engine::is_gecko(),
        "Chromium has getLayoutMap, Firefox does not"
    );
    if has_layout_map() {
        assert!(
            known.is_some(),
            "the layout map seeded the page's keys before any keydown"
        );
    } else {
        assert_eq!(
            known, None,
            "no layout map: nothing is known before a press"
        );
    }
}
