//! Review of PR #1527: checkbox / radio items in the web DOM menu bar.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::Signal;
use rinch_web::{Menu, MenuItem};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn press(code: &str, key: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_code(code);
    init.set_key(key);
    init.set_ctrl_key(true);
    init.set_alt_key(true);
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    document().body().unwrap().dispatch_event(&event).unwrap();
    event.default_prevented()
}

#[wasm_bindgen_test]
fn web_menu_bar_check_marks_follow_their_signals() {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let on = Signal::new(false);
    let model = Signal::new(2u8);
    let menu = Menu::new()
        .item(MenuItem::checkbox("Mute", on).shortcut("Ctrl+Alt+M"))
        .item(MenuItem::radio("One", model, 1))
        .item(MenuItem::radio("Two", model, 2))
        .item(MenuItem::checkbox("Off", Signal::new(true)).enabled(false));
    let root = rinch_web::mount_into_with_menu_bar(
        &host,
        ThemeProviderProps::default(),
        vec![("View", menu)],
        move |scope: &mut RenderScope| scope.create_element("div"),
    );
    let rows: Vec<web_sys::HtmlElement> = {
        let list = host
            .query_selector_all(".rinch-app-menu-entry[aria-checked]")
            .unwrap();
        (0..list.length())
            .map(|i| list.item(i).unwrap().dyn_into().unwrap())
            .collect()
    };
    assert_eq!(rows.len(), 4, "every toggle row carries aria-checked");
    let aria = |i: usize| rows[i].get_attribute("aria-checked").unwrap();
    assert_eq!(rows[0].get_attribute("role").as_deref(), Some("menuitemcheckbox"));
    assert_eq!(rows[1].get_attribute("role").as_deref(), Some("menuitemradio"));
    assert_eq!((aria(0).as_str(), aria(1).as_str(), aria(2).as_str()), ("false", "false", "true"));

    // The mark's visibility comes from the component stylesheet.
    let mark = |i: usize| rows[i].query_selector(".rinch-app-menu-entry__check").unwrap().unwrap();
    let vis = |i: usize| {
        web_sys::window()
            .unwrap()
            .get_computed_style(&mark(i))
            .unwrap()
            .unwrap()
            .get_property_value("visibility")
            .unwrap()
    };
    assert_eq!(vis(0), "hidden");
    assert_eq!(vis(2), "visible");

    // App write.
    on.set(true);
    assert_eq!(aria(0), "true");
    assert_eq!(vis(0), "visible");

    // Click.
    rows[0].click();
    assert!(!on.get(), "a click unchecks");
    assert_eq!(aria(0), "false");
    rows[1].click();
    assert_eq!(model.get(), 1);
    assert_eq!((aria(1).as_str(), aria(2).as_str()), ("true", "false"));

    // Chord.
    assert!(press("KeyM", "m"), "the chord is consumed");
    assert!(on.get(), "the chord toggles");
    assert_eq!(aria(0), "true");

    // Disabled toggle: shown checked, a click does not flip it.
    assert_eq!(aria(3), "true");
    rows[3].click();
    assert_eq!(aria(3), "true", "a disabled toggle does not flip");

    root.unmount();
    host.remove();
}
