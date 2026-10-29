//! Issue #2033: a `<label>` with an `onclick` of its own must not suppress a
//! *different* `onclick` above the control it labels.
//!
//! Chrome 156, real mouse on the label of `label { for: c, onclick: A }` beside
//! `div { onclick: B, input#c }`: `click@label | focus@c | click@c` — A, then B
//! (the issue's CDP measurement). Desktop runs A then B once each (#866,
//! `label_activation_866_tests.rs`). rinch-web dispatches `data-rid` from
//! `pointerdown`, and its #272 rule suppresses a trusted click inside a gesture
//! whose press acted; the click the label forwards to `#c` was suppressed with
//! it, so B never ran. The rule the fix keeps is desktop's: a handler the press
//! already ran is not run again — Checkbox/Radio/Switch put their handler on
//! the label that wraps the control, and one press is one toggle.
//!
//! As in `keyboard_activation.rs`, a test can only dispatch untrusted events;
//! `__force_trusted_clicks(true)` makes the gate read them as trusted. The
//! label's activation behaviour (forwarding a click to its control) runs for a
//! dispatched click too, in Chrome and Firefox; each test records the clicks
//! the document sees, so that forward is a measured positive control, not an
//! assumption.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::events::EventHandlerId;
use rinch_web::RootHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-2033";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    /// `click@<id or tag>` for every click the document sees, in order.
    clicks: Rc<RefCell<Vec<String>>>,
    listener: Closure<dyn FnMut(web_sys::Event)>,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        rinch_web::__reset_activation_state();
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
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
        let clicks = Rc::new(RefCell::new(Vec::new()));
        let log = clicks.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let Some(el) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else {
                return;
            };
            let name = if el.id().is_empty() {
                el.tag_name().to_lowercase()
            } else {
                el.id()
            };
            log.borrow_mut().push(format!("click@{name}"));
        });
        document()
            .add_event_listener_with_callback_and_bool(
                "click",
                listener.as_ref().unchecked_ref(),
                true,
            )
            .unwrap();
        rinch_web::__force_trusted_clicks(true);
        Self {
            root,
            host,
            clicks,
            listener,
        }
    }

    fn el(&self, id: &str) -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into()
            .unwrap()
    }

    fn query(&self, selector: &str) -> web_sys::HtmlElement {
        self.host
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no {selector}"))
            .dyn_into()
            .unwrap()
    }

    fn clicks(&self) -> Vec<String> {
        self.clicks.borrow().clone()
    }

    fn teardown(self) {
        let _ = document().remove_event_listener_with_callback_and_bool(
            "click",
            self.listener.as_ref().unchecked_ref(),
            true,
        );
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn counter(scope: &mut RenderScope) -> (EventHandlerId, Rc<Cell<u32>>) {
    let count = Rc::new(Cell::new(0u32));
    let c = count.clone();
    (scope.register_handler(move || c.set(c.get() + 1)), count)
}

fn with_rid(el: &NodeHandle, rid: EventHandlerId) {
    el.set_attribute("data-rid", &rid.0.to_string());
}

fn pointer_event(el: &web_sys::Element, name: &str) {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(if name == "pointerdown" { 1 } else { 0 });
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// A mouse press on `el` as the browser delivers it: `pointerdown`,
/// `pointerup`, then the detail-1 `click` — whose activation behaviour, on a
/// label, forwards a click to the label's control.
fn mouse_press(el: &web_sys::Element) {
    pointer_event(el, "pointerdown");
    pointer_event(el, "pointerup");
    let r = el.get_bounding_client_rect();
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_detail(1);
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

async fn next_task() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

fn checked(el: &web_sys::HtmlElement) -> bool {
    el.dyn_ref::<web_sys::HtmlInputElement>().unwrap().checked()
}

// ── Raw markup ──────────────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn a_label_onclick_and_a_different_onclick_above_its_control_both_run_once() {
    // The issue's markup. Desktop pin:
    // `a_clickable_label_and_its_controls_clickable_both_run_once`.
    let counts: Rc<RefCell<Option<(Rc<Cell<u32>>, Rc<Cell<u32>>)>>> = Rc::default();
    let out = counts.clone();
    let f = Fixture::mount(move |scope| {
        let (a, ca) = counter(scope);
        let (b, cb) = counter(scope);
        *out.borrow_mut() = Some((ca, cb));
        let root = scope.create_element("div");
        let label = scope.create_element("label");
        label.set_attribute("id", "lbl");
        label.set_attribute("for", "c");
        with_rid(&label, a);
        label.append_child(&scope.create_text("label text"));
        root.append_child(&label);
        let wrap = scope.create_element("div");
        wrap.set_attribute("id", "wrap");
        with_rid(&wrap, b);
        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "c");
        wrap.append_child(&input);
        root.append_child(&wrap);
        root
    });
    let (a, b) = counts.borrow().clone().unwrap();
    mouse_press(&f.el("lbl"));
    assert_eq!(
        f.clicks(),
        ["click@lbl", "click@c"],
        "positive control: the label forwarded a click to its control (Chrome 156's sequence)"
    );
    assert!(checked(&f.el("c")), "and the control toggled");
    assert_eq!(a.get(), 1, "the label's own onclick runs once, on the press");
    assert_eq!(
        b.get(),
        1,
        "the different onclick above the control runs once, as in Chrome and on desktop"
    );
    // The gesture is over: the next task's clicks are new interactions.
    next_task().await;
    assert_eq!((a.get(), b.get()), (1, 1));
    f.teardown();
}

#[wasm_bindgen_test]
fn a_label_onclick_wrapping_a_different_onclick_around_the_control_runs_each_once() {
    // `label { onclick: A, div { onclick: B, input } }`. Chrome runs A, B, A
    // (the forward bubbles back through the label); desktop runs the nearest
    // `onclick` above the control unless the press already ran it: A, B.
    let counts: Rc<RefCell<Option<(Rc<Cell<u32>>, Rc<Cell<u32>>)>>> = Rc::default();
    let out = counts.clone();
    let f = Fixture::mount(move |scope| {
        let (a, ca) = counter(scope);
        let (b, cb) = counter(scope);
        *out.borrow_mut() = Some((ca, cb));
        let label = scope.create_element("label");
        label.set_attribute("id", "lbl");
        with_rid(&label, a);
        let text = scope.create_element("span");
        text.set_attribute("id", "txt");
        text.append_child(&scope.create_text("label text"));
        label.append_child(&text);
        let inner = scope.create_element("div");
        with_rid(&inner, b);
        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "c");
        inner.append_child(&input);
        label.append_child(&inner);
        label
    });
    let (a, b) = counts.borrow().clone().unwrap();
    mouse_press(&f.el("txt"));
    assert_eq!(f.clicks(), ["click@txt", "click@c"], "positive control");
    assert_eq!((a.get(), b.get()), (1, 1));
    f.teardown();
}

#[wasm_bindgen_test]
fn the_same_onclick_on_a_label_wrapping_its_control_runs_once() {
    // Checkbox/Radio/Switch's shape, and #272's: the forward resolves to the
    // handler the press already ran.
    let count: Rc<RefCell<Option<Rc<Cell<u32>>>>> = Rc::default();
    let out = count.clone();
    let f = Fixture::mount(move |scope| {
        let (a, ca) = counter(scope);
        *out.borrow_mut() = Some(ca);
        let label = scope.create_element("label");
        label.set_attribute("id", "lbl");
        with_rid(&label, a);
        let text = scope.create_element("span");
        text.set_attribute("id", "txt");
        text.append_child(&scope.create_text("label text"));
        label.append_child(&text);
        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "c");
        label.append_child(&input);
        label
    });
    let a = count.borrow().clone().unwrap();
    mouse_press(&f.el("txt"));
    assert_eq!(f.clicks(), ["click@txt", "click@c"], "positive control");
    assert!(checked(&f.el("c")));
    assert_eq!(a.get(), 1, "one press, one run");
    f.teardown();
}

#[wasm_bindgen_test]
fn the_same_onclick_above_both_a_label_for_and_its_control_runs_once() {
    // `div { onclick: A, label { for: c }, input#c }`: the press on the label
    // text dispatches A; the forward to #c bubbles through A again.
    let count: Rc<RefCell<Option<Rc<Cell<u32>>>>> = Rc::default();
    let out = count.clone();
    let f = Fixture::mount(move |scope| {
        let (a, ca) = counter(scope);
        *out.borrow_mut() = Some(ca);
        let root = scope.create_element("div");
        with_rid(&root, a);
        let label = scope.create_element("label");
        label.set_attribute("id", "lbl");
        label.set_attribute("for", "c");
        label.append_child(&scope.create_text("label text"));
        root.append_child(&label);
        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "c");
        root.append_child(&input);
        root
    });
    let a = count.borrow().clone().unwrap();
    mouse_press(&f.el("lbl"));
    assert_eq!(f.clicks(), ["click@lbl", "click@c"], "positive control");
    assert_eq!(a.get(), 1);
    f.teardown();
}

#[wasm_bindgen_test]
fn a_trailing_click_outside_any_label_after_an_acted_press_stays_suppressed() {
    // The different-handler exception is for a label's forward only: a press
    // on an inner clickable whose trailing click lands on an outer one (a
    // press dragged off the inner) still runs the inner alone, as on desktop.
    let counts: Rc<RefCell<Option<(Rc<Cell<u32>>, Rc<Cell<u32>>)>>> = Rc::default();
    let out = counts.clone();
    let f = Fixture::mount(move |scope| {
        let (a, ca) = counter(scope);
        let (b, cb) = counter(scope);
        *out.borrow_mut() = Some((ca, cb));
        let outer = scope.create_element("div");
        outer.set_attribute("id", "outer");
        with_rid(&outer, b);
        let inner = scope.create_element("button");
        inner.set_attribute("id", "inner");
        with_rid(&inner, a);
        inner.append_child(&scope.create_text("inner"));
        outer.append_child(&inner);
        outer
    });
    let (a, b) = counts.borrow().clone().unwrap();
    pointer_event(&f.el("inner"), "pointerdown");
    pointer_event(&f.el("outer"), "pointerup");
    f.el("outer").click();
    assert_eq!(f.clicks(), ["click@outer"]);
    assert_eq!((a.get(), b.get()), (1, 0));
    f.teardown();
}

// ── Components: one press, one toggle ───────────────────────────────────────

#[wasm_bindgen_test]
fn a_checkbox_toggles_once_per_press_on_its_label_text() {
    let toggles = Rc::new(Cell::new(0u32));
    let t = toggles.clone();
    let f = Fixture::mount(move |__scope| {
        let on = Signal::new(false);
        let t = t.clone();
        rsx! {
            Checkbox {
                label: "Accept",
                checked_fn: move || on.get(),
                onchange: move || { t.set(t.get() + 1); on.update(|v| *v = !*v) },
            }
        }
    });
    mouse_press(&f.query(".rinch-checkbox__label"));
    assert_eq!(f.clicks().len(), 2, "positive control: {:?}", f.clicks());
    assert_eq!(toggles.get(), 1);
    f.teardown();
}

#[wasm_bindgen_test]
fn a_switch_toggles_once_per_press_on_its_label_text() {
    let toggles = Rc::new(Cell::new(0u32));
    let t = toggles.clone();
    let f = Fixture::mount(move |__scope| {
        let on = Signal::new(false);
        let t = t.clone();
        rsx! {
            Switch {
                label: "Wi-Fi",
                checked_fn: move || on.get(),
                onchange: move || { t.set(t.get() + 1); on.update(|v| *v = !*v) },
            }
        }
    });
    mouse_press(&f.query(".rinch-switch__label"));
    assert_eq!(f.clicks().len(), 2, "positive control: {:?}", f.clicks());
    assert_eq!(toggles.get(), 1);
    f.teardown();
}

#[wasm_bindgen_test]
fn a_radio_selects_once_per_press_on_its_label_text() {
    let selects = Rc::new(Cell::new(0u32));
    let s = selects.clone();
    let f = Fixture::mount(move |__scope| {
        let s = s.clone();
        rsx! {
            Radio {
                name: "g2033",
                value: "a",
                label: "Option A",
                onchange: move || s.set(s.get() + 1),
            }
        }
    });
    mouse_press(&f.query(".rinch-radio__label"));
    assert_eq!(f.clicks().len(), 2, "positive control: {:?}", f.clicks());
    assert_eq!(selects.get(), 1);
    f.teardown();
}
