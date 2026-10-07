//! Round-2 review fixtures for PR #1435 (app image sources in the browser).
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::image::reload_image;
use rinch_core::dom::NodeHandle;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::{Effect, Signal};
use rinch_web::register_image_url_scheme;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Let microtasks run, then a macrotask (`setTimeout(0)`), twice.
async fn settle() {
    for _ in 0..2 {
        let p = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
    }
}

fn host() -> web_sys::Element {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

/// A "not yet" resolver that notes the request in a signal (W1's own shape),
/// with the picture's `<img>` rendered by a block that reads that signal (a
/// `{|| …}` / `if` / reactive component prop that re-creates its content).
/// Desktop asks its loader once: the miss is cached until `reload_image`. Here
/// "not yet" is not remembered, so every re-created element asks again, from a
/// fresh microtask, and the page never gets back to the event loop.
#[wasm_bindgen_test]
async fn r2_a_not_yet_resolver_that_writes_a_signal_read_by_its_img_loops() {
    let asked = Rc::new(Cell::new(0u32));
    // A timer queued before the loop starts: if the loop starves the event
    // loop, it has not fired when the valve opens.
    let fired = Rc::new(Cell::new(false));
    let fired_at_valve = Rc::new(Cell::new(None::<bool>));
    let fired_cb = fired.clone();
    let timer = Closure::once_into_js(move || fired_cb.set(true));
    web_sys::window()
        .unwrap()
        .set_timeout_with_callback_and_timeout_and_arguments_0(timer.unchecked_ref(), 0)
        .unwrap();
    let (fired_in, valve_in) = (fired.clone(), fired_at_valve.clone());
    let wanted: Rc<RefCell<Option<Signal<u32>>>> = Rc::new(RefCell::new(None));
    let (asked_in, wanted_in) = (asked.clone(), wanted.clone());
    register_image_url_scheme("rv2-loop", move |_src| {
        let n = asked_in.get() + 1;
        asked_in.set(n);
        if n >= 200 {
            if valve_in.get().is_none() {
                valve_in.set(Some(fired_in.get()));
            }
            // Safety valve so the test terminates.
            return Some("https://example.test/x.png".into());
        }
        if let Some(sig) = *wanted_in.borrow() {
            sig.update(|v| *v += 1);
        }
        None
    });
    let host = host();
    let wanted_mount = wanted.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let sig = Signal::new(0u32);
        *wanted_mount.borrow_mut() = Some(sig);
        let root = scope.create_element("div");
        let slot = scope.create_element("div");
        root.append_child(&slot);
        let slot_in = slot.clone();
        let made: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
        let maker = Rc::new(RefCell::new(scope.cache_scope()));
        Effect::new(move || {
            let n = sig.get();
            for old in made.borrow_mut().drain(..) {
                old.discard();
            }
            let img = maker.borrow_mut().create_element("img");
            img.set_attribute("src", "rv2-loop:pic");
            img.set_attribute("data-n", &n.to_string());
            slot_in.append_child(&img);
            made.borrow_mut().push(img);
        });
        root
    });
    settle().await;
    let calls = asked.get();
    root.unmount();
    host.remove();
    rinch_web::__reset_image_sources();
    assert_eq!(
        fired_at_valve.get(),
        None,
        "the valve opened (Some(false) = the loop ran 200 rounds without a timer getting in)"
    );
    assert_eq!(
        calls, 1,
        "a not-yet answer re-asked by every re-created element: {calls} resolver calls \
         before the safety valve (desktop: 1)"
    );
}

/// `set_inner_html` markup with an app source: the browser saw the raw
/// `rv2-ih:pic` before `adopt_parsed_images` took it away. Count error events.
#[wasm_bindgen_test]
async fn r2_set_inner_html_app_source_fires_no_error_event() {
    register_image_url_scheme("rv2-ih", |_src| None);
    let host = host();
    let top: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let top_in = top.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let root = scope.create_element("div");
        *top_in.borrow_mut() = Some(root.clone());
        root
    });
    let errors = Rc::new(Cell::new(0u32));
    let errors_in = errors.clone();
    let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
        if e.target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|el| el.tag_name().eq_ignore_ascii_case("img"))
        {
            errors_in.set(errors_in.get() + 1);
        }
    });
    // `error` does not bubble: capture on the document.
    document()
        .add_event_listener_with_callback_and_bool("error", on_error.as_ref().unchecked_ref(), true)
        .unwrap();
    top.borrow()
        .clone()
        .unwrap()
        .set_inner_html("<img id='rv2-ih' src='rv2-ih:pic'>");
    settle().await;
    document()
        .remove_event_listener_with_callback_and_bool(
            "error",
            on_error.as_ref().unchecked_ref(),
            true,
        )
        .unwrap();
    let shown = document()
        .get_element_by_id("rv2-ih")
        .and_then(|e| e.get_attribute("src"));
    root.unmount();
    host.remove();
    rinch_web::__reset_image_sources();
    assert_eq!(shown, None, "control: not yet shows nothing");
    assert_eq!(
        errors.get(),
        0,
        "the browser tried to fetch the app's source"
    );
}

/// Two islands show one source; one unmounts; a reload reaches the other.
#[wasm_bindgen_test]
async fn r2_two_islands_one_unmounts_the_reload_reaches_the_other() {
    let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let answer_in = answer.clone();
    register_image_url_scheme("rv2-isl", move |_src| answer_in.borrow().clone());
    let mut roots = Vec::new();
    for id in ["rv2-a", "rv2-b"] {
        let host = host();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
            let img = scope.create_element("img");
            img.set_attribute("id", id);
            img.set_attribute("src", "rv2-isl:pic");
            img
        });
        roots.push((root, host));
    }
    let (a, host_a) = roots.remove(0);
    a.unmount();
    host_a.remove();
    *answer.borrow_mut() = Some("https://example.test/isl.png".into());
    reload_image("rv2-isl:pic");
    let shown = document()
        .get_element_by_id("rv2-b")
        .and_then(|e| e.get_attribute("src"));
    let (b, host_b) = roots.remove(0);
    b.unmount();
    host_b.remove();
    rinch_web::__reset_image_sources();
    assert_eq!(shown.as_deref(), Some("https://example.test/isl.png"));
}
