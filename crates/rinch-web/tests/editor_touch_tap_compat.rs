//! Browser-driven tests for issue #302: a touch tap in the web editor is
//! serviced **once**.
//!
//! Since #249 a touch tap is handled from its `pointerup` (so the tap itself is
//! the user gesture iOS wants before it raises the keyboard), and the browser
//! then synthesizes a *compatibility* `mousedown` at the same point. That
//! `mousedown` used to run the whole press again — refocus, empty and refill the
//! capture textarea under the soft keyboard, a second selection transaction, a
//! second `on_link_click`. It must now do nothing for the tap already serviced,
//! while a double tap's compatibility `mousedown` (`detail == 2`) still selects
//! the word — the browser's own click counting is what drives that.
//!
//! Headless Chrome does not synthesize compatibility mouse events from a
//! dispatched `PointerEvent`, so each fixture dispatches the sequence a real tap
//! produces itself: `pointerdown`, `pointerup` (`pointerType: "touch"`), then
//! the `mousedown` / `mouseup` at the same point with the click count the browser
//! would give it. Every fixture proves the `pointerup` reached rinch (a
//! transaction and a field write happened, the capture textarea took focus)
//! before it asserts that the `mousedown` added nothing.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_touch_tap_compat
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{EditorState, Node, Plugin, PluginKey, Transaction};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-302";

/// Paragraph 1's text runs 1..=16 ("world" is 7..12); paragraph 2 opens at 17.
const CONTENT: &str = "<p>Hello world one</p><p>Second paragraph here</p>";
const WORLD: (usize, usize) = (7, 12);

/// A link over "link text" (7..16) in paragraph 1.
const LINKED: &str = "<p>Hello <a href=\"https://example.com/x\">link text</a> here</p>";

/// Counts every transaction the editor applies: `Plugin::apply` runs once per
/// applied transaction, selection-only ones included.
struct TxCounter(Rc<Cell<u32>>);

impl Plugin for TxCounter {
    fn key(&self) -> PluginKey {
        PluginKey("test-302-tx-counter")
    }
    fn apply(
        &self,
        _tr: &Transaction,
        _old: &EditorState,
        _new_doc: &Node,
        _prev: Option<&dyn Any>,
    ) -> Option<Rc<dyn Any>> {
        self.0.set(self.0.get() + 1);
        None
    }
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    tx: Rc<Cell<u32>>,
    links: Rc<Cell<u32>>,
}

impl Fixture {
    fn mount(content: &str) -> Self {
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
        let tx = Rc::new(Cell::new(0u32));
        assert!(handle.add_plugin(Rc::new(TxCounter(tx.clone()))));
        let links = Rc::new(Cell::new(0u32));
        let seen = links.clone();
        handle.on_link_click(move |_| {
            seen.set(seen.get() + 1);
            true
        });
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        Self {
            root,
            host,
            handle,
            tx,
            links,
        }
    }

    fn paragraph(&self, i: u32) -> web_sys::Element {
        document()
            .query_selector_all("[data-pm-editor] p")
            .unwrap()
            .item(i)
            .unwrap_or_else(|| panic!("no paragraph {i}"))
            .dyn_into()
            .unwrap()
    }

    /// The viewport centre of character `ch` of paragraph `p`.
    fn point(&self, p: u32, ch: u32) -> (f32, f32) {
        let (text, off) = text_at(&self.paragraph(p), ch);
        let range = document().create_range().unwrap();
        range.set_start(&text, off).unwrap();
        range.set_end(&text, off + 1).unwrap();
        let r = range.get_bounding_client_rect();
        (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        )
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    fn capture_focused(&self) -> bool {
        document().active_element().as_deref() == Some(self.capture().as_ref())
    }

    /// Focus the editor without a press, so the capture textarea exists, and
    /// wrap its `value` setter in a counter. Returns the counter's reader.
    fn instrument_capture(&self) -> impl Fn() -> u32 + use<> {
        self.handle.focus();
        let ta = self.capture();
        assert!(
            self.capture_focused(),
            "positive control: EditorHandle::focus focuses the capture textarea"
        );
        let install = js_sys::Function::new_with_args(
            "ta",
            "const d = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value');\
             ta.__writes302 = 0;\
             Object.defineProperty(ta, 'value', {\
               configurable: true,\
               get() { return d.get.call(this); },\
               set(v) { this.__writes302++; d.set.call(this, v); },\
             });",
        );
        install.call1(&wasm_bindgen::JsValue::NULL, &ta).unwrap();
        move || {
            js_sys::Reflect::get(&ta, &"__writes302".into())
                .unwrap()
                .as_f64()
                .unwrap() as u32
        }
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn under(x: f32, y: f32) -> web_sys::Element {
    document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"))
}

/// A touch contact's `pointerdown` + `pointerup` at `(x, y)`.
fn touch_pointer(x: f32, y: f32) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_type("touch");
    init.set_pointer_id(7);
    init.set_is_primary(true);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let target = under(x, y);
    init.set_buttons(1);
    let down = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    target.dispatch_event(&down).unwrap();
    init.set_buttons(0);
    let up = web_sys::PointerEvent::new_with_event_init_dict("pointerup", &init).unwrap();
    target.dispatch_event(&up).unwrap();
}

/// A primary `mousedown` at `(x, y)` with click count `detail`, as the browser
/// synthesizes after a tap (or as a real mouse sends). Returns it, for
/// `defaultPrevented`.
fn mousedown(x: f32, y: f32, detail: i32) -> web_sys::MouseEvent {
    mouse("mousedown", x, y, detail, 1)
}

fn mouseup(x: f32, y: f32, detail: i32) {
    mouse("mouseup", x, y, detail, 0);
}

fn mouse(name: &str, x: f32, y: f32, detail: i32, buttons: u16) -> web_sys::MouseEvent {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(buttons);
    init.set_detail(detail);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    under(x, y).dispatch_event(&ev).unwrap();
    ev
}

fn text_at(el: &web_sys::Element, ch: u32) -> (web_sys::Node, u32) {
    fn walk(node: &web_sys::Node, ch: u32, seen: &mut u32) -> Option<(web_sys::Node, u32)> {
        let kids = node.child_nodes();
        for i in 0..kids.length() {
            let kid = kids.item(i)?;
            if kid.node_type() == web_sys::Node::TEXT_NODE {
                let len = kid
                    .text_content()
                    .unwrap_or_default()
                    .encode_utf16()
                    .count() as u32;
                if ch < *seen + len {
                    return Some((kid, ch - *seen));
                }
                *seen += len;
            } else if let Some(found) = walk(&kid, ch, seen) {
                return Some(found);
            }
        }
        None
    }
    walk(el, ch, &mut 0).unwrap_or_else(|| panic!("no character {ch} in <{}>", el.tag_name()))
}

/// The heart of #302: the tap's compatibility `mousedown` issues no second
/// selection transaction and writes nothing to the capture textarea, while the
/// press is still consumed and the tap's caret and focus stand.
///
/// Kills: no early return for the compatibility `mousedown` (main before the
/// fix: one more transaction, two more field writes — empty then refill).
#[wasm_bindgen_test]
fn a_taps_compatibility_mousedown_redoes_nothing() {
    let f = Fixture::mount(CONTENT);
    let writes = f.instrument_capture();
    // Tap paragraph 2, away from where `focus()` left the caret (the start).
    let (x, y) = f.point(1, 3);
    let (tx0, w0) = (f.tx.get(), writes());

    touch_pointer(x, y);
    let (tx1, w1) = (f.tx.get(), writes());
    // Positive control: the pointerup path serviced the tap.
    assert_eq!(
        tx1 - tx0,
        1,
        "the tap's pointerup places the caret: one transaction"
    );
    assert!(w1 > w0, "and re-mirrors the capture textarea");
    assert!(f.capture_focused(), "and focuses the capture textarea");
    let caret = f.handle.selection();
    assert!(
        caret.is_empty() && (20..=22).contains(&caret.head().0),
        "caret at the tap, got {caret:?}"
    );

    let down = mousedown(x, y, 1);
    mouseup(x, y, 1);
    assert_eq!(
        f.tx.get(),
        tx1,
        "the compatibility mousedown issues no transaction"
    );
    assert_eq!(writes(), w1, "nor writes the capture textarea");
    assert!(
        down.default_prevented(),
        "but it is still consumed, as before"
    );
    assert_eq!(f.handle.selection(), caret, "the tap's caret stands");
    assert!(f.capture_focused(), "and so does its focus");
    assert_eq!(
        f.capture().value(),
        "Second paragraph here",
        "the field still mirrors the caret's block"
    );
    f.teardown();
}

/// A double tap still selects the word: the second tap's compatibility
/// `mousedown` carries `detail == 2` and is run in full.
///
/// Kills: skipping every compatibility `mousedown` after a serviced tap,
/// whatever its click count.
#[wasm_bindgen_test]
fn a_double_tap_still_selects_the_word() {
    let f = Fixture::mount(CONTENT);
    let (x, y) = f.point(0, 8);

    touch_pointer(x, y);
    mousedown(x, y, 1);
    mouseup(x, y, 1);
    assert!(
        f.capture_focused(),
        "positive control: the first tap focuses the editor"
    );
    assert!(f.handle.selection().is_empty(), "one tap places a caret");

    touch_pointer(x, y);
    mousedown(x, y, 2);
    mouseup(x, y, 2);
    let sel = f.handle.selection();
    assert_eq!(
        (sel.from().0, sel.to().0),
        WORLD,
        "the double tap's compatibility mousedown selects the word"
    );
    f.teardown();
}

/// A mouse press after a tap is its own press. One at another point moves the
/// caret, even while a tap whose compatibility `mousedown` never came is still
/// recorded; a second press at the tap's point, once the tap's compatibility
/// `mousedown` has been answered, is run in full too.
///
/// Kills: matching on time alone (no position check), and a record the
/// compatibility `mousedown` does not consume.
#[wasm_bindgen_test]
fn a_mouse_press_after_a_tap_is_unchanged() {
    let f = Fixture::mount(CONTENT);
    let (x, y) = f.point(0, 2);
    // A tap with no compatibility `mousedown` after it (a page may not get one).
    touch_pointer(x, y);
    assert!(
        f.capture_focused(),
        "positive control: the tap focused the editor"
    );

    // A mouse press elsewhere, straight after.
    let tx = f.tx.get();
    let (x2, y2) = f.point(1, 10);
    mousedown(x2, y2, 1);
    mouseup(x2, y2, 1);
    assert_eq!(
        f.tx.get(),
        tx + 1,
        "a mouse press elsewhere is a transaction"
    );
    let head = f.handle.selection().head().0;
    assert!(
        (27..=29).contains(&head),
        "and moves the caret there, got {head}"
    );

    // A full tap at the first point, then a mouse press at that same point: the
    // tap is spent by its compatibility `mousedown`, so this press moves nothing
    // new but is run in full.
    touch_pointer(x, y);
    mousedown(x, y, 1);
    mouseup(x, y, 1);
    let tx = f.tx.get();
    mousedown(x, y, 1);
    mouseup(x, y, 1);
    assert_eq!(
        f.tx.get(),
        tx + 1,
        "a later press at the tap's point is run in full"
    );
    let head = f.handle.selection().head().0;
    assert!(
        (2..=4).contains(&head),
        "and the caret is at that point, got {head}"
    );
    f.teardown();
}

/// A plain mouse click, with no tap anywhere, is unchanged: one transaction,
/// caret at the click, capture textarea focused.
#[wasm_bindgen_test]
fn a_mouse_click_without_a_tap_is_unchanged() {
    let f = Fixture::mount(CONTENT);
    let (x, y) = f.point(0, 8);
    let tx = f.tx.get();
    let down = mousedown(x, y, 1);
    mouseup(x, y, 1);
    assert!(down.default_prevented(), "the press is consumed");
    assert!(f.capture_focused(), "the click focuses the editor");
    assert_eq!(f.tx.get(), tx + 1, "one transaction");
    let head = f.handle.selection().head().0;
    assert!((8..=10).contains(&head), "caret at the click, got {head}");
    f.teardown();
}

/// A tap on a link is offered to `on_link_click` once, not once per run.
///
/// Kills: no early return for the compatibility `mousedown` (main: 2 offers).
#[wasm_bindgen_test]
fn a_tap_on_a_link_is_offered_once() {
    let f = Fixture::mount(LINKED);
    let (x, y) = f.point(0, 9);
    touch_pointer(x, y);
    assert_eq!(
        f.links.get(),
        1,
        "positive control: the tap's pointerup offers the link"
    );
    mousedown(x, y, 1);
    mouseup(x, y, 1);
    assert_eq!(
        f.links.get(),
        1,
        "the compatibility mousedown does not offer it again"
    );
    f.teardown();
}

/// The compatibility `mousedown` is skipped only while the tap's state stands.
/// If the capture textarea lost focus between the `pointerup` and it, the press
/// runs in full and focuses the editor again.
///
/// Kills: skipping without checking that the tap's editor still holds focus.
#[wasm_bindgen_test]
fn a_compat_mousedown_after_focus_left_runs_the_press() {
    let f = Fixture::mount(CONTENT);
    let (x, y) = f.point(0, 8);
    touch_pointer(x, y);
    assert!(
        f.capture_focused(),
        "positive control: the tap focused the editor"
    );
    f.capture().blur().unwrap();
    assert!(!f.capture_focused(), "the blur took focus away");

    let tx = f.tx.get();
    mousedown(x, y, 1);
    mouseup(x, y, 1);
    assert!(
        f.capture_focused(),
        "the compatibility mousedown focuses the editor again"
    );
    assert_eq!(f.tx.get(), tx + 1, "and places the caret");
    f.teardown();
}
