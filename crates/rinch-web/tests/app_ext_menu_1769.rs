//! `rinch_web::AppExt` with a menu: `App::menu` mounts the app under the DOM
//! menu bar, as `rinch_web::mount_with_menu_bar` does (issue #1769). An app
//! that names no title leaves the page's own `<title>` alone.
#![cfg(target_arch = "wasm32")]

use rinch::App;
use rinch::menu::{Menu, MenuItem};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_web::AppExt as _;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn app(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("id", "app-ext-menu-1769");
    root
}

#[wasm_bindgen_test]
fn run_with_a_menu_mounts_under_the_menu_bar_and_keeps_the_page_title() {
    document().set_title("Page title 1769");

    App::new(app)
        .menu(vec![
            ("File", Menu::new().item(MenuItem::new("New"))),
            ("Edit", Menu::new().item(MenuItem::new("Undo"))),
        ])
        .run();

    // Positive control: the app mounted.
    let el = document()
        .get_element_by_id("app-ext-menu-1769")
        .expect("run mounted the app");
    let wrapper = el
        .closest(".rinch-app-menu-bar-wrapper")
        .unwrap()
        .expect("the app sits inside the menu bar's wrapper");
    let bar = wrapper
        .query_selector(".rinch-app-menu-bar")
        .unwrap()
        .expect("the bar is drawn");
    let labels = bar.text_content().unwrap_or_default();
    assert!(
        labels.contains("File") && labels.contains("Edit"),
        "both top-level menus are in the bar: {labels:?}"
    );
    assert_eq!(document().title(), "Page title 1769");
}
