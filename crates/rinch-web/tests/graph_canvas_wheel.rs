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

/// Resolve after `ms` of wall clock.
async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
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
async fn at_the_end_the_gesture_stops_and_a_new_wheel_pans() {
    let f = Fixture::mount();
    assert!(!wheel_at("g-row", 0.0, 40.0, false));
    let list = by_id("g-list");
    list.set_scroll_top(list.scroll_height() - list.client_height());
    assert!(wheel_at("g-row", 0.0, 40.0, false), "cancelled");
    assert_eq!(f.pan(), (0.0, 0.0), "the rest of the gesture does not pan");

    sleep_ms(600).await;
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
