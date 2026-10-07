//! A press in the editor lands on the same model position whichever of the two
//! "caret from a point" calls the browser has.
//!
//! Firefox has only the standard `document.caretPositionFromPoint`; rinch-web
//! asked only for WebKit's `caretRangeFromPoint`, so in Firefox a press placed
//! no caret at all. `caret_point_from_point` now asks for whichever exists. This
//! runs each press twice in one browser: as it is, and with
//! `caretRangeFromPoint` hidden on the document, which is the document Firefox
//! presents. The answers must be the same positions.
//!
//! This is Chrome's implementation of the standard call, not Firefox's: no wasm
//! suite runs in Firefox (#1461).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_caret_point_standard_api
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-caret-point";
/// A 1x1 transparent GIF, stretched by the inline style.
const GIF: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
const OLD_CALL: &str = "caretRangeFromPoint";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Hide `caretRangeFromPoint` on the document (an own property shadowing the
/// prototype's) until the guard drops.
struct OnlyTheStandardCall;

impl OnlyTheStandardCall {
    fn new() -> Self {
        let descriptor = js_sys::Object::new();
        js_sys::Reflect::set(&descriptor, &"value".into(), &JsValue::UNDEFINED).unwrap();
        js_sys::Reflect::set(&descriptor, &"configurable".into(), &JsValue::TRUE).unwrap();
        js_sys::Object::define_property(
            document().unchecked_ref::<js_sys::Object>(),
            &OLD_CALL.into(),
            &descriptor,
        );
        assert!(
            js_sys::Reflect::get(&document(), &OLD_CALL.into())
                .unwrap()
                .is_undefined(),
            "positive control: the old call is hidden"
        );
        assert!(
            js_sys::Reflect::get(&document(), &"caretPositionFromPoint".into())
                .unwrap()
                .is_function(),
            "this browser has the standard call"
        );
        Self
    }
}

impl Drop for OnlyTheStandardCall {
    fn drop(&mut self) {
        js_sys::Reflect::delete_property(
            document().unchecked_ref::<js_sys::Object>(),
            &OLD_CALL.into(),
        )
        .unwrap();
    }
}

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl F {
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
            // Fixed at the top of the viewport, over the runner's own output, so
            // every probe is a point the browser can hit.
            "position: fixed; top: 0; left: 0; z-index: 9999; background: white; \
             font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 400px;",
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
        Self { root, host, handle }
    }

    /// The text node holding `needle`, and the UTF-16 offset of its first character.
    fn find(&self, needle: &str) -> (web_sys::Node, u32) {
        fn walk(node: &web_sys::Node, needle: &str) -> Option<(web_sys::Node, u32)> {
            if node.node_type() == web_sys::Node::TEXT_NODE {
                let text = node.text_content().unwrap_or_default();
                let at = text.find(needle)?;
                return Some((node.clone(), text[..at].encode_utf16().count() as u32));
            }
            let kids = node.child_nodes();
            (0..kids.length())
                .filter_map(|i| kids.item(i))
                .find_map(|kid| walk(&kid, needle))
        }
        let editor = document()
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("editor");
        if let Some(found) = walk(&editor, needle) {
            return found;
        }
        panic!("no text {needle:?}");
    }

    /// The client rect of `needle`'s first character.
    fn rect(&self, needle: &str) -> web_sys::DomRect {
        let (node, at) = self.find(needle);
        let range = document().create_range().unwrap();
        range.set_start(&node, at).unwrap();
        range.set_end(&node, at + 1).unwrap();
        range.get_bounding_client_rect()
    }

    /// Press and release at a point `dx` into `needle`'s first character
    /// (a fraction of its width); the head afterwards.
    fn press(&self, needle: &str, dx: f64) -> usize {
        let r = self.rect(needle);
        let (x, y) = (r.x() + r.width() * dx, r.y() + r.height() / 2.0);
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        self.handle.selection().head().0
    }

    /// Press on `from`, move to `to`, release; the selection's two ends.
    fn drag(&self, from: &str, to: &str) -> (usize, usize) {
        let (a, b) = (self.rect(from), self.rect(to));
        let (ax, ay) = (a.x() + 1.0, a.y() + a.height() / 2.0);
        let (bx, by) = (b.x() + 1.0, b.y() + b.height() / 2.0);
        mouse("mousedown", ax, ay);
        mouse("mousemove", bx, by);
        mouse("mouseup", bx, by);
        let selection = self.handle.selection();
        (selection.from().0, selection.to().0)
    }

    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f64, y: f64) {
    let target = document()
        .element_from_point(x as f32, y as f32)
        .expect("hit");
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

fn html() -> String {
    format!(
        "<p>alpha bravo<br>charlie delta</p>\
         <p>echo <img src=\"{GIF}\" style=\"width:20px;height:16px\"> foxtrot</p>\
         <ul><li><p>golf</p><ul><li><p>hotel</p></li></ul></li></ul>\
         <blockquote><p>india</p></blockquote>\
         <table><tr><td><p>juliet</p></td><td><p>kilo</p></td></tr></table>\
         <p>lima</p>"
    )
}

/// Every probe: the text pressed on, and how far into its first character.
const PROBES: &[(&str, f64)] = &[
    ("alpha", 0.1),
    ("bravo", 0.9),
    ("charlie", 0.1), // the start of the line after a hard break
    ("delta", 0.6),
    ("echo", 0.1),
    ("foxtrot", 0.1), // just after an image
    ("golf", 0.1),
    ("hotel", 0.9),  // a nested list item
    ("india", 0.1),  // inside a quote
    ("juliet", 0.9), // a table cell
    ("kilo", 0.1),
    ("lima", 0.1),
];

#[wasm_bindgen_test]
fn a_press_lands_on_the_same_position_with_only_the_standard_call() {
    let f = F::new(&html());
    let with_both: Vec<usize> = PROBES
        .iter()
        .map(|(needle, dx)| f.press(needle, *dx))
        .collect();
    // Positive control: the presses reached the editor and told the probes apart.
    let mut distinct = with_both.clone();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        PROBES.len(),
        "each probe is its own position: {with_both:?}"
    );
    assert!(
        with_both.windows(2).all(|w| w[0] < w[1]),
        "in document order: {with_both:?}"
    );

    let hidden = OnlyTheStandardCall::new();
    let with_standard: Vec<usize> = PROBES
        .iter()
        .map(|(needle, dx)| f.press(needle, *dx))
        .collect();
    drop(hidden);
    assert_eq!(with_standard, with_both, "probes: {PROBES:?}");
    f.done();
}

#[wasm_bindgen_test]
fn a_drag_selects_the_same_range_with_only_the_standard_call() {
    let f = F::new(&html());
    let drags = [
        ("bravo", "delta"),
        ("echo", "foxtrot"),
        ("golf", "hotel"),
        ("alpha", "lima"),
    ];
    let with_both: Vec<(usize, usize)> = drags.iter().map(|(a, b)| f.drag(a, b)).collect();
    assert!(
        with_both.iter().all(|(from, to)| from < to),
        "each drag selected something: {with_both:?}"
    );

    let hidden = OnlyTheStandardCall::new();
    let with_standard: Vec<(usize, usize)> = drags.iter().map(|(a, b)| f.drag(a, b)).collect();
    drop(hidden);
    assert_eq!(with_standard, with_both);
    f.done();
}

#[wasm_bindgen_test]
fn with_neither_call_a_press_places_nothing_and_does_not_throw() {
    let f = F::new(&html());
    let before = f.press("lima", 0.1);
    let hidden = OnlyTheStandardCall::new();
    let descriptor = js_sys::Object::new();
    js_sys::Reflect::set(&descriptor, &"value".into(), &JsValue::UNDEFINED).unwrap();
    js_sys::Reflect::set(&descriptor, &"configurable".into(), &JsValue::TRUE).unwrap();
    js_sys::Object::define_property(
        document().unchecked_ref::<js_sys::Object>(),
        &"caretPositionFromPoint".into(),
        &descriptor,
    );
    let after = f.press("alpha", 0.1);
    js_sys::Reflect::delete_property(
        document().unchecked_ref::<js_sys::Object>(),
        &"caretPositionFromPoint".into(),
    )
    .unwrap();
    drop(hidden);
    assert_eq!(after, before, "nothing to ask, so the caret stays");
    f.done();
}
