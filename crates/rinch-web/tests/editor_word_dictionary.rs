//! The editor's word segmentation on the web (#939): with the opt-in
//! `word-dictionary` feature a CJK run is split into words by ICU's
//! dictionary, as on desktop; without it the run is one word. Both halves
//! are here so each build runs the one that applies to it:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_word_dictionary
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown \
//!     --features word-dictionary --test editor_word_dictionary
//! ```
//!
//! The delete arrives as the browser sends it, a `beforeinput`
//! `deleteWordBackward` on the capture textarea, after a genuine press focused
//! the editor (mounted through `rinch_web::mount_into`).
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

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    fn focused(content: &str) -> Self {
        let host = document().create_element("div").unwrap();
        host.set_attribute(
            "style",
            "font-size: 16px; line-height: 24px; padding: 20px;",
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
        self.host
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("an editor")
    }

    fn text(&self) -> String {
        self.editor_el()
            .query_selector("p")
            .unwrap()
            .expect("a paragraph")
            .text_content()
            .unwrap_or_default()
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    fn before_input(&self, input_type: &str) -> bool {
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type(input_type);
        let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
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

/// `漢字です テスト`: "漢字です" is 1..5; the caret after "です", at 5.
fn deleted_word_before_desu() -> String {
    let f = Fixture::focused("<p>漢字です テスト</p>");
    f.handle.set_selection(Selection::cursor(Pos(5)));
    assert!(f.before_input("deleteWordBackward"), "the editor takes it");
    let text = f.text();
    f.teardown();
    text
}

/// With the dictionary: "です" goes, "漢字" stays (#939's Chrome 153 row).
#[cfg(feature = "word-dictionary")]
#[wasm_bindgen_test]
fn the_dictionary_splits_a_cjk_run() {
    assert_eq!(deleted_word_before_desu(), "漢字 テスト");
}

/// Without it (the default build): the run is one word.
#[cfg(not(feature = "word-dictionary"))]
#[wasm_bindgen_test]
fn without_the_dictionary_a_cjk_run_is_one_word() {
    assert_eq!(deleted_word_before_desu(), " テスト");
}
