//! A selection moved while the page's **first** mount is still building owes
//! an overlay pass before `editor_input::install` has registered the web's
//! scheduler (#1001's review). Nothing was told, and a later owe of the same
//! already-owed document tells nobody either, so the owed pass stuck until
//! something else ran `refresh_caret`. `install` now schedules it itself.
//!
//! Its own test binary on purpose: `install` runs once per page, and only the
//! first mount of a binary reaches this shape.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_selection_before_install
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_editor_view::registry;
use rinch_web::create_editor;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

async fn next_task() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test]
async fn a_pass_owed_during_the_first_mount_is_run_once_install_can_schedule_it() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html("<p>line 000</p><p>line 001</p>"));
    let mounted = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let node = mounted.mount(scope);
            mounted.set_selection(Selection::text(Pos(11), Pos(19)));
            node
        },
    );
    assert_eq!(
        handle.selection(),
        Selection::text(Pos(11), Pos(19)),
        "control: the selection moved during the build"
    );
    assert!(
        registry::any_overlay_pass_owed(),
        "control: the move owed a pass before any scheduler existed"
    );
    next_task().await;
    assert!(
        !registry::any_overlay_pass_owed(),
        "install scheduled the pass it found owed, and it ran"
    );
    root.unmount();
    host.remove();
}
