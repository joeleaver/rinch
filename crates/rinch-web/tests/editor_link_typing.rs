//! Browser-driven test: a key typed right after a link in the web editor is not
//! part of the link (`link` is a non-inclusive mark), and one typed inside it is.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_link_typing
//! ```
//!
//! The model rule has its own tests in `rinch-editor-core`; this one drives the
//! browser's real typing path (a `keydown` on the focused capture textarea) and
//! reads the result back from the page's own `<a>`.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-link-typing";

/// "see " is 1..5, the link "here" 5..9, " now" 9..13.
const CONTENT: &str = r#"<p>see <a href="https://rust-lang.org">here</a> now</p>"#;

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
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
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        Self { root, host, handle }
    }

    fn editor_el(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("the editor is mounted")
    }

    fn capture(&self) -> web_sys::Element {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
    }

    /// Focus the editor with a genuine left press on its first character.
    fn focus_editor(&self) {
        let para = self.editor_el().query_selector("p").unwrap().unwrap();
        let text = para.first_child().unwrap();
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
            document().active_element().as_ref() == Some(&self.capture()),
            "positive control: a left press must focus the capture textarea"
        );
    }

    /// Type `key` at `pos` through the browser's key path.
    fn type_at(&self, pos: usize, key: &str) {
        self.handle.set_selection(Selection::cursor(Pos(pos)));
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(key);
        init.set_code(&format!("Key{}", key.to_uppercase()));
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        assert!(ev.default_prevented(), "the editor typed the key");
    }

    /// The page's paragraph text and the text of its one `<a>`.
    fn page(&self) -> (String, String) {
        let p = self.editor_el().query_selector("p").unwrap().unwrap();
        let anchors = p.query_selector_all("a").unwrap();
        assert_eq!(anchors.length(), 1, "one link on the page");
        let a = anchors.item(0).unwrap();
        (
            p.text_content().unwrap_or_default(),
            a.text_content().unwrap_or_default(),
        )
    }

    fn teardown(self) {
        rinch_web::__reset_activation_state();
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

/// A key typed right after the link's last letter lands after the `<a>`, and
/// the caret there reports no link; one typed inside it lands in the `<a>`.
#[wasm_bindgen_test]
fn a_key_typed_after_a_link_is_outside_it_and_one_inside_is_in_it() {
    let f = Fixture::mount();
    f.focus_editor();

    f.type_at(9, "x");
    assert_eq!(
        f.page(),
        ("see herex now".to_string(), "here".to_string()),
        "typed right after the link: outside it"
    );
    assert_eq!(f.handle.active_link_href(), None);

    f.type_at(7, "y");
    assert_eq!(
        f.page(),
        ("see heyrex now".to_string(), "heyre".to_string()),
        "typed inside the link: in it"
    );
    assert_eq!(
        f.handle.active_link_href().as_deref(),
        Some("https://rust-lang.org")
    );
    f.teardown();
}
