//! Files from the user in a browser (issue #1771): an `ondropfiles` element
//! receives the files of a real `drop` `DragEvent` as `LocalFile`s whose bytes
//! read back; an editor that takes pictures hands the other files of a drop to
//! it (desktop's rule); and the `FilePicker`'s hidden input resolves on
//! `change` and `cancel`, and resolves empty outside a user gesture instead of
//! waiting for ever.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test files_1771
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::file::{FilePicker, LocalFile};
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{ImageInput, RootHandle, create_editor};
use wasm_bindgen::JsCast;
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

async fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if done() {
            return true;
        }
        wait(10).await;
    }
    done()
}

fn file(name: &str, mime: &str, bytes: &[u8]) -> web_sys::File {
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes).buffer());
    let options = web_sys::FilePropertyBag::new();
    options.set_type(mime);
    web_sys::File::new_with_buffer_source_sequence_and_options(&parts, name, &options).unwrap()
}

/// Fire `dragover` then `drop` carrying `files` at `target`'s centre; answers
/// whether each was prevented.
fn drop_on(target: &web_sys::Element, files: &[web_sys::File]) -> (bool, bool) {
    let dt = web_sys::DataTransfer::new().unwrap();
    for f in files {
        dt.items().add_with_file(f).unwrap();
    }
    let r = target.get_bounding_client_rect();
    let (x, y) = (r.x() + r.width() / 2.0, r.y() + r.height() / 2.0);
    let fire = |name: &str| {
        let init = web_sys::DragEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_client_x(x as i32);
        init.set_client_y(y as i32);
        init.set_data_transfer(Some(&dt));
        let ev = web_sys::DragEvent::new_with_event_init_dict(name, &init).unwrap();
        target.dispatch_event(&ev).unwrap();
        ev.default_prevented()
    };
    (fire("dragover"), fire("drop"))
}

fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> (RootHandle, web_sys::Element) {
    let host = document().create_element("div").unwrap();
    host.set_attribute("style", "width: 300px;").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    (root, host)
}

type Got = Rc<RefCell<Vec<(String, u64, Option<std::path::PathBuf>, Vec<u8>)>>>;

/// Record each dropped file's name, size, path and (after reading) bytes.
fn recorder(got: &Got) -> impl Fn(Vec<LocalFile>) + 'static {
    let got = got.clone();
    move |files: Vec<LocalFile>| {
        for f in files {
            let got = got.clone();
            let (name, size, path) = (f.name().to_string(), f.size(), f.path().map(|p| p.to_path_buf()));
            f.read(move |bytes| {
                got.borrow_mut()
                    .push((name, size, path, bytes.expect("a dropped file reads")));
            });
        }
    }
}

#[wasm_bindgen_test]
async fn dropped_files_reach_ondropfiles_with_their_names_and_bytes() {
    let got: Got = Rc::default();
    let on_drop = recorder(&got);
    let (root, host) = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                div { id: "zone", style: "height: 60px;", ondropfiles: on_drop, span { "drop here" } }
                div { id: "elsewhere", style: "height: 60px;", "not a zone" }
            }
        }
    });
    let zone = host.query_selector("#zone").unwrap().unwrap();
    let span = zone.query_selector("span").unwrap().unwrap();
    let a = file("model.glb", "model/gltf-binary", b"glTF\x02\x00\x00\x00");
    let b = file("notes.txt", "text/plain", b"hello");
    // Dropped on a child: the nearest `ondropfiles` ancestor takes it.
    assert_eq!(drop_on(&span, &[a, b]), (true, true), "both events prevented over a zone");
    assert!(wait_for(|| got.borrow().len() == 2).await, "both files read: {:?}", got.borrow());
    let mut seen = got.borrow().clone();
    seen.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        seen,
        vec![
            ("model.glb".into(), 8, None, b"glTF\x02\x00\x00\x00".to_vec()),
            ("notes.txt".into(), 5, None, b"hello".to_vec()),
        ]
    );
    // Outside a zone the browser keeps the drop, and nothing is dispatched.
    let elsewhere = host.query_selector("#elsewhere").unwrap().unwrap();
    assert_eq!(drop_on(&elsewhere, &[file("x.bin", "", b"x")]), (false, false));
    wait(30).await;
    assert_eq!(got.borrow().len(), 2);
    root.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn an_editor_that_takes_pictures_hands_the_other_files_to_ondropfiles() {
    let got: Got = Rc::default();
    let on_drop = recorder(&got);
    let offered: Rc<RefCell<Vec<String>>> = Rc::default();
    let editor = create_editor();
    assert!(editor.load_html("<p>alpha</p>"));
    let log = offered.clone();
    editor.on_image_input(move |input: ImageInput| {
        log.borrow_mut().push(input.name.unwrap_or_default());
        Some(("app-blob:1".into(), String::new()))
    });
    let mounted = editor.clone();
    let (root, host) = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div { id: "zone", ondropfiles: on_drop, {mounted.mount(__scope)} }
        }
    });
    let para = host.query_selector("[data-pm-editor] p").unwrap().unwrap();
    let png = file("pic.png", "image/png", b"\x89PNG\r\n\x1a\nrest");
    let pdf = file("doc.pdf", "application/pdf", b"%PDF-1.7");
    assert_eq!(drop_on(&para, &[png, pdf]), (true, true));
    assert!(wait_for(|| offered.borrow().len() == 1 && got.borrow().len() == 1).await);
    assert_eq!(*offered.borrow(), vec!["pic.png".to_string()]);
    assert_eq!(got.borrow()[0].0, "doc.pdf");
    // Read-only: the editor takes nothing, so every file is the zone's.
    editor.set_read_only(true);
    let png = file("pic2.png", "image/png", b"\x89PNG\r\n\x1a\nmore");
    assert_eq!(drop_on(&para, &[png]), (true, true));
    assert!(wait_for(|| got.borrow().len() == 2).await);
    assert_eq!(got.borrow()[1].0, "pic2.png");
    assert_eq!(offered.borrow().len(), 1);
    root.unmount();
    host.remove();
}

/// The picker's input, if one is in the page.
fn picker_input() -> Option<web_sys::HtmlInputElement> {
    document()
        .query_selector(&format!("input[{}]", rinch::file::__PICKER_ATTR))
        .unwrap()
        .map(|e| e.unchecked_into())
}

#[wasm_bindgen_test]
async fn the_picker_resolves_on_change_and_cancel_and_not_without_a_gesture() {
    // A mount installs the browser's task scheduler, which `open` rides.
    let (root, host) = mount(|__scope: &mut RenderScope| rsx! { div { "picker" } });
    // No user gesture in a test: the picker says so and resolves empty
    // rather than wait for an event that never comes.
    let picked: Rc<RefCell<Option<Vec<LocalFile>>>> = Rc::default();
    let into = picked.clone();
    FilePicker::new().open(move |files| *into.borrow_mut() = Some(files));
    assert!(picker_input().is_none(), "no input without a gesture");
    assert!(wait_for(|| picked.borrow().is_some()).await);
    assert!(picked.borrow().as_ref().unwrap().is_empty());

    rinch::file::__set_require_user_activation(false);
    // `change`: the files picked, filters as `accept`.
    let pick = FilePicker::new()
        .add_filter("glTF", &["glb", "gltf"])
        .multiple(true)
        .pick();
    let input = picker_input().expect("the picker's input is in the page");
    assert_eq!(input.accept(), ".glb,.gltf");
    assert!(input.multiple());
    let dt = web_sys::DataTransfer::new().unwrap();
    dt.items()
        .add_with_file(&file("a.glb", "", b"abc"))
        .unwrap();
    input.set_files(dt.files().as_ref());
    input
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    let result: Rc<RefCell<Option<Vec<(String, Vec<u8>)>>>> = Rc::default();
    let out = result.clone();
    spawn_local(async move {
        let mut v = Vec::new();
        for f in pick.await {
            v.push((f.name().to_string(), f.bytes().await.unwrap()));
        }
        *out.borrow_mut() = Some(v);
    });
    assert!(wait_for(|| result.borrow().is_some()).await);
    assert_eq!(
        result.borrow().clone().unwrap(),
        vec![("a.glb".to_string(), b"abc".to_vec())]
    );
    assert!(picker_input().is_none(), "the input goes once answered");

    // `cancel`: no files.
    let cancelled: Rc<RefCell<Option<usize>>> = Rc::default();
    let into = cancelled.clone();
    FilePicker::new().open(move |files| *into.borrow_mut() = Some(files.len()));
    let input = picker_input().expect("a second picker input");
    input
        .dispatch_event(&web_sys::Event::new("cancel").unwrap())
        .unwrap();
    assert!(wait_for(|| cancelled.borrow().is_some()).await);
    assert_eq!(*cancelled.borrow(), Some(0));
    assert!(picker_input().is_none());
    rinch::file::__set_require_user_activation(true);
    root.unmount();
    host.remove();
}
