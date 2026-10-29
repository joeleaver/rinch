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

fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
) -> (RootHandle, web_sys::Element) {
    let host = document().create_element("div").unwrap();
    host.set_attribute("style", "width: 300px;").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    (root, host)
}

/// Each picked file's name and bytes.
type NamedBytes = Vec<(String, Vec<u8>)>;

type Got = Rc<RefCell<Vec<(String, u64, Option<std::path::PathBuf>, Vec<u8>)>>>;

/// Record each dropped file's name, size, path and (after reading) bytes.
fn recorder(got: &Got) -> impl Fn(Vec<LocalFile>) + 'static {
    let got = got.clone();
    move |files: Vec<LocalFile>| {
        for f in files {
            let got = got.clone();
            let (name, size, path) = (
                f.name().to_string(),
                f.size(),
                f.path().map(|p| p.to_path_buf()),
            );
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
    let on_drop_outer = recorder(&got);
    let (root, host) = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                div { id: "zone", style: "height: 60px;", ondropfiles: on_drop, span { "drop here" } }
                div { id: "elsewhere", style: "height: 60px;", "not a zone" }
                div { id: "outer", style: "height: 60px;", ondropfiles: on_drop_outer,
                    div { id: "inner", style: "height: 30px;", onfiledrop: |_| {}, "paths only" }
                }
            }
        }
    });
    let zone = host.query_selector("#zone").unwrap().unwrap();
    let span = zone.query_selector("span").unwrap().unwrap();
    let a = file("model.glb", "model/gltf-binary", b"glTF\x02\x00\x00\x00");
    let b = file("notes.txt", "text/plain", b"hello");
    // Dropped on a child: the nearest `ondropfiles` ancestor takes it.
    assert_eq!(
        drop_on(&span, &[a, b]),
        (true, true),
        "both events prevented over a zone"
    );
    assert!(
        wait_for(|| got.borrow().len() == 2).await,
        "both files read: {:?}",
        got.borrow()
    );
    let mut seen = got.borrow().clone();
    seen.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        seen,
        vec![
            (
                "model.glb".into(),
                8,
                None,
                b"glTF\x02\x00\x00\x00".to_vec()
            ),
            ("notes.txt".into(), 5, None, b"hello".to_vec()),
        ]
    );
    // Outside a zone the browser keeps the drop, and nothing is dispatched.
    let elsewhere = host.query_selector("#elsewhere").unwrap().unwrap();
    assert_eq!(
        drop_on(&elsewhere, &[file("x.bin", "", b"x")]),
        (false, false)
    );
    wait(30).await;
    assert_eq!(got.borrow().len(), 2);
    // Nested zones: the nearest of either kind wins, as on desktop. A
    // paths-only `onfiledrop` zone takes nothing in a browser, but the drop is
    // still prevented (the browser must not open the file); the `ondropfiles`
    // zone around it is not reached.
    let inner = host.query_selector("#inner").unwrap().unwrap();
    assert_eq!(drop_on(&inner, &[file("y.bin", "", b"y")]), (true, true));
    wait(30).await;
    assert_eq!(got.borrow().len(), 2, "the outer zone is not the nearest");
    // Positive control: the outer zone itself does take a drop.
    let outer = host.query_selector("#outer").unwrap().unwrap();
    let r = outer.get_bounding_client_rect();
    let below_inner = document()
        .element_from_point(r.x() as f32 + 5.0, (r.y() + r.height()) as f32 - 5.0)
        .unwrap();
    assert_eq!(
        drop_on(&below_inner, &[file("z.bin", "", b"z")]),
        (true, true)
    );
    assert!(wait_for(|| got.borrow().len() == 3).await);
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
    assert!(
        wait_for(|| offered.borrow().len() == 1 && got.borrow().len() == 1).await,
        "offered {:?}, dropped {:?}",
        offered.borrow(),
        got.borrow()
    );
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

/// `navigator.userActivation.isActive` (Chrome 155 and Firefox 155 have it).
fn user_activation_is_active() -> bool {
    let get = |o: &wasm_bindgen::JsValue, k: &str| {
        js_sys::Reflect::get(o, &wasm_bindgen::JsValue::from_str(k)).unwrap()
    };
    let navigator = get(&web_sys::window().unwrap(), "navigator");
    let activation = get(&navigator, "userActivation");
    get(&activation, "isActive").as_bool().unwrap()
}

#[wasm_bindgen_test]
async fn the_picker_resolves_on_change_and_cancel_and_not_without_a_gesture() {
    // A mount installs the browser's task scheduler, which `open` rides.
    let (root, host) = mount(|__scope: &mut RenderScope| rsx! { div { "picker" } });
    // No user gesture: the browser refuses `showPicker()`, and the picker
    // resolves empty rather than wait for an event that never comes. A
    // harness that grants every page a gesture (Playwright's Firefox does)
    // would open a real picker here, so the phase runs only without one.
    if !user_activation_is_active() {
        let picked: Rc<RefCell<Option<Vec<LocalFile>>>> = Rc::default();
        let into = picked.clone();
        FilePicker::new().open(move |files| *into.borrow_mut() = Some(files));
        assert!(picker_input().is_none(), "no input left without a gesture");
        assert!(wait_for(|| picked.borrow().is_some()).await);
        assert!(picked.borrow().as_ref().unwrap().is_empty());
    }

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
    let result: Rc<RefCell<Option<NamedBytes>>> = Rc::default();
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

/// The guide's example (`docs/src/guide/platform.md#files-from-the-user-web-and-desktop`),
/// verbatim, so it keeps compiling; a drop on its zone reads the file.
#[component]
fn importer() -> NodeHandle {
    let status = Signal::new(String::from("pick or drop a model"));
    let load = move |files: Vec<LocalFile>| {
        for file in files {
            spawn_local(async move {
                match file.bytes().await {
                    Ok(bytes) => status.set(format!("{}: {} bytes", file.name(), bytes.len())),
                    Err(e) => status.set(format!("{}: {e}", file.name())),
                }
            });
        }
    };
    rsx! {
        div {
            button {
                onclick: move || FilePicker::new().add_filter("glTF", &["glb", "gltf"]).open(load),
                "Import…"
            }
            div { ondropfiles: load, style: "height: 120px", "drop a model here" }
            p { {|| status.get()} }
        }
    }
}

#[wasm_bindgen_test]
async fn the_guide_example_reads_a_dropped_file() {
    let (root, host) = mount(|__scope: &mut RenderScope| importer(__scope));
    let zone = host
        .query_selector("div[data-ondropfiles]")
        .unwrap()
        .unwrap();
    assert_eq!(drop_on(&zone, &[file("m.glb", "", b"12345")]), (true, true));
    let p = host.query_selector("p").unwrap().unwrap();
    assert!(
        wait_for(|| p.text_content().as_deref() == Some("m.glb: 5 bytes")).await,
        "{:?}",
        p.text_content()
    );
    root.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn a_picker_with_no_answer_ends_when_the_window_focus_comes_back() {
    let (root, host) = mount(|__scope: &mut RenderScope| rsx! { div { "picker" } });
    rinch::file::__set_require_user_activation(false);
    rinch::file::__force_focus_return_watch(true);
    let picked: Rc<RefCell<Option<usize>>> = Rc::default();
    let into = picked.clone();
    FilePicker::new().open(move |files| *into.borrow_mut() = Some(files.len()));
    assert!(picker_input().is_some(), "the picker's input is up");
    wait(50).await;
    assert!(
        picked.borrow().is_none(),
        "no answer before the focus comes back"
    );
    web_sys::window()
        .unwrap()
        .dispatch_event(&web_sys::Event::new("focus").unwrap())
        .unwrap();
    assert!(
        wait_for(|| picked.borrow().is_some()).await,
        "resolved after focus returned"
    );
    assert_eq!(*picked.borrow(), Some(0));
    assert!(picker_input().is_none(), "and the input is gone");
    rinch::file::__force_focus_return_watch(false);
    rinch::file::__set_require_user_activation(true);
    root.unmount();
    host.remove();
}

/// Every error a listener threw while this is alive, from the window's `error`
/// event (`dispatchEvent` reports a listener's throw there; it does not rethrow).
struct ThrownErrors {
    seen: Rc<RefCell<Vec<String>>>,
    listener: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
}

impl ThrownErrors {
    fn watch() -> Self {
        let seen: Rc<RefCell<Vec<String>>> = Rc::default();
        let log = seen.clone();
        let listener = wasm_bindgen::closure::Closure::wrap(Box::new(move |e: web_sys::Event| {
            let message = js_sys::Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
                .unwrap_or_default();
            log.borrow_mut().push(message);
            // Reported here, so not again as an uncaught error.
            e.prevent_default();
        })
            as Box<dyn FnMut(web_sys::Event)>);
        web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("error", listener.as_ref().unchecked_ref())
            .unwrap();
        Self { seen, listener }
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.seen.borrow_mut())
    }
}

impl Drop for ThrownErrors {
    fn drop(&mut self) {
        let _ = web_sys::window()
            .unwrap()
            .remove_event_listener_with_callback("error", self.listener.as_ref().unchecked_ref());
    }
}

/// Issue #1855: an answered picker's input keeps no listener whose closure is
/// gone. The input stays an object the browser (or a test driver) can still
/// fire `change`/`cancel` at after the answer, and a listener left on it with
/// its closure freed threw "closure invoked recursively or after being
/// dropped" at every such event.
#[wasm_bindgen_test]
async fn an_answered_picker_input_takes_later_events_without_throwing() {
    let (root, host) = mount(|__scope: &mut RenderScope| rsx! { div { "picker" } });
    rinch::file::__set_require_user_activation(false);
    let thrown = ThrownErrors::watch();

    // Positive control: a listener's throw is seen through the window. Chrome
    // says "Uncaught Error: control throw", Firefox "Error: control throw".
    let control = document().create_element("div").unwrap();
    let throwing = js_sys::Function::new_no_args("throw new Error('control throw')");
    control
        .add_event_listener_with_callback("ping", &throwing)
        .unwrap();
    control
        .dispatch_event(&web_sys::Event::new("ping").unwrap())
        .unwrap();
    let seen = thrown.take();
    assert!(
        seen.len() == 1 && seen[0].ends_with("Error: control throw"),
        "{seen:?}"
    );

    // Answered with `change`; then the same input gets `change` and `cancel`
    // again, as when a driver's chooser is torn down after its pick.
    let picked: Rc<RefCell<Vec<usize>>> = Rc::default();
    let into = picked.clone();
    FilePicker::new().open(move |files| into.borrow_mut().push(files.len()));
    let input = picker_input().expect("the picker's input is in the page");
    let dt = web_sys::DataTransfer::new().unwrap();
    dt.items()
        .add_with_file(&file("a.glb", "", b"abc"))
        .unwrap();
    input.set_files(dt.files().as_ref());
    input
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    assert!(wait_for(|| !picked.borrow().is_empty()).await);
    wait(20).await;
    for name in ["change", "cancel", "change"] {
        input
            .dispatch_event(&web_sys::Event::new(name).unwrap())
            .unwrap();
    }
    wait(20).await;
    assert_eq!(
        thrown.take(),
        Vec::<String>::new(),
        "after a `change` answer"
    );
    assert_eq!(*picked.borrow(), vec![1], "answered once");

    // Answered with `cancel`, then fired at again.
    let cancelled: Rc<RefCell<Vec<usize>>> = Rc::default();
    let into = cancelled.clone();
    FilePicker::new().open(move |files| into.borrow_mut().push(files.len()));
    let input = picker_input().expect("a second picker input");
    input
        .dispatch_event(&web_sys::Event::new("cancel").unwrap())
        .unwrap();
    assert!(wait_for(|| !cancelled.borrow().is_empty()).await);
    wait(20).await;
    for name in ["cancel", "change"] {
        input
            .dispatch_event(&web_sys::Event::new(name).unwrap())
            .unwrap();
    }
    wait(20).await;
    assert_eq!(
        thrown.take(),
        Vec::<String>::new(),
        "after a `cancel` answer"
    );
    assert_eq!(*cancelled.borrow(), vec![0], "answered once");

    // A picker whose future is dropped unanswered: its input still gets the
    // browser's answer later.
    let pick = FilePicker::new().pick();
    let input = picker_input().expect("a third picker input");
    drop(pick);
    assert!(picker_input().is_none(), "a dropped picker takes its input");
    for name in ["change", "cancel"] {
        input
            .dispatch_event(&web_sys::Event::new(name).unwrap())
            .unwrap();
    }
    wait(20).await;
    assert_eq!(
        thrown.take(),
        Vec::<String>::new(),
        "after the future was dropped"
    );

    drop(thrown);
    rinch::file::__set_require_user_activation(true);
    root.unmount();
    host.remove();
}
