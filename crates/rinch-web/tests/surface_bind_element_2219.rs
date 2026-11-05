//! `RenderSurfaceHandle::bind_element` in the browser (issue #2219): a surface
//! whose element the app built locks that element, through the host
//! `rinch-web` installs at mount, and the lock ends with the binding.
//!
//! As in `pointer_lock_2092.rs`, the browser's half is played by the test
//! (`document.pointerLockElement` overridden, `pointerlockchange`
//! dispatched, `exitPointerLock` counted), and here the element's
//! `requestPointerLock` is counted too, to see which element was asked.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test surface_bind_element_2219
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::render_surface::{
    BindElementOptions, LockOnRebind, RenderSurfaceHandle, SurfaceEvent, create_render_surface,
};
use rinch::window::{PointerLockMode, pointer_lock};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;
type Slot = Rc<RefCell<Option<NodeHandle>>>;

struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
    /// The app's two canvases, as rinch nodes and as the browser's elements.
    a: NodeHandle,
    b: NodeHandle,
    el_a: web_sys::Element,
    el_b: web_sys::Element,
}

/// Two `<canvas>` elements the app built itself, mounted; with `bind`, the
/// component binds `surface` to the first as it renders.
async fn mount(surface: &RenderSurfaceHandle, bind: bool) -> Mounted {
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 40px; top: 40px; width: 200px; height: 100px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let (sa, sb): (Slot, Slot) = (Rc::default(), Rc::default());
    let (sf, ka, kb) = (surface.clone(), sa.clone(), sb.clone());
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| -> NodeHandle {
            let root = scope.create_element("div");
            let a = scope.create_element("canvas");
            a.set_attribute("id", "bind-2219-a");
            let b = scope.create_element("canvas");
            b.set_attribute("id", "bind-2219-b");
            root.append_child(&a);
            root.append_child(&b);
            if bind {
                sf.bind_element(&a);
            }
            *ka.borrow_mut() = Some(a);
            *kb.borrow_mut() = Some(b);
            root
        },
    );
    let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL)).await;
    let a = sa.borrow().clone().unwrap();
    let b = sb.borrow().clone().unwrap();
    Mounted {
        root,
        host,
        a,
        b,
        el_a: document().get_element_by_id("bind-2219-a").unwrap(),
        el_b: document().get_element_by_id("bind-2219-b").unwrap(),
    }
}

impl Mounted {
    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn hear(surface: &RenderSurfaceHandle) -> Log {
    let log: Log = Rc::default();
    let sink = log.clone();
    surface.set_event_handler(move |e| {
        if let SurfaceEvent::PointerMotion { dx, dy } = e {
            sink.borrow_mut().push(format!("surface {dx},{dy}"));
        }
    });
    log
}

/// Play the browser: `document.pointerLockElement` answers `element`, and a
/// `pointerlockchange` says it changed.
fn browser_holds(element: Option<&web_sys::Element>) {
    let getter = match element {
        Some(el) => {
            let el = el.clone();
            Closure::<dyn Fn() -> JsValue>::new(move || el.clone().into())
        }
        None => Closure::<dyn Fn() -> JsValue>::new(|| JsValue::NULL),
    };
    let descriptor = js_sys::Object::new();
    js_sys::Reflect::set(&descriptor, &"get".into(), getter.as_ref()).unwrap();
    js_sys::Reflect::set(&descriptor, &"configurable".into(), &JsValue::TRUE).unwrap();
    js_sys::Object::define_property(&document(), &"pointerLockElement".into(), &descriptor);
    getter.forget();
    let ev = web_sys::Event::new("pointerlockchange").unwrap();
    document().dispatch_event(&ev).unwrap();
}

fn browser_restored() {
    js_sys::Reflect::delete_property(&document(), &"pointerLockElement".into()).unwrap();
}

/// Count `document.exitPointerLock()` calls, and play the browser's answer.
fn spy_exit() -> Rc<Cell<u32>> {
    let count = Rc::new(Cell::new(0));
    let c = count.clone();
    let exit = Closure::<dyn Fn()>::new(move || {
        c.set(c.get() + 1);
        browser_holds(None);
    });
    js_sys::Reflect::set(&document(), &"exitPointerLock".into(), exit.as_ref()).unwrap();
    exit.forget();
    count
}

fn unspy_exit() {
    js_sys::Reflect::delete_property(&document(), &"exitPointerLock".into()).unwrap();
}

/// Count `element.requestPointerLock()` calls (the request is not sent).
fn spy_request(element: &web_sys::Element) -> Rc<Cell<u32>> {
    let count = Rc::new(Cell::new(0));
    let c = count.clone();
    let request = Closure::<dyn Fn()>::new(move || c.set(c.get() + 1));
    js_sys::Reflect::set(element, &"requestPointerLock".into(), request.as_ref()).unwrap();
    request.forget();
    count
}

fn mouse_move(target: &web_sys::Element, dx: i32, dy: i32) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_movement_x(dx);
    init.set_movement_y(dy);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("mousemove", &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// The issue's case: unbound, the surface has no element; bound to the app's
/// canvas, its request asks the browser to lock that canvas, the grant is the
/// surface's, and the motion reaches its handler (the handle was never
/// mounted by the `RenderSurface` component). Unbinding exits the lock.
#[wasm_bindgen_test]
async fn a_surface_locks_the_canvas_the_app_bound_and_unbinding_exits() {
    let surface = create_render_surface();
    let log = hear(&surface);
    let m = mount(&surface, false).await;
    let exits = spy_exit();
    let asked_a = spy_request(&m.el_a);
    assert!(
        surface.request_pointer_lock().is_err(),
        "the issue: no element"
    );

    surface.bind_element(&m.a);
    surface.request_pointer_lock().unwrap();
    assert_eq!(
        asked_a.get(),
        1,
        "the browser was asked for the app's canvas"
    );
    browser_holds(Some(&m.el_a));
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    assert!(surface.has_pointer_lock());
    mouse_move(&m.el_a, 6, -3);
    assert_eq!(*log.borrow(), ["surface 6,-3"]);

    surface.unbind_element();
    assert_eq!(exits.get(), 1, "unbinding exited the lock");
    assert_eq!(pointer_lock(), None);
    assert!(!surface.has_pointer_lock());
    unspy_exit();
    browser_restored();
    m.teardown();
}

/// Re-binding while locked exits by default; `LockOnRebind::Move` asks the
/// browser for the new element without exiting first, and its grant keeps
/// the lock the surface's.
#[wasm_bindgen_test]
async fn rebinding_exits_or_moves_the_lock() {
    let surface = create_render_surface();
    let m = mount(&surface, false).await;
    let exits = spy_exit();
    let (asked_a, asked_b) = (spy_request(&m.el_a), spy_request(&m.el_b));

    surface.bind_element(&m.a);
    surface.request_pointer_lock().unwrap();
    browser_holds(Some(&m.el_a));
    surface.bind_element(&m.b);
    assert_eq!(exits.get(), 1, "re-bound: exited");
    assert_eq!(pointer_lock(), None);

    surface.bind_element(&m.a);
    surface.request_pointer_lock().unwrap();
    browser_holds(Some(&m.el_a));
    assert_eq!(asked_a.get(), 2);
    surface
        .bind_element_with(
            &m.b,
            BindElementOptions::new().lock_on_rebind(LockOnRebind::Move),
        )
        .unwrap();
    assert_eq!(exits.get(), 1, "a move does not exit");
    assert_eq!(
        asked_b.get(),
        1,
        "the browser was asked for the new element"
    );
    browser_holds(Some(&m.el_b));
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    assert!(surface.has_pointer_lock());
    rinch::window::exit_pointer_lock();
    unspy_exit();
    browser_restored();
    m.teardown();
}

/// A binding made while the component rendered goes with it: the unmount
/// exits the lock it holds.
#[wasm_bindgen_test]
async fn a_binding_made_while_rendering_exits_on_unmount() {
    let surface = create_render_surface();
    let m = mount(&surface, true).await;
    let exits = spy_exit();
    let asked_a = spy_request(&m.el_a);
    surface.request_pointer_lock().unwrap();
    assert_eq!(asked_a.get(), 1);
    browser_holds(Some(&m.el_a));
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    let Mounted { root, host, .. } = m;
    root.unmount();
    assert_eq!(exits.get(), 1, "the unmount exited it");
    assert_eq!(pointer_lock(), None);
    assert!(surface.element().is_none(), "unbound");
    host.remove();
    unspy_exit();
    browser_restored();
}

/// The handle's last clone dropping exits the lock its surface holds.
#[wasm_bindgen_test]
async fn dropping_the_handle_exits_its_lock() {
    let mounted_with = create_render_surface();
    let m = mount(&mounted_with, false).await;
    let exits = spy_exit();
    let _asked = spy_request(&m.el_b);
    let surface = create_render_surface();
    surface.bind_element(&m.b);
    surface.request_pointer_lock().unwrap();
    browser_holds(Some(&m.el_b));
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    drop(surface);
    assert_eq!(exits.get(), 1);
    assert_eq!(pointer_lock(), None);
    unspy_exit();
    browser_restored();
    m.teardown();
}
