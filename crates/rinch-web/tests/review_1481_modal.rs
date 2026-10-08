#![cfg(target_arch = "wasm32")]

use std::rc::Rc;

use rinch::components::Modal;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-review-1481-modal";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn capture() -> web_sys::HtmlTextAreaElement {
    document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .expect("capture textarea")
        .dyn_into()
        .unwrap()
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document().element_from_point(x, y).expect("hit");
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(1);
    init.set_detail(1);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

async fn microtask() {
    let p = js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL);
    wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
}

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    open: Signal<bool>,
}

/// An editor with one paragraph, focused by a press, and beside it a closed
/// `Modal` holding one text input.
fn mounted() -> F {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i) {
                n.dyn_into::<web_sys::Element>().unwrap().remove();
            }
        }
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST, "").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; top: 0; left: 0; z-index: 9999; background: white; \
         font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 400px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html("<p>hello there</p>"));
    let open = Signal::new(false);
    let m = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            page.append_child(&m.mount(s));
            let input = s.create_element("input");
            input.set_attribute("id", "in-modal");
            input.set_attribute("style", "display: block; width: 120px; height: 28px");
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
    let p = document()
        .query_selector("[data-pm-editor] p")
        .unwrap()
        .expect("block");
    let r = p.get_bounding_client_rect();
    mouse("mousedown", (r.x() + 2.0) as f32, (r.y() + 12.0) as f32);
    mouse("mouseup", (r.x() + 2.0) as f32, (r.y() + 12.0) as f32);
    let ta = capture();
    assert!(
        document().active_element().as_ref() == Some(ta.as_ref()),
        "positive control: a press reached rinch and focused the editor"
    );
    F {
        root,
        host,
        handle,
        open,
    }
}

fn caret_visible() -> bool {
    let Some(c) = document()
        .query_selector("[data-pm-editor] [data-pm-caret]")
        .unwrap()
    else {
        return false;
    };
    let st = web_sys::window()
        .unwrap()
        .get_computed_style(&c)
        .unwrap()
        .unwrap();
    st.get_property_value("display").unwrap() != "none"
        && st.get_property_value("visibility").unwrap() == "visible"
        && c.get_client_rects().length() > 0
}

fn active_id() -> String {
    document()
        .active_element()
        .map(|el| el.id())
        .unwrap_or_default()
}

/// W2a: the handler that opens a modal also blurs the editor, after the open.
#[wasm_bindgen_test]
async fn w2a_open_then_blur_in_one_batch() {
    let f = mounted();
    microtask().await;
    let h = f.handle.clone();
    let open = f.open;
    rinch_core::batch(move || {
        open.set(true);
        h.blur();
    });
    assert_eq!(active_id(), "in-modal", "the modal took the keyboard");
    assert!(!caret_visible());
    f.open.set(false);
    microtask().await;
    console_log!("W2a after close active = {:?} tag={:?}", active_id(), document().active_element().map(|e| e.tag_name()));
    let before = f.handle.doc().content().size();
    f.handle.insert_text("!");
    assert!(f.handle.doc().content().size() > before, "the editor still edits");
    f.root.unmount();
    f.host.remove();
}

/// W2b: blur first, then open, in one batch; and blur from an effect that
/// reads the same signal.
#[wasm_bindgen_test]
async fn w2b_blur_then_open_and_blur_from_an_effect() {
    let f = mounted();
    microtask().await;
    let h = f.handle.clone();
    let open = f.open;
    let _e = rinch_core::reactive::Effect::new(move || {
        if open.get() {
            h.blur();
        }
    });
    let h = f.handle.clone();
    rinch_core::batch(move || {
        h.blur();
        open.set(true);
    });
    assert_eq!(active_id(), "in-modal", "the modal took the keyboard");
    assert!(!caret_visible());
    f.open.set(false);
    microtask().await;
    console_log!("W2b after close active = {:?} tag={:?}", active_id(), document().active_element().map(|e| e.tag_name()));
    f.handle.focus();
    microtask().await;
    assert!(caret_visible(), "focus() brings the caret back");
    f.root.unmount();
    f.host.remove();
}
