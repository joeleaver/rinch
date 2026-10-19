//! Review fixtures for PR #1580 (#1146). Each names the finding it pins.
//! Synthetic presses: the browser focuses nothing for them, so a focus asserted
//! here is one rinch made; a press left to the browser is pinned at
//! `defaultPrevented`.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-1580-review";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    log: Log,
}

/// `#field` (a text field), then:
/// - `#row` (`tabindex=0`) holding `#dis`, a disabled `<button>` with a handler;
/// - `#cerow` (`tabindex=0`) holding `#cef`, a `contenteditable=false` span with a handler;
/// - `#ce` (`contenteditable`) holding `#cespan`, a span with a handler;
/// - `#bar` (`data-nofocus`, with a handler) holding `#nfin`, a text field.
fn mount() -> Fixture {
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
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute("style", "width: 300px; margin: 20px").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let el = |scope: &mut RenderScope, tag: &str, id: &str, handler: bool| -> NodeHandle {
                let e = scope.create_element(tag);
                e.set_attribute("id", id);
                e.append_child(&scope.create_text(id));
                if handler {
                    let sink = sink.clone();
                    let name = id.to_string();
                    let rid = scope.register_handler(move || sink.borrow_mut().push(name.clone()));
                    e.set_attribute("data-rid", &rid.0.to_string());
                }
                e
            };
            let field = scope.create_element("input");
            field.set_attribute("id", "field");
            root.append_child(&field);

            let row = el(scope, "div", "row", false);
            row.set_attribute("tabindex", "0");
            let dis = el(scope, "button", "dis", true);
            dis.set_attribute("disabled", "");
            row.append_child(&dis);
            root.append_child(&row);

            let cerow = el(scope, "div", "cerow", false);
            cerow.set_attribute("tabindex", "0");
            let cef = el(scope, "span", "cef", true);
            cef.set_attribute("contenteditable", "false");
            cerow.append_child(&cef);
            root.append_child(&cerow);

            let ce = el(scope, "div", "ce", false);
            ce.set_attribute("contenteditable", "");
            ce.append_child(&el(scope, "span", "cespan", true));
            root.append_child(&ce);

            let bar = el(scope, "div", "bar", true);
            bar.set_attribute("data-nofocus", "");
            let nfin = scope.create_element("input");
            nfin.set_attribute("id", "nfin");
            bar.append_child(&nfin);
            root.append_child(&bar);
            root
        },
    );
    Fixture { root, host, log }
}

impl Fixture {
    fn focus_field(&self) {
        by_id("field").focus().unwrap();
        assert_eq!(active_id(), "field", "positive control: the field is focused");
        self.log.borrow_mut().clear();
    }
    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn by_id(id: &str) -> web_sys::HtmlElement {
    document().get_element_by_id(id).unwrap().dyn_into().unwrap()
}

fn active_id() -> String {
    document().active_element().map(|a| a.id()).unwrap_or_default()
}

fn press(el: &web_sys::Element) -> web_sys::PointerEvent {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(1);
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let down = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    el.dispatch_event(&down).unwrap();
    init.set_buttons(0);
    let up = web_sys::PointerEvent::new_with_event_init_dict("pointerup", &init).unwrap();
    el.dispatch_event(&up).unwrap();
    down
}

/// Finding 2: Chrome 155 (trusted press on a twin with no rinch) focuses the
/// `tabindex` row when the press lands on a disabled button inside it; the
/// PR stops its walk at the disabled button, whose `focus()` is refused.
#[wasm_bindgen_test]
fn a_press_on_a_disabled_button_in_a_tabindex_row_focuses_the_row() {
    let f = mount();
    f.focus_field();
    press(&by_id("dis"));
    assert_eq!(*f.log.borrow(), ["dis"], "positive control: the handler ran");
    assert_eq!(active_id(), "row");
    f.teardown();
}

/// Finding 2: the same for a `contenteditable=false` island, which matches
/// `[contenteditable]` and is not focusable.
#[wasm_bindgen_test]
fn a_press_on_a_non_editable_island_in_a_tabindex_row_focuses_the_row() {
    let f = mount();
    f.focus_field();
    press(&by_id("cef"));
    assert_eq!(*f.log.borrow(), ["cef"]);
    assert_eq!(active_id(), "cerow");
    f.teardown();
}

/// Pins the `in_content_editable` arm of `press_focus_for` (mutant M1): a
/// handler inside editable content is left to the browser.
#[wasm_bindgen_test]
fn a_press_on_a_handler_inside_editable_content_is_left_to_the_browser() {
    let f = mount();
    f.focus_field();
    let ev = press(&by_id("cespan"));
    assert_eq!(*f.log.borrow(), ["cespan"]);
    assert!(!ev.default_prevented());
    f.teardown();
}

/// #1579 (finding 4): a text field inside a `data-nofocus` region is focused
/// by a click on desktop; on the web the region's `preventDefault` cancels it.
#[wasm_bindgen_test]
fn a_text_field_inside_a_nofocus_region_is_left_to_the_browser_issue_1579() {
    let f = mount();
    f.focus_field();
    let ev = press(&by_id("nfin"));
    assert_eq!(*f.log.borrow(), ["bar"]);
    assert!(!ev.default_prevented());
    f.teardown();
}
