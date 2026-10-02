//! A menu chord with no Ctrl/Cmd/Alt yields its key to a focused text field
//! (issue #1169) — the browser half of
//! `rinch::app::menu_chord_text_focus_1169_tests`.
//!
//! The menu bar's chords run from a `window`-capture `keydown` listener that
//! calls `preventDefault` on a match, so a bare `"/"` item used to cancel the
//! `/` typed into every `<input>` on the page. `defaultPrevented` on a
//! synthetic, cancelable `keydown` is the observable for "the menu took the
//! key", and a counter behind the item's `on_click` for "the item ran".
//!
//! Its own file — its own page — because it arms bare chords, which the
//! `Ctrl+Alt` chords of `menu_bar_shortcuts.rs` were chosen never to collide
//! with.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{Menu, MenuItem, RootHandle};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Dispatch a cancelable keydown at `target` (the focused element, where a
/// real keystroke is dispatched) and answer whether anything prevented it.
fn press_at(target: &web_sys::Element, code: &str, key: &str, ctrl: bool, shift: bool) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_code(code);
    init.set_key(key);
    init.set_ctrl_key(ctrl);
    init.set_shift_key(shift);
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    target.dispatch_event(&event).unwrap();
    event.default_prevented()
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    /// One counter per chord, in the order given to [`Fixture::mount`].
    fired: Vec<Rc<Cell<u32>>>,
}

impl Fixture {
    fn mount(chords: &[&'static str]) -> Self {
        let host = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let mut menu = Menu::new();
        let mut fired = Vec::new();
        for chord in chords {
            let count = Rc::new(Cell::new(0u32));
            let c = count.clone();
            menu = menu.item(
                MenuItem::new(*chord)
                    .shortcut(*chord)
                    .on_click(move || c.set(c.get() + 1)),
            );
            fired.push(count);
        }
        let root = rinch_web::mount_into_with_menu_bar(
            &host,
            ThemeProviderProps::default(),
            vec![("File", menu)],
            move |scope: &mut RenderScope| {
                let div = scope.create_element("div");
                for (tag, ty) in [
                    ("input", None),
                    ("textarea", None),
                    ("input", Some("checkbox")),
                    ("input", Some("search")),
                ] {
                    let el = scope.create_element(tag);
                    if let Some(ty) = ty {
                        el.set_attribute("type", ty);
                    }
                    el.set_attribute("data-test-field", ty.unwrap_or(tag));
                    div.append_child(&el);
                }
                let editable = scope.create_element("div");
                editable.set_attribute("contenteditable", "true");
                editable.set_attribute("data-test-field", "editable");
                div.append_child(&editable);
                let plain = scope.create_element("div");
                plain.set_attribute("tabindex", "0");
                plain.set_attribute("data-test-field", "plain");
                div.append_child(&plain);
                div
            },
        );
        Self { root, host, fired }
    }

    fn field(&self, name: &str) -> web_sys::HtmlElement {
        self.host
            .query_selector(&format!("[data-test-field='{name}']"))
            .unwrap()
            .unwrap_or_else(|| panic!("field {name}"))
            .dyn_into()
            .unwrap()
    }

    /// Focus `name` and press a key at it; answers `defaultPrevented`.
    fn press_in(&self, name: &str, code: &str, key: &str, ctrl: bool, shift: bool) -> bool {
        let field = self.field(name);
        field.focus().unwrap();
        assert_eq!(
            document().active_element().as_ref(),
            Some(field.as_ref() as &web_sys::Element),
            "precondition: {name} has focus"
        );
        press_at(&field, code, key, ctrl, shift)
    }

    fn counts(&self) -> Vec<u32> {
        self.fired.iter().map(|c| c.get()).collect()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

#[wasm_bindgen_test]
fn a_bare_slash_is_left_to_a_focused_input_and_fires_from_elsewhere() {
    let fixture = Fixture::mount(&["/"]);

    assert!(
        !fixture.press_in("input", "Slash", "/", false, false),
        "the menu must not cancel the `/` typed into the field"
    );
    assert_eq!(fixture.counts(), vec![0]);

    // Positive control: the listener is armed and the chord is live — with
    // focus on a non-text control it runs the item and takes the key.
    assert!(fixture.press_in("plain", "Slash", "/", false, false));
    assert_eq!(fixture.counts(), vec![1]);
    fixture.teardown();
}

#[wasm_bindgen_test]
fn every_text_field_kind_keeps_bare_and_shifted_keys() {
    let fixture = Fixture::mount(&["N", "Shift+/", "Backspace"]);
    for name in ["input", "textarea", "search", "editable"] {
        assert!(!fixture.press_in(name, "KeyN", "n", false, false), "{name}");
        assert!(!fixture.press_in(name, "Slash", "?", false, true), "{name}");
        assert!(
            !fixture.press_in(name, "Backspace", "Backspace", false, false),
            "{name}"
        );
    }
    assert_eq!(fixture.counts(), vec![0, 0, 0]);
    fixture.teardown();
}

#[wasm_bindgen_test]
fn a_ctrl_chord_a_function_key_and_a_checkbox_still_take_the_key() {
    let fixture = Fixture::mount(&["Ctrl+/", "F5", "/"]);
    assert!(fixture.press_in("input", "Slash", "/", true, false));
    assert!(fixture.press_in("textarea", "F5", "F5", false, false));
    // A checkbox has no text to type `/` into.
    assert!(fixture.press_in("checkbox", "Slash", "/", false, false));
    assert_eq!(fixture.counts(), vec![1, 1, 1]);
    fixture.teardown();
}
