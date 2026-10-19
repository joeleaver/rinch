//! An event of the wrong interface under a name rinch listens for — a plain
//! `Event("keydown")`, a `MouseEvent("pointerdown")` — must not throw out of
//! rinch's document listeners.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! Anything can be dispatched at the document, or at an element that bubbles to
//! it, under the name "keydown": `document.dispatchEvent(new Event("keydown"))`
//! from page script, a polyfill, a browser extension, another framework sharing
//! the page. wasm-bindgen hands a `Closure<dyn FnMut(web_sys::KeyboardEvent)>`
//! whatever arrives, cast **unchecked**, so the delegation's `event.key()` read
//! `undefined` and the string marshaller threw `Cannot read properties of
//! undefined (reading 'length')` — from a document-level listener, on a page
//! that had done nothing wrong, four times over. The `pointerdown` listener had
//! the same cast and threw the same TypeError from `pointerType`.
//!
//! That is the release spelling. Under wasm-bindgen's debug glue, which is what
//! `wasm-bindgen-test-runner` generates, the same reads fail as `expected a
//! string argument, found undefined`, and a `bool` or number read fails too
//! (`expected a boolean argument`) where release glue quietly coerces
//! `undefined` to `false` or `0`. Both are a throw these tests catch; measured
//! both ways in Chrome 153.
//!
//! A throw inside a listener does not propagate to `dispatchEvent`'s caller, and
//! it does not disarm the listener for the next event either — the browser
//! reports it and carries on. So the damage is exactly the report: a page that
//! dispatches such an event routinely fills the console with TypeErrors from
//! rinch, which is both alarming and misleading. `window`'s `error` event is
//! where that report lands, and it is what these tests read.
//!
//! **Each fixture mounts a root through `rinch_web::mount_into`, and that is the
//! whole test.** The listeners under test are installed only by a mount
//! (`mount_tree` → `ensure_event_delegation`), and every file under `tests/` is
//! its own wasm binary on its own page. A version of this test that dispatched
//! its events without mounting anything linked no `rinch_web` code at all, so
//! there was no listener to throw and it passed with the guards reverted —
//! measured in Chrome 153. Two positive controls keep that from coming back
//! silently: a genuine event must reach rinch, which proves the listener exists
//! and runs; and a listener that throws on purpose must be seen by the error
//! watch, which proves a throw from a listener is what `window`'s `error` event
//! reports.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// A mounted root, torn down in [`Mounted::teardown`].
struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
}

impl Mounted {
    fn new(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        rinch_web::__reset_activation_state();
        let host = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
        Self { root, host }
    }

    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

/// Collects `window.onerror` reports for the duration of a test.
struct ErrorWatch {
    errors: Rc<RefCell<Vec<String>>>,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl ErrorWatch {
    /// Install the watch and prove it hears a listener's throw, so that an
    /// empty report later means nothing threw rather than nothing listened.
    fn install() -> Self {
        let errors = Rc::new(RefCell::new(Vec::new()));
        let sink = errors.clone();
        let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
            // The `ErrorEvent`'s own `message` where the browser gives one, the
            // event's type otherwise — either way it is a report that must not
            // appear. Read reflectively rather than through `web_sys::ErrorEvent`
            // so the test needs no extra web-sys feature for a string it only
            // prints.
            let message = js_sys::Reflect::get(&event, &JsValue::from_str("message"))
                .ok()
                .and_then(|m| m.as_string())
                .unwrap_or_else(|| event.type_());
            sink.borrow_mut().push(message);
        }) as Box<dyn FnMut(web_sys::Event)>);
        web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("error", closure.as_ref().unchecked_ref())
            .unwrap();
        let watch = Self { errors, closure };

        let thrower = js_sys::Function::new_no_args("throw new Error('deliberate')");
        document()
            .add_event_listener_with_callback("rinch-test-throw", &thrower)
            .unwrap();
        document()
            .dispatch_event(&web_sys::Event::new("rinch-test-throw").unwrap())
            .unwrap();
        document()
            .remove_event_listener_with_callback("rinch-test-throw", &thrower)
            .unwrap();
        assert_eq!(
            watch.take().len(),
            1,
            "the error watch must see a listener's throw, or an empty report means nothing"
        );
        watch
    }

    /// Everything reported since the last `take`.
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.errors.borrow_mut())
    }
}

impl Drop for ErrorWatch {
    fn drop(&mut self) {
        let _ = web_sys::window()
            .unwrap()
            .remove_event_listener_with_callback("error", self.closure.as_ref().unchecked_ref());
    }
}

/// Dispatch a bare, bubbling `Event` — no richer interface — at `target`.
fn dispatch_plain(target: &web_sys::EventTarget, name: &str) {
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    let event = web_sys::Event::new_with_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&event).unwrap();
}

/// Dispatch a genuine `KeyboardEvent` on the document. `Shift` is a key no
/// branch of the delegation acts on, so the interceptor is all it reaches.
fn dispatch_key(name: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Shift");
    init.set_code("ShiftLeft");
    let event = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(name, &init).unwrap();
    document().dispatch_event(&event).unwrap();
}

/// A plain `Event("keydown")` used to throw out of the document listener.
/// Nothing about it is a key press, so nothing should happen at all.
#[wasm_bindgen_test]
fn a_plain_event_named_keydown_is_ignored_rather_than_throwing() {
    let mounted = Mounted::new(|scope| scope.create_element("div"));

    // Registered outside any render, so it is the thread-global fallback every
    // dispatch reaches, and it lives until cleared below.
    let presses = Rc::new(Cell::new(0u32));
    let releases = Rc::new(Cell::new(0u32));
    {
        let (presses, releases) = (presses.clone(), releases.clone());
        rinch_core::events::set_keyboard_interceptor(move |key| {
            let counter = if key.is_up() { &releases } else { &presses };
            counter.set(counter.get() + 1);
            false
        });
    }

    let watch = ErrorWatch::install();

    // Positive control: both delegation listeners are installed and run.
    dispatch_key("keydown");
    dispatch_key("keyup");
    assert_eq!(
        (presses.get(), releases.get()),
        (1, 1),
        "a genuine press and release must each reach the interceptor, or nothing below is under test"
    );

    dispatch_plain(document().as_ref(), "keydown");
    dispatch_plain(document().as_ref(), "keyup");

    let reported = watch.take();
    assert!(
        reported.is_empty(),
        "a non-keyboard event named keydown/keyup must not raise: {reported:?}"
    );
    assert_eq!(
        (presses.get(), releases.get()),
        (1, 1),
        "a non-keyboard event is not a key, so it must not reach the interceptor"
    );

    rinch_core::events::clear_keyboard_interceptor();
    drop(watch);
    mounted.teardown();
}

/// The same unchecked cast on `pointerdown`: a non-pointer event bubbling from
/// an element read `undefined` for `pointerType`. It is dispatched from an
/// element because that is the release-build throw — at the document itself the
/// listener finds no element target and never reaches that read, and only the
/// debug glue's `isPrimary` check would still have caught it.
#[wasm_bindgen_test]
fn a_non_pointer_event_named_pointerdown_is_ignored_rather_than_throwing() {
    let clicks = Rc::new(Cell::new(0u32));
    let counter = clicks.clone();
    let mounted = Mounted::new(move |scope| {
        let el = scope.create_element("div");
        el.set_attribute("id", "press-target");
        let id = scope.register_handler(move || counter.set(counter.get() + 1));
        el.set_attribute("data-rid", &id.0.to_string());
        el
    });
    let target: web_sys::EventTarget = document().get_element_by_id("press-target").unwrap().into();

    let watch = ErrorWatch::install();

    // Positive control: the pointerdown listener is installed and dispatches.
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_pointer_type("mouse");
    init.set_is_primary(true);
    let press = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    target.dispatch_event(&press).unwrap();
    assert_eq!(
        clicks.get(),
        1,
        "a genuine pointerdown must dispatch the handler, or nothing below is under test"
    );
    rinch_web::__reset_activation_state();

    dispatch_plain(&target, "pointerdown");
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    let mouse = web_sys::MouseEvent::new_with_mouse_event_init_dict("pointerdown", &init).unwrap();
    target.dispatch_event(&mouse).unwrap();

    let reported = watch.take();
    assert!(
        reported.is_empty(),
        "a non-pointer event named pointerdown must not raise: {reported:?}"
    );
    assert_eq!(
        clicks.get(),
        1,
        "a non-pointer event is not a press, so it must not dispatch the handler"
    );

    drop(watch);
    mounted.teardown();
}

/// A genuine bubbling `PointerEvent` at `(x, y)` — the shape a real press,
/// move or release has, so the positive controls below run the listener the
/// way the browser does.
fn dispatch_pointer(target: &web_sys::EventTarget, name: &str, (x, y): (i32, i32), buttons: u16) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_type("mouse");
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_client_x(x);
    init.set_client_y(y);
    init.set_buttons(buttons);
    let event = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&event).unwrap();
}

/// Issue #811. A pointer-capture `Drag` armed by a real press must be ended
/// only by a real pointer event. A plain `Event("pointerup")` read `clientX` /
/// `clientY` as `undefined`, which release glue coerces to `0` — so it
/// committed the drag through `on_end(0, 0)` — and debug glue throws on
/// (`expected a number argument`). `pointermove` read `buttons` as `0` and
/// healed the drag through `on_cancel`; `pointercancel` cancelled it outright.
/// None of them is a pointer event, so none of them may touch the drag.
#[wasm_bindgen_test]
fn a_plain_event_named_pointerup_neither_commits_a_live_drag_nor_throws() {
    let ends: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = ends.clone();
    let mounted = Mounted::new(move |scope| {
        let el = scope.create_element("div");
        el.set_attribute("id", "drag-handle");
        el.set_attribute(
            "style",
            "position: fixed; left: 100px; top: 100px; width: 40px; height: 40px;",
        );
        let id = scope.register_handler(move || {
            let (end, cancel) = (sink.clone(), sink.clone());
            rinch_core::Drag::absolute()
                .on_end(move |x, y| end.borrow_mut().push(format!("end {x},{y}")))
                .on_cancel(move |x, y| cancel.borrow_mut().push(format!("cancel {x},{y}")))
                .start();
        });
        el.set_attribute("data-rid", &id.0.to_string());
        el
    });
    let handle: web_sys::EventTarget = document().get_element_by_id("drag-handle").unwrap().into();

    let watch = ErrorWatch::install();

    // Arm the drag with a genuine press, and prove the arming worked.
    dispatch_pointer(&handle, "pointerdown", (120, 120), 1);
    assert!(
        rinch_core::Drag::is_active(),
        "a genuine pointerdown on the handle must arm the drag, or nothing below is under test"
    );

    for name in ["pointerup", "pointermove", "pointercancel"] {
        dispatch_plain(&handle, name);
        dispatch_plain(document().as_ref(), name);
        // A `MouseEvent` carries numbers where a plain `Event` carries none, so
        // it is the shape that would commit at a real-looking position.
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        init.set_client_x(7);
        init.set_client_y(9);
        let mouse = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
        handle.dispatch_event(&mouse).unwrap();
    }

    let reported = watch.take();
    assert!(
        reported.is_empty(),
        "a non-pointer event named pointerup/pointermove/pointercancel must not raise: {reported:?}"
    );
    assert!(
        ends.borrow().is_empty(),
        "a non-pointer event must neither commit nor cancel the drag: {:?}",
        ends.borrow()
    );
    assert!(
        rinch_core::Drag::is_active(),
        "the drag armed by a real press must still be live"
    );

    // Positive control: the pointerup listener is installed, and a genuine
    // release commits the drag where it happened.
    dispatch_pointer(&handle, "pointerup", (130, 131), 0);
    assert_eq!(*ends.borrow(), vec!["end 130,131".to_string()]);
    assert!(watch.take().is_empty());

    drop(watch);
    mounted.teardown();
}

/// Issue #811: `click` is deliberately NOT narrowed to `MouseEvent` — a plain
/// `Event("click")` activating a `data-rid` element is what automation does
/// (`dispatchEvent(new Event("click"))`), and untrusted clicks are let through
/// by design (#240). What it must not do is throw: `detail` and `clientX` read
/// `undefined`, which debug glue rejects (`expected a number argument`).
#[wasm_bindgen_test]
fn a_plain_event_named_click_still_activates_without_throwing() {
    let clicks = Rc::new(Cell::new(0u32));
    let counter = clicks.clone();
    let mounted = Mounted::new(move |scope| {
        let el = scope.create_element("div");
        el.set_attribute("id", "click-target");
        let id = scope.register_handler(move || counter.set(counter.get() + 1));
        el.set_attribute("data-rid", &id.0.to_string());
        el
    });
    let target: web_sys::EventTarget = document().get_element_by_id("click-target").unwrap().into();

    let watch = ErrorWatch::install();

    // Positive control: a genuine click reaches the handler.
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    let click = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    target.dispatch_event(&click).unwrap();
    assert_eq!(clicks.get(), 1, "a genuine click must dispatch the handler");

    dispatch_plain(&target, "click");
    let reported = watch.take();
    assert!(
        reported.is_empty(),
        "a plain Event named click must not raise: {reported:?}"
    );
    assert_eq!(
        clicks.get(),
        2,
        "a plain Event named click still activates the element, as it did before"
    );
    // The click context is the element's centre: the event has no position.
    let ctx = rinch_core::events::get_click_context();
    assert!(
        ctx.mouse_x > 0.0 && ctx.mouse_x == ctx.element_x + ctx.element_width / 2.0,
        "a click with no position activates at the element's centre: {ctx:?}"
    );

    drop(watch);
    mounted.teardown();
}

/// Issue #811: `contextmenu` follows `click`. A plain `Event("contextmenu")`
/// on an element with a live `data-oncontextmenu` handler read `clientX` as
/// `undefined` (a throw under debug glue, `0` under release); it now runs the
/// handler at the element's centre, as a click with no position does.
#[wasm_bindgen_test]
fn a_plain_event_named_contextmenu_runs_its_handler_without_throwing() {
    let menus = Rc::new(Cell::new(0u32));
    let counter = menus.clone();
    let mounted = Mounted::new(move |scope| {
        let el = scope.create_element("div");
        el.set_attribute("id", "menu-target");
        el.set_attribute(
            "style",
            "position: fixed; left: 200px; top: 60px; width: 40px; height: 20px;",
        );
        let id = scope.register_handler(move || counter.set(counter.get() + 1));
        el.set_attribute("data-oncontextmenu", &id.0.to_string());
        el
    });
    let target: web_sys::EventTarget = document().get_element_by_id("menu-target").unwrap().into();

    let watch = ErrorWatch::install();

    // Positive control: a genuine contextmenu reaches the handler.
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(2);
    init.set_client_x(205);
    init.set_client_y(65);
    let menu = web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
    target.dispatch_event(&menu).unwrap();
    assert_eq!(menus.get(), 1, "a genuine contextmenu must dispatch the handler");
    assert_eq!(rinch_core::events::get_click_context().mouse_x, 205.0);

    dispatch_plain(&target, "contextmenu");
    let reported = watch.take();
    assert!(
        reported.is_empty(),
        "a plain Event named contextmenu must not raise: {reported:?}"
    );
    assert_eq!(menus.get(), 2);
    let ctx = rinch_core::events::get_click_context();
    assert_eq!(
        (ctx.mouse_x, ctx.mouse_y),
        (220.0, 70.0),
        "a contextmenu with no position runs at the element's centre"
    );

    drop(watch);
    mounted.teardown();
}

/// Issue #811: a `paste` that is not a `ClipboardEvent` is not a paste, even
/// when page script hangs a `clipboardData` on it. It used to fill rinch's
/// clipboard buffers and reach the app's paste interceptor.
#[wasm_bindgen_test]
fn a_plain_event_named_paste_reaches_no_paste_interceptor() {
    let mounted = Mounted::new(|scope| scope.create_element("div"));
    let pastes: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
    {
        let pastes = pastes.clone();
        rinch_core::events::set_paste_interceptor(move |data| {
            pastes.borrow_mut().push(data.text.clone());
            false
        });
    }
    let watch = ErrorWatch::install();

    // Positive control: a genuine paste reaches the interceptor.
    let dt = web_sys::DataTransfer::new().unwrap();
    dt.set_data("text/plain", "real").unwrap();
    let init = web_sys::ClipboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_clipboard_data(Some(&dt));
    let paste = web_sys::ClipboardEvent::new_with_event_init_dict("paste", &init).unwrap();
    if paste.clipboard_data().as_ref() != Some(&dt) {
        // Firefox's constructor drops the member (see support/clipboard.rs).
        let desc = js_sys::Object::new();
        js_sys::Reflect::set(&desc, &"value".into(), &dt).unwrap();
        let target: &js_sys::Object = paste.unchecked_ref();
        js_sys::Reflect::define_property(target, &"clipboardData".into(), &desc).unwrap();
    }
    document().dispatch_event(&paste).unwrap();
    assert_eq!(*pastes.borrow(), vec![Some("real".to_string())]);

    // A plain `Event` named paste, carrying a `clipboardData` of its own.
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    let fake = web_sys::Event::new_with_event_init_dict("paste", &init).unwrap();
    let dt = web_sys::DataTransfer::new().unwrap();
    dt.set_data("text/plain", "fake").unwrap();
    let desc = js_sys::Object::new();
    js_sys::Reflect::set(&desc, &"value".into(), &dt).unwrap();
    let target: &js_sys::Object = fake.unchecked_ref();
    js_sys::Reflect::define_property(target, &"clipboardData".into(), &desc).unwrap();
    document().dispatch_event(&fake).unwrap();
    dispatch_plain(document().as_ref(), "copy");
    dispatch_plain(document().as_ref(), "cut");

    let reported = watch.take();
    assert!(
        reported.is_empty(),
        "a plain Event named paste/copy/cut must not raise: {reported:?}"
    );
    assert_eq!(
        *pastes.borrow(),
        vec![Some("real".to_string())],
        "a plain Event named paste is not a paste"
    );

    rinch_core::events::clear_paste_interceptor();
    drop(watch);
    mounted.teardown();
}

/// Issue #811, the `RenderSurface` canvas: its own pointer and wheel listeners
/// had the same unchecked typed cast. A plain `Event("pointerdown")` at the
/// canvas read `offsetX` / `button` / `pointerId` as `undefined` — a throw
/// under debug glue, a `MouseDown` at (0, 0) under release — and a plain
/// `Event("pointerleave")` reached the app as a `MouseLeave`.
#[wasm_bindgen_test]
async fn a_non_pointer_event_at_a_render_surface_reaches_no_surface_handler() {
    use rinch::render_surface::{RenderSurface, SurfaceEvent, create_render_surface};
    use rinch_core::Component;

    let surface = create_render_surface();
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = seen.clone();
    surface.set_event_handler(move |e| {
        let s = match e {
            SurfaceEvent::MouseDown { x, y, .. } => format!("down {x},{y}"),
            SurfaceEvent::MouseMove { x, y } => format!("move {x},{y}"),
            SurfaceEvent::MouseUp { x, y, .. } => format!("up {x},{y}"),
            SurfaceEvent::MouseEnter { .. } => "enter".to_string(),
            SurfaceEvent::MouseLeave => "leave".to_string(),
            SurfaceEvent::MouseWheel { .. } => "wheel".to_string(),
            _ => return,
        };
        sink.borrow_mut().push(s);
    });
    let id = surface.id();
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 50px; top: 50px; width: 200px; height: 100px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        RenderSurface {
            surface: Some(surface.clone()),
        }
        .render(scope, &[])
    });
    // The canvas's listeners are installed from a microtask after mount.
    let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL)).await;
    let canvas: web_sys::EventTarget = document()
        .get_element_by_id(&format!("rinch-surface-{id}"))
        .expect("the surface's canvas")
        .into();

    let watch = ErrorWatch::install();

    // Positive control: the canvas listeners are installed and run.
    dispatch_pointer(&canvas, "pointerdown", (60, 70), 1);
    dispatch_pointer(&canvas, "pointerup", (60, 70), 0);
    assert_eq!(
        *seen.borrow(),
        vec!["down 10,20".to_string(), "up 10,20".to_string()],
        "genuine pointer events must reach the surface, or nothing below is under test"
    );
    seen.borrow_mut().clear();

    for name in [
        "pointerdown",
        "pointermove",
        "pointerup",
        "pointercancel",
        "pointerenter",
        "pointerleave",
        "wheel",
    ] {
        dispatch_plain(&canvas, name);
    }

    let reported = watch.take();
    assert!(
        reported.is_empty(),
        "a plain Event at the canvas must not raise: {reported:?}"
    );
    assert!(
        seen.borrow().is_empty(),
        "a plain Event is not pointer input: {:?}",
        seen.borrow()
    );

    drop(watch);
    root.unmount();
    host.remove();
}
