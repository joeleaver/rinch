//! Word delete on the web (issue #303): a `beforeinput` `deleteWordBackward` /
//! `deleteWordForward` runs the core commands of the same name, so the web gets
//! the rules pinned in `rinch-editor-core` — a block edge joins, an inline atom
//! beside the caret goes on its own — and no chord is bound on a `keydown`: the
//! browser resolves the platform's chord (Ctrl on Windows / Linux, Alt on
//! macOS) into the `beforeinput`, and Cmd+Backspace stays the browser's line
//! delete.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_word_delete
//! ```
//!
//! Every fixture mounts through `rinch_web::mount_into`, focuses with a genuine
//! press and checks the capture textarea holds focus before asserting anything.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-word-delete";

/// A 1x1 transparent PNG, so the image needs no network.
const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    /// One editor over `content`, focused by a real press on the first
    /// paragraph's first character.
    fn focused(content: &str) -> Self {
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
        assert!(handle.load_html(content));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        let f = Self { root, host, handle };
        let para = f
            .editor_el()
            .query_selector("p")
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
        f
    }

    fn editor_el(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("an editor")
    }

    /// The editor's text as the page shows it, block by block.
    fn text(&self) -> String {
        let ps = self.editor_el().query_selector_all("p").unwrap();
        (0..ps.length())
            .map(|i| ps.item(i).unwrap().text_content().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("|")
    }

    fn images(&self) -> u32 {
        self.editor_el().query_selector_all("img").unwrap().length()
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// A `beforeinput` of `input_type` on the capture textarea; answers whether
    /// the editor took it (`preventDefault`ed).
    fn before_input(&self, input_type: &str) -> bool {
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type(input_type);
        let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        ev.default_prevented()
    }

    /// A Backspace `keydown` with the given modifiers; answers whether the
    /// editor consumed it.
    fn backspace(&self, ctrl: bool, alt: bool, meta: bool) -> bool {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key("Backspace");
        init.set_code("Backspace");
        init.set_ctrl_key(ctrl);
        init.set_alt_key(alt);
        init.set_meta_key(meta);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        ev.default_prevented()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
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

/// "Hello world one": the caret in "wor|ld" (10). Backward takes "wor",
/// forward "ld".
#[wasm_bindgen_test]
fn a_word_delete_takes_the_word_beside_the_caret() {
    let f = Fixture::focused("<p>Hello world one</p>");
    f.handle.set_selection(Selection::cursor(Pos(10)));
    assert!(f.before_input("deleteWordBackward"));
    assert_eq!(f.text(), "Hello ld one");
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(7)));
    assert!(f.before_input("deleteWordForward"));
    assert_eq!(f.text(), "Hello  one");
    f.teardown();
}

/// At a paragraph's start a word delete joins, as Backspace does.
#[wasm_bindgen_test]
fn a_word_delete_at_a_block_start_joins() {
    let f = Fixture::focused("<p>ab</p><p>cd</p>");
    f.handle.set_selection(Selection::cursor(Pos(5)));
    assert!(f.before_input("deleteWordBackward"));
    assert_eq!(f.text(), "abcd");
    f.teardown();
}

/// `foo<img>bar`: "foo" 1..4, the image 4..5, "bar" 5..8. Right after the image
/// a word delete takes the image alone — the core's rule. The motion-and-delete
/// the glue used to do counted the image as a space and took "foo" with it.
#[wasm_bindgen_test]
fn a_word_delete_beside_an_image_takes_the_image_alone() {
    let f = Fixture::focused(&format!(r#"<p>foo<img src="{PNG}">bar</p>"#));
    assert_eq!(f.images(), 1, "positive control: the image is on the page");
    f.handle.set_selection(Selection::cursor(Pos(5)));
    assert!(f.before_input("deleteWordBackward"));
    assert_eq!(f.images(), 0);
    assert_eq!(f.text(), "foobar");
    f.teardown();
}

/// No word-delete chord is bound on the web's `keydown`: Ctrl+, Alt+ and
/// Cmd+Backspace are left to the browser, which turns the platform's word chord
/// into the `beforeinput` above and Cmd+Backspace into its line delete. A
/// plain Backspace, the control, is the keymap's.
#[wasm_bindgen_test]
fn modified_backspace_is_left_to_the_browser() {
    let f = Fixture::focused("<p>Hello world one</p>");
    f.handle.set_selection(Selection::cursor(Pos(10)));
    for (what, ctrl, alt, meta) in [
        ("Ctrl", true, false, false),
        ("Alt", false, true, false),
        ("Cmd", false, false, true),
    ] {
        assert!(!f.backspace(ctrl, alt, meta), "{what}+Backspace is the browser's");
        assert_eq!(f.text(), "Hello world one", "{what}+Backspace edits nothing");
    }
    assert!(f.backspace(false, false, false), "control: Backspace is consumed");
    assert_eq!(f.text(), "Hello wold one", "control: Backspace deletes a char");
    f.teardown();
}
