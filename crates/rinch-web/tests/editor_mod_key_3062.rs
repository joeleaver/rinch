//! #3062 on the web: the keymap's `Mod` is Cmd on macOS and Ctrl elsewhere
//! (ProseMirror's rule, from `navigator.platform`), and Cmd+Y is not redo on
//! macOS. Before, `Mod` was Ctrl **or** Meta on every OS: on a Mac, Ctrl+B
//! (Cocoa's caret-back key) bolded and Ctrl+A selected all; on Windows a
//! Win+key chord that reached the page ran the editor's command.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_mod_key_3062
//! ```
//!
//! Both platforms run in CI's Linux browsers through `__set_mac_platform`.
//! Every fixture mounts through `rinch_web::mount_into` and focuses the editor
//! with a genuine press, checked before anything else. `<p>abc</p>`: the text
//! is at 1..4.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-mod-key-3062";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

/// The modifiers of a key press.
#[derive(Clone, Copy, Default)]
struct Mods {
    ctrl: bool,
    meta: bool,
    shift: bool,
}

const CTRL: Mods = Mods {
    ctrl: true,
    meta: false,
    shift: false,
};
const META: Mods = Mods {
    ctrl: false,
    meta: true,
    shift: false,
};

fn mouse(kind: &str, x: f32, y: f32) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    init.set_button(0);
    init.set_buttons(if kind == "mousedown" { 1 } else { 0 });
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(kind, &init).unwrap();
    let target = document()
        .element_from_point(x, y)
        .expect("an element under the point");
    target.dispatch_event(&ev).unwrap();
}

impl Fixture {
    /// One editor over `<p>abc</p>` with the page taken as macOS or not,
    /// focused by a real press on its first character.
    fn focused(mac: bool) -> Self {
        rinch_web::__set_mac_platform(Some(mac));
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html("<p>abc</p>"));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        let f = Self { root, host, handle };
        let para = document()
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("a paragraph");
        let text = para.first_child().expect("the paragraph has a text node");
        let range = document().create_range().unwrap();
        range.set_start(&text, 0).unwrap();
        range.set_end(&text, 1).unwrap();
        let r = range.get_bounding_client_rect();
        let (x, y) = (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        );
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(f.capture().as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
        f.handle.set_selection(Selection::text(Pos(1), Pos(4)));
        f
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// A `keydown` of `key` / `code` on the capture textarea; answers whether
    /// the editor consumed it (`preventDefault`ed).
    fn key(&self, key: &str, code: &str, mods: Mods) -> bool {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(key);
        init.set_code(code);
        init.set_ctrl_key(mods.ctrl);
        init.set_meta_key(mods.meta);
        init.set_shift_key(mods.shift);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        ev.default_prevented()
    }

    fn html(&self) -> String {
        node_to_html(&self.handle.doc())
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        rinch_web::__set_mac_platform(None);
    }
}

/// On macOS, Cmd is `Mod`: Cmd+B bolds; Ctrl+B is left to the browser (the
/// caret-back key in a Cocoa text view) and bolds nothing.
#[wasm_bindgen_test]
fn on_macos_cmd_is_mod_and_ctrl_is_not() {
    let f = Fixture::focused(true);
    let ctrl = f.key("b", "KeyB", CTRL);
    let after_ctrl = f.html();
    let cmd = f.key("b", "KeyB", META);
    let after_cmd = f.html();
    f.teardown();
    assert!(!ctrl, "Ctrl+B on a Mac is not preventDefaulted");
    assert_eq!(after_ctrl, "<p>abc</p>", "Ctrl+B on a Mac bolds nothing");
    assert!(cmd, "Cmd+B is the editor's");
    assert_eq!(after_cmd, "<p><strong>abc</strong></p>", "Cmd+B bolds");
}

/// Off macOS, Ctrl is `Mod`: Ctrl+B bolds; the Windows (Meta) key with B is
/// left to the browser and the system.
#[wasm_bindgen_test]
fn off_macos_ctrl_is_mod_and_meta_is_not() {
    let f = Fixture::focused(false);
    let meta = f.key("b", "KeyB", META);
    let after_meta = f.html();
    let ctrl = f.key("b", "KeyB", CTRL);
    let after_ctrl = f.html();
    f.teardown();
    assert!(!meta, "Win+B is not preventDefaulted");
    assert_eq!(after_meta, "<p>abc</p>", "Win+B bolds nothing");
    assert!(ctrl, "Ctrl+B is the editor's");
    assert_eq!(after_ctrl, "<p><strong>abc</strong></p>", "Ctrl+B bolds");
}

/// Off macOS, Ctrl+Y redoes, as Chrome's `contenteditable` does there.
#[wasm_bindgen_test]
fn off_macos_ctrl_y_redoes() {
    let f = Fixture::focused(false);
    assert!(f.key("b", "KeyB", CTRL), "positive control: bold");
    assert!(f.key("z", "KeyZ", CTRL), "undo");
    let undone = f.html();
    let redo = f.key("y", "KeyY", CTRL);
    let redone = f.html();
    f.teardown();
    assert_eq!(undone, "<p>abc</p>");
    assert!(redo, "Ctrl+Y is the editor's");
    assert_eq!(redone, "<p><strong>abc</strong></p>", "Ctrl+Y redoes");
}

/// On macOS Cmd+Y opens History in Chrome, and redo is Cmd+Shift+Z only: Cmd+Y
/// is left to the browser and redoes nothing.
#[wasm_bindgen_test]
fn on_macos_cmd_y_is_not_redo_and_cmd_shift_z_is() {
    let f = Fixture::focused(true);
    assert!(f.key("b", "KeyB", META), "positive control: bold");
    assert!(f.key("z", "KeyZ", META), "undo");
    let undone = f.html();
    let cmd_y = f.key("y", "KeyY", META);
    let after_cmd_y = f.html();
    let shift_z = f.key(
        "z",
        "KeyZ",
        Mods {
            shift: true,
            ..META
        },
    );
    let redone = f.html();
    f.teardown();
    assert_eq!(undone, "<p>abc</p>");
    assert!(!cmd_y, "Cmd+Y is not preventDefaulted");
    assert_eq!(after_cmd_y, "<p>abc</p>", "Cmd+Y redoes nothing");
    assert!(shift_z, "Cmd+Shift+Z is the editor's");
    assert_eq!(redone, "<p><strong>abc</strong></p>", "Cmd+Shift+Z redoes");
}
