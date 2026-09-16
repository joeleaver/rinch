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
