//! Browser-driven tests for `EditorHandle::on_image_input` on the web: an
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

const HOST_MARKER: &str = "data-test-host-image-input";
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

#[wasm_bindgen_test]
async fn a_pasted_image_file_is_offered_as_its_bytes_and_the_apps_src_is_inserted() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    let bytes = png_bytes(b"a screenshot");
    f.handle.set_selection(Selection::cursor(Pos(6))); // alpha| bravo

    f.paste(&[file("image.png", "image/png", &bytes)], None);
    // The read is asynchronous; the person types at the start meanwhile.
    f.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(f.handle.insert_text("X"));
    until(|| !offered.borrow().is_empty()).await;

    assert_eq!(
        *offered.borrow(),
        vec![(
            ImageInputSource::Paste,
            bytes,
            "image/png".to_string(),
            Some("image.png".to_string())
        )]
    );
    assert_eq!(f.html(), r#"<p>Xalpha<img src="app-blob:1"> bravo</p>"#);
    assert!(!f.html().contains("data:"));
    f.teardown();
}

#[wasm_bindgen_test]
async fn an_answer_that_comes_later_is_inserted_at_the_anchor() {
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
    assert_eq!(
        f.html(),
        CONTENT,
        "nothing yet, and no data: URL in its place"
    );

    let anchor = kept.borrow_mut().take().unwrap();
    assert!(f.handle.insert_image_at(&anchor, "app-blob:late", ""));
    assert_eq!(f.html(), r#"<p>alpha<img src="app-blob:late"> bravo</p>"#);
    f.teardown();
}

#[wasm_bindgen_test]
async fn copy_image_html_offers_the_file_and_html_with_words_is_an_ordinary_paste() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    let picture = file("image.png", "image/png", &png_bytes(b"copied"));

    // A browser's "Copy image": the file, and an `<img>` pointing at where it
    // was copied from. The app gets the bytes; the remote `<img>` is not pasted.
    f.paste(
        std::slice::from_ref(&picture),
        Some(r#"<meta charset="utf-8"><img src="https://example.test/cat.png" alt="cat">"#),
    );
    until(|| !offered.borrow().is_empty()).await;
    assert_eq!(f.html(), r#"<p><img src="app-blob:1">alpha bravo</p>"#);

    // Html with words as well as a picture is the html paste it always was.
    f.handle.set_selection(Selection::cursor(Pos(1)));
    f.paste(
        std::slice::from_ref(&picture),
        Some(r#"<p>see <img src="https://example.test/dog.png"></p>"#),
    );
    settle().await;
    assert_eq!(
        offered.borrow().len(),
        1,
        "the app was not offered this one"
    );
    assert!(
        f.html().contains(r#"src="https://example.test/dog.png""#),
        "{}",
        f.html()
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn with_no_callback_a_pasted_image_is_a_data_url_as_before() {
    let f = Fixture::focused();
    f.paste(&[file("image.png", "image/png", &png_bytes(b"x"))], None);
    let handle = f.handle.clone();
    until(move || node_to_html(&handle.doc()).contains("<img")).await;
    assert!(
        f.html().contains(r#"src="data:image/png;base64,"#),
        "{}",
        f.html()
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn image_files_dropped_on_the_editor_land_at_the_drop_point_in_order() {
    let f = Fixture::focused();
    let offered = f.taking_pictures();
    let first = png_bytes(b"first");
    let second = b"\xff\xd8\xff\xe0 a jpeg".to_vec();

    let (over, dropped) = f.drop_files(
        &[
            file("1.png", "image/png", &first),
            file("notes.txt", "text/plain", b"words"),
            // Called a PNG by whoever named it; its bytes say JPEG.
            file("2.png", "image/png", &second),
        ],
        f.point_at(5),
    );
    assert!(
        over,
        "the editor says it wants the drop, or the browser never delivers it"
    );
    assert!(
        dropped,
        "and takes it, so the browser does not open the file instead"
    );
    assert_eq!(
        f.handle.selection(),
        Selection::cursor(Pos(6)),
        "the caret goes to the drop point at once"
    );
    until(|| offered.borrow().len() == 2).await;

    assert_eq!(
        *offered.borrow(),
        vec![
            (
                ImageInputSource::Drop,
                first,
                "image/png".to_string(),
                Some("1.png".to_string())
            ),
            (
                ImageInputSource::Drop,
                second,
                "image/jpeg".to_string(),
                Some("2.png".to_string())
            ),
        ],
        "the image files, in order, typed by their bytes; not the text file"
    );
    assert_eq!(
        f.html(),
        r#"<p>alpha<img src="app-blob:1"><img src="app-blob:2"> bravo</p>"#
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn a_drop_on_an_editor_that_does_not_take_pictures_is_left_to_the_browser() {
    let f = Fixture::focused();
    let picture = file("1.png", "image/png", &png_bytes(b"x"));

    // No callback.
    assert_eq!(
        f.drop_files(std::slice::from_ref(&picture), f.point_at(5)),
        (false, false)
    );

    // A callback, but read-only.
    let offered = f.taking_pictures();
    f.handle.set_read_only(true);
    assert_eq!(
        f.drop_files(std::slice::from_ref(&picture), f.point_at(5)),
        (false, false)
    );
    settle().await;
    assert!(offered.borrow().is_empty());
    assert_eq!(f.html(), CONTENT);
    assert_eq!(
        f.handle.selection(),
        Selection::cursor(Pos(1)),
        "caret unmoved"
    );
    f.teardown();
}
