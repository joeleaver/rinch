//! Browser-driven tests for an element's own `onwheel` on the web backend:
//! the element under the pointer first, then its ancestors; a consuming
//! handler stops the walk and cancels the wheel (`preventDefault`, which is
//! what keeps a trusted wheel from scrolling or a pinch from zooming the
//! page); deltas in logical pixels with lines normalized; the modifiers.
//!
//! A synthetic `wheel` has no default action, so "nothing scrolls" is read as
//! the event's `defaultPrevented`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test element_wheel
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-element-wheel-host";

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    log: Log,
    events: Rc<RefCell<Vec<WheelEvent>>>,
    consume: Rc<Cell<bool>>,
}

impl Fixture {
    /// A scroll container whose `onwheel` never consumes, around `inner`,
    /// whose `onwheel` consumes when `consume` is set or Ctrl is held, around
    /// a `leaf` with no handler.
    fn mount() -> Self {
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
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let events: Rc<RefCell<Vec<WheelEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let consume = Rc::new(Cell::new(false));
        let (log_in, events_in, consume_in) = (log.clone(), events.clone(), consume.clone());
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |__scope: &mut RenderScope| {
                let (l1, l2) = (log_in.clone(), log_in.clone());
                let ev = events_in.clone();
                let consume = consume_in.clone();
                rsx! {
                    div {
                        id: "w-scroller",
                        style: "width: 300px; height: 100px; overflow-y: auto",
                        onwheel: move |_w| {
                            l1.borrow_mut().push("scroller".into());
                            false
                        },
                        div {
                            id: "w-inner",
                            style: "height: 400px",
                            onwheel: move |w: &WheelEvent| {
                                l2.borrow_mut().push("inner".into());
                                ev.borrow_mut().push(*w);
                                consume.get() || w.modifiers.ctrl
                            },
                            div { id: "w-leaf", style: "height: 50px" }
                        }
                    }
                }
            },
        );
        Self {
            root,
            host,
            log,
            events,
            consume,
        }
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn take_events(&self) -> Vec<WheelEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

struct Wheel {
    delta_x: f64,
    delta_y: f64,
    mode: u32,
    ctrl: bool,
    shift: bool,
}

impl Default for Wheel {
    fn default() -> Self {
        Self {
            delta_x: 0.0,
            delta_y: 40.0,
            mode: 0,
            ctrl: false,
            shift: false,
        }
    }
}

/// A wheel dispatched at `id`, where the browser sends it: the element under
/// the pointer. Answers whether it was cancelled.
fn wheel_at(id: &str, w: Wheel) -> bool {
    let target = document().get_element_by_id(id).unwrap();
    let init = web_sys::WheelEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(17);
    init.set_client_y(23);
    init.set_delta_x(w.delta_x);
    init.set_delta_y(w.delta_y);
    init.set_delta_mode(w.mode);
    init.set_ctrl_key(w.ctrl);
    init.set_shift_key(w.shift);
    let ev = web_sys::WheelEvent::new_with_event_init_dict("wheel", &init).unwrap();
    target.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

/// Nearest first, and an unconsumed wheel is not cancelled: the browser goes
/// on to scroll.
#[wasm_bindgen_test]
fn onwheel_runs_nearest_first_and_leaves_an_unconsumed_wheel_alone() {
    let f = Fixture::mount();
    let cancelled = wheel_at("w-leaf", Wheel::default());
    assert_eq!(f.take_log(), ["inner", "scroller"]);
    assert!(!cancelled, "nobody consumed it: the browser scrolls");
    f.teardown();
}

/// A consumed wheel stops the walk and is cancelled, so no ancestor scrolls.
#[wasm_bindgen_test]
fn a_consumed_wheel_stops_the_walk_and_is_cancelled() {
    let f = Fixture::mount();
    f.consume.set(true);
    assert!(wheel_at("w-leaf", Wheel::default()));
    assert_eq!(f.take_log(), ["inner"]);

    // A wheel over the scroller itself, outside `inner`, never reaches it.
    f.consume.set(true);
    assert!(!wheel_at("w-scroller", Wheel::default()));
    assert_eq!(f.take_log(), ["scroller"]);
    f.teardown();
}

/// The payload: pixel deltas as given, a line delta at 40 px a line, the
/// pointer in client px, the modifiers — and a Ctrl wheel (how browsers
/// report a trackpad pinch) is consumed here, so the page does not zoom.
#[wasm_bindgen_test]
fn the_wheel_carries_logical_deltas_the_pointer_and_modifiers() {
    let f = Fixture::mount();
    wheel_at(
        "w-leaf",
        Wheel {
            delta_x: 5.0,
            delta_y: -12.5,
            shift: true,
            ..Wheel::default()
        },
    );
    let w = f.take_events()[0];
    assert_eq!((w.delta_x, w.delta_y), (5.0, -12.5));
    assert_eq!((w.x, w.y), (17.0, 23.0));
    assert!(w.modifiers.shift && !w.modifiers.ctrl);

    wheel_at(
        "w-leaf",
        Wheel {
            delta_y: 3.0,
            mode: web_sys::WheelEvent::DOM_DELTA_LINE,
            ..Wheel::default()
        },
    );
    let w = f.take_events()[0];
    assert_eq!(w.delta_y, 120.0, "three lines are 120 px");

    f.take_log();
    let cancelled = wheel_at(
        "w-leaf",
        Wheel {
            delta_y: -10.0,
            ctrl: true,
            ..Wheel::default()
        },
    );
    let w = f.take_events()[0];
    assert!(w.modifiers.ctrl);
    assert!(w.zoom_factor() > 1.0, "a pinch outwards zooms in");
    assert!(cancelled, "the pinch is the app's, not the page zoom");
    assert_eq!(f.take_log(), ["inner"]);
    f.teardown();
}
