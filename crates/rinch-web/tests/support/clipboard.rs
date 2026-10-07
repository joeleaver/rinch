//! A synthetic clipboard event for the browser tests
//! (`#[path = "support/clipboard.rs"] mod clipboard;`).

use wasm_bindgen::JsCast;

/// A bubbling, cancelable `ClipboardEvent` named `name` (`copy`, `cut`,
/// `paste`) whose `clipboardData` **is** `dt`, in every engine.
///
/// The standard constructor takes the transfer in its init dictionary
/// (`{ clipboardData }`), and Chromium and WebKit hand that same object to
/// the listeners. Firefox ignores the member: its constructor reads the
/// non-standard `{ data, dataType }` pair instead, so the event arrives
/// with a fresh, empty `DataTransfer` (measured, Firefox 155 and 157). A
/// listener then reads nothing from a synthetic `paste`, and what it writes
/// during a `copy` never reaches the test's `dt`. A paste fixture built
/// that way fails in Firefox whatever the product does with a real paste.
///
/// So where the constructed event does not already carry `dt`, an own
/// `clipboardData` property on the event names it. The product reads
/// `event.clipboardData` like any listener, and sees `dt`. Where the
/// constructor did carry it (Chromium), nothing is defined and the event is
/// exactly the one these tests always dispatched.
#[allow(dead_code)]
pub fn clipboard_event(name: &str, dt: &web_sys::DataTransfer) -> web_sys::ClipboardEvent {
    let init = web_sys::ClipboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_clipboard_data(Some(dt));
    let ev = web_sys::ClipboardEvent::new_with_event_init_dict(name, &init).unwrap();
    if ev.clipboard_data().as_ref() != Some(dt) {
        let desc = js_sys::Object::new();
        js_sys::Reflect::set(&desc, &"value".into(), dt).unwrap();
        let target: &js_sys::Object = ev.unchecked_ref();
        js_sys::Reflect::define_property(target, &"clipboardData".into(), &desc).unwrap();
        assert!(
            ev.clipboard_data().as_ref() == Some(dt),
            "the event carries the test's DataTransfer"
        );
    }
    ev
}
