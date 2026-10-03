//! #1181: the web keeps U+2028, U+2029 and U+0085 as the one character
//! the browser lays out (`DomDocument::substituted_char_flat_bytes` is empty
//! there), so every caret sits at the boundary Chrome's own `Range` puts
//! there, and a press after the character lands after it. From the review of
//! #1269 (round 2), which measured it in Chrome 153; with the desktop table
//! as the web's answer it fails (`caret 2: drawn at 70.31, Chrome's boundary
//! at 53.52`).
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-1181";
fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl F {
    /// One paragraph in a 400px monospace editor, 16px / 24px, focused.
    fn new(html: &str) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST}]")) {
            for i in 0..stale.length() {
                if let Some(n) = stale.item(i) {
                    n.dyn_into::<web_sys::Element>().unwrap().remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 400px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(html));
        let m = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |s: &mut RenderScope| m.mount(s),
        );
        let f = Self { root, host, handle };
        // Focus with a press on the first character, then check a genuine
        // event reached rinch: the capture textarea took focus.
        let r = f.char_rect(0, 0);
        f.click(r.x() + 1.0, r.y() + r.height() / 2.0);
        let cap = document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap();
        assert!(
            document().active_element().as_ref() == Some(&cap),
            "positive control: a press reached rinch and focused the editor"
        );
        f
    }
    fn block(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("block")
    }
    /// The block's `n`th text node (marks aside: these fixtures have none).
    fn text_node(&self, n: usize) -> web_sys::Node {
        let kids = self.block().child_nodes();
        (0..kids.length())
            .filter_map(|i| kids.item(i))
            .filter(|c| c.node_type() == 3)
            .nth(n)
            .expect("text node")
    }
    fn char_rect(&self, node: usize, i: u32) -> web_sys::DomRect {
        let t = self.text_node(node);
        let r = document().create_range().unwrap();
        r.set_start(&t, i).unwrap();
        r.set_end(&t, i + 1).unwrap();
        r.get_bounding_client_rect()
    }
    fn click(&self, x: f64, y: f64) {
        mouse("mousedown", x as f32, y as f32);
        mouse("mouseup", x as f32, y as f32);
    }
    /// The head, as a char offset into the paragraph.
    fn head(&self) -> i64 {
        self.handle.selection().head().0 as i64 - 1
    }
    fn at(&self, i: usize) {
        self.handle.set_selection(Selection::cursor(Pos(i + 1)));
    }
    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document().element_from_point(x, y).expect("hit");
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

/// #1181 / review of #1269 round 2: the web's caret map counts U+2028 as the
/// one character the browser lays out (no substitution), so each caret sits
/// at the boundary Chrome's own Range puts there, and a press after the
/// separator lands after it.
#[wasm_bindgen_test]
fn carets_and_a_press_across_a_line_separator() {
    for sep in ['\u{2028}', '\u{2029}', '\u{85}'] {
        let f = F::new(&format!("<p>a{sep}bc</p>"));
        let text = f.text_node(0).text_content().unwrap();
        assert_eq!(
            text.chars().count(),
            4,
            "positive control: the DOM holds the char {sep:?}"
        );
        let n = text.encode_utf16().count() as u32;
        assert_eq!(n, 4);
        let line_y = f.char_rect(0, 0).y();
        for i in 0..=4usize {
            let want = if i < 4 {
                f.char_rect(0, i as u32).x()
            } else {
                f.char_rect(0, 3).right()
            };
            f.at(i);
            let r = f
                .handle
                .caret_rect(f.handle.selection().head())
                .expect("caret rect");
            assert!(
                (r.x as f64 - want).abs() < 1.5,
                "{sep:?} caret {i}: drawn at {}, Chrome's boundary at {want}",
                r.x
            );
            assert!(
                (r.y as f64 - line_y).abs() < 2.0,
                "{sep:?} caret {i} on the one line"
            );
        }
        // A press on the right edge of b lands after b (char 3).
        let b = f.char_rect(0, 2);
        f.click(b.right() - 1.0, b.y() + b.height() / 2.0);
        assert_eq!(f.head(), 3, "{sep:?}: a press on b's right half is after b");
        let c = f.char_rect(0, 1);
        if c.width() > 2.0 {
            f.click(c.right() - 1.0, c.y() + c.height() / 2.0);
            assert_eq!(
                f.head(),
                2,
                "{sep:?}: a press on the separator's right half is after it"
            );
        }
        f.done();
    }
}
