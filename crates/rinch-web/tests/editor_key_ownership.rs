//! Browser-driven tests for which keys the rich-text editor owns on the web
//! (issue #271).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_key_ownership
//! ```
//!
//! The editor's `keydown` listener sits on `document` in the capture phase, so
//! it sees every key on the page before anything else does. It used to route a
//! key to the last editor that was *clicked* unless the key's target was a text
//! control — so once an editor had been clicked, Enter and Space on a focused
//! `tabindex` element or button anywhere else were split into the editor's
//! paragraph and stopped, and keyboard activation went inert page-wide. The
//! editor owns a key only while its capture textarea holds focus.
//!
//! The second half: a toolbar command reached with no pointer event (Enter on a
//! focused `tabindex` control, the keyboard's `click` on a `<button>`,
//! `element.click()`) changes the document without any of the input events the
//! caret overlay is refreshed from, so the caret stayed where the edit left it.
//!
//! Every fixture mounts through `rinch_web::mount_into` and focuses the editor
//! with a genuine press, proving the capture textarea holds focus first.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::{EditorHandle, RootHandle, create_editor};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-key-ownership";

const CONTENT: &str = "<p>Hello world one</p><p>Second paragraph here</p>";
const TEXT: &str = "Hello world one|Second paragraph here";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    count: Rc<Cell<u32>>,
}

impl Fixture {
    /// One editor over [`CONTENT`] above three controls — a `tabindex` div, a
    /// `<button>` and a `<select>`. The div and the button share one handler,
    /// which counts and then runs a toolbar-style edit on the editor (types
    /// `XYZ` at the caret).
    fn mount() -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        rinch_web::__reset_activation_state();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(CONTENT));
        let count = Rc::new(Cell::new(0u32));
        let counter = count.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| build(scope, handle, counter),
        );
        Self { root, host, count }
    }

    fn el(&self, id: &str) -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into()
            .unwrap()
    }

    fn editor_el(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("the editor is mounted")
    }

    /// The editor's text as the page shows it, block by block.
    fn text(&self) -> String {
        let ps = self.editor_el().query_selector_all("p").unwrap();
        (0..ps.length())
            .map(|i| ps.item(i).unwrap().text_content().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("|")
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    fn caret_x(&self) -> f64 {
        self.editor_el()
            .query_selector("[data-pm-caret]")
            .unwrap()
            .expect("a caret overlay")
            .get_bounding_client_rect()
            .x()
    }

    /// Focus the editor with a genuine left press on character 3 of its first
    /// paragraph, and prove the capture textarea holds focus.
    fn focus_editor(&self) {
        let para = self.editor_el().query_selector("p").unwrap().unwrap();
        let text = para.first_child().unwrap();
        let range = document().create_range().unwrap();
        range.set_start(&text, 3).unwrap();
        range.set_end(&text, 4).unwrap();
        let r = range.get_bounding_client_rect();
        let (x, y) = (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        );
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(self.capture().as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
    }

    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn build(scope: &mut RenderScope, handle: EditorHandle, count: Rc<Cell<u32>>) -> NodeHandle {
    let wrapper = scope.create_element("div");
    wrapper.append_child(&handle.mount(scope));
    let ed = handle.clone();
    let rid = scope.register_handler(move || {
        count.set(count.get() + 1);
        ed.insert_text("XYZ");
    });
    for (tag, id, tabindex) in [("div", "node", Some("0")), ("button", "button", None)] {
        let el = scope.create_element(tag);
        el.set_attribute("id", id);
        el.set_attribute("data-rid", &rid.0.to_string());
        if let Some(t) = tabindex {
            el.set_attribute("tabindex", t);
        }
        el.append_child(&scope.create_text("act"));
        wrapper.append_child(&el);
    }
    let select = scope.create_element("select");
    select.set_attribute("id", "select");
    let option = scope.create_element("option");
    option.append_child(&scope.create_text("one"));
    select.append_child(&option);
    wrapper.append_child(&select);
    wrapper
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
    mouse_on(&target, name, x, y);
}

fn mouse_on(target: &web_sys::Element, name: &str, x: f32, y: f32) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(1);
    init.set_detail(1);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

fn keydown(el: &web_sys::Element, key: &str, code: &str) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

/// The positive control for everything below: while the capture textarea holds
/// focus, the editor does own Enter — it splits the paragraph and consumes it.
#[wasm_bindgen_test]
fn the_focused_editor_owns_enter() {
    let f = Fixture::mount();
    f.focus_editor();
    let ev = keydown(f.capture().as_ref(), "Enter", "Enter");
    assert!(ev.default_prevented(), "the editor consumed Enter");
    assert_ne!(f.text(), TEXT, "and split its paragraph");
    assert_eq!(f.count.get(), 0);
    f.teardown();
}

/// The issue's case: an editor was clicked, then a `tabindex` control took
/// focus. Enter there is that control's activation, not a paragraph split.
#[wasm_bindgen_test]
fn enter_on_a_focused_control_after_an_editor_click_activates_it() {
    let f = Fixture::mount();
    f.focus_editor();
    let node = f.el("node");
    node.focus().unwrap();
    assert!(document().active_element().as_deref() == Some(node.as_ref()));
    keydown(node.as_ref(), "Enter", "Enter");
    assert_eq!(f.count.get(), 1, "Enter must activate the focused control");
    assert_eq!(
        f.text().replace("XYZ", ""),
        TEXT,
        "and the editor split nothing (the command's own XYZ aside)"
    );
    f.teardown();
}

/// Space, likewise — and the editor must not type it either.
#[wasm_bindgen_test]
fn space_on_a_focused_control_after_an_editor_click_activates_it() {
    let f = Fixture::mount();
    f.focus_editor();
    let node = f.el("node");
    node.focus().unwrap();
    keydown(node.as_ref(), " ", "Space");
    assert_eq!(f.count.get(), 1, "Space must activate the focused control");
    assert_eq!(
        f.text().replace("XYZ", ""),
        TEXT,
        "and the editor typed no space (the command's own XYZ aside)"
    );
    f.teardown();
}

/// A printable key on a focused button is not the editor's either: nothing is
/// typed into a document whose keyboard focus is elsewhere.
#[wasm_bindgen_test]
fn a_key_on_a_focused_button_types_nothing_into_the_editor() {
    let f = Fixture::mount();
    f.focus_editor();
    let button = f.el("button");
    button.focus().unwrap();
    let ev = keydown(button.as_ref(), "q", "KeyQ");
    assert!(!ev.default_prevented(), "the page keeps the key");
    assert_eq!(f.text(), TEXT, "the editor typed nothing");
    f.teardown();
}

/// Enter on a focused `tabindex` toolbar control runs its command, and the
/// caret overlay follows the edit — no pointer event is there to refresh it.
#[wasm_bindgen_test]
fn a_keyboard_activated_command_moves_the_caret_overlay() {
    let f = Fixture::mount();
    f.focus_editor();
    let before = f.caret_x();
    let node = f.el("node");
    node.focus().unwrap();
    keydown(node.as_ref(), "Enter", "Enter");
    assert_eq!(f.count.get(), 1, "positive control: the command ran");
    assert!(f.text().contains("XYZ"), "and edited: {}", f.text());
    let after = f.caret_x();
    assert!(
        after > before + 20.0,
        "the caret moved past the three typed characters ({before} -> {after})"
    );
    f.teardown();
}

/// The same through a pointer-less `click` on a `<button>` — what Enter or
/// Space on a focused button, or assistive technology, produces.
#[wasm_bindgen_test]
fn a_pointerless_click_command_moves_the_caret_overlay() {
    let f = Fixture::mount();
    f.focus_editor();
    let before = f.caret_x();
    f.el("button").click();
    assert_eq!(f.count.get(), 1, "positive control: the command ran");
    assert!(f.text().contains("XYZ"), "and edited: {}", f.text());
    let after = f.caret_x();
    assert!(
        after > before + 20.0,
        "the caret moved past the three typed characters ({before} -> {after})"
    );
    f.teardown();
}

/// A press on a `<select>` moves the keyboard to it, so the editor lets go of
/// the capture textarea — the same text-control rule keyboard activation uses.
#[wasm_bindgen_test]
fn a_press_on_a_select_blurs_the_editor() {
    let f = Fixture::mount();
    f.focus_editor();
    let select = f.el("select");
    let r = select.get_bounding_client_rect();
    let (x, y) = ((r.x() + 5.0) as f32, (r.y() + 5.0) as f32);
    mouse_on(select.as_ref(), "mousedown", x, y);
    assert!(
        document().active_element().as_deref() != Some(f.capture().as_ref()),
        "the capture textarea no longer holds focus"
    );
    f.teardown();
}
