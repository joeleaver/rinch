//! The node-graph canvas's wheel on the web backend, over node bodies that
//! hold scroll containers: a plain wheel a list inside a node can follow is
//! left to the browser (not cancelled, so the list scrolls) and the canvas
//! does not pan; one no scroller can follow pans and is cancelled; a list
//! that reaches its end mid-gesture keeps the rest of the gesture (cancelled,
//! no pan) and a new wheel pans; Ctrl+wheel zooms over the list; an element
//! marked `data-graph-nowheel` is left entirely to the browser.
//!
//! A synthetic `wheel` has no default action, so "the browser scrolls" is
//! read as the event not being `defaultPrevented`, and "the list is at its
//! end" is set with `scrollTop`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test graph_canvas_wheel
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_graph_view::{
    AnyConnection, CanvasOptions, GraphState, GraphView, NO_WHEEL_ATTR, Point, Viewport,
    graph_canvas,
};
use rinch_web::RootHandle;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

type View = GraphView<&'static str, (), ()>;

const HOST_MARKER: &str = "data-graph-wheel-host";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn by_id(id: &str) -> web_sys::Element {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("#{id}"))
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    view: View,
}

impl Fixture {
    /// A 600×400 canvas with a list node (`#g-list`: 60px tall, 300px of
    /// rows, `#g-row` inside), an opted-out node (`#g-opted-child` inside a
    /// `data-graph-nowheel` element) and a plain node (`#g-plain`); the
    /// canvas element is `#g-canvas`.
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
        let out: Rc<Cell<Option<View>>> = Rc::new(Cell::new(None));
        let out_in = out.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let view: View = GraphView::new(GraphState::new(), AnyConnection);
                view.edit(|tx| {
                    tx.add_node(Point::new(20.0, 20.0), "list", [])?;
                    tx.add_node(Point::new(240.0, 20.0), "opted", [])?;
                    tx.add_node(Point::new(20.0, 160.0), "plain", [])?;
                    tx.add_node(Point::new(240.0, 160.0), "strip", [])?;
                    tx.add_node(Point::new(420.0, 20.0), "nested", [])?;
                    Ok(())
                })
                .unwrap();
                out_in.set(Some(view));
                let v = view;
                let div = |scope: &mut RenderScope, id: &str, style: &str| {
                    let e = scope.create_element("div");
                    e.set_attribute("id", id);
                    e.set_attribute("style", style);
                    e
                };
                let render = move |scope: &mut RenderScope, id| match v
                    .with_state(|s| s.graph().node(id).unwrap().data)
                {
                    "list" => {
                        let list = div(
                            scope,
                            "g-list",
                            "width: 120px; height: 60px; overflow-y: auto",
                        );
                        list.append_child(&div(scope, "g-row", "height: 300px"));
                        list
                    }
                    "opted" => {
                        let opted = div(scope, "g-opted", "width: 120px; padding: 4px");
                        opted.set_attribute(NO_WHEEL_ATTR, "");
                        opted.append_child(&div(scope, "g-opted-child", "height: 30px"));
                        opted
                    }
                    "strip" => {
                        let strip = div(
                            scope,
                            "g-strip",
                            "width: 120px; height: 40px; overflow-x: auto",
                        );
                        strip.append_child(&div(
                            scope,
                            "g-strip-wide",
                            "width: 400px; height: 20px",
                        ));
                        strip
                    }
                    "nested" => {
                        let outer = div(
                            scope,
                            "g-outer",
                            "width: 120px; height: 80px; overflow-y: auto",
                        );
                        outer.append_child(&div(scope, "g-outer-top", "height: 20px"));
                        let inner = div(
                            scope,
                            "g-inner",
                            "width: 100px; height: 40px; overflow-y: auto",
                        );
                        inner.append_child(&div(scope, "g-inner-row", "height: 200px"));
                        outer.append_child(&inner);
                        outer.append_child(&div(scope, "g-outer-bottom", "height: 100px"));
                        outer
                    }
                    _ => div(scope, "g-plain", "width: 120px; height: 40px"),
                };
                let canvas = graph_canvas(
                    scope,
                    &view,
                    render,
                    CanvasOptions {
                        style: "width: 600px; height: 400px".into(),
                        ..CanvasOptions::default()
                    },
                );
                canvas.set_attribute("id", "g-canvas");
                canvas
            },
        );
        let view = out.take().expect("mounted");
        // The gesture clock at 0 ms; tests move it with `__set_wheel_clock`.
        rinch_graph_view::__set_wheel_clock(Some(0));
        rinch_web::__set_mac_platform(Some(false));
        Self { root, host, view }
    }

    fn viewport(&self) -> Viewport {
        rinch_core::untracked(|| self.view.viewport())
    }

    fn pan(&self) -> (f32, f32) {
        let vp = self.viewport();
        (vp.x, vp.y)
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        rinch_web::__set_mac_platform(None);
        rinch_graph_view::__set_wheel_clock(None);
    }
}

/// A wheel dispatched at `id` (DOM sign: positive `delta_y` scrolls down).
/// Answers whether it was cancelled.
fn wheel_at(id: &str, delta_x: f64, delta_y: f64, ctrl: bool) -> bool {
    let init = web_sys::WheelEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(40);
    init.set_client_y(40);
    init.set_delta_x(delta_x);
    init.set_delta_y(delta_y);
    init.set_ctrl_key(ctrl);
    let ev = web_sys::WheelEvent::new_with_event_init_dict("wheel", &init).unwrap();
    by_id(id).dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

/// A wheel over a list row the list can follow is the browser's: not
/// cancelled, and the canvas does not pan. Up, at the list's top, it pans.
#[wasm_bindgen_test]
fn a_list_in_a_node_takes_the_wheel_it_can_follow() {
    let f = Fixture::mount();
    assert!(by_id("g-list").scroll_height() > by_id("g-list").client_height());
    assert!(!wheel_at("g-row", 0.0, 40.0, false), "left to the browser");
    assert_eq!(f.pan(), (0.0, 0.0));
    f.teardown();

    let f = Fixture::mount();
    assert!(wheel_at("g-row", 0.0, -40.0, false), "the canvas took it");
    assert_eq!(f.pan(), (0.0, 40.0));
    f.teardown();
}

/// At its end mid-gesture the list keeps the rest of the gesture: cancelled
/// (nothing else scrolls) and no pan. A new wheel pans.
#[wasm_bindgen_test]
fn at_the_end_the_gesture_stops_and_a_new_wheel_pans() {
    let f = Fixture::mount();
    assert!(!wheel_at("g-row", 0.0, 40.0, false));
    let list = by_id("g-list");
    list.set_scroll_top(list.scroll_height() - list.client_height());
    assert!(wheel_at("g-row", 0.0, 40.0, false), "cancelled");
    assert_eq!(f.pan(), (0.0, 0.0), "the rest of the gesture does not pan");

    rinch_graph_view::__set_wheel_clock(Some(600));
    assert!(wheel_at("g-row", 0.0, 40.0, false));
    assert_eq!(f.pan(), (0.0, -40.0), "a new wheel pans");
    f.teardown();
}

/// Ctrl+wheel (a pinch, as browsers send it) zooms over the list.
#[wasm_bindgen_test]
fn ctrl_wheel_zooms_over_a_list() {
    let f = Fixture::mount();
    // Down, the way the list could scroll: zooms out, not scrolls.
    assert!(wheel_at("g-row", 0.0, 50.0, true));
    assert!(f.viewport().zoom < 1.0, "{:?}", f.viewport());
    f.teardown();
}

/// Inside `data-graph-nowheel` the canvas neither pans nor zooms, and the
/// wheel is not cancelled: the browser handles it.
#[wasm_bindgen_test]
fn an_opted_out_element_is_left_to_the_browser() {
    let f = Fixture::mount();
    assert!(!wheel_at("g-opted-child", 0.0, 40.0, false));
    assert!(!wheel_at("g-opted-child", 0.0, -50.0, true));
    assert_eq!(f.pan(), (0.0, 0.0));
    assert_eq!(f.viewport().zoom, 1.0);
    f.teardown();
}

/// Over a plain body and the background the canvas pans and cancels, so the
/// page never scrolls.
#[wasm_bindgen_test]
fn a_plain_body_and_the_background_pan() {
    let f = Fixture::mount();
    assert!(wheel_at("g-plain", 0.0, 40.0, false));
    assert!(wheel_at("g-canvas", 25.0, 0.0, false));
    assert_eq!(f.pan(), (-25.0, -40.0));
    f.teardown();
}

/// A gesture the canvas pans with stays the canvas's: a wheel 40 ms later
/// over a list is not left to the list (review of #2084). A new gesture is.
#[wasm_bindgen_test]
fn a_panning_gesture_is_not_taken_over_by_a_list() {
    let f = Fixture::mount();
    assert!(wheel_at("g-canvas", 40.0, 0.0, false));
    rinch_graph_view::__set_wheel_clock(Some(40));
    assert!(wheel_at("g-row", 0.0, 40.0, false), "the canvas kept it");
    assert_eq!(f.pan(), (-40.0, -40.0));
    rinch_graph_view::__set_wheel_clock(Some(600));
    assert!(
        !wheel_at("g-row", 0.0, 40.0, false),
        "a new gesture: the list's"
    );
    assert_eq!(f.pan(), (-40.0, -40.0));
    f.teardown();
}

/// A diagonal wheel over a strip that scrolls only sideways is left to the
/// browser, which gives it to the strip whole (Chrome and Firefox, measured
/// with trusted wheels: the page does not move).
#[wasm_bindgen_test]
fn a_diagonal_wheel_over_a_strip_is_the_strips() {
    let f = Fixture::mount();
    assert!(by_id("g-strip").scroll_width() > by_id("g-strip").client_width());
    assert!(!wheel_at("g-strip-wide", 40.0, 6.0, false));
    assert_eq!(f.pan(), (0.0, 0.0));
    f.teardown();
}

/// A list at its end inside a scroller that can still move: the canvas
/// leaves the wheel to the browser, which chains it to the outer scroller.
#[wasm_bindgen_test]
fn a_list_at_its_end_inside_a_movable_scroller_is_left_to_the_browser() {
    let f = Fixture::mount();
    let inner = by_id("g-inner");
    inner.set_scroll_top(inner.scroll_height());
    assert!(!wheel_at("g-inner-row", 0.0, 40.0, false));
    assert_eq!(f.pan(), (0.0, 0.0));
    // With the outer one at its end too, a new wheel pans the canvas.
    let outer = by_id("g-outer");
    outer.set_scroll_top(outer.scroll_height());
    rinch_graph_view::__set_wheel_clock(Some(600));
    assert!(wheel_at("g-inner-row", 0.0, 40.0, false));
    assert_eq!(f.pan(), (0.0, -40.0));
    f.teardown();
}

/// Shift turns a vertical wheel sideways off macOS only
/// (`WheelEvent::scroll_deltas`, set from `navigator.platform`): on macOS
/// the canvas pans down, elsewhere sideways.
#[wasm_bindgen_test]
fn shift_turns_the_canvas_pan_sideways_off_macos_only() {
    let shift_wheel = |id: &str| {
        let init = web_sys::WheelEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_delta_y(40.0);
        init.set_shift_key(true);
        let ev = web_sys::WheelEvent::new_with_event_init_dict("wheel", &init).unwrap();
        by_id(id).dispatch_event(&ev).unwrap();
    };
    let f = Fixture::mount();
    shift_wheel("g-canvas");
    assert_eq!(f.pan(), (-40.0, 0.0), "Linux / Windows: sideways");
    f.teardown();

    let f = Fixture::mount();
    rinch_web::__set_mac_platform(Some(true));
    shift_wheel("g-canvas");
    rinch_web::__set_mac_platform(None);
    assert_eq!(f.pan(), (0.0, -40.0), "macOS: down");
    f.teardown();
}
