//! #2075: a node view in the page's **first** mount is watched for resizes too.
//!
//! The node-view `ResizeObserver` learns of a view when the view mounts, during
//! the mount's build. Its listener used to be installed after the build, with
//! the rest of the editor's page setup, so the views of the page's first mount
//! were never observed: one that grew left the caret below it where it was.
//! `editor_node_views.rs` passed only because an earlier test in its page had
//! already mounted once; run alone, or first, it failed ("156 -> 156").
//!
//! This file is its own wasm binary, so its one test is its page's first mount.
//! Keep it to one test.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test node_view_first_mount_2075
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::Signal;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{NodeViewSpec, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn caret_top(host: &web_sys::Element) -> f64 {
    host.query_selector("[data-pm-caret]")
        .unwrap()
        .expect("a caret overlay")
        .get_bounding_client_rect()
        .top()
}

async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(resolve.unchecked_ref())
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

async fn after_microtasks() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// A frame and the microtasks after it: a `ResizeObserver` delivery and the
/// overlay pass it owes.
async fn frame() {
    next_frame().await;
    after_microtasks().await;
}

#[wasm_bindgen_test]
async fn a_view_in_the_first_mount_that_grows_moves_the_caret_below_it() {
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; top: 0; left: 0; width: 500px; font-family: monospace; \
         font-size: 16px; line-height: 24px; padding: 20px; background: white;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();

    let handle = create_editor();
    assert!(handle.load_html(r#"<p>ab</p><div data-pm-type="embed" data-x="1"></div><p>cd</p>"#));
    let grow: std::rc::Rc<std::cell::Cell<Option<Signal<u32>>>> = Default::default();
    let g = grow.clone();
    handle
        .register_node_view(NodeViewSpec::new(
            "embed",
            move |s: &mut RenderScope, _ctx| {
                let height = Signal::new(20u32);
                g.set(Some(height));
                let root = s.create_element("div");
                s.create_effect({
                    let root = root.clone();
                    move || root.set_style("height", &format!("{}px", height.get()))
                });
                root
            },
        ))
        .keep();
    let h = handle.clone();
    // The page's first mount: nothing has installed the editor's page setup yet.
    let _root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| h.mount(scope),
    );
    // `ab` 0..4, the embed 4..5, `cd` 5..9: the caret in `cd`, below the view.
    handle.focus();
    handle.set_selection(Selection::cursor(Pos(7)));
    // Settled: the selection's pass and the first `ResizeObserver` delivery.
    for _ in 0..3 {
        frame().await;
    }
    let before = caret_top(&host);
    assert!(before > 50.0, "the caret is below the view: {before}");

    grow.get().unwrap().set(120);
    // Wait on the caret, up to a deadline: the delivery comes at the browser's
    // next rendering opportunity, whenever that is.
    let start = js_sys::Date::now();
    let mut after = caret_top(&host);
    while (after - before - 100.0).abs() >= 1.0 && js_sys::Date::now() - start < 2_000.0 {
        frame().await;
        after = caret_top(&host);
    }
    assert!(
        (after - before - 100.0).abs() < 1.0,
        "the caret followed the first mount's view as it grew: {before} -> {after}"
    );
}
