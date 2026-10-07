//! Review of PR #1436: what its own browser fixtures do not sample.
//! image file on a real `paste` `ClipboardEvent`, and image files on a real
//! `drop` `DragEvent` over the editor, are offered to the app as their bytes,
//! and the `src` the app answers with is what the document carries. No `data:`
//! URL is made while the callback is registered.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_image_input
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{
    EditorHandle, ImageInput, ImageInputSource, RootHandle, SelectionAnchor, create_editor,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-review-1436";
const CONTENT: &str = "<p>alpha bravo</p>";

/// A PNG by its first bytes; the rest is whatever, nothing here decodes it.
fn png_bytes(tail: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(tail);
    bytes
}

fn file(name: &str, mime: &str, bytes: &[u8]) -> web_sys::File {
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes).buffer());
    let options = web_sys::FilePropertyBag::new();
    options.set_type(mime);
    web_sys::File::new_with_buffer_source_sequence_and_options(&parts, name, &options).unwrap()
}

/// What the app was offered, minus the anchor.
type Offered = Rc<RefCell<Vec<(ImageInputSource, Vec<u8>, String, Option<String>)>>>;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    /// One editor over [`CONTENT`], focused by a real press, caret at 1.
    fn focused() -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(el) = stale
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(CONTENT));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        let f = Self { root, host, handle };
        let (x, y) = f.point_at(1);
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(f.capture().as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
        f.handle.set_selection(Selection::cursor(Pos(1)));
        f
    }

    /// Register a callback that records every offer and answers `app-blob:N`.
    fn taking_pictures(&self) -> Offered {
        let offered: Offered = Rc::default();
        let log = offered.clone();
        self.handle.on_image_input(move |input: ImageInput| {
            let mut log = log.borrow_mut();
            log.push((input.source, input.bytes, input.mime, input.name));
            Some((format!("app-blob:{}", log.len()), String::new()))
        });
        offered
    }

    /// The viewport point between chars `i - 1` and `i` of the paragraph.
    fn point_at(&self, i: u32) -> (f32, f32) {
        let para = self
            .host
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("the paragraph");
        let text = para.first_child().expect("its text");
        let range = document().create_range().unwrap();
        range.set_start(&text, i).unwrap();
        range.set_end(&text, i).unwrap();
        let r = range.get_bounding_client_rect();
        (r.x() as f32, (r.y() + r.height() / 2.0) as f32)
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    fn html(&self) -> String {
        node_to_html(&self.handle.doc())
    }

    /// A `paste` on the capture textarea carrying `files` and, if given,
    /// `text/html`, as the browser fires it.
    fn paste(&self, files: &[web_sys::File], html: Option<&str>) {
        let dt = web_sys::DataTransfer::new().unwrap();
        if let Some(html) = html {
            dt.set_data("text/html", html).unwrap();
        }
        for file in files {
            dt.items().add_with_file(file).unwrap();
        }
        let init = web_sys::ClipboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_clipboard_data(Some(&dt));
        let ev = web_sys::ClipboardEvent::new_with_event_init_dict("paste", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
    }

    /// A `dragover` then a `drop` of `files` at `(x, y)`, on whatever is under
    /// that point. Answers whether each was taken (`preventDefault`).
    fn drop_files(&self, files: &[web_sys::File], (x, y): (f32, f32)) -> (bool, bool) {
        let dt = web_sys::DataTransfer::new().unwrap();
        for file in files {
            dt.items().add_with_file(file).unwrap();
        }
        let target = document()
            .element_from_point(x, y)
            .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
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

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
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

/// Let pending file reads finish: wait (bounded) until `done`.
async fn until(done: impl Fn() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        let tick = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 10)
                .unwrap();
        });
        let _ = wasm_bindgen_futures::JsFuture::from(tick).await;
    }
    panic!("the file read never completed");
}

/// A fixed wait, for asserting that something did *not* happen.
async fn settle() {
    let tick = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 80)
            .unwrap();
    });
    let _ = wasm_bindgen_futures::JsFuture::from(tick).await;
}

/// Positive control for the whole file: a genuine `paste` reaches rinch and the
/// app is offered the file.
#[wasm_bindgen_test]
async fn w0_control_a_paste_reaches_the_app() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    f.paste(&[file("image.png", "image/png", &png_bytes(b"x"))], None);
    until(|| !offered.borrow().is_empty()).await;
    f.teardown();
}

/// W1. A picture dropped on a READ-ONLY editor whose app takes pictures: the
/// editor does not take it, and the browser is left to its default, which for a
/// dropped file is to navigate the tab to that file. The app is gone.
#[wasm_bindgen_test]
async fn w1_a_drop_on_a_read_only_editor_does_not_navigate_the_page_away() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    f.handle.set_read_only(true);
    let picture = file("1.png", "image/png", &png_bytes(b"x"));
    let (over, dropped) = f.drop_files(std::slice::from_ref(&picture), f.point_at(5));
    settle().await;
    assert!(offered.borrow().is_empty(), "control: nothing offered");
    assert_eq!(f.html(), CONTENT, "control: nothing inserted");
    let html = f.html();
    f.teardown();
    assert!(
        over && dropped,
        "default not prevented (dragover {over}, drop {dropped}): the browser opens the file in place of the app; doc {html}"
    );
}

/// W2. A drop of files none of which is a picture, on an editor that takes
/// pictures: prevented (the page is not navigated), nothing offered, caret
/// where it was.
#[wasm_bindgen_test]
async fn w2_a_drop_with_no_picture_is_prevented_and_changes_nothing() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    let (over, dropped) = f.drop_files(
        &[file("report.pdf", "application/pdf", b"%PDF-1.7")],
        f.point_at(5),
    );
    settle().await;
    assert_eq!((over, dropped), (true, true));
    assert!(offered.borrow().is_empty());
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(1)));
    f.teardown();
}

/// W3. What the web offers that desktop never does: a file the browser calls an
/// image whose bytes are none of the four sniffed formats (an SVG). Pins the
/// documented divergence: offered, typed as the browser typed it.
#[wasm_bindgen_test]
async fn w3_an_svg_is_offered_on_the_web_typed_by_the_browser() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    f.paste(
        &[file(
            "logo.svg",
            "image/svg+xml",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
        )],
        None,
    );
    until(|| !offered.borrow().is_empty()).await;
    assert_eq!(offered.borrow()[0].2, "image/svg+xml");
    f.teardown();
}

/// W4. The picture is being read/stored and the person has moved on: a late
/// answer must not take their caret (the web half of review fixture R1).
#[wasm_bindgen_test]
async fn w4_a_late_answer_does_not_take_the_persons_caret() {
    let f = Fixture::focused();
    let kept: Rc<RefCell<Option<SelectionAnchor>>> = Rc::default();
    let kept_in = kept.clone();
    f.handle.on_image_input(move |input| {
        *kept_in.borrow_mut() = Some(input.anchor);
        None
    });
    f.handle.set_selection(Selection::cursor(Pos(6)));
    f.paste(&[file("image.png", "image/png", &png_bytes(b"x"))], None);
    until(|| kept.borrow().is_some()).await;
    // The person goes on typing at the end of the line.
    f.handle.set_selection(Selection::cursor(Pos(12)));
    assert!(f.handle.insert_text("!"));
    let anchor = kept.borrow_mut().take().unwrap();
    assert!(f.handle.insert_image_at(&anchor, "app-blob:late", ""));
    assert!(f.handle.insert_text("?"));
    let html = f.html();
    f.teardown();
    assert_eq!(html, r#"<p>alpha<img src="app-blob:late"> bravo!?</p>"#);
}

/// W5. A drop on an editor that did not have the keyboard gives it the keyboard
/// (the PR's own drop fixture starts from a focused editor, so removing
/// `handle.focus()` from `on_drop` survives it).
#[wasm_bindgen_test]
async fn w5_a_drop_focuses_an_unfocused_editor() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    f.capture().blur().unwrap();
    assert!(
        document().active_element().as_deref() != Some(f.capture().as_ref()),
        "control: blurred"
    );
    let (over, dropped) = f.drop_files(
        &[file("1.png", "image/png", &png_bytes(b"x"))],
        f.point_at(5),
    );
    assert_eq!((over, dropped), (true, true));
    until(|| !offered.borrow().is_empty()).await;
    let focused = document().active_element().as_deref() == Some(f.capture().as_ref());
    f.teardown();
    assert!(focused, "the editor took the keyboard");
}

/// W6. The file read is asynchronous in a browser: the person pastes a picture
/// and moves on before it has been read. An app that answers at once gets the
/// picture where the paste was aimed, and the person's caret stays where they
/// went (the read's completion offers at the anchor, not at the live caret).
#[wasm_bindgen_test]
async fn w6_a_picture_read_after_the_person_moved_on_leaves_their_caret() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    f.handle.set_selection(Selection::cursor(Pos(6)));
    f.paste(&[file("image.png", "image/png", &png_bytes(b"x"))], None);
    assert!(offered.borrow().is_empty(), "control: not read yet");
    f.handle.set_selection(Selection::cursor(Pos(12)));
    until(|| !offered.borrow().is_empty()).await;
    let caret = f.handle.selection();
    assert!(f.handle.insert_text("?"));
    let html = f.html();
    f.teardown();
    assert_eq!(caret, Selection::cursor(Pos(13)), "after `bravo`, shifted");
    assert_eq!(html, r#"<p>alpha<img src="app-blob:1"> bravo?</p>"#);
}

/// W7. Over a read-only editor whose app takes pictures the drag shows the
/// "not here" cursor (`dropEffect` `none`), where an editable one says `copy`.
#[wasm_bindgen_test]
async fn w7_a_drag_over_a_read_only_editor_says_none() {
    let f = Fixture::focused();
    let _offered = f.taking_pictures();
    let effect = |f: &Fixture| {
        let dt = web_sys::DataTransfer::new().unwrap();
        dt.items()
            .add_with_file(&file("1.png", "image/png", &png_bytes(b"x")))
            .unwrap();
        let (x, y) = f.point_at(5);
        let init = web_sys::DragEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_client_x(x as i32);
        init.set_client_y(y as i32);
        init.set_data_transfer(Some(&dt));
        let ev = web_sys::DragEvent::new_with_event_init_dict("dragover", &init).unwrap();
        document()
            .element_from_point(x, y)
            .unwrap()
            .dispatch_event(&ev)
            .unwrap();
        (ev.default_prevented(), dt.drop_effect())
    };
    let editable = effect(&f);
    f.handle.set_read_only(true);
    let read_only = effect(&f);
    f.teardown();
    assert_eq!(editable, (true, "copy".to_string()), "control");
    assert_eq!(read_only, (true, "none".to_string()));
}
