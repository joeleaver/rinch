//! Review fixtures for PR #1435 (app image schemes in the browser).
//!
//! `finding_*` tests FAIL at the PR head; `holds_*` / `observe_*` pass and pin
//! or measure a behaviour.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rinch::image::{ImageLoadResult, register_image_scheme, reload_image, unregister_image_scheme};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::{LOGICAL_SRC_ATTR, RootHandle, create_editor, register_image_url_scheme};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const RED_3X2_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x9d, 0x74, 0x66,
    0x1a, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x86, 0x19, 0x90, 0x39, 0x00, 0x9b, 0x7e, 0x0b, 0xf5, 0x0f, 0x5f, 0x26, 0x22, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Let the microtask that asks the app for a newly given source run.
async fn tick() {
    for _ in 0..2 {
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL))
            .await
            .unwrap();
    }
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-review-1435";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    top: Rc<RefCell<Option<NodeHandle>>>,
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
        let top = Rc::new(RefCell::new(None));
        let top_in = top.clone();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
            let (root, made) = build(scope);
            *handles_in.borrow_mut() = made;
            *top_in.borrow_mut() = Some(root.clone());
            root
        });
        Self {
            root,
            host,
            top,
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
    fn shown(&self, id: &str) -> Option<String> {
        self.img(id).get_attribute("src")
    }
    fn handle(&self, id: &str) -> NodeHandle {
        self.handles.borrow()[id].clone()
    }
    fn top(&self) -> NodeHandle {
        self.top.borrow().clone().unwrap()
    }
    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        rinch_web::__reset_image_sources();
    }
}

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

// ---------------------------------------------------------------------------
// W1. `reload_image` finds elements with `document.querySelectorAll("img")`,
//     so an `<img>` that is detached at that moment is never told.
// ---------------------------------------------------------------------------

/// `NodeHandle::remove()` is a detach (CLAUDE.md, #719): the subtree keeps its
/// identity and is shown again when re-appended (`if open { {panel} }` with a
/// captured handle). A picture that arrives while its element is out is lost:
/// the reload does not reach it, and nothing asks again when it comes back.
#[wasm_bindgen_test]
fn finding_a_reload_misses_an_img_that_is_detached_and_it_comes_back_empty() {
    let urls = Urls::default();
    urls.register("rv-w1");
    let f = Fixture::with_images(&[("pic", "rv-w1:late"), ("stays", "rv-w1:late")]);
    assert_eq!(f.shown("pic"), None, "control: not yet");

    let img = f.handle("pic");
    img.remove(); // hidden branch, captured handle

    urls.put("rv-w1:late", "https://example.test/late.png");
    reload_image("rv-w1:late");
    assert_eq!(
        f.shown("stays").as_deref(),
        Some("https://example.test/late.png"),
        "control: the reload reached the element that was in the page"
    );

    f.top().append_child(&img); // shown again
    assert_eq!(img.get_attribute("src").as_deref(), Some("rv-w1:late"));
    assert_eq!(
        f.shown("pic").as_deref(),
        Some("https://example.test/late.png"),
        "the element names a source the app has since resolved, and shows nothing"
    );
    f.teardown();
}

/// The same hole with rinch-owned object URLs: the detached element keeps
/// pointing at the object URL the reload REVOKED.
#[wasm_bindgen_test]
fn finding_a_detached_img_is_left_pointing_at_a_revoked_object_url() {
    let held: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(RED_3X2_PNG.to_vec()));
    let held_in = held.clone();
    register_image_scheme("rv-w1b", move |_src: &str| {
        ImageLoadResult::Loaded(held_in.lock().unwrap().clone())
    });
    let f = Fixture::with_images(&[("pic", "rv-w1b:pic"), ("stays", "rv-w1b:pic")]);
    let first = f.shown("pic").expect("control: an object URL");
    let img = f.handle("pic");
    img.remove();
    reload_image("rv-w1b:pic"); // new bytes: new URL, old one revoked
    let second = f.shown("stays").expect("an object URL");
    assert_ne!(first, second, "control: the reload made a new URL");
    f.top().append_child(&img);
    let after = f.shown("pic");
    f.teardown();
    unregister_image_scheme("rv-w1b");
    assert_eq!(
        after.as_deref(),
        Some(second.as_str()),
        "re-attached, the element still points at the revoked URL {first}"
    );
}

// ---------------------------------------------------------------------------
// W2. An app-owned URL can never be forgotten: "its URLs are the app's to
//     revoke", but rinch keeps handing the remembered one out.
// ---------------------------------------------------------------------------

/// The PR body: "An app with many large pictures should make and revoke its
/// own URLs through `register_image_url_scheme`". The app evicts a blob,
/// revokes its object URL, and tells rinch with the only call there is. rinch
/// asks, hears "not yet" — and still gives the dead URL to the next element
/// without asking.
#[wasm_bindgen_test]
async fn finding_an_app_cannot_make_rinch_forget_a_url_it_revoked() {
    let urls = Urls::default();
    urls.register("rv-w2");
    urls.put("rv-w2:pic", "blob:https://example.test/evicted-later");
    let f = Fixture::with_images(&[("a", "rv-w2:pic"), ("b", "pictures/plain.png")]);
    assert_eq!(
        f.shown("a").as_deref(),
        Some("blob:https://example.test/evicted-later")
    );

    // The app evicts the blob and revokes its URL; the element showing it is
    // scrolled away / removed.
    f.handle("a").set_attribute("src", "pictures/plain.png");
    urls.held.borrow_mut().clear();
    reload_image("rv-w2:pic");

    // A new element names the source.
    let asked = urls.asked.get();
    f.handle("b").set_attribute("src", "rv-w2:pic");
    tick().await;
    assert_eq!(
        (urls.asked.get() - asked, f.shown("b")),
        (1, None),
        "the app was not asked, and the browser was handed the URL it revoked"
    );
    f.teardown();
}

// ---------------------------------------------------------------------------
// W4. Markup that does not go through `set_attribute` is not resolved.
// ---------------------------------------------------------------------------

/// `NodeHandle::set_inner_html` / `Element::Html`: on desktop every parsed
/// `<img>` is loaded through the scheme loader like any other. On the web the
/// markup goes to `innerHTML` untouched, so the browser is handed the app's
/// scheme (a failed fetch and a console error per image) and the resolver is
/// never asked.
#[wasm_bindgen_test]
async fn finding_an_img_made_by_set_inner_html_is_not_resolved() {
    let urls = Urls::default();
    urls.register("rv-w4");
    urls.put("rv-w4:pic", "https://example.test/w4.png");
    let f = Fixture::with_images(&[]);
    f.top()
        .set_inner_html(r#"<p><img id="parsed" src="rv-w4:pic"></p>"#);
    tick().await;
    let shown = f.shown("parsed");
    let asked = urls.asked.get();
    f.teardown();
    assert_eq!(
        (asked, shown.as_deref()),
        (1, Some("https://example.test/w4.png")),
    );
}

// ---------------------------------------------------------------------------
// W5. A reload re-points `<img>` elements rinch does not own.
// ---------------------------------------------------------------------------

/// `rinch_web::mount_into` is for an island in somebody else's page. The
/// reload walks the whole document, so a page `<img>` outside every rinch root
/// whose `src` equals the reloaded source is rewritten and stamped.
#[wasm_bindgen_test]
fn finding_a_reload_rewrites_an_img_outside_every_rinch_root() {
    let foreign = document().create_element("img").unwrap();
    foreign.set_attribute("id", "foreign").unwrap();
    foreign.set_attribute("src", "rv-w5:pic").unwrap();
    document().body().unwrap().append_child(&foreign).unwrap();

    let urls = Urls::default();
    urls.register("rv-w5");
    urls.put("rv-w5:pic", "https://example.test/w5.png");
    let f = Fixture::with_images(&[("mine", "rv-w5:pic")]);
    reload_image("rv-w5:pic");
    let src = foreign.get_attribute("src");
    let stamped = foreign.get_attribute(LOGICAL_SRC_ATTR);
    foreign.remove();
    f.teardown();
    assert_eq!(
        (src.as_deref(), stamped),
        (Some("rv-w5:pic"), None),
        "the page's own element is not rinch's to rewrite"
    );
}

// ---------------------------------------------------------------------------
// W8. A reload of a source no element names loads it, wraps it and keeps it.
// ---------------------------------------------------------------------------

/// Desktop (`reloading_a_source_nothing_asked_for_starts_nothing`): "a source
/// no element has asked for is left alone". An app's sync thread naturally
/// calls `reload_image` for every blob that arrives, shown or not. On the web
/// each such call runs the loader, copies the bytes into a `Blob`, makes an
/// object URL and remembers it — for the life of the page.
#[wasm_bindgen_test]
fn finding_a_reload_of_a_source_nothing_names_loads_and_retains_it() {
    let asked = Arc::new(Mutex::new(0u32));
    let asked_in = asked.clone();
    register_image_scheme("rv-w8", move |_src: &str| {
        *asked_in.lock().unwrap() += 1;
        ImageLoadResult::Loaded(RED_3X2_PNG.to_vec())
    });
    let f = Fixture::with_images(&[("other", "pictures/plain.png")]);
    for i in 0..50 {
        reload_image(&format!("rv-w8:blob-{i}"));
    }
    let n = *asked.lock().unwrap();
    f.teardown();
    unregister_image_scheme("rv-w8");
    assert_eq!(
        n, 0,
        "50 reloads of sources nothing shows ran the loader {n} times"
    );
}

// ---------------------------------------------------------------------------
// Observations (pass; they measure).
// ---------------------------------------------------------------------------

/// "No negative caching: a resolver that answers `None` is called once per
/// element that is given the source." How often is an editor image "given"
/// its source while the user types in the same paragraph?
#[wasm_bindgen_test]
fn observe_resolver_calls_for_a_not_yet_editor_image_while_typing() {
    let urls = Urls::default();
    let handle = create_editor();
    assert!(handle.load_html(r#"<p>see <img src="rv-w7:pic" alt="a cat"> here</p>"#));
    let mounted = handle.clone();
    urls.register("rv-w7");
    let f = Fixture::mount(move |scope| (mounted.mount(scope), HashMap::new()));
    let after_mount = urls.asked.get();
    handle.set_selection(rinch_editor_core::Selection::cursor(
        rinch_editor_core::Pos(1),
    ));
    for _ in 0..10 {
        assert!(handle.insert_text("X"));
    }
    let after_typing = urls.asked.get();
    f.teardown();
    assert_eq!(
        (after_mount, after_typing - after_mount),
        (1, 0),
        "(asked at mount, asked again over 10 keystrokes in the image's paragraph)"
    );
}

/// An object URL rinch made outlives `unregister_image_scheme` and the unmount
/// of the only root that showed it (the PR's stated limit: "no eviction").
#[wasm_bindgen_test]
async fn observe_an_owned_object_url_outlives_unregister_and_unmount() {
    register_image_scheme("rv-w6", move |_src: &str| {
        ImageLoadResult::Loaded(RED_3X2_PNG.to_vec())
    });
    let f = Fixture::with_images(&[("pic", "rv-w6:pic")]);
    let url = f.shown("pic").expect("an object URL");
    f.root.unmount();
    f.host.remove();
    assert!(unregister_image_scheme("rv-w6"));
    reload_image("rv-w6:pic");
    let window = web_sys::window().unwrap();
    let alive = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&url))
        .await
        .is_ok();
    rinch_web::__reset_image_sources();
    assert!(
        alive,
        "(observation) the bytes are still held after unmount + unregister + reload"
    );
}

// ---------------------------------------------------------------------------
// Pins added in the review round.
// ---------------------------------------------------------------------------

/// A resolved source is remembered from its FIRST answer: an element given it
/// later is pointed at the remembered URL at once, and the app is not asked
/// again. (The review's mutant W3 — "resolve does not remember a first
/// answer" — survived the PR's tests, which checked memory only after a
/// reload.)
#[wasm_bindgen_test]
async fn holds_a_first_answer_is_remembered_for_the_next_element() {
    let urls = Urls::default();
    urls.register("rv-first");
    urls.put("rv-first:pic", "https://example.test/first.png");
    let f = Fixture::with_images(&[("a", "rv-first:pic"), ("b", "pictures/plain.png")]);
    assert_eq!(f.shown("a").as_deref(), Some("https://example.test/first.png"));
    assert_eq!(urls.asked.get(), 1);
    f.handle("b").set_attribute("src", "rv-first:pic");
    assert_eq!(
        f.shown("b").as_deref(),
        Some("https://example.test/first.png"),
        "pointed at the remembered answer at once"
    );
    tick().await;
    assert_eq!(urls.asked.get(), 1, "and the app was not asked again");
    f.teardown();
}

/// With a URL resolver and a bytes loader both registered for one scheme, the
/// resolver answers (the documented precedence; mutant W7 survived).
#[wasm_bindgen_test]
fn holds_a_url_resolver_wins_over_a_loader_for_the_same_scheme() {
    let loaded = Arc::new(Mutex::new(0u32));
    let loaded_in = loaded.clone();
    register_image_scheme("rv-both", move |_src: &str| {
        *loaded_in.lock().unwrap() += 1;
        ImageLoadResult::Loaded(RED_3X2_PNG.to_vec())
    });
    let urls = Urls::default();
    urls.register("rv-both");
    urls.put("rv-both:pic", "https://example.test/both.png");
    let f = Fixture::with_images(&[("pic", "rv-both:pic")]);
    let shown = f.shown("pic");
    let n = *loaded.lock().unwrap();
    f.teardown();
    unregister_image_scheme("rv-both");
    assert_eq!(shown.as_deref(), Some("https://example.test/both.png"));
    assert_eq!(n, 0, "the bytes loader was not asked");
}

/// Sources given to elements while mounting are asked for once each, before
/// `mount_into` returns, however many elements name them.
#[wasm_bindgen_test]
fn mounting_asks_once_per_source() {
    let urls = Urls::default();
    urls.register("rv-mount");
    let f = Fixture::with_images(&[
        ("a", "rv-mount:x"),
        ("b", "rv-mount:x"),
        ("c", "rv-mount:x"),
        ("d", "rv-mount:y"),
    ]);
    let asked = urls.asked.get();
    f.teardown();
    assert_eq!(asked, 2);
}
