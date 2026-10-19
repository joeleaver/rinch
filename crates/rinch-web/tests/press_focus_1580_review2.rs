//! Review-2 fixtures (not the author's) for #1146 (PR #1580's review): touch presses, the
//! scroll a press must not make, a `<select>` inside a clickable, a draggable
//! tap inside a `data-nofocus` region, and `PasswordInput`'s visibility toggle.
//!
//! Synthetic presses make the browser focus nothing, so every focus asserted
//! here is one rinch made, and a press left to the browser is pinned at
//! `defaultPrevented`. The positive control of each fixture is that the
//! handler ran (a genuine event reached rinch's delegation).
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

const HOST_MARKER: &str = "data-test-host-1580-review2";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    log: Log,
}

/// `#field`, `#btn` (handler), `#track` (a `tabindex` div whose handler arms
/// `Drag::percent`), `#plain` (a non-focusable div with no handler), and
/// `#nfin`, an `<input data-nofocus>` with a handler on its wrapper `#nfwrap`.
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
    host.set_attribute("style", "width: 300px; margin: 20px")
        .unwrap();
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
                if tag != "input" {
                    e.append_child(&scope.create_text(id));
                }
                if handler {
                    let sink = sink.clone();
                    let name = id.to_string();
                    let rid = scope.register_handler(move || sink.borrow_mut().push(name.clone()));
                    e.set_attribute("data-rid", &rid.0.to_string());
                }
                e
            };
            root.append_child(&el(scope, "input", "field", false));
            root.append_child(&el(scope, "button", "btn", true));
            let track = el(scope, "div", "track", false);
            track.set_attribute("tabindex", "0");
            track.set_attribute("style", "width: 200px; height: 20px");
            {
                let sink = sink.clone();
                let rid = scope.register_handler(move || {
                    sink.borrow_mut().push("track".into());
                    rinch_core::Drag::percent().on_move(|_, _| {}).start();
                });
                track.set_attribute("data-rid", &rid.0.to_string());
            }
            root.append_child(&track);
            root.append_child(&el(scope, "div", "plain", false));
            let wrap = el(scope, "div", "nfwrap", true);
            let nfin = el(scope, "input", "nfin", false);
            nfin.set_attribute("data-nofocus", "");
            wrap.append_child(&nfin);
            root.append_child(&wrap);
            root
        },
    );
    Fixture { root, host, log }
}

impl Fixture {
    fn focus(&self, id: &str) {
        by_id(id).focus().unwrap();
        assert_eq!(active_id(), id, "positive control: #{id} is focused");
        self.log.borrow_mut().clear();
    }
    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn by_id(id: &str) -> web_sys::HtmlElement {
    document()
        .get_element_by_id(id)
        .unwrap()
        .dyn_into()
        .unwrap()
}

fn active_id() -> String {
    document()
        .active_element()
        .map(|a| a.id())
        .unwrap_or_default()
}

fn pointer(
    el: &web_sys::Element,
    kind: &str,
    pointer_type: &str,
    dy: f64,
) -> web_sys::PointerEvent {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(if pointer_type == "mouse" { 1 } else { 7 });
    init.set_is_primary(true);
    init.set_pointer_type(pointer_type);
    init.set_button(if kind == "pointermove" { -1 } else { 0 });
    init.set_buttons(if kind == "pointerdown" || kind == "pointermove" {
        1
    } else {
        0
    });
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0 + dy) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

fn down(el: &web_sys::Element, pointer_type: &str) -> web_sys::PointerEvent {
    pointer(el, "pointerdown", pointer_type, 0.0)
}

fn up(el: &web_sys::Element, pointer_type: &str) {
    pointer(el, "pointerup", pointer_type, 0.0);
}

/// A finger dragging a slider: the handler armed a pointer-capture `Drag` on
/// contact, so a move past the scroll slop is the drag, not a pan, and the
/// track takes the focus on release as a mouse press gives it at once.
/// Kills the `&& !drag_active` guard's removal (review 2, mutant J).
#[wasm_bindgen_test]
fn a_finger_dragging_a_slider_focuses_the_track_on_release() {
    let f = mount();
    f.focus("field");
    let track = by_id("track");
    down(&track, "touch");
    assert_eq!(
        *f.log.borrow(),
        ["track"],
        "positive control: the handler ran"
    );
    assert!(
        rinch_core::Drag::is_active(),
        "positive control: the drag is armed"
    );
    pointer(&track, "pointermove", "touch", 40.0);
    up(&track, "touch");
    assert_eq!(active_id(), "track");
    f.teardown();
}

/// A held-back touch focus whose release was missed does not outlive the next
/// primary press: a later release of the same pointer id (a pen's is stable)
/// must not focus the old target. Kills dropping the clear on a primary
/// `pointerdown` (review 2, mutant G).
#[wasm_bindgen_test]
fn a_stale_touch_focus_is_dropped_by_the_next_primary_press() {
    let f = mount();
    f.focus("field");
    down(&by_id("btn"), "pen");
    assert_eq!(
        *f.log.borrow(),
        ["btn"],
        "positive control: the handler ran"
    );
    // The release never arrives; the next press lands on nothing focusable.
    let plain = by_id("plain");
    down(&plain, "pen");
    up(&plain, "pen");
    assert_eq!(active_id(), "field");
    f.teardown();
}

/// A field that carries `data-nofocus` itself is the region, not a field
/// inside it: the press is prevented and moves no focus, as desktop's claim
/// walk reads `data-nofocus` on the node before its tag (`resolve_click_focus`
/// rule 1). Kills dropping `!t.is_same_node(region)` (review 2, mutant E).
#[wasm_bindgen_test]
fn a_field_that_is_itself_the_nofocus_region_is_prevented() {
    let f = mount();
    f.focus("field");
    let ev = down(&by_id("nfin"), "mouse");
    assert_eq!(
        *f.log.borrow(),
        ["nfwrap"],
        "positive control: the handler ran"
    );
    assert!(ev.default_prevented());
    assert_eq!(active_id(), "field");
    f.teardown();
}
