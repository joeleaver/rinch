//! **Real** touches in Chrome for #1582 and #1588. A synthetic `pointerdown`
//! neither pans the page nor makes the browser send the compatibility mouse
//! events that follow a tap, and both bugs live there; so the touches here are
//! trusted ones, sent by `support/real_keys.mjs` over the DevTools protocol
//! (`Input.dispatchTouchEvent`) when the page asks for them through its title.
//! It runs only through `support/run_real_keys.sh`, which builds the test with
//! `RINCH_REAL_KEYS=1`, starts Chrome with a debugging port and runs the helper
//! beside it:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver crates/rinch-web/tests/support/run_real_keys.sh
//! ```
//!
//! An ordinary run reports these **ignored**, never `ok`: nothing would send the
//! touches. CI's Chrome job runs them through the script.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::components::Modal;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal};
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-real-touch";

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

thread_local! {
    static TOUCH_N: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Ask the helper for one trusted touch through `points` (viewport CSS px: down
/// at the first, moved through the rest, up at the last), wait until it says it
/// sent it, and give the browser time for what follows a tap.
async fn real_touch(points: &[(f64, f64)]) {
    let n = TOUCH_N.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let path = points
        .iter()
        .map(|(x, y)| format!("{},{}", x.round(), y.round()))
        .collect::<Vec<_>>()
        .join(";");
    document().set_title(&format!("RINCH-REAL-TOUCH:{n}:{path}"));
    let sent = format!("RINCH-REAL-TOUCH-SENT:{n}");
    for _ in 0..200 {
        sleep(50).await;
        if document().title() == sent {
            // The compatibility mouse events and the click come after the
            // touch has ended.
            sleep(400).await;
            return;
        }
    }
    panic!("the helper never sent touch #{n} (is support/real_keys.mjs running?)");
}

/// Records the type of every pointer event, `mousedown` and `click` the window
/// sees from now on (capture phase).
fn record_events() -> Rc<RefCell<Vec<String>>> {
    let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |e: web_sys::Event| sink.borrow_mut().push(e.type_()),
    );
    let win = web_sys::window().unwrap();
    for name in [
        "pointerdown",
        "pointerup",
        "pointercancel",
        "mousedown",
        "click",
    ] {
        win.add_event_listener_with_callback_and_bool(name, cb.as_ref().unchecked_ref(), true)
            .unwrap();
    }
    cb.forget();
    seen
}

fn require_helper() {
    assert!(
        option_env!("RINCH_REAL_KEYS").is_some(),
        "built without RINCH_REAL_KEYS: run tests/support/run_real_keys.sh, which sends the touches"
    );
}

fn clear_stale() {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i) {
                n.dyn_into::<web_sys::Element>().unwrap().remove();
            }
        }
    }
    rinch_web::__reset_activation_state();
    let _ = document()
        .document_element()
        .unwrap()
        .remove_attribute("style");
    let _ = document().body().unwrap().remove_attribute("style");
    web_sys::window().unwrap().scroll_to_with_x_and_y(0.0, 0.0);
}

fn centre(el: &web_sys::Element) -> (f64, f64) {
    let r = el.get_bounding_client_rect();
    (r.x() + r.width() / 2.0, r.y() + r.height() / 2.0)
}

type Log = Rc<RefCell<Vec<String>>>;

/// A button with a handler 200px down a page 3000px taller than the viewport.
fn scrolling_page() -> (RootHandle, web_sys::Element, Log) {
    clear_stale();
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            let btn = s.create_element("button");
            btn.set_attribute("id", "btn");
            btn.set_attribute(
                "style",
                "display: block; width: 200px; height: 60px; margin-top: 200px",
            );
            btn.append_child(&s.create_text("press"));
            let sink = sink.clone();
            let rid = s.register_handler(move || sink.borrow_mut().push("btn".into()));
            btn.set_attribute("data-rid", &rid.0.to_string());
            page.append_child(&btn);
            let spacer = s.create_element("div");
            spacer.set_attribute("style", "height: 3000px");
            page.append_child(&spacer);
            page
        },
    );
    (root, host, log)
}

/// Positive control for the touch path: a tap on the button clicks it once.
#[wasm_bindgen_test]
#[ignore = "needs trusted touches: run tests/support/run_real_keys.sh"]
async fn a_real_tap_on_a_button_of_a_scrolling_page_clicks_it_once() {
    require_helper();
    let (root, host, log) = scrolling_page();
    sleep(50).await;
    let at = centre(&document().get_element_by_id("btn").unwrap());
    real_touch(&[at]).await;
    assert_eq!(*log.borrow(), ["btn"]);
    root.unmount();
    host.remove();
}

/// #1582: a finger that lands on the button and pans the page has not pressed it.
#[wasm_bindgen_test]
#[ignore = "needs trusted touches: run tests/support/run_real_keys.sh"]
async fn a_real_pan_that_starts_on_a_button_does_not_click_it() {
    require_helper();
    let (root, host, log) = scrolling_page();
    sleep(50).await;
    let (x, y) = centre(&document().get_element_by_id("btn").unwrap());
    let path: Vec<(f64, f64)> = (0..=8).map(|i| (x, y - 10.0 * i as f64)).collect();
    let seen = record_events();
    real_touch(&path).await;
    let scrolled = web_sys::window().unwrap().scroll_y().unwrap();
    // Positive control: the browser took the contact for a pan.
    assert!(
        seen.borrow().iter().any(|e| e == "pointercancel") && scrolled > 0.0,
        "no pan happened: scrolled {scrolled}, events {:?}",
        seen.borrow()
    );
    assert!(
        log.borrow().is_empty(),
        "the pan clicked: {:?}",
        log.borrow()
    );
    root.unmount();
    host.remove();
}

/// #1588: a tap on a button inside a scroller whose handler opens a `Modal`
/// (`trap_focus`) leaves the focus in the modal. The compatibility `mousedown`
/// that followed the tap landed on the modal's overlay and blurred its field.
#[wasm_bindgen_test]
#[ignore = "needs trusted touches: run tests/support/run_real_keys.sh"]
async fn a_real_tap_that_opens_a_modal_from_a_scroller_leaves_the_focus_in_it() {
    require_helper();
    clear_stale();
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; top: 0; left: 0; z-index: 9999; background: white; width: 400px",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let open = Signal::new(false);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            let scroller = s.create_element("div");
            scroller.set_attribute("id", "scroller");
            scroller.set_attribute("style", "height: 150px; overflow: auto");
            let btn = s.create_element("button");
            btn.set_attribute("id", "opener");
            btn.set_attribute("style", "display: block; width: 200px; height: 60px");
            btn.append_child(&s.create_text("open"));
            let rid = s.register_handler(move || open.set(true));
            btn.set_attribute("data-rid", &rid.0.to_string());
            scroller.append_child(&btn);
            let filler = s.create_element("div");
            filler.set_attribute("style", "height: 600px");
            scroller.append_child(&filler);
            page.append_child(&scroller);
            let input = s.create_element("input");
            input.set_attribute("id", "in-modal");
            let modal = Modal {
                opened_fn: Some(Rc::new(move || open.get())),
                trap_focus: true,
                with_close_button: false,
                ..Default::default()
            }
            .render(s, &[input]);
            page.append_child(&modal);
            page
        },
    );
    sleep(50).await;
    let at = centre(&document().get_element_by_id("opener").unwrap());
    real_touch(&[at]).await;
    assert!(open.get(), "positive control: the tap opened the modal");
    let active = document().active_element();
    assert_eq!(
        active.as_ref().map(|e| e.id()).unwrap_or_default(),
        "in-modal",
        "focus after the tap: {:?}",
        active.map(|e| e.tag_name())
    );
    open.set(false);
    root.unmount();
    host.remove();
}
