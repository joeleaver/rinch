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
    (
        range.start_container().unwrap(),
        range.start_offset().unwrap(),
    )
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
    let f = F::new(&format!("<p>alpha<img src=\"{GIF}\" alt=\"\">bravo</p>"));
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

/// A press on the empty line between two breaks: Chrome resolves it to an
/// ELEMENT point, `(<p>, 2)` — after the first `<br>` — not a text point.
/// Lands between the breaks (char 6). At HEAD the element point matched no
/// text node, so it counted every byte in the block and landed at its end.
#[wasm_bindgen_test]
fn a_click_on_the_empty_line_between_two_breaks_lands_between_them() {
    let f = F::new("<p>alpha<br><br>bravo</p>");
    let a = f.char_rect(0, 0);
    let b = f.char_rect(1, 0);
    assert!(
        b.y() > a.y() + 40.0,
        "positive control: an empty line between them"
    );
    let (x, y) = (a.x() + 30.0, (a.bottom() + b.y()) / 2.0);
    let (node, off) = dom_point(x, y);
    assert!(
        node == f.block().into() && off == 2,
        "oracle: an element point after the first <br>"
    );
    f.click(x, y);
    assert_eq!(f.head(), 6);
    f.done();
}

// ── The capture-textarea mirror: IME commits and autocorrect ─────────────────
//
// Every keyboard edit the web lets the browser make lands in the hidden capture
// textarea and is recovered by diffing it against the mirror of the caret's
// block. The mirror used to be `text_content()`, which has no character for a
// leaf, so a char offset right after a `<br>` named both sides of it, as the
// byte offset did (#1025). On Android every keyboard word is a composition.

impl F {
    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }
    /// The paragraph's content with a hard break as `|` and an image as `#`.
    fn para(&self) -> String {
        let doc = self.handle.doc();
        let p = doc.child(0);
        (0..p.child_count())
            .map(|i| {
                let c = p.child(i);
                match c.text() {
                    Some(t) => t.to_string(),
                    None if c.type_name() == "hard_break" => "|".into(),
                    None => "#".into(),
                }
            })
            .collect()
    }
    /// An IME composes `data` at the textarea's caret and commits it: the
    /// field's value is what the browser leaves there.
    fn compose(&self, data: &str) {
        let ta = self.capture();
        let v = ta.value();
        let at = ta.selection_start().unwrap().unwrap() as usize;
        let units: Vec<u16> = v.encode_utf16().collect();
        let now = String::from_utf16(&units[..at]).unwrap()
            + data
            + &String::from_utf16(&units[at..]).unwrap();
        composition(&ta, "compositionstart", "");
        composition(&ta, "compositionupdate", data);
        ta.set_value(&now);
        let end = (at + data.encode_utf16().count()) as u32;
        ta.set_selection_range(end, end).unwrap();
        composition(&ta, "compositionend", data);
    }
}

fn composition(ta: &web_sys::HtmlTextAreaElement, name: &str, data: &str) {
    let init = web_sys::CompositionEventInit::new();
    init.set_bubbles(true);
    init.set_data(data);
    let ev = web_sys::CompositionEvent::new_with_event_init_dict(name, &init).unwrap();
    ta.dispatch_event(&ev).unwrap();
}

/// A composition committed at the start of the line after a break lands after
/// it: `alpha<br>Xbravo`. At #1101's first head: `alphaX<br>bravo`. The mirror
/// gives the break a character of its own, so the IME sees the line break too.
#[wasm_bindgen_test]
fn a_composition_after_a_break_lands_after_it() {
    let f = F::new("<p>alpha<br>bravo</p>");
    let b = f.char_rect(1, 0);
    f.click(b.x() + 1.0, b.y() + b.height() / 2.0);
    assert_eq!(f.head(), 6, "precondition: the caret is after the break");
    let ta = f.capture();
    assert_eq!(ta.value(), "alpha\nbravo", "the mirror holds the break");
    assert_eq!(ta.selection_start().unwrap(), Some(6));
    f.compose("X");
    assert_eq!(f.para(), "alpha|Xbravo");
    let s = f.handle.selection();
    assert_eq!(
        (s.anchor().0, s.head().0),
        (8, 8),
        "a collapsed caret after the commit, where the field left it"
    );
    f.done();
}

/// The other side: a composition committed at the end of the line before the
/// break stays before it (`alphaX<br>bravo`).
#[wasm_bindgen_test]
fn a_composition_before_a_break_stays_before_it() {
    let f = F::new("<p>alpha<br>bravo</p>");
    let a = f.char_rect(0, 4);
    f.click(a.right() - 1.0, a.y() + a.height() / 2.0);
    assert_eq!(f.head(), 5);
    f.compose("X");
    assert_eq!(f.para(), "alphaX|bravo");
    assert_eq!(f.head(), 6);
    f.done();
}

/// An image: a composition right after it lands after it. Two leaves ahead of
/// the point (an image and a break) so a mirror that counted only one of them
/// would land between.
#[wasm_bindgen_test]
fn a_composition_after_an_image_lands_after_it() {
    let f = F::new(&format!(
        "<p>ab<br>alpha<img src=\"{GIF}\" alt=\"\">bravo</p>"
    ));
    f.block()
        .query_selector("img")
        .unwrap()
        .unwrap()
        .set_attribute("style", "width: 24px; height: 16px")
        .unwrap();
    let b = f.char_rect(2, 0);
    f.click(b.x() + 1.0, b.y() + b.height() / 2.0);
    assert_eq!(f.head(), 9, "precondition: after the image");
    assert_eq!(f.capture().value(), "ab\nalpha\u{FFFC}bravo");
    f.compose("X");
    assert_eq!(f.para(), "ab|alpha#Xbravo");
    f.done();
}

/// Autocorrect replaces the word after a break (`insertReplacementText`,
/// reconciled from the field on `input`): the break survives.
#[wasm_bindgen_test]
fn an_autocorrect_of_the_word_after_a_break_keeps_the_break() {
    let f = F::new("<p>alpha<br>bravo</p>");
    let b = f.char_rect(1, 4);
    f.click(b.right() - 1.0, b.y() + b.height() / 2.0);
    assert_eq!(f.head(), 11);
    let ta = f.capture();
    ta.set_value("alpha\nBravo");
    ta.set_selection_range(11, 11).unwrap();
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_input_type("insertReplacementText");
    let ev = web_sys::InputEvent::new_with_event_init_dict("input", &init).unwrap();
    ta.dispatch_event(&ev).unwrap();
    assert_eq!(f.para(), "alpha|Bravo");
    assert_eq!(f.head(), 11);
    f.done();
}

/// An astral character (a surrogate pair: two UTF-16 units, one model
/// position) before a break: the DOM point's UTF-16 offset is counted as
/// characters. From the review of #1101 (p1).
#[wasm_bindgen_test]
fn an_astral_character_before_a_break_counts_as_one() {
    // a=0 😀=1 b=2 <br>=3 c=4
    let f = F::new("<p>a\u{1F600}b<br>cd</p>");
    let c = f.char_rect(1, 0);
    f.click(c.x() + 1.0, c.y() + c.height() / 2.0);
    assert_eq!(f.head(), 4, "after the break");
    let b = f.char_rect(0, 3);
    let (x, y) = (b.x() + 1.0, b.y() + b.height() / 2.0);
    let (node, off) = dom_point(x, y);
    assert!(
        node == f.text_node(0) && off == 3,
        "oracle: before `b`, UTF-16 unit 3"
    );
    f.click(x, y);
    assert_eq!(f.head(), 2, "before `b`: the emoji is one position");
    f.done();
}

/// Enter on a link after a break (a `click` with `detail == 0`, no pointer):
/// the link is found from its anchor's first character, which is after the
/// break. Before #1101 that character mapped before the break and the link was
/// never offered. From the review of #1101 (p5).
#[wasm_bindgen_test]
fn enter_on_a_link_after_a_break_offers_it() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let f = F::new("<p>ab<br><a href=\"https://example.com/x\">link</a></p>");
    let got: Rc<RefCell<Option<(usize, usize)>>> = Rc::default();
    let g = got.clone();
    f.handle.on_link_click(move |c| {
        *g.borrow_mut() = Some((c.link.from.0, c.link.to.0));
        true
    });
    let a = f.block().query_selector("a").unwrap().expect("anchor");
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_detail(0);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    a.dispatch_event(&ev).unwrap();
    assert_eq!(*got.borrow(), Some((4, 8)));
    f.done();
}
