//! A DOM point beside an inline leaf — a hard break (`<br>`) or an image —
//! resolves to the model position on the side the DOM says (issue #1025).
//!
//! A leaf is one model position wide and zero text bytes wide, so the flat
//! byte offset the web used to hand `EditorHandle::pos_at` named both sides of
//! it at once and resolved to the one BEFORE. A click at the start of the line
//! after a Shift+Enter put the caret before the break, at the end of the line
//! above; Home there did the same.
//!
//! The oracle is Chrome's own answer to "where is this point": each fixture
//! asserts, as a positive control, that `caretRangeFromPoint` names a DOM
//! point on the expected side of the leaf (a text node after it, or one
//! before it), and then that the editor's head is that side's model position.
//! Chrome 153's native `contenteditable` puts its caret at that same DOM point.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_leaf_dom_point
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-1025";
/// A 1x1 transparent GIF, stretched by the inline style.
const GIF: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

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
    fn key(&self, k: &str) {
        let cap = document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap();
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(k);
        init.set_code(k);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        cap.dispatch_event(&ev).unwrap();
    }
    /// The vertical middle of the caret overlay's drawn rect, in client px.
    fn caret_mid_y(&self) -> f64 {
        let r = self
            .handle
            .caret_rect(self.handle.selection().head())
            .expect("caret rect");
        r.y as f64 + r.height as f64 / 2.0
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

/// `caretRangeFromPoint`'s start, the DOM point Chrome resolves `(x, y)` to.
fn dom_point(x: f64, y: f64) -> (web_sys::Node, u32) {
    let func: js_sys::Function = js_sys::Reflect::get(&document(), &"caretRangeFromPoint".into())
        .unwrap()
        .dyn_into()
        .unwrap();
    let range: web_sys::Range = func
        .call2(&document(), &x.into(), &y.into())
        .unwrap()
        .dyn_into()
        .unwrap();
    (range.start_container().unwrap(), range.start_offset().unwrap())
}

const BR: &str = "<p>alpha bravo<br>charlie delta</p>";

/// A press on the left edge of line 2's first character, after a hard break,
/// lands AFTER the break (char 12) and draws on line 2. At HEAD: char 11,
/// before the break, drawn at the end of line 1.
#[wasm_bindgen_test]
fn a_click_at_the_start_of_the_line_after_a_break_lands_after_it() {
    let f = F::new(BR);
    let c = f.char_rect(1, 0);
    let a = f.char_rect(0, 0);
    assert!(c.y() > a.y() + 10.0, "positive control: two lines");
    let (x, y) = (c.x() + 1.0, c.y() + c.height() / 2.0);
    let (node, off) = dom_point(x, y);
    assert!(
        node == f.text_node(1) && off == 0,
        "oracle: Chrome resolves the point to the start of the text after the <br>"
    );
    f.click(x, y);
    assert_eq!(f.head(), 12, "after the break, not before it");
    assert!(
        f.caret_mid_y() > c.y() && f.caret_mid_y() < c.bottom(),
        "the caret draws on line 2"
    );
    f.done();
}

/// A press on the right edge of line 1's last character, before the break,
/// stays BEFORE it (char 11) — the other side of the same ambiguity.
#[wasm_bindgen_test]
fn a_click_at_the_end_of_the_line_before_a_break_stays_before_it() {
    let f = F::new(BR);
    let o = f.char_rect(0, 10);
    let (x, y) = (o.right() - 1.0, o.y() + o.height() / 2.0);
    let (node, off) = dom_point(x, y);
    assert!(
        node == f.text_node(0) && off == 11,
        "oracle: Chrome resolves the point to the end of the text before the <br>"
    );
    f.at(15);
    f.click(x, y);
    assert_eq!(f.head(), 11);
    f.done();
}

/// Home on the line after a break goes to that line's start, after the break
/// (Chrome: `alpha bravo<br>|charlie`). At HEAD: before the break.
#[wasm_bindgen_test]
fn home_after_a_break_stays_after_it() {
    let f = F::new(BR);
    f.at(15);
    f.key("Home");
    assert_eq!(f.head(), 12);
    f.key("Home");
    assert_eq!(f.head(), 12, "a second Home stays");
    f.done();
}

/// An image is a leaf too: a press on the left edge of the character after
/// it lands after it (char 6), one on the right edge of the character before
/// it stays before it (char 5).
#[wasm_bindgen_test]
fn a_click_beside_an_image_lands_on_the_side_it_was_made() {
    let f = F::new(&format!(
        "<p>alpha<img src=\"{GIF}\" alt=\"\">bravo</p>"
    ));
    // The model keeps no inline style; size the rendered image directly.
    f.block()
        .query_selector("img")
        .unwrap()
        .expect("the image renders as an <img>")
        .set_attribute("style", "width: 24px; height: 16px")
        .unwrap();
    let b = f.char_rect(1, 0);
    let a = f.char_rect(0, 4);
    assert!(
        b.x() > a.right() + 20.0,
        "positive control: the image sits between them"
    );
    let (x, y) = (b.x() + 1.0, b.y() + b.height() / 2.0);
    let (node, off) = dom_point(x, y);
    assert!(
        node == f.text_node(1) && off == 0,
        "oracle: the point is the start of the text after the image"
    );
    f.click(x, y);
    assert_eq!(f.head(), 6, "after the image");
    let (x, y) = (a.right() - 1.0, a.y() + a.height() / 2.0);
    f.click(x, y);
    assert_eq!(f.head(), 5, "before the image");
    f.done();
}

/// Two breaks in a row: a press on line 3 lands after BOTH (char 7); an
/// off-by-one leaf count would stop between them.
#[wasm_bindgen_test]
fn a_click_after_two_breaks_lands_after_both() {
    let f = F::new("<p>alpha<br><br>bravo</p>");
    let b = f.char_rect(1, 0);
    let (x, y) = (b.x() + 1.0, b.y() + b.height() / 2.0);
    f.click(x, y);
    assert_eq!(f.head(), 7);
    f.done();
}
