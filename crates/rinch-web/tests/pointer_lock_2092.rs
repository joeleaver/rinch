//! `rinch::window`'s pointer lock in the browser (issue #2092), through the
//! host `rinch-web` installs at mount.
//!
//! A real lock needs a user gesture, which a script cannot make: the trusted
//! click that grants one is in `pointer_lock_real_2092.rs` (Chrome, through
//! `support/run_real_keys.sh`). Here the browser's half is played by the test:
//! `document.pointerLockElement` is overridden to say what the browser would
//! hold, `pointerlockchange` is dispatched, and `mousemove` carries a
//! `movementX`/`Y` — which is exactly what rinch reads, in Chrome and in
//! Firefox. What is not played is the request itself: an ungestured
//! `requestPointerLock` is sent for real, and refused for real.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test pointer_lock_2092
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::render_surface::{
    RenderSurface, RenderSurfaceHandle, SurfaceEvent, create_render_surface,
};
use rinch::window::{PointerLockMode, exit_pointer_lock, on_pointer_motion, pointer_lock};
use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn sleep(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
}

type Log = Rc<RefCell<Vec<String>>>;

struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
    surface: RenderSurfaceHandle,
    canvas: web_sys::Element,
    /// What the surface and the motion handler heard.
    log: Log,
    /// The text of `p#lock-state`: `pointer_lock()`, through a `{|| …}`.
    state: web_sys::Element,
}

/// A surface and `p#lock-state`, mounted; a motion handler registered by the
/// component.
async fn mount() -> Mounted {
    let surface = create_render_surface();
    let log: Log = Rc::default();
    let sink = log.clone();
    surface.set_event_handler(move |e| match e {
        SurfaceEvent::PointerMotion { dx, dy } => {
            sink.borrow_mut().push(format!("surface {dx},{dy}"))
        }
        SurfaceEvent::MouseMove { .. } => sink.borrow_mut().push("surface move".into()),
        _ => {}
    });
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 40px; top: 40px; width: 200px; height: 100px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let (sf, sink) = (surface.clone(), log.clone());
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| -> NodeHandle {
            let sink = sink.clone();
            let _ = on_pointer_motion(move |m| {
                sink.borrow_mut().push(format!("handler {},{}", m.dx, m.dy))
            });
            let root = scope.create_element("div");
            let p = scope.create_element("p");
            p.set_attribute("id", "lock-state");
            let text = scope.create_text("");
            p.append_child(&text);
            scope.create_effect(move || text.set_text(&format!("{:?}", pointer_lock())));
            root.append_child(&p);
            let canvas = RenderSurface {
                surface: Some(sf.clone()),
            }
            .render(scope, &[]);
            root.append_child(&canvas);
            root
        },
    );
    let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL)).await;
    let canvas = document()
        .get_element_by_id(&format!("rinch-surface-{}", surface.id()))
        .expect("the surface's canvas");
    let state = document().get_element_by_id("lock-state").unwrap();
    Mounted {
        root,
        host,
        surface,
        canvas,
        log,
        state,
    }
}

impl Mounted {
    fn shown(&self) -> String {
        self.state.text_content().unwrap_or_default()
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
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

/// Give `document.pointerLockElement` back to the browser.
fn browser_restored() {
    js_sys::Reflect::delete_property(&document(), &"pointerLockElement".into()).unwrap();
}

/// Count `document.exitPointerLock()` calls, and play the browser's answer:
/// the lock is gone.
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

fn mouse_move(target: &web_sys::Element, dx: i32, dy: i32) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_movement_x(dx);
    init.set_movement_y(dy);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("mousemove", &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// Without a user gesture the browser refuses for real: the request is
/// pending (`Ok`, nothing granted), then the refusal ends it.
#[wasm_bindgen_test]
async fn an_ungestured_request_is_refused_by_the_browser() {
    let m = mount().await;
    assert_eq!(m.surface.request_pointer_lock(), Ok(()));
    assert!(m.surface.has_pointer_lock(), "pending");
    assert_eq!(pointer_lock(), None, "nothing granted yet");
    for _ in 0..50 {
        if !m.surface.has_pointer_lock() {
            break;
        }
        sleep(20).await;
    }
    assert!(
        !m.surface.has_pointer_lock(),
        "the browser's pointerlockerror ended the request"
    );
    assert_eq!(m.shown(), "None");
    assert!(document().pointer_lock_element().is_none());
    m.teardown();
}

/// The browser's grant and its exit (Escape, a tab switch) reach
/// `pointer_lock()` and a `{|| …}` showing it; while it holds, `movementX`/`Y`
/// reach the surface that asked and the motion handler. Before the grant only
/// the handler hears motion.
#[wasm_bindgen_test]
async fn the_browsers_grant_and_exit_are_followed_and_motion_delivered() {
    let m = mount().await;
    mouse_move(&m.canvas, 2, 1);
    assert_eq!(
        m.take(),
        ["handler 2,1"],
        "positive control: motion reaches rinch"
    );

    m.surface.request_pointer_lock().unwrap();
    browser_holds(Some(&m.canvas));
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    assert_eq!(m.shown(), "Some(Locked)");

    mouse_move(&m.canvas, 37, -15);
    assert_eq!(m.take(), ["surface 37,-15", "handler 37,-15"]);

    // The browser ends it (its Escape): no exit call needed from rinch.
    browser_holds(None);
    assert_eq!(pointer_lock(), None);
    assert_eq!(m.shown(), "None");
    assert!(!m.surface.has_pointer_lock());
    mouse_move(&m.canvas, 4, 4);
    assert_eq!(
        m.take(),
        ["handler 4,4"],
        "unlocked: the surface hears none"
    );
    browser_restored();
    m.teardown();
}

/// `exit_pointer_lock` and the surface's unmount each ask the browser to
/// exit, once; with nothing held nothing is asked.
#[wasm_bindgen_test]
async fn exit_and_unmount_release_through_the_browser() {
    let m = mount().await;
    let exits = spy_exit();

    m.surface.request_pointer_lock().unwrap();
    browser_holds(Some(&m.canvas));
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    exit_pointer_lock();
    assert_eq!(exits.get(), 1);
    assert_eq!(pointer_lock(), None);
    exit_pointer_lock();
    assert_eq!(exits.get(), 1, "nothing held, nothing asked");

    m.surface.request_pointer_lock().unwrap();
    browser_holds(Some(&m.canvas));
    assert_eq!(m.shown(), "Some(Locked)");
    let Mounted { root, host, .. } = m;
    root.unmount();
    assert_eq!(exits.get(), 2, "the unmount released it");
    assert_eq!(pointer_lock(), None);
    host.remove();
    unspy_exit();
    browser_restored();
}

fn pointer_move(target: &web_sys::Element) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_pointer_type("mouse");
    init.set_pointer_id(1);
    init.set_is_primary(true);
    let ev = web_sys::PointerEvent::new_with_event_init_dict("pointermove", &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// While locked the surface hears no position moves (they would all be at
/// the frozen position), only `PointerMotion`, as on desktop.
#[wasm_bindgen_test]
async fn a_locked_surface_hears_no_position_moves() {
    let m = mount().await;
    pointer_move(&m.canvas);
    assert_eq!(
        m.take(),
        ["surface move"],
        "positive control: unlocked, it moves"
    );
    m.surface.request_pointer_lock().unwrap();
    browser_holds(Some(&m.canvas));
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    pointer_move(&m.canvas);
    assert!(m.take().is_empty(), "locked: no position moves");
    exit_pointer_lock();
    browser_restored();
    m.teardown();
}

/// Press-to-look with a quick click: the request is withdrawn before the
/// browser answers, and the late grant is given back, not adopted.
#[wasm_bindgen_test]
async fn a_grant_after_the_request_was_withdrawn_is_given_back() {
    let m = mount().await;
    let exits = spy_exit();
    m.surface.request_pointer_lock().unwrap();
    exit_pointer_lock();
    assert_eq!(exits.get(), 0, "nothing granted yet, nothing to exit");
    // The browser's late grant.
    browser_holds(Some(&m.canvas));
    assert_eq!(exits.get(), 1, "the late grant is given back");
    assert_eq!(pointer_lock(), None);
    assert_eq!(m.shown(), "None");
    assert!(!m.surface.has_pointer_lock());
    unspy_exit();
    browser_restored();
    m.teardown();
}

/// A refused move from A (granted) to B: the browser still holds A, which
/// rinch no longer has on record, so it is given back and nothing is
/// reported as B's.
#[wasm_bindgen_test]
async fn a_refused_move_gives_the_old_lock_back() {
    let m = mount().await;
    let exits = spy_exit();
    m.surface.request_pointer_lock().unwrap();
    browser_holds(Some(&m.canvas));
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    // Move it to the window; the browser refuses, keeping the canvas locked.
    rinch::window::request_pointer_lock().unwrap();
    let ev = web_sys::Event::new("pointerlockerror").unwrap();
    document().dispatch_event(&ev).unwrap();
    assert_eq!(exits.get(), 1, "the canvas's lock is given back");
    assert_eq!(pointer_lock(), None);
    assert!(!m.surface.has_pointer_lock());
    unspy_exit();
    browser_restored();
    m.teardown();
}
