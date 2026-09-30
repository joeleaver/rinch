//! A paragraph ending in a hard break, on the web (#1172).
//!
//! The browser lays `<p>ab<br></p>` out as ONE line — a line box after a
//! block's last forced break exists only when something follows it — so the
//! caret after a trailing Shift+Enter had no line to sit on. The view now
//! renders a trailing-break placeholder (a second, unmodelled `<br>`, as
//! ProseMirror's `ProseMirror-trailingBreak`), which makes the line; and the
//! web answers the caret after a `<br>` from what follows it
//! (`WebDocument::br_caret_viewport_rect`), since a `<br>` has no bytes in the
//! browser and the byte query drew that caret at the next *character*: the
//! end of line 1 after a trailing break, and line 3 after the first of two.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_trailing_break
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-1172";

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

impl F {
    /// The paragraph's line box height: its computed `line-height` (the
    /// editor's stylesheet sets its own).
    fn line(&self) -> f64 {
        let cs = web_sys::window()
            .unwrap()
            .get_computed_style(&self.block())
            .unwrap()
            .unwrap();
        let lh = cs.get_property_value("line-height").unwrap();
        lh.trim_end_matches("px").parse().expect("a px line-height")
    }
    /// The caret rect's top, relative to the block's top, for char `i`.
    fn caret_top(&self, i: usize) -> f64 {
        let r = self.handle.caret_rect(Pos(i + 1)).expect("caret rect");
        r.y as f64 - self.block().get_bounding_client_rect().y()
    }
    fn placeholders(&self) -> u32 {
        self.block()
            .query_selector_all("br[data-pm-trailing-break]")
            .unwrap()
            .length()
    }
}

/// `ab<br>` shows two lines, and the caret after the break is on the second.
#[wasm_bindgen_test]
fn a_paragraph_ending_in_a_break_shows_its_empty_line_and_the_caret_is_on_it() {
    let f = F::new("<p>ab<br></p>");
    let line = f.line();
    assert_eq!(f.placeholders(), 1);
    let h = f.block().get_bounding_client_rect().height();
    assert!((h - 2.0 * line).abs() < 0.5, "two lines, got {h}");
    assert!(f.caret_top(2) < line, "before the break: line 1");
    let top = f.caret_top(3);
    assert!(
        (line..2.0 * line).contains(&top),
        "after the break: line 2, got {top}"
    );
    // A press on the empty line lands after the break.
    let a = f.char_rect(0, 0);
    f.at(0);
    f.click(
        a.x() + 100.0,
        f.block().get_bounding_client_rect().y() + 1.5 * line,
    );
    assert_eq!(f.head(), 3);
    // The drawn caret overlay (placed from the layout-local query) is there too.
    let caret = document()
        .query_selector("[data-pm-editor] [data-pm-caret]")
        .unwrap()
        .expect("a caret overlay")
        .get_bounding_client_rect();
    let top = caret.y() - f.block().get_bounding_client_rect().y();
    assert!(
        (line..2.0 * line).contains(&top),
        "the drawn caret is on line 2, got {top}"
    );
    // Typing there: the text takes the line and the placeholder goes.
    f.handle.insert_text("x");
    assert_eq!(f.placeholders(), 0);
    let h = f.block().get_bounding_client_rect().height();
    assert!((h - 2.0 * line).abs() < 0.5, "still two lines, got {h}");
    f.done();
}

/// The first of two breaks: the caret after it is on the empty line 2, not
/// on line 3 at the next character.
#[wasm_bindgen_test]
fn the_caret_after_the_first_of_two_breaks_is_on_the_line_between() {
    let f = F::new("<p>ab<br><br>cd</p>");
    let line = f.line();
    assert_eq!(f.placeholders(), 0, "the paragraph does not end in a break");
    let top = f.caret_top(3);
    assert!(
        (line..2.0 * line).contains(&top),
        "between the breaks: line 2, got {top}"
    );
    let top = f.caret_top(4);
    assert!(
        (2.0 * line..3.0 * line).contains(&top),
        "after both: line 3, got {top}"
    );
    f.done();
}

/// Control: after a break followed by text, the caret is at that text's start
/// on line 2, as it always was.
#[wasm_bindgen_test]
fn the_caret_after_a_break_before_text_is_at_the_texts_start() {
    let f = F::new("<p>ab<br>cd</p>");
    let line = f.line();
    let top = f.caret_top(3);
    assert!((line..2.0 * line).contains(&top), "line 2, got {top}");
    let c = f.char_rect(1, 0);
    let x = f.handle.caret_rect(Pos(4)).unwrap().x as f64;
    assert!(
        (x - c.x()).abs() < 1.0,
        "at `c`'s left edge: {x} vs {}",
        c.x()
    );
    f.done();
}
