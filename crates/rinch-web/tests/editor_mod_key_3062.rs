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
    alt: bool,
    /// `getModifierState("AltGraph")`, as Windows reports an AltGr press.
    alt_graph: bool,
}

const CTRL: Mods = Mods {
    ctrl: true,
    meta: false,
    shift: false,
    alt: false,
    alt_graph: false,
};
const META: Mods = Mods {
    ctrl: false,
    meta: true,
    shift: false,
    alt: false,
    alt_graph: false,
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
        Self::focused_on(Some(mac))
    }

    /// [`focused`](Self::focused), the platform `Some` override or, for
    /// `None`, the browser's own answer.
    fn focused_on(mac: Option<bool>) -> Self {
        rinch_web::__set_mac_platform(mac);
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
        init.set_alt_key(mods.alt);
        init.set_modifier_alt_graph(mods.alt_graph);
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

/// Give `navigator.platform` the answer `platform` on this page (an own
/// property shadowing the prototype's getter); `None` takes it away again.
fn fake_platform(platform: Option<&str>) {
    let navigator: wasm_bindgen::JsValue = web_sys::window().unwrap().navigator().into();
    let navigator = js_sys::Object::from(navigator);
    match platform {
        Some(p) => {
            let descriptor = js_sys::Object::new();
            let getter = js_sys::Function::new_no_args(&format!("return {p:?};"));
            js_sys::Reflect::set(&descriptor, &"get".into(), &getter).unwrap();
            js_sys::Reflect::set(&descriptor, &"configurable".into(), &true.into()).unwrap();
            js_sys::Object::define_property(&navigator, &"platform".into(), &descriptor);
        }
        None => {
            js_sys::Reflect::delete_property(&navigator, &"platform".into()).unwrap();
        }
    }
}

/// iOS and iPadOS are Apple platforms too (ProseMirror's
/// `/Mac|iP(hone|[oa]d)/`): a hardware keyboard's Cmd+B bolds there and
/// Ctrl+B does not. Asked of the browser itself, no override.
#[wasm_bindgen_test]
fn on_ios_and_ipados_cmd_is_mod() {
    for platform in ["iPhone", "iPad"] {
        fake_platform(Some(platform));
        let read_back = web_sys::window().unwrap().navigator().platform().unwrap();
        let f = Fixture::focused_on(None);
        let ctrl = f.key("b", "KeyB", CTRL);
        let after_ctrl = f.html();
        let cmd = f.key("b", "KeyB", META);
        let after_cmd = f.html();
        f.teardown();
        fake_platform(None);
        assert_eq!(read_back, platform, "positive control: the platform took");
        assert!(!ctrl, "{platform}: Ctrl+B is not the editor's");
        assert_eq!(after_ctrl, "<p>abc</p>", "{platform}: Ctrl+B bolds nothing");
        assert!(cmd, "{platform}: Cmd+B is the editor's");
        assert_eq!(after_cmd, "<p><strong>abc</strong></p>", "{platform}");
    }
}

/// AltGr on Windows arrives as Ctrl+Alt with `AltGraph` held (#1291): an
/// AltGr press that types a character is text, not the app's Mod-Alt chord.
/// German AltGr+7 types `{` (digits are keyed physically, so it is the
/// `Digit7` key); the app bound Mod-Alt-7 to bold. A real Ctrl+Alt+7, no
/// `AltGraph`, is the chord (the positive control).
#[wasm_bindgen_test]
fn an_alt_graph_character_is_not_a_ctrl_alt_chord() {
    // Start from the fallbacks, whatever an earlier test's keys taught.
    rinch_web::__forget_key_layout();
    let f = Fixture::focused(false);
    f.handle
        .bind_key("Mod-Alt-7", "toggleBold")
        .expect("a base command");
    let alt_gr = Mods {
        alt: true,
        alt_graph: true,
        ..CTRL
    };
    let typed = f.key("{", "Digit7", alt_gr);
    let after_alt_gr = f.html();
    let chord = f.key(
        "7",
        "Digit7",
        Mods {
            alt_graph: false,
            ..alt_gr
        },
    );
    let after_chord = f.html();
    f.teardown();
    assert!(!typed, "the AltGr press is left to the browser to type");
    assert_eq!(after_alt_gr, "<p>abc</p>", "AltGr+7 did not bold");
    assert!(chord, "Ctrl+Alt+7 is the app's chord");
    assert_eq!(after_chord, "<p><strong>abc</strong></p>", "and bolds");
}

/// Ctrl+Alt with `AltGraph` held: how Windows reports any AltGr press.
const ALT_GR: Mods = Mods {
    ctrl: true,
    meta: false,
    shift: false,
    alt: true,
    alt_graph: true,
};

/// An AltGr press that types nothing new is the chord, as on desktop
/// (`is_alt_graph_text`: the text must differ from what the key types with
/// no modifier). German and Polish Windows layouts report right-Alt+1 as
/// AltGraph with `key` `"1"`, the digit key's own character; the app's
/// Mod-Alt-1 must run there. A one-character `key` alone used to be taken as
/// typed text, and the chord was dropped.
#[wasm_bindgen_test]
fn an_alt_graph_press_that_types_its_own_key_is_the_chord() {
    // Start from the fallbacks, whatever an earlier test's keys taught.
    rinch_web::__forget_key_layout();
    let f = Fixture::focused(false);
    f.handle
        .bind_key("Mod-Alt-1", "toggleBold")
        .expect("a base command");
    let chord = f.key("1", "Digit1", ALT_GR);
    let after = f.html();
    f.teardown();
    assert!(chord, "right-Alt+1 typing nothing is the app's Mod-Alt-1");
    assert_eq!(after, "<p><strong>abc</strong></p>", "and bolds");
}

/// AltGr+Shift (#3062 review): German AltGr+Shift+1 types nothing, and
/// Chrome on Windows then reports `key` `"!"`, the key's Shift character.
/// Desktop's `text` is empty for that press, so it is the app's chord there
/// (`is_alt_graph_text`); it is the chord here too. Comparing `"!"` against
/// the unshifted `"1"` read it as AltGr text and dropped the chord. The
/// positive control: AltGr+Shift reporting a character the key's Shift level
/// does not type (`"¡"`) is text, left to the browser.
#[wasm_bindgen_test]
fn an_alt_graph_shift_press_reporting_the_shift_character_is_the_chord() {
    rinch_web::__forget_key_layout();
    let f = Fixture::focused(false);
    f.handle
        .bind_key("Mod-Alt-Shift-1", "toggleBold")
        .expect("a base command");
    let alt_gr_shift = Mods {
        shift: true,
        ..ALT_GR
    };
    let typed = f.key("\u{a1}", "Digit1", alt_gr_shift);
    let after_typed = f.html();
    let chord = f.key("!", "Digit1", alt_gr_shift);
    let after_chord = f.html();
    f.teardown();
    assert!(
        !typed,
        "AltGr+Shift+1 typing `¡` is text, left to the browser"
    );
    assert_eq!(after_typed, "<p>abc</p>", "and bolds nothing");
    assert!(chord, "AltGr+Shift+1 reporting `!` is the app's chord");
    assert_eq!(after_chord, "<p><strong>abc</strong></p>", "and bolds");
}

/// What a key types unmodified is learned from the layout's own plain
/// presses, not assumed to be the US one: on a French layout `Digit3` types
/// `"`, so an AltGr press reporting `"` there typed nothing new and is the
/// chord. The positive control: an AltGr press on the same key reporting `#`
/// (which French AltGr+3 types) is text, left to the browser.
#[wasm_bindgen_test]
fn the_unmodified_character_is_the_layouts_not_the_us_one() {
    // Start from the fallbacks, whatever an earlier test's keys taught.
    rinch_web::__forget_key_layout();
    let f = Fixture::focused(false);
    f.handle
        .bind_key("Mod-Alt-3", "toggleBold")
        .expect("a base command");
    let plain = f.key("\"", "Digit3", Mods::default());
    let typed = f.html();
    f.handle.set_selection(Selection::text(Pos(1), Pos(2)));
    let hash = f.key("#", "Digit3", ALT_GR);
    let after_hash = f.html();
    let chord = f.key("\"", "Digit3", ALT_GR);
    let after_chord = f.html();
    f.teardown();
    assert!(
        plain,
        "positive control: the plain press reached the editor"
    );
    assert_eq!(typed, "<p>\"</p>", "and typed over the selection");
    assert!(!hash, "AltGr+3 typing `#` is text, left to the browser");
    assert_eq!(after_hash, "<p>\"</p>", "and bolds nothing");
    assert!(chord, "AltGr+3 reporting the key's own `\"` is the chord");
    assert_eq!(after_chord, "<p><strong>\"</strong></p>", "and bolds");
}

/// The core learns the page's platform as the mount starts, before any key:
/// a component labelling a chord while it renders (a toolbar tooltip, the
/// guide's recipe) spells it for a Mac on a Mac browser. The core's own
/// default on wasm is "not macOS", which it kept until the first editor
/// keydown. The positive control: the same render on a non-Apple platform
/// answers `false`.
#[wasm_bindgen_test]
fn a_chord_label_rendered_at_mount_knows_the_platform() {
    use rinch_editor_core::keymap::{host_is_macos, set_host_is_macos};
    let render_answer = |platform: &str| {
        fake_platform(Some(platform));
        // The opposite of the right answer, so only the mount can fix it.
        set_host_is_macos(!platform.starts_with("Mac"));
        let seen = std::rc::Rc::new(std::cell::Cell::new(None));
        let host = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let record = seen.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                record.set(Some(host_is_macos()));
                scope.create_element("div")
            },
        );
        root.unmount();
        host.remove();
        fake_platform(None);
        seen.get().expect("the build ran")
    };
    let mac = render_answer("MacIntel");
    let linux = render_answer("Linux x86_64");
    rinch_web::__set_mac_platform(None);
    assert!(mac, "a render on a Mac browser labels for macOS");
    assert_eq!(
        rinch_editor_core::KeyBinding::parse("Mod-Shift-s")
            .unwrap()
            .label(mac),
        "⇧⌘S"
    );
    assert!(!linux, "positive control: elsewhere it does not");
}
