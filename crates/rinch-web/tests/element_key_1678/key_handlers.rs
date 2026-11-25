//! The element `onkeydown` / `onkeyup` handlers: prevent-default, order against
//! the interceptor and the dismiss stack, ancestors and composition.

use super::*;

/// Positive control and the shape: a genuine keydown at the focused textarea
/// reaches its `onkeydown` (the delegation is installed by the mount), a
/// consumed Enter is cancelled and reaches no ancestor and no `onsubmit`,
/// and Shift+Enter goes on to the ancestor and is not cancelled.
#[wasm_bindgen_test]
fn onkeydown_consumes_enter_and_shift_enter_goes_on() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();

    assert!(key("keydown", "Enter", "Enter", false, false), "cancelled");
    assert_eq!(f.take_log(), ["down:Enter"], "no ancestor, no submit");

    assert!(
        !key("keydown", "Enter", "Enter", true, false),
        "not cancelled"
    );
    assert_eq!(f.take_log(), ["down:S-Enter", "outer:Enter"]);
    f.teardown();
}

/// After the interceptor (#3062: it is a `window` capture listener now) and
/// before the dismiss stack, as on desktop: a field's handler takes Escape and
/// the overlay stays.
#[wasm_bindgen_test]
fn onkeydown_runs_after_the_interceptor_and_before_the_dismiss_stack() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let s1 = seen.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        s1.borrow_mut().push(k.key.clone());
        false
    });
    let dismissed = Rc::new(Cell::new(0));
    let d1 = dismissed.clone();
    let entry = push_dismiss_handler(0, move || {
        d1.set(d1.get() + 1);
        true
    });

    assert!(key("keydown", "Escape", "Escape", false, false));
    assert_eq!(dismissed.get(), 0, "the field took Escape");
    assert_eq!(*seen.borrow(), ["Escape"], "the interceptor first");

    assert!(!key("keydown", "x", "KeyX", false, false));
    assert_eq!(*seen.borrow(), ["Escape", "x"]);
    drop(entry);
    rinch_core::clear_keyboard_interceptor();
    f.teardown();
}

/// `onkeyup` hears the release, and a consuming one keeps it from the
/// ancestors (there is no ancestor `onkeyup` here; the release still reaches
/// the interceptor).
#[wasm_bindgen_test]
fn onkeyup_hears_the_release_and_the_interceptor_still_does() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();
    let ups = Rc::new(Cell::new(0));
    let u1 = ups.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        if k.is_up() {
            u1.set(u1.get() + 1);
        }
        false
    });
    key("keyup", "q", "KeyQ", false, false);
    assert_eq!(f.take_log(), ["up:q"]);
    assert_eq!(ups.get(), 1);
    rinch_core::clear_keyboard_interceptor();
    f.teardown();
}

/// A key the input method is composing with is not offered.
#[wasm_bindgen_test]
fn a_composing_key_is_not_offered() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();
    assert!(!key("keydown", "Enter", "Enter", false, true));
    assert!(!key("keydown", "Process", "KeyN", false, false));
    assert!(
        !f.take_log().iter().any(|e| e.starts_with("down:")),
        "not offered while composing"
    );
    f.teardown();
}

/// A `keyCode` 229 Enter that is not marked composing (Safari's IME-confirm
/// Enter, and an Android soft keyboard's plain Enter) is offered, and still
/// submits: refusing it would break Enter on Android. The `Process` key is
/// not offered (`a_composing_key_is_not_offered`).
#[wasm_bindgen_test]
fn a_key_code_229_enter_is_offered() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Enter");
    init.set_code("Enter");
    js_sys::Reflect::set(&init, &"keyCode".into(), &229.into()).unwrap();
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    assert_eq!(ev.key_code(), 229, "positive control: the init carried it");
    let target: web_sys::EventTarget = document().active_element().unwrap().into();
    target.dispatch_event(&ev).unwrap();
    assert!(ev.default_prevented(), "the field's handler took it");
    assert_eq!(f.take_log(), ["down:Enter"]);
    f.teardown();
}
