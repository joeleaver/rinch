//! An editor that unmounts while its hidden capture `<textarea>` holds focus
//! leaves nothing of its document behind in that field (issue #1112).
//!
//! The capture textarea mirrors the caret's textblock (#1101: U+FFFC for an
//! image, `\n` for a hard break) and carries the model selection. It used to
//! stay focused, full and selected after the editor went away, and `on_copy`
//! returned without `preventDefault` once no editor was focused — so a Ctrl+C
//! then was the browser's native copy of the stale mirror.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_unmount_capture
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-1112";
const GIF: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

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

/// Dispatch a `copy`/`cut` at `target`; answers (defaultPrevented, text/plain).
fn clipboard_event(target: &web_sys::EventTarget, name: &str) -> (bool, String) {
    let dt = web_sys::DataTransfer::new().unwrap();
    let init = web_sys::ClipboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_clipboard_data(Some(&dt));
    let ev = web_sys::ClipboardEvent::new_with_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
    (ev.default_prevented(), dt.get_data("text/plain").unwrap())
}

async fn microtask() {
    let p = js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL);
    wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
}

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

/// `<p>ab<img>cd</p>`, focused by a press, the selection over `b<img>c`.
async fn mounted_with_selection() -> F {
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
        "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 400px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html(&format!(
        "<p>ab<img src=\"{GIF}\" alt=\"\" style=\"width: 16px; height: 16px\">cd</p>"
    )));
    let m = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| m.mount(s),
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
        "positive control: a press reached rinch and focused the capture textarea"
    );
    handle.set_selection(Selection::text(Pos(2), Pos(5)));
    microtask().await;
    // Positive control: the mirror is live, carrying the image's U+FFFC, and a
    // copy while the editor is focused is rinch's (the model's own text).
    assert_eq!(ta.value(), "ab\u{FFFC}cd", "the mirror carries the block");
    let (prevented, plain) = clipboard_event(ta.as_ref(), "copy");
    assert!(prevented, "a focused editor's copy is rinch's");
    assert!(!plain.contains('\u{FFFC}'), "the model copy: {plain:?}");
    F { root, host, handle }
}

#[wasm_bindgen_test]
async fn unmounting_the_focused_editor_empties_and_releases_the_capture_field() {
    let f = mounted_with_selection().await;
    let ta = capture();
    f.root.unmount();
    assert_eq!(ta.value(), "", "no mirror text left in the field");
    assert!(
        document().active_element().as_ref() != Some(ta.as_ref()),
        "the capture field no longer holds the keyboard"
    );
    drop(f.handle);
    f.host.remove();
}

#[wasm_bindgen_test]
async fn a_copy_or_cut_on_the_capture_field_after_unmount_is_not_the_browsers() {
    let f = mounted_with_selection().await;
    let ta = capture();
    f.root.unmount();
    // Whatever focus did, a clipboard event aimed at the capture field with no
    // editor behind it must not fall through to a native copy of the field.
    for name in ["copy", "cut"] {
        let (prevented, plain) = clipboard_event(ta.as_ref(), name);
        assert!(prevented, "{name} after unmount must be prevented");
        assert_eq!(plain, "", "{name} after unmount writes nothing");
    }
    drop(f.handle);
    f.host.remove();
}
