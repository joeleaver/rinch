//! Issue #1822: an `ondropfiles` handler reads the drop point through
//! `get_click_context()` in a browser, as it does on the desktop (#1650): the
//! `drop` event's client position, the zone element's box (not the child the
//! drop landed on), the viewport, and the modifiers held. The files an editor
//! hands back from a mixed drop carry the same context.
//!
//! Every fixture drops off the zone's centre and off its origin, holds a
//! modifier, and first writes a context that is wrong in every field, so a
//! dispatch that sets no context cannot pass by agreeing with a default or
//! with the last one.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test drop_point_1822
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::file::LocalFile;
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::events::{ClickContext, ModifierState, MouseButton, TextHitInfo};
use rinch_web::{ImageInput, RootHandle, create_editor};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn wait(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

fn file(name: &str, mime: &str, bytes: &[u8]) -> web_sys::File {
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes).buffer());
    let options = web_sys::FilePropertyBag::new();
    options.set_type(mime);
    web_sys::File::new_with_buffer_source_sequence_and_options(&parts, name, &options).unwrap()
}

/// A context no correct dispatch can leave behind.
fn poison() {
    rinch_core::events::set_click_context(ClickContext {
        mouse_x: -1.0,
        mouse_y: -1.0,
        element_x: -1.0,
        element_y: -1.0,
        element_width: -1.0,
        element_height: -1.0,
        text_hit: TextHitInfo::default(),
        viewport_width: -1.0,
        viewport_height: -1.0,
        button: MouseButton::Right,
        modifiers: ModifierState::default(),
    });
}

/// Fire `dragover` then `drop` carrying `files` at `target`, at client point
/// `(x, y)`, with `modifiers` held; answers whether each was prevented.
fn drop_at(
    target: &web_sys::Element,
    files: &[web_sys::File],
    (x, y): (i32, i32),
    modifiers: ModifierState,
) -> (bool, bool) {
    let dt = web_sys::DataTransfer::new().unwrap();
    for f in files {
        dt.items().add_with_file(f).unwrap();
    }
    let fire = |name: &str| {
        let init = web_sys::DragEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_client_x(x);
        init.set_client_y(y);
        init.set_shift_key(modifiers.shift);
        init.set_ctrl_key(modifiers.ctrl);
        init.set_alt_key(modifiers.alt);
        init.set_meta_key(modifiers.meta);
        init.set_data_transfer(Some(&dt));
        let ev = web_sys::DragEvent::new_with_event_init_dict(name, &init).unwrap();
        target.dispatch_event(&ev).unwrap();
        ev.default_prevented()
    };
    (fire("dragover"), fire("drop"))
}

fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
) -> (RootHandle, web_sys::Element) {
    let host = document().create_element("div").unwrap();
    host.set_attribute("style", "width: 400px;").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    (root, host)
}

type Seen = Rc<RefCell<Vec<(Vec<String>, ClickContext)>>>;

/// Record each drop's file names and the click context the handler reads.
fn recorder(seen: &Seen) -> impl Fn(Vec<LocalFile>) + 'static {
    let seen = seen.clone();
    move |files: Vec<LocalFile>| {
        let names = files.iter().map(|f| f.name().to_string()).collect();
        seen.borrow_mut()
            .push((names, rinch_core::events::get_click_context()));
    }
}

fn viewport() -> (f32, f32) {
    let w = web_sys::window().unwrap();
    (
        w.inner_width().unwrap().as_f64().unwrap() as f32,
        w.inner_height().unwrap().as_f64().unwrap() as f32,
    )
}

fn assert_box(ctx: &ClickContext, el: &web_sys::Element) {
    let r = el.get_bounding_client_rect();
    let want = (
        r.x() as f32,
        r.y() as f32,
        r.width() as f32,
        r.height() as f32,
    );
    let got = (
        ctx.element_x,
        ctx.element_y,
        ctx.element_width,
        ctx.element_height,
    );
    assert!(
        (got.0 - want.0).abs() < 0.01
            && (got.1 - want.1).abs() < 0.01
            && (got.2 - want.2).abs() < 0.01
            && (got.3 - want.3).abs() < 0.01,
        "element box {got:?}, wanted the zone's {want:?}"
    );
}

/// A point inside `el`, `(dx, dy)` from its top-left corner.
fn point_in(el: &web_sys::Element, (dx, dy): (f64, f64)) -> (i32, i32) {
    let r = el.get_bounding_client_rect();
    ((r.x() + dx).round() as i32, (r.y() + dy).round() as i32)
}

#[wasm_bindgen_test]
async fn an_ondropfiles_handler_reads_the_drop_point_and_its_zones_box() {
    let seen: Seen = Rc::default();
    let on_drop = recorder(&seen);
    let (root, host) = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                div {
                    id: "zone",
                    style: "margin: 17px 0 0 23px; width: 210px; height: 90px; padding: 5px;",
                    ondropfiles: on_drop,
                    span { id: "child", "drop here" }
                }
            }
        }
    });
    let zone = host.query_selector("#zone").unwrap().unwrap();
    let child = host.query_selector("#child").unwrap().unwrap();
    // Off the zone's origin and centre, on its child.
    let (x, y) = point_in(&child, (11.0, 7.0));
    let held = ModifierState {
        shift: true,
        alt: true,
        ..ModifierState::default()
    };
    poison();
    assert_eq!(
        drop_at(&child, &[file("model.glb", "", b"glTF")], (x, y), held),
        (true, true)
    );
    let got = seen.borrow().clone();
    assert_eq!(got.len(), 1, "the handler ran once");
    assert_eq!(got[0].0, vec!["model.glb".to_string()]);
    let ctx = got[0].1;
    assert_eq!((ctx.mouse_x, ctx.mouse_y), (x as f32, y as f32));
    assert_box(&ctx, &zone);
    assert_eq!((ctx.viewport_width, ctx.viewport_height), viewport());
    assert_eq!(ctx.button, MouseButton::Left);
    assert!(
        ctx.modifiers.shift && ctx.modifiers.alt && !ctx.modifiers.ctrl && !ctx.modifiers.meta,
        "modifiers {:?}",
        ctx.modifiers
    );
    root.unmount();
    host.remove();
}

/// The files of a mixed drop an editor does not take reach the zone around it
/// with the drop point and the zone's box.
#[wasm_bindgen_test]
async fn the_files_an_editor_hands_back_carry_the_drop_point() {
    let seen: Seen = Rc::default();
    let on_drop = recorder(&seen);
    let editor = create_editor();
    assert!(editor.load_html("<p>alpha bravo</p>"));
    editor.on_image_input(|_: ImageInput| None);
    let mounted = editor.clone();
    let (root, host) = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                id: "zone",
                style: "margin: 9px 0 0 31px; padding: 13px;",
                ondropfiles: on_drop,
                {mounted.mount(__scope)}
            }
        }
    });
    let zone = host.query_selector("#zone").unwrap().unwrap();
    let para = host.query_selector("[data-pm-editor] p").unwrap().unwrap();
    let (x, y) = point_in(&para, (6.0, 4.0));
    let held = ModifierState {
        ctrl: true,
        ..ModifierState::default()
    };
    poison();
    let png = file("pic.png", "image/png", b"\x89PNG\r\n\x1a\nrest");
    let pdf = file("doc.pdf", "application/pdf", b"%PDF-1.7");
    assert_eq!(drop_at(&para, &[png, pdf], (x, y), held), (true, true));
    wait(10).await;
    let got = seen.borrow().clone();
    assert_eq!(got.len(), 1, "the zone got the PDF");
    assert_eq!(got[0].0, vec!["doc.pdf".to_string()]);
    let ctx = got[0].1;
    assert_eq!((ctx.mouse_x, ctx.mouse_y), (x as f32, y as f32));
    assert_box(&ctx, &zone);
    assert_eq!((ctx.viewport_width, ctx.viewport_height), viewport());
    assert!(ctx.modifiers.ctrl && !ctx.modifiers.shift, "{:?}", ctx.modifiers);
    root.unmount();
    host.remove();
}
