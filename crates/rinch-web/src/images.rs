//! App-resolved image sources in the browser.
//!
//! The browser loads an `<img>` itself, so nothing here loads or decodes a
//! picture. What an app needs instead is for an `<img>` whose `src` only the
//! app understands (`myapp-blob:1234`) to *show* something the browser can
//! load: an object URL (`blob:…`) over bytes the app holds. This module is that
//! mapping.
//!
//! # The two ways to answer
//!
//! - [`rinch::image::register_image_scheme`]: the **same call as on desktop**.
//!   The loader answers with the picture's encoded bytes; this backend wraps
//!   them in a `Blob`, makes the object URL, and revokes it when the source is
//!   reloaded with new bytes.
//! - [`register_image_url_scheme`], browser only: the resolver answers with a
//!   **URL** the app made itself (it already has a `Blob`, or a signed link),
//!   and may capture `Rc` state, which a desktop loader cannot.
//!
//! Either may answer "not yet" (`ImageLoadResult::Failed`, `None`). The element
//! then shows nothing, and [`rinch::image::reload_image`] asks again for every
//! `<img>` rinch was given that source: the same call as on desktop.
//!
//! # When the app is asked
//!
//! Never inside a DOM write. `WebDocument::set_attribute` runs while
//! `NodeHandle` holds the document's `RefCell`, and an app's resolver may write
//! a signal whose effect touches the DOM, so an element given a source with no
//! remembered answer shows nothing until a microtask asks (once per source),
//! and a mount asks for what it was given before `mount_into` returns.
//! `reload_image` asks at once: it is called from app code, not from a write.
//!
//! # What the element carries
//!
//! The source rinch was given stays the element's identity: it is kept in
//! [`LOGICAL_SRC_ATTR`], and [`DomDocument::get_attribute`]`("src")` answers it.
//! Only the browser's own `src` attribute holds the resolved URL (or is absent
//! while the answer is "not yet", so the browser never tries to fetch a scheme
//! it does not know). The rich-text editor's document is not derived from the
//! DOM at all, so an `image` node's `src`, and what collaboration sends, is the
//! app's own URL throughout.
//!
//! Only `<img src>` is resolved. A `background-image: url(…)` is the browser's
//! from the stylesheet down and never passes through here.
//!
//! [`rinch::image::register_image_scheme`]: rinch_core::image::register_image_scheme
//! [`rinch::image::reload_image`]: rinch_core::image::reload_image
//! [`DomDocument::get_attribute`]: rinch_core::dom::DomDocument::get_attribute

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rinch_core::image::{ImageLoadResult, image_scheme_loader, scheme_of};
use wasm_bindgen::JsCast;

/// The attribute an `<img>` keeps the source rinch was given in, while its
/// browser `src` holds what the app resolved that source to.
pub const LOGICAL_SRC_ATTR: &str = "data-rinch-src";

/// A browser-only resolver: a source in, a URL the browser can load out, or
/// `None` for "not yet".
type UrlResolver = Rc<dyn Fn(&str) -> Option<String>>;

/// What a source resolved to.
struct Resolved {
    url: String,
    /// rinch made this object URL (from a loader's bytes) and revokes it when
    /// the source is reloaded. A URL a resolver answered with is the app's.
    owned: bool,
}

thread_local! {
    static URL_RESOLVERS: RefCell<HashMap<String, UrlResolver>> = RefCell::new(HashMap::new());
    /// Answers so far, by source.
    static RESOLVED: RefCell<HashMap<String, Resolved>> = RefCell::new(HashMap::new());
    /// Sources the app last answered "not yet" for. An element given one shows
    /// nothing and asks nothing; only `reload_image` asks again, as on desktop,
    /// where a miss is cached until a reload. Without it a resolver whose
    /// signal write re-creates its `<img>` was asked again from a fresh
    /// microtask for ever, and the page never got back to its event loop.
    static NOT_YET: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// The `<img>` elements rinch was given each source for, attached to the
    /// page or not: what a reload re-points. An element leaves its list when
    /// its `src` changes or is removed and when its document forgets it
    /// (`forget_element`), so a page `<img>` outside every rinch root is never
    /// here.
    static ELEMENTS: RefCell<HashMap<String, Vec<web_sys::Element>>> =
        RefCell::new(HashMap::new());
    /// How many elements `ELEMENTS` holds, so the node-forgetting walk pays
    /// one `Cell` read while an app has no such source.
    static ELEMENT_COUNT: Cell<usize> = const { Cell::new(0) };
    /// Sources given to an element with no remembered answer, waiting to be
    /// asked for (`resolve_pending`) outside the document's borrow.
    static PENDING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PENDING_SCHEDULED: Cell<bool> = const { Cell::new(false) };
}

/// Schemes the browser loads itself. A source with any other scheme is
/// recorded against its element even before the app registers it, so a reload
/// after a late registration finds the element.
fn browser_scheme(scheme: &str) -> bool {
    matches!(
        scheme,
        "http" | "https" | "data" | "blob" | "file" | "about" | "javascript" | "ftp" | "filesystem"
    )
}

/// The key a scheme an app names is registered under (no colon, any case), or
/// `None` for a name that is not a scheme, has a colon in it, or is `data`.
fn registrable_scheme(scheme: &str) -> Option<String> {
    scheme_of(&format!("{scheme}:")).filter(|key| key.len() == scheme.len() && key != "data")
}

/// Answer for every `<img>` source whose URL scheme is `scheme` with a **URL
/// the browser can load**, typically an object URL the app made from bytes it
/// holds. The browser-only sibling of
/// [`rinch::image::register_image_scheme`](rinch_core::image::register_image_scheme),
/// which works here too and takes the bytes instead; this one wins when both
/// are registered for a scheme.
///
/// `resolver` is called on the one browser thread and may capture `Rc` state.
/// It is **never called inside a DOM write**: an element given such a `src`
/// shows a remembered answer at once, and otherwise shows nothing until a
/// microtask asks the resolver (before the next paint), once per source
/// however many elements were given it. So a resolver may write signals and
/// touch rinch's DOM. Sources given while a root is mounting are asked for
/// before `mount_into` returns. `Some(url)` is shown and remembered for that
/// source (the resolver is not asked again until a reload). `None` means "not
/// yet": the element shows nothing, the answer is remembered too (an element
/// given the source later shows nothing and asks nothing, as a desktop loader's
/// miss is cached), and
/// [`rinch::image::reload_image(src)`](rinch_core::image::reload_image) asks
/// again for every `<img>` rinch was given that source, attached to the page
/// or not, which is also how a picture whose URL changed is refreshed.
///
/// A URL the resolver answers with is the app's to revoke; rinch never does.
/// To make rinch forget one (the app evicted the blob), call `reload_image`
/// and answer `None`: elements already showing the picture keep it (a browser
/// keeps a loaded image), and an element given the source later is handed
/// nothing until the next reload. A reload of a source no element was given
/// forgets its answer, "not yet" included, without asking.
///
/// A resolver or loader that **panics** aborts the whole app: wasm has no
/// unwinding, so nothing catches it here as desktop does.
///
/// ```ignore
/// rinch_web::register_image_url_scheme("myapp-blob", move |src| {
///     match blobs.borrow().object_url(src) {
///         Some(url) => Some(url),
///         None => {
///             // fetch, decrypt, `Url::create_object_url_with_blob`, then:
///             //     rinch::image::reload_image(src)
///             blobs.borrow_mut().request(src);
///             None
///         }
///     }
/// });
/// ```
///
/// Registering again replaces the earlier resolver. The scheme is matched
/// case-insensitively and given without the colon.
///
/// # Panics
///
/// If `scheme` is not a URL scheme of two or more characters (a name with a
/// `:` in it is not one), or is `data`.
pub fn register_image_url_scheme(
    scheme: &str,
    resolver: impl Fn(&str) -> Option<String> + 'static,
) {
    let key = registrable_scheme(scheme)
        .unwrap_or_else(|| panic!("`{scheme}` is not a URL scheme an app can load images for"));
    URL_RESOLVERS.with(|r| r.borrow_mut().insert(key, Rc::new(resolver)));
}

/// Remove the resolver [`register_image_url_scheme`] installed for `scheme`
/// (named as it was registered). Returns whether there was one. Elements
/// already showing a resolved picture keep it.
pub fn unregister_image_url_scheme(scheme: &str) -> bool {
    let Some(key) = registrable_scheme(scheme) else {
        return false;
    };
    URL_RESOLVERS.with(|r| r.borrow_mut().remove(&key).is_some())
}

/// Whether the app answers for `src`: its scheme has a URL resolver or a
/// loader registered.
pub(crate) fn is_app_source(src: &str) -> bool {
    let Some(scheme) = scheme_of(src) else {
        return false;
    };
    URL_RESOLVERS.with(|r| r.borrow().contains_key(&scheme)) || image_scheme_loader(src).is_some()
}

/// Whether an `<img>` given `src` is recorded for reloads: an app source, or
/// one whose scheme the app may register later.
fn recorded_source(src: &str) -> bool {
    scheme_of(src).is_some_and(|s| !browser_scheme(&s))
}

/// The remembered answer for `src`, if any.
fn remembered(src: &str) -> Option<String> {
    RESOLVED.with(|r| r.borrow().get(src).map(|a| a.url.clone()))
}

/// Ask the app for `src`. Called with no borrow of this module's state held,
/// and never from inside a `WebDocument` write (a resolver may write signals,
/// whose effects touch the DOM).
fn ask(src: &str) -> Option<Resolved> {
    let scheme = scheme_of(src)?;
    if let Some(resolver) = URL_RESOLVERS.with(|r| r.borrow().get(&scheme).cloned()) {
        return resolver(src).map(|url| Resolved { url, owned: false });
    }
    match image_scheme_loader(src)?.load(src) {
        ImageLoadResult::Loaded(bytes) => {
            object_url(&bytes).map(|url| Resolved { url, owned: true })
        }
        ImageLoadResult::Failed(_) => None,
    }
}

/// Remember `fresh` for `src`, revoking an object URL rinch made for an older
/// answer that it replaces.
fn remember(src: &str, fresh: Resolved) {
    let url = fresh.url.clone();
    let old = RESOLVED.with(|r| r.borrow_mut().insert(src.to_string(), fresh));
    if let Some(old) = old
        && old.owned
        && old.url != url
    {
        web_sys::Url::revoke_object_url(&old.url).ok();
    }
}

/// Forget the answer for `src`, revoking it when rinch made it.
fn forget_answer(src: &str) {
    NOT_YET.with(|n| n.borrow_mut().remove(src));
    if let Some(old) = RESOLVED.with(|r| r.borrow_mut().remove(src))
        && old.owned
    {
        web_sys::Url::revoke_object_url(&old.url).ok();
    }
}

/// The source `img` was last recorded under.
fn recorded_as(img: &web_sys::Element) -> Option<String> {
    img.get_attribute(LOGICAL_SRC_ATTR)
        .or_else(|| img.get_attribute("src"))
}

fn record(src: &str, img: &web_sys::Element) {
    ELEMENTS.with(|e| {
        let mut e = e.borrow_mut();
        let list = e.entry(src.to_string()).or_default();
        if !list.iter().any(|x| x == img) {
            list.push(img.clone());
            ELEMENT_COUNT.set(ELEMENT_COUNT.get() + 1);
        }
    });
}

fn unrecord(src: &str, img: &web_sys::Element) {
    ELEMENTS.with(|e| {
        let mut e = e.borrow_mut();
        if let Some(list) = e.get_mut(src) {
            let before = list.len();
            list.retain(|x| x != img);
            ELEMENT_COUNT.set(ELEMENT_COUNT.get() - (before - list.len()));
            if list.is_empty() {
                e.remove(src);
            }
        }
    });
}

/// The elements rinch was given `src` for that still name it.
fn elements_naming(src: &str) -> Vec<web_sys::Element> {
    ELEMENTS
        .with(|e| e.borrow().get(src).cloned().unwrap_or_default())
        .into_iter()
        .filter(|img| recorded_as(img).as_deref() == Some(src))
        .collect()
}

/// Take `img` out of the reload record: its document has forgotten it (a
/// discard, `set_inner_html`, the document's drop). Free while no element is
/// recorded.
pub(crate) fn forget_node(node: &web_sys::Node) {
    if ELEMENT_COUNT.get() == 0 {
        return;
    }
    let Some(img) = node.dyn_ref::<web_sys::Element>() else {
        return;
    };
    if !img.tag_name().eq_ignore_ascii_case("img") {
        return;
    }
    if let Some(src) = recorded_as(img) {
        unrecord(&src, img);
    }
}

/// Write `value` as the `src` of `img`: an app-resolved source keeps `value`
/// in [`LOGICAL_SRC_ATTR`] and gives the browser the remembered URL, or no
/// `src` until [`resolve_pending`] has asked the app; any other source is
/// written as given. Called from inside the document's borrow, so it never
/// calls app code.
pub(crate) fn set_img_src(img: &web_sys::Element, name: &str, value: &str) {
    if let Some(prev) = recorded_as(img) {
        unrecord(&prev, img);
    }
    if !is_app_source(value) {
        img.remove_attribute(LOGICAL_SRC_ATTR).ok();
        img.set_attribute(name, value).ok();
        if recorded_source(value) {
            record(value, img);
        }
        return;
    }
    img.set_attribute(LOGICAL_SRC_ATTR, value).ok();
    record(value, img);
    match remembered(value) {
        Some(url) => point(img, Some(&url)),
        None if NOT_YET.with(|n| n.borrow().contains(value)) => point(img, None),
        None => {
            point(img, None);
            queue(value);
        }
    }
}

/// The `src` of `img` is being removed: it leaves the reload record.
pub(crate) fn remove_img_src(img: &web_sys::Element) {
    if let Some(prev) = recorded_as(img) {
        unrecord(&prev, img);
    }
    img.remove_attribute(LOGICAL_SRC_ATTR).ok();
}

/// Route every `<img src>` under `root` through [`set_img_src`]: markup that
/// reached the page without `set_attribute` (`set_inner_html`).
pub(crate) fn adopt_parsed_images(root: &web_sys::Element) {
    let Ok(images) = root.query_selector_all("img[src]") else {
        return;
    };
    for i in 0..images.length() {
        let Some(img) = images
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        if let Some(src) = img.get_attribute("src")
            && recorded_source(&src)
        {
            set_img_src(&img, "src", &src);
        }
    }
}

/// Ask for `src` from a microtask, once however many elements are given it.
fn queue(src: &str) {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        if !p.iter().any(|s| s == src) {
            p.push(src.to_string());
        }
    });
    if PENDING_SCHEDULED.replace(true) {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let run = wasm_bindgen::closure::Closure::once_into_js(resolve_pending);
    window.queue_microtask(run.unchecked_ref());
}

/// Ask the app for every source given to an element since the last time, and
/// point the elements that still name it. Runs from a microtask and at the end
/// of a mount, outside every borrow.
pub(crate) fn resolve_pending() {
    PENDING_SCHEDULED.set(false);
    let sources: Vec<String> = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for src in sources {
        if let Some(url) = remembered(&src) {
            for img in elements_naming(&src) {
                point(&img, Some(&url));
            }
            continue;
        }
        if elements_naming(&src).is_empty() || NOT_YET.with(|n| n.borrow().contains(&src)) {
            continue;
        }
        let Some(fresh) = ask(&src) else {
            NOT_YET.with(|n| n.borrow_mut().insert(src));
            continue;
        };
        let url = fresh.url.clone();
        remember(&src, fresh);
        for img in elements_naming(&src) {
            point(&img, Some(&url));
        }
    }
}

fn object_url(bytes: &[u8]) -> Option<String> {
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes).buffer());
    let options = web_sys::BlobPropertyBag::new();
    if let Some(mime) = sniff_mime(bytes) {
        options.set_type(mime);
    }
    let blob = web_sys::Blob::new_with_buffer_source_sequence_and_options(&parts, &options).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}
fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        let head = &bytes[..bytes.len().min(256)];
        let head = String::from_utf8_lossy(head);
        let head = head.trim_start_matches('\u{feff}').trim_start();
        (head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg")))
            .then_some("image/svg+xml")
    }
}

/// Give the browser `url` to load for `img`, or nothing.
fn point(img: &web_sys::Element, url: Option<&str>) {
    match url {
        Some(url) => {
            if img.get_attribute("src").as_deref() != Some(url) {
                img.set_attribute("src", url).ok();
            }
        }
        None => {
            img.remove_attribute("src").ok();
        }
    }
}

/// The source rinch was given for `el`'s `src`, when what the browser holds is
/// a resolved URL (or nothing yet).
pub(crate) fn logical_src(el: &web_sys::Element) -> Option<String> {
    el.get_attribute(LOGICAL_SRC_ATTR)
}

/// The browser half of [`rinch_core::image::reload_image`]: ask the app again
/// for `src` and re-point every `<img>` rinch was given it, attached to the
/// page or not (a hidden branch's captured element is shown again with the
/// new picture). Elements outside every rinch root are not touched.
///
/// - No element was given `src`: nothing is asked, and a remembered answer is
///   forgotten (an object URL rinch made for it is revoked).
/// - The answer is "not yet": an element showing a picture keeps it. "Not
///   yet" is remembered (only the next reload asks again), and a URL the app
///   answered with before is forgotten; one rinch made is kept while elements
///   show it.
/// - A new answer: every element points at it, and an object URL rinch made
///   for the old one is revoked.
pub(crate) fn reload(src: &str) {
    if !is_app_source(src) {
        return;
    }
    // Anything still waiting for its first answer is asked for first, so this
    // reload is the last word.
    resolve_pending();
    if elements_naming(src).is_empty() {
        forget_answer(src);
        return;
    }
    let fresh = ask(src);
    let named = elements_naming(src);
    let Some(fresh) = fresh else {
        NOT_YET.with(|n| n.borrow_mut().insert(src.to_string()));
        let app_owned = RESOLVED.with(|r| r.borrow().get(src).is_some_and(|a| !a.owned));
        if app_owned {
            RESOLVED.with(|r| r.borrow_mut().remove(src));
        }
        // An element with nothing to show keeps nothing.
        for img in &named {
            if img.get_attribute(LOGICAL_SRC_ATTR).is_none() {
                img.set_attribute(LOGICAL_SRC_ATTR, src).ok();
                img.remove_attribute("src").ok();
            }
        }
        return;
    };
    NOT_YET.with(|n| n.borrow_mut().remove(src));
    let url = fresh.url.clone();
    for img in &named {
        img.set_attribute(LOGICAL_SRC_ATTR, src).ok();
        point(img, Some(&url));
    }
    remember(src, fresh);
}

/// Make this backend answer `rinch::image::reload_image`. Idempotent.
pub(crate) fn install_reloader() {
    thread_local! {
        static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if !INSTALLED.replace(true) {
        rinch_core::image::add_image_reloader(reload);
    }
}

/// Forget every remembered answer, recorded element, waiting source and
/// browser-only resolver, revoking the object URLs rinch made. For tests,
/// which share one page.
#[doc(hidden)]
pub fn __reset_image_sources() {
    URL_RESOLVERS.with(|r| r.borrow_mut().clear());
    ELEMENTS.with(|e| e.borrow_mut().clear());
    ELEMENT_COUNT.set(0);
    PENDING.with(|p| p.borrow_mut().clear());
    NOT_YET.with(|n| n.borrow_mut().clear());
    let answers: Vec<Resolved> =
        RESOLVED.with(|r| r.borrow_mut().drain().map(|(_, a)| a).collect());
    for answer in answers.into_iter().filter(|a| a.owned) {
        web_sys::Url::revoke_object_url(&answer.url).ok();
    }
}
