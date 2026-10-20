//! When a touch press on a `data-rid` element waits for its release, and what
//! that press does to the browser's own defaults (#1582, #1588).
//!
//! A touch or pen press whose finger may start a pan dispatches its click on
//! `pointerup`, and a pan (`pointercancel`, or a move past the slop) drops it.
//! #1582: the walk that decides "may start a pan" read every ancestor's
//! `overflow` and so missed the **page**: `<html>` computes `overflow: visible`
//! while the document scrolls, and every touch on a button of a scrolling page
//! clicked on contact, a pan included. It now counts the viewport, and stops
//! deferring where `touch-action: none` says the finger cannot pan (a slider
//! arms its `Drag` on contact, on a scrolling page and inside a scroller).
//!
//! #1588: a deferred press was not `preventDefault`ed, so after a tap Chrome
//! sent its compatibility `mousedown` to whatever was under the finger by then
//! — the overlay the handler opened — and that press blurred the field the
//! overlay had focused. It is prevented now, as the press on contact always
//! was, unless the browser is to focus a form control at the press. The
//! compatibility events themselves are only sent for a real touch:
//! `real_touch_1582_1588.rs` drives that through the DevTools protocol.
//!
//! Synthetic events: each fixture's positive control is that the handler ran
//! (a genuine event reached rinch's delegation).
//!
//! **Mount through `rinch_web::mount_into`**: without the owner `mount_tree`
//! pushes, component effects are disposed at once.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-1582-1588";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    log: Log,
}

/// What the page around the fixture is.
#[derive(Clone, Copy, PartialEq)]
enum Page {
    /// Taller than the viewport: the document scrolls.
    Scrolls,
    /// Taller than the viewport, with `overflow: hidden` on `<html>`.
    HtmlHidden,
    /// Taller than the viewport, with `overflow: hidden` on `<body>` only
    /// (propagated to the viewport, since `<html>` is `visible`).
    BodyHidden,
}

/// Mounts, inside a box of `wrapper_style`:
/// - `#btn`, a `<button>` with a handler;
/// - `#track`, a `touch-action: none` div whose handler arms `Drag::percent`;
/// - `#plaindrag`, the same handler on a div with no `touch-action`;
/// - `#row`, a handled div holding `#field`, an `<input>`;
///
/// and a 3000px spacer after it, so the page is taller than the viewport.
fn mount(page: Page, wrapper_style: &str) -> Fixture {
    rinch_web::__reset_activation_state();
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i)
                && let Ok(el) = n.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
    let html = document().document_element().unwrap();
    let body = document().body().unwrap();
    let _ = html.remove_attribute("style");
    let _ = body.remove_attribute("style");
    match page {
        Page::Scrolls => {}
        Page::HtmlHidden => html.set_attribute("style", "overflow: hidden").unwrap(),
        Page::BodyHidden => body.set_attribute("style", "overflow: hidden").unwrap(),
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute("style", "width: 300px; margin: 20px")
        .unwrap();
    body.append_child(&host).unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let wrapper_style = wrapper_style.to_string();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let outer = scope.create_element("div");
            let wrap = scope.create_element("div");
            wrap.set_attribute("id", "wrap");
            wrap.set_attribute("style", &wrapper_style);
            let handled = |scope: &mut RenderScope, tag: &str, id: &str| -> NodeHandle {
                let e = scope.create_element(tag);
                e.set_attribute("id", id);
                e.append_child(&scope.create_text(id));
                let sink = sink.clone();
                let name = id.to_string();
                let rid = scope.register_handler(move || sink.borrow_mut().push(name.clone()));
                e.set_attribute("data-rid", &rid.0.to_string());
                e
            };
            wrap.append_child(&handled(scope, "button", "btn"));
            let track = scope.create_element("div");
            track.set_attribute("id", "track");
            track.set_attribute(
                "style",
                "width: 200px; height: 20px; touch-action: none; background: #ccc",
            );
            {
                let sink = sink.clone();
                let rid = scope.register_handler(move || {
                    sink.borrow_mut().push("track".into());
                    rinch_core::Drag::percent().on_move(|_, _| {}).start();
                });
                track.set_attribute("data-rid", &rid.0.to_string());
            }
            wrap.append_child(&track);
            // The same handler on a div that lets a finger pan.
            let plaindrag = scope.create_element("div");
            plaindrag.set_attribute("id", "plaindrag");
            plaindrag.set_attribute("style", "width: 200px; height: 20px");
            {
                let sink = sink.clone();
                let rid = scope.register_handler(move || {
                    sink.borrow_mut().push("plaindrag".into());
                    let sink = sink.clone();
                    rinch_core::Drag::percent()
                        .on_move(|_, _| {})
                        .on_end(move |_, _| sink.borrow_mut().push("end".into()))
                        .start();
                });
                plaindrag.set_attribute("data-rid", &rid.0.to_string());
            }
            wrap.append_child(&plaindrag);
            let row = handled(scope, "div", "row");
            let field = scope.create_element("input");
            field.set_attribute("id", "field");
            row.append_child(&field);
            wrap.append_child(&row);
            // Content that overflows `#wrap` when it is a scroller.
            let filler = scope.create_element("div");
            filler.set_attribute("style", "height: 600px");
            wrap.append_child(&filler);
            outer.append_child(&wrap);
            let spacer = scope.create_element("div");
            spacer.set_attribute("style", "height: 3000px");
            outer.append_child(&spacer);
            outer
        },
    );
    Fixture { root, host, log }
}

const SCROLLER: &str = "height: 200px; overflow: auto";

impl Fixture {
    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
    fn teardown(self) {
        rinch_core::Drag::cancel();
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
        let _ = document()
            .document_element()
            .unwrap()
            .remove_attribute("style");
        let _ = document().body().unwrap().remove_attribute("style");
    }
}

fn by_id(id: &str) -> web_sys::Element {
    document().get_element_by_id(id).unwrap()
}

fn page_scrolls() -> bool {
    let se = document().scrolling_element().unwrap();
    se.scroll_height() > se.client_height()
}

fn pointer(el: &web_sys::Element, kind: &str, dy: f64) -> web_sys::PointerEvent {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(7);
    init.set_is_primary(true);
    init.set_pointer_type("touch");
    init.set_button(if kind == "pointermove" { -1 } else { 0 });
    init.set_buttons(if kind == "pointerdown" || kind == "pointermove" {
        1
    } else {
        0
    });
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height().min(20.0) / 2.0 + dy) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

/// #1582: a finger on a button of a scrolling page has not clicked it yet — it
/// may be the start of a scroll. The click waits for the release.
/// Kills dropping the viewport from the walk.
#[wasm_bindgen_test]
fn a_touch_on_a_button_of_a_scrolling_page_clicks_on_release() {
    let f = mount(Page::Scrolls, "");
    assert!(page_scrolls(), "positive control: the page scrolls");
    let btn = by_id("btn");
    pointer(&btn, "pointerdown", 0.0);
    assert!(f.log().is_empty(), "clicked on contact: {:?}", f.log());
    pointer(&btn, "pointerup", 0.0);
    assert_eq!(f.log(), ["btn"], "the tap clicks on release");
    f.teardown();
}

/// #1582's own shape: the browser takes the contact for a pan
/// (`pointercancel`), and the button is not clicked at all.
#[wasm_bindgen_test]
fn a_touch_pan_from_a_button_of_a_scrolling_page_clicks_nothing() {
    let f = mount(Page::Scrolls, "");
    let btn = by_id("btn");
    pointer(&btn, "pointerdown", 0.0);
    pointer(&btn, "pointermove", -40.0);
    pointer(&btn, "pointercancel", -60.0);
    assert!(f.log().is_empty(), "a pan clicked: {:?}", f.log());
    // Positive control: the same button is reachable.
    pointer(&btn, "pointerdown", 0.0);
    pointer(&btn, "pointerup", 0.0);
    assert_eq!(f.log(), ["btn"]);
    f.teardown();
}

/// A page whose viewport does not scroll (`overflow: hidden` on `<html>`, as a
/// scroll lock sets it) keeps the click on contact, however tall the content.
/// Kills reading the scrolling element's size without the viewport's
/// `overflow`.
#[wasm_bindgen_test]
fn a_touch_on_a_page_that_cannot_scroll_clicks_on_contact() {
    let f = mount(Page::HtmlHidden, "");
    let btn = by_id("btn");
    pointer(&btn, "pointerdown", 0.0);
    assert_eq!(f.log(), ["btn"]);
    pointer(&btn, "pointerup", 0.0);
    assert_eq!(f.log(), ["btn"], "one click");
    f.teardown();
}

/// `overflow: hidden` on `<body>` alone reaches the viewport too (`<html>` is
/// `visible`, so the body's value is propagated). Kills reading `<html>`'s
/// `overflow` only.
#[wasm_bindgen_test]
fn a_page_whose_body_hides_overflow_cannot_scroll_either() {
    let f = mount(Page::BodyHidden, "");
    pointer(&by_id("btn"), "pointerdown", 0.0);
    assert_eq!(f.log(), ["btn"]);
    pointer(&by_id("btn"), "pointerup", 0.0);
    f.teardown();
}

/// A `touch-action: none` control (a `Slider`) cannot start a pan, so on a
/// scrolling page its handler runs on contact and the `Drag` it arms is live
/// while the finger moves. Kills dropping the `touch-action` check (with the
/// viewport counted, every press of a scrolling page would otherwise wait).
#[wasm_bindgen_test]
fn a_touch_action_none_control_on_a_scrolling_page_arms_on_contact() {
    let f = mount(Page::Scrolls, "");
    let track = by_id("track");
    pointer(&track, "pointerdown", 0.0);
    assert_eq!(f.log(), ["track"]);
    assert!(
        rinch_core::Drag::is_active(),
        "the drag is armed on contact"
    );
    pointer(&track, "pointerup", 0.0);
    f.teardown();
}

/// The same control inside a scroller: it waited for the release before (and
/// armed its `Drag` only then), though the finger on it could not pan.
#[wasm_bindgen_test]
fn a_touch_action_none_control_in_a_scroller_arms_on_contact() {
    let f = mount(Page::HtmlHidden, SCROLLER);
    let wrap = by_id("wrap");
    assert!(
        wrap.scroll_height() > wrap.client_height(),
        "positive control: #wrap scrolls"
    );
    pointer(&by_id("track"), "pointerdown", 0.0);
    assert_eq!(f.log(), ["track"]);
    assert!(rinch_core::Drag::is_active());
    pointer(&by_id("track"), "pointerup", 0.0);
    f.teardown();
}

/// #1588: a deferred touch press on a button is `preventDefault`ed, which is
/// what keeps the browser from sending its compatibility `mousedown` after the
/// tap. Kills dropping the deferred path's `preventDefault`.
#[wasm_bindgen_test]
fn a_deferred_touch_press_on_a_button_is_prevented() {
    let f = mount(Page::HtmlHidden, SCROLLER);
    let ev = pointer(&by_id("btn"), "pointerdown", 0.0);
    assert!(
        f.log().is_empty(),
        "positive control: the press was deferred"
    );
    assert!(ev.default_prevented(), "the deferred press is prevented");
    pointer(&by_id("btn"), "pointerup", 0.0);
    assert_eq!(f.log(), ["btn"], "positive control: the tap clicked");
    f.teardown();
}

/// …and a deferred press on a text field inside a clickable row is the
/// browser's: it focuses the field and places the caret, so it is not
/// prevented. Kills preventing every deferred press.
#[wasm_bindgen_test]
fn a_deferred_touch_press_on_a_field_in_a_clickable_row_is_left_to_the_browser() {
    let f = mount(Page::HtmlHidden, SCROLLER);
    let ev = pointer(&by_id("field"), "pointerdown", 0.0);
    assert!(
        f.log().is_empty(),
        "positive control: the press was deferred"
    );
    assert!(!ev.default_prevented());
    pointer(&by_id("field"), "pointerup", 0.0);
    assert_eq!(
        f.log(),
        ["row"],
        "positive control: the tap clicked the row"
    );
    f.teardown();
}

/// A deferred tap whose handler arms a `Drag` ends that drag at the release
/// (through `on_end`), rather than leave it following a finger that has
/// lifted. Kills dropping the `finish_drag` after the deferred dispatch.
#[wasm_bindgen_test]
fn a_deferred_tap_that_arms_a_drag_ends_it_at_the_release() {
    let f = mount(Page::HtmlHidden, SCROLLER);
    let el = by_id("plaindrag");
    pointer(&el, "pointerdown", 0.0);
    assert!(
        f.log().is_empty(),
        "positive control: the press was deferred"
    );
    pointer(&el, "pointerup", 0.0);
    assert_eq!(f.log(), ["plaindrag", "end"]);
    assert!(!rinch_core::Drag::is_active(), "the drag outlived the tap");
    f.teardown();
}
