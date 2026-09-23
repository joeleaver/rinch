//! Browser-driven tests for the editor's paste hook (`Plugin::handle_paste`) on
//! the web: a real `paste` `ClipboardEvent` on the capture textarea reaches the
//! app's plugin through `EditorHandle::paste`, the entry point desktop's Ctrl+V
//! shares.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_paste_hook
//! ```
//!
//! Every fixture mounts a real editor through `rinch_web::mount_into` and focuses
//! it with a genuine `mousedown`, proving the capture textarea holds focus
//! before anything is pasted.
#![cfg(target_arch = "wasm32")]

use std::cell::Cell;
use std::rc::Rc;

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Mark, PasteContent, Plugin, PluginKey, Pos, Selection,
    Transaction,
};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-paste-hook";

/// "the docs" is 5..13.
const CONTENT: &str = "<p>see the docs here</p>";
const LINKED: &str = "<p>see <a href=\"https://example.test/\">the docs</a> here</p>";

fn words() -> Selection {
    Selection::text(Pos(5), Pos(13))
}

fn is_url(text: &str) -> bool {
    ["https://", "http://", "pimble:"]
        .iter()
        .any(|scheme| text.starts_with(scheme))
        && !text.contains(char::is_whitespace)
}

/// A pasted URL links the selected words, or at a caret inserts linked text
/// (a note's title for `pimble:`, the URL for the web). Counts its offers.
struct Linker(Rc<Cell<u32>>);

impl Plugin for Linker {
    fn key(&self) -> PluginKey {
        PluginKey("test.linker")
    }

    fn handle_paste(&self, state: &EditorState, paste: &PasteContent) -> Option<Transaction> {
        self.0.set(self.0.get() + 1);
        let url = paste.text.as_deref().map(str::trim).filter(|t| is_url(t))?;
        let link = Mark::new(
            state.schema().mark_type("link")?.clone(),
            Attrs::from_iter([("href", AttrValue::from(url.to_string()))]),
        );
        let (from, to) = (state.selection.from().0, state.selection.to().0);
        let mut tr = state.tr();
        if from < to {
            tr.add_mark(from, to, link).ok()?;
        } else {
            let label = if url.starts_with("pimble:") {
                "Linked note"
            } else {
                url
            };
            let text = state.schema().text_with_marks(label, vec![link]).ok()?;
            let len = text.text_len();
            tr.replace_with(from, from, Fragment::from_node(text))
                .ok()?;
            tr.set_selection(Selection::cursor(Pos(from + len)));
        }
        Some(tr)
    }
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    offered: Rc<Cell<u32>>,
}

impl Fixture {
    /// One editor over [`CONTENT`], with the `Linker` unless `plugin` is false,
    /// focused by a real press.
    fn focused(plugin: bool) -> Self {
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
        let offered = Rc::new(Cell::new(0));
        let handle = create_editor();
        if plugin {
            assert!(handle.add_plugin(Rc::new(Linker(offered.clone()))));
        }
        assert!(handle.load_html(CONTENT));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        let f = Self {
            root,
            host,
            handle,
            offered,
        };
        f.focus();
        f
    }

    fn focus(&self) {
        let para = self
            .host
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("the paragraph");
        let text = para.first_child().expect("its text");
        let range = document().create_range().unwrap();
        range.set_start(&text, 1).unwrap();
        range.set_end(&text, 2).unwrap();
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

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// The model, serialized.
    fn html(&self) -> String {
        node_to_html(&self.handle.doc())
    }

    /// The first `<a>` in the page's editor, as `(href, text)`.
    fn page_link(&self) -> Option<(String, String)> {
        let a = self.host.query_selector("[data-pm-editor] a").unwrap()?;
        Some((
            a.get_attribute("href").unwrap_or_default(),
            a.text_content().unwrap_or_default(),
        ))
    }

    /// A `paste` on the capture textarea carrying `text/plain` and, if given,
    /// `text/html`, as the browser fires it. Answers whether the editor took
    /// the event (`preventDefault`).
    fn paste(&self, plain: &str, html: Option<&str>) -> bool {
        let dt = web_sys::DataTransfer::new().unwrap();
        dt.set_data("text/plain", plain).unwrap();
        if let Some(html) = html {
            dt.set_data("text/html", html).unwrap();
        }
        let init = web_sys::ClipboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_clipboard_data(Some(&dt));
        let ev = web_sys::ClipboardEvent::new_with_event_init_dict("paste", &init).unwrap();
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

fn keydown(f: &Fixture, key: &str, code: &str, ctrl: bool) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    init.set_ctrl_key(ctrl);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    f.capture().dispatch_event(&ev).unwrap();
}

#[wasm_bindgen_test]
fn a_pasted_url_over_selected_words_links_them() {
    let f = Fixture::focused(true);
    f.handle.set_selection(words());
    assert!(f.paste("https://example.test/", None));
    assert_eq!(f.html(), LINKED);
    assert_eq!(
        f.page_link(),
        Some(("https://example.test/".into(), "the docs".into())),
        "the page shows the link"
    );
    assert_eq!(f.offered.get(), 1);
    f.teardown();

    // Control: without the plugin the URL replaces the words.
    let f = Fixture::focused(false);
    f.handle.set_selection(words());
    assert!(f.paste("https://example.test/", None));
    assert_eq!(f.html(), "<p>see https://example.test/ here</p>");
    f.teardown();
}

/// A browser's copy of a link carries html and text; the plugin reads the text,
/// and unclaimed, the html is what goes in.
#[wasm_bindgen_test]
fn html_with_a_url_beside_it_shows_the_plugin_the_text() {
    let html = "<a href=\"https://example.test/\">Example</a>";
    let f = Fixture::focused(true);
    f.handle.set_selection(words());
    assert!(f.paste("https://example.test/", Some(html)));
    assert_eq!(f.html(), LINKED);
    f.teardown();

    let f = Fixture::focused(false);
    f.handle.set_selection(words());
    assert!(f.paste("https://example.test/", Some(html)));
    assert_eq!(
        f.html(),
        "<p>see <a href=\"https://example.test/\">Example</a> here</p>"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_pasted_url_at_the_caret_inserts_the_linked_text() {
    let f = Fixture::focused(true);
    f.handle.set_selection(Selection::cursor(Pos(5)));
    assert!(f.paste("pimble:1234/5678", None));
    assert_eq!(
        f.html(),
        "<p>see <a href=\"pimble:1234/5678\">Linked note</a>the docs here</p>"
    );
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(16)));
    f.teardown();
}

#[wasm_bindgen_test]
fn text_the_plugin_does_not_claim_gets_the_default() {
    let f = Fixture::focused(true);
    f.handle.set_selection(words());
    assert!(f.paste("plain words", None));
    assert_eq!(f.html(), "<p>see plain words here</p>");
    assert_eq!(f.offered.get(), 1, "offered, declined");
    f.teardown();
}

#[wasm_bindgen_test]
fn ctrl_z_takes_a_claimed_paste_back_in_one_step() {
    let f = Fixture::focused(true);
    f.handle.set_selection(words());
    assert!(f.paste("https://example.test/", None));
    assert_eq!(f.html(), LINKED);
    keydown(&f, "z", "KeyZ", true);
    assert_eq!(f.html(), CONTENT);
    f.teardown();
}

/// A paste event that reaches a read-only editor (its capture field is
/// `readonly`, so a browser should send none) changes nothing, the plugin's
/// claim included.
#[wasm_bindgen_test]
fn a_read_only_editor_is_unaffected_by_the_hook() {
    let f = Fixture::focused(true);
    f.handle.set_selection(words());
    f.handle.set_read_only(true);
    f.paste("https://example.test/", None);
    f.paste("plain words", None);
    assert_eq!(f.html(), CONTENT);
    assert_eq!(f.page_link(), None);
    // Control: writable again, the same paste lands.
    f.handle.set_read_only(false);
    assert!(f.paste("https://example.test/", None));
    assert_eq!(f.html(), LINKED);
    f.teardown();
}
