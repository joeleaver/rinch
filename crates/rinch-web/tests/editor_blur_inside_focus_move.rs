//! A modal that opens while an editor holds the keyboard takes the keyboard
//! without a panic.
//!
//! Reported from Pimble: File > "New Store..." (or its Ctrl+N) with a note's
//! editor focused panicked the page with "RefCell already borrowed" and left
//! every editor dead. The modal's focus effect moves focus with
//! `NodeHandle::focus_into`, which held the document borrowed while it called
//! the browser's `focus()`. The browser dispatches the capture textarea's
//! `blur` synchronously inside that call, and the editor's blur handler
//! repaints the caret there, writing styles into the document. The browser
//! work now runs once the borrow is released (`DomDocument::take_after_borrow`),
//! so the repaint happens in the `blur`, as for a press or Tab.
//! `focus_move_listener_reentry.rs` covers the other listeners.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_blur_inside_focus_move
//! ```
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

const HOST: &str = "data-test-host-blur-inside-focus-move";

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

#[wasm_bindgen_test]
async fn a_modal_opening_while_an_editor_holds_the_keyboard_takes_it() {
    let f = mounted();
    microtask().await;
    assert!(
        caret_visible(),
        "positive control: the focused editor draws a caret"
    );
    // The modal's focus effect runs inside this `set`, and its `focus()` blurs
    // the editor's capture textarea.
    f.open.set(true);
    assert_eq!(active_id(), "in-modal", "the modal took the keyboard");
    assert!(
        !caret_visible(),
        "the blur repainted the caret before `set` returned"
    );

    // Closing hands it back, and the editor still takes typing afterwards:
    // nothing was left borrowed by a panic.
    f.open.set(false);
    microtask().await;
    let before = f.handle.doc().content().size();
    f.handle.insert_text("!");
    assert!(
        f.handle.doc().content().size() > before,
        "the editor still edits"
    );
    f.root.unmount();
    f.host.remove();
}
