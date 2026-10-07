//! Image loading abstraction for rinch.
//!
//! Defines the [`ImageLoader`] trait which backends use to load images
//! from various sources (local files, network, etc.), and the registry of
//! **app-installed URL schemes** ([`register_image_scheme`]) that lets an app
//! answer for sources only it can resolve (`myapp-blob:…`) while every other
//! source keeps going to the document's own loader.

use std::sync::{Arc, RwLock};

/// Result of an image load operation.
pub enum ImageLoadResult {
    /// Successfully loaded raw **encoded** bytes: the contents of a PNG, JPEG,
    /// GIF or WebP file, exactly as they would sit on disk. rinch decodes them
    /// (on the same background thread that called the loader); a loader never
    /// hands over decoded pixels.
    Loaded(Vec<u8>),
    /// Loading failed with an error message.
    ///
    /// The failure is remembered for that source: the document does not ask
    /// again on its own, however many elements name it. A source that may
    /// answer later (a file still downloading, a blob that has not arrived) is
    /// asked again by calling `reload_image` (`rinch::image::reload_image`,
    /// `rinch_dom::image_cache::reload_image`) once it can.
    Failed(String),
}

/// Trait for loading image data from a source string (file path or URL).
///
/// # What a loader returns
///
/// The **encoded** bytes of the image ([`ImageLoadResult::Loaded`]), or a
/// failure. Decoding is rinch's job.
///
/// # Where it runs
///
/// On a background thread spawned for that one load ([`std::thread::spawn`]),
/// never on the UI thread, so blocking I/O is acceptable: a loader may read a
/// file, make a request, or wait on a channel. It is called once per source
/// per document (results are cached by source string), and from several
/// threads at once when several sources are loading, hence `Send + Sync`.
///
/// A loader that blocks for a long time holds its thread for that long. One
/// that does not know yet whether the bytes will ever exist should answer
/// [`ImageLoadResult::Failed`] and have the app call `reload_image` when they
/// arrive, rather than park a thread per missing picture.
///
/// A loader that **panics** has failed that load: the panic is caught on the
/// load's thread and recorded as an [`ImageLoadResult::Failed`] carrying its
/// message, so the source can be reloaded like any other failure. A loader
/// must not call `reload_image` for the source it is being asked for: that
/// discards the answer it is about to give and asks again, for ever.
///
/// # Closures
///
/// Any `Fn(&str) -> ImageLoadResult + Send + Sync + 'static` is a loader.
pub trait ImageLoader: Send + Sync + 'static {
    /// Load image bytes from the given source.
    ///
    /// `src` is the source string exactly as the element or stylesheet spelled
    /// it: typically a file path or URL, depending on the loader.
    fn load(&self, src: &str) -> ImageLoadResult;
}

impl<F> ImageLoader for F
where
    F: Fn(&str) -> ImageLoadResult + Send + Sync + 'static,
{
    fn load(&self, src: &str) -> ImageLoadResult {
        self(src)
    }
}

/// The app-installed scheme loaders, process-wide: image loads run on
/// background threads and are shared by every document (every window, the
/// embed contexts), so a scheme means the same thing in all of them.
static SCHEME_LOADERS: RwLock<Vec<(String, Arc<dyn ImageLoader>)>> = RwLock::new(Vec::new());

/// The URL scheme **of an image source**, lowercased
/// (`scheme_of("MyApp-Blob:1234")` is `Some("myapp-blob")`): the part before
/// the first `:` when it
/// is a valid scheme (RFC 3986: a letter, then letters, digits, `+`, `-`, `.`)
/// of **two or more** characters. A single letter is a Windows drive
/// (`C:\pictures\a.png`), not a scheme. The source is read as spelled: leading
/// white space is not trimmed, so `" myapp:x"` has no scheme.
///
/// This is the question the registry asks of each source; an app rarely needs
/// it. (`App::image_scheme` is a different thing: it *registers* a loader for
/// a scheme.)
pub fn scheme_of(src: &str) -> Option<String> {
    let (scheme, _) = src.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if scheme.len() < 2
        || !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

/// Install `loader` as the answer for every image source whose URL scheme is
/// `scheme` (`"myapp-blob"` for `myapp-blob:1234`), in every document of this
/// process, for `<img src>` and `background-image: url(…)` alike.
///
/// Sources with any other scheme, and plain paths, still go to the document's
/// own loader (files, plus HTTP(S) with rinch's `image-network` feature), so
/// the fallback is automatic: an app installs only what it alone can resolve.
/// Registering a scheme the default loader also handles (`https`, `file`)
/// takes it over. `data` is the one scheme that cannot be registered: no app
/// loader is ever offered a `data:` URL (an `<img>`'s is decoded in place).
///
/// The registration is process-wide and lasts until
/// [`unregister_image_scheme`]: it is not tied to the component that made it
/// and stays after that component unmounts (a loader is `Send + Sync`, so it
/// can hold no `Signal` an unmount would free).
///
/// The scheme is matched case-insensitively and is given without the colon.
/// Registering it again replaces the earlier loader. See [`ImageLoader`] for
/// what the loader returns and which thread calls it.
///
/// # Panics
///
/// If `scheme` is not a valid URL scheme of two or more characters (see
/// [`scheme_of`]; a name with a `:` in it is not one), or if it is `data`:
/// such a registration could never be asked for a source.
pub fn register_image_scheme(scheme: &str, loader: impl ImageLoader) {
    register_image_scheme_arc(scheme, Arc::new(loader));
}

/// [`register_image_scheme`] for a loader that is already shared.
pub fn register_image_scheme_arc(scheme: &str, loader: Arc<dyn ImageLoader>) {
    let key = registrable_scheme(scheme)
        .unwrap_or_else(|| panic!("`{scheme}` is not a URL scheme an app can load images for"));
    let mut loaders = SCHEME_LOADERS.write().unwrap_or_else(|e| e.into_inner());
    match loaders.iter_mut().find(|(s, _)| *s == key) {
        Some(entry) => entry.1 = loader,
        None => loaders.push((key, loader)),
    }
}

/// The registry key for `scheme` as an app names it (no colon, any case), or
/// `None` for a name that is not a scheme or is `data`. Registering and
/// unregistering read a name the same way.
fn registrable_scheme(scheme: &str) -> Option<String> {
    scheme_of(&format!("{scheme}:")).filter(|key| key.len() == scheme.len() && key != "data")
}

/// Remove the loader installed for `scheme` (named as it was registered: no
/// colon, any case). Returns whether there was one; a name that could not
/// have been registered answers `false`.
/// Sources already loaded (or already failed) keep their cached answer, and a
/// load already running finishes with the loader it started with.
pub fn unregister_image_scheme(scheme: &str) -> bool {
    let Some(key) = registrable_scheme(scheme) else {
        return false;
    };
    let mut loaders = SCHEME_LOADERS.write().unwrap_or_else(|e| e.into_inner());
    let before = loaders.len();
    loaders.retain(|(s, _)| *s != key);
    loaders.len() != before
}

/// The app-installed loader that answers for `src`, if its scheme has one.
pub fn image_scheme_loader(src: &str) -> Option<Arc<dyn ImageLoader>> {
    let scheme = scheme_of(src)?;
    SCHEME_LOADERS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(s, _)| *s == scheme)
        .map(|(_, loader)| loader.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheme_is_what_precedes_the_colon_and_a_drive_letter_is_not_one() {
        assert_eq!(scheme_of("pimble-blob:a/b").as_deref(), Some("pimble-blob"));
        assert_eq!(scheme_of("HTTPS://x/y.png").as_deref(), Some("https"));
        assert_eq!(scheme_of("a+b.c-d:x").as_deref(), Some("a+b.c-d"));
        assert_eq!(scheme_of("C:\\pictures\\a.png"), None);
        assert_eq!(scheme_of("pictures/a.png"), None);
        assert_eq!(scheme_of("pictures/a:b.png"), None);
        assert_eq!(scheme_of("1x:y"), None);
        assert_eq!(scheme_of(":y"), None);
    }

    #[test]
    fn a_registered_scheme_answers_for_its_sources_only_and_can_be_replaced() {
        register_image_scheme("core-test-a", |src: &str| {
            ImageLoadResult::Failed(format!("first {src}"))
        });
        let failure = |src: &str| match image_scheme_loader(src)?.load(src) {
            ImageLoadResult::Failed(e) => Some(e),
            ImageLoadResult::Loaded(_) => None,
        };
        assert_eq!(
            failure("core-test-a:1").as_deref(),
            Some("first core-test-a:1")
        );
        assert_eq!(
            failure("CORE-TEST-A:1").as_deref(),
            Some("first CORE-TEST-A:1"),
            "a scheme is case-insensitive, and the loader sees the source as spelled"
        );
        assert!(image_scheme_loader("core-test-other:1").is_none());
        assert!(
            image_scheme_loader("core-test-a").is_none(),
            "no colon, no scheme"
        );

        register_image_scheme("Core-Test-A", |_: &str| {
            ImageLoadResult::Failed("second".into())
        });
        assert_eq!(failure("core-test-a:1").as_deref(), Some("second"));

        assert!(unregister_image_scheme("core-test-a"));
        assert!(!unregister_image_scheme("core-test-a"));
        assert!(image_scheme_loader("core-test-a:1").is_none());
    }

    #[test]
    #[should_panic(expected = "`DATA` is not a URL scheme an app can load images for")]
    fn the_data_scheme_cannot_be_registered() {
        register_image_scheme("DATA", |_: &str| ImageLoadResult::Failed(String::new()));
    }

    #[test]
    #[should_panic(expected = "not a URL scheme")]
    fn a_name_with_a_colon_in_it_is_not_registered_as_its_first_part() {
        // `my:app` would otherwise be read as the scheme `my`.
        register_image_scheme("core-test-my:app", |_: &str| {
            ImageLoadResult::Failed(String::new())
        });
    }

    #[test]
    fn unregistering_reads_the_name_as_registering_does() {
        register_image_scheme("core-test-un", |_: &str| {
            ImageLoadResult::Failed(String::new())
        });
        assert!(
            !unregister_image_scheme("core-test-un:"),
            "a trailing colon is not the registered name"
        );
        assert!(image_scheme_loader("core-test-un:1").is_some());
        assert!(unregister_image_scheme("CORE-TEST-UN"), "any case");
        assert!(image_scheme_loader("core-test-un:1").is_none());
        assert!(!unregister_image_scheme("c"));
        assert!(!unregister_image_scheme("data"));
    }

    #[test]
    #[should_panic(expected = "not a URL scheme")]
    fn registering_something_that_is_not_a_scheme_panics() {
        register_image_scheme("c", |_: &str| ImageLoadResult::Failed(String::new()));
    }
}
