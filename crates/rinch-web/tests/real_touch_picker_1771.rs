//! **Real** touch for the file picker (#1771). A touch or pen `pointerdown`
//! runs a `data-rid` click on contact when nothing can scroll (#1582), and that
//! press grants the page no user activation, so a picker opened from it is
//! refused (`showPicker()` throws `NotAllowedError`). The picker waits for the
//! pointer's release, which does grant it. Only a trusted touch shows that,
//! so this runs through `support/run_real_keys.sh`, like
//! `real_touch_1582_1588.rs`; an ordinary run reports it **ignored**.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::file::FilePicker;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
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

/// Ask `support/real_keys.mjs` for one trusted tap at `(x, y)`.
async fn real_tap(x: f64, y: f64) {
    document().set_title(&format!("RINCH-REAL-TOUCH:1:{},{}", x.round(), y.round()));
    for _ in 0..200 {
        sleep(50).await;
        if document().title() == "RINCH-REAL-TOUCH-SENT:1" {
            sleep(400).await;
            return;
        }
    }
    panic!("the helper never sent the touch (is support/real_keys.mjs running?)");
}

#[wasm_bindgen_test]
#[ignore = "needs trusted touches: run tests/support/run_real_keys.sh"]
async fn a_real_tap_on_a_button_of_a_page_that_does_not_scroll_opens_the_picker() {
    assert!(
        option_env!("RINCH_REAL_KEYS").is_some(),
        "built without RINCH_REAL_KEYS: run tests/support/run_real_keys.sh"
    );
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    // What the handler saw: was its press one that grants no activation?
    let log: Rc<RefCell<Vec<bool>>> = Rc::default();
    let picked: Rc<RefCell<Option<usize>>> = Rc::default();
    let (sink, into) = (log.clone(), picked.clone());
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let btn = s.create_element("button");
            btn.set_attribute("id", "pick");
            btn.set_attribute("style", "display: block; width: 200px; height: 60px");
            btn.append_child(&s.create_text("Import…"));
            let rid = s.register_handler(move || {
                sink.borrow_mut()
                    .push(rinch_core::events::press_without_activation().is_some());
                let into = into.clone();
                FilePicker::new().open(move |files| *into.borrow_mut() = Some(files.len()));
            });
            btn.set_attribute("data-rid", &rid.0.to_string());
            btn
        },
    );
    sleep(50).await;
    let r = document()
        .get_element_by_id("pick")
        .unwrap()
        .get_bounding_client_rect();
    real_tap(r.x() + r.width() / 2.0, r.y() + r.height() / 2.0).await;
    // Positive control: the tap clicked on contact, a press without activation.
    assert_eq!(
        *log.borrow(),
        vec![true],
        "one click, run on the touch press"
    );
    // Shown at the release, with activation — not refused at the press.
    assert_eq!(rinch::file::__last_picker_outcome(), "shown");
    // Answer it, so nothing is left behind.
    if let Some(input) = document()
        .query_selector(&format!("input[{}]", rinch::file::__PICKER_ATTR))
        .unwrap()
    {
        input
            .unchecked_into::<web_sys::HtmlInputElement>()
            .dispatch_event(&web_sys::Event::new("cancel").unwrap())
            .unwrap();
    }
    sleep(50).await;
    assert_eq!(*picked.borrow(), Some(0));
    root.unmount();
    host.remove();
}
