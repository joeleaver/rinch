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
//! then shows nothing, and [`rinch::image::reload_image`] makes every `<img>`
//! naming that source ask again: the same call as on desktop.
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

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::image::{ImageLoadResult, image_scheme, image_scheme_loader};
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
    /// Answers so far, by source. Only successes: "not yet" is asked again by
    /// the next element that names the source, and by a reload.
    static RESOLVED: RefCell<HashMap<String, Resolved>> = RefCell::new(HashMap::new());
}

/// Answer for every `<img>` source whose URL scheme is `scheme` with a **URL
/// the browser can load**, typically an object URL the app made from bytes it
/// holds. The browser-only sibling of
/// [`rinch::image::register_image_scheme`](rinch_core::image::register_image_scheme),
/// which works here too and takes the bytes instead; this one wins when both
/// are registered for a scheme.
///
/// `resolver` is called on the one browser thread, synchronously, when an
/// `<img>` is given such a `src`, and may capture `Rc` state. `Some(url)` is
/// shown and remembered for that source (the resolver is not asked again until
/// a reload). `None` means "not yet": the element shows nothing, and
/// [`rinch::image::reload_image(src)`](rinch_core::image::reload_image) makes
/// every `<img>` with that source ask again, which is also how a picture whose
/// URL changed is refreshed. A URL the resolver answers with is the app's to
/// revoke; rinch never does.
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
/// If `scheme` is not a URL scheme of two or more characters.
pub fn register_image_url_scheme(
    scheme: &str,
    resolver: impl Fn(&str) -> Option<String> + 'static,
) {
    let key = image_scheme(&format!("{scheme}:"))
        .unwrap_or_else(|| panic!("`{scheme}` is not a URL scheme an image source can carry"));
    URL_RESOLVERS.with(|r| r.borrow_mut().insert(key, Rc::new(resolver)));
}

/// Remove the resolver [`register_image_url_scheme`] installed for `scheme`.
/// Returns whether there was one. Elements already showing a resolved picture
/// keep it.
pub fn unregister_image_url_scheme(scheme: &str) -> bool {
    let key = scheme.to_ascii_lowercase();
    URL_RESOLVERS.with(|r| r.borrow_mut().remove(&key).is_some())
}

/// Whether the app answers for `src`: its scheme has a URL resolver or a
/// loader registered.
pub(crate) fn is_app_source(src: &str) -> bool {
    let Some(scheme) = image_scheme(src) else {
        return false;
    };
    URL_RESOLVERS.with(|r| r.borrow().contains_key(&scheme)) || image_scheme_loader(src).is_some()
}

/// The URL the browser should load for `src`, asking the app when no answer is
/// remembered. `None` is "not yet".
fn resolve(src: &str) -> Option<String> {
    if let Some(url) = RESOLVED.with(|r| r.borrow().get(src).map(|a| a.url.clone())) {
        return Some(url);
    }
    let answer = ask(src)?;
    let url = answer.url.clone();
    RESOLVED.with(|r| r.borrow_mut().insert(src.to_string(), answer));
    Some(url)
}

/// Ask the app for `src`, with no borrow held (a resolver may call back into
/// this module, to register or to reload).
fn ask(src: &str) -> Option<Resolved> {
    let scheme = image_scheme(src)?;
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

/// An object URL over `bytes`, typed by what they are so the browser renders
/// them (an SVG is not sniffed by an `<img>`; the raster formats are).
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

/// Write `value` as the `src` of `img`: an app-resolved source keeps `value`
/// in [`LOGICAL_SRC_ATTR`] and gives the browser the resolved URL (or no `src`
/// while the answer is "not yet"); any other source is written as given.
pub(crate) fn set_img_src(img: &web_sys::Element, name: &str, value: &str) {
    if !is_app_source(value) {
        img.remove_attribute(LOGICAL_SRC_ATTR).ok();
        img.set_attribute(name, value).ok();
        return;
    }
    img.set_attribute(LOGICAL_SRC_ATTR, value).ok();
    point(img, resolve(value).as_deref());
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

/// The browser half of [`rinch_core::image::reload_image`]: forget the answer
/// for `src`, ask the app again, and re-point every `<img>` on the page that
/// names it. An object URL rinch made for the old bytes is revoked once no
/// element points at it. A "not yet" leaves a picture that is on screen where
/// it is.
pub(crate) fn reload(src: &str) {
    if !is_app_source(src) {
        return;
    }
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let fresh = ask(src);
    let Ok(images) = document.query_selector_all("img") else {
        return;
    };
    let named: Vec<web_sys::Element> = (0..images.length())
        .filter_map(|i| images.item(i)?.dyn_into::<web_sys::Element>().ok())
        .filter(|img| match img.get_attribute(LOGICAL_SRC_ATTR) {
            Some(logical) => logical == src,
            // Rendered before the scheme was registered: the browser was
            // handed the app's own URL.
            None => img.get_attribute("src").as_deref() == Some(src),
        })
        .collect();
    let Some(fresh) = fresh else {
        // Still not there. An element with nothing to show keeps nothing; one
        // showing the old picture keeps it (and its remembered answer).
        for img in &named {
            if img.get_attribute(LOGICAL_SRC_ATTR).is_none() {
                img.set_attribute(LOGICAL_SRC_ATTR, src).ok();
                img.remove_attribute("src").ok();
            }
        }
        return;
    };
    for img in &named {
        img.set_attribute(LOGICAL_SRC_ATTR, src).ok();
        point(img, Some(&fresh.url));
    }
    let url = fresh.url.clone();
    let old = RESOLVED.with(|r| r.borrow_mut().insert(src.to_string(), fresh));
    if let Some(old) = old
        && old.owned
        && old.url != url
    {
        web_sys::Url::revoke_object_url(&old.url).ok();
    }
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

/// Forget every remembered answer and browser-only resolver, revoking the
/// object URLs rinch made. For tests, which share one page.
#[doc(hidden)]
pub fn __reset_image_sources() {
    URL_RESOLVERS.with(|r| r.borrow_mut().clear());
    let answers: Vec<Resolved> =
        RESOLVED.with(|r| r.borrow_mut().drain().map(|(_, a)| a).collect());
    for answer in answers.into_iter().filter(|a| a.owned) {
        web_sys::Url::revoke_object_url(&answer.url).ok();
    }
}
