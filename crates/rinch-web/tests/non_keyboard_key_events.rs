//! A `keydown` that is not a `KeyboardEvent` must not throw out of the document.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! Anything can be dispatched at `document` under the name "keydown":
//! `document.dispatchEvent(new Event("keydown"))` from page script, a polyfill,
//! a browser extension, another framework sharing the page. wasm-bindgen hands a
//! `Closure<dyn FnMut(web_sys::KeyboardEvent)>` whatever arrives, cast
//! **unchecked**, so the delegation's `event.key()` read `undefined` and the
//! string marshaller threw `Cannot read properties of undefined (reading
//! 'length')` — from a document-level listener, on a page that had done nothing
//! wrong, four times over.
//!
//! A throw inside a listener does not propagate to `dispatchEvent`'s caller, and
//! it does not disarm the listener for the next event either — the browser
//! reports it and carries on. So the damage is exactly the report: a page that
//! dispatches such an event routinely fills the console with TypeErrors from
//! rinch, which is both alarming and misleading. `window`'s `error` event is
//! where that report lands, and it is what this test reads.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Collects `window.onerror` reports for the duration of a test.
struct ErrorWatch {
    errors: Rc<RefCell<Vec<String>>>,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl ErrorWatch {
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
        Self { errors, closure }
    }

    fn reported(&self) -> Vec<String> {
        self.errors.borrow().clone()
    }
}

impl Drop for ErrorWatch {
    fn drop(&mut self) {
        let _ = web_sys::window()
            .unwrap()
            .remove_event_listener_with_callback("error", self.closure.as_ref().unchecked_ref());
    }
}

/// Dispatch a bare `Event` — not a `KeyboardEvent` — on the document.
fn dispatch_plain(name: &str) {
    let event = web_sys::Event::new(name).unwrap();
    document().dispatch_event(&event).unwrap();
}

/// The regression: a plain `Event("keydown")` used to throw out of the document
/// listener. Nothing about it is a key press, so nothing should happen at all.
#[wasm_bindgen_test]
fn a_plain_event_named_keydown_is_ignored_rather_than_throwing() {
    let watch = ErrorWatch::install();

    dispatch_plain("keydown");
    dispatch_plain("keyup");

    assert!(
        watch.reported().is_empty(),
        "a non-keyboard event named keydown/keyup must not raise: {:?}",
        watch.reported()
    );
}
