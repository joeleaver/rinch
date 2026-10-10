//! A `RenderSurface`'s pointer input in the browser with pointer events on
//! (`RenderSurfaceHandle::set_pointer_events`): the device and pressure of a
//! press, two touches and Ctrl+wheel as `Pinch`, and the same input as the
//! mouse events it always was on a surface that did not ask.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test render_surface_pointer_events
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::render_surface::{
    RenderSurface, RenderSurfaceHandle, SurfaceEvent, create_render_surface,
};
use rinch_core::Component;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
    canvas: web_sys::Element,
    seen: Rc<RefCell<Vec<String>>>,
}

/// A 200×100 surface, mounted, its canvas listeners installed.
async fn mount(pointer_events: bool) -> Mounted {
    let surface: RenderSurfaceHandle = create_render_surface();
    surface.set_pointer_events(pointer_events);
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = seen.clone();
    surface.set_event_handler(move |e| {
        let s = match e {
            SurfaceEvent::MouseDown { x, y, .. } => format!("down {x},{y}"),
            SurfaceEvent::MouseMove { x, y } => format!("move {x},{y}"),
            SurfaceEvent::MouseUp { x, y, .. } => format!("up {x},{y}"),
            SurfaceEvent::MouseWheel { delta_y, .. } => format!("wheel {delta_y}"),
            SurfaceEvent::PointerDown { x, y, pointer, .. } => format!(
                "pdown {x},{y} {:?} {} #{}",
                pointer.kind, pointer.pressure, pointer.id
            ),
            SurfaceEvent::PointerMove { x, y, pointer } => {
                format!("pmove {x},{y} {:?} {}", pointer.kind, pointer.pressure)
            }
            SurfaceEvent::PointerUp { x, y, pointer, .. } => {
                format!("pup {x},{y} {:?} {}", pointer.kind, pointer.pressure)
            }
            SurfaceEvent::PointerCancel { pointer } => format!("pcancel #{}", pointer.id),
            SurfaceEvent::Pinch { x, y, scale } => format!("pinch {x},{y} {scale:.3}"),
            _ => return,
        };
        sink.borrow_mut().push(s);
    });
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 50px; top: 50px; width: 200px; height: 100px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let id = surface.id();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            RenderSurface {
                surface: Some(surface.clone()),
            }
            .render(scope, &[])
        },
    );
    // The canvas's listeners are installed from a microtask after mount.
    let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
        &wasm_bindgen::JsValue::NULL,
    ))
    .await;
    let canvas = document()
        .get_element_by_id(&format!("rinch-surface-{id}"))
        .expect("the surface's canvas");
    Mounted {
        root,
        host,
        canvas,
        seen,
    }
}

impl Mounted {
    fn pointer(&self, name: &str, kind: &str, id: i32, (x, y): (i32, i32), pressure: f32) {
        let init = web_sys::PointerEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_pointer_type(kind);
        init.set_pointer_id(id);
        init.set_is_primary(id == 1 || id == 10);
        init.set_pressure(pressure);
        init.set_client_x(x);
        init.set_client_y(y);
        init.set_buttons(if pressure > 0.0 { 1 } else { 0 });
        let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
        self.canvas.dispatch_event(&ev).unwrap();
    }

    fn wheel(&self, (x, y): (i32, i32), delta_y: f64, ctrl: bool) -> bool {
        let init = web_sys::WheelEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_client_x(x);
        init.set_client_y(y);
        init.set_delta_y(delta_y);
        init.set_ctrl_key(ctrl);
        let ev = web_sys::WheelEvent::new_with_event_init_dict("wheel", &init).unwrap();
        self.canvas.dispatch_event(&ev).unwrap();
        ev.default_prevented()
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.seen.borrow_mut())
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

#[wasm_bindgen_test]
async fn a_pen_says_it_is_a_pen_and_how_hard_it_presses() {
    let m = mount(true).await;
    m.pointer("pointerdown", "pen", 7, (60, 70), 0.25);
    m.pointer("pointermove", "pen", 7, (80, 70), 0.75);
    m.pointer("pointerup", "pen", 7, (80, 70), 0.0);
    assert_eq!(
        m.take(),
        [
            "pdown 10,20 Pen 0.25 #7",
            "pmove 30,20 Pen 0.75",
            "pup 30,20 Pen 0",
        ]
    );
    m.teardown();
}

#[wasm_bindgen_test]
async fn without_pointer_events_a_pen_is_the_mouse_events_it_always_was() {
    let m = mount(false).await;
    m.pointer("pointerdown", "pen", 7, (60, 70), 0.25);
    m.pointer("pointermove", "pen", 7, (80, 70), 0.75);
    m.pointer("pointerup", "pen", 7, (80, 70), 0.0);
    assert_eq!(m.take(), ["down 10,20", "move 30,20", "up 30,20"]);
    // And a cancelled press still ends as a release.
    m.pointer("pointerdown", "touch", 10, (60, 70), 0.5);
    m.pointer("pointercancel", "touch", 10, (60, 70), 0.0);
    assert_eq!(m.take(), ["down 10,20", "up 10,20"]);
    m.teardown();
}

#[wasm_bindgen_test]
async fn two_touches_spreading_apart_pinch_and_a_cancel_says_so() {
    let m = mount(true).await;
    m.pointer("pointerdown", "touch", 10, (100, 100), 0.5);
    m.pointer("pointerdown", "touch", 11, (150, 100), 0.5);
    m.take();
    m.pointer("pointermove", "touch", 11, (200, 100), 0.5);
    let got = m.take();
    assert_eq!(
        got.iter()
            .filter(|e| e.starts_with("pinch"))
            .collect::<Vec<_>>(),
        ["pinch 100,50 2.000"],
        "{got:?}"
    );
    m.pointer("pointercancel", "touch", 11, (200, 100), 0.0);
    assert_eq!(m.take(), ["pcancel #11"]);
    m.pointer("pointermove", "touch", 10, (90, 100), 0.5);
    assert!(!m.take().iter().any(|e| e.starts_with("pinch")));
    m.pointer("pointerup", "touch", 10, (90, 100), 0.0);
    m.teardown();
}

/// Ctrl+wheel (a trackpad pinch, as the browser reports one) is a `Pinch`
/// with pointer events and a wheel without; either way the page does not
/// scroll or zoom.
#[wasm_bindgen_test]
async fn ctrl_wheel_is_a_pinch_with_pointer_events() {
    let m = mount(true).await;
    assert!(m.wheel((100, 100), -100.0, true));
    assert!(m.wheel((100, 100), 30.0, false));
    assert_eq!(m.take(), ["pinch 50,50 2.718", "wheel 30"]);
    m.teardown();

    let m = mount(false).await;
    assert!(m.wheel((100, 100), -100.0, true));
    assert_eq!(m.take(), ["wheel -100"]);
    m.teardown();
}
