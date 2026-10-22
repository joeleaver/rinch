//! `rinch_web::AppExt` runs a `rinch::App` builder chain in the browser
//! (issue #1769): the same `App::new(..).title(..).theme(..).run()` an app
//! writes for the desktop mounts it here as a whole-page app.
//!
//! Its own file — its own page — because `run` mounts under `#rinch-body` for
//! the page's lifetime. The menu-bar half is `app_ext_menu_1769.rs`.
#![cfg(target_arch = "wasm32")]

use rinch::App;
use rinch_core::Signal;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::AppExt as _;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

thread_local! {
    static COUNT: std::cell::Cell<Option<Signal<u32>>> = const { std::cell::Cell::new(None) };
}

fn app(scope: &mut RenderScope) -> NodeHandle {
    let count = Signal::new(1u32);
    COUNT.with(|c| c.set(Some(count)));
    let root = scope.create_element("div");
    root.set_attribute("id", "app-ext-1769");
    let text = scope.create_text("");
    root.append_child(&text);
    scope.create_effect(move || text.set_text(&format!("count {}", count.get())));
    root
}

#[wasm_bindgen_test]
fn run_mounts_the_app_sets_the_title_and_installs_the_theme() {
    let before = document().title();
    assert_ne!(
        before, "Built by App 1769",
        "the page must not start with the title"
    );
    assert!(
        document().get_element_by_id("app-ext-1769").is_none(),
        "nothing is mounted before run"
    );

    App::new(app)
        .title("Built by App 1769")
        .size(640, 480)
        .theme(ThemeProviderProps {
            primary_color: Some("grape".into()),
            ..Default::default()
        })
        .run();

    // Positive control: the app's own element is on the page, under the
    // whole-page root, and it is live — a signal write reaches it.
    let el = document()
        .get_element_by_id("app-ext-1769")
        .expect("run mounted the app");
    assert!(
        el.closest("#rinch-body").unwrap().is_some(),
        "mounted as a whole-page app"
    );
    assert_eq!(el.text_content().as_deref(), Some("count 1"));
    COUNT.with(|c| c.get().unwrap()).set(2);
    assert_eq!(el.text_content().as_deref(), Some("count 2"));

    assert_eq!(document().title(), "Built by App 1769");
    // No menu was configured, so there is no bar.
    assert!(
        document()
            .query_selector(".rinch-app-menu-bar")
            .unwrap()
            .is_none()
    );
    // The theme reached the page's theme <style>.
    let theme = document()
        .query_selector("style[data-rinch-theme]")
        .unwrap()
        .expect("a theme <style>")
        .text_content()
        .unwrap_or_default();
    assert!(
        theme.contains("--rinch-primary-color") && theme.contains("grape"),
        "the configured primary colour is installed"
    );
}
