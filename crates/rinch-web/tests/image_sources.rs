//! Browser-driven tests for app-resolved image sources: an `<img>` whose `src`
//! has a scheme the app registered shows what the app resolves it to, the
//! source it was given stays its identity, and `rinch::image::reload_image`
//! makes every element naming a source ask again.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test image_sources
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rinch::image::{ImageLoadResult, register_image_scheme, reload_image, unregister_image_scheme};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::serialize::node_to_html;
use rinch_web::{LOGICAL_SRC_ATTR, RootHandle, create_editor, register_image_url_scheme};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// A 3x2 opaque-red PNG.
const RED_3X2_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x9d, 0x74, 0x66,
    0x1a, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x86, 0x19, 0x90, 0x39, 0x00, 0x9b, 0x7e, 0x0b, 0xf5, 0x0f, 0x5f, 0x26, 0x22, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// A 1x1 PNG.
const ONE_BY_ONE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x56, 0xc7, 0x2f, 0x0d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-image-sources";

/// A mounted root holding one `<img id=…>` per `(id, src)`.
struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handles: Rc<RefCell<HashMap<&'static str, NodeHandle>>>,
}

impl Fixture {
    fn with_images(images: &'static [(&'static str, &'static str)]) -> Self {
        Self::mount(move |scope| {
            let root = scope.create_element("div");
            let mut made = HashMap::new();
            for (id, src) in images {
                let img = scope.create_element("img");
                img.set_attribute("id", id);
                img.set_attribute("src", src);
                root.append_child(&img);
                made.insert(*id, img);
            }
            (root, made)
        })
    }

    fn mount(
        build: impl FnOnce(&mut RenderScope) -> (NodeHandle, HashMap<&'static str, NodeHandle>)
        + 'static,
    ) -> Self {
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
        document().body().unwrap().append_child(&host).unwrap();
        let handles = Rc::new(RefCell::new(HashMap::new()));
        let handles_in = handles.clone();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
            let (root, made) = build(scope);
            *handles_in.borrow_mut() = made;
            root
        });
        Self {
            root,
            host,
            handles,
        }
    }

    fn img(&self, id: &str) -> web_sys::HtmlImageElement {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into()
            .unwrap()
    }

    /// What the browser was given to load.
    fn shown(&self, id: &str) -> Option<String> {
        self.img(id).get_attribute("src")
    }

    /// What rinch answers for the element's `src`.
    fn src(&self, id: &str) -> Option<String> {
        self.handles.borrow()[id].get_attribute("src")
    }

    fn handle(&self, id: &str) -> NodeHandle {
        self.handles.borrow()[id].clone()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        rinch_web::__reset_image_sources();
    }
}

/// Wait until the browser has decoded `img`'s **current** `src` (or failed
/// to), and answer its natural size.
///
/// `decode()` alone is not that in Firefox: called right after `src` changed,
/// it resolves for the image the element was already showing, while the new
/// request is still pending (measured, Firefox 157: the 3x2 picture's size
/// after the 1x1 one was set). So this also waits, bounded, until the element
/// is `complete` on the URL it was last given.
async fn decoded(img: &web_sys::HtmlImageElement) -> (u32, u32) {
    let _ = wasm_bindgen_futures::JsFuture::from(img.decode()).await;
    for _ in 0..200 {
        if img.complete() && img.current_src() == img.src() {
            break;
        }
        let tick = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 10)
                .unwrap();
        });
        let _ = wasm_bindgen_futures::JsFuture::from(tick).await;
    }
    let _ = wasm_bindgen_futures::JsFuture::from(img.decode()).await;
    (img.natural_width(), img.natural_height())
}

/// An app's store of object URLs by source, as `register_image_url_scheme`'s
/// resolver reads it: a source it does not hold yet is "not yet".
#[derive(Clone, Default)]
struct Urls {
    held: Rc<RefCell<HashMap<String, String>>>,
    asked: Rc<Cell<u32>>,
}

impl Urls {
    fn register(&self, scheme: &str) {
        let urls = self.clone();
        register_image_url_scheme(scheme, move |src| {
            urls.asked.set(urls.asked.get() + 1);
            urls.held.borrow().get(src).cloned()
        });
    }
    fn put(&self, src: &str, url: &str) {
        self.held.borrow_mut().insert(src.into(), url.into());
    }
}

#[wasm_bindgen_test]
fn a_registered_scheme_shows_the_apps_url_and_keeps_the_source_it_was_given() {
    let urls = Urls::default();
    urls.register("web-img-a");
    urls.put("web-img-a:cat", "https://example.test/resolved-cat.png");

    let f = Fixture::with_images(&[
        ("mine", "web-img-a:cat"),
        ("plain", "pictures/elsewhere.png"),
        ("other", "web-img-unregistered:dog"),
    ]);
    assert_eq!(
        f.shown("mine").as_deref(),
        Some("https://example.test/resolved-cat.png"),
        "the browser loads what the app resolved the source to"
    );
    assert_eq!(
        f.img("mine").get_attribute(LOGICAL_SRC_ATTR).as_deref(),
        Some("web-img-a:cat")
    );
    assert_eq!(
        f.src("mine").as_deref(),
        Some("web-img-a:cat"),
        "and rinch still answers the source it was given"
    );

    // Everything else is written as given and carries no second attribute.
    for (id, src) in [
        ("plain", "pictures/elsewhere.png"),
        ("other", "web-img-unregistered:dog"),
    ] {
        assert_eq!(f.shown(id).as_deref(), Some(src), "{id}");
        assert_eq!(f.img(id).get_attribute(LOGICAL_SRC_ATTR), None, "{id}");
        assert_eq!(f.src(id).as_deref(), Some(src), "{id}");
    }
    assert_eq!(
        urls.asked.get(),
        1,
        "the resolver is asked for its own scheme only"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn not_yet_shows_nothing_and_a_reload_makes_every_element_with_that_source_ask_again() {
    let urls = Urls::default();
    urls.register("web-img-b");
    urls.put("web-img-b:here", "https://example.test/here.png");

    let f = Fixture::with_images(&[
        ("first", "web-img-b:late"),
        ("second", "web-img-b:late"),
        ("here", "web-img-b:here"),
    ]);
    for id in ["first", "second"] {
        assert_eq!(
            f.shown(id),
            None,
            "{id}: the browser is not handed a scheme it cannot load"
        );
        assert_eq!(f.src(id).as_deref(), Some("web-img-b:late"), "{id}");
    }
    let asked = urls.asked.get();

    // A reload while the answer is still "not yet" changes nothing.
    reload_image("web-img-b:late");
    assert_eq!(f.shown("first"), None);
    assert_eq!(urls.asked.get(), asked + 1);

    // The picture arrives; the app says so.
    urls.put("web-img-b:late", "https://example.test/late.png");
    reload_image("web-img-b:late");
    for id in ["first", "second"] {
        assert_eq!(
            f.shown(id).as_deref(),
            Some("https://example.test/late.png"),
            "{id}"
        );
        assert_eq!(f.src(id).as_deref(), Some("web-img-b:late"), "{id}");
    }
    assert_eq!(
        f.shown("here").as_deref(),
        Some("https://example.test/here.png"),
        "an element with another source is left alone"
    );

    // A resolved source is remembered: an element that takes it later does not
    // ask the app again.
    let asked = urls.asked.get();
    f.handle("here").set_attribute("src", "web-img-b:late");
    assert_eq!(
        f.shown("here").as_deref(),
        Some("https://example.test/late.png")
    );
    assert_eq!(urls.asked.get(), asked);
    f.teardown();
}

#[wasm_bindgen_test]
fn a_reload_swaps_a_picture_on_screen_and_a_not_yet_leaves_it() {
    let urls = Urls::default();
    urls.register("web-img-c");
    urls.put("web-img-c:pic", "https://example.test/v1.png");
    let f = Fixture::with_images(&[("pic", "web-img-c:pic")]);
    assert_eq!(
        f.shown("pic").as_deref(),
        Some("https://example.test/v1.png")
    );

    urls.put("web-img-c:pic", "https://example.test/v2.png");
    reload_image("web-img-c:pic");
    assert_eq!(
        f.shown("pic").as_deref(),
        Some("https://example.test/v2.png")
    );

    urls.held.borrow_mut().clear();
    reload_image("web-img-c:pic");
    assert_eq!(
        f.shown("pic").as_deref(),
        Some("https://example.test/v2.png"),
        "the picture it had stays until there is another"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn changing_and_removing_the_source_keeps_the_two_attributes_in_step() {
    let urls = Urls::default();
    urls.register("web-img-d");
    urls.put("web-img-d:pic", "https://example.test/d.png");
    let f = Fixture::with_images(&[("pic", "web-img-d:pic")]);
    let img = f.handle("pic");

    // To a source the app does not answer for: an ordinary `src` again.
    img.set_attribute("src", "pictures/plain.png");
    assert_eq!(f.shown("pic").as_deref(), Some("pictures/plain.png"));
    assert_eq!(f.img("pic").get_attribute(LOGICAL_SRC_ATTR), None);
    assert_eq!(f.src("pic").as_deref(), Some("pictures/plain.png"));

    // Back, then removed.
    img.set_attribute("src", "web-img-d:pic");
    assert_eq!(
        f.shown("pic").as_deref(),
        Some("https://example.test/d.png")
    );
    img.remove_attribute("src");
    assert_eq!(f.shown("pic"), None);
    assert_eq!(f.img("pic").get_attribute(LOGICAL_SRC_ATTR), None);
    assert_eq!(f.src("pic"), None);
    f.teardown();
}

#[wasm_bindgen_test]
fn an_element_rendered_before_its_scheme_was_registered_is_picked_up_by_a_reload() {
    let f = Fixture::with_images(&[("early", "web-img-e:pic")]);
    assert_eq!(f.shown("early").as_deref(), Some("web-img-e:pic"));

    let urls = Urls::default();
    urls.register("web-img-e");
    urls.put("web-img-e:pic", "https://example.test/e.png");
    reload_image("web-img-e:pic");
    assert_eq!(
        f.shown("early").as_deref(),
        Some("https://example.test/e.png")
    );
    assert_eq!(f.src("early").as_deref(), Some("web-img-e:pic"));
    f.teardown();
}

/// `rinch::image::register_image_scheme`, the call a desktop app makes, works
/// in the browser unchanged: the loader's bytes are shown through an object
/// URL rinch makes, and the browser really decodes them.
#[wasm_bindgen_test]
async fn the_desktop_loader_call_shows_its_bytes_through_an_object_url() {
    let held: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let held_in = held.clone();
    register_image_scheme("web-img-bytes", move |src: &str| {
        match held_in.lock().unwrap().clone() {
            Some(bytes) => ImageLoadResult::Loaded(bytes),
            None => ImageLoadResult::Failed(format!("{src} is not here yet")),
        }
    });

    let f = Fixture::with_images(&[("pic", "web-img-bytes:pic")]);
    assert_eq!(f.shown("pic"), None, "not yet");

    *held.lock().unwrap() = Some(RED_3X2_PNG.to_vec());
    reload_image("web-img-bytes:pic");
    let first = f.shown("pic").expect("an object URL");
    assert!(first.starts_with("blob:"), "{first}");
    assert_eq!(
        decoded(&f.img("pic")).await,
        (3, 2),
        "the browser decoded the bytes"
    );
    assert_eq!(f.src("pic").as_deref(), Some("web-img-bytes:pic"));

    // New bytes under the same source: a new object URL, and the old one is
    // revoked (fetching it now fails).
    *held.lock().unwrap() = Some(ONE_BY_ONE_PNG.to_vec());
    reload_image("web-img-bytes:pic");
    let second = f.shown("pic").expect("an object URL");
    assert_ne!(first, second);
    assert_eq!(decoded(&f.img("pic")).await, (1, 1));
    let window = web_sys::window().unwrap();
    let stale = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&first)).await;
    assert!(
        stale.is_err(),
        "the object URL over the old bytes was revoked"
    );

    f.teardown();
    assert!(unregister_image_scheme("web-img-bytes"));
}

/// The editor's `image` node: the page shows the resolved URL, and the model
/// (what is saved, and what collaboration sends) keeps the app's own source.
#[wasm_bindgen_test]
fn an_editor_image_shows_the_resolved_url_and_the_model_keeps_the_apps_source() {
    let urls = Urls::default();
    let handle = create_editor();
    assert!(handle.load_html(r#"<p>see <img src="web-img-f:pic" alt="a cat"> here</p>"#));
    let mounted = handle.clone();
    urls.register("web-img-f");
    let f = Fixture::mount(move |scope| (mounted.mount(scope), HashMap::new()));
    let img = || {
        f.host
            .query_selector("[data-pm-editor] img")
            .unwrap()
            .expect("the image element")
    };
    assert_eq!(img().get_attribute("src"), None, "not yet");

    urls.put("web-img-f:pic", "https://example.test/f.png");
    reload_image("web-img-f:pic");
    assert_eq!(
        img().get_attribute("src").as_deref(),
        Some("https://example.test/f.png")
    );
    assert_eq!(
        node_to_html(&handle.doc()),
        r#"<p>see <img alt="a cat" src="web-img-f:pic"> here</p>"#,
        "the document never sees the resolved URL"
    );

    // An edit that re-renders the paragraph keeps showing the picture.
    handle.set_selection(rinch_editor_core::Selection::cursor(
        rinch_editor_core::Pos(1),
    ));
    assert!(handle.insert_text("X"));
    assert_eq!(
        img().get_attribute("src").as_deref(),
        Some("https://example.test/f.png")
    );
    f.teardown();
}
