//! A `RenderSurface` whose canvas is renewed (#2813): after the page's WebGPU
//! device is lost for good, a surface that drew GPU frames gets a fresh
//! `<canvas>` (`renew_gpu_canvas`, #1768). That needs a WebGPU adapter, which
//! headless CI has none of, so these drive the renewal itself through the
//! test hook `RenderSurfaceHandle::__renew_canvas`, which the GPU path calls.
//!
//! - A press the old canvas held ends with the renewal, as a press whose
//!   release never arrives does: the old canvas's pointer capture and
//!   listeners went with it, so its release would never reach the surface.
//! - A bounds signal taken on the canvas follows it to the new one.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test render_surface_renewal_2813
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::render_surface::{
    RenderSurface, RenderSurfaceHandle, SurfaceEvent, create_render_surface,
};
use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn microtask() {
    let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
        &wasm_bindgen::JsValue::NULL,
    ))
    .await;
}

async fn frames(n: usize) {
    for _ in 0..n {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .request_animation_frame(resolve.unchecked_ref())
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }
}

struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
    surface: RenderSurfaceHandle,
    node: NodeHandle,
    seen: Rc<RefCell<Vec<String>>>,
}

/// A 200×100 surface at (50, 50), mounted, its canvas listeners installed.
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
            SurfaceEvent::PointerDown { x, y, pointer, .. } => {
                format!("pdown {x},{y} #{}", pointer.id)
            }
            SurfaceEvent::PointerMove { x, y, pointer } => format!("pmove {x},{y} #{}", pointer.id),
            SurfaceEvent::PointerUp { x, y, pointer, .. } => format!("pup {x},{y} #{}", pointer.id),
            SurfaceEvent::PointerCancel { pointer } => format!("pcancel #{}", pointer.id),
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
    let node: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let node_in = node.clone();
    let s = surface.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let n = RenderSurface {
                surface: Some(s.clone()),
            }
            .render(scope, &[]);
            *node_in.borrow_mut() = Some(n.clone());
            n
        },
    );
    microtask().await;
    let node = node.borrow().clone().expect("the canvas node");
    Mounted {
        root,
        host,
        surface,
        node,
        seen,
    }
}

impl Mounted {
    /// The surface's canvas, looked up again: it may be a new element.
    fn canvas(&self) -> web_sys::HtmlElement {
        document()
            .get_element_by_id(&format!("rinch-surface-{}", self.surface.id()))
            .expect("the surface's canvas")
            .unchecked_into()
    }

    fn pointer(&self, name: &str, id: i32, (x, y): (i32, i32), buttons: u16) {
        let init = web_sys::PointerEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_pointer_type("mouse");
        init.set_pointer_id(id);
        init.set_is_primary(true);
        init.set_pressure(if buttons != 0 { 0.5 } else { 0.0 });
        init.set_client_x(x);
        init.set_client_y(y);
        init.set_buttons(buttons);
        let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
        self.canvas().dispatch_event(&ev).unwrap();
    }

    /// Renew the canvas and let the renewal's microtasks run.
    async fn renew(&self) {
        let old = self.canvas();
        assert!(self.surface.__renew_canvas(), "the canvas was renewed");
        assert!(
            !self.canvas().is_same_node(Some(&old)),
            "the surface has a new canvas"
        );
        microtask().await;
        microtask().await;
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.seen.borrow_mut())
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// A press held when the canvas is renewed ends with a `PointerCancel`; the
/// new canvas then takes input (its listeners are installed), and its
/// release with no press of its own is delivered as any release is.
#[wasm_bindgen_test]
async fn a_renewal_mid_press_cancels_the_press() {
    let m = mount(true).await;
    m.pointer("pointerdown", 3, (60, 70), 1);
    m.pointer("pointermove", 3, (90, 75), 1);
    assert_eq!(m.take(), ["pdown 10,20 #3", "pmove 40,25 #3"]);
    m.renew().await;
    assert_eq!(m.take(), ["pcancel #3"], "the held press ended");
    // The new canvas hears input.
    m.pointer("pointermove", 3, (95, 75), 1);
    assert_eq!(m.take(), ["pmove 45,25 #3"]);
    // Renewed again with nothing held: nothing is ended twice.
    m.renew().await;
    assert_eq!(m.take(), Vec::<String>::new());
    m.teardown();
}

/// Without pointer events the held press ends as a `MouseUp` where the
/// pointer last was, as a cancelled press does.
#[wasm_bindgen_test]
async fn without_pointer_events_a_renewal_mid_press_releases_at_the_last_position() {
    let m = mount(false).await;
    m.pointer("pointerdown", 1, (60, 70), 1);
    m.pointer("pointermove", 1, (130, 110), 1);
    assert_eq!(m.take(), ["down 10,20", "move 80,60"]);
    m.renew().await;
    assert_eq!(m.take(), ["up 80,60"]);
    m.teardown();
}

/// A press released before the renewal is not held: nothing is ended.
#[wasm_bindgen_test]
async fn a_released_press_is_not_ended_again() {
    let m = mount(true).await;
    m.pointer("pointerdown", 4, (60, 70), 1);
    m.pointer("pointerup", 4, (60, 70), 0);
    assert_eq!(m.take(), ["pdown 10,20 #4", "pup 10,20 #4"]);
    m.renew().await;
    assert_eq!(m.take(), Vec::<String>::new());
    m.teardown();
}

/// A bounds signal taken on the canvas follows it to the new one.
#[wasm_bindgen_test]
async fn the_canvas_bounds_signal_follows_the_new_canvas() {
    let m = mount(true).await;
    let scope = rinch_core::reactive::Scope::new();
    let sig = scope.run(|| m.node.bounds_signal());
    frames(2).await;
    assert_eq!(sig.get().width, 200.0, "the first rect");
    m.renew().await;
    frames(2).await;
    // The host keeps its size; only the new canvas changes.
    m.canvas().style().set_property("width", "123px").unwrap();
    frames(3).await;
    assert_eq!(
        sig.get().width,
        123.0,
        "the signal follows the new canvas's resize"
    );
    scope.dispose();
    m.teardown();
}
